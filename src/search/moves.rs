//! The neighbourhood: the set of small changes the search can make.
//!
//! Every move is expressed as a list of `(slot, new owner)` changes, which is
//! all [`crate::search::incremental`] needs to apply and roll it back. Moves
//! only ever propose people who are actually available, so hard availability is
//! a property of the search space rather than something the score has to
//! penalise — it simply cannot be violated.
//!
//! Moves operate on *units*. Normally a unit is one slot; with `rotation.lock`
//! set it is a whole rotation, which is what makes locked rotations structural
//! rather than a soft preference.

use rand::distr::weighted::WeightedIndex;
use rand::distr::Distribution;
use rand::RngExt;

use crate::model::problem::{HumanIdx, Problem};
use crate::model::Assignment;
use crate::search::incremental::Change;

/// The kinds of move the search can make, and how often each is tried.
///
/// The mix matters: small moves refine a schedule but cannot escape a local
/// optimum where two people would both have to change at once, while the run
/// and swap moves make those coordinated changes in one step.
const WEIGHTS: [u32; 5] = [
    35, // Reassign      - retarget a single unit
    25, // SwapUnits     - trade two units between people
    15, // ReassignRun   - hand a whole shift to somebody else
    15, // SwapRuns      - trade two shifts
    10, // ShiftBoundary - slide a handoff earlier or later
];

/// Picks which kind of move to attempt.
///
/// The distribution is built once and reused, because the search consults it
/// on every one of its hundreds of thousands of steps.
pub struct MoveGenerator {
    kinds: WeightedIndex<u32>,
}

impl Default for MoveGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl MoveGenerator {
    pub fn new() -> Self {
        Self {
            kinds: WeightedIndex::new(WEIGHTS)
                .expect("the move weights are a valid distribution"),
        }
    }

    /// Generates a random move into `changes`.
    ///
    /// Returns `false` if no move could be produced this attempt (for example
    /// because the randomly chosen unit is frozen), in which case the caller
    /// should simply try again.
    pub fn generate<R: RngExt + ?Sized>(
        &self,
        problem: &Problem,
        assignment: &Assignment,
        rng: &mut R,
        changes: &mut Vec<Change>,
    ) -> bool {
        changes.clear();

        match self.kinds.sample(rng) {
            0 => reassign(problem, assignment, rng, changes),
            1 => swap_units(problem, assignment, rng, changes),
            2 => reassign_run(problem, assignment, rng, changes),
            3 => swap_runs(problem, assignment, rng, changes),
            _ => shift_boundary(problem, assignment, rng, changes),
        }
    }
}

/// Picks a random unit which is not frozen.
fn pick_unit<R: RngExt + ?Sized>(problem: &Problem, rng: &mut R) -> Option<usize> {
    let unit = rng.random_range(0..problem.unit_count());
    (!problem.is_unit_frozen(unit)).then_some(unit)
}

/// Queues the changes needed to give every slot in a unit to `human`.
fn set_unit(problem: &Problem, unit: usize, human: Option<HumanIdx>, changes: &mut Vec<Change>) {
    for slot in problem.units[unit].clone() {
        changes.push((slot, human));
    }
}

/// The person currently covering a unit, if the unit is uniformly covered.
fn unit_owner(problem: &Problem, assignment: &Assignment, unit: usize) -> Option<HumanIdx> {
    let range = problem.units[unit].clone();
    let owner = assignment.get(range.start);
    debug_assert!(
        range.clone().all(|slot| assignment.get(slot) == owner),
        "units must always be uniformly assigned"
    );
    owner
}

/// Retargets a single unit to a random eligible person, or clears it.
fn reassign<R: RngExt + ?Sized>(
    problem: &Problem,
    assignment: &Assignment,
    rng: &mut R,
    changes: &mut Vec<Change>,
) -> bool {
    let Some(unit) = pick_unit(problem, rng) else {
        return false;
    };

    let domain = &problem.unit_domains[unit];
    if domain.is_empty() {
        return false;
    }

    let current = unit_owner(problem, assignment, unit);
    let candidate = domain[rng.random_range(0..domain.len())];
    if Some(candidate) == current {
        return false;
    }

    set_unit(problem, unit, Some(candidate), changes);
    true
}

