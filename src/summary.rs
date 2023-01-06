use std::{collections::HashMap, fmt::Display};

use chrono::Duration;

use crate::solver::ScheduleSlot;

pub struct Summary {
    workload: HashMap<String, Duration>,
    longest_shift: HashMap<String, Duration>,
    shift_length_histogram: HashMap<i64, usize>,
}

impl<T: AsRef<[ScheduleSlot]>> From<T> for Summary {
    fn from(schedule: T) -> Self {
        let mut workload = map! {};
        let mut longest_shift = map! {};
        let mut shift_length_histogram = map!{};

        let mut current_on_call: Option<(String, Duration)> = None;

        for slot in schedule.as_ref() {
            let human = slot.human.as_deref().unwrap_or("UNASSIGNED");

            workload
                .entry(human.to_string())
                .and_modify(|e| *e = *e + slot.time.len())
                .or_insert_with(|| slot.time.len());

            let shift_len = if let Some((person, length)) = current_on_call {
                let new_length = if person == human {
                    length + slot.time.len()
                } else {
                    shift_length_histogram.entry(length.num_hours()).and_modify(|e| *e += 1).or_insert(1);
                    slot.time.len()
                };

                current_on_call = Some((human.to_string(), new_length));

                new_length
            } else {
                current_on_call = Some((human.to_string(), slot.time.len()));
                slot.time.len()
            };

            longest_shift
                .entry(human.to_string())
                .and_modify(|e| {
                    if *e < shift_len {
                        *e = shift_len;
                    }
                })
                .or_insert(shift_len);
        }

        Self {
            workload,
            longest_shift,
            shift_length_histogram
        }
    }
}

impl Summary {
    fn stats<I: IntoIterator<Item = Duration>>(items: I) -> (i64, i64, i64) {
        let mut items: Vec<_> = items.into_iter().collect();
        items.sort();

        let min = items.first().copied().unwrap_or_else(Duration::zero);
        let max = items.last().copied().unwrap_or_else(Duration::zero);
        let avg = items.iter().sum::<Duration>() / items.len() as i32;

        (min.num_hours(), max.num_hours(), avg.num_hours())
    }
}

impl Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut workload: Vec<_> = self.workload.iter().collect();
        workload.sort_by_key(|(_, v)| -v.num_hours());
        let (wl_min, wl_max, wl_avg) = Self::stats(self.workload.values().copied());

        let mut longest_shift: Vec<_> = self.longest_shift.iter().collect();
        longest_shift.sort_by_key(|(_, v)| -v.num_hours());
        let (ls_min, ls_max, ls_avg) = Self::stats(self.longest_shift.values().copied());

        writeln!(f, "Workload: (min: {wl_min}, avg: {wl_avg}, max: {wl_max})")?;
        for (human, workload) in workload {
            writeln!(f, "  {}: {} hours", human, workload.num_hours())?;
        }

        writeln!(f)?;
        writeln!(f, "Longest shift: (min: {ls_min}, avg: {ls_avg}, max: {ls_max})")?;
        for (human, shift) in longest_shift {
            writeln!(f, "  {}: {} hours", human, shift.num_hours())?;
        }

        writeln!(f)?;
        writeln!(f, "Shift length histogram:")?;
        for (length, count) in self.shift_length_histogram.iter() {
            writeln!(f, " {} | {} hours", count, length)?;
        }

        Ok(())
    }
}
