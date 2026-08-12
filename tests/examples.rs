//! End-to-end tests over the shipped example configurations.
//!
//! These exercise the whole pipeline — YAML in, schedule out — and assert the
//! properties users actually care about: everybody gets covered, nobody gets
//! stitched up, the rules are honoured, and re-running produces the same answer.

use chrono::{Duration, NaiveDate, NaiveDateTime};
use on_call::config::Config;
use on_call::model::{Assignment, Problem};
use on_call::objectives;
use on_call::schedule::Schedule;
use on_call::search::{self, construct::construct, Options};

fn midnight(year: i32, month: u32, day: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
}

fn load(name: &str) -> Config {
    let path = format!("{}/examples/{}", env!("CARGO_MANIFEST_DIR"), name);
    let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("opening {path}: {e}"));
    serde_yaml::from_reader(file).unwrap_or_else(|e| panic!("parsing {path}: {e}"))
}

fn problem_for(name: &str, days: i64) -> Problem {
    let config = load(name);
    config.validate().expect("example configs must be valid");

    Problem::build(
        &config,
        midnight(2023, 1, 2),
        midnight(2023, 1, 2) + Duration::days(days),
    )
    .expect("example configs must produce a solvable problem")
}

/// Step budget for the integration tests.
///
/// Deliberately lower than the CLI default: these tests assert that the
/// optimizer reaches a good schedule, not that it squeezes out the last few
/// points, and they run in debug builds where every step is expensive.
const STEPS: u64 = 60_000;

fn solve(problem: &Problem, seed: u64) -> Assignment {
    search::solve(
        problem,
        &Options {
            steps: STEPS,
            seed,
            ..Options::default()
        },
    )
    .0
}

const EXAMPLES: [&str; 4] = [
    "rotation.yaml",
    "3-day.yaml",
    "weekly.yaml",
    "constrained.yaml",
];

#[test]
fn every_example_config_parses_and_validates() {
    for example in EXAMPLES {
        let config = load(example);
        config
            .validate()
            .unwrap_or_else(|e| panic!("{example} failed validation: {e}"));
    }
}

#[test]
fn every_example_produces_a_fully_covered_schedule() {
    for example in EXAMPLES {
        let problem = problem_for(example, 120);
        let assignment = solve(&problem, 1);

        assert_eq!(
            assignment.unassigned_count(),
            0,
            "{example} left slots uncovered"
        );
    }
}

#[test]
fn every_example_only_schedules_available_people() {
    for example in EXAMPLES {
        let problem = problem_for(example, 120);
        let assignment = solve(&problem, 2);

        for slot in 0..problem.slot_count() {
            if let Some(human) = assignment.get(slot) {
                assert!(
                    problem.is_available(human, slot),
                    "{example} scheduled {} for {} which they cannot cover",
                    problem.humans[human],
                    problem.slots[slot]
                );
            }
        }
    }
}

#[test]
fn every_example_is_reproducible() {
    for example in EXAMPLES {
        let problem = problem_for(example, 120);

        let first = solve(&problem, 7);
        let second = solve(&problem, 7);

        assert_eq!(
            first.slots(),
            second.slots(),
            "{example} produced different schedules across runs"
        );
    }
}

#[test]
fn optimization_improves_on_greedy_construction_for_every_example() {
    for example in EXAMPLES {
        let problem = problem_for(example, 120);

        let greedy = objectives::evaluate(&problem, &construct(&problem));
        let optimized = objectives::evaluate(&problem, &solve(&problem, 3));

        assert!(
            optimized <= greedy,
            "{example}: optimization made things worse ({greedy} then {optimized})"
        );
    }
}

#[test]
fn workloads_land_close_to_their_targets() {
    for example in EXAMPLES {
        let problem = problem_for(example, 180);
        let assignment = solve(&problem, 4);

        for human in 0..problem.human_count() {
            let assigned = assignment.assigned_minutes(human);
            let target = problem.targets[human];
            let deviation = (assigned - target).abs();

            // Slots are indivisible, so exact targets are rarely achievable.
            // Landing within two average slots of the target is a reasonable
            // bar, and catches any real fairness regression.
            assert!(
                deviation <= problem.scale * 2,
                "{example}: {} is {}h from their {}h target",
                problem.humans[human],
                deviation / 60,
                target / 60
            );
        }
    }
}

