use std::{collections::HashMap, fmt::Display};

use chrono::{Duration, Weekday};
use serde::{Serialize, Deserialize};

use crate::constraints::Constraint;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    /// The nominal length of a single on-call shift, expressed in days.
    ///
    /// This is a *target* rather than a hard rule: the optimizer penalises runs
    /// which deviate from it, but will accept a shorter or longer shift when
    /// doing so produces a materially better schedule overall. Set
    /// `rotation.lock` if you need it enforced structurally.
    #[serde(rename = "shiftLength", with="duration_days")]
    pub shift_length: Duration,

    /// Constraints which determine the time slots requiring on-call coverage.
    #[serde(default)]
    pub constraints: Vec<Constraint>,

    /// Hard rules that a schedule must satisfy.
    #[serde(default)]
    pub rules: Rules,

    /// Relative importance of each soft objective.
    #[serde(default)]
    pub weights: Weights,

    /// How shift boundaries are permitted to fall.
    #[serde(default)]
    pub rotation: Rotation,

    /// How each engineer's fair share of the workload is derived.
    #[serde(default)]
    pub fairness: FairnessMode,

    pub humans: HashMap<String, Human>,
}

impl Config {
    /// The rest period the optimizer aims for between shifts, falling back to
    /// the configured shift length when not explicitly set.
    pub fn desired_rest(&self) -> Duration {
        self.rules.desired_rest.unwrap_or(self.shift_length)
    }

