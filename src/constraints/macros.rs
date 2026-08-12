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
                // Keep pulling from the source until we produce something, or
                // the source runs dry. A constraint that filters a range out
                // entirely yields an empty segment, and stopping there would
                // silently truncate the rest of the schedule.
                while self.buffer.is_empty() {
                    match self.source.next() {
                        Some(range) => {
                            self.buffer.extend(self.segment(range));
                        },
                        None => return None,
                    }
                }

                self.buffer.pop_front()
            }
        }
    };
}
