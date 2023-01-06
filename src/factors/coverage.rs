use chrono::Duration;

use super::{Optimizer, Config, ScheduleSlot, TimeRange, Candidate};

pub struct Coverage;

impl Optimizer for Coverage {
    fn init(_config: &Config) -> Box<dyn Optimizer>
    where
        Self: Sized {
        Box::new(Self)
    }

    fn name(&self) -> &'static str {
        "coverage"
    }

    fn update(&mut self, _slot: &ScheduleSlot) {
        // No-op
    }

    fn weight(&self) -> f64 {
        5.0
    }

    fn cost(&self, _config: &Config, slots_to_fill: &[TimeRange], candidate: &Candidate) -> Option<f64> {
        let total_duration = slots_to_fill.iter().map(|s| s.len()).sum::<Duration>();
        let covered_duration = slots_to_fill.iter().zip(candidate.available_slots.iter()).filter(|(_, available)| **available).map(|(slot, _)| slot.len()).sum::<Duration>();

        if covered_duration.is_zero() {
            return None;
        }

        Some(1.0 - (covered_duration.num_seconds() as f64 / total_duration.num_seconds() as f64))
    }
}