//! Coverage: every slot should have somebody on it.

use crate::model::problem::{HumanIdx, Problem, SlotIdx};
use crate::model::{Assignment, Score};

pub const NAME: &str = "coverage";

/// The penalty for leaving a slot uncovered.
///
/// This lands in the hard tier because an unfilled slot is a real operational
/// gap, not a matter of taste. Note that when a slot has an empty domain — when
/// literally nobody is available — this penalty is unavoidable, and the hard
/// score will never reach zero. That is reported rather than treated as an
/// error, since the residual tells you exactly how much coverage you are short.
#[inline]
pub fn slot_penalty(problem: &Problem, slot: SlotIdx, assignee: Option<HumanIdx>) -> Score {
    match assignee {
        Some(_) => Score::ZERO,
        None => Score::hard(problem.slot_minutes[slot]),
    }
}

/// Evaluates coverage across the whole schedule.
pub fn evaluate(problem: &Problem, assignment: &Assignment) -> Score {
    (0..problem.slot_count())
        .map(|slot| slot_penalty(problem, slot, assignment.get(slot)))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives::testing::fixture;

    #[test]
    fn a_fully_covered_schedule_scores_zero() {
        let fixture = fixture();
        let mut assignment = Assignment::empty(&fixture.problem);
        for slot in 0..fixture.problem.slot_count() {
            assignment.assign(&fixture.problem, slot, Some(0));
        }

        assert_eq!(evaluate(&fixture.problem, &assignment), Score::ZERO);
    }

    #[test]
    fn uncovered_slots_cost_their_duration_in_the_hard_tier() {
        let fixture = fixture();
        let assignment = Assignment::empty(&fixture.problem);

        let expected = Score::hard(fixture.problem.total_demand());
        assert_eq!(evaluate(&fixture.problem, &assignment), expected);
    }

    #[test]
    fn covering_one_more_slot_always_helps() {
        let fixture = fixture();
        let mut assignment = Assignment::empty(&fixture.problem);
        let before = evaluate(&fixture.problem, &assignment);

        assignment.assign(&fixture.problem, 0, Some(0));
        let after = evaluate(&fixture.problem, &assignment);

        assert!(after < before);
        assert_eq!(before - after, Score::hard(fixture.problem.slot_minutes[0]));
    }
}