#[test]
fn a_baseline_makes_a_rerun_a_no_op_whatever_the_seed() {
    // Feeding a schedule back in as its own baseline is the commonest way this
    // tool gets used: re-plan after a config tweak without disrupting a rota
    // people have already arranged their lives around. Nothing has changed
    // here, so nothing should move — and that must not depend on the optimizer
    // happening to walk the same path, so every seed is checked.
    let problem = problem_for("3-day.yaml", 120);
    let original = solve(&problem, 5);
    let schedule = Schedule::from_assignment(&problem, &original);

    let mut anchored = problem_for("3-day.yaml", 120);
    let (baseline, report) = schedule.to_baseline(&anchored);
    assert!(!report.has_warnings());
    assert_eq!(report.matched_slots, anchored.slot_count());
    anchored = anchored.with_baseline(baseline);

    for seed in [5, 11, 42, 99] {
        assert_eq!(
            solve(&anchored, seed).slots(),
            original.slots(),
            "re-running against its own output moved shifts (seed {seed})"
        );
    }
}

#[test]
fn without_a_baseline_a_different_seed_is_free_to_reshuffle() {
    // The counterpart to the test above: churn is only suppressed because the
    // baseline suppresses it, not because the search is incapable of finding a
    // different schedule of comparable quality.
    let problem = problem_for("3-day.yaml", 120);
    let original = solve(&problem, 5);

    let churn = (0..problem.slot_count())
        .filter(|&slot| solve(&problem, 42).get(slot) != original.get(slot))
        .count();

    assert!(
        churn > problem.slot_count() / 10,
        "expected an unanchored re-run to differ substantially, got {churn} of {}",
        problem.slot_count()
    );
}

#[test]
fn frozen_slots_are_never_moved() {
    let problem = problem_for("3-day.yaml", 120);
    let original = solve(&problem, 6);
    let schedule = Schedule::from_assignment(&problem, &original);

    let mut anchored = problem_for("3-day.yaml", 120);
    let (baseline, _) = schedule.to_baseline(&anchored);
    anchored = anchored.with_baseline(baseline.clone());
    let frozen_count = anchored.freeze_before(midnight(2023, 3, 1));
    assert!(frozen_count > 0);

    let rerun = solve(&anchored, 99);

    for (slot, &owner) in baseline.iter().enumerate() {
        if anchored.is_frozen(slot) {
            assert_eq!(
                rerun.get(slot),
                owner,
                "frozen slot {} was reassigned",
                anchored.slots[slot]
            );
        }
    }
}

#[test]
fn hard_rules_in_the_constrained_example_are_honoured() {
    let problem = problem_for("constrained.yaml", 180);
    let assignment = solve(&problem, 8);

    let max_consecutive = problem
        .max_consecutive_minutes
        .expect("the constrained example should set maxConsecutiveHours");
    let min_rest = problem
        .min_rest_minutes
        .expect("the constrained example should set minRestHours");

    for human in 0..problem.human_count() {
        let runs: Vec<_> = assignment.slots_of(human).runs().collect();

        for &(start, end) in runs.iter() {
            assert!(
                problem.covered_minutes(start, end) <= max_consecutive,
                "{} exceeded the maximum shift length",
                problem.humans[human]
            );
        }

        for pair in runs.windows(2) {
            assert!(
                problem.gap_minutes(pair[0].1, pair[1].0) >= min_rest,
                "{} did not get the minimum rest between shifts",
                problem.humans[human]
            );
        }
    }
}

#[test]
fn rotation_locking_keeps_handoffs_on_boundaries() {
    let config = load("locked.yaml");
    let problem = Problem::build(&config, midnight(2023, 1, 2), midnight(2023, 4, 2)).unwrap();
    let assignment = solve(&problem, 9);

    assert!(problem.rotation_locked);

    for unit in 0..problem.unit_count() {
        let owners: Vec<_> = problem.units[unit]
            .clone()
            .map(|slot| assignment.get(slot))
            .collect();

        assert!(
            owners.windows(2).all(|pair| pair[0] == pair[1]),
            "rotation {unit} was split between people: {owners:?}"
        );
    }
}

