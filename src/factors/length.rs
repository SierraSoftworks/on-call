use chrono::Duration;

use crate::{config::Config, solver::ScheduleSlot, timerange::TimeRange};

use super::{Optimizer, Candidate};

pub struct Length {
    current: Option<(String, Duration)>,
    limit: Duration,
}

impl Optimizer for Length {
    fn init(config: &Config) -> Box<dyn Optimizer>
    where
        Self: Sized {
        Box::new(Self {
            current: None,
            limit: config.shift_length,
        })
    }

    fn name(&self) -> &'static str {
        "length"
    }

    fn weight(&self) -> f64 {
        100.0
    }

    fn update(&mut self, slot: &ScheduleSlot) {
        let new_length = if let Some((person, length)) = self.current.as_ref() {
            if Some(person) == slot.human.as_ref() {
                *length + slot.time.len()
            } else {
                slot.time.len()
            }
        } else {
            slot.time.len()
        };

        self.current = slot.human.clone().map(|h| (h, new_length));
    }

    fn cost(&self, _config: &Config, _slots_to_fill: &[TimeRange], candidate: &Candidate) -> Option<f64> {
        match self.current.as_ref() {
            Some((person, length)) if person == candidate.human => {
                let remaining = self.limit - *length;
                Some(1.0 - (remaining.num_hours() as f64 / self.limit.num_hours() as f64))
            },
            Some(_) => Some(0.0),
            None => None,
        }
    }
}