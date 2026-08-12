use chrono::{Duration, NaiveDate, NaiveDateTime, Utc};
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use on_call::model::Problem;
use on_call::schedule::Schedule;
use on_call::{config, output, search, summary};

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Path to the YAML file describing the rotation.
    #[arg(short, long)]
    config: PathBuf,

    /// First day to schedule. Defaults to today.
    #[arg(short, long)]
    start: Option<NaiveDate>,

    /// Day to schedule up to, exclusive. Defaults to 28 days after the start.
    #[arg(short, long)]
    end: Option<NaiveDate>,

    /// How to render the resulting schedule.
    #[arg(short, long, value_enum, default_value_t)]
    format: output::OutputFormat,

    /// A previously generated schedule (in JSON) to stay close to.
    ///
    /// Without this, a small change to the config is free to reshuffle every
    /// shift. With it, the optimizer pays for each change it makes and so only
    /// moves shifts when the improvement is worth the disruption.
    #[arg(long, value_name = "FILE")]
    baseline: Option<PathBuf>,

    /// Treat shifts ending before this date as already published and pin them.
    ///
    /// Requires --baseline.
    #[arg(long, value_name = "DATE", requires = "baseline")]
    freeze_before: Option<NaiveDate>,

    /// How many candidate changes the optimizer should evaluate.
    ///
    /// More steps produce better schedules with diminishing returns. This is
    /// the default stopping condition because it is deterministic.
    #[arg(long, default_value_t = 200_000)]
    steps: u64,

    /// Seed for the optimizer. The same seed always produces the same schedule.
    #[arg(long, default_value_t = 0)]
    seed: u64,

    /// Stop after this many seconds, even if the step budget is not exhausted.
    ///
    /// Makes the output depend on machine speed, so it is off by default.
    #[arg(long, value_name = "SECONDS")]
    time_budget: Option<u64>,

    /// Show the score broken down by objective, and optimizer statistics.
    #[arg(long)]
    explain: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Error: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args = Args::parse();

    let config: config::Config = {
        let file = std::fs::File::open(&args.config).map_err(|err| {
            format!("unable to open config {}: {}", args.config.display(), err)
        })?;

        serde_yaml::from_reader(file).map_err(|err| {
            format!("unable to parse config {}: {}", args.config.display(), err)
        })?
    };

    config.validate()?;

    let start = args.start.unwrap_or_else(|| Utc::now().naive_utc().date());
    let end = args.end.unwrap_or(start + Duration::days(28));

    if end <= start {
        return Err(format!("--end ({end}) must be after --start ({start})").into());
    }

    let mut problem = Problem::build(&config, midnight(start), midnight(end))?;

    describe(&config, &problem);

    if let Some(path) = args.baseline.as_ref() {
        let previous = Schedule::load(path)?;
        let (baseline, report) = previous.to_baseline(&problem);

        eprintln!();
        eprintln!("Baseline: {report}");
        if report.matched_slots == 0 {
            eprintln!("  WARNING: the baseline did not match any of this schedule; it will have no effect");
        }

        problem = problem.with_baseline(baseline);

        if let Some(freeze) = args.freeze_before {
            let frozen = problem.freeze_before(midnight(freeze));
            eprintln!("  {frozen} slots before {freeze} pinned to their published owner");
        }
    }

    let options = search::Options {
        steps: args.steps,
        seed: args.seed,
        time_budget: args.time_budget.map(std::time::Duration::from_secs),
        ..search::Options::default()
    };

    let (assignment, statistics) = search::solve(&problem, &options);

    let summary = summary::Summary::new(&problem, &assignment).with_statistics(statistics);
    eprintln!();
    eprintln!("{summary}");

    if args.explain {
        eprintln!();
        eprintln!("{}", summary.explain());
    }

    let schedule = Schedule::from_assignment(&problem, &assignment);

    eprintln!();
    eprintln!("Schedule:");
    args.format.print(&schedule)?;

    if schedule.has_gaps() {
        eprintln!();
        eprintln!(
            "WARNING: parts of this schedule have nobody available to cover them. \
             Relax an availability constraint, or widen the team."
        );
        return Ok(ExitCode::FAILURE);
    }

    if !summary.score().is_feasible() {
        eprintln!();
        eprintln!(
            "WARNING: this schedule breaks one or more of the rules in `rules:`. \
             Run with --explain to see which, and either relax the rule or allow more --steps."
        );
        return Ok(ExitCode::FAILURE);
    }

    Ok(ExitCode::SUCCESS)
}

/// Midnight UTC at the start of a date.
fn midnight(date: NaiveDate) -> NaiveDateTime {
    date.and_hms_opt(0, 0, 0)
        .expect("midnight is always a valid time")
}

/// Echoes the resolved inputs so it is obvious what the optimizer was asked for.
fn describe(config: &config::Config, problem: &Problem) {
    eprintln!("Humans:");

    for name in problem.humans.iter() {
        eprintln!("  {}: {}", name, config.humans[name]);
    }

    eprintln!();
    eprintln!(
        "Schedule: {} slots covering {}h, in shifts of about {}h",
        problem.slot_count(),
        problem.total_demand() / 60,
        problem.target_run_minutes / 60,
    );

    if config.rotation.lock {
        eprintln!(
            "  rotations are locked: {} rotations, handoffs only at their boundaries",
            problem.rotation_count()
        );
    }

    eprintln!("  rules: {}", config.rules);
}
