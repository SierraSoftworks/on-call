use chrono::{NaiveDate};

use crate::timerange::TimeRange;

constraint_iterator!(UnavailableIterator(start: NaiveDate, end: NaiveDate) = |self, range| {
    let conflict = TimeRange::new(self.start.and_time(time!(0, 0)), self.end.and_time(time!(0, 0))).intersection(&range);

    match conflict {
        Some(conflict) if conflict.is_zero() => vec![range],
        Some(conflict) if conflict.start != range.start && conflict.end != range.end => {
            vec![
                TimeRange::new(range.start, conflict.start),
                TimeRange::new(conflict.end, range.end),
            ]
        },
        Some(conflict) if conflict.start != range.start => {
            vec![TimeRange::new(range.start, conflict.start)]
        },
        Some(conflict) if conflict.end != range.end => {
            vec![TimeRange::new(conflict.end, range.end)]
        },
        _ => vec![range],
    }
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unavailable() {
        let output: Vec<TimeRange> = UnavailableIterator::new(
            vec![
                TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 3)),
            ].into_iter(),
            date!(2020, 1, 1),
            date!(2020, 1, 2),
        ).collect();

        let expected = vec![
            TimeRange::new(date_time!(2020, 1, 2), date_time!(2020, 1, 3)),
        ];

        assert_eq!(output, expected);
    }
}