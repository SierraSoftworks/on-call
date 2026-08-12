//! The immutable problem definition that the search operates over.
//!
//! Everything expensive or config-shaped is resolved once, here, into flat
//! integer-indexed arrays: slot durations in minutes, a dense availability
//! grid, per-person fair-share targets, and a precomputed preference bias
//! grid. The search loop then touches nothing but `usize` indices and `i64`
//! arithmetic.

use std::collections::BTreeSet;

use chrono::{Datelike, Duration, NaiveDateTime, Weekday};

use crate::config::{Config, FairnessMode, PreferenceKind, RotationBoundary};
use crate::model::score::{weight_from_f64, WEIGHT_SCALE};
use crate::timerange::TimeRange;

/// Index of a person within [`Problem::humans`].
pub type HumanIdx = usize;

/// Index of a slot within [`Problem::slots`].
pub type SlotIdx = usize;

/// The fully-resolved scheduling problem.
pub struct Problem {
    /// Contiguous, ordered, non-overlapping slots requiring coverage.
    ///
    /// These are *atomic*: every slot is either wholly coverable or wholly
    /// uncoverable by each person, so availability is never partial.
    pub slots: Vec<TimeRange>,

    /// Duration of each slot, in minutes.
    pub slot_minutes: Vec<i64>,

    /// Prefix sums over `slot_minutes`, so the covered duration of a run can be
    /// computed in constant time.
    minutes_prefix: Vec<i64>,

    /// Names of each person, sorted, so that indices are stable across runs.
    pub humans: Vec<String>,

    /// Dense `humans × slots` availability grid.
    availability: Vec<bool>,

    /// Per-slot list of the people who may cover it. This is the search domain.
    pub domains: Vec<Vec<HumanIdx>>,

    /// Per-person total on-call minutes they should ideally end up with.
    pub targets: Vec<i64>,

    /// Per-person on-call minutes carried over from previous scheduling runs.
    pub prior_workload: Vec<i64>,

    /// Dense `humans × slots` preference bias, in weighted minutes. Positive
    /// values discourage the assignment, negative values encourage it.
    bias: Vec<i64>,

    /// Whether any preference bias is non-zero, so the objective can be skipped.
    pub has_preferences: bool,

    /// Indices at which a new rotation may begin. Always contains slot 0.
    pub rotation_starts: Vec<SlotIdx>,

    /// Whether handoffs are structurally restricted to rotation boundaries.
    pub rotation_locked: bool,

    /// The atomic units of assignment.
    ///
    /// Normally one unit per slot. When `rotation.lock` is set, one unit per
    /// rotation, so the search can only ever move whole rotations and handoffs
    /// cannot land mid-rotation by construction.
    pub units: Vec<std::ops::Range<SlotIdx>>,

    /// The people who can cover an entire unit.
    pub unit_domains: Vec<Vec<HumanIdx>>,

    /// Which unit each slot belongs to.
    pub unit_of_slot: Vec<usize>,

    /// Target duration of a single shift, in minutes.
    pub target_run_minutes: i64,

    /// Per-person shift-length target, in minutes.
    ///
    /// Capped at the longest unbroken stretch each person's availability
    /// actually permits. Somebody who works Mondays, Wednesdays and Fridays
    /// cannot produce a five-day shift, and penalising them for that would
    /// price them out of the rota entirely — which is precisely the bias that
    /// capacity-adjusted fairness exists to remove.
    pub achievable_run_minutes: Vec<i64>,

    /// Hard cap on the wall-clock duration of a single shift, in minutes.
    pub max_consecutive_minutes: Option<i64>,

    /// Hard minimum rest between consecutive shifts, in minutes.
    pub min_rest_minutes: Option<i64>,

    /// Soft target for rest between consecutive shifts, in minutes.
    pub desired_rest_minutes: i64,

    /// Divisor used to normalise quadratic penalties back into minutes.
    pub scale: i64,

    /// Fixed-point objective weights.
    pub weights: ProblemWeights,

    /// Baseline assignment to stay close to, if one was supplied.
    pub baseline: Option<Vec<Option<HumanIdx>>>,

    /// Slots whose assignment is fixed and may not be changed.
    pub frozen: Vec<bool>,
}

/// Objective weights in fixed-point form.
#[derive(Debug, Clone, Copy)]
pub struct ProblemWeights {
    pub fairness: i64,
    pub run_length: i64,
    pub rest: i64,
    pub preference: i64,
    pub stability: i64,
}

/// Anything that stopped us from building a usable problem.
#[derive(Debug)]
pub struct ProblemError(pub String);

impl std::fmt::Display for ProblemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProblemError {}

impl std::fmt::Debug for Problem {
    /// Deliberately summarised: the dense grids run to hundreds of thousands of
    /// entries and are useless in a panic message.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Problem")
            .field("slots", &self.slots.len())
            .field("humans", &self.humans)
            .field("targets", &self.targets)
            .field("rotations", &self.rotation_starts.len())
            .field("rotation_locked", &self.rotation_locked)
            .field("scale", &self.scale)
            .finish_non_exhaustive()
    }
}

