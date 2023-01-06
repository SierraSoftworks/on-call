use chrono::{Duration, NaiveDate, Utc};
use clap::Parser;
use std::path::PathBuf;

mod config;
mod constraints;
mod factors;
#[macro_use]
mod macros;
mod output;
mod solver;
mod summary;
mod timerange;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    #[arg(short, long)]
    config: PathBuf,

    #[arg(short, long)]
    start: Option<NaiveDate>,

    #[arg(short, long)]
    end: Option<NaiveDate>,

    #[arg(short, long, value_enum, default_value = "human")]
    format: output::OutputFormat,

    #[arg(long)]
    debug: bool,
}

fn main() {
    let args = Args::parse();

    let config: config::Config = {
        let file = std::fs::File::open(args.config).unwrap();
        serde_yaml::from_reader(file).unwrap()
    };

    eprintln!("Humans:");
    for (human, constraints) in config.humans.iter() {
        eprintln!("  {}:", human);
        for constraint in constraints.iter() {
            eprintln!("    - {}", constraint);
        }
    }

    let start = args.start.unwrap_or_else(|| Utc::now().naive_utc().date());
    let end = args.end.unwrap_or_else(|| start + Duration::days(28));

    let mut scheduler = solver::Scheduler::new(&config);
    if args.debug {
        scheduler = scheduler.with_debug();
    }

    let schedule = scheduler.schedule(
        start
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Utc)
            .unwrap(),
        end.and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Utc)
            .unwrap(),
    );

    let summary: summary::Summary = (&schedule).into();

    eprintln!();
    eprintln!("{}", summary);

    eprintln!();
    eprintln!("Schedule:");

    args.format.print(&schedule).unwrap();

    if schedule.iter().any(|slot| slot.human.is_none()) {
        println!();
        println!("WARNING: There are unassigned slots in the schedule. This is likely due to constraints that are too restrictive.");
        std::process::exit(1);
    }
}
