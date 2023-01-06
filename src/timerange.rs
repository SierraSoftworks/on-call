use std::fmt::Display;

use chrono::{NaiveDateTime, Duration};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: NaiveDateTime,
    pub end: NaiveDateTime,
}

#[allow(unused)]
impl TimeRange {
    pub fn new(start: NaiveDateTime, end: NaiveDateTime) -> Self {
        Self {
            start,
            end,
        }
    }

    pub fn is_zero(&self) -> bool {
        self.start == self.end
    }

    pub fn contains(&self, time: NaiveDateTime) -> bool {
        time >= self.start && time <= self.end
    }

    pub fn len(&self) -> Duration {
        self.end - self.start
    }

    pub fn intersection(&self, other: &TimeRange) -> Option<TimeRange> {
        if self.start > other.end || self.end < other.start {
            None
        } else {
            Some(TimeRange {
                start: self.start.max(other.start),
                end: self.end.min(other.end),
            })
        }
    }

    pub fn union(&self, other: &TimeRange) -> Option<TimeRange> {
        if self.start > other.end || self.end < other.start {
            None
        } else {
            Some(TimeRange {
                start: self.start.min(other.start),
                end: self.end.max(other.end),
            })
        }
    }

    pub fn contiguous(&self, other: &TimeRange) -> bool {
        self.start <= other.end && self.end >= other.start
    }
}

impl Display for TimeRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} - {}", self.start, self.end)
    }
}