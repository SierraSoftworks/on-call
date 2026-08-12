//! Late-acceptance hill climbing.
//!
//! A candidate move is accepted if it beats either the current score or the
//! score from `history_length` steps ago. That single rule is enough to escape
//! local optima — the search will accept a worse schedule now if it was doing
//! worse a while back — while still trending downward, and unlike simulated
//! annealing it has no temperature schedule to tune. There is one parameter,
//! and its effect is easy to reason about: longer history means more tolerance
//! for detours.
//!
//! The search is deterministic: given the same problem, seed and step budget it
//! performs exactly the same sequence of moves and returns exactly the same
//! schedule.

use rand::SeedableRng;

use crate::model::{Assignment, Problem, Score};
use crate::search::incremental::{Change, Incremental};
use crate::search::lns;
use crate::search::moves::MoveGenerator;

/// The generator the search draws from.
///
/// This is deliberately a specific, value-stable generator rather than
/// `rand`'s `StdRng`. The tool promises that the same inputs and seed always
/// produce the same schedule, and people rely on that to review rotas in
/// version control and to make `--baseline` meaningful. `StdRng` is documented
/// as free to change algorithm between `rand` releases, which would silently
/// reshuffle every published schedule the next time the lockfile was updated.
/// `Pcg64`'s output for a given seed is part of its published API, so upgrading
/// `rand` cannot move anybody's on-call shifts.
pub type SearchRng = rand_pcg::Pcg64;

/// How the search should be run.
#[derive(Debug, Clone)]
pub struct Options {
    /// Number of candidate moves to evaluate.
    pub steps: u64,
    /// Seed for the move generator. The same seed always produces the same
    /// schedule, on every platform and for every build of the tool.
    pub seed: u64,
    /// Length of the late-acceptance history.
    ///
    /// Longer means more tolerance for detours. Too long and the search never
    /// settles: it keeps accepting moves that were reasonable thousands of
    /// steps ago and wanders instead of converging.
    pub history_length: usize,
    /// Optional wall-clock budget. Non-deterministic, so it is off by default.
    pub time_budget: Option<std::time::Duration>,
    /// How many steps without improvement before a ruin-and-recreate kick.
    ///
    /// `None` scales this with the step budget, which is almost always what you
    /// want — see [`Options::stagnation_limit`]. `Some(0)` disables kicks.
    pub stagnation_limit: Option<u64>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            steps: 200_000,
            seed: 0,
            history_length: 200,
            time_budget: None,
            stagnation_limit: None,
        }
    }
}

impl Options {
    /// The number of fruitless steps to tolerate before kicking the search.
    ///
    /// Kicks are only worth making if there is enough budget left to recover
    /// from one. Tearing out a slice of the schedule reliably makes it worse in
    /// the short term, and if the search runs out of steps before it has
    /// rebuilt, that damage is what gets returned. Measured across the example
    /// rotas, kicking every `steps / 4` is a good balance: frequent enough to
    /// break out of a local optimum, rare enough to always recover. Kicking
    /// four times as often made every example *worse*, not better.
    pub fn stagnation_limit(&self) -> u64 {
        self.stagnation_limit.unwrap_or(self.steps / 4)
    }
}

/// What the search did, for reporting.
#[derive(Debug, Clone, Copy)]
pub struct Statistics {
    pub steps: u64,
    pub accepted: u64,
    pub improvements: u64,
    pub kicks: u64,
    pub initial: Score,
    pub best: Score,
}

