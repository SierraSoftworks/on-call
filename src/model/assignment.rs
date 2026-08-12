//! The mutable state the search operates on: who is covering which slot.

use crate::model::problem::{HumanIdx, Problem, SlotIdx};

/// A compact set of slot indices, backed by a bitmap.
///
/// The search needs to answer "where does the run containing this slot start
/// and end?" and "when was this person last on-call before slot N?" millions of
/// times per second. A bitmap answers both with word-level scans instead of
/// walking the schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotSet {
    words: Vec<u64>,
    len: usize,
}

impl SlotSet {
    pub fn new(len: usize) -> Self {
        Self {
            words: vec![0; len.div_ceil(64)],
            len,
        }
    }

    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn contains(&self, index: SlotIdx) -> bool {
        index < self.len && (self.words[index / 64] >> (index % 64)) & 1 == 1
    }

    #[inline]
    pub fn insert(&mut self, index: SlotIdx) {
        debug_assert!(index < self.len);
        self.words[index / 64] |= 1u64 << (index % 64);
    }

    #[inline]
    pub fn remove(&mut self, index: SlotIdx) {
        debug_assert!(index < self.len);
        self.words[index / 64] &= !(1u64 << (index % 64));
    }

    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|&word| word == 0)
    }

    pub fn count(&self) -> usize {
        self.words.iter().map(|word| word.count_ones() as usize).sum()
    }

    /// The smallest set index `>= from`.
    pub fn next_set(&self, from: SlotIdx) -> Option<SlotIdx> {
        if from >= self.len {
            return None;
        }

        let mut word = from / 64;
        let mut bits = self.words[word] & (!0u64 << (from % 64));

        loop {
            if bits != 0 {
                let index = word * 64 + bits.trailing_zeros() as usize;
                return (index < self.len).then_some(index);
            }

            word += 1;
            if word >= self.words.len() {
                return None;
            }
            bits = self.words[word];
        }
    }

    /// The largest set index `<= from`.
    pub fn prev_set(&self, from: SlotIdx) -> Option<SlotIdx> {
        if self.len == 0 {
            return None;
        }

        let from = from.min(self.len - 1);
        let mut word = from / 64;
        let bit = from % 64;
        let mask = if bit == 63 { !0u64 } else { (1u64 << (bit + 1)) - 1 };
        let mut bits = self.words[word] & mask;

        loop {
            if bits != 0 {
                return Some(word * 64 + (63 - bits.leading_zeros() as usize));
            }

            if word == 0 {
                return None;
            }
            word -= 1;
            bits = self.words[word];
        }
    }

    /// The smallest unset index `> from`, ignoring padding past the end.
    fn next_unset_after(&self, from: SlotIdx) -> Option<SlotIdx> {
        let start = from + 1;
        if start >= self.len {
            return None;
        }

        let mut word = start / 64;
        let mut bits = !self.words[word] & (!0u64 << (start % 64));

        loop {
            if bits != 0 {
                let index = word * 64 + bits.trailing_zeros() as usize;
                return (index < self.len).then_some(index);
            }

            word += 1;
            if word >= self.words.len() {
                return None;
            }
            bits = !self.words[word];
        }
    }

    /// The largest unset index `< from`.
    fn prev_unset_before(&self, from: SlotIdx) -> Option<SlotIdx> {
        if from == 0 {
            return None;
        }

        let target = from - 1;
        let mut word = target / 64;
        let bit = target % 64;
        let mask = if bit == 63 { !0u64 } else { (1u64 << (bit + 1)) - 1 };
        let mut bits = !self.words[word] & mask;

        loop {
            if bits != 0 {
                return Some(word * 64 + (63 - bits.leading_zeros() as usize));
            }

            if word == 0 {
                return None;
            }
            word -= 1;
            bits = !self.words[word];
        }
    }

    /// The inclusive bounds of the unbroken run of set indices containing
    /// `index`, or `None` if `index` is not set.
    pub fn run_bounds(&self, index: SlotIdx) -> Option<(SlotIdx, SlotIdx)> {
        if !self.contains(index) {
            return None;
        }

        let start = self.prev_unset_before(index).map_or(0, |i| i + 1);
        let end = self
            .next_unset_after(index)
            .map_or(self.len - 1, |i| i - 1);

        Some((start, end))
    }

    /// Iterates the set's unbroken runs as inclusive `(start, end)` pairs.
    pub fn runs(&self) -> Runs<'_> {
        Runs {
            set: self,
            cursor: 0,
        }
    }

    /// Iterates the individual set indices.
    pub fn iter(&self) -> impl Iterator<Item = SlotIdx> + '_ {
        let mut cursor = Some(0);
        std::iter::from_fn(move || {
            let next = self.next_set(cursor?)?;
            cursor = next.checked_add(1);
            Some(next)
        })
    }
}

