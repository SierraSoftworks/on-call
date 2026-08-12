//! Preferences: honour what people asked for, where it is affordable.

use crate::model::problem::{HumanIdx, Problem, SlotIdx};
use crate::model::score::apply_weight;
use crate::model::{Assignment, Score};

pub const NAME: &str = "preference";

/// The penalty for placing somebody on a slot they asked to avoid, or the
/// (negative) reward for one they asked for.
///
/// All of the work happens when the problem is built: each person's preference
/// constraints are run through the same machinery as their availability, and
/// the result is baked into a dense bias grid. Evaluating a preference here is
/// therefore a single array lookup.
#[inline]
pub fn slot_penalty(problem: &Problem, slot: SlotIdx, assignee: Option<HumanIdx>) -> Score {
    match assignee {
        Some(human) if problem.has_preferences => Score::soft(apply_weight(
            problem.weights.preference,
            problem.bias(human, slot),
        )),
        _ => Score::ZERO,
    }
}

/// Evaluates preferences across the whole schedule.
pub fn evaluate(problem: &Problem, assignment: &Assignment) -> Score {
    if !problem.has_preferences {
        return Score::ZERO;
    }

    (0..problem.slot_count())
        .map(|slot| slot_penalty(problem, slot, assignment.get(slot)))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives::testing::{fixture, fixture_with_preferences};

    #[test]
    fn no_preferences_means_no_penalty() {
        let fixture = fixture();
        let mut assignment = Assignment::empty(&fixture.problem);
        for slot in 0..fixture.problem.slot_count() {
            assignment.assign(&fixture.problem, slot, Some(0));
        }

        assert_eq!(evaluate(&fixture.problem, &assignment), Score::ZERO);
    }

    #[test]
    fn avoided_slots_cost_and_preferred_slots_reward() {
        let fixture = fixture_with_preferences();
        let problem = &fixture.problem;
        let alice = problem.human_index("alice@example.com").unwrap();

        let mut avoided = None;
        let mut preferred = None;
        for slot in 0..problem.slot_count() {
            let bias = problem.bias(alice, slot);
            if bias > 0 {
                avoided.get_or_insert(slot);
            } else if bias < 0 {
                preferred.get_or_insert(slot);
            }
        }

        let avoided = avoided.expect("expected an avoided slot");
        let preferred = preferred.expect("expected a preferred slot");

        assert!(slot_penalty(problem, avoided, Some(alice)) > Score::ZERO);
        assert!(slot_penalty(problem, preferred, Some(alice)) < Score::ZERO);
    }

    #[test]
    fn an_unassigned_slot_carries_no_preference_penalty() {
        let fixture = fixture_with_preferences();
        assert_eq!(slot_penalty(&fixture.problem, 0, None), Score::ZERO);
    }

    #[test]
    fn other_peoples_preferences_do_not_apply() {
        let fixture = fixture_with_preferences();
        let problem = &fixture.problem;
        let bob = problem.human_index("bob@example.com").unwrap();

        for slot in 0..problem.slot_count() {
            assert_eq!(slot_penalty(problem, slot, Some(bob)), Score::ZERO);
        }
    }

    #[test]
    fn preferences_never_touch_the_hard_tier() {
        let fixture = fixture_with_preferences();
        let problem = &fixture.problem;
        let alice = problem.human_index("alice@example.com").unwrap();

        for slot in 0..problem.slot_count() {
            assert_eq!(slot_penalty(problem, slot, Some(alice)).hard, 0);
        }
    }

    #[test]
    fn a_zero_weight_disables_the_objective() {
        let mut fixture = fixture_with_preferences();
        fixture.problem.weights.preference = 0;
        let problem = &fixture.problem;
        let alice = problem.human_index("alice@example.com").unwrap();

        for slot in 0..problem.slot_count() {
            assert_eq!(slot_penalty(problem, slot, Some(alice)), Score::ZERO);
        }
    }
}
