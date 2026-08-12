//! Rest: people need recovery time between shifts.

use crate::model::problem::{HumanIdx, Problem};
use crate::model::score::{apply_weight, normalised_square};
use crate::model::{Assignment, Score};

pub const NAME: &str = "rest";

/// The penalty for a gap of `gap_minutes` between two of a person's shifts.
///
/// Falling short of the desired rest is penalised quadratically, but there is
/// deliberately no reward for exceeding it. An uncapped "maximise time since
/// last shift" reward — which is what the old recency factor computed — is
/// gamed by front-loading one person and then never scheduling them again;
/// capping the reward removes that incentive and leaves fairness in charge of
/// long-run distribution.
///
/// Gaps are wall-clock, unlike shift lengths: what matters for recovery is how
/// much real time passed, including the weekend.
#[inline]
pub fn gap_penalty(problem: &Problem, gap_minutes: i64) -> Score {
    let mut score = Score::ZERO;

    if let Some(min) = problem.min_rest_minutes {
        if gap_minutes < min {
            score += Score::hard(min - gap_minutes);
        }
    }

    let shortfall = (problem.desired_rest_minutes - gap_minutes).max(0);
    score += Score::soft(apply_weight(
        problem.weights.rest,
        normalised_square(shortfall, problem.scale),
    ));

    score
}

/// Evaluates the gaps between all of one person's consecutive shifts.
pub fn human_penalty(problem: &Problem, assignment: &Assignment, human: HumanIdx) -> Score {
    let mut score = Score::ZERO;
    let mut previous_end = None;

    for (start, end) in assignment.slots_of(human).runs() {
        if let Some(previous_end) = previous_end {
            score += gap_penalty(problem, problem.gap_minutes(previous_end, start));
        }
        previous_end = Some(end);
    }

    score
}

/// Evaluates rest across the whole team.
pub fn evaluate(problem: &Problem, assignment: &Assignment) -> Score {
    (0..problem.human_count())
        .map(|human| human_penalty(problem, assignment, human))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives::testing::fixture;

    #[test]
    fn a_long_enough_gap_scores_zero() {
        let fixture = fixture();
        let desired = fixture.problem.desired_rest_minutes;

        assert_eq!(gap_penalty(&fixture.problem, desired), Score::ZERO);
        assert_eq!(gap_penalty(&fixture.problem, desired * 10), Score::ZERO);
    }

    #[test]
    fn there_is_no_reward_for_resting_longer_than_desired() {
        let fixture = fixture();
        let desired = fixture.problem.desired_rest_minutes;

        assert_eq!(
            gap_penalty(&fixture.problem, desired),
            gap_penalty(&fixture.problem, desired * 100),
            "extra rest beyond the target must not be rewarded"
        );
    }

    #[test]
    fn shorter_gaps_cost_progressively_more() {
        let fixture = fixture();
        let desired = fixture.problem.desired_rest_minutes;

        let slight = gap_penalty(&fixture.problem, desired - 60);
        let severe = gap_penalty(&fixture.problem, desired - 600);

        assert!(slight > Score::ZERO);
        assert!(severe > slight);
    }

    #[test]
    fn breaching_the_minimum_lands_in_the_hard_tier() {
        let mut fixture = fixture();
        fixture.problem.min_rest_minutes = Some(960);

        let ok = gap_penalty(&fixture.problem, 960);
        let breach = gap_penalty(&fixture.problem, 480);

        assert_eq!(ok.hard, 0);
        assert_eq!(breach.hard, 480, "the shortfall is measured in minutes");
    }

    #[test]
    fn no_minimum_means_no_hard_penalty() {
        let mut fixture = fixture();
        fixture.problem.min_rest_minutes = None;

        assert_eq!(gap_penalty(&fixture.problem, 0).hard, 0);
    }

    #[test]
    fn a_single_shift_has_no_gaps_to_score() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);

        assignment.assign(problem, 0, Some(0));
        assignment.assign(problem, 1, Some(0));

        assert_eq!(human_penalty(problem, &assignment, 0), Score::ZERO);
    }

    #[test]
    fn back_to_back_shifts_are_penalised() {
        let fixture = fixture();
        let problem = &fixture.problem;

        let mut spread = Assignment::empty(problem);
        spread.assign(problem, 0, Some(0));
        spread.assign(problem, 8, Some(0));

        let mut cramped = Assignment::empty(problem);
        cramped.assign(problem, 0, Some(0));
        cramped.assign(problem, 2, Some(0));

        assert!(
            human_penalty(problem, &spread, 0) < human_penalty(problem, &cramped, 0),
            "shifts further apart should score better"
        );
    }

    #[test]
    fn a_zero_weight_disables_the_soft_penalty_but_not_the_minimum() {
        let mut fixture = fixture();
        fixture.problem.weights.rest = 0;
        fixture.problem.min_rest_minutes = Some(960);

        let penalty = gap_penalty(&fixture.problem, 480);
        assert_eq!(penalty.soft, 0);
        assert_eq!(penalty.hard, 480);
    }
}