#[test]
fn preferences_are_respected_when_they_are_affordable() {
    let problem = problem_for("constrained.yaml", 180);
    let assignment = solve(&problem, 10);

    // Nobody should be landed with a large pile of shifts they asked to avoid
    // when the team has the slack to arrange otherwise.
    for human in 0..problem.human_count() {
        let avoided: i64 = assignment
            .slots_of(human)
            .iter()
            .map(|slot| problem.bias(human, slot).max(0))
            .sum();

        let total: i64 = (0..problem.slot_count())
            .map(|slot| problem.bias(human, slot).max(0))
            .sum();

        if total > 0 {
            assert!(
                avoided * 4 <= total,
                "{} was given most of the slots they asked to avoid",
                problem.humans[human]
            );
        }
    }
}

#[test]
fn the_schedule_covers_the_requested_horizon_exactly() {
    let problem = problem_for("rotation.yaml", 30);
    let assignment = solve(&problem, 12);
    let schedule = Schedule::from_assignment(&problem, &assignment);

    assert_eq!(
        schedule.shifts.first().unwrap().time.start,
        problem.slots[0].start
    );
    assert_eq!(
        schedule.shifts.last().unwrap().time.end,
        problem.slots[problem.slot_count() - 1].end
    );

    // Shifts must be ordered and must never overlap. They are allowed to have
    // gaps between them: the rota only covers working hours, so the time
    // between Friday evening and Monday morning is genuinely uncovered and
    // must not be presented as though somebody were on-call.
    for pair in schedule.shifts.windows(2) {
        assert!(
            pair[0].time.end <= pair[1].time.start,
            "{:?} overlaps {:?}",
            pair[0],
            pair[1]
        );
    }

    let covered: i64 = schedule
        .shifts
        .iter()
        .map(|shift| shift.time.len().num_minutes())
        .sum();
    assert_eq!(
        covered,
        problem.total_demand(),
        "the shifts should account for exactly the scheduled time, no more"
    );
}

#[test]
fn schedules_round_trip_through_json() {
    let problem = problem_for("weekly.yaml", 60);
    let assignment = solve(&problem, 13);
    let schedule = Schedule::from_assignment(&problem, &assignment);

    let encoded = serde_json::to_string(&schedule).unwrap();
    let decoded: Schedule = serde_json::from_str(&encoded).unwrap();

    assert_eq!(decoded, schedule);
    assert_eq!(decoded.to_baseline(&problem).0, assignment.slots().to_vec());
}

#[test]
fn a_single_person_team_still_produces_a_schedule() {
    let config: Config = serde_yaml::from_str(
        r#"
        shiftLength: 1
        constraints:
          - !DayOfWeek [Mon, Tue, Wed, Thu, Fri]
          - !TimeOfDay
            start: 09:00:00
            end: 17:00:00
        humans:
          solo@example.com: {}
        "#,
    )
    .unwrap();

    let problem = Problem::build(&config, midnight(2023, 1, 2), midnight(2023, 2, 2)).unwrap();
    let assignment = solve(&problem, 14);

    assert_eq!(assignment.unassigned_count(), 0);
    assert_eq!(
        assignment.assigned_minutes(0),
        problem.total_demand(),
        "the only available person should cover everything"
    );
}

#[test]
fn an_impossible_slot_is_reported_rather_than_hidden() {
    let config: Config = serde_yaml::from_str(
        r#"
        shiftLength: 1
        constraints:
          - !DayOfWeek [Mon, Tue, Wed, Thu, Fri]
          - !TimeOfDay
            start: 09:00:00
            end: 17:00:00
        humans:
          parttime@example.com:
            constraints:
              - !DayOfWeek [Mon]
        "#,
    )
    .unwrap();

    let problem = Problem::build(&config, midnight(2023, 1, 2), midnight(2023, 1, 9)).unwrap();
    let assignment = solve(&problem, 15);
    let schedule = Schedule::from_assignment(&problem, &assignment);

    assert!(schedule.has_gaps(), "the gap should be visible in the output");
    assert!(!objectives::evaluate(&problem, &assignment).is_feasible());
}