impl Problem {
    /// Builds a problem from a config and a scheduling horizon.
    pub fn build(
        config: &Config,
        start: NaiveDateTime,
        end: NaiveDateTime,
    ) -> Result<Self, ProblemError> {
        config.validate().map_err(ProblemError)?;

        // Humans are sorted so that indices — and therefore every tie-break
        // downstream — are stable regardless of HashMap iteration order.
        let mut humans: Vec<String> = config.humans.keys().cloned().collect();
        humans.sort();

        let base_slots = generate_slots(config, start, end);
        let human_ranges: Vec<Vec<TimeRange>> = humans
            .iter()
            .map(|name| available_ranges(&config.humans[name].constraints, &base_slots))
            .collect();

        let slots = atomise(&base_slots, &human_ranges);
        if slots.is_empty() {
            return Err(ProblemError(
                "the schedule horizon contains no slots requiring coverage; check your constraints and --start/--end"
                    .to_string(),
            ));
        }

        let slot_minutes: Vec<i64> = slots.iter().map(|slot| slot.len().num_minutes()).collect();

        let minutes_prefix = {
            let mut prefix = Vec::with_capacity(slot_minutes.len() + 1);
            prefix.push(0);
            for minutes in slot_minutes.iter() {
                prefix.push(prefix.last().unwrap() + minutes);
            }
            prefix
        };

        // A shift length is expressed in days, but slots need not be whole days
        // (a `TimeOfDay` constraint makes them shorter, and splitting at
        // availability boundaries makes them shorter still). Convert via the
        // average coverage per calendar day so that `shiftLength: 3` means
        // "three days' worth of on-call" under any slot layout.
        let covered_days = slots
            .iter()
            .map(|slot| slot.start.date())
            .collect::<BTreeSet<_>>()
            .len()
            .max(1) as i64;
        let total_demand: i64 = slot_minutes.iter().sum();
        let minutes_per_day = (total_demand / covered_days).max(1);
        let target_run_minutes = (config.shift_length.num_days().max(1) * minutes_per_day).max(1);

        let availability = build_availability(&slots, &human_ranges);
        let domains = build_domains(&slots, &humans, &availability);

        let prior_workload: Vec<i64> = humans
            .iter()
            .map(|name| config.humans[name].prior_workload.num_minutes())
            .collect();

        let targets = compute_targets(config, &humans, &slot_minutes, &availability, &prior_workload);

        let (bias, has_preferences) = build_bias(config, &humans, &slots, &slot_minutes)?;
        let achievable_run_minutes = build_achievable_runs(
            &humans,
            &slots,
            &slot_minutes,
            &availability,
            target_run_minutes,
        );
        let rotation_starts = build_rotations(config, &slots);

        let (units, unit_domains, unit_of_slot) = build_units(
            config.rotation.lock,
            &slots,
            &rotation_starts,
            &domains,
            &availability,
            humans.len(),
        );

        let scale = {
            let total: i64 = slot_minutes.iter().sum();
            (total / slot_minutes.len() as i64).max(1)
        };

        Ok(Self {
            slot_minutes,
            availability,
            domains,
            targets,
            prior_workload,
            bias,
            has_preferences,
            rotation_starts,
            rotation_locked: config.rotation.lock,
            units,
            unit_domains,
            unit_of_slot,
            target_run_minutes,
            achievable_run_minutes,
            max_consecutive_minutes: config.rules.max_consecutive.map(|d| d.num_minutes()),
            min_rest_minutes: config.rules.min_rest.map(|d| d.num_minutes()),
            desired_rest_minutes: config.desired_rest().num_minutes(),
            scale,
            weights: ProblemWeights {
                fairness: weight_from_f64(config.weights.fairness),
                run_length: weight_from_f64(config.weights.run_length),
                rest: weight_from_f64(config.weights.rest),
                preference: weight_from_f64(config.weights.preference),
                stability: weight_from_f64(config.weights.stability),
            },
            baseline: None,
            frozen: vec![false; slots.len()],
            minutes_prefix,
            humans,
            slots,
        })
    }

    /// Number of slots requiring coverage.
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// Number of people available to be scheduled.
    pub fn human_count(&self) -> usize {
        self.humans.len()
    }

    /// Number of rotations, when rotation locking is in effect.
    pub fn rotation_count(&self) -> usize {
        self.rotation_starts.len()
    }

    /// The half-open slot range covered by a rotation.
    pub fn rotation_slots(&self, rotation: usize) -> std::ops::Range<SlotIdx> {
        let start = self.rotation_starts[rotation];
        let end = self
            .rotation_starts
            .get(rotation + 1)
            .copied()
            .unwrap_or(self.slots.len());
        start..end
    }

    /// Whether `human` may cover `slot`.
    #[inline]
    pub fn is_available(&self, human: HumanIdx, slot: SlotIdx) -> bool {
        self.availability[human * self.slots.len() + slot]
    }

    /// The preference bias for placing `human` on `slot`, in weighted minutes.
    #[inline]
    pub fn bias(&self, human: HumanIdx, slot: SlotIdx) -> i64 {
        if self.bias.is_empty() {
            0
        } else {
            self.bias[human * self.slots.len() + slot]
        }
    }

    /// Total on-call minutes that must be covered.
    pub fn total_demand(&self) -> i64 {
        self.slot_minutes.iter().sum()
    }

