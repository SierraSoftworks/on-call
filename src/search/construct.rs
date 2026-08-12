//! Building an initial schedule for the search to improve on.
//!
//! Local search needs somewhere to start. Starting from a random assignment
//! works but wastes a lot of the step budget climbing out of the noise, so we
//! build a reasonable schedule greedily first: walk the units in order and give
//! each one to whoever the score likes best right now.
//!
//! This is only a starting point. Unlike the greedy algorithm it replaces, it
//! makes no claim to be a good schedule on its own — it exists so that the
//! optimizer converges faster, and every decision it makes is up for revision.

use crate::model::problem::{HumanIdx, Problem};
use crate::model::{Assignment, Score};
use crate::search::incremental::{Change, Incremental};

/// Greedily builds an initial schedule.
///
/// When a baseline is supplied it is used as the starting point, rather than
/// being treated as something to find our way back to. Starting from the
/// published schedule means the search begins at zero stability cost and only
/// moves a shift when the gain genuinely outweighs the disruption — whereas
/// starting from a greedy schedule and relying on the stability objective to
/// pull us back would leave the final answer at the mercy of whichever local
/// optimum the search happened to reach.
///
/// Anything the baseline does not cover — new time in the horizon, or somebody
/// who has since become unavailable — is filled in greedily around it.
pub fn construct(problem: &Problem) -> Assignment {
    let mut assignment = Assignment::empty(problem);

    if let Some(baseline) = problem.baseline.as_ref() {
        for (slot, &owner) in baseline.iter().enumerate() {
            match owner {
                Some(human) if problem.is_available(human, slot) => {
                    assignment.assign(problem, slot, Some(human));
                }
                _ => {}
            }
        }
    }

    let mut incremental = Incremental::new(problem, &assignment);
    let mut undo: Vec<Change> = Vec::new();
    let mut changes: Vec<Change> = Vec::new();

    for unit in 0..problem.unit_count() {
        if problem.is_unit_frozen(unit) {
            continue;
        }

        // Units the baseline already filled are left alone; the search will
        // revisit them if there is something to gain.
        if problem.units[unit]
            .clone()
            .all(|slot| assignment.get(slot).is_some())
        {
            continue;
        }

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

            incremental.apply(problem, &mut assignment, &changes, &mut undo);
            let score = incremental.score();
            let rollback = undo.clone();
            incremental.undo(problem, &mut assignment, &rollback);

            // Ties are broken by the lowest index, and indices follow sorted
            // names, so construction is deterministic.
            if best.is_none_or(|(best_score, _)| score < best_score) {
                best = Some((score, candidate));
            }
        }

        if let Some((_, candidate)) = best {
            changes.clear();
            for slot in problem.units[unit].clone() {
                changes.push((slot, Some(candidate)));
            }
            incremental.apply(problem, &mut assignment, &changes, &mut undo);
        }
    }

    assignment
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives;
    use crate::objectives::testing::fixture;

    #[test]
    fn every_coverable_slot_gets_filled() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let assignment = construct(problem);

        for slot in 0..problem.slot_count() {
            if !problem.domains[slot].is_empty() {
                assert!(
                    assignment.get(slot).is_some(),
                    "slot {slot} was coverable but left empty"
                );
            }
        }
    }

    #[test]
    fn only_available_people_are_assigned() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let assignment = construct(problem);

        for slot in 0..problem.slot_count() {
            if let Some(human) = assignment.get(slot) {
                assert!(problem.is_available(human, slot));
            }
        }
    }

    #[test]
    fn construction_is_deterministic() {
        let fixture = fixture();
        let first = construct(&fixture.problem);
        let second = construct(&fixture.problem);

        assert_eq!(first.slots(), second.slots());
    }

    #[test]
    fn construction_beats_an_empty_schedule() {
        let fixture = fixture();
        let problem = &fixture.problem;

        let empty = objectives::evaluate(problem, &Assignment::empty(problem));
        let built = objectives::evaluate(problem, &construct(problem));

        assert!(built < empty, "{built} should beat {empty}");
    }

    #[test]
    fn construction_spreads_the_work_around() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let assignment = construct(problem);

        for human in 0..problem.human_count() {
            assert!(
                assignment.assigned_minutes(human) > 0,
                "{} was given nothing to do",
                problem.humans[human]
            );
        }
    }

    #[test]
    fn frozen_slots_keep_their_baseline_owner() {
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> =
            (0..fixture.problem.slot_count()).map(|_| Some(1)).collect();
        fixture.problem = fixture.problem.with_baseline(baseline);
        fixture.problem.freeze_before(date_time!(2023, 1, 16));

        let problem = &fixture.problem;
        let assignment = construct(problem);

        for slot in 0..problem.slot_count() {
            if problem.is_frozen(slot) {
                assert_eq!(
                    assignment.get(slot),
                    Some(1),
                    "frozen slot {slot} should have kept its baseline owner"
                );
            }
        }
    }

    #[test]
    fn a_baseline_is_used_as_the_starting_point() {
        // Construction should adopt the whole published schedule, not just the
        // frozen part of it. Starting from the baseline means the search begins
        // at zero stability cost, so a re-run with nothing changed has nothing
        // to gain by moving anybody.
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> = (0..fixture.problem.slot_count())
            .map(|slot| Some((slot / 2) % 3))
            .collect();
        fixture.problem = fixture.problem.with_baseline(baseline.clone());

        let assignment = construct(&fixture.problem);

        assert_eq!(
            assignment.slots(),
            baseline.as_slice(),
            "construction should have adopted the baseline verbatim"
        );
    }

    #[test]
    fn gaps_in_a_baseline_are_filled_in_greedily() {
        // New time in the horizon, or somebody who has since become
        // unavailable, leaves holes the baseline cannot fill.
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> = (0..fixture.problem.slot_count())
            .map(|slot| (slot % 3 != 0).then_some(1))
            .collect();
        fixture.problem = fixture.problem.with_baseline(baseline);

        let problem = &fixture.problem;
        let assignment = construct(problem);

        assert_eq!(
            assignment.unassigned_count(),
            0,
            "the holes should have been filled"
        );

        for slot in 0..problem.slot_count() {
            if slot % 3 != 0 {
                assert_eq!(
                    assignment.get(slot),
                    Some(1),
                    "slot {slot} was covered by the baseline and should be untouched"
                );
            }
        }
    }

    #[test]
    fn a_baseline_owner_who_became_unavailable_is_replaced() {
        use crate::config::{Config, Human};
        use crate::constraints::Constraint;
        use crate::model::Problem;
        use chrono::{Duration, Weekday};

        let config = Config::for_test(
            Duration::days(1),
            [
                (
                    "alice@example.com".to_string(),
                    Human::default().with_constraints(vec![Constraint::DayOfWeek(vec![
                        Weekday::Mon,
                    ])]),
                ),
                ("bob@example.com".to_string(), Human::default()),
            ]
            .into_iter()
            .collect(),
        )
        .with_constraints(vec![
            Constraint::DayOfWeek(vec![Weekday::Mon, Weekday::Tue]),
            Constraint::TimeOfDay {
                start: time!(9, 0),
                end: time!(17, 0),
            },
        ]);

        let problem = Problem::build(
            &config,
            date_time!(2023, 1, 2),
            date_time!(2023, 1, 2) + Duration::days(14),
        )
        .unwrap();

        // The baseline puts Alice everywhere, but she can now only work Mondays.
        let baseline = vec![Some(0); problem.slot_count()];
        let problem = problem.with_baseline(baseline);
        let assignment = construct(&problem);

        for slot in 0..problem.slot_count() {
            let owner = assignment.get(slot).expect("every slot should be covered");
            assert!(
                problem.is_available(owner, slot),
                "an unavailable baseline owner should have been replaced"
            );
        }
    }

    #[test]
    fn units_are_assigned_as_a_whole_when_rotations_are_locked() {
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

        let assignment = construct(&problem);

        for unit in 0..problem.unit_count() {
            let owners: Vec<_> = problem.units[unit]
                .clone()
                .map(|slot| assignment.get(slot))
                .collect();
            assert!(
                owners.windows(2).all(|w| w[0] == w[1]),
                "rotation {unit} was split between people: {owners:?}"
            );
        }
    }
}