/// Iterator over the unbroken runs within a [`SlotSet`].
pub struct Runs<'a> {
    set: &'a SlotSet,
    cursor: SlotIdx,
}

impl Iterator for Runs<'_> {
    type Item = (SlotIdx, SlotIdx);

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.set.next_set(self.cursor)?;
        let end = self
            .set
            .next_unset_after(start)
            .map_or(self.set.len - 1, |i| i - 1);

        self.cursor = end + 1;
        Some((start, end))
    }
}

/// A candidate schedule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    slots: Vec<Option<HumanIdx>>,
    by_human: Vec<SlotSet>,
    assigned_minutes: Vec<i64>,
}

impl Assignment {
    /// Creates an entirely unassigned schedule.
    pub fn empty(problem: &Problem) -> Self {
        Self {
            slots: vec![None; problem.slot_count()],
            by_human: (0..problem.human_count())
                .map(|_| SlotSet::new(problem.slot_count()))
                .collect(),
            assigned_minutes: vec![0; problem.human_count()],
        }
    }

    /// Who is covering a slot.
    #[inline]
    pub fn get(&self, slot: SlotIdx) -> Option<HumanIdx> {
        self.slots[slot]
    }

    /// The raw slot-to-person mapping.
    pub fn slots(&self) -> &[Option<HumanIdx>] {
        &self.slots
    }

    /// The slots a person is covering.
    #[inline]
    pub fn slots_of(&self, human: HumanIdx) -> &SlotSet {
        &self.by_human[human]
    }

    /// Total minutes a person is on-call for, excluding prior workload.
    #[inline]
    pub fn assigned_minutes(&self, human: HumanIdx) -> i64 {
        self.assigned_minutes[human]
    }

    /// Assigns (or clears) a slot, keeping the derived indexes in step.
    pub fn assign(&mut self, problem: &Problem, slot: SlotIdx, human: Option<HumanIdx>) {
        let previous = self.slots[slot];
        if previous == human {
            return;
        }

        if let Some(previous) = previous {
            self.by_human[previous].remove(slot);
            self.assigned_minutes[previous] -= problem.slot_minutes[slot];
        }

        if let Some(human) = human {
            debug_assert!(
                problem.is_available(human, slot),
                "refusing to assign {} to a slot they cannot cover",
                problem.humans[human]
            );
            self.by_human[human].insert(slot);
            self.assigned_minutes[human] += problem.slot_minutes[slot];
        }

        self.slots[slot] = human;
    }