/// Trades two units between their owners.
///
/// This keeps both people's total workload unchanged, which lets the search
/// restructure shifts without disturbing a fairness balance it has already
/// found — something a pair of independent reassignments would struggle to do.
fn swap_units<R: RngExt + ?Sized>(
    problem: &Problem,
    assignment: &Assignment,
    rng: &mut R,
    changes: &mut Vec<Change>,
) -> bool {
    let (Some(first), Some(second)) = (pick_unit(problem, rng), pick_unit(problem, rng)) else {
        return false;
    };

    if first == second {
        return false;
    }

    let (Some(left), Some(right)) = (
        unit_owner(problem, assignment, first),
        unit_owner(problem, assignment, second),
    ) else {
        return false;
    };

    if left == right {
        return false;
    }

    if !problem.unit_domains[first].contains(&right)
        || !problem.unit_domains[second].contains(&left)
    {
        return false;
    }

    set_unit(problem, first, Some(right), changes);
    set_unit(problem, second, Some(left), changes);
    true
}

/// Finds the unit-aligned run of consecutive units owned by the same person
/// that contains `unit`.
fn run_of_unit(problem: &Problem, assignment: &Assignment, unit: usize) -> Option<(usize, usize)> {
    let owner = unit_owner(problem, assignment, unit)?;

    let mut start = unit;
    while start > 0 && unit_owner(problem, assignment, start - 1) == Some(owner) {
        start -= 1;
    }

    let mut end = unit;
    while end + 1 < problem.unit_count()
        && unit_owner(problem, assignment, end + 1) == Some(owner)
    {
        end += 1;
    }

    Some((start, end))
}

/// Whether a run of units is free to be modified.
fn run_is_movable(problem: &Problem, start: usize, end: usize) -> bool {
    (start..=end).all(|unit| !problem.is_unit_frozen(unit))
}

/// Whether somebody can cover every unit in a run.
fn can_cover_run(problem: &Problem, start: usize, end: usize, human: HumanIdx) -> bool {
    (start..=end).all(|unit| problem.unit_domains[unit].contains(&human))
}

/// Hands an entire shift to somebody else.
///
/// Single-unit moves cannot do this in one step without passing through a
/// fragmented intermediate state that scores badly and is likely to be
/// rejected, so this is how large fairness corrections actually happen.
fn reassign_run<R: RngExt + ?Sized>(
    problem: &Problem,
    assignment: &Assignment,
    rng: &mut R,
    changes: &mut Vec<Change>,
) -> bool {
    let Some(unit) = pick_unit(problem, rng) else {
        return false;
    };

    let Some((start, end)) = run_of_unit(problem, assignment, unit) else {
        return false;
    };

    if !run_is_movable(problem, start, end) {
        return false;
    }

    let current = unit_owner(problem, assignment, unit);
    let candidates: Vec<HumanIdx> = (0..problem.human_count())
        .filter(|&human| Some(human) != current && can_cover_run(problem, start, end, human))
        .collect();

    if candidates.is_empty() {
        return false;
    }

    let candidate = candidates[rng.random_range(0..candidates.len())];
    for unit in start..=end {
        set_unit(problem, unit, Some(candidate), changes);
    }

    true
}

/// Trades two whole shifts between their owners.
fn swap_runs<R: RngExt + ?Sized>(
    problem: &Problem,
    assignment: &Assignment,
    rng: &mut R,
    changes: &mut Vec<Change>,
) -> bool {
    let (Some(first), Some(second)) = (pick_unit(problem, rng), pick_unit(problem, rng)) else {
        return false;
    };

    let (Some(left), Some(right)) = (
        run_of_unit(problem, assignment, first),
        run_of_unit(problem, assignment, second),
    ) else {
        return false;
    };

    if left == right {
        return false;
    }

    let (Some(left_owner), Some(right_owner)) = (
        unit_owner(problem, assignment, left.0),
        unit_owner(problem, assignment, right.0),
    ) else {
        return false;
    };

    if left_owner == right_owner {
        return false;
    }

    if !run_is_movable(problem, left.0, left.1) || !run_is_movable(problem, right.0, right.1) {
        return false;
    }

    if !can_cover_run(problem, left.0, left.1, right_owner)
        || !can_cover_run(problem, right.0, right.1, left_owner)
    {
        return false;
    }

    for unit in left.0..=left.1 {
        set_unit(problem, unit, Some(right_owner), changes);
    }
    for unit in right.0..=right.1 {
        set_unit(problem, unit, Some(left_owner), changes);
    }

    true
}

