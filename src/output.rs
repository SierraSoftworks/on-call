use std::fmt::Display;

use crate::solver::ScheduleSlot;


#[derive(clap::ValueEnum, Clone, Debug)]
pub enum OutputFormat {
    None,
    Human,
    Json,
    Csv,
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Human
    }
}

impl Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::None => write!(f, "none"),
            OutputFormat::Human => write!(f, "human"),
            OutputFormat::Json => write!(f, "json"),
            OutputFormat::Csv => write!(f, "csv"),
        }
    }
}

impl OutputFormat {
    pub fn print(self, schedule: &[ScheduleSlot]) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            OutputFormat::None => Ok(()),
            OutputFormat::Human => {
                Self::print_human(schedule)
            },
            OutputFormat::Json => {
                Self::print_json(schedule)
            },
            OutputFormat::Csv => {
                Self::print_csv(schedule)
            },
        }
    }

    fn print_human(schedule: &[ScheduleSlot]) -> Result<(), Box<dyn std::error::Error>> {
        for slot in schedule {
            println!("  {}: {}", slot.time, slot.human.as_deref().unwrap_or("UNASSIGNED"));
        }

        Ok(())
    }

    fn print_json(schedule: &[ScheduleSlot]) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(schedule)?;
        println!("{}", json);

        Ok(())
    }

    fn print_csv(schedule: &[ScheduleSlot]) -> Result<(), Box<dyn std::error::Error>> {
        println!("start,end,human");
        for slot in schedule {
            println!("{},{},{}", slot.time.start, slot.time.end, slot.human.as_deref().unwrap_or("UNASSIGNED"));
        }

        Ok(())
    }
}