use std::{collections::HashMap, fmt::Display};

use chrono::Duration;
use serde::{Serialize, Deserialize};

use crate::constraints::Constraint;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(rename = "shiftLength", with="duration_days")]
    pub shift_length: Duration,
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    pub humans: HashMap<String, Human>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Human {
    #[serde(default)]
    pub constraints: Vec<Constraint>,
    #[serde(rename = "priorWorkload", with="duration_hours", default="Duration::zero")]
    pub prior_workload: Duration,
}

#[cfg(test)]
#[allow(dead_code)]
impl Human {
    pub fn with_constraints(self, constraints: Vec<Constraint>) -> Self {
        Self {
            constraints,
            ..self
        }
    }

    pub fn with_prior_workload(self, prior_workload: Duration) -> Self {
        Self {
            prior_workload,
            ..self
        }
    }
}

impl Default for Human {
    fn default() -> Self {
        Self {
            constraints: Vec::new(),
            prior_workload: Duration::zero(),
        }
    }
}

impl Display for Human {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut info = vec![];

        if !self.prior_workload.is_zero() {
            info.push(format!("prior workload: {} hours", self.prior_workload.num_hours()));
        }

        for constraint in self.constraints.iter() {
            info.push(format!("{}", constraint));
        }

        write!(f, "{}", info.join(", "))?;

        Ok(())
    }
}

mod duration_days {
    use chrono::Duration;
    use serde::Deserialize;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        let days = duration.num_days();
        serializer.serialize_u64(days as u64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where D: serde::Deserializer<'de> {
        let days = u64::deserialize(deserializer)?;
        Ok(Duration::days(days as i64))
    }
}

mod duration_hours {
    use chrono::Duration;
    use serde::Deserialize;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        let hours = duration.num_hours();
        serializer.serialize_u64(hours as u64)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where D: serde::Deserializer<'de> {
        let hours = u64::deserialize(deserializer)?;
        Ok(Duration::hours(hours as i64))
    }
}

#[allow(unused)]
mod iso8601_duration {
    use std::{iter::Peekable, str::Chars};

    use chrono::Duration;
    use serde::Deserialize;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        let days = duration.num_seconds() / 86400;
        let hours = (duration.num_seconds() % 86400) / 3600;
        let minutes = (duration.num_seconds() % 3600) / 60;
        let seconds = duration.num_seconds() % 60;
        let millis = duration.num_milliseconds() % 1000;

        let mut s = "P".to_string();

        if days > 0 {
            s.push_str(&format!("{}D", days));
        }

        if hours > 0 || minutes > 0 || seconds > 0 || millis > 0 {
            s.push('T');
        }

        if hours > 0 {
            s.push_str(&format!("{}H", hours));
        }

        if minutes > 0 {
            s.push_str(&format!("{}M", minutes));
        }

        if seconds > 0 {
            s.push_str(&format!("{}", seconds));

            if millis > 0 {
                s.push_str(&format!(".{:03}S", millis));
            } else {
                s.push('S');
            }
        }

        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where D: serde::Deserializer<'de> {
        let s = String::deserialize(deserializer)?;
        
        if !s.starts_with('P') {
            return Err(serde::de::Error::custom("Invalid duration format, durations must be specified in ISO8601 format like 'P1DT1H'"));
        }

        let mut chars = s.chars().peekable();

        while matches!(chars.peek(), Some('P') | Some('T')) {
            chars.next();
        }

        let read_number = |chars: &mut Peekable<Chars>| -> Result<Option<u64>, D::Error> {
            let mut number = String::new();
            while let Some(c) = chars.peek() {
                if c.is_ascii_digit() {
                    number.push(chars.next().unwrap());
                } else {
                    break;
                }
            }

            if number.is_empty() {
                return Ok(None);
            }

            number.parse().map(Some).map_err(serde::de::Error::custom)
        };

        let mut duration = Duration::zero();
        while let Some(n) = read_number(&mut chars)? {
            let adjustment = match chars.next() {
                None => return Err(serde::de::Error::custom("Invalid duration format, durations must be specified in ISO8601 format like 'P1DT1H'")),
                Some('D') => {
                    if chars.peek() == Some(&'T') {
                        chars.next();
                    }

                    Duration::days(n as i64)
                },
                Some('H') => Duration::hours(n as i64),
                Some('M') => Duration::minutes(n as i64),
                Some('S') => {
                    if chars.peek() == Some(&'.') {
                        chars.next();

                        if let Some(millis) = read_number(&mut chars)? {
                            Duration::seconds(n as i64) + Duration::milliseconds(millis as i64)
                        } else {
                            return Err(serde::de::Error::custom("Invalid duration format, durations must be specified in ISO8601 format like 'P1DT1H' (encountered a decimal point without any digits following it)"));
                        }
                    } else {
                        Duration::seconds(n as i64)
                    }
                },
                Some(c) => return Err(serde::de::Error::custom(format!("Invalid duration format, durations must be specified in ISO8601 format like 'P1DT1H' (encountered an unrecognized segment type '{}')", c))),
            };

            duration = duration + adjustment;
        }


        Ok(duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct DurationDemo {
        #[serde(with="iso8601_duration")]
        duration: Duration,
    }

    #[test]
    fn duration_serialize()
    {
        assert_eq!(serde_yaml::from_str::<DurationDemo>("duration: PT10S").unwrap().duration, Duration::seconds(10));
        assert_eq!(serde_yaml::from_str::<DurationDemo>("duration: PT10M").unwrap().duration, Duration::minutes(10));
        assert_eq!(serde_yaml::from_str::<DurationDemo>("duration: PT10H").unwrap().duration, Duration::hours(10));
        assert_eq!(serde_yaml::from_str::<DurationDemo>("duration: P1DT10H").unwrap().duration, Duration::days(1) + Duration::hours(10));
        assert_eq!(serde_yaml::from_str::<DurationDemo>("duration: P1DT10H10M10S").unwrap().duration, Duration::days(1) + Duration::hours(10) + Duration::minutes(10) + Duration::seconds(10));
    }

    #[test]
    fn duration_deserialize()
    {
        assert_eq!(serde_yaml::to_string(&DurationDemo { duration: Duration::seconds(10) }).unwrap().trim(), "duration: PT10S");
        assert_eq!(serde_yaml::to_string(&DurationDemo { duration: Duration::minutes(10) }).unwrap().trim(), "duration: PT10M");
        assert_eq!(serde_yaml::to_string(&DurationDemo { duration: Duration::hours(10) }).unwrap().trim(), "duration: PT10H");
        assert_eq!(serde_yaml::to_string(&DurationDemo { duration: Duration::days(1) + Duration::hours(10) }).unwrap().trim(), "duration: P1DT10H");
        assert_eq!(serde_yaml::to_string(&DurationDemo { duration: Duration::days(1) + Duration::hours(10) + Duration::minutes(10) + Duration::seconds(10) }).unwrap().trim(), "duration: P1DT10H10M10S");
    }

    #[test]
    fn config_deserialize()
    {
        let config = r#"
        shiftLength: 1
        constraints:
            - !DayOfWeek [Mon, Tue, Wed, Thu, Fri]
            - !TimeRange
              start: 08:00:00
              end: 16:00:00
        humans:
            alice@example.com:
                - !None
            bob@example.com:
                - !Unavailable
                  start: 2019-01-01
                  end: 2019-01-04
        "#;

        let config: Config = serde_yaml::from_str(config).expect("the config should be deserializable");
        assert_eq!(config.shift_length, Duration::days(1));
        assert_eq!(config.constraints.len(), 2);
        assert_eq!(config.humans.len(), 2);
    }
}