/// Runs the search, returning the best schedule found.
pub fn optimize(
    problem: &Problem,
    initial: Assignment,
    options: &Options,
) -> (Assignment, Statistics) {
    let mut assignment = initial;
    let mut incremental = Incremental::new(problem, &assignment);

    let initial_score = incremental.score();
    let mut best = assignment.clone();
    let mut best_score = initial_score;

    let mut history = vec![initial_score; options.history_length.max(1)];
    let mut rng = SearchRng::seed_from_u64(options.seed);
    let generator = MoveGenerator::new();

    let mut changes: Vec<Change> = Vec::with_capacity(16);
    let mut undo: Vec<Change> = Vec::with_capacity(16);
    let mut rollback: Vec<Change> = Vec::with_capacity(16);

    let started = std::time::Instant::now();
    let mut statistics = Statistics {
        steps: 0,
        accepted: 0,
        improvements: 0,
        kicks: 0,
        initial: initial_score,
        best: initial_score,
    };

    let stagnation_limit = options.stagnation_limit();
    let mut since_improvement = 0u64;

    for step in 0..options.steps {
        // Checking the clock is relatively expensive, so only do it
        // occasionally. This is also why a time budget is opt-in: it makes the
        // result depend on how fast the machine is.
        if let Some(budget) = options.time_budget {
            if step % 1024 == 0 && started.elapsed() >= budget {
                break;
            }
        }

        statistics.steps += 1;

        let cursor = (step as usize) % history.len();

        if !generator.generate(problem, &assignment, &mut rng, &mut changes) {
            continue;
        }

        let current = incremental.score();
        incremental.apply(problem, &mut assignment, &changes, &mut undo);
        let candidate = incremental.score();

        let accepted = candidate <= current || candidate <= history[cursor];

        if accepted {
            statistics.accepted += 1;

            if candidate < best_score {
                statistics.improvements += 1;
                best_score = candidate;
                best.clone_from(&assignment);
                since_improvement = 0;
            } else {
                since_improvement += 1;
            }
        } else {
            rollback.clear();
            rollback.extend_from_slice(&undo);
            incremental.undo(problem, &mut assignment, &rollback);
            since_improvement += 1;
        }

        history[cursor] = incremental.score();

        // A long run without improvement means the neighbourhood is exhausted.
        // Tear out a chunk of the schedule and rebuild it to land somewhere
        // genuinely new, rather than continuing to nibble at the same optimum.
        if stagnation_limit > 0 && since_improvement >= stagnation_limit {
            assignment.clone_from(&best);
            incremental = Incremental::new(problem, &assignment);
            lns::ruin_and_recreate(problem, &mut assignment, &mut incremental, &mut rng);

            statistics.kicks += 1;
            since_improvement = 0;

            let score = incremental.score();
            if score < best_score {
                best_score = score;
                best.clone_from(&assignment);
            }

            history.iter_mut().for_each(|entry| *entry = score);
        }

        // In debug builds, periodically prove the incrementally maintained
        // score still matches a from-scratch evaluation. Too slow for release,
        // but it means any drift is caught by the test suite immediately.
        #[cfg(debug_assertions)]
        if step % 4096 == 0 {
            incremental.verify(problem, &assignment);
        }
    }

    statistics.best = best_score;
    (best, statistics)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives;
    use crate::objectives::testing::fixture;
    use crate::search::construct::construct;

    fn options(steps: u64) -> Options {
        Options {
            steps,
            seed: 42,
            history_length: 200,
            time_budget: None,
            stagnation_limit: None,
        }
    }

    #[test]
    fn optimizing_never_returns_a_worse_schedule_than_it_started_with() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let initial = construct(problem);
        let initial_score = objectives::evaluate(problem, &initial);

        let (result, statistics) = optimize(problem, initial, &options(20_000));
        let final_score = objectives::evaluate(problem, &result);

        assert!(
            final_score <= initial_score,
            "{final_score} should be no worse than {initial_score}"
        );
        assert_eq!(statistics.best, final_score);
        assert_eq!(statistics.initial, initial_score);
    }

    #[test]
    fn optimizing_actually_improves_on_construction() {
        // The easy fixture is solved outright by construction, so this uses the
        // awkward roster where greedy demonstrably leaves value on the table.
        let fixture = crate::objectives::testing::fixture_hard();
        let problem = &fixture.problem;
        let initial = construct(problem);
        let initial_score = objectives::evaluate(problem, &initial);

        let (result, statistics) = optimize(problem, initial, &options(200_000));
        let final_score = objectives::evaluate(problem, &result);

        assert!(
            final_score < initial_score,
            "the search should find something better than {initial_score}, got {final_score}"
        );
        assert!(
            statistics.improvements > 0,
            "improvements should have been recorded"
        );
    }

    #[test]
    fn construction_alone_already_solves_an_easy_roster() {
        // Worth pinning: on a simple, unconstrained rota the greedy start lands
        // on a clean repeating cycle, and the optimizer should leave it alone
        // rather than churning it for no gain.
        let fixture = fixture();
        let problem = &fixture.problem;
        let initial = construct(problem);

        let (result, _) = optimize(problem, initial.clone(), &options(20_000));

        assert_eq!(result.slots(), initial.slots());
    }

    #[test]
    fn the_reported_score_matches_the_returned_schedule() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let (result, statistics) = optimize(problem, construct(problem), &options(20_000));

        assert_eq!(statistics.best, objectives::evaluate(problem, &result));
    }

    /// Pins the exact output of the search's generator.
    ///
    /// This is the guard on the tool's stable-output promise. If a dependency
    /// upgrade ever changes the algorithm behind [`SearchRng`], every schedule
    /// this tool has produced would change on the next run — and without this
    /// test, nothing would say so. A failure here is not something to paper
    /// over by updating the numbers: it means published rotas are about to
    /// move, which needs to be a deliberate, released decision rather than a
    /// side effect of `cargo update`.
    #[test]
    fn the_search_generator_is_value_stable() {
        use rand::RngExt;

        let mut rng = SearchRng::seed_from_u64(0);
        let sequence: Vec<u64> = (0..4).map(|_| rng.random()).collect();

        assert_eq!(
            sequence,
            vec![
                2354861276966075475,
                6411218084291373563,
                13092586260176364081,
                718076624554797018,
            ],
            "the search generator changed; see this test's documentation"
        );

        // Seeding is part of the same promise: `seed_from_u64` has to expand a
        // seed the same way across versions, not just the generator itself.
        assert_eq!(
            SearchRng::seed_from_u64(12345).random::<u64>(),
            4242560371181940401
        );
    }

    #[test]
    fn the_stagnation_limit_scales_with_the_step_budget() {
        assert_eq!(
            Options {
                steps: 200_000,
                ..Options::default()
            }
            .stagnation_limit(),
            50_000
        );

        assert_eq!(
            Options {
                steps: 1_000_000,
                ..Options::default()
            }
            .stagnation_limit(),
            250_000
        );

        // An explicit setting still wins, and zero means never kick.
        assert_eq!(
            Options {
                steps: 200_000,
                stagnation_limit: Some(7),
                ..Options::default()
            }
            .stagnation_limit(),
            7
        );
    }

    #[test]
    fn kicking_too_often_is_worse_than_not_kicking_at_all() {
        // Pins the tuning result behind the default stagnation limit. A kick
        // reliably makes the schedule worse before it makes it better, so
        // kicking faster than the search can recover is counterproductive —
        // which is why the limit scales with the budget rather than being a
        // fixed count.
        let fixture = crate::objectives::testing::fixture_hard();
        let problem = &fixture.problem;

        let run = |stagnation: Option<u64>| {
            let options = Options {
                steps: 50_000,
                seed: 3,
                stagnation_limit: stagnation,
                ..Options::default()
            };
            optimize(problem, construct(problem), &options).1.best
        };

        let scaled = run(None);
        let frantic = run(Some(500));

        assert!(
            scaled < frantic,
            "the scaled limit ({scaled}) should beat kicking every 500 steps ({frantic})"
        );
    }

    #[test]
    fn the_same_seed_produces_the_same_schedule() {
        let fixture = fixture();
        let problem = &fixture.problem;

        let (first, _) = optimize(problem, construct(problem), &options(20_000));
        let (second, _) = optimize(problem, construct(problem), &options(20_000));

        assert_eq!(
            first.slots(),
            second.slots(),
            "the search must be reproducible"
        );
    }

    #[test]
    fn the_seed_changes_the_search_trajectory() {
        let fixture = crate::objectives::testing::fixture_hard();
        let problem = &fixture.problem;

        let mut first_options = options(20_000);
        first_options.seed = 1;
        let mut second_options = options(20_000);
        second_options.seed = 2;

        let (first, first_stats) = optimize(problem, construct(problem), &first_options);
        let (second, second_stats) = optimize(problem, construct(problem), &second_options);

        // Two seeds may legitimately converge on the same schedule, but they
        // must not walk the same path to get there — otherwise the seed is not
        // reaching the move generator.
        assert!(
            first.slots() != second.slots() || first_stats.accepted != second_stats.accepted,
            "the seed should influence the search"
        );
    }

    #[test]
    fn a_longer_budget_is_never_worse() {
        let fixture = fixture();
        let problem = &fixture.problem;

        let (short, _) = optimize(problem, construct(problem), &options(5_000));
        let (long, _) = optimize(problem, construct(problem), &options(60_000));

        assert!(
            objectives::evaluate(problem, &long) <= objectives::evaluate(problem, &short),
            "more steps should not produce a worse result"
        );
    }

    #[test]
    fn the_result_only_uses_available_people() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let (result, _) = optimize(problem, construct(problem), &options(20_000));

        for slot in 0..problem.slot_count() {
            if let Some(human) = result.get(slot) {
                assert!(problem.is_available(human, slot));
            }
        }
    }

    #[test]
    fn zero_steps_returns_the_initial_schedule() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let initial = construct(problem);

        let (result, statistics) = optimize(problem, initial.clone(), &options(0));

        assert_eq!(result.slots(), initial.slots());
        assert_eq!(statistics.steps, 0);
    }

    #[test]
    fn frozen_slots_survive_optimization() {
        let mut fixture = fixture();
        let baseline: Vec<Option<usize>> =
            (0..fixture.problem.slot_count()).map(|_| Some(1)).collect();
        fixture.problem = fixture.problem.with_baseline(baseline);
        fixture.problem.freeze_before(date_time!(2023, 1, 16));

        let problem = &fixture.problem;
        let (result, _) = optimize(problem, construct(problem), &options(20_000));

        for slot in 0..problem.slot_count() {
            if problem.is_frozen(slot) {
                assert_eq!(result.get(slot), Some(1), "frozen slot {slot} moved");
            }
        }
    }
}
