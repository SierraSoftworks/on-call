use std::fmt::Display;

use chrono::{Duration, Weekday, DurationRound, Datelike, NaiveTime, NaiveDate};
use serde::{Serialize, Deserialize};
use crate::timerange::TimeRange;
 
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Constraint {
    None,
    DayOfWeek(Vec<Weekday>),
    TimeRange {
        start: NaiveTime,
        end: NaiveTime,
    },
    Unavailable {
        start: NaiveDate,
        end: NaiveDate,
    }
}

impl Constraint {
    pub fn suitable_ranges(&self, range: TimeRange) -> Vec<TimeRange> {
        match self {
            Constraint::None => {
                vec![range]
            },
            Constraint::TimeRange { start, end } => {
                let start_aligned = range.start.date();

                let mut ranges = vec![];
                let mut current = start_aligned;
                while current <= range.end.date() {
                    let current_range = TimeRange::new(current.and_time(*start), current.and_time(*end));
                    let intersection = range.intersection(&current_range);
                    if let Some(intersection) = intersection {
                        if intersection.is_zero() {
                            continue;
                        }

                        ranges.push(intersection);
                    }

                    current += Duration::days(1);
                }

                ranges
            },
            Constraint::DayOfWeek(days) => {
                let start_aligned = range.start.duration_trunc(Duration::days(1)).unwrap();

                let mut ranges = vec![];
                let mut current = start_aligned;
                while current < range.end {
                    if days.contains(&current.weekday()) {
                        let current_range = TimeRange::new(current, current + Duration::days(1));
                        let intersection = range.intersection(&current_range);
                        if let Some(intersection) = intersection {
                            if intersection.is_zero() {
                                continue;
                            }

                            ranges.push(intersection);
                        }
                    }

                    current += Duration::days(1);
                }

                ranges
            }
            Constraint::Unavailable { start, end } => {
                let unavailable = TimeRange::new(start.and_hms_opt(0, 0, 0).unwrap(), end.and_hms_opt(23, 59, 59).unwrap());
                let intersection = range.intersection(&unavailable);
                if let Some(intersection) = intersection {
                    if intersection.is_zero() {
                        vec![range]
                    } else {
                        vec![TimeRange::new(range.start, intersection.start), TimeRange::new(intersection.end, range.end)]
                    }
                } else {
                    vec![range]
                }
            },
        }
    }
}

impl Display for Constraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Constraint::None => write!(f, "always available"),
            Constraint::DayOfWeek(days) => {
                let mut days_str = String::new();
                for day in days {
                    days_str.push_str(&format!("{:?}, ", day));
                }

                write!(f, "available on {}", days_str)
            },
            Constraint::TimeRange { start, end } => {
                write!(f, "available between {} and {}", start, end)
            },
            Constraint::Unavailable { start, end } => {
                write!(f, "unavailable from {} to {}", start, end)
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::NaiveDateTime;

    use super::*;

    #[test]
    fn test_time_of_day_constraint() {
        let constraint = Constraint::TimeRange {
            start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end:  NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        };

        let range = TimeRange::new(
            NaiveDateTime::from_str("2020-01-01T00:00:00").expect("a valid start time"),
            NaiveDateTime::from_str("2020-01-02T00:00:00").expect("a valid end time"),
        );

        let ranges = constraint.suitable_ranges(range);
        assert_eq!(ranges.len(), 1);

        assert_eq!(ranges[0].start, NaiveDateTime::from_str("2020-01-01T09:00:00").unwrap());
        assert_eq!(ranges[0].end, NaiveDateTime::from_str("2020-01-01T17:00:00").unwrap());
    }

    #[test]
    fn test_day_of_week_constraint() {
        let constraint = Constraint::DayOfWeek(vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri]);

        let range = TimeRange::new(
            NaiveDateTime::from_str("2020-01-01T00:00:00").expect("a valid start time"),
            NaiveDateTime::from_str("2020-01-02T00:00:00").expect("a valid end time"),
        );

        let ranges = constraint.suitable_ranges(range);
        assert_eq!(ranges.len(), 1);

        assert_eq!(ranges[0].start, NaiveDateTime::from_str("2020-01-01T00:00:00").unwrap());
        assert_eq!(ranges[0].end, NaiveDateTime::from_str("2020-01-02T00:00:00").unwrap());
    }

    #[test]
    fn test_unavailable_constraint() {
        let constraint = Constraint::Unavailable {
            start: NaiveDate::from_str("2020-01-02").expect("a valid start time"),
            end: NaiveDate::from_str("2020-01-04").expect("a valid end time"),
        };

        let range = TimeRange::new(
            NaiveDateTime::from_str("2020-01-01T00:00:00").expect("a valid start time"),
            NaiveDateTime::from_str("2020-01-07T00:00:00").expect("a valid end time"),
        );

        let ranges = constraint.suitable_ranges(range);
        assert_eq!(ranges.len(), 2);

        assert_eq!(ranges[0].start, NaiveDateTime::from_str("2020-01-01T00:00:00").unwrap());
        assert_eq!(ranges[0].end, NaiveDateTime::from_str("2020-01-02T00:00:00").unwrap());

        assert_eq!(ranges[1].start, NaiveDateTime::from_str("2020-01-04T23:59:59").unwrap());
        assert_eq!(ranges[1].end, NaiveDateTime::from_str("2020-01-07T00:00:00").unwrap());
    }
}