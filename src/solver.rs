use crate::{
    config::Config,
    constraints::Constraint,
    factors::{self, Candidate, Optimizer},
    timerange::TimeRange,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScheduleSlot {
    #[serde(flatten)]
    pub time: TimeRange,
    pub human: Option<String>,
}

pub struct Scheduler<'a> {
    config: &'a Config,
    factors: Vec<Box<dyn Optimizer>>,
    debug: bool,
}

impl<'a> Scheduler<'a> {
    pub fn new(config: &'a Config) -> Self {
        let factors = factors::all(config);

        Self {
            config,
            factors,
            debug: false,
        }
    }

    pub fn with_debug(self) -> Self {
        Self {
            debug: true,
            ..self
        }
    }

    pub fn schedule(&mut self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<ScheduleSlot> {
        let mut slots = Vec::new();

        // TODO: In future, accept a weights input to allow iterative scheduling while remaining fair

        let mut slots_to_fill = vec![TimeRange::new(start.naive_utc(), end.naive_utc())];
        for constraint in &self.config.constraints {
            let mut new_ranges = vec![];
            for range in slots_to_fill {
                let suitable_ranges = constraint.suitable_ranges(range);
                new_ranges.extend(suitable_ranges);
            }
            slots_to_fill = new_ranges;
        }

        for slot in slots_to_fill.chunks(self.config.shift_length.abs().num_days() as usize) {
            let mut rotation_assignments = self.schedule_rotation(slot);

            for assignment in rotation_assignments.iter() {
                for factor in self.factors.iter_mut() {
                    factor.update(assignment);
                }
            }

            slots.append(&mut rotation_assignments);
        }

        slots
    }

    fn schedule_rotation(&self, slots_to_fill: &[TimeRange]) -> Vec<ScheduleSlot> {
        if slots_to_fill.is_empty() {
            return vec![];
        }

        let mut candidates = self
            .config
            .humans
            .iter()
            .map(|(human, constraints)| {
                let available_slots = self.possible_coverage(constraints, slots_to_fill);

                let mut candidate = Candidate::new(human, available_slots);
                for factor in self.factors.iter() {
                    factor.populate(self.config, slots_to_fill, &mut candidate);
                }

                candidate
            })
            .filter(|candidate| candidate.is_available())
            .collect::<Vec<_>>();

        candidates.sort_by_key(|candidate| candidate.human);
        candidates.sort_by_key(|candidate| (i64::max_value() as f64 * candidate.cost()) as i64);

        if self.debug {
            eprintln!();
            eprintln!("Candidates for rotation {:?}:", slots_to_fill);
            for (priority, candidate) in candidates.iter().enumerate() {
                eprintln!(" {priority}. {candidate:?}");
            }
        }

        // We then assign slots to the least busy person who can cover them on a first-come, first-serve basis until all slots are filled and/or all candidates have been exhausted
        let mut slot_assignments: Vec<Option<String>> =
            slots_to_fill.iter().map(|_| None).collect();
        for candidate in candidates {
            for (index, assignment) in slot_assignments.iter_mut().enumerate() {
                if assignment.is_none() && candidate.available_slots[index] {
                    *assignment = Some(candidate.human.to_string());
                }
            }

            if slot_assignments
                .iter()
                .all(|assignment| assignment.is_some())
            {
                break;
            }
        }

        slot_assignments
            .iter()
            .zip(slots_to_fill.iter())
            .map(|(assignment, slot)| ScheduleSlot {
                time: *slot,
                human: assignment.clone(),
            })
            .collect()
    }

    /// Returns a vector of booleans indicating whether each slot can be covered by the given constraints.
    fn possible_coverage(&self, constraints: &[Constraint], slots: &[TimeRange]) -> Vec<bool> {
        let mut slots_to_fill = slots.to_vec();
        for constraint in constraints {
            let mut new_ranges = vec![];
            for range in slots_to_fill {
                let suitable_ranges = constraint.suitable_ranges(range);
                new_ranges.extend(suitable_ranges);
            }
            slots_to_fill = new_ranges;
        }

        slots
            .iter()
            .map(|slot| slots_to_fill.binary_search(slot).is_ok())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, NaiveDate, NaiveTime};

    use crate::summary;

    use super::*;

    #[test]
    fn test_schedule() {
        let config = Config {
            shift_length: Duration::days(1),
            constraints: vec![
                Constraint::DayOfWeek(vec![
                    chrono::Weekday::Mon,
                    chrono::Weekday::Tue,
                    chrono::Weekday::Wed,
                    chrono::Weekday::Thu,
                    chrono::Weekday::Fri,
                ]),
                Constraint::TimeRange {
                    start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    end: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                },
            ],
            humans: map![
                "alice@example.com" => vec![
                    Constraint::DayOfWeek(vec![chrono::Weekday::Mon, chrono::Weekday::Wed, chrono::Weekday::Fri]),
                ],
                "bob@example.com" => vec![
                    Constraint::Unavailable { start: NaiveDate::from_ymd_opt(2022, 12, 23).unwrap(), end: NaiveDate::from_ymd_opt(2023, 1, 2).unwrap() }
                ],
                "claire@example.com" => vec![
                    Constraint::None,
                ]
            ],
        };

        let schedule = Scheduler::new(&config).schedule(
            NaiveDate::from_ymd_opt(2023, 1, 1)
                .unwrap()
                .and_time(NaiveTime::default())
                .and_local_timezone(Utc)
                .unwrap(),
                NaiveDate::from_ymd_opt(2023, 12, 31)
                .unwrap()
                .and_time(NaiveTime::default())
                .and_local_timezone(Utc)
                .unwrap(),
        );

        assert!(
            schedule.iter().all(|slot| slot.human.is_some()),
            "all slots must be filled"
        );

        let summary = summary::Summary::from(&schedule);
        
        let (min, _, max) = summary.workload_stats();
        assert!(
            (max - min) <= 16,
            "assignments should be within 2 shifts of each other"
        );

        let (_, _, max) = summary.longest_shift_stats();
        assert_eq!(
            max, 8, "the longest shift should be 8 hours",
        );
    }
}
