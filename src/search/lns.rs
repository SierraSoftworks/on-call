//! Large-neighbourhood search: ruin and recreate.
//!
//! Hill climbing eventually reaches a point where no single move — and no pair
//! of moves reachable through the neighbourhood — improves anything, but the
//! schedule is still not great. The way out is to make a change too large to
//! reach one move at a time: clear a contiguous window of the schedule
//! entirely, then rebuild it greedily.
//!
//! Rebuilding is what makes this different from random restarts. The rest of
//! the schedule is left intact, so the repaired window is fitted to a context
//! that is already good, and the result is usually close to feasible straight
//! away rather than needing thousands of steps to recover.

use rand::seq::SliceRandom;
use rand::RngExt;

use crate::model::problem::HumanIdx;
use crate::model::{Assignment, Problem, Score};
use crate::search::incremental::{Change, Incremental};

/// Fraction of the schedule to tear out, as a divisor of the unit count.
const MIN_WINDOW_DIVISOR: usize = 16;
const MAX_WINDOW_DIVISOR: usize = 4;

/// Clears a random window of the schedule and rebuilds it greedily.
pub fn ruin_and_recreate<R: RngExt + ?Sized>(
    problem: &Problem,
    assignment: &mut Assignment,
    incremental: &mut Incremental,
    rng: &mut R,
) {
    let units = problem.unit_count();
    if units == 0 {
        return;
    }

    let smallest = (units / MIN_WINDOW_DIVISOR).max(1);
    let largest = (units / MAX_WINDOW_DIVISOR).max(smallest + 1);
    let width = rng.random_range(smallest..largest).min(units);
    let start = rng.random_range(0..units - width + 1);

    let mut changes: Vec<Change> = Vec::new();
    let mut undo: Vec<Change> = Vec::new();

    // Ruin: clear everything in the window that is not pinned.
    for unit in start..start + width {
        if problem.is_unit_frozen(unit) {
            continue;
        }
        for slot in problem.units[unit].clone() {
            changes.push((slot, None));
        }
    }

    if changes.is_empty() {
        return;
    }

    incremental.apply(problem, assignment, &changes, &mut undo);

    // Recreate: refill the window one unit at a time, in a random order so that
    // repeated kicks explore different rebuilds rather than reproducing the
    // same greedy answer.
    let mut order: Vec<usize> = (start..start + width)
        .filter(|&unit| !problem.is_unit_frozen(unit))
        .collect();
    order.shuffle(rng);

    for unit in order {
        let domain = &problem.unit_domains[unit];
        if domain.is_empty() {
            continue;
        }

        let mut best: Option<(Score, HumanIdx)> = None;

        for &candidate in domain {
            changes.clear();
            for slot in problem.units[unit].clone() {
                changes.push((slot, Some(candidate)));
            }

            incremental.apply(problem, assignment, &changes, &mut undo);
            let score = incremental.score();
            let rollback = undo.clone();
            incremental.undo(problem, assignment, &rollback);

            if best.is_none_or(|(best_score, _)| score < best_score) {
                best = Some((score, candidate));
            }
        }

        if let Some((_, candidate)) = best {
            changes.clear();
            for slot in problem.units[unit].clone() {
                changes.push((slot, Some(candidate)));
            }
            incremental.apply(problem, assignment, &changes, &mut undo);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives;
    use crate::objectives::testing::fixture;
    use crate::search::construct::construct;
    use rand::SeedableRng;
    use rand_pcg::Pcg64;

    #[test]
    fn the_score_stays_consistent_after_a_kick() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = construct(problem);
        let mut incremental = Incremental::new(problem, &assignment);
        let mut rng = Pcg64::seed_from_u64(7);

        for _ in 0..50 {
            ruin_and_recreate(problem, &mut assignment, &mut incremental, &mut rng);
            incremental.verify(problem, &assignment);
        }
    }

    #[test]
    fn a_kick_leaves_the_schedule_covered() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = construct(problem);
        let mut incremental = Incremental::new(problem, &assignment);
        let mut rng = Pcg64::seed_from_u64(8);

        ruin_and_recreate(problem, &mut assignment, &mut incremental, &mut rng);

        for slot in 0..problem.slot_count() {
            if !problem.domains[slot].is_empty() {
                assert!(
                    assignment.get(slot).is_some(),
                    "slot {slot} was left uncovered by the rebuild"
                );
            }
        }
    }

    #[test]
    fn a_kick_only_assigns_available_people() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = construct(problem);
        let mut incremental = Incremental::new(problem, &assignment);
        let mut rng = Pcg64::seed_from_u64(9);

        for _ in 0..20 {
            ruin_and_recreate(problem, &mut assignment, &mut incremental, &mut rng);
            for slot in 0..problem.slot_count() {
                if let Some(human) = assignment.get(slot) {
                    assert!(problem.is_available(human, slot));
                }
            }
        }
    }

    #[test]
    fn a_kick_leaves_frozen_slots_alone() {
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> =
            (0..fixture.problem.slot_count()).map(|_| Some(2)).collect();
        fixture.problem = fixture.problem.with_baseline(baseline);
        fixture.problem.freeze_before(date_time!(2023, 1, 16));

        let problem = &fixture.problem;
        let mut assignment = construct(problem);
        let mut incremental = Incremental::new(problem, &assignment);
        let mut rng = Pcg64::seed_from_u64(10);

        for _ in 0..20 {
            ruin_and_recreate(problem, &mut assignment, &mut incremental, &mut rng);
            for slot in 0..problem.slot_count() {
                if problem.is_frozen(slot) {
                    assert_eq!(assignment.get(slot), Some(2));
                }
            }
        }
    }

    #[test]
    fn a_kick_does_not_wreck_a_good_schedule() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = construct(problem);
        let mut incremental = Incremental::new(problem, &assignment);
        let mut rng = Pcg64::seed_from_u64(11);

        let before = objectives::evaluate(problem, &assignment);
        ruin_and_recreate(problem, &mut assignment, &mut incremental, &mut rng);
        let after = objectives::evaluate(problem, &assignment);

        // Rebuilding greedily into an existing context should land in the same
        // ballpark, not orders of magnitude worse.
        assert!(
            after.hard <= before.hard,
            "a kick should not create coverage gaps: {before} then {after}"
        );
    }

    #[test]
    fn repeated_kicks_explore_different_rebuilds() {
        // The rebuild order is shuffled so that a stuck search does not simply
        // reproduce the same greedy repair every time it is kicked.
        // A roster with real choices to make; on a trivially optimal one the
        // greedy repair would converge to the same answer whatever the order.
        let fixture = crate::objectives::testing::fixture_hard();
        let problem = &fixture.problem;

        let rebuild = |seed: u64| {
            let mut assignment = construct(problem);
            let mut incremental = Incremental::new(problem, &assignment);
            let mut rng = Pcg64::seed_from_u64(seed);
            ruin_and_recreate(problem, &mut assignment, &mut incremental, &mut rng);
            assignment.slots().to_vec()
        };

        assert_ne!(rebuild(1), rebuild(2));
        assert_eq!(rebuild(3), rebuild(3), "a kick must still be reproducible");
    }
}
