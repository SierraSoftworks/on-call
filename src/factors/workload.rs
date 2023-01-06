use std::collections::HashMap;

use chrono::Duration;

use super::{Optimizer, Config, ScheduleSlot, TimeRange, Candidate};


pub struct Workload {
    workload: HashMap<String, Duration>,
}

impl Optimizer for Workload {
    fn init(config: &Config) -> Box<dyn Optimizer>
    where
        Self: Sized {
        let mut workload = HashMap::new();
        for human in config.humans.iter() {
            workload.insert(human.0.clone(), Duration::zero());
        }

        Box::new(Self { workload })
    }

    fn name(&self) -> &'static str {
        "workload"
    }

    fn weight(&self) -> f64 {
        5.0
    }

    fn update(&mut self, slot: &ScheduleSlot) {
        if let Some(human) = slot.human.as_deref() {
            let workload = self.workload.entry(human.to_string()).or_insert_with(Duration::zero);
            *workload = *workload + slot.time.len();
        }
    }

    fn cost(&self, _config: &Config, _slots_to_fill: &[TimeRange], candidate: &Candidate) -> Option<f64> {
        let min = self.workload.values().min().copied().unwrap_or_else(Duration::zero);
        let max = self.workload.values().max().copied().unwrap_or_else(Duration::max_value);

        let range = max - min;

        if range.is_zero() {
            return None;
        }

        self.workload.get(candidate.human).map(|&workload| {
            (workload - min).num_seconds() as f64 / range.num_seconds() as f64
        })
    }
}