    /// Total covered minutes across an inclusive run of slots.
    #[inline]
    pub fn covered_minutes(&self, start: SlotIdx, end: SlotIdx) -> i64 {
        self.minutes_prefix[end + 1] - self.minutes_prefix[start]
    }

    /// The wall-clock gap, in minutes, between the end of one run and the start
    /// of the next.
    #[inline]
    pub fn gap_minutes(&self, first_end: SlotIdx, second_start: SlotIdx) -> i64 {
        (self.slots[second_start].start - self.slots[first_end].end).num_minutes()
    }

    /// Resolves a person's name to their index.
    pub fn human_index(&self, name: &str) -> Option<HumanIdx> {
        self.humans.binary_search_by(|h| h.as_str().cmp(name)).ok()
    }

    /// Attaches a baseline schedule for the stability objective to anchor to.
    ///
    /// Baseline entries naming people who are not in the current config are
    /// ignored, as are entries whose time range no longer matches a slot.
    pub fn with_baseline(mut self, baseline: Vec<Option<HumanIdx>>) -> Self {
        debug_assert_eq!(baseline.len(), self.slots.len());
        self.baseline = Some(baseline);
        self
    }

    /// Freezes every slot which ends at or before `instant`, pinning it to its
    /// current baseline assignment so already-published shifts cannot move.
    pub fn freeze_before(&mut self, instant: NaiveDateTime) -> usize {
        let mut frozen = 0;
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.end <= instant {
                self.frozen[index] = true;
                frozen += 1;
            }
        }
        frozen
    }

    /// Whether a slot's assignment is pinned.
    #[inline]
    pub fn is_frozen(&self, slot: SlotIdx) -> bool {
        self.frozen[slot]
    }

    /// Number of atomic assignment units.
    pub fn unit_count(&self) -> usize {
        self.units.len()
    }

    /// Whether any slot in a unit is pinned.
    pub fn is_unit_frozen(&self, unit: usize) -> bool {
        self.units[unit].clone().any(|slot| self.frozen[slot])
    }
}

/// Folds the schedule-level constraints over the horizon to produce the slots
/// that require coverage.
fn generate_slots(config: &Config, start: NaiveDateTime, end: NaiveDateTime) -> Vec<TimeRange> {
    let initial: Box<dyn Iterator<Item = TimeRange>> =
        Box::new(std::iter::once(TimeRange::new(start, end)));

    let mut slots: Vec<TimeRange> = config
        .constraints
        .iter()
        .fold(initial, |ranges, constraint| constraint.flat_map(ranges))
        .filter(|range| !range.is_zero())
        .collect();

    slots.sort();
    slots.dedup();
    slots
}

/// Folds a person's constraints over the slot list to produce the ranges they
/// are actually able to cover.
fn available_ranges(
    constraints: &[crate::constraints::Constraint],
    slots: &[TimeRange],
) -> Vec<TimeRange> {
    let initial: Box<dyn Iterator<Item = TimeRange>> = Box::new(slots.iter().copied());

    let mut ranges: Vec<TimeRange> = constraints
        .iter()
        .fold(initial, |ranges, constraint| constraint.flat_map(ranges))
        .filter(|range| !range.is_zero())
        .collect();

    ranges.sort();

    // Merge touching or overlapping ranges so that containment checks below can
    // rely on a single range covering a slot.
    let mut merged: Vec<TimeRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => {
                last.end = last.end.max(range.end);
            }
            _ => merged.push(range),
        }
    }

    merged
}

/// Splits the base slots at every point where somebody's availability changes.
///
/// This is what makes the availability grid exact. Without it, a slot which a
/// person can only partially cover has to be treated as either fully available
/// (producing schedules they cannot honour) or fully unavailable (needlessly
/// discarding capacity). After splitting, neither case can arise.
fn atomise(base: &[TimeRange], human_ranges: &[Vec<TimeRange>]) -> Vec<TimeRange> {
    // Collect every availability boundary once, then reuse it for each slot.
    let mut cuts: BTreeSet<NaiveDateTime> = BTreeSet::new();
    for ranges in human_ranges {
        for range in ranges {
            cuts.insert(range.start);
            cuts.insert(range.end);
        }
    }

    let mut slots = Vec::with_capacity(base.len());
    for slot in base {
        let mut previous = slot.start;
        for &cut in cuts.range((
            std::ops::Bound::Excluded(slot.start),
            std::ops::Bound::Excluded(slot.end),
        )) {
            slots.push(TimeRange::new(previous, cut));
            previous = cut;
        }

        if previous < slot.end {
            slots.push(TimeRange::new(previous, slot.end));
        }
    }

    slots
}

/// Builds the dense `humans × slots` availability grid.
fn build_availability(slots: &[TimeRange], human_ranges: &[Vec<TimeRange>]) -> Vec<bool> {
    let mut availability = vec![false; human_ranges.len() * slots.len()];

    for (human, ranges) in human_ranges.iter().enumerate() {
        let offset = human * slots.len();
        for (index, slot) in slots.iter().enumerate() {
            // Ranges are sorted and merged, so the only candidate is the last
            // range starting at or before the slot.
            let candidate = ranges.partition_point(|range| range.start <= slot.start);
            if candidate == 0 {
                continue;
            }

            let range = &ranges[candidate - 1];
            availability[offset + index] = range.start <= slot.start && slot.end <= range.end;
        }
    }

    availability
}