    /// Validates the parts of the configuration that serde cannot express,
    /// returning a human-readable description of the first problem found.
    pub fn validate(&self) -> Result<(), String> {
        if self.shift_length <= Duration::zero() {
            return Err("shiftLength must be at least one day".to_string());
        }

        if self.humans.is_empty() {
            return Err("at least one human must be defined".to_string());
        }

        for (name, human) in self.humans.iter() {
            for preference in human.preferences.iter() {
                preference
                    .resolve()
                    .map_err(|err| format!("{}: {}", name, err))?;
            }

            if let Some(capacity) = human.capacity {
                if !capacity.is_finite() || capacity < 0.0 {
                    return Err(format!(
                        "{}: capacity must be a non-negative number, got {}",
                        name, capacity
                    ));
                }
            }
        }

        for (name, weight) in [
            ("fairness", self.weights.fairness),
            ("runLength", self.weights.run_length),
            ("rest", self.weights.rest),
            ("preference", self.weights.preference),
            ("stability", self.weights.stability),
        ] {
            if !weight.is_finite() || weight < 0.0 {
                return Err(format!(
                    "weights.{} must be a non-negative number, got {}",
                    name, weight
                ));
            }
        }

        if let Some(RotationBoundary::EverySlots(0)) = self.rotation.boundary {
            return Err("rotation.boundary EverySlots must be at least 1".to_string());
        }

        if let (Some(min_rest), Some(max_consecutive)) =
            (self.rules.min_rest, self.rules.max_consecutive)
        {
            if min_rest < Duration::zero() || max_consecutive <= Duration::zero() {
                return Err(
                    "rules.minRestHours must be non-negative and rules.maxConsecutiveHours must be positive"
                        .to_string(),
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
#[allow(dead_code)]
impl Config {
    /// Builds a minimal config for tests; fields are then overridden with the
    /// `with_*` helpers so that adding a new config field does not require
    /// touching every test.
    pub fn for_test(shift_length: Duration, humans: HashMap<String, Human>) -> Self {
        Self {
            shift_length,
            constraints: Vec::new(),
            rules: Rules::default(),
            weights: Weights::default(),
            rotation: Rotation::default(),
            fairness: FairnessMode::default(),
            humans,
        }
    }

    pub fn with_constraints(self, constraints: Vec<Constraint>) -> Self {
        Self { constraints, ..self }
    }

    pub fn with_rules(self, rules: Rules) -> Self {
        Self { rules, ..self }
    }

    pub fn with_weights(self, weights: Weights) -> Self {
        Self { weights, ..self }
    }

    pub fn with_rotation(self, rotation: Rotation) -> Self {
        Self { rotation, ..self }
    }

    pub fn with_fairness(self, fairness: FairnessMode) -> Self {
        Self { fairness, ..self }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Human {
    /// Hard availability constraints. The optimizer will never assign this
    /// person to a slot they cannot cover.
    #[serde(default)]
    pub constraints: Vec<Constraint>,

    /// Soft preferences. Violating these is permitted, but costs.
    #[serde(default)]
    pub preferences: Vec<Preference>,

    /// How far ahead of their fair share this person already is, carried over
    /// from a previous scheduling run. Reduces their target here, so they are
    /// scheduled less until the rest of the team catches up.
    ///
    /// May be negative for somebody who is behind and owed more on-call time.
    /// The `carry forward` column of the summary reports what to put here for
    /// the next run.
    #[serde(rename = "priorWorkload", with="duration_hours", default="Duration::zero")]
    pub prior_workload: Duration,

    /// Overrides this person's share of the total workload, as a multiplier
    /// relative to a full-time team member. A half-time engineer would use
    /// `0.5`. When omitted, the share is derived from how much of the schedule
    /// their availability constraints actually permit them to cover.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity: Option<f64>,
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

    pub fn with_preferences(self, preferences: Vec<Preference>) -> Self {
        Self {
            preferences,
            ..self
        }
    }

    pub fn with_prior_workload(self, prior_workload: Duration) -> Self {
        Self {
            prior_workload,
            ..self
        }
    }

    pub fn with_capacity(self, capacity: f64) -> Self {
        Self {
            capacity: Some(capacity),
            ..self
        }
    }
}

impl Default for Human {
    fn default() -> Self {
        Self {
            constraints: Vec::new(),
            preferences: Vec::new(),
            prior_workload: Duration::zero(),
            capacity: None,
        }
    }
}

impl Display for Human {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut info = vec![];

        if !self.prior_workload.is_zero() {
            info.push(format!("prior workload: {} hours", self.prior_workload.num_hours()));
        }

        if let Some(capacity) = self.capacity {
            info.push(format!("capacity: {}x", capacity));
        }

        for constraint in self.constraints.iter() {
            info.push(format!("{}", constraint));
        }

        for preference in self.preferences.iter() {
            info.push(format!("{}", preference));
        }

        if info.is_empty() {
            info.push("always available".to_string());
        }

        write!(f, "{}", info.join(", "))?;

        Ok(())
    }
}

/// A soft scheduling preference.
///
/// Exactly one of `avoid` or `prefer` must be set. We use two optional fields
/// rather than a tagged enum because the constraint values themselves rely on
/// YAML tags (`!DayOfWeek`), and nesting a tag inside a `#[serde(flatten)]`ed
/// enum does not survive serde_yaml's buffering.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct Preference {
    /// Times this person would rather not be on-call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avoid: Option<Constraint>,

    /// Times this person would rather be on-call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer: Option<Constraint>,

    /// How strongly this preference is held, relative to other preferences.
    #[serde(default = "default_preference_weight")]
    pub weight: f64,
}

fn default_preference_weight() -> f64 {
    1.0
}

/// Which way a preference pushes the optimizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreferenceKind {
    Avoid,
    Prefer,
}

impl Preference {
    /// Resolves this preference into a constraint and a direction, rejecting
    /// entries which set both or neither of `avoid` and `prefer`.
    pub fn resolve(&self) -> Result<(&Constraint, PreferenceKind), String> {
        match (&self.avoid, &self.prefer) {
            (Some(constraint), None) => Ok((constraint, PreferenceKind::Avoid)),
            (None, Some(constraint)) => Ok((constraint, PreferenceKind::Prefer)),
            (Some(_), Some(_)) => Err(
                "a preference may set either 'avoid' or 'prefer', but not both".to_string(),
            ),
            (None, None) => Err(
                "a preference must set either 'avoid' or 'prefer'".to_string(),
            ),
        }
    }
}

impl Display for Preference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.resolve() {
            Ok((constraint, PreferenceKind::Avoid)) => {
                write!(f, "prefers not to be {} (weight {})", constraint, self.weight)
            }
            Ok((constraint, PreferenceKind::Prefer)) => {
                write!(f, "prefers to be {} (weight {})", constraint, self.weight)
            }
            Err(err) => write!(f, "invalid preference ({})", err),
        }
    }
}

/// Hard rules which the optimizer treats as constraints rather than costs.
///
/// Every field is optional. Note that if a rule cannot be satisfied for a given
/// team and horizon, the optimizer will minimise the extent of the violation
/// rather than failing outright, and will report the residual violation.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    /// The minimum amount of time that must elapse between the end of one
    /// shift and the start of that person's next shift.
    #[serde(
        rename = "minRestHours",
        with = "optional_duration_hours",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub min_rest: Option<Duration>,

    /// The maximum wall-clock duration a single unbroken shift may span.
    #[serde(
        rename = "maxConsecutiveHours",
        with = "optional_duration_hours",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub max_consecutive: Option<Duration>,

    /// The amount of rest between shifts the optimizer aims for. Falling short
    /// of this is penalised softly; there is no reward for exceeding it.
    ///
    /// Defaults to the configured shift length.
    #[serde(
        rename = "desiredRestHours",
        with = "optional_duration_hours",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub desired_rest: Option<Duration>,
}

impl Display for Rules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = vec![];

