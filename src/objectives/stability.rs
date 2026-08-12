//! Stability: don't churn a schedule people have already planned around.

use crate::model::problem::{HumanIdx, Problem, SlotIdx};
use crate::model::score::apply_weight;
use crate::model::{Assignment, Score};

pub const NAME: &str = "stability";

/// The penalty for departing from a previously published schedule.
///
/// Without this, re-running the tool after a small config change is free to
/// reshuffle every shift, which is operationally useless — people plan their
/// lives around the roster. Supplying `--baseline` makes the optimizer pay for
/// each change, so it only moves shifts when the improvement justifies it.
///
/// Slots with no baseline entry (newly added time, or a person who has since
/// left the team) are free to be assigned however the other objectives prefer.
#[inline]
pub fn slot_penalty(problem: &Problem, slot: SlotIdx, assignee: Option<HumanIdx>) -> Score {
    let Some(baseline) = problem.baseline.as_ref() else {
        return Score::ZERO;
    };

    match baseline[slot] {
        Some(expected) if assignee != Some(expected) => Score::soft(apply_weight(
            problem.weights.stability,
            problem.slot_minutes[slot],
        )),
        _ => Score::ZERO,
    }
}

/// Evaluates stability across the whole schedule.
pub fn evaluate(problem: &Problem, assignment: &Assignment) -> Score {
    if problem.baseline.is_none() {
        return Score::ZERO;
    }

    (0..problem.slot_count())
        .map(|slot| slot_penalty(problem, slot, assignment.get(slot)))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives::testing::fixture;

    #[test]
    fn without_a_baseline_nothing_is_penalised() {
        let fixture = fixture();
        let mut assignment = Assignment::empty(&fixture.problem);
        for slot in 0..fixture.problem.slot_count() {
            assignment.assign(&fixture.problem, slot, Some(slot % 2));
        }

        assert_eq!(evaluate(&fixture.problem, &assignment), Score::ZERO);
    }

    #[test]
    fn matching_the_baseline_exactly_scores_zero() {
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> = (0..fixture.problem.slot_count())
            .map(|slot| Some(slot % 2))
            .collect();

        let mut assignment = Assignment::empty(&fixture.problem);
        for slot in 0..fixture.problem.slot_count() {
            assignment.assign(&fixture.problem, slot, Some(slot % 2));
        }

        fixture.problem = fixture.problem.with_baseline(baseline);
        assert_eq!(evaluate(&fixture.problem, &assignment), Score::ZERO);
    }

    #[test]
    fn each_departure_costs_the_slot_duration() {
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> =
            (0..fixture.problem.slot_count()).map(|_| Some(0)).collect();
        fixture.problem = fixture.problem.with_baseline(baseline);

        let mut assignment = Assignment::empty(&fixture.problem);
        for slot in 0..fixture.problem.slot_count() {
            assignment.assign(&fixture.problem, slot, Some(0));
        }

        let before = evaluate(&fixture.problem, &assignment);
        assignment.assign(&fixture.problem, 0, Some(1));
        let after = evaluate(&fixture.problem, &assignment);

        assert_eq!(before, Score::ZERO);
        assert_eq!(
            after,
            Score::soft(apply_weight(
                fixture.problem.weights.stability,
                fixture.problem.slot_minutes[0]
            ))
        );
    }

    #[test]
    fn leaving_a_baselined_slot_empty_also_counts_as_a_change() {
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> =
            (0..fixture.problem.slot_count()).map(|_| Some(0)).collect();
        fixture.problem = fixture.problem.with_baseline(baseline);

        assert!(slot_penalty(&fixture.problem, 0, None) > Score::ZERO);
    }

    #[test]
    fn slots_missing_from_the_baseline_are_free() {
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> =
            (0..fixture.problem.slot_count()).map(|_| None).collect();
        fixture.problem = fixture.problem.with_baseline(baseline);

        assert_eq!(slot_penalty(&fixture.problem, 0, Some(1)), Score::ZERO);
    }

    #[test]
    fn stability_never_touches_the_hard_tier() {
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> =
            (0..fixture.problem.slot_count()).map(|_| Some(0)).collect();
        fixture.problem = fixture.problem.with_baseline(baseline);

        assert_eq!(slot_penalty(&fixture.problem, 0, Some(1)).hard, 0);
    }

    #[test]
    fn a_zero_weight_disables_the_objective() {
        let mut fixture = fixture();
        let baseline: Vec<Option<HumanIdx>> =
            (0..fixture.problem.slot_count()).map(|_| Some(0)).collect();
        fixture.problem = fixture.problem.with_baseline(baseline);
        fixture.problem.weights.stability = 0;

        assert_eq!(slot_penalty(&fixture.problem, 0, Some(1)), Score::ZERO);
    }
}
