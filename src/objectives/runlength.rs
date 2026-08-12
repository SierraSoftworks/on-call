//! Run length: shifts should be about as long as the configured shift length.

use crate::model::problem::{HumanIdx, Problem};
use crate::model::score::{apply_weight, normalised_square};
use crate::model::{Assignment, Score};

pub const NAME: &str = "runLength";

/// The penalty for a single unbroken shift covering `minutes` of on-call time.
///
/// Deviation is penalised in both directions: an over-long shift burns people
/// out, and a string of very short shifts means constant handoffs. Because the
/// penalty is quadratic, this also suppresses fragmentation without needing a
/// separate handoff objective — splitting one target-length shift into two half
/// shifts costs more than leaving it whole.
///
/// The target is per-person, capped at the longest shift their availability
/// actually permits. Measuring a part-timer against a shift length they
/// structurally cannot work makes every shift they take expensive, and the
/// optimizer responds by not scheduling them at all — the exact bias that
/// capacity-adjusted fairness exists to remove.
///
/// Lengths are measured in *covered* minutes rather than wall-clock span, so a
/// shift running Friday through Monday on a weekdays-only schedule counts the
/// hours actually on the hook, not the weekend in between.
#[inline]
pub fn run_penalty(problem: &Problem, human: HumanIdx, covered_minutes: i64) -> Score {
    let target = problem.achievable_run_minutes[human];

    let mut score = Score::soft(apply_weight(
        problem.weights.run_length,
        normalised_square(covered_minutes - target, problem.scale),
    ));

    if let Some(max) = problem.max_consecutive_minutes {
        if covered_minutes > max {
            score += Score::hard(covered_minutes - max);
        }
    }

    score
}

/// Evaluates every shift belonging to one person.
pub fn human_penalty(problem: &Problem, assignment: &Assignment, human: HumanIdx) -> Score {
    assignment
        .slots_of(human)
        .runs()
        .map(|(start, end)| run_penalty(problem, human, problem.covered_minutes(start, end)))
        .sum()
}

/// Evaluates every shift in the schedule.
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
    fn a_target_length_shift_scores_zero() {
        let fixture = fixture();
        assert_eq!(
            run_penalty(&fixture.problem, 0, fixture.problem.target_run_minutes),
            Score::ZERO
        );
    }

    #[test]
    fn shift_length_target_tracks_calendar_days() {
        // The fixture is a weekday 09:00-17:00 schedule with shiftLength 2, so
        // a shift should target two working days, i.e. sixteen hours.
        let fixture = fixture();
        assert_eq!(fixture.problem.target_run_minutes, 2 * 8 * 60);
    }

    #[test]
    fn both_too_short_and_too_long_are_penalised() {
        let fixture = fixture();
        let target = fixture.problem.target_run_minutes;

        assert!(run_penalty(&fixture.problem, 0, target - 480) > Score::ZERO);
        assert!(run_penalty(&fixture.problem, 0, target + 480) > Score::ZERO);
    }

    #[test]
    fn splitting_a_shift_in_two_costs_more_than_leaving_it_whole() {
        let fixture = fixture();
        let target = fixture.problem.target_run_minutes;

        let whole = run_penalty(&fixture.problem, 0, target);
        let split = run_penalty(&fixture.problem, 0, target / 2)
            + run_penalty(&fixture.problem, 0, target / 2);

        assert!(
            whole < split,
            "fragmentation should be discouraged: {whole} vs {split}"
        );
    }

    #[test]
    fn exceeding_the_hard_cap_lands_in_the_hard_tier() {
        let mut fixture = fixture();
        fixture.problem.max_consecutive_minutes = Some(960);

        let within = run_penalty(&fixture.problem, 0, 960);
        let beyond = run_penalty(&fixture.problem, 0, 1440);

        assert_eq!(within.hard, 0);
        assert_eq!(beyond.hard, 480, "the overrun is measured in minutes");
    }

    #[test]
    fn no_hard_cap_means_no_hard_penalty() {
        let mut fixture = fixture();
        fixture.problem.max_consecutive_minutes = None;

        assert_eq!(run_penalty(&fixture.problem, 0, 100_000).hard, 0);
    }

    #[test]
    fn a_zero_weight_disables_the_soft_penalty_but_not_the_cap() {
        let mut fixture = fixture();
        fixture.problem.weights.run_length = 0;
        fixture.problem.max_consecutive_minutes = Some(480);

        let penalty = run_penalty(&fixture.problem, 0, 960);
        assert_eq!(penalty.soft, 0);
        assert_eq!(penalty.hard, 480, "hard rules ignore soft weights");
    }

    #[test]
    fn human_penalty_walks_every_run() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);

        // Two separate single-slot shifts.
        assignment.assign(problem, 0, Some(0));
        assignment.assign(problem, 3, Some(0));

        let expected = run_penalty(problem, 0, problem.covered_minutes(0, 0))
            + run_penalty(problem, 0, problem.covered_minutes(3, 3));

        assert_eq!(human_penalty(problem, &assignment, 0), expected);
    }

    #[test]
    fn contiguous_slots_form_a_single_run() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let mut assignment = Assignment::empty(problem);

        assignment.assign(problem, 0, Some(0));
        assignment.assign(problem, 1, Some(0));

        assert_eq!(
            human_penalty(problem, &assignment, 0),
            run_penalty(problem, 0, problem.covered_minutes(0, 1))
        );
    }
}
