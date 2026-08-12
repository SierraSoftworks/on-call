//! Reporting: what the optimizer produced, and how good it is.

use std::collections::BTreeMap;
use std::fmt::Display;

use crate::model::{Assignment, Problem, Score};
use crate::objectives;
use crate::search::Statistics;

/// A human-readable assessment of a schedule.
pub struct Summary {
    score: Score,
    breakdown: Vec<(&'static str, Score)>,
    people: Vec<PersonSummary>,
    shift_histogram: BTreeMap<i64, usize>,
    uncovered_minutes: i64,
    uncovered_slots: usize,
    statistics: Option<Statistics>,
}

/// Per-person figures.
pub struct PersonSummary {
    pub name: String,
    /// On-call minutes in this schedule.
    pub assigned: i64,
    /// On-call minutes this person should ideally have had.
    pub target: i64,
    /// On-call minutes carried in from previous runs.
    pub prior: i64,
    /// Longest single shift, in minutes.
    pub longest_shift: i64,
    /// Shortest rest between two shifts, in minutes.
    pub shortest_rest: Option<i64>,
    pub shifts: usize,
}

impl PersonSummary {
    /// How far from the ideal this person ended up. Positive means overworked.
    pub fn deviation(&self) -> i64 {
        self.assigned - self.target
    }

    /// The `priorWorkload` to carry into the next scheduling run so that this
    /// person's share stays balanced over time.
    ///
    /// This is simply the deviation: prior workload has already been worked
    /// into the target, so somebody who hit their reduced target has cleared
    /// their debt and starts the next run level. Adding the old prior workload
    /// back in here would charge them for it twice.
    pub fn carry_forward(&self) -> i64 {
        self.deviation()
    }
}

impl Summary {
    /// Assesses a schedule.
    pub fn new(problem: &Problem, assignment: &Assignment) -> Self {
        let mut people = Vec::with_capacity(problem.human_count());
        let mut shift_histogram: BTreeMap<i64, usize> = BTreeMap::new();

        for human in 0..problem.human_count() {
            let runs: Vec<_> = assignment.slots_of(human).runs().collect();

            let mut longest_shift = 0;
            for &(start, end) in runs.iter() {
                let minutes = problem.covered_minutes(start, end);
                longest_shift = longest_shift.max(minutes);
                *shift_histogram.entry(minutes / 60).or_default() += 1;
            }

            let shortest_rest = runs
                .windows(2)
                .map(|pair| problem.gap_minutes(pair[0].1, pair[1].0))
                .min();

            people.push(PersonSummary {
                name: problem.humans[human].clone(),
                assigned: assignment.assigned_minutes(human),
                target: problem.targets[human],
                prior: problem.prior_workload[human],
                longest_shift,
                shortest_rest,
                shifts: runs.len(),
            });
        }

        // Worst offenders first, so the interesting rows are at the top.
        people.sort_by(|a, b| {
            b.deviation()
                .abs()
                .cmp(&a.deviation().abs())
                .then_with(|| a.name.cmp(&b.name))
        });

        let uncovered: Vec<usize> = (0..problem.slot_count())
            .filter(|&slot| assignment.get(slot).is_none())
            .collect();

        Self {
            score: objectives::evaluate(problem, assignment),
            breakdown: objectives::breakdown(problem, assignment),
            people,
            shift_histogram,
            uncovered_minutes: uncovered.iter().map(|&s| problem.slot_minutes[s]).sum(),
            uncovered_slots: uncovered.len(),
            statistics: None,
        }
    }

    /// Attaches search statistics for reporting.
    pub fn with_statistics(mut self, statistics: Statistics) -> Self {
        self.statistics = Some(statistics);
        self
    }

    pub fn score(&self) -> Score {
        self.score
    }

    pub fn people(&self) -> &[PersonSummary] {
        &self.people
    }

    /// Minimum, mean and maximum assigned hours across the team.
    pub fn workload_stats(&self) -> (i64, i64, i64) {
        stats(self.people.iter().map(|person| person.assigned / 60))
    }

    /// Minimum, mean and maximum longest-shift hours across the team.
    pub fn longest_shift_stats(&self) -> (i64, i64, i64) {
        stats(self.people.iter().map(|person| person.longest_shift / 60))
    }

    /// The largest gap between anybody's actual and ideal workload, in hours.
    #[allow(dead_code)]
    pub fn fairness_spread(&self) -> i64 {
        self.people
            .iter()
            .map(|person| person.deviation().abs() / 60)
            .max()
            .unwrap_or(0)
    }

