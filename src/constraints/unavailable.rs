use chrono::NaiveDate;

use crate::timerange::TimeRange;

constraint_iterator!(UnavailableIterator(start: NaiveDate, end: NaiveDate) = |self, range| {
    let blackout = TimeRange::new(self.start.and_time(time!(0, 0)), self.end.and_time(time!(0, 0)));

    match blackout.intersection(&range) {
        // No overlap at all, or the ranges merely touch at an endpoint.
        None => vec![range],
        Some(conflict) if conflict.is_zero() => vec![range],

        // The blackout falls in the middle, splitting the range in two.
        Some(conflict) if conflict.start > range.start && conflict.end < range.end => vec![
            TimeRange::new(range.start, conflict.start),
            TimeRange::new(conflict.end, range.end),
        ],

        // The blackout trims the end of the range.
        Some(conflict) if conflict.start > range.start => {
            vec![TimeRange::new(range.start, conflict.start)]
        },

        // The blackout trims the start of the range.
        Some(conflict) if conflict.end < range.end => {
            vec![TimeRange::new(conflict.end, range.end)]
        },

        // The blackout swallows the range whole, leaving nothing behind.
        Some(_) => vec![],
    }
});

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(ranges: Vec<TimeRange>, start: NaiveDate, end: NaiveDate) -> Vec<TimeRange> {
        UnavailableIterator::new(ranges.into_iter(), start, end).collect()
    }

    #[test]
    fn test_unavailable() {
        let output = apply(
            vec![TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 3))],
            date!(2020, 1, 1),
            date!(2020, 1, 2),
        );

        assert_eq!(
            output,
            vec![TimeRange::new(date_time!(2020, 1, 2), date_time!(2020, 1, 3))]
        );
    }

    #[test]
    fn a_blackout_covering_the_whole_range_removes_it() {
        let output = apply(
            vec![TimeRange::new(
                date_time!(2020, 1, 2, 9, 0, 0),
                date_time!(2020, 1, 2, 17, 0, 0),
            )],
            date!(2020, 1, 1),
            date!(2020, 1, 5),
        );

        assert!(
            output.is_empty(),
            "a range entirely inside the blackout must be removed, got {output:?}"
        );
    }

    #[test]
    fn a_blackout_exactly_matching_the_range_removes_it() {
        let output = apply(
            vec![TimeRange::new(date_time!(2020, 1, 2), date_time!(2020, 1, 3))],
            date!(2020, 1, 2),
            date!(2020, 1, 3),
        );

        assert!(output.is_empty(), "got {output:?}");
    }

    #[test]
    fn a_blackout_in_the_middle_splits_the_range() {
        let output = apply(
            vec![TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 7))],
            date!(2020, 1, 3),
            date!(2020, 1, 5),
        );

        assert_eq!(
            output,
            vec![
                TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 3)),
                TimeRange::new(date_time!(2020, 1, 5), date_time!(2020, 1, 7)),
            ]
        );
    }

    #[test]
    fn a_blackout_trimming_the_end_shortens_the_range() {
        let output = apply(
            vec![TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 7))],
            date!(2020, 1, 5),
            date!(2020, 1, 9),
        );

        assert_eq!(
            output,
            vec![TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 5))]
        );
    }

    #[test]
    fn a_disjoint_blackout_leaves_the_range_alone() {
        let range = TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 3));

        assert_eq!(
            apply(vec![range], date!(2020, 6, 1), date!(2020, 6, 5)),
            vec![range]
        );
        // Touching at an endpoint is not an overlap.
        assert_eq!(
            apply(vec![range], date!(2020, 1, 3), date!(2020, 1, 5)),
            vec![range]
        );
    }

    #[test]
    fn removing_one_range_does_not_truncate_the_rest() {
        // Regression: an empty segment used to terminate the iterator, silently
        // discarding every remaining range.
        let output = apply(
            vec![
                TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 2)),
                TimeRange::new(date_time!(2020, 1, 3), date_time!(2020, 1, 4)),
                TimeRange::new(date_time!(2020, 1, 5), date_time!(2020, 1, 6)),
            ],
            date!(2020, 1, 3),
            date!(2020, 1, 4),
        );

        assert_eq!(
            output,
            vec![
                TimeRange::new(date_time!(2020, 1, 1), date_time!(2020, 1, 2)),
                TimeRange::new(date_time!(2020, 1, 5), date_time!(2020, 1, 6)),
            ],
            "the third range must survive the second being removed"
        );
    }
}
