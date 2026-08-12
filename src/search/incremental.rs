//! Incremental score maintenance.
//!
//! The reference evaluator in [`crate::objectives`] walks the entire schedule.
//! The search needs to evaluate millions of candidate moves, so instead we keep
//! a running total together with a cached decomposition, and on each move
//! recompute only the parts that changed.
//!
//! The decomposition mirrors how the objectives are defined:
//!
//! * slot-local terms (coverage, preferences, stability) depend only on a slot
//!   and its owner, so a change costs O(1) per touched slot;
//! * workload (fairness) depends on a person's total minutes, so a change costs
//!   O(1) per touched person;
//! * shape (shift length and rest) depends on how a person's slots group into
//!   shifts, so it is recomputed for the touched people only.
//!
//! Moves are applied speculatively and rolled back if rejected. Doing it that
//! way — rather than predicting a delta and then separately applying it — means
//! there is only one code path, so "what the search thought a move was worth"
//! and "what the move actually did" cannot disagree.

use crate::model::problem::{HumanIdx, Problem, SlotIdx};
use crate::model::{Assignment, Score};
use crate::objectives;

/// A record of what a slot held before a move, used to roll it back.
pub type Change = (SlotIdx, Option<HumanIdx>);

/// A running score plus the cached per-person terms it is built from.
pub struct Incremental {
    score: Score,
    /// Shift length and rest, per person.
    shape: Vec<Score>,
    /// Fairness, per person.
    workload: Vec<Score>,
    /// Reusable scratch for the set of people a move touched.
    dirty: Vec<bool>,
    dirty_list: Vec<HumanIdx>,
}

impl Incremental {
    /// Builds the cache by evaluating the assignment from scratch.
    pub fn new(problem: &Problem, assignment: &Assignment) -> Self {
        let shape: Vec<Score> = (0..problem.human_count())
            .map(|human| objectives::human_shape(problem, assignment, human))
            .collect();

        let workload: Vec<Score> = (0..problem.human_count())
            .map(|human| {
                objectives::human_workload(problem, human, assignment.assigned_minutes(human))
            })
            .collect();

        let slot_total: Score = (0..problem.slot_count())
            .map(|slot| objectives::slot_local(problem, slot, assignment.get(slot)))
            .sum();

        let score = slot_total
            + shape.iter().copied().sum::<Score>()
            + workload.iter().copied().sum::<Score>();

        Self {
            score,
            shape,
            workload,
            dirty: vec![false; problem.human_count()],
            dirty_list: Vec::with_capacity(8),
        }
    }

    /// The current score.
    #[inline]
    pub fn score(&self) -> Score {
        self.score
    }

    /// Applies a set of slot changes, recording what to pass to [`Self::undo`].
    ///
    /// `undo` is cleared and repopulated, so the same buffer can be reused for
    /// every move without allocating.
    pub fn apply(
        &mut self,
        problem: &Problem,
        assignment: &mut Assignment,
        changes: &[Change],
        undo: &mut Vec<Change>,
    ) {
        undo.clear();
        undo.reserve(changes.len());

        // Everyone losing or gaining a slot needs their shape and workload
        // recomputed. Collect them before mutating so we can subtract the stale
        // contributions first.
        for &(slot, next) in changes {
            let previous = assignment.get(slot);
            undo.push((slot, previous));

            if let Some(human) = previous {
                self.mark_dirty(human);
            }
            if let Some(human) = next {
                self.mark_dirty(human);
            }
        }

        for &human in self.dirty_list.iter() {
            self.score -= self.shape[human] + self.workload[human];
        }

        for &(slot, next) in changes {
            self.score -= objectives::slot_local(problem, slot, assignment.get(slot));
            assignment.assign(problem, slot, next);
            self.score += objectives::slot_local(problem, slot, next);
        }

        for &human in self.dirty_list.iter() {
            self.shape[human] = objectives::human_shape(problem, assignment, human);
            self.workload[human] = objectives::human_workload(
                problem,
                human,
                assignment.assigned_minutes(human),
            );
            self.score += self.shape[human] + self.workload[human];
        }

        for &human in self.dirty_list.iter() {
            self.dirty[human] = false;
        }
        self.dirty_list.clear();
    }

    /// Rolls a move back. Undoing is just applying the recorded previous state.
    pub fn undo(&mut self, problem: &Problem, assignment: &mut Assignment, undo: &[Change]) {
        // The scratch buffer is only needed to satisfy `apply`; the caller
        // already holds the state we are restoring.
        let mut discard = Vec::with_capacity(undo.len());
        self.apply(problem, assignment, undo, &mut discard);
    }

    #[inline]
    fn mark_dirty(&mut self, human: HumanIdx) {
        if !self.dirty[human] {
            self.dirty[human] = true;
            self.dirty_list.push(human);
        }
    }