    /// Renders the per-objective breakdown, for `--explain`.
    pub fn explain(&self) -> String {
        use std::fmt::Write;

        let mut out = String::new();
        let _ = writeln!(out, "Score: {}", self.score);
        let _ = writeln!(out);
        let _ = writeln!(out, "Objectives:");

        let mut rows: Vec<_> = self.breakdown.iter().collect();
        rows.sort_by_key(|(_, score)| (-score.hard, -score.soft));

        for (name, score) in rows {
            if *score == Score::ZERO {
                let _ = writeln!(out, "  {name:<12} -");
            } else {
                let _ = writeln!(out, "  {name:<12} {score}");
            }
        }

        if let Some(statistics) = self.statistics {
            let _ = writeln!(out);
            let _ = writeln!(out, "Search:");
            let _ = writeln!(out, "  steps         {}", statistics.steps);
            let _ = writeln!(out, "  accepted      {}", statistics.accepted);
            let _ = writeln!(out, "  improvements  {}", statistics.improvements);
            let _ = writeln!(out, "  restarts      {}", statistics.kicks);
            let _ = writeln!(out, "  initial score {}", statistics.initial);
            let _ = writeln!(out, "  final score   {}", statistics.best);
        }

        out
    }
}

fn stats<I: IntoIterator<Item = i64>>(items: I) -> (i64, i64, i64) {
    let items: Vec<i64> = items.into_iter().collect();
    if items.is_empty() {
        return (0, 0, 0);
    }

    let min = *items.iter().min().unwrap();
    let max = *items.iter().max().unwrap();
    let mean = items.iter().sum::<i64>() / items.len() as i64;

    (min, mean, max)
}

/// Formats a minute count as hours, keeping the sign for deviations.
fn hours(minutes: i64) -> String {
    format!("{}h", minutes / 60)
}

impl Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Score: {}", self.score)?;

        if self.uncovered_slots > 0 {
            writeln!(
                f,
                "  WARNING: {} of on-call time across {} slots has nobody available to cover it",
                hours(self.uncovered_minutes),
                self.uncovered_slots
            )?;
        }

        writeln!(f)?;

        let (min, mean, max) = self.workload_stats();
        writeln!(f, "Workload: (min: {min}h, avg: {mean}h, max: {max}h)")?;
        writeln!(
            f,
            "  {:<28} {:>8} {:>8} {:>8}  carry forward",
            "", "actual", "target", "delta"
        )?;

        for person in self.people.iter() {
            let deviation = person.deviation();
            writeln!(
                f,
                "  {:<28} {:>8} {:>8} {:>8}  {}",
                person.name,
                hours(person.assigned),
                hours(person.target),
                format!("{}{}", if deviation > 0 { "+" } else { "" }, hours(deviation)),
                hours(person.carry_forward()),
            )?;
        }

        writeln!(f)?;
        let (min, mean, max) = self.longest_shift_stats();
        writeln!(f, "Longest shift: (min: {min}h, avg: {mean}h, max: {max}h)")?;
        for person in self.people.iter() {
            let rest = person
                .shortest_rest
                .map(|rest| format!(", shortest rest {}", hours(rest)))
                .unwrap_or_default();

            writeln!(
                f,
                "  {:<28} {:>5} across {} shifts{}",
                person.name,
                hours(person.longest_shift),
                person.shifts,
                rest
            )?;
        }

        if !self.shift_histogram.is_empty() {
            writeln!(f)?;
            writeln!(f, "Shift length histogram:")?;

            let widest = self
                .shift_histogram
                .values()
                .copied()
                .max()
                .unwrap_or(1)
                .max(1);

            for (length, count) in self.shift_histogram.iter().rev() {
                let bar = "#".repeat(((count * 30) / widest).max(1));
                writeln!(f, "  {length:>3}h | {count:>4} {bar}")?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objectives::testing::{fixture, fixture_hard};
    use crate::search::construct::construct;
    use crate::search::{solve, Options};

    #[test]
    fn a_summary_reports_every_person() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let summary = Summary::new(problem, &construct(problem));

        assert_eq!(summary.people().len(), problem.human_count());

        let mut names: Vec<&str> = summary.people().iter().map(|p| p.name.as_str()).collect();
        names.sort();
        assert_eq!(names, problem.humans.iter().map(|h| h.as_str()).collect::<Vec<_>>());
    }

    #[test]
    fn assigned_totals_match_the_assignment() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let assignment = construct(problem);
        let summary = Summary::new(problem, &assignment);

        let total: i64 = summary.people().iter().map(|p| p.assigned).sum();
        assert_eq!(total, problem.total_demand());
    }

    #[test]
    fn deviation_is_the_gap_between_actual_and_target() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let summary = Summary::new(problem, &construct(problem));