        if let Some(min_rest) = self.min_rest {
            parts.push(format!("at least {}h rest between shifts", min_rest.num_hours()));
        }

        if let Some(max_consecutive) = self.max_consecutive {
            parts.push(format!("shifts no longer than {}h", max_consecutive.num_hours()));
        }

        if parts.is_empty() {
            write!(f, "none")
        } else {
            write!(f, "{}", parts.join(", "))
        }
    }
}

/// The relative importance of each soft objective.
///
/// Every objective reports its penalty in the same units — "minutes of
/// wrongness" — so these weights are directly comparable to one another. A
/// weight of zero disables the objective entirely.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct Weights {
    /// How evenly the workload is spread across the team.
    #[serde(default = "default_fairness_weight")]
    pub fairness: f64,

    /// How closely shift lengths track the configured `shiftLength`.
    #[serde(rename = "runLength", default = "default_run_length_weight")]
    pub run_length: f64,

    /// How much recovery time people get between shifts.
    #[serde(default = "default_rest_weight")]
    pub rest: f64,

    /// How strongly individual preferences are respected.
    #[serde(default = "default_preference_objective_weight")]
    pub preference: f64,

    /// How closely the schedule tracks a baseline provided via `--baseline`.
    #[serde(default = "default_stability_weight")]
    pub stability: f64,
}

fn default_fairness_weight() -> f64 {
    10.0
}

fn default_run_length_weight() -> f64 {
    10.0
}

fn default_rest_weight() -> f64 {
    3.0
}

fn default_preference_objective_weight() -> f64 {
    1.0
}

fn default_stability_weight() -> f64 {
    5.0
}

impl Default for Weights {
    fn default() -> Self {
        Self {
            fairness: default_fairness_weight(),
            run_length: default_run_length_weight(),
            rest: default_rest_weight(),
            preference: default_preference_objective_weight(),
            stability: default_stability_weight(),
        }
    }
}

/// Controls where shift boundaries are allowed to fall.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct Rotation {
    /// When set, handoffs may only occur at a rotation boundary. This is
    /// enforced structurally — the optimizer assigns whole rotations rather
    /// than individual slots — which makes the schedule highly predictable at
    /// the cost of some flexibility around partial availability.
    #[serde(default)]
    pub lock: bool,

    /// Where rotation boundaries fall. Defaults to every `shiftLength` slots,
    /// which reproduces a simple fixed rotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boundary: Option<RotationBoundary>,
}

