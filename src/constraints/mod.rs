use std::fmt::Display;

use crate::timerange::TimeRange;
use chrono::{NaiveDate, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

#[macro_use]
mod macros;
mod dayofweek;
mod timeofday;
mod unavailable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Constraint {
    None,
    DayOfWeek(Vec<Weekday>),
    TimeOfDay { start: NaiveTime, end: NaiveTime },
    Unavailable { start: NaiveDate, end: NaiveDate },
}

impl Constraint {
    pub fn flat_map<'a, I: Iterator<Item = TimeRange> + 'a>(
        &self,
        ranges: I,
    ) -> Box<dyn Iterator<Item = TimeRange> + 'a> {
        match self {
            Constraint::None => Box::new(ranges),
            Constraint::DayOfWeek(days) => {
                Box::new(dayofweek::DayOfWeekIterator::new(ranges, days.clone()))
            }
            Constraint::TimeOfDay { start, end } => {
                Box::new(timeofday::TimeOfDayIterator::new(ranges, *start, *end))
            }
            Constraint::Unavailable { start, end } => {
                Box::new(unavailable::UnavailableIterator::new(ranges, *start, *end))
            }
        }
    }
}

impl Display for Constraint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Constraint::None => write!(f, "always available"),
            Constraint::DayOfWeek(days) => {
                write!(f, "available on {:?}", days)
            }
            Constraint::TimeOfDay { start, end } => {
                write!(f, "available between {} and {}", start, end)
            }
            Constraint::Unavailable { start, end } => {
                write!(f, "unavailable from {} to {}", start, end)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_time_of_day_constraint() {
        let constraint = Constraint::TimeOfDay {
            start: NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        };

        let output: Vec<TimeRange> = constraint
            .flat_map(
                vec![TimeRange::new(
                    date_time!(2020, 1, 1),
                    date_time!(2020, 1, 2),
                )]
                .into_iter(),
            )
            .collect();

        let expected = vec![TimeRange::new(
            date_time!(2020, 1, 1, 9, 0, 0),
            date_time!(2020, 1, 1, 17, 0, 0),
        )];

        assert_eq!(output, expected);
    }

    #[test]
    fn test_day_of_week_constraint() {
        let constraint = Constraint::DayOfWeek(vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
        ]);
        
        let output: Vec<TimeRange> = constraint
            .flat_map(
                vec![TimeRange::new(
                    date_time!(2020, 1, 1),
                    date_time!(2020, 1, 2),
                )]
                .into_iter(),
            )
            .collect();

        let expected = vec![TimeRange::new(
            date_time!(2020, 1, 1),
            date_time!(2020, 1, 2),
        )];

        assert_eq!(output, expected);
    }

    #[test]
    fn test_unavailable_constraint() {
        let constraint = Constraint::Unavailable {
            start: date!(2020, 1, 2),
            end: date!(2020, 1, 4),
        };
        
        let output: Vec<TimeRange> = constraint
            .flat_map(
                vec![TimeRange::new(
                    date_time!(2020, 1, 1),
                    date_time!(2020, 1, 7),
                )]
                .into_iter(),
            )
            .collect();

        let expected = vec![TimeRange::new(
            date_time!(2020, 1, 1),
            date_time!(2020, 1, 2),
        ), TimeRange::new(
            date_time!(2020, 1, 4),
            date_time!(2020, 1, 7),
        )];

        assert_eq!(output, expected);
    }
}
