use chrono::{Datelike, Duration, DurationRound, Weekday};

use crate::timerange::TimeRange;

constraint_iterator!(
    DayOfWeekIterator(days: Vec<Weekday>) = |self, range| {
        let mut result = Vec::new();

        let mut current_start = range.start;
        let mut next_range: Option<TimeRange> = None;

        while current_start <= range.end {
            if self.days.contains(&current_start.weekday()) {
                let start = current_start.duration_trunc(Duration::days(1)).unwrap();
                let end = start + Duration::days(1);
                let current_range = TimeRange::new(start.max(range.start), end.min(range.end));

                match next_range {
                    Some(next) if current_range.is_zero() && next.is_zero() => {
                        next_range = None
                    },
                    Some(next) if current_range.is_zero() => {
                        next_range = None;
                        result.push(next);
                    },
                    Some(next) => {
                        if let Some(union) = current_range.union(&next) {
                            next_range = Some(union);
                        } else {
                            result.push(next);
                            next_range = Some(current_range);
                        }
                    },
                    _ if current_range.is_zero() => {
                        next_range = None;
                    },
                    _ => {
                        next_range = Some(current_range);
                    }
                }
            }

            current_start += chrono::Duration::days(1);
        }

        if let Some(next_range) = next_range {
            result.push(next_range);
        }

        result
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alternating_days() {
        let output: Vec<TimeRange> = DayOfWeekIterator::new(
            vec![TimeRange::new(
                date_time!(2020, 1, 1),
                date_time!(2020, 1, 8),
            )]
            .into_iter(),
            vec![Weekday::Mon, Weekday::Wed, Weekday::Fri],
        )
        .collect();

        let expected = vec![
            TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 2)),
            TimeRange::new(date_time!(2020, 1, 3), date_time!(2020, 1, 4)),
            TimeRange::new(date_time!(2020, 1, 6), date_time!(2020, 1, 7)),
        ];

        assert_eq!(
            output, expected,
            "The iterator should generate the correct sequence of ranges"
        );
    }

    #[test]
    fn test_contiguous_days() {
        let output: Vec<TimeRange> = DayOfWeekIterator::new(
            vec![TimeRange::new(
                date_time!(2020, 1, 1),
                date_time!(2020, 1, 8),
            )]
            .into_iter(),
            vec![Weekday::Sat, Weekday::Sun],
        )
        .collect();

        let expected = vec![TimeRange::new(
            date_time!(2020, 1, 4),
            date_time!(2020, 1, 6),
        )];

        assert_eq!(
            output, expected,
            "The iterator should generate the correct sequence of ranges"
        );
    }

    #[test]
    fn filtered_ranges_do_not_truncate_the_iterator() {
        // Regression: `segment` returning an empty vec used to end the
        // iterator, so the first non-matching range silently discarded every
        // range after it. Here Tuesday and Thursday are filtered out, and
        // Wednesday and Friday must still come through.
        let days: Vec<TimeRange> = (2..=6)
            .map(|day| {
                TimeRange::new(
                    date_time!(2023, 1, day, 9, 0, 0),
                    date_time!(2023, 1, day, 17, 0, 0),
                )
            })
            .collect();

        let output: Vec<TimeRange> = DayOfWeekIterator::new(
            days.into_iter(),
            vec![Weekday::Mon, Weekday::Wed, Weekday::Fri],
        )
        .collect();

        assert_eq!(
            output,
            vec![
                TimeRange::new(date_time!(2023, 1, 2, 9, 0, 0), date_time!(2023, 1, 2, 17, 0, 0)),
                TimeRange::new(date_time!(2023, 1, 4, 9, 0, 0), date_time!(2023, 1, 4, 17, 0, 0)),
                TimeRange::new(date_time!(2023, 1, 6, 9, 0, 0), date_time!(2023, 1, 6, 17, 0, 0)),
            ],
            "every matching day must survive the non-matching ones being dropped"
        );
    }
}