/// Inverts the availability grid into a per-slot list of eligible people.
fn build_domains(
    slots: &[TimeRange],
    humans: &[String],
    availability: &[bool],
) -> Vec<Vec<HumanIdx>> {
    (0..slots.len())
        .map(|slot| {
            (0..humans.len())
                .filter(|&human| availability[human * slots.len() + slot])
                .collect()
        })
        .collect()
}

/// Derives each person's fair share of the total workload.
///
/// The share is proportional to how much of the schedule they can actually
/// cover (or equal, under [`FairnessMode::Equal`]), and prior workload is
/// treated as time already served against a larger notional total so that
/// `sum(targets) == total demand` exactly. Anybody whose prior workload already
/// exceeds their share is pinned at zero and their share redistributed.
fn compute_targets(
    config: &Config,
    humans: &[String],
    slot_minutes: &[i64],
    availability: &[bool],
    prior_workload: &[i64],
) -> Vec<i64> {
    let count = humans.len();
    let demand: i64 = slot_minutes.iter().sum();

    let raw: Vec<f64> = humans
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let multiplier = config.humans[name].capacity.unwrap_or(1.0);
            let base = match config.fairness {
                FairnessMode::Equal => demand as f64,
                FairnessMode::Capacity => {
                    let offset = index * slot_minutes.len();
                    slot_minutes
                        .iter()
                        .enumerate()
                        .filter(|(slot, _)| availability[offset + slot])
                        .map(|(_, &minutes)| minutes as f64)
                        .sum()
                }
            };
            base * multiplier
        })
        .collect();

    let mut targets = vec![0i64; count];
    let mut pinned = vec![false; count];

    // Repeatedly hand out the remaining demand to whoever is not yet pinned at
    // zero. Each pass pins at least one more person, so this terminates.
    for _ in 0..=count {
        let active_share: f64 = (0..count).filter(|&h| !pinned[h]).map(|h| raw[h]).sum();
        let active_prior: i64 = (0..count)
            .filter(|&h| !pinned[h])
            .map(|h| prior_workload[h])
            .sum();

        if active_share <= 0.0 {
            break;
        }

        let notional = (demand + active_prior) as f64;
        let mut changed = false;

        for human in 0..count {
            if pinned[human] {
                continue;
            }

            let target = notional * (raw[human] / active_share) - prior_workload[human] as f64;
            if target < 0.0 {
                pinned[human] = true;
                targets[human] = 0;
                changed = true;
            } else {
                targets[human] = target.round() as i64;
            }
        }

        if !changed {
            break;
        }
    }

    // Rounding leaves the targets a few minutes off the true demand. Nudge the
    // largest target so that perfect fairness remains exactly achievable and
    // therefore scores zero.
    let residual = demand - targets.iter().sum::<i64>();
    if residual != 0 {
        if let Some(largest) = (0..count)
            .filter(|&h| !pinned[h])
            .max_by_key(|&h| targets[h])
        {
            targets[largest] += residual;
        } else if count > 0 {
            targets[0] += residual;
        }
    }

    targets
}

/// Precomputes the per-person, per-slot preference bias.
fn build_bias(
    config: &Config,
    humans: &[String],
    slots: &[TimeRange],
    slot_minutes: &[i64],
) -> Result<(Vec<i64>, bool), ProblemError> {
    let mut bias = vec![0i64; humans.len() * slots.len()];
    let mut has_preferences = false;

    for (index, name) in humans.iter().enumerate() {
        let human = &config.humans[name];
        if human.preferences.is_empty() {
            continue;
        }

        let offset = index * slots.len();

        for preference in human.preferences.iter() {
            let (constraint, kind) = preference
                .resolve()
                .map_err(|err| ProblemError(format!("{}: {}", name, err)))?;

            // Run the preference through the same constraint machinery used for
            // availability: the ranges it leaves behind are the matching times.
            let matching = available_ranges(std::slice::from_ref(constraint), slots);
            let weight = weight_from_f64(preference.weight);
            let sign = match kind {
                PreferenceKind::Avoid => 1,
                PreferenceKind::Prefer => -1,
            };

            for (slot_index, slot) in slots.iter().enumerate() {
                let candidate = matching.partition_point(|range| range.start <= slot.start);
                let matched = candidate > 0 && {
                    let range = &matching[candidate - 1];
                    range.start <= slot.start && slot.end <= range.end
                };

                if matched {
                    has_preferences = true;
                    bias[offset + slot_index] +=
                        sign * weight * slot_minutes[slot_index] / WEIGHT_SCALE;
                }
            }
        }
    }

    if !has_preferences {
        return Ok((Vec::new(), false));
    }

    Ok((bias, true))
}