/// Where a new rotation is permitted to begin.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum RotationBoundary {
    /// A new rotation begins on slots which start on one of these weekdays.
    DayOfWeek(Vec<Weekday>),
    /// A new rotation begins every N slots.
    EverySlots(usize),
}

/// How each person's fair share of the total workload is derived.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum FairnessMode {
    /// Each person's target is proportional to how much of the schedule their
    /// availability actually lets them cover. Somebody available only on
    /// Mondays, Wednesdays and Fridays is expected to carry roughly 60% of the
    /// load of a colleague with no restrictions.
    #[default]
    Capacity,
    /// Everybody is expected to carry the same number of hours regardless of
    /// their availability. Availability then only constrains *when* they are
    /// on-call, not how much.
    Equal,
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
        serializer.serialize_i64(duration.num_hours())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where D: serde::Deserializer<'de> {
        let hours = i64::deserialize(deserializer)?;
        Ok(Duration::hours(hours))
    }
}

mod optional_duration_hours {
    use chrono::Duration;
    use serde::Deserialize;

    pub fn serialize<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where S: serde::Serializer {
        match duration {
            Some(duration) => serializer.serialize_u64(duration.num_hours() as u64),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where D: serde::Deserializer<'de> {
        let hours = Option::<u64>::deserialize(deserializer)?;
        Ok(hours.map(|hours| Duration::hours(hours as i64)))
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

            duration += adjustment;
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
            - !TimeOfDay
              start: 08:00:00
              end: 16:00:00
        humans:
            alice@example.com:
                constraints:
                    - !None
                priorWorkload: 72
            bob@example.com:
                constraints:
                    - !Unavailable
                      start: 2019-01-01
                      end: 2019-01-04
        "#;

        let config: Config = serde_yaml::from_str(config).expect("the config should be deserializable");
        assert_eq!(config.shift_length, Duration::days(1));
        assert_eq!(config.constraints.len(), 2);
        assert_eq!(config.humans.len(), 2);
        config.validate().expect("the config should be valid");
    }

    /// Parses the full schema exactly as documented in the README, so the docs
    /// cannot drift away from what the tool actually accepts.
    #[test]
    fn the_documented_schema_parses() {
        let config = r#"
        shiftLength: 3
        constraints:
          - !DayOfWeek [Mon, Tue, Wed, Thu, Fri]
          - !TimeOfDay
            start: 08:00:00
            end: 16:00:00
        rules:
          minRestHours: 48
          maxConsecutiveHours: 72
          desiredRestHours: 120
        weights:
          fairness: 10
          runLength: 10
          rest: 3
          preference: 1
          stability: 5
        fairness: capacity
        rotation:
          lock: true
          boundary: !DayOfWeek [Mon]
        humans:
          alice@example.com:
            constraints:
              - !DayOfWeek [Mon, Wed, Fri]
          bob@example.com:
            capacity: 0.5
            constraints:
              - !Unavailable
                start: 2023-01-01
                end: 2023-01-07
          claire@example.com:
            preferences:
              - avoid: !DayOfWeek [Mon]
                weight: 3
              - prefer: !TimeOfDay
                  start: 08:00:00
                  end: 12:00:00
          erica@example.com:
            priorWorkload: 24
        "#;

        let config: Config = serde_yaml::from_str(config).expect("the documented schema must parse");
        config.validate().expect("the documented schema must validate");

        assert_eq!(config.shift_length, Duration::days(3));
        assert_eq!(config.rules.min_rest, Some(Duration::hours(48)));
        assert_eq!(config.rules.max_consecutive, Some(Duration::hours(72)));
        assert_eq!(config.desired_rest(), Duration::hours(120));
        assert_eq!(config.weights.fairness, 10.0);
        assert_eq!(config.fairness, FairnessMode::Capacity);
        assert!(config.rotation.lock);
        assert!(matches!(
            config.rotation.boundary,
            Some(RotationBoundary::DayOfWeek(_))
        ));
        assert_eq!(config.humans["bob@example.com"].capacity, Some(0.5));
        assert_eq!(config.humans["claire@example.com"].preferences.len(), 2);
        assert_eq!(
            config.humans["erica@example.com"].prior_workload,
            Duration::hours(24)
        );
    }

    #[test]
    fn the_every_slots_rotation_boundary_parses() {
        let config: Config = serde_yaml::from_str(
            r#"
            shiftLength: 5
            rotation:
              lock: true
              boundary: !EverySlots 5
            humans:
              alice@example.com: {}
            "#,
        )
        .expect("!EverySlots must parse as documented");

        assert!(matches!(
            config.rotation.boundary,
            Some(RotationBoundary::EverySlots(5))
        ));
    }

    #[test]
    fn a_minimal_config_uses_defaults_throughout() {
        let config: Config = serde_yaml::from_str(
            r#"
            shiftLength: 1
            humans:
              alice@example.com: {}
            "#,
        )
        .expect("shiftLength and humans should be the only required keys");

        config.validate().unwrap();
        assert!(config.constraints.is_empty());
        assert_eq!(config.rules.min_rest, None);
        assert_eq!(config.weights.fairness, 10.0);
        assert!(!config.rotation.lock);
        assert_eq!(config.fairness, FairnessMode::Capacity);
        assert_eq!(config.desired_rest(), Duration::days(1));
    }

    #[test]
    fn a_negative_prior_workload_is_accepted() {
        let config: Config = serde_yaml::from_str(
            r#"
            shiftLength: 1
            humans:
              alice@example.com:
                priorWorkload: -16
            "#,
        )
        .expect("somebody who is owed on-call time should be expressible");

        assert_eq!(
            config.humans["alice@example.com"].prior_workload,
            Duration::hours(-16)
        );
    }

    #[test]
    fn fairness_mode_accepts_equal() {
        let config: Config = serde_yaml::from_str(
            r#"
            shiftLength: 1
            fairness: equal
            humans:
              alice@example.com: {}
            "#,
        )
        .unwrap();

        assert_eq!(config.fairness, FairnessMode::Equal);
    }

    #[test]
    fn a_preference_setting_both_avoid_and_prefer_is_rejected() {
        let config: Config = serde_yaml::from_str(
            r#"
            shiftLength: 1
            humans:
              alice@example.com:
                preferences:
                  - avoid: !None
                    prefer: !None
            "#,
        )
        .unwrap();

        let error = config.validate().unwrap_err();
        assert!(error.contains("alice@example.com"), "{error}");
        assert!(error.contains("not both"), "{error}");
    }

    #[test]
    fn a_preference_setting_neither_avoid_nor_prefer_is_rejected() {
        let config: Config = serde_yaml::from_str(
            r#"
            shiftLength: 1
            humans:
              alice@example.com:
                preferences:
                  - weight: 3
            "#,
        )
        .unwrap();

        assert!(config.validate().is_err());
    }

    #[test]
    fn invalid_weights_and_capacities_are_rejected() {
        let mut config: Config = serde_yaml::from_str(
            r#"
            shiftLength: 1
            humans:
              alice@example.com: {}
            "#,
        )
        .unwrap();

        config.weights.fairness = -1.0;
        assert!(config.validate().unwrap_err().contains("weights.fairness"));

        config.weights.fairness = 1.0;
        config.humans.get_mut("alice@example.com").unwrap().capacity = Some(-0.5);
        assert!(config.validate().unwrap_err().contains("capacity"));
    }

    #[test]
    fn an_empty_team_is_rejected() {
        let config: Config = serde_yaml::from_str(
            r#"
            shiftLength: 1
            humans: {}
            "#,
        )
        .unwrap();

        assert!(config.validate().unwrap_err().contains("at least one human"));
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_silently_ignored() {
        // Typos in rule and weight names would otherwise be invisible.
        let result: Result<Config, _> = serde_yaml::from_str(
            r#"
            shiftLength: 1
            rules:
              minRestHrs: 48
            humans:
              alice@example.com: {}
            "#,
        );

        assert!(result.is_err(), "a misspelled rule should not be ignored");
    }
}