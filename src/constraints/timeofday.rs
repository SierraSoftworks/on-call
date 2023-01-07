use chrono::NaiveTime;

use crate::timerange::TimeRange;

constraint_iterator!(TimeOfDayIterator(start: NaiveTime, end: NaiveTime) = |self, range| {
    let mut current = range.start;
    let mut result = Vec::new();

    while current <= range.end {
        let current_range = if self.start < self.end {
            TimeRange::new(current.date().and_time(self.start), current.date().and_time(self.end))
        } else {
            TimeRange::new(current.date().and_time(self.start), current.date().and_time(self.end) + chrono::Duration::days(1))
        };

        let intersection = range.intersection(&current_range);
        if let Some(intersection) = intersection {
            if !intersection.is_zero() {
                result.push(intersection);
            }
        }

        current += chrono::Duration::days(1);
    }

    result
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_same_day() {
        let output: Vec<TimeRange> = TimeOfDayIterator::new(
            vec![
                TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 3)),
            ].into_iter(),
            time!(9, 00),
            time!(17, 00),
        ).collect();

        let expected = vec![
            TimeRange::new(date_time!(2020, 1, 1, 9, 0, 0), date_time!(2020, 1, 1, 17, 0, 0)),
            TimeRange::new(date_time!(2020, 1, 2, 9, 0, 0), date_time!(2020, 1, 2, 17, 0, 0)),
        ];

        assert_eq!(output, expected);
    }

    #[test]
    fn test_day_wrap() {
        let output: Vec<TimeRange> = TimeOfDayIterator::new(
            vec![
                TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 3)),
            ].into_iter(),
            time!(17, 00),
            time!(9, 00),
        ).collect();

        let expected = vec![
            TimeRange::new(date_time!(2020, 1, 1, 17, 0, 0), date_time!(2020, 1, 2, 9, 0, 0)),
            TimeRange::new(date_time!(2020, 1, 2, 17, 0, 0), date_time!(2020, 1, 3, 0, 0, 0)),
        ];

        assert_eq!(output, expected);
    }
}