    /// Recomputes the score from scratch and asserts it matches the running
    /// total.
    ///
    /// This is the guard rail for the whole incremental path: if a move ever
    /// updates the cache incorrectly, the running score silently diverges from
    /// what the schedule is actually worth and the search starts optimising
    /// fiction. Because scores are integers the comparison is exact, so this
    /// catches drift the moment it happens rather than eventually.
    pub fn verify(&self, problem: &Problem, assignment: &Assignment) {
        let expected = objectives::evaluate(problem, assignment);
        assert_eq!(
            self.score, expected,
            "incrementally maintained score has drifted from the reference evaluation"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives::testing::fixture;
    use rand::{RngExt, SeedableRng};
    use rand_pcg::Pcg64;

    #[test]
    fn matches_the_reference_on_construction() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);
        for slot in 0..problem.slot_count() {
            assignment.assign(problem, slot, Some((slot / 2) % 3));
        }

        let incremental = Incremental::new(problem, &assignment);
        assert_eq!(
            incremental.score(),
            objectives::evaluate(problem, &assignment)
        );
    }

    #[test]
    fn matches_the_reference_on_an_empty_schedule() {
        let fixture = fixture();
        let assignment = Assignment::empty(&fixture.problem);
        let incremental = Incremental::new(&fixture.problem, &assignment);

        assert_eq!(
            incremental.score(),
            objectives::evaluate(&fixture.problem, &assignment)
        );
    }

    #[test]
    fn a_single_change_updates_the_score_exactly() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);
        for slot in 0..problem.slot_count() {
            assignment.assign(problem, slot, Some((slot / 2) % 3));
        }

        let mut incremental = Incremental::new(problem, &assignment);
        let mut undo = Vec::new();

        incremental.apply(problem, &mut assignment, &[(5, Some(1))], &mut undo);
        incremental.verify(problem, &assignment);
    }

    #[test]
    fn undo_restores_both_the_assignment_and_the_score() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);
        for slot in 0..problem.slot_count() {
            assignment.assign(problem, slot, Some((slot / 2) % 3));
        }

        let before_score = objectives::evaluate(problem, &assignment);
        let before_slots = assignment.slots().to_vec();

        let mut incremental = Incremental::new(problem, &assignment);
        let mut undo = Vec::new();

        incremental.apply(
            problem,
            &mut assignment,
            &[(3, Some(2)), (4, None), (5, Some(0))],
            &mut undo,
        );
        assert_ne!(assignment.slots(), before_slots.as_slice());

        let undo_changes = undo.clone();
        incremental.undo(problem, &mut assignment, &undo_changes);

        assert_eq!(assignment.slots(), before_slots.as_slice());
        assert_eq!(incremental.score(), before_score);
        incremental.verify(problem, &assignment);
    }

    /// The single most important test for a delta-evaluated search: hammer the
    /// incremental path with random moves and assert it never drifts from the
    /// reference evaluation. Exact equality works because scores are integers.
    #[test]
    fn score_never_drifts_under_random_moves() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);
        let mut incremental = Incremental::new(problem, &assignment);
        let mut rng = Pcg64::seed_from_u64(12345);
        let mut undo = Vec::new();
        let mut changes = Vec::new();

        for step in 0..20_000 {
            changes.clear();

            // Mix single-slot changes with multi-slot ones so that run merging,
            // splitting and clearing all get exercised.
            let width = rng.random_range(1..5);
            let start = rng.random_range(0..problem.slot_count());
            for slot in start..(start + width).min(problem.slot_count()) {
                let domain = &problem.domains[slot];
                let choice = if domain.is_empty() || rng.random_ratio(1, 5) {
                    None
                } else {
                    Some(domain[rng.random_range(0..domain.len())])
                };
                changes.push((slot, choice));
            }

            incremental.apply(problem, &mut assignment, &changes, &mut undo);

            if rng.random_bool(0.5) {
                let undo_changes = undo.clone();
                incremental.undo(problem, &mut assignment, &undo_changes);
            }

            if step % 250 == 0 {
                incremental.verify(problem, &assignment);
            }
        }

        incremental.verify(problem, &assignment);
    }

    #[test]
    fn score_never_drifts_with_every_objective_engaged() {
        let mut fixture = fixture();
        fixture.problem.min_rest_minutes = Some(16 * 60);
        fixture.problem.max_consecutive_minutes = Some(24 * 60);
        let baseline: Vec<Option<HumanIdx>> = (0..fixture.problem.slot_count())
            .map(|slot| Some(slot % 3))
            .collect();
        let problem = &fixture.problem.with_baseline(baseline);

        let mut assignment = Assignment::empty(problem);
        let mut incremental = Incremental::new(problem, &assignment);
        let mut rng = Pcg64::seed_from_u64(999);
        let mut undo = Vec::new();

        for step in 0..20_000 {
            let slot = rng.random_range(0..problem.slot_count());
            let domain = &problem.domains[slot];
            let choice = if domain.is_empty() || rng.random_ratio(1, 6) {
                None
            } else {
                Some(domain[rng.random_range(0..domain.len())])
            };

            incremental.apply(problem, &mut assignment, &[(slot, choice)], &mut undo);

            if step % 200 == 0 {
                incremental.verify(problem, &assignment);
            }
        }

        incremental.verify(problem, &assignment);
    }

    #[test]
    fn applying_a_no_op_change_leaves_the_score_alone() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);
        assignment.assign(problem, 0, Some(0));

        let mut incremental = Incremental::new(problem, &assignment);
        let before = incremental.score();
        let mut undo = Vec::new();

        incremental.apply(problem, &mut assignment, &[(0, Some(0))], &mut undo);

        assert_eq!(incremental.score(), before);
        incremental.verify(problem, &assignment);
    }
}
