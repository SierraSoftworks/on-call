//! The schedule: what the optimizer produces, and how it round-trips.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::model::problem::HumanIdx;
use crate::model::{Assignment, Problem};
use crate::timerange::TimeRange;

/// One unbroken stretch of on-call cover.
///
/// Internally the schedule is made of atomic slots, which are split wherever
/// anybody's availability changes and are therefore often much shorter than a
/// real shift. Adjacent slots with the same owner are merged back together
/// here, so the output describes shifts as a person would.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Shift {
    #[serde(flatten)]
    pub time: TimeRange,
    pub human: Option<String>,
}

/// A complete schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Schedule {
    pub shifts: Vec<Shift>,
}

impl Schedule {
    /// Renders an assignment as merged shifts.
    ///
    /// Only slots that are genuinely contiguous in time are merged. A Friday
    /// and the following Monday belong to the same *shift* as far as the
    /// optimizer is concerned, but printing them as one range would claim
    /// somebody was on-call over the weekend, so they stay separate rows.
    pub fn from_assignment(problem: &Problem, assignment: &Assignment) -> Self {
        let mut shifts: Vec<Shift> = Vec::new();

        for slot in 0..problem.slot_count() {
            let human = assignment.get(slot).map(|human| problem.humans[human].clone());
            let range = problem.slots[slot];

            match shifts.last_mut() {
                Some(last) if last.human == human && last.time.end == range.start => {
                    last.time.end = range.end;
                }
                _ => shifts.push(Shift { time: range, human }),
            }
        }

        Self { shifts }
    }

    /// Reads a previously written schedule from JSON.
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let file = std::fs::File::open(path)
            .map_err(|err| format!("unable to open baseline {}: {}", path.display(), err))?;

        let shifts: Vec<Shift> = serde_json::from_reader(file)
            .map_err(|err| format!("unable to parse baseline {}: {}", path.display(), err))?;

        Ok(Self { shifts })
    }

    /// Projects this schedule onto a problem's slots.
    ///
    /// A slot takes its baseline owner from whichever shift wholly contains it,
    /// which means a baseline written as merged shifts maps cleanly back onto
    /// finer-grained slots. Entries naming people who are no longer on the team,
    /// or covering times that are no longer scheduled, are ignored: the point of
    /// a baseline is to avoid gratuitous churn, not to resurrect stale data.
    pub fn to_baseline(&self, problem: &Problem) -> (Vec<Option<HumanIdx>>, BaselineReport) {
        let mut baseline = vec![None; problem.slot_count()];
        let mut report = BaselineReport::default();

        // Sorting lets us binary search for the shift covering each slot.
        let mut shifts: Vec<&Shift> = self.shifts.iter().collect();
        shifts.sort_by_key(|shift| shift.time.start);

        for (index, slot) in problem.slots.iter().enumerate() {
            let candidate = shifts.partition_point(|shift| shift.time.start <= slot.start);
            if candidate == 0 {
                report.unmatched_slots += 1;
                continue;
            }

            let shift = shifts[candidate - 1];
            if shift.time.start > slot.start || slot.end > shift.time.end {
                report.unmatched_slots += 1;
                continue;
            }

            let Some(name) = shift.human.as_deref() else {
                continue;
            };

            match problem.human_index(name) {
                Some(human) if problem.is_available(human, index) => {
                    baseline[index] = Some(human);
                    report.matched_slots += 1;
                }
                Some(_) => report.now_unavailable += 1,
                None => report.unknown_people += 1,
            }
        }

        (baseline, report)
    }

    /// Whether any part of the schedule has nobody covering it.
    pub fn has_gaps(&self) -> bool {
        self.shifts.iter().any(|shift| shift.human.is_none())
    }
}

/// What happened when a baseline was projected onto the current problem.
#[derive(Debug, Clone, Copy, Default)]
pub struct BaselineReport {
    /// Slots which took an owner from the baseline.
    pub matched_slots: usize,
    /// Slots the baseline did not cover.
    pub unmatched_slots: usize,
    /// Slots whose baseline owner is no longer available then.
    pub now_unavailable: usize,
    /// Slots naming somebody who is no longer on the team.
    pub unknown_people: usize,
}