/// Determines where rotations begin.
fn build_rotations(config: &Config, slots: &[TimeRange]) -> Vec<SlotIdx> {
    let boundary = config.rotation.boundary.clone().unwrap_or_else(|| {
        // Reproduce a simple fixed rotation when nothing more specific is asked
        // for: a new shift every `shiftLength` slots.
        RotationBoundary::EverySlots(config.shift_length.num_days().max(1) as usize)
    });

    let mut starts = vec![0usize];

    match boundary {
        RotationBoundary::EverySlots(every) => {
            let every = every.max(1);
            let mut index = every;
            while index < slots.len() {
                starts.push(index);
                index += every;
            }
        }
        RotationBoundary::DayOfWeek(days) => {
            let days: Vec<Weekday> = days;
            for index in 1..slots.len() {
                let date = slots[index].start.date();
                let previous = slots[index - 1].start.date();

                // Only the first slot of a matching day opens a rotation, so a
                // day split into several atomic slots stays in one rotation.
                if date != previous && days.contains(&date.weekday()) {
                    starts.push(index);
                }
            }
        }
    }

    starts
}

/// Works out the longest unbroken shift each person's availability permits,
/// capped at the schedule-wide target.
///
/// Two slots count as joinable when they are adjacent in the schedule, not when
/// they are adjacent in wall-clock time — a Friday and the following Monday are
/// one shift on a weekdays-only rota.
fn build_achievable_runs(
    humans: &[String],
    slots: &[TimeRange],
    slot_minutes: &[i64],
    availability: &[bool],
    target_run_minutes: i64,
) -> Vec<i64> {
    (0..humans.len())
        .map(|human| {
            let offset = human * slots.len();
            let mut longest = 0;
            let mut current = 0;

            for slot in 0..slots.len() {
                if availability[offset + slot] {
                    current += slot_minutes[slot];
                    longest = longest.max(current);

                    // No point looking further once they can already reach the
                    // schedule-wide target.
                    if longest >= target_run_minutes {
                        return target_run_minutes;
                    }
                } else {
                    current = 0;
                }
            }

            longest.max(1).min(target_run_minutes)
        })
        .collect()
}

/// Determines the atomic units of assignment.
///
/// Without rotation locking each slot stands alone. With it, whole rotations
/// move together, and a person is only eligible for a rotation if they can
/// cover every slot in it — which is the price of guaranteeing that handoffs
/// land exactly on rotation boundaries.
fn build_units(
    locked: bool,
    slots: &[TimeRange],
    rotation_starts: &[SlotIdx],
    domains: &[Vec<HumanIdx>],
    availability: &[bool],
    human_count: usize,
) -> (Vec<std::ops::Range<SlotIdx>>, Vec<Vec<HumanIdx>>, Vec<usize>) {
    if !locked {
        let units: Vec<_> = (0..slots.len()).map(|slot| slot..slot + 1).collect();
        let unit_of_slot = (0..slots.len()).collect();
        return (units, domains.to_vec(), unit_of_slot);
    }

    let mut units = Vec::with_capacity(rotation_starts.len());
    for (index, &start) in rotation_starts.iter().enumerate() {
        let end = rotation_starts
            .get(index + 1)
            .copied()
            .unwrap_or(slots.len());
        units.push(start..end);
    }

    let unit_domains: Vec<Vec<HumanIdx>> = units
        .iter()
        .map(|unit| {
            (0..human_count)
                .filter(|&human| {
                    unit.clone()
                        .all(|slot| availability[human * slots.len() + slot])
                })
                .collect()
        })
        .collect();

    let mut unit_of_slot = vec![0usize; slots.len()];
    for (index, unit) in units.iter().enumerate() {
        for slot in unit.clone() {
            unit_of_slot[slot] = index;
        }
    }

    (units, unit_domains, unit_of_slot)
}

