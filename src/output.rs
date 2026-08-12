use std::fmt::Display;

use crate::schedule::Schedule;

#[derive(clap::ValueEnum, Clone, Copy, Debug, Default)]
pub enum OutputFormat {
    /// Print only the statistics, not the schedule itself.
    None,
    /// A readable list of shifts.
    #[default]
    Human,
    /// JSON, which is also the format accepted by `--baseline`.
    Json,
    /// Comma-separated values, with RFC 4180 quoting.
    Csv,
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
    pub fn print(self, schedule: &Schedule) -> Result<(), Box<dyn std::error::Error>> {
        let mut out = std::io::stdout().lock();
        self.write(schedule, &mut out)
    }

    pub fn write(
        self,
        schedule: &Schedule,
        out: &mut impl std::io::Write,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            OutputFormat::None => Ok(()),
            OutputFormat::Human => Self::write_human(schedule, out),
            OutputFormat::Json => Self::write_json(schedule, out),
            OutputFormat::Csv => Self::write_csv(schedule, out),
        }
    }

    fn write_human(
        schedule: &Schedule,
        out: &mut impl std::io::Write,
    ) -> Result<(), Box<dyn std::error::Error>> {
        for shift in schedule.shifts.iter() {
            writeln!(
                out,
                "  {}: {}",
                shift.time,
                shift.human.as_deref().unwrap_or("UNASSIGNED")
            )?;
        }

        Ok(())
    }

    fn write_json(
        schedule: &Schedule,
        out: &mut impl std::io::Write,
    ) -> Result<(), Box<dyn std::error::Error>> {
        serde_json::to_writer_pretty(&mut *out, schedule)?;
        writeln!(out)?;

        Ok(())
    }

    fn write_csv(
        schedule: &Schedule,
        out: &mut impl std::io::Write,
    ) -> Result<(), Box<dyn std::error::Error>> {
        writeln!(out, "start,end,human")?;

        for shift in schedule.shifts.iter() {
            writeln!(
                out,
                "{},{},{}",
                shift.time.start,
                shift.time.end,
                escape_csv(shift.human.as_deref().unwrap_or("UNASSIGNED"))
            )?;
        }

        Ok(())
    }
}

/// Quotes a CSV field when it contains anything that would otherwise break the
/// record structure. Names come from user-supplied config, so this cannot be
/// assumed safe.
fn escape_csv(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::Shift;
    use crate::timerange::TimeRange;

    fn schedule() -> Schedule {
        Schedule {
            shifts: vec![
                Shift {
                    time: TimeRange::new(
                        date_time!(2023, 1, 2, 9, 0, 0),
                        date_time!(2023, 1, 3, 17, 0, 0),
                    ),
                    human: Some("alice@example.com".to_string()),
                },
                Shift {
                    time: TimeRange::new(
                        date_time!(2023, 1, 4, 9, 0, 0),
                        date_time!(2023, 1, 4, 17, 0, 0),
                    ),
                    human: None,
                },
            ],
        }
    }

    fn render(format: OutputFormat) -> String {
        let mut buffer = Vec::new();
        format.write(&schedule(), &mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    #[test]
    fn none_writes_nothing() {
        assert_eq!(render(OutputFormat::None), "");
    }

    #[test]
    fn human_lists_each_shift() {
        let output = render(OutputFormat::Human);

        assert!(output.contains("alice@example.com"));
        assert!(output.contains("UNASSIGNED"));
        assert_eq!(output.lines().count(), 2);
    }

    #[test]
    fn json_round_trips() {
        let output = render(OutputFormat::Json);
        let parsed: Schedule = serde_json::from_str(&output).unwrap();

        assert_eq!(parsed, schedule());
    }

    #[test]
    fn json_uses_the_flattened_shape_the_baseline_reader_expects() {
        let output = render(OutputFormat::Json);
        let value: serde_json::Value = serde_json::from_str(&output).unwrap();

        let first = &value[0];
        assert!(first.get("start").is_some());
        assert!(first.get("end").is_some());
        assert!(first.get("human").is_some());
    }

    #[test]
    fn csv_has_a_header_and_one_row_per_shift() {
        let output = render(OutputFormat::Csv);
        let lines: Vec<&str> = output.lines().collect();

        assert_eq!(lines[0], "start,end,human");
        assert_eq!(lines.len(), 3);
        assert!(lines[2].ends_with("UNASSIGNED"));
    }

    #[test]
    fn csv_quotes_fields_that_would_break_the_format() {
        assert_eq!(escape_csv("alice@example.com"), "alice@example.com");
        assert_eq!(escape_csv("Smith, Alice"), "\"Smith, Alice\"");
        assert_eq!(escape_csv("she said \"hi\""), "\"she said \"\"hi\"\"\"");
        assert_eq!(escape_csv("two\nlines"), "\"two\nlines\"");
    }

    #[test]
    fn the_default_format_is_human_readable() {
        assert!(matches!(OutputFormat::default(), OutputFormat::Human));
        assert_eq!(OutputFormat::default().to_string(), "human");
    }
}
