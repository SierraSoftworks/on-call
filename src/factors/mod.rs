use std::collections::HashMap;

use crate::{config::Config, solver::ScheduleSlot, timerange::TimeRange};

mod coverage;
mod length;
mod recency;
mod workload;

pub fn all(config: &Config) -> Vec<Box<dyn Optimizer>> {
    vec![
        coverage::Coverage::init(config),
        length::Length::init(config),
        recency::Recency::init(config),
        workload::Workload::init(config),
    ]
}

#[derive(Debug, Clone, Copy)]
pub struct Cost {
    pub cost: f64,
    pub weight: f64,
}

pub trait Optimizer {
    fn init(config: &Config) -> Box<dyn Optimizer>
    where
        Self: Sized;

    fn name(&self) -> &'static str;

    fn weight(&self) -> f64 {
        1.0
    }

    fn update(&mut self, slot: &ScheduleSlot);

    fn cost(&self, config: &Config, slots_to_fill: &[TimeRange], candidate: &Candidate) -> Option<f64>;

    fn populate(&self, config: &Config, slots_to_fill: &[TimeRange], candidate: &mut Candidate) {
        if let Some(cost) = self.cost(config, slots_to_fill, candidate)
        {
            candidate.add_factor(self.name(), Cost {
                cost,
                weight: self.weight(),
            });
        }
    }
}

pub struct Candidate<'a> {
    pub human: &'a str,
    pub available_slots: Vec<bool>,
    pub factors: HashMap<&'a str, Cost>,
}

impl<'a> std::fmt::Debug for Candidate<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:20} = {:.5} {:?}", self.human, self.cost(), self.factors)
    }
}

impl<'a> Candidate<'a> {
    pub fn new(human: &'a str, available_slots: Vec<bool>) -> Self {
        Self {
            human,
            available_slots,
            factors: HashMap::new(),
        }
    }
    
    pub fn is_available(&self) -> bool {
        self.available_slots.iter().any(|s| *s)
    }

    pub fn add_factor(&mut self, name: &'a str, cost: Cost) -> &mut Self {
        self.factors.insert(name, cost);
        self
    }

    /// Calculates the relative cost used to determine which candidate to assign to a given
    /// on-call slot. This score is intended to be minimized (i.e. low cost = better assignments).
    pub fn cost(&self) -> f64 {
        if self.factors.is_empty() {
            return 0.0;
        }

        let cost: f64 = self.factors.values().map(|c| c.cost * c.weight).sum();
        let weight: f64 = self.factors.values().map(|c| c.weight).sum();

        cost / weight
    }
}