//! What makes one schedule better than another.
//!
//! Each objective is defined here as a *local* penalty function — over a single
//! slot, a single person, a single shift, or a single gap — plus a loop that
//! aggregates those locals across the whole schedule.
//!
//! The from-scratch aggregation in this module is the specification: it is
//! written for clarity, not speed, and is what the tests compare against. The
//! search uses [`crate::search::incremental`], which maintains the same total
//! by re-evaluating only the parts a move touched. The two are checked against
//! each other continuously (see the score corruption tests), so the fast path
//! can never silently drift from the definition.

pub mod coverage;
pub mod fairness;
pub mod preference;
pub mod rest;
pub mod runlength;
pub mod stability;

use crate::model::problem::HumanIdx;
use crate::model::{Assignment, Problem, Score};

/// Every objective, in a stable order.
pub const NAMES: [&str; 6] = [
    coverage::NAME,
    fairness::NAME,
    runlength::NAME,
    rest::NAME,
    preference::NAME,
    stability::NAME,
];

/// Evaluates a whole schedule from scratch.
///
/// This is the reference implementation. It is O(slots + humans × runs), which
/// is fine for reporting but far too slow to call inside the search loop.
pub fn evaluate(problem: &Problem, assignment: &Assignment) -> Score {
    coverage::evaluate(problem, assignment)
        + fairness::evaluate(problem, assignment)
        + runlength::evaluate(problem, assignment)
        + rest::evaluate(problem, assignment)
        + preference::evaluate(problem, assignment)
        + stability::evaluate(problem, assignment)
}

/// Evaluates a schedule, reporting each objective's contribution separately.
pub fn breakdown(problem: &Problem, assignment: &Assignment) -> Vec<(&'static str, Score)> {
    vec![
        (coverage::NAME, coverage::evaluate(problem, assignment)),
        (fairness::NAME, fairness::evaluate(problem, assignment)),
        (runlength::NAME, runlength::evaluate(problem, assignment)),
        (rest::NAME, rest::evaluate(problem, assignment)),
        (preference::NAME, preference::evaluate(problem, assignment)),
        (stability::NAME, stability::evaluate(problem, assignment)),
    ]
}

/// The objectives which depend only on a slot and who is covering it.
///
/// These have O(1) deltas: changing one slot's owner affects exactly that
/// slot's contribution and nothing else.
#[inline]
pub fn slot_local(
    problem: &Problem,
    slot: crate::model::SlotIdx,
    assignee: Option<HumanIdx>,
) -> Score {
    coverage::slot_penalty(problem, slot, assignee)
        + preference::slot_penalty(problem, slot, assignee)
        + stability::slot_penalty(problem, slot, assignee)
}

/// The objectives which depend on the *shape* of one person's schedule — how
/// their slots group into shifts, and how far apart those shifts are.
///
/// Changing a slot's owner can merge, split, extend or remove a shift, so these
/// have to be recomputed for the two people involved rather than adjusted.
#[inline]
pub fn human_shape(problem: &Problem, assignment: &Assignment, human: HumanIdx) -> Score {
    runlength::human_penalty(problem, assignment, human)
        + rest::human_penalty(problem, assignment, human)
}

/// The objective which depends on a person's total workload.
#[inline]
pub fn human_workload(problem: &Problem, human: HumanIdx, assigned_minutes: i64) -> Score {
    fairness::human_penalty(problem, human, assigned_minutes)
}

#[cfg(test)]
pub mod testing {
    //! Shared fixtures for objective tests.

    use crate::config::{Config, Human, Preference};
    use crate::constraints::Constraint;
    use crate::model::Problem;
    use chrono::{Duration, Weekday};

    pub struct Fixture {
        pub problem: Problem,
    }

    fn build(humans: Vec<(&str, Human)>, shift_days: i64) -> Fixture {
        let config = Config::for_test(
            Duration::days(shift_days),
            humans
                .into_iter()
                .map(|(name, human)| (name.to_string(), human))
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
        ]);

        let problem = Problem::build(
            &config,
            date_time!(2023, 1, 2),
            date_time!(2023, 1, 2) + Duration::days(28),
        )
        .expect("the fixture should build");

