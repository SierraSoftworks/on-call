//! The optimizer: turning a problem into a schedule.

pub mod construct;
pub mod incremental;
pub mod lahc;
pub mod lns;
pub mod moves;

use crate::model::{Assignment, Problem};

pub use lahc::{Options, Statistics};

/// Builds an initial schedule and then optimizes it.
pub fn solve(problem: &Problem, options: &Options) -> (Assignment, Statistics) {
    let initial = construct::construct(problem);
    lahc::optimize(problem, initial, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Human};
    use crate::constraints::Constraint;
    use crate::model::problem::HumanIdx;
    use crate::model::Score;
    use crate::objectives;
    use crate::objectives::testing::fixture;
    use chrono::{Duration, Weekday};

    #[test]
    fn solve_produces_a_covered_schedule() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let options = Options {
            steps: 50_000,
            seed: 1,
            ..Options::default()
        };

        let (assignment, statistics) = solve(problem, &options);

        assert_eq!(assignment.unassigned_count(), 0);
        assert!(statistics.best.is_feasible());
        assert_eq!(statistics.best, objectives::evaluate(problem, &assignment));
    }

    #[test]
    fn solve_is_reproducible() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let options = Options {
            steps: 20_000,
            seed: 7,
            ..Options::default()
        };

        let (first, _) = solve(problem, &options);
        let (second, _) = solve(problem, &options);

        assert_eq!(first.slots(), second.slots());
    }

    /// Builds an instance small enough to solve by exhaustive enumeration.
    fn tiny_problem(days: i64, humans: Vec<(&str, Human)>) -> Problem {
        let config = Config::for_test(
            Duration::days(1),
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

        Problem::build(
            &config,
            date_time!(2023, 1, 2),
            date_time!(2023, 1, 2) + Duration::days(days),
        )
        .unwrap()
    }

    /// Exhaustively enumerates every possible schedule and returns the best
    /// score. Only tractable for a handful of slots.
    fn brute_force(problem: &Problem) -> Score {
        let slots = problem.slot_count();
        let humans = problem.human_count();
        let options = humans + 1; // +1 for "unassigned"

        let total = (options as u64).pow(slots as u32);
        assert!(total <= 2_000_000, "instance is too large to enumerate");

        let mut best = Score::new(i64::MAX, i64::MAX);
        let mut assignment = Assignment::empty(problem);

        for encoded in 0..total {
            let mut value = encoded;
            let mut legal = true;

            for slot in 0..slots {
                let choice = (value % options as u64) as usize;
                value /= options as u64;

                let assignee: Option<HumanIdx> = if choice == humans {
                    None
                } else {
                    Some(choice)
                };

                if let Some(human) = assignee {
                    if !problem.is_available(human, slot) {
                        legal = false;
                        break;
                    }
                }

                assignment.assign(problem, slot, assignee);
            }

            if !legal {
                // Reset so the next candidate starts from a clean slate.
                for slot in 0..slots {
                    assignment.assign(problem, slot, None);
                }
                continue;
            }

            best = best.min(objectives::evaluate(problem, &assignment));
        }

        best
    }

    #[test]
    fn the_search_finds_the_true_optimum_on_a_tiny_instance() {
        let problem = tiny_problem(
            7,
            vec![
                ("alice@example.com", Human::default()),
                ("bob@example.com", Human::default()),
                ("claire@example.com", Human::default()),
            ],
        );

        // Five weekday slots, three people: 4^5 = 1024 candidate schedules.
        assert_eq!(problem.slot_count(), 5);

        let optimum = brute_force(&problem);
        let (assignment, statistics) = solve(
            &problem,
            &Options {
                steps: 50_000,
                seed: 3,
                ..Options::default()
            },
        );

        assert_eq!(
            statistics.best, optimum,
            "the search settled for {} when {} was reachable",
            statistics.best, optimum
        );
        assert_eq!(objectives::evaluate(&problem, &assignment), optimum);
    }

    #[test]
    fn the_search_finds_the_true_optimum_with_awkward_availability() {
        let problem = tiny_problem(
            7,
            vec![
                (
                    "alice@example.com",
                    Human::default().with_constraints(vec![Constraint::DayOfWeek(vec![
                        Weekday::Mon,
                        Weekday::Tue,
                    ])]),
                ),
                (
                    "bob@example.com",
                    Human::default().with_constraints(vec![Constraint::Unavailable {
                        start: date!(2023, 1, 4),
                        end: date!(2023, 1, 6),
                    }]),
                ),
                ("claire@example.com", Human::default()),
            ],
        );

        let optimum = brute_force(&problem);
        let (_, statistics) = solve(
            &problem,
            &Options {
                steps: 50_000,
                seed: 4,
                ..Options::default()
            },
        );

        assert_eq!(statistics.best, optimum);
    }

    #[test]
    fn the_search_finds_the_true_optimum_when_a_slot_is_uncoverable() {
        // Only Alice exists and she cannot work Wednesdays, so the optimum
        // still leaves a gap. The search should find the best schedule that
        // exists rather than thrashing against the impossible slot.
        let problem = tiny_problem(
            7,
            vec![(
                "alice@example.com",
                Human::default().with_constraints(vec![Constraint::DayOfWeek(vec![
                    Weekday::Mon,
                    Weekday::Tue,
                    Weekday::Thu,
                    Weekday::Fri,
                ])]),
            )],
        );

        let optimum = brute_force(&problem);
        let (assignment, statistics) = solve(
            &problem,
            &Options {
                steps: 20_000,
                seed: 5,
                ..Options::default()
            },
        );

        assert_eq!(statistics.best, optimum);
        assert!(
            !statistics.best.is_feasible(),
            "an uncoverable slot should be reported as an unmet hard constraint"
        );
        assert_eq!(assignment.unassigned_count(), 1);
    }

    #[test]
    fn hard_rules_are_satisfied_when_they_can_be() {
        let fixture = crate::objectives::testing::fixture_hard();
        let mut problem = fixture.problem;
        problem.min_rest_minutes = Some(24 * 60);
        problem.max_consecutive_minutes = Some(3 * 8 * 60);

        let (assignment, _) = solve(
            &problem,
            &Options {
                steps: 300_000,
                seed: 6,
                ..Options::default()
            },
        );

        // Verify the rules directly against the schedule rather than trusting
        // the score, so a scoring bug cannot mask a violation.
        for human in 0..problem.human_count() {
            let runs: Vec<_> = assignment.slots_of(human).runs().collect();

            for &(start, end) in runs.iter() {
                assert!(
                    problem.covered_minutes(start, end) <= 3 * 8 * 60,
                    "{} worked a shift longer than the cap",
                    problem.humans[human]
                );
            }

            for pair in runs.windows(2) {
                assert!(
                    problem.gap_minutes(pair[0].1, pair[1].0) >= 24 * 60,
                    "{} did not get the minimum rest",
                    problem.humans[human]
                );
            }
        }
    }
}