impl BaselineReport {
    /// Whether anything about the baseline needed to be discarded.
    pub fn has_warnings(&self) -> bool {
        self.unmatched_slots > 0 || self.now_unavailable > 0 || self.unknown_people > 0
    }
}

impl std::fmt::Display for BaselineReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} slots anchored to the baseline", self.matched_slots)?;

        if self.unmatched_slots > 0 {
            write!(f, ", {} not covered by it", self.unmatched_slots)?;
        }
        if self.now_unavailable > 0 {
            write!(
                f,
                ", {} whose previous owner is no longer available",
                self.now_unavailable
            )?;
        }
        if self.unknown_people > 0 {
            write!(
                f,
                ", {} naming somebody no longer on the team",
                self.unknown_people
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives::testing::fixture;
    use crate::search::construct::construct;

    /// A round-the-clock rota, so consecutive slots really are contiguous and
    /// merging has something to do.
    ///
    /// The third person's half-day availability is what forces the horizon to
    /// be split into multiple slots at all; alice and bob remain available
    /// throughout.
    fn continuous_problem() -> Problem {
        use crate::config::{Config, Human};
        use crate::constraints::Constraint;
        use chrono::Duration;

        let config = Config::for_test(
            Duration::days(1),
            [
                ("alice@example.com".to_string(), Human::default()),
                ("bob@example.com".to_string(), Human::default()),
                (
                    "mornings@example.com".to_string(),
                    Human::default().with_constraints(vec![Constraint::TimeOfDay {
                        start: time!(0, 0),
                        end: time!(12, 0),
                    }]),
                ),
            ]
            .into_iter()
            .collect(),
        );

        Problem::build(
            &config,
            date_time!(2023, 1, 2),
            date_time!(2023, 1, 2) + Duration::days(10),
        )
        .unwrap()
    }

    #[test]
    fn contiguous_slots_with_the_same_owner_become_one_shift() {
        let problem = continuous_problem();
        let mut assignment = Assignment::empty(&problem);

        let midpoint = problem.slot_count() / 2;
        for slot in 0..midpoint {
            assignment.assign(&problem, slot, Some(0));
        }
        for slot in midpoint..problem.slot_count() {
            assignment.assign(&problem, slot, Some(1));
        }

        let schedule = Schedule::from_assignment(&problem, &assignment);

        assert_eq!(schedule.shifts.len(), 2);
        assert_eq!(schedule.shifts[0].time.start, problem.slots[0].start);
        assert_eq!(schedule.shifts[0].time.end, problem.slots[midpoint - 1].end);
        assert_eq!(
            schedule.shifts[0].human.as_deref(),
            Some("alice@example.com")
        );
        assert_eq!(schedule.shifts[1].time.start, problem.slots[midpoint].start);
    }

    #[test]
    fn slots_separated_in_time_are_never_merged() {
        // The weekday fixture runs 09:00-17:00, so consecutive slots are a day
        // apart. Merging them would print a shift spanning the night, claiming
        // cover that nobody is actually providing.
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);
        for slot in 0..problem.slot_count() {
            assignment.assign(problem, slot, Some(0));
        }

        let schedule = Schedule::from_assignment(problem, &assignment);

        assert_eq!(
            schedule.shifts.len(),
            problem.slot_count(),
            "non-contiguous slots must stay separate even with the same owner"
        );

        for shift in schedule.shifts.iter() {
            assert_eq!(
                shift.time.len().num_hours(),
                8,
                "each shift should cover only the hours actually on the rota"
            );
        }
    }

    #[test]
    fn merged_shifts_never_overlap_or_reorder() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let schedule = Schedule::from_assignment(problem, &construct(problem));

        for pair in schedule.shifts.windows(2) {
            assert!(
                pair[0].time.end <= pair[1].time.start,
                "{:?} overlaps {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn uncovered_stretches_are_reported_as_gaps() {
        let problem = continuous_problem();
        let assignment = Assignment::empty(&problem);
        let schedule = Schedule::from_assignment(&problem, &assignment);

        assert_eq!(schedule.shifts.len(), 1);
        assert!(schedule.has_gaps());
        assert_eq!(schedule.shifts[0].human, None);
    }

    #[test]
    fn a_full_schedule_has_no_gaps() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let schedule = Schedule::from_assignment(problem, &construct(problem));

        assert!(!schedule.has_gaps());
    }

    #[test]
    fn a_schedule_round_trips_through_a_baseline() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let assignment = construct(problem);
        let schedule = Schedule::from_assignment(problem, &assignment);

        let (baseline, report) = schedule.to_baseline(problem);

        assert_eq!(baseline, assignment.slots().to_vec());
        assert_eq!(report.matched_slots, problem.slot_count());
        assert!(!report.has_warnings());
    }

    #[test]
    fn a_baseline_round_trips_through_json() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let schedule = Schedule::from_assignment(problem, &construct(problem));

        let json = serde_json::to_string(&schedule).unwrap();
        let parsed: Schedule = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed, schedule);
        assert_eq!(parsed.to_baseline(problem).0, schedule.to_baseline(problem).0);
    }

    #[test]
    fn unknown_people_in_a_baseline_are_ignored_and_reported() {
        let fixture = fixture();
        let problem = &fixture.problem;

        let schedule = Schedule {
            shifts: vec![Shift {
                time: TimeRange::new(problem.slots[0].start, problem.slots[0].end),
                human: Some("departed@example.com".to_string()),
            }],
        };

        let (baseline, report) = schedule.to_baseline(problem);

        assert!(baseline.iter().all(|entry| entry.is_none()));
        assert_eq!(report.unknown_people, 1);
        assert!(report.has_warnings());
    }

    #[test]
    fn baseline_entries_outside_the_horizon_are_ignored() {
        let fixture = fixture();
        let problem = &fixture.problem;

        let schedule = Schedule {
            shifts: vec![Shift {
                time: TimeRange::new(date_time!(2019, 1, 1), date_time!(2019, 1, 2)),
                human: Some("alice@example.com".to_string()),
            }],
        };

        let (baseline, report) = schedule.to_baseline(problem);

        assert!(baseline.iter().all(|entry| entry.is_none()));
        assert_eq!(report.unmatched_slots, problem.slot_count());
    }

    #[test]
    fn a_merged_baseline_shift_anchors_every_slot_it_spans() {
        let fixture = fixture();
        let problem = &fixture.problem;

        let schedule = Schedule {
            shifts: vec![Shift {
                time: TimeRange::new(problem.slots[0].start, problem.slots[4].end),
                human: Some("bob@example.com".to_string()),
            }],
        };

        let (baseline, report) = schedule.to_baseline(problem);
        let bob = problem.human_index("bob@example.com").unwrap();

        for (slot, owner) in baseline.iter().enumerate().take(5) {
            assert_eq!(*owner, Some(bob), "slot {slot} should be anchored");
        }
        assert_eq!(report.matched_slots, 5);
    }

    #[test]
    fn a_baseline_owner_who_became_unavailable_is_dropped() {
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
            date_time!(2023, 1, 2) + Duration::days(7),
        )
        .unwrap();

        // Alice covered the Tuesday last time, but can now only work Mondays.
        let tuesday = problem
            .slots
            .iter()
            .position(|slot| {
                use chrono::Datelike;
                slot.start.date().weekday() == Weekday::Tue
            })
            .unwrap();

        let schedule = Schedule {
            shifts: vec![Shift {
                time: problem.slots[tuesday],
                human: Some("alice@example.com".to_string()),
            }],
        };

        let (baseline, report) = schedule.to_baseline(&problem);

        assert_eq!(baseline[tuesday], None);
        assert_eq!(report.now_unavailable, 1);
    }
}