        for person in summary.people() {
            assert_eq!(person.deviation(), person.assigned - person.target);
        }
    }

    #[test]
    fn carry_forward_can_credit_somebody_who_is_behind() {
        // Being owed on-call time is as real as owing it, so carry-forward is
        // signed. Feeding a negative value back in as priorWorkload raises that
        // person's target next time round.
        let fixture = fixture_hard();
        let problem = &fixture.problem;
        let summary = Summary::new(problem, &construct(problem));

        assert!(
            summary
                .people()
                .iter()
                .any(|person| person.carry_forward() < 0),
            "greedy construction should leave somebody short"
        );
    }

    #[test]
    fn uncovered_time_is_reported() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let assignment = Assignment::empty(problem);
        let summary = Summary::new(problem, &assignment);

        assert_eq!(summary.uncovered_slots, problem.slot_count());
        assert_eq!(summary.uncovered_minutes, problem.total_demand());
        assert!(format!("{summary}").contains("WARNING"));
    }

    #[test]
    fn a_covered_schedule_carries_no_warning() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let summary = Summary::new(problem, &construct(problem));

        assert!(!format!("{summary}").contains("WARNING"));
    }

    #[test]
    fn the_optimizer_produces_a_tighter_spread_than_construction() {
        let fixture = fixture_hard();
        let problem = &fixture.problem;

        let greedy = Summary::new(problem, &construct(problem));
        let (optimized, _) = solve(
            problem,
            &Options {
                steps: 200_000,
                seed: 1,
                ..Options::default()
            },
        );
        let optimized = Summary::new(problem, &optimized);

        assert!(
            optimized.score() < greedy.score(),
            "optimized {} should beat greedy {}",
            optimized.score(),
            greedy.score()
        );
    }

    #[test]
    fn explain_lists_every_objective() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let summary = Summary::new(problem, &construct(problem));
        let explained = summary.explain();

        for name in objectives::NAMES {
            assert!(explained.contains(name), "missing {name} in:\n{explained}");
        }
    }

    #[test]
    fn explain_includes_search_statistics_when_available() {
        let fixture = fixture();
        let problem = &fixture.problem;
        let (assignment, statistics) = solve(
            problem,
            &Options {
                steps: 5_000,
                ..Options::default()
            },
        );

        let summary = Summary::new(problem, &assignment).with_statistics(statistics);
        let explained = summary.explain();

        assert!(explained.contains("Search:"));
        assert!(explained.contains("steps"));
    }

    #[test]
    fn carry_forward_is_the_deviation_not_the_accumulated_prior() {
        // Regression: carry-forward used to add the existing prior workload to
        // the deviation, which charged people twice for time they had already
        // worked off.
        let fixture = fixture_hard();
        let problem = &fixture.problem;
        let summary = Summary::new(problem, &construct(problem));

        for person in summary.people() {
            assert_eq!(person.carry_forward(), person.deviation());
        }

        // Somebody carrying prior workload who hits their reduced target has
        // cleared it, and should carry nothing into the next run.
        let claire = summary
            .people()
            .iter()
            .find(|person| person.name == "claire@example.com")
            .expect("claire is in the hard fixture");

        assert!(claire.prior > 0, "claire should be carrying prior workload");
        assert_eq!(
            claire.carry_forward(),
            claire.assigned - claire.target,
            "clearing the debt should leave nothing to carry"
        );
    }

    #[test]
    fn a_negative_carry_forward_round_trips_through_the_config() {
        use crate::config::Human;
        use chrono::Duration;

        let human = Human::default().with_prior_workload(Duration::hours(-12));
        let encoded = serde_yaml::to_string(&human).unwrap();

        assert!(
            encoded.contains("-12"),
            "a negative carry-forward must survive serialisation: {encoded}"
        );

        let decoded: Human = serde_yaml::from_str(&encoded).unwrap();
        assert_eq!(decoded.prior_workload, Duration::hours(-12));
    }

    #[test]
    fn display_renders_without_panicking() {
        let fixture = fixture_hard();
        let problem = &fixture.problem;
        let summary = Summary::new(problem, &construct(problem));

        let rendered = format!("{summary}");
        assert!(rendered.contains("Workload:"));
        assert!(rendered.contains("Longest shift:"));
        assert!(rendered.contains("Shift length histogram:"));
    }

    #[test]
    fn stats_handles_an_empty_input() {
        assert_eq!(stats(Vec::<i64>::new()), (0, 0, 0));
        assert_eq!(stats(vec![5]), (5, 5, 5));
        assert_eq!(stats(vec![1, 2, 3]), (1, 2, 3));
    }
}