/// Converts a chrono duration to whole minutes, for config plumbing.
#[allow(dead_code)]
pub fn minutes(duration: Duration) -> i64 {
    duration.num_minutes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Human, Preference, Rotation, Rules};
    use crate::constraints::Constraint;
    use chrono::NaiveTime;

    fn horizon(days: i64) -> (NaiveDateTime, NaiveDateTime) {
        let start = date_time!(2023, 1, 2);
        (start, start + Duration::days(days))
    }

    fn weekday_config(humans: Vec<(&str, Human)>) -> Config {
        Config::for_test(
            Duration::days(1),
            humans
                .into_iter()
                .map(|(name, human)| (name.to_string(), human))
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
        ])
    }

    #[test]
    fn humans_are_indexed_in_sorted_order() {
        let config = weekday_config(vec![
            ("zoe@example.com", Human::default()),
            ("alice@example.com", Human::default()),
            ("mike@example.com", Human::default()),
        ]);

        let (start, end) = horizon(7);
        let problem = Problem::build(&config, start, end).unwrap();

        assert_eq!(
            problem.humans,
            vec!["alice@example.com", "mike@example.com", "zoe@example.com"],
            "indices must not depend on HashMap iteration order"
        );
    }

    #[test]
    fn slots_are_contiguous_and_non_overlapping() {
        let config = weekday_config(vec![("alice@example.com", Human::default())]);
        let (start, end) = horizon(14);
        let problem = Problem::build(&config, start, end).unwrap();

        for window in problem.slots.windows(2) {
            assert!(
                window[0].end <= window[1].start,
                "slots must not overlap: {:?} then {:?}",
                window[0],
                window[1]
            );
        }

        assert!(problem.slot_minutes.iter().all(|&m| m > 0));
    }

    #[test]
    fn partial_availability_splits_slots_instead_of_discarding_them() {
        // Alice can only cover mornings; the schedule wants 09:00-17:00.
        // Without atomic decomposition her half-day would be unusable.
        let config = weekday_config(vec![
            (
                "alice@example.com",
                Human::default().with_constraints(vec![Constraint::TimeOfDay {
                    start: time!(9, 0),
                    end: time!(13, 0),
                }]),
            ),
            ("bob@example.com", Human::default()),
        ]);

        let (start, end) = horizon(3);
        let problem = Problem::build(&config, start, end).unwrap();

        let alice = problem.human_index("alice@example.com").unwrap();
        let bob = problem.human_index("bob@example.com").unwrap();

        // Every 8h day should have been split into a 4h morning and 4h afternoon.
        assert!(
            problem.slots.iter().any(|s| s.len().num_hours() == 4),
            "expected the day to be split at Alice's availability boundary"
        );

        let alice_minutes: i64 = (0..problem.slot_count())
            .filter(|&s| problem.is_available(alice, s))
            .map(|s| problem.slot_minutes[s])
            .sum();

        assert!(alice_minutes > 0, "Alice's mornings must remain usable");
        assert!(
            (0..problem.slot_count()).all(|s| problem.is_available(bob, s)),
            "Bob has no constraints so must be available everywhere"
        );

        // Availability must be all-or-nothing per atomic slot.
        for slot in 0..problem.slot_count() {
            if problem.is_available(alice, slot) {
                let range = problem.slots[slot];
                assert!(
                    range.start.time() >= time!(9, 0) && range.end.time() <= time!(13, 0),
                    "Alice should only be available within her window, got {range}"
                );
            }
        }
    }

    #[test]
    fn domains_exclude_unavailable_people() {
        let config = weekday_config(vec![
            (
                "alice@example.com",
                Human::default().with_constraints(vec![Constraint::DayOfWeek(vec![Weekday::Mon])]),
            ),
            ("bob@example.com", Human::default()),
        ]);

        let (start, end) = horizon(7);
        let problem = Problem::build(&config, start, end).unwrap();
        let alice = problem.human_index("alice@example.com").unwrap();

        for slot in 0..problem.slot_count() {
            let is_monday = problem.slots[slot].start.date().weekday() == Weekday::Mon;
            assert_eq!(
                problem.domains[slot].contains(&alice),
                is_monday,
                "Alice should only appear in Monday domains"
            );
        }
    }

    #[test]
    fn targets_sum_to_total_demand() {
        let config = weekday_config(vec![
            ("alice@example.com", Human::default()),
            ("bob@example.com", Human::default()),
            ("claire@example.com", Human::default()),
        ]);

        let (start, end) = horizon(28);
        let problem = Problem::build(&config, start, end).unwrap();

        assert_eq!(
            problem.targets.iter().sum::<i64>(),
            problem.total_demand(),
            "perfect fairness must be exactly achievable"
        );
    }

    #[test]
    fn capacity_mode_scales_targets_by_availability() {
        let config = weekday_config(vec![
            (
                "alice@example.com",
                // Available 3 of 5 weekdays.
                Human::default().with_constraints(vec![Constraint::DayOfWeek(vec![
                    Weekday::Mon,
                    Weekday::Wed,
                    Weekday::Fri,
                ])]),
            ),
            ("bob@example.com", Human::default()),
        ]);

        let (start, end) = horizon(70);
        let problem = Problem::build(&config, start, end).unwrap();

        let alice = problem.human_index("alice@example.com").unwrap();
        let bob = problem.human_index("bob@example.com").unwrap();

        assert!(
            problem.targets[alice] < problem.targets[bob],
            "a part-time engineer should be given a smaller target"
        );

        // Alice can cover 3/8 of the total availability pool (3 days vs 5).
        let ratio = problem.targets[alice] as f64 / problem.targets[bob] as f64;
        assert!(
            (ratio - 0.6).abs() < 0.05,
            "expected roughly a 3:5 split, got {ratio}"
        );
    }

    #[test]
    fn equal_mode_gives_everybody_the_same_target() {
        let config = weekday_config(vec![
            (
                "alice@example.com",
                Human::default().with_constraints(vec![Constraint::DayOfWeek(vec![
                    Weekday::Mon,
                    Weekday::Wed,
                    Weekday::Fri,
                ])]),
            ),
            ("bob@example.com", Human::default()),
        ])
        .with_fairness(FairnessMode::Equal);

        let (start, end) = horizon(70);
        let problem = Problem::build(&config, start, end).unwrap();

        let alice = problem.human_index("alice@example.com").unwrap();
        let bob = problem.human_index("bob@example.com").unwrap();

        assert!(
            (problem.targets[alice] - problem.targets[bob]).abs() <= 1,
            "equal mode should ignore availability, got {:?}",
            problem.targets
        );
    }

    #[test]
    fn explicit_capacity_overrides_derived_share() {
        let config = weekday_config(vec![
            ("alice@example.com", Human::default().with_capacity(0.5)),
            ("bob@example.com", Human::default()),
        ]);

        let (start, end) = horizon(70);
        let problem = Problem::build(&config, start, end).unwrap();

        let alice = problem.human_index("alice@example.com").unwrap();
        let bob = problem.human_index("bob@example.com").unwrap();

        let ratio = problem.targets[alice] as f64 / problem.targets[bob] as f64;
        assert!((ratio - 0.5).abs() < 0.02, "expected a 1:2 split, got {ratio}");
    }

    #[test]
    fn prior_workload_reduces_the_target() {
        let config = weekday_config(vec![
            (
                "alice@example.com",
                Human::default().with_prior_workload(Duration::hours(40)),
            ),
            ("bob@example.com", Human::default()),
        ]);

        let (start, end) = horizon(28);
        let problem = Problem::build(&config, start, end).unwrap();

        let alice = problem.human_index("alice@example.com").unwrap();
        let bob = problem.human_index("bob@example.com").unwrap();

        assert!(
            problem.targets[alice] < problem.targets[bob],
            "prior workload should be worked off, got {:?}",
            problem.targets
        );
        assert_eq!(problem.targets.iter().sum::<i64>(), problem.total_demand());
    }

    #[test]
    fn overwhelming_prior_workload_pins_a_target_at_zero() {
        let config = weekday_config(vec![
            (
                "alice@example.com",
                Human::default().with_prior_workload(Duration::hours(100_000)),
            ),
            ("bob@example.com", Human::default()),
        ]);

        let (start, end) = horizon(14);
        let problem = Problem::build(&config, start, end).unwrap();

        let alice = problem.human_index("alice@example.com").unwrap();
        assert_eq!(problem.targets[alice], 0);
        assert_eq!(problem.targets.iter().sum::<i64>(), problem.total_demand());
    }

    #[test]
    fn preferences_produce_signed_bias() {
        let config = weekday_config(vec![
            (
                "alice@example.com",
                Human::default().with_preferences(vec![
                    Preference {
                        avoid: Some(Constraint::DayOfWeek(vec![Weekday::Fri])),
                        prefer: None,
                        weight: 2.0,
                    },
                    Preference {
                        avoid: None,
                        prefer: Some(Constraint::DayOfWeek(vec![Weekday::Mon])),
                        weight: 1.0,
                    },
                ]),
            ),
            ("bob@example.com", Human::default()),
        ]);

        let (start, end) = horizon(7);
        let problem = Problem::build(&config, start, end).unwrap();
        let alice = problem.human_index("alice@example.com").unwrap();
        let bob = problem.human_index("bob@example.com").unwrap();

        assert!(problem.has_preferences);

        for slot in 0..problem.slot_count() {
            let weekday = problem.slots[slot].start.date().weekday();
            let bias = problem.bias(alice, slot);

            match weekday {
                Weekday::Fri => assert!(bias > 0, "Fridays should be discouraged, got {bias}"),
                Weekday::Mon => assert!(bias < 0, "Mondays should be encouraged, got {bias}"),
                _ => assert_eq!(bias, 0),
            }

            assert_eq!(problem.bias(bob, slot), 0, "Bob has no preferences");
        }
    }

    #[test]
    fn no_preferences_means_no_bias_grid() {
        let config = weekday_config(vec![("alice@example.com", Human::default())]);
        let (start, end) = horizon(7);
        let problem = Problem::build(&config, start, end).unwrap();

        assert!(!problem.has_preferences);
        assert_eq!(problem.bias(0, 0), 0);
    }

    #[test]
    fn rotations_default_to_fixed_length_chunks() {
        let config = Config::for_test(
            Duration::days(3),
            [("alice@example.com".to_string(), Human::default())]
                .into_iter()
                .collect(),
        )
        .with_constraints(vec![Constraint::TimeOfDay {
            start: time!(9, 0),
            end: time!(17, 0),
        }]);

        let (start, end) = horizon(9);
        let problem = Problem::build(&config, start, end).unwrap();

        assert_eq!(problem.rotation_starts, vec![0, 3, 6]);
        assert_eq!(problem.rotation_slots(0), 0..3);
        assert_eq!(problem.rotation_slots(2), 6..9);
    }

    #[test]
    fn day_of_week_rotation_boundaries_land_on_the_named_day() {
        let config = weekday_config(vec![("alice@example.com", Human::default())]).with_rotation(
            Rotation {
                lock: true,
                boundary: Some(RotationBoundary::DayOfWeek(vec![Weekday::Mon])),
            },
        );

        let (start, end) = horizon(21);
        let problem = Problem::build(&config, start, end).unwrap();

        for &slot in problem.rotation_starts.iter().skip(1) {
            assert_eq!(
                problem.slots[slot].start.date().weekday(),
                Weekday::Mon,
                "rotations should only start on Mondays"
            );
        }

        assert!(problem.rotation_count() >= 3);
    }

    #[test]
    fn rotation_slots_partition_the_schedule() {
        let config = weekday_config(vec![("alice@example.com", Human::default())]);
        let (start, end) = horizon(21);
        let problem = Problem::build(&config, start, end).unwrap();

        let mut covered = 0;
        for rotation in 0..problem.rotation_count() {
            let range = problem.rotation_slots(rotation);
            assert_eq!(range.start, covered, "rotations must be contiguous");
            covered = range.end;
        }

        assert_eq!(covered, problem.slot_count(), "rotations must cover everything");
    }

    #[test]
    fn rules_are_converted_to_minutes() {
        let config = weekday_config(vec![("alice@example.com", Human::default())]).with_rules(
            Rules {
                min_rest: Some(Duration::hours(48)),
                max_consecutive: Some(Duration::hours(72)),
                desired_rest: None,
            },
        );

        let (start, end) = horizon(7);
        let problem = Problem::build(&config, start, end).unwrap();

        assert_eq!(problem.min_rest_minutes, Some(48 * 60));
        assert_eq!(problem.max_consecutive_minutes, Some(72 * 60));
        // Defaults to the shift length when not set.
        assert_eq!(problem.desired_rest_minutes, 24 * 60);
    }

    #[test]
    fn an_empty_horizon_is_rejected() {
        let config = weekday_config(vec![("alice@example.com", Human::default())]);
        let start = date_time!(2023, 1, 7); // Saturday
        let end = date_time!(2023, 1, 8); // Sunday

        let error = Problem::build(&config, start, end).unwrap_err();
        assert!(
            error.to_string().contains("no slots"),
            "expected a helpful error, got {error}"
        );
    }

    #[test]
    fn freezing_pins_early_slots() {
        let config = weekday_config(vec![("alice@example.com", Human::default())]);
        let (start, end) = horizon(14);
        let mut problem = Problem::build(&config, start, end).unwrap();

        let frozen = problem.freeze_before(start + Duration::days(5));
        assert!(frozen > 0);

        for slot in 0..problem.slot_count() {
            let expected = problem.slots[slot].end <= start + Duration::days(5);
            assert_eq!(problem.is_frozen(slot), expected);
        }
    }

    #[test]
    fn unavailable_periods_remove_availability() {
        let config = weekday_config(vec![
            (
                "alice@example.com",
                Human::default().with_constraints(vec![Constraint::Unavailable {
                    start: date!(2023, 1, 2),
                    end: date!(2023, 1, 6),
                }]),
            ),
            ("bob@example.com", Human::default()),
        ]);

        let (start, end) = horizon(14);
        let problem = Problem::build(&config, start, end).unwrap();
        let alice = problem.human_index("alice@example.com").unwrap();

        for slot in 0..problem.slot_count() {
            let date = problem.slots[slot].start.date();
            if date >= date!(2023, 1, 2) && date < date!(2023, 1, 6) {
                assert!(
                    !problem.is_available(alice, slot),
                    "Alice is on leave on {date}"
                );
            }
        }
    }

    #[test]
    fn a_slot_nobody_can_cover_has_an_empty_domain() {
        let config = weekday_config(vec![(
            "alice@example.com",
            Human::default().with_constraints(vec![Constraint::DayOfWeek(vec![Weekday::Mon])]),
        )]);

        let (start, end) = horizon(7);
        let problem = Problem::build(&config, start, end).unwrap();

        assert!(
            problem.domains.iter().any(|d| d.is_empty()),
            "Tue-Fri should be uncoverable"
        );
        assert!(
            problem.domains.iter().any(|d| !d.is_empty()),
            "Monday should be coverable"
        );
    }

    #[test]
    fn scale_reflects_typical_slot_length() {
        let config = weekday_config(vec![("alice@example.com", Human::default())]);
        let (start, end) = horizon(14);
        let problem = Problem::build(&config, start, end).unwrap();

        assert_eq!(problem.scale, 8 * 60, "slots are eight hours long");
    }

    #[test]
    fn human_index_round_trips() {
        let config = weekday_config(vec![
            ("alice@example.com", Human::default()),
            ("bob@example.com", Human::default()),
        ]);

        let (start, end) = horizon(7);
        let problem = Problem::build(&config, start, end).unwrap();

        for (index, name) in problem.humans.iter().enumerate() {
            assert_eq!(problem.human_index(name), Some(index));
        }
        assert_eq!(problem.human_index("nobody@example.com"), None);
    }

    #[test]
    fn zero_length_horizon_slots_are_dropped() {
        let config = weekday_config(vec![("alice@example.com", Human::default())]);
        let start = date_time!(2023, 1, 2, 9, 0, 0);
        let end = date_time!(2023, 1, 2, 9, 0, 0);

        assert!(Problem::build(&config, start, end).is_err());
    }

    #[test]
    fn invalid_preferences_are_reported_with_the_person_name() {
        let config = weekday_config(vec![(
            "alice@example.com",
            Human::default().with_preferences(vec![Preference {
                avoid: Some(Constraint::None),
                prefer: Some(Constraint::None),
                weight: 1.0,
            }]),
        )]);

        let (start, end) = horizon(7);
        let error = Problem::build(&config, start, end).unwrap_err();
        assert!(error.to_string().contains("alice@example.com"), "{error}");
    }

    #[test]
    fn time_of_day_wrap_produces_usable_slots() {
        // An overnight schedule: 20:00 through 04:00.
        let config = Config::for_test(
            Duration::days(1),
            [("alice@example.com".to_string(), Human::default())]
                .into_iter()
                .collect(),
        )
        .with_constraints(vec![Constraint::TimeOfDay {
            start: NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
            end: NaiveTime::from_hms_opt(4, 0, 0).unwrap(),
        }]);

        let (start, end) = horizon(7);
        let problem = Problem::build(&config, start, end).unwrap();

        assert!(problem.slot_count() > 0);
        assert!(problem.slot_minutes.iter().all(|&m| m > 0));
    }
}

