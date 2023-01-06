use std::collections::HashMap;

use chrono::{NaiveDateTime};

use super::{Optimizer, Config, ScheduleSlot, TimeRange, Candidate};


pub struct Recency {
    recency: HashMap<String, NaiveDateTime>,
}

impl Optimizer for Recency {
    fn init(config: &Config) -> Box<dyn Optimizer>
    where
        Self: Sized {
        let mut recency = HashMap::new();
        for human in config.humans.iter() {
            recency.insert(human.0.clone(), NaiveDateTime::from_timestamp_opt(0, 0).unwrap());
        }

        Box::new(Self { recency })
    }

    fn name(&self) -> &'static str {
        "recency"
    }

    fn weight(&self) -> f64 {
        1.0
    }

    fn update(&mut self, slot: &ScheduleSlot) {
        if let Some(human) = slot.human.as_deref() {
            self.recency.insert(human.to_string(), slot.time.end);
        }
    }

    fn cost(&self, _config: &Config, _slots_to_fill: &[TimeRange], candidate: &Candidate) -> Option<f64> {
        let min = self.recency.values().min().copied().unwrap_or_else(|| NaiveDateTime::from_timestamp_opt(0, 0).unwrap());
        let max = self.recency.values().max().copied().unwrap_or_else(|| NaiveDateTime::from_timestamp_opt(0, 0).unwrap());

        let range = max - min;

        if range.is_zero() {
            return None;
        }

        self.recency.get(candidate.human).map(|&recency| {
            let delta = recency - min;
            delta.num_seconds() as f64 / range.num_seconds() as f64
        })
    }
}
