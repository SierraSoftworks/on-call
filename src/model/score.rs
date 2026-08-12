//! The objective score used to rank candidate schedules.

use std::fmt::Display;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

/// A two-tier score, compared lexicographically: a schedule with fewer hard
/// constraint violations always beats one with more, regardless of how good its
/// soft score is.
///
/// Both tiers are integers rather than floats. That is deliberate: the search
/// maintains this score incrementally, and integer arithmetic lets us assert
/// *exact* equality against a from-scratch recomputation (see the score
/// corruption tests) instead of fighting floating point drift.
///
/// Lower is better, and `Ord` is derived in field order so `hard` dominates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub struct Score {
    /// Magnitude of hard constraint violations. Must reach zero for a schedule
    /// to be considered feasible. This is a magnitude rather than a count so
    /// that the search has a gradient to follow toward feasibility.
    pub hard: i64,
    /// Weighted sum of soft penalties. Minimised once `hard` is zero.
    pub soft: i64,
}

impl Score {
    pub const ZERO: Score = Score { hard: 0, soft: 0 };

    pub const fn new(hard: i64, soft: i64) -> Self {
        Self { hard, soft }
    }

    pub const fn hard(hard: i64) -> Self {
        Self { hard, soft: 0 }
    }

    pub const fn soft(soft: i64) -> Self {
        Self { hard: 0, soft }
    }

    /// Whether the schedule satisfies every hard constraint.
    pub const fn is_feasible(&self) -> bool {
        self.hard == 0
    }
}

impl Add for Score {
    type Output = Score;

    fn add(self, rhs: Score) -> Score {
        Score {
            hard: self.hard.saturating_add(rhs.hard),
            soft: self.soft.saturating_add(rhs.soft),
        }
    }
}

impl Sub for Score {
    type Output = Score;

    fn sub(self, rhs: Score) -> Score {
        Score {
            hard: self.hard.saturating_sub(rhs.hard),
            soft: self.soft.saturating_sub(rhs.soft),
        }
    }
}

impl Neg for Score {
    type Output = Score;

    fn neg(self) -> Score {
        Score {
            hard: -self.hard,
            soft: -self.soft,
        }
    }
}

impl AddAssign for Score {
    fn add_assign(&mut self, rhs: Score) {
        *self = *self + rhs;
    }
}

impl SubAssign for Score {
    fn sub_assign(&mut self, rhs: Score) {
        *self = *self - rhs;
    }
}

impl Sum for Score {
    fn sum<I: Iterator<Item = Score>>(iter: I) -> Score {
        iter.fold(Score::ZERO, |acc, score| acc + score)
    }
}

impl Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}hard/{}soft", self.hard, self.soft)
    }
}

/// The fixed-point scale used for configurable weights.
///
/// Weights are written as decimals in YAML (`fairness: 2.5`) but stored as
/// integers scaled by this factor so that score arithmetic stays exact.
pub const WEIGHT_SCALE: i64 = 1_000;

/// Converts a user-supplied weight into fixed-point form.
pub fn weight_from_f64(weight: f64) -> i64 {
    (weight * WEIGHT_SCALE as f64).round() as i64
}

/// Converts a fixed-point weight back into a decimal, for display.
pub fn weight_to_f64(weight: i64) -> f64 {
    weight as f64 / WEIGHT_SCALE as f64
}

/// Applies a fixed-point weight to a raw penalty.
///
/// Uses a 128-bit intermediate so that large penalties multiplied by large
/// weights cannot overflow before the scale is divided back out.
pub fn apply_weight(weight: i64, penalty: i64) -> i64 {
    let scaled = (weight as i128) * (penalty as i128) / (WEIGHT_SCALE as i128);
    scaled.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

/// Computes `value² / scale`, saturating rather than overflowing.
///
/// Several objectives penalise a deviation quadratically so that the search
/// always has a gradient to follow (a linear or min/max penalty produces large
/// plateaus where most moves score identically and the search stalls). Dividing
/// by a scale — typically a representative slot length — keeps the result in
/// the same units as the input, so weights stay interpretable and the numbers
/// stay far away from `i64` limits.
pub fn normalised_square(value: i64, scale: i64) -> i64 {
    if scale <= 0 {
        return 0;
    }

    let squared = (value as i128) * (value as i128) / (scale as i128);
    squared.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_dominates_soft_when_comparing() {
        let infeasible_but_pretty = Score::new(1, 0);
        let feasible_but_ugly = Score::new(0, 1_000_000);

        assert!(
            feasible_but_ugly < infeasible_but_pretty,
            "any feasible schedule must beat any infeasible one"
        );
    }

    #[test]
    fn soft_breaks_ties_within_the_same_hard_tier() {
        assert!(Score::new(3, 10) < Score::new(3, 11));
        assert!(Score::new(3, 10) > Score::new(2, i64::MAX));
    }

    #[test]
    fn addition_and_subtraction_round_trip() {
        let a = Score::new(3, 40);
        let b = Score::new(1, 5);

        assert_eq!(a + b, Score::new(4, 45));
        assert_eq!(a + b - b, a);
        assert_eq!(-a + a, Score::ZERO);
    }

    #[test]
    fn addition_saturates_instead_of_overflowing() {
        let big = Score::new(i64::MAX, i64::MAX);
        assert_eq!(big + big, big, "should saturate rather than wrap");
    }

    #[test]
    fn sum_folds_from_zero() {
        let scores = vec![Score::soft(1), Score::soft(2), Score::hard(3)];
        assert_eq!(
            scores.into_iter().sum::<Score>(),
            Score::new(3, 3)
        );
        assert_eq!(Vec::<Score>::new().into_iter().sum::<Score>(), Score::ZERO);
    }

    #[test]
    fn feasibility_tracks_the_hard_tier() {
        assert!(Score::soft(9999).is_feasible());
        assert!(!Score::hard(1).is_feasible());
    }

    #[test]
    fn weights_round_trip_through_fixed_point() {
        for weight in [0.0, 0.5, 1.0, 2.5, 100.0] {
            let fixed = weight_from_f64(weight);
            assert!((weight_to_f64(fixed) - weight).abs() < 1e-9, "{weight}");
        }
    }

    #[test]
    fn apply_weight_scales_penalties() {
        assert_eq!(apply_weight(weight_from_f64(1.0), 500), 500);
        assert_eq!(apply_weight(weight_from_f64(2.0), 500), 1000);
        assert_eq!(apply_weight(weight_from_f64(0.5), 500), 250);
        assert_eq!(apply_weight(weight_from_f64(0.0), 500), 0);
    }

    #[test]
    fn apply_weight_does_not_overflow_on_large_inputs() {
        let result = apply_weight(weight_from_f64(1000.0), i64::MAX / 2);
        assert_eq!(result, i64::MAX, "should clamp rather than wrap");
    }

    #[test]
    fn normalised_square_keeps_units() {
        // A deviation of exactly one scale unit costs one scale unit.
        assert_eq!(normalised_square(480, 480), 480);
        // Doubling the deviation quadruples the cost.
        assert_eq!(normalised_square(960, 480), 1920);
        // The penalty is symmetric around zero.
        assert_eq!(normalised_square(-480, 480), 480);
        assert_eq!(normalised_square(0, 480), 0);
    }

    #[test]
    fn normalised_square_tolerates_a_zero_scale() {
        assert_eq!(normalised_square(100, 0), 0);
        assert_eq!(normalised_square(100, -5), 0);
    }

    #[test]
    fn normalised_square_saturates() {
        assert_eq!(normalised_square(i64::MAX, 1), i64::MAX);
    }
}