    /// Number of slots with nobody assigned.
    pub fn unassigned_count(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_none()).count()
    }

    /// Rebuilds the derived indexes from the slot mapping.
    ///
    /// Only needed when the slot mapping is constructed wholesale (for example
    /// when loading a baseline); `assign` maintains them incrementally.
    pub fn rebuild(&mut self, problem: &Problem) {
        for set in self.by_human.iter_mut() {
            *set = SlotSet::new(problem.slot_count());
        }
        self.assigned_minutes.iter_mut().for_each(|m| *m = 0);

        for (slot, assignee) in self.slots.iter().enumerate() {
            if let Some(human) = assignee {
                self.by_human[*human].insert(slot);
                self.assigned_minutes[*human] += problem.slot_minutes[slot];
            }
        }
    }

    /// Creates an assignment directly from a slot mapping.
    pub fn from_slots(problem: &Problem, slots: Vec<Option<HumanIdx>>) -> Self {
        debug_assert_eq!(slots.len(), problem.slot_count());

        let mut assignment = Self {
            slots,
            by_human: (0..problem.human_count())
                .map(|_| SlotSet::new(problem.slot_count()))
                .collect(),
            assigned_minutes: vec![0; problem.human_count()],
        };
        assignment.rebuild(problem);
        assignment
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_from(len: usize, indices: &[usize]) -> SlotSet {
        let mut set = SlotSet::new(len);
        for &index in indices {
            set.insert(index);
        }
        set
    }

    #[test]
    fn insert_remove_and_contains() {
        let mut set = SlotSet::new(200);
        assert!(set.is_empty());

        set.insert(0);
        set.insert(63);
        set.insert(64);
        set.insert(199);

        assert!(set.contains(0));
        assert!(set.contains(63));
        assert!(set.contains(64));
        assert!(set.contains(199));
        assert!(!set.contains(1));
        assert!(!set.contains(198));
        assert_eq!(set.count(), 4);

        set.remove(64);
        assert!(!set.contains(64));
        assert_eq!(set.count(), 3);
    }

    #[test]
    fn next_and_prev_set_cross_word_boundaries() {
        let set = set_from(200, &[5, 63, 64, 130]);

        assert_eq!(set.next_set(0), Some(5));
        assert_eq!(set.next_set(5), Some(5));
        assert_eq!(set.next_set(6), Some(63));
        assert_eq!(set.next_set(64), Some(64));
        assert_eq!(set.next_set(65), Some(130));
        assert_eq!(set.next_set(131), None);
        assert_eq!(set.next_set(500), None);

        assert_eq!(set.prev_set(199), Some(130));
        assert_eq!(set.prev_set(130), Some(130));
        assert_eq!(set.prev_set(129), Some(64));
        assert_eq!(set.prev_set(63), Some(63));
        assert_eq!(set.prev_set(4), None);
    }

    #[test]
    fn prev_set_on_empty_set_is_none() {
        let set = SlotSet::new(64);
        assert_eq!(set.prev_set(10), None);
        assert_eq!(set.next_set(0), None);
        assert_eq!(SlotSet::new(0).prev_set(0), None);
    }

    #[test]
    fn run_bounds_finds_the_enclosing_run() {
        let set = set_from(20, &[3, 4, 5, 10, 15, 16]);

        assert_eq!(set.run_bounds(3), Some((3, 5)));
        assert_eq!(set.run_bounds(4), Some((3, 5)));
        assert_eq!(set.run_bounds(5), Some((3, 5)));
        assert_eq!(set.run_bounds(10), Some((10, 10)));
        assert_eq!(set.run_bounds(16), Some((15, 16)));
        assert_eq!(set.run_bounds(7), None);
    }

    #[test]
    fn run_bounds_handles_runs_touching_the_edges() {
        let set = set_from(8, &[0, 1, 6, 7]);
        assert_eq!(set.run_bounds(0), Some((0, 1)));
        assert_eq!(set.run_bounds(7), Some((6, 7)));
    }

    #[test]
    fn run_bounds_handles_a_completely_full_set() {
        let set = set_from(70, &(0..70).collect::<Vec<_>>());
        assert_eq!(set.run_bounds(0), Some((0, 69)));
        assert_eq!(set.run_bounds(69), Some((0, 69)));
        assert_eq!(set.runs().collect::<Vec<_>>(), vec![(0, 69)]);
    }

    #[test]
    fn runs_enumerates_every_block() {
        let set = set_from(20, &[3, 4, 5, 10, 15, 16]);
        assert_eq!(
            set.runs().collect::<Vec<_>>(),
            vec![(3, 5), (10, 10), (15, 16)]
        );

        assert_eq!(SlotSet::new(10).runs().count(), 0);
    }

    #[test]
    fn runs_span_word_boundaries() {
        let set = set_from(200, &(60..70).collect::<Vec<_>>());
        assert_eq!(set.runs().collect::<Vec<_>>(), vec![(60, 69)]);
    }

    #[test]
    fn iter_yields_indices_in_order() {
        let set = set_from(200, &[0, 5, 63, 64, 199]);
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![0, 5, 63, 64, 199]);
        assert_eq!(SlotSet::new(10).iter().count(), 0);
    }

    mod assignment {
        use super::*;
        use crate::config::{Config, Human};
        use crate::constraints::Constraint;
        use crate::model::problem::Problem;
        use chrono::{Duration, Weekday};

        fn problem() -> Problem {
            let config = Config::for_test(
                Duration::days(1),
                [
                    ("alice@example.com".to_string(), Human::default()),
                    ("bob@example.com".to_string(), Human::default()),
                ]
                .into_iter()
                .collect(),
            )
            .with_constraints(vec![
                Constraint::DayOfWeek(vec![
                    Weekday::Mon,
                    Weekday::Tue,
                    Weekday::Wed,
                    Weekday::Thu,
                    Weekday::Fri,
                ]),
                Constraint::TimeOfDay {
                    start: time!(9, 0),
                    end: time!(17, 0),
                },
            ]);

            Problem::build(&config, date_time!(2023, 1, 2), date_time!(2023, 1, 16)).unwrap()
        }

        #[test]
        fn assigning_updates_every_derived_index() {
            let problem = problem();
            let mut assignment = Assignment::empty(&problem);

            assert_eq!(assignment.unassigned_count(), problem.slot_count());
            assert_eq!(assignment.assigned_minutes(0), 0);

            assignment.assign(&problem, 0, Some(0));
            assignment.assign(&problem, 1, Some(0));
            assignment.assign(&problem, 2, Some(1));

            assert_eq!(assignment.get(0), Some(0));
            assert_eq!(assignment.get(2), Some(1));
            assert_eq!(
                assignment.assigned_minutes(0),
                problem.slot_minutes[0] + problem.slot_minutes[1]
            );
            assert_eq!(assignment.assigned_minutes(1), problem.slot_minutes[2]);
            assert_eq!(assignment.slots_of(0).runs().collect::<Vec<_>>(), vec![(0, 1)]);
            assert_eq!(assignment.unassigned_count(), problem.slot_count() - 3);
        }

        #[test]
        fn reassigning_moves_minutes_between_people() {
            let problem = problem();
            let mut assignment = Assignment::empty(&problem);

            assignment.assign(&problem, 0, Some(0));
            assignment.assign(&problem, 0, Some(1));

            assert_eq!(assignment.assigned_minutes(0), 0);
            assert_eq!(assignment.assigned_minutes(1), problem.slot_minutes[0]);
            assert!(!assignment.slots_of(0).contains(0));
            assert!(assignment.slots_of(1).contains(0));
        }

        #[test]
        fn clearing_a_slot_restores_the_unassigned_count() {
            let problem = problem();
            let mut assignment = Assignment::empty(&problem);

            assignment.assign(&problem, 3, Some(1));
            assignment.assign(&problem, 3, None);

            assert_eq!(assignment.get(3), None);
            assert_eq!(assignment.assigned_minutes(1), 0);
            assert_eq!(assignment.unassigned_count(), problem.slot_count());
        }

        #[test]
        fn assigning_the_same_person_twice_is_a_no_op() {
            let problem = problem();
            let mut assignment = Assignment::empty(&problem);

            assignment.assign(&problem, 0, Some(0));
            let minutes = assignment.assigned_minutes(0);
            assignment.assign(&problem, 0, Some(0));

            assert_eq!(assignment.assigned_minutes(0), minutes);
        }

        #[test]
        fn rebuild_matches_incremental_maintenance() {
            let problem = problem();
            let mut incremental = Assignment::empty(&problem);

            for slot in 0..problem.slot_count() {
                incremental.assign(&problem, slot, Some(slot % 2));
            }

            let wholesale =
                Assignment::from_slots(&problem, incremental.slots().to_vec());

            assert_eq!(incremental, wholesale);
        }
    }
}