        Fixture { problem }
    }

    /// Three unconstrained engineers on a weekday 09:00-17:00 rota with
    /// two-day shifts.
    pub fn fixture() -> Fixture {
        build(
            vec![
                ("alice@example.com", Human::default()),
                ("bob@example.com", Human::default()),
                ("claire@example.com", Human::default()),
            ],
            2,
        )
    }

    /// As [`fixture`], but Alice avoids Fridays and prefers Mondays.
    pub fn fixture_with_preferences() -> Fixture {
        build(
            vec![
                (
                    "alice@example.com",
                    Human::default().with_preferences(vec![
                        Preference {
                            avoid: Some(Constraint::DayOfWeek(vec![Weekday::Fri])),
                            prefer: None,
                            weight: 1.0,
                        },
                        Preference {
                            avoid: None,
                            prefer: Some(Constraint::DayOfWeek(vec![Weekday::Mon])),
                            weight: 1.0,
                        },
                    ]),
                ),
                ("bob@example.com", Human::default()),
                ("claire@example.com", Human::default()),
            ],
            2,
        )
    }

    /// A deliberately awkward roster: a part-timer, somebody on leave, somebody
    /// carrying prior workload, and preferences that pull against fairness.
    ///
    /// Greedy construction does noticeably badly here because every early
    /// decision constrains the later ones, which makes it a useful measure of
    /// whether the optimizer is actually earning its keep.
    pub fn fixture_hard() -> Fixture {
        let config = Config::for_test(
            Duration::days(3),
            [
                (
                    "alice@example.com".to_string(),
                    Human::default().with_constraints(vec![Constraint::DayOfWeek(vec![
                        Weekday::Mon,
                        Weekday::Wed,
                        Weekday::Fri,
                    ])]),
                ),
                (
                    "bob@example.com".to_string(),
                    Human::default().with_constraints(vec![Constraint::Unavailable {
                        start: date!(2023, 1, 16),
                        end: date!(2023, 1, 30),
                    }]),
                ),
                (
                    "claire@example.com".to_string(),
                    Human::default().with_prior_workload(Duration::hours(48)),
                ),
                (
                    "donovan@example.com".to_string(),
                    Human::default().with_preferences(vec![Preference {
                        avoid: Some(Constraint::DayOfWeek(vec![Weekday::Mon])),
                        prefer: None,
                        weight: 3.0,
                    }]),
                ),
                ("erica@example.com".to_string(), Human::default()),
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
        ]);

        let problem = Problem::build(
            &config,
            date_time!(2023, 1, 2),
            date_time!(2023, 1, 2) + Duration::days(120),
        )
        .expect("the hard fixture should build");

        Fixture { problem }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testing::fixture;

    #[test]
    fn breakdown_sums_to_the_total() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);

        for slot in 0..problem.slot_count() {
            assignment.assign(problem, slot, Some(slot % 3));
        }

        let total: Score = breakdown(problem, &assignment)
            .into_iter()
            .map(|(_, score)| score)
            .sum();

        assert_eq!(total, evaluate(problem, &assignment));
    }

    #[test]
    fn breakdown_reports_every_objective() {
        let fixture = fixture();
        let assignment = Assignment::empty(&fixture.problem);
        let names: Vec<&str> = breakdown(&fixture.problem, &assignment)
            .into_iter()
            .map(|(name, _)| name)
            .collect();

        assert_eq!(names, NAMES.to_vec());
    }

    #[test]
    fn an_empty_schedule_is_infeasible() {
        let fixture = fixture();
        let assignment = Assignment::empty(&fixture.problem);

        assert!(!evaluate(&fixture.problem, &assignment).is_feasible());
    }

    #[test]
    fn a_sensible_rota_beats_one_person_doing_everything() {
        let fixture = fixture();
        let problem = &fixture.problem;

        let mut hogged = Assignment::empty(problem);
        for slot in 0..problem.slot_count() {
            hogged.assign(problem, slot, Some(0));
        }

        // Two-day shifts rotating through the team, matching shiftLength.
        let mut rota = Assignment::empty(problem);
        for slot in 0..problem.slot_count() {
            rota.assign(problem, slot, Some((slot / 2) % 3));
        }

        assert!(
            evaluate(problem, &rota) < evaluate(problem, &hogged),
            "rotating fairly should beat one person covering everything"
        );
    }

    #[test]
    fn slot_local_and_human_terms_reconstruct_the_total() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);
        for slot in 0..problem.slot_count() {
            assignment.assign(problem, slot, Some((slot / 2) % 3));
        }

        let reconstructed: Score = (0..problem.slot_count())
            .map(|slot| slot_local(problem, slot, assignment.get(slot)))
            .sum::<Score>()
            + (0..problem.human_count())
                .map(|human| {
                    human_shape(problem, &assignment, human)
                        + human_workload(problem, human, assignment.assigned_minutes(human))
                })
                .sum::<Score>();

        assert_eq!(
            reconstructed,
            evaluate(problem, &assignment),
            "the decomposition the search relies on must be exact"
        );
    }
}