/// Slides the handoff between two adjacent shifts earlier or later.
///
/// This is the cheapest way to correct a shift that is one unit too long or too
/// short, which is by far the most common residual flaw in an otherwise good
/// schedule.
fn shift_boundary<R: RngExt + ?Sized>(
    problem: &Problem,
    assignment: &Assignment,
    rng: &mut R,
    changes: &mut Vec<Change>,
) -> bool {
    let Some(unit) = pick_unit(problem, rng) else {
        return false;
    };

    let Some((start, end)) = run_of_unit(problem, assignment, unit) else {
        return false;
    };

    let owner = unit_owner(problem, assignment, start);

    // Either extend this run backwards over its predecessor, or give away the
    // first unit of it to whoever precedes it.
    let grow = rng.random_bool(0.5);

    if grow {
        if start == 0 {
            return false;
        }
        let target = start - 1;
        if problem.is_unit_frozen(target) {
            return false;
        }
        let Some(owner) = owner else { return false };
        if !problem.unit_domains[target].contains(&owner) {
            return false;
        }
        if unit_owner(problem, assignment, target) == Some(owner) {
            return false;
        }

        set_unit(problem, target, Some(owner), changes);
        true
    } else {
        if start == 0 || start == end {
            return false;
        }
        let Some(predecessor) = unit_owner(problem, assignment, start - 1) else {
            return false;
        };
        if problem.is_unit_frozen(start) {
            return false;
        }
        if !problem.unit_domains[start].contains(&predecessor) {
            return false;
        }

        set_unit(problem, start, Some(predecessor), changes);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives::testing::fixture;
    use rand::SeedableRng;
    use rand_pcg::Pcg64;

    fn seeded(problem: &Problem) -> Assignment {
        let mut assignment = Assignment::empty(problem);
        for slot in 0..problem.slot_count() {
            assignment.assign(problem, slot, Some((slot / 2) % 3));
        }
        assignment
    }

    #[test]
    fn generated_moves_only_ever_propose_available_people() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let assignment = seeded(problem);
        let generator = MoveGenerator::new();
        let mut rng = Pcg64::seed_from_u64(1);
        let mut changes = Vec::new();

        for _ in 0..20_000 {
            if !generator.generate(problem, &assignment, &mut rng, &mut changes) {
                continue;
            }

            for &(slot, human) in changes.iter() {
                if let Some(human) = human {
                    assert!(
                        problem.is_available(human, slot),
                        "move proposed {} for a slot they cannot cover",
                        problem.humans[human]
                    );
                }
            }
        }
    }

    #[test]
    fn generated_moves_never_touch_frozen_slots() {
        let mut fixture = fixture();
        fixture.problem.freeze_before(date_time!(2023, 1, 16));
        let problem = &fixture.problem;
        let assignment = seeded(problem);
        let generator = MoveGenerator::new();
        let mut rng = Pcg64::seed_from_u64(2);
        let mut changes = Vec::new();

        let mut touched = 0;
        for _ in 0..20_000 {
            if !generator.generate(problem, &assignment, &mut rng, &mut changes) {
                continue;
            }
            touched += 1;

            for &(slot, _) in changes.iter() {
                assert!(
                    !problem.is_frozen(slot),
                    "move touched frozen slot {slot}"
                );
            }
        }

        assert!(touched > 0, "expected at least some moves to be generated");
    }

    #[test]
    fn generated_moves_are_non_empty_when_reported_as_successful() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let assignment = seeded(problem);
        let generator = MoveGenerator::new();
        let mut rng = Pcg64::seed_from_u64(3);
        let mut changes = Vec::new();

        for _ in 0..5_000 {
            if generator.generate(problem, &assignment, &mut rng, &mut changes) {
                assert!(!changes.is_empty());
            }
        }
    }

    #[test]
    fn moves_respect_rotation_locking() {
        use crate::config::{Config, Human, Rotation};
        use crate::constraints::Constraint;
        use crate::model::Problem;
        use chrono::{Duration, Weekday};

        let config = Config::for_test(
            Duration::days(3),
            [
                ("alice@example.com".to_string(), Human::default()),
                ("bob@example.com".to_string(), Human::default()),
            ]
            .into_iter()
            .collect(),
        )
        .with_constraints(vec![
            Constraint::DayOfWeek(vec![
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
            ]),
            Constraint::TimeOfDay {
                start: time!(9, 0),
                end: time!(17, 0),
            },
        ])
        .with_rotation(Rotation {
            lock: true,
            boundary: None,
        });

        let problem = Problem::build(
            &config,
            date_time!(2023, 1, 2),
            date_time!(2023, 1, 2) + Duration::days(28),
        )
        .unwrap();

        let mut assignment = Assignment::empty(&problem);
        for unit in 0..problem.unit_count() {
            for slot in problem.units[unit].clone() {
                assignment.assign(&problem, slot, Some(unit % 2));
            }
        }

        let generator = MoveGenerator::new();
        let mut rng = Pcg64::seed_from_u64(4);
        let mut changes = Vec::new();

        for _ in 0..10_000 {
            if !generator.generate(&problem, &assignment, &mut rng, &mut changes) {
                continue;
            }

            // Every touched unit must be touched in its entirety, otherwise a
            // handoff could land mid-rotation.
            let mut per_unit: std::collections::HashMap<usize, usize> =
                std::collections::HashMap::new();
            for &(slot, _) in changes.iter() {
                *per_unit.entry(problem.unit_of_slot[slot]).or_default() += 1;
            }

            for (unit, count) in per_unit {
                assert_eq!(
                    count,
                    problem.units[unit].len(),
                    "rotation {unit} was only partially reassigned"
                );
            }
        }
    }

    #[test]
    fn every_move_kind_can_fire() {
        // A sanity check that the move mix is actually reachable, so a bug in
        // one generator cannot silently reduce the neighbourhood.
        let fixture = fixture();
        let problem = &fixture.problem;
        let assignment = seeded(problem);
        let mut changes = Vec::new();

        type Generator = fn(&Problem, &Assignment, &mut Pcg64, &mut Vec<Change>) -> bool;

        let generators: [(&str, Generator); 5] = [
            ("reassign", reassign),
            ("swap_units", swap_units),
            ("reassign_run", reassign_run),
            ("swap_runs", swap_runs),
            ("shift_boundary", shift_boundary),
        ];

        for (name, generator) in generators {
            let mut rng = Pcg64::seed_from_u64(5);
            let fired = (0..5_000).any(|_| {
                changes.clear();
                generator(problem, &assignment, &mut rng, &mut changes)
            });
            assert!(fired, "{name} never produced a move");
        }
    }

    #[test]
    fn swap_units_preserves_total_workload() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = seeded(problem);
        let mut rng = Pcg64::seed_from_u64(6);
        let mut changes = Vec::new();

        for _ in 0..2_000 {
            changes.clear();
            if !swap_units(problem, &assignment, &mut rng, &mut changes) {
                continue;
            }

            let before: Vec<i64> = (0..problem.human_count())
                .map(|h| assignment.assigned_minutes(h))
                .collect();

            let restore: Vec<Change> =
                changes.iter().map(|&(slot, _)| (slot, assignment.get(slot))).collect();
            for &(slot, human) in changes.iter() {
                assignment.assign(problem, slot, human);
            }

            let after: Vec<i64> = (0..problem.human_count())
                .map(|h| assignment.assigned_minutes(h))
                .collect();

            for &(slot, human) in restore.iter() {
                assignment.assign(problem, slot, human);
            }

            assert_eq!(
                before.iter().sum::<i64>(),
                after.iter().sum::<i64>(),
                "a swap must not change the total assigned time"
            );
        }
    }
}
