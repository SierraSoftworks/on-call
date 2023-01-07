macro_rules! constraint_iterator {
    ($name:ident (
        $($field:ident: $type:ty),*
    ) = |$self:ident, $range:ident| {
        $($body:tt)*
    }) => {
        pub struct $name<S: Iterator<Item = $crate::timerange::TimeRange>> {
            source: S,
            buffer: std::collections::VecDeque<$crate::timerange::TimeRange>,
            $($field: $type),*
        }

        impl<S: Iterator<Item = $crate::timerange::TimeRange>> $name<S> {
            #[allow(dead_code)]
            pub fn new(source: S, $($field: $type),*) -> Self {
                Self { source, buffer: std::collections::VecDeque::new(), $($field),* }
            }

            fn segment(&$self, $range: $crate::timerange::TimeRange) -> Vec<$crate::timerange::TimeRange> {
                $($body)*
            }
        }

        impl<S: Iterator<Item = $crate::timerange::TimeRange>> Iterator for $name<S> {
            type Item = $crate::timerange::TimeRange;

            fn next(&mut self) -> Option<Self::Item> {
                if self.buffer.is_empty() {
                    match self.source.next() {
                        Some(range) => {
                            self.buffer.extend(self.segment(range));
                        },
                        None => {},
                    }
                }

                self.buffer.pop_front()
            }
        }
    };
}

macro_rules! time {
    ($hour:expr, $minute:expr) => {
        time!($hour, $minute, 0)
    };
    ($hour:expr, $minute:expr, $second:expr) => {
        chrono::NaiveTime::from_hms_opt($hour, $minute, $second).unwrap()
    };
}

#[cfg(test)]
macro_rules! date {
    ($year:expr, $month:expr, $day:expr) => {
        chrono::NaiveDate::from_ymd_opt($year, $month, $day).unwrap()
    };
}

#[cfg(test)]
macro_rules! date_time {
    ($year:expr, $month:expr, $day:expr) => {
        date_time!($year, $month, $day, 0, 0, 0)
    };
    ($year:expr, $month:expr, $day:expr, $hour:expr, $minute:expr, $second:expr) => {
        chrono::NaiveDate::from_ymd_opt($year, $month, $day).and_then(|d| d.and_hms_opt($hour, $minute, $second)).unwrap()
    };
}