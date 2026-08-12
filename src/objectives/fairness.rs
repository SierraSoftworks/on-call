//! Fairness: everybody should end up close to their fair share of on-call time.

use crate::model::problem::{HumanIdx, Problem};
use crate::model::score::{apply_weight, normalised_square};
use crate::model::{Assignment, Score};

pub const NAME: &str = "fairness";

/// The penalty for one person's workload deviating from their target.
///
/// The deviation is squared for two reasons. First, it makes a single large
/// imbalance worse than several small ones, which matches how teams actually
/// experience unfairness. Second — and more importantly for the search — it
/// gives the objective a gradient everywhere. Minimising the raw min/max spread
/// instead produces a huge plateau on which most moves score identically, and
/// local search stalls on plateaus.
///
/// Targets are computed so that they sum to exactly the total demand, so a
/// perfectly fair schedule scores zero and the optimum is well-defined.
#[inline]
pub fn human_penalty(problem: &Problem, human: HumanIdx, assigned_minutes: i64) -> Score {
    let deviation = assigned_minutes - problem.targets[human];

    Score::soft(apply_weight(
        problem.weights.fairness,
        normalised_square(deviation, problem.scale),
    ))
}

/// Evaluates fairness across the whole team.
pub fn evaluate(problem: &Problem, assignment: &Assignment) -> Score {
    (0..problem.human_count())
        .map(|human| human_penalty(problem, human, assignment.assigned_minutes(human)))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives::testing::fixture;

    #[test]
    fn hitting_every_target_exactly_scores_zero() {
        let fixture = fixture();
        let problem = &fixture.problem;

        for human in 0..problem.human_count() {
            assert_eq!(
                human_penalty(problem, human, problem.targets[human]),
                Score::ZERO
            );
        }
    }

    #[test]
    fn deviation_is_penalised_symmetrically() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let target = problem.targets[0];

        let over = human_penalty(problem, 0, target + 480);
        let under = human_penalty(problem, 0, target - 480);

        assert_eq!(over, under, "being over and under by the same amount costs the same");
        assert!(over > Score::ZERO);
    }

    #[test]
    fn larger_deviations_cost_disproportionately_more() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let target = problem.targets[0];

        let single = human_penalty(problem, 0, target + 480).soft;
        let double = human_penalty(problem, 0, target + 960).soft;

        assert!(
            double >= single * 3,
            "quadratic penalty expected, got {single} then {double}"
        );
    }

    #[test]
    fn one_big_imbalance_beats_several_small_ones() {
        let fixture = fixture();
        let problem = &fixture.problem;

        // Two people 480 minutes out each...
        let spread = human_penalty(problem, 0, problem.targets[0] + 480)
            + human_penalty(problem, 1, problem.targets[1] + 480);
        // ...versus one person 960 minutes out.
        let concentrated = human_penalty(problem, 0, problem.targets[0] + 960)
            + human_penalty(problem, 1, problem.targets[1]);

        assert!(
            spread < concentrated,
            "spreading the imbalance should be preferred: {spread} vs {concentrated}"
        );
    }

    #[test]
    fn a_zero_weight_disables_the_objective() {
        let mut fixture = fixture();
        fixture.problem.weights.fairness = 0;

        assert_eq!(
            human_penalty(&fixture.problem, 0, fixture.problem.targets[0] + 100_000),
            Score::ZERO
        );
    }

    #[test]
    fn fairness_never_touches_the_hard_tier() {
        let fixture = fixture();
        let penalty = human_penalty(&fixture.problem, 0, fixture.problem.targets[0] + 99_999);

        assert_eq!(penalty.hard, 0, "fairness is a preference, not a rule");
    }

    #[test]
    fn evaluate_sums_every_person() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);

        for slot in 0..problem.slot_count() {
            assignment.assign(problem, slot, Some(0));
        }

        let expected: Score = (0..problem.human_count())
            .map(|h| human_penalty(problem, h, assignment.assigned_minutes(h)))
            .sum();

        assert_eq!(evaluate(problem, &assignment), expected);
        assert!(
            evaluate(problem, &assignment) > Score::ZERO,
            "one person doing everything is not fair"
        );
    }
}
