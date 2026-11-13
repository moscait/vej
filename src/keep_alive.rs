//! Ollama-style keep-alive durations.
//!
//! A keep-alive controls how long a model stays resident after its last request
//! completes. Accepted forms (as JSON, TOML, or CLI strings):
//! - Go-style durations: `"30s"`, `"5m"`, `"1h30m"`, `"250ms"`, `"1.5h"`
//! - Bare numbers, interpreted as seconds: `300`, `"300"`, `0.5`
//! - `0` / `"0"`: unload as soon as the last in-flight request finishes
//! - Any negative value (`-1`, `"-1"`, `"-5m"`): never unload

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepAlive {
    /// Keep the model resident this long after its last use. `Duration::ZERO` unloads immediately.
    For(Duration),
    /// Never unload automatically.
    Forever,
}

impl Default for KeepAlive {
    fn default() -> Self {
        Self::For(Duration::from_secs(300))
    }
}

const NANOS_PER_SEC: f64 = 1e9;

/// Go-style duration units, longest suffix first so `ms` wins over `m`.
const UNITS: [(&str, f64); 6] = [
    ("ns", 1.0),
    ("us", 1e3),
    ("ms", 1e6),
    ("s", NANOS_PER_SEC),
    ("m", 60.0 * NANOS_PER_SEC),
    ("h", 3600.0 * NANOS_PER_SEC),
];

impl KeepAlive {
    pub fn parse(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        let (is_negative, body) = match trimmed.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, trimmed),
        };
        if body.is_empty() || body.starts_with(['-', '+']) {
            bail!("invalid keep_alive '{input}': expected e.g. \"5m\", \"30s\", 0, or -1");
        }

        let is_bare_number = body.bytes().all(|b| b.is_ascii_digit() || b == b'.');
        let duration = if is_bare_number {
            let secs: f64 = body
                .parse()
                .map_err(|_| anyhow!("invalid keep_alive '{input}': not a number"))?;
            nanos_to_duration(secs * NANOS_PER_SEC, input)?
        } else {
            parse_go_duration(body, input)?
        };

        Ok(if is_negative && !duration.is_zero() {
            Self::Forever
        } else {
            Self::For(duration)
        })
    }

    pub fn from_seconds(secs: f64) -> Result<Self> {
        if secs.is_nan() {
            bail!("invalid keep_alive: NaN");
        }
        if secs < 0.0 {
            return Ok(Self::Forever);
        }
        nanos_to_duration(secs * NANOS_PER_SEC, &secs.to_string()).map(Self::For)
    }

    pub fn unloads_immediately(&self) -> bool {
        matches!(self, Self::For(d) if d.is_zero())
    }

    /// When an idle model should be released, or `None` if it never should.
    pub fn deadline(&self, idle_since: Instant) -> Option<Instant> {
        match self {
            Self::For(d) => idle_since.checked_add(*d),
            Self::Forever => None,
        }
    }
}

/// Parses a sequence of `<number><unit>` components such as `1h30m` or `1.5s`.
fn parse_go_duration(body: &str, original: &str) -> Result<Duration> {
    let invalid = || anyhow!("invalid keep_alive '{original}': expected e.g. \"5m\", \"1h30m\"");
    let mut rest = body;
    let mut total_nanos = 0.0_f64;

    while !rest.is_empty() {
        let number_len = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let value: f64 = rest[..number_len].parse().map_err(|_| invalid())?;
        rest = &rest[number_len..];

        let (unit, unit_nanos) = UNITS
            .iter()
            .filter(|(suffix, _)| rest.starts_with(suffix))
            .max_by_key(|(suffix, _)| suffix.len())
            .ok_or_else(invalid)?;
        rest = &rest[unit.len()..];
        total_nanos += value * unit_nanos;
    }

    nanos_to_duration(total_nanos, original)
}

fn nanos_to_duration(nanos: f64, original: &str) -> Result<Duration> {
    if !nanos.is_finite() || nanos < 0.0 || nanos > u64::MAX as f64 {
        bail!("invalid keep_alive '{original}': out of range");
    }
    Ok(Duration::from_nanos(nanos.round() as u64))
}

impl FromStr for KeepAlive {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

impl fmt::Display for KeepAlive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let duration = match self {
            Self::Forever => return f.write_str("-1"),
            Self::For(d) if d.is_zero() => return f.write_str("0"),
            Self::For(d) => *d,
        };
        if duration.subsec_nanos() != 0 {
            let nanos = duration.as_nanos();
            return if nanos % 1_000_000 == 0 {
                write!(f, "{}ms", nanos / 1_000_000)
            } else {
                write!(f, "{nanos}ns")
            };
        }
        match duration.as_secs() {
            s if s % 3600 == 0 => write!(f, "{}h", s / 3600),
            s if s % 60 == 0 => write!(f, "{}m", s / 60),
            s => write!(f, "{s}s"),
        }
    }
}

impl Serialize for KeepAlive {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for KeepAlive {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(KeepAliveVisitor)
    }
}

struct KeepAliveVisitor;

impl serde::de::Visitor<'_> for KeepAliveVisitor {
    type Value = KeepAlive;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a duration string like \"5m\" or a number of seconds (negative = forever)")
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<KeepAlive, E> {
        KeepAlive::from_seconds(v as f64).map_err(E::custom)
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<KeepAlive, E> {
        KeepAlive::from_seconds(v as f64).map_err(E::custom)
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<KeepAlive, E> {
        KeepAlive::from_seconds(v).map_err(E::custom)
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<KeepAlive, E> {
        KeepAlive::parse(v).map_err(E::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> KeepAlive {
        KeepAlive::For(Duration::from_secs(s))
    }

    #[test]
    fn defaults_to_five_minutes() {
        assert_eq!(KeepAlive::default(), secs(300));
    }

    #[test]
    fn parses_go_style_durations() {
        assert_eq!(KeepAlive::parse("5m").unwrap(), secs(300));
        assert_eq!(KeepAlive::parse("30s").unwrap(), secs(30));
        assert_eq!(KeepAlive::parse("2h").unwrap(), secs(7200));
        assert_eq!(KeepAlive::parse("1h30m").unwrap(), secs(5400));
        assert_eq!(KeepAlive::parse(" 10m ").unwrap(), secs(600));
        assert_eq!(
            KeepAlive::parse("250ms").unwrap(),
            KeepAlive::For(Duration::from_millis(250))
        );
        assert_eq!(KeepAlive::parse("1.5h").unwrap(), secs(5400));
    }

    #[test]
    fn parses_bare_numbers_as_seconds() {
        assert_eq!(KeepAlive::parse("300").unwrap(), secs(300));
        assert_eq!(
            KeepAlive::parse("0.5").unwrap(),
            KeepAlive::For(Duration::from_millis(500))
        );
    }

    #[test]
    fn zero_means_unload_immediately() {
        let ka = KeepAlive::parse("0").unwrap();
        assert_eq!(ka, KeepAlive::For(Duration::ZERO));
        assert!(ka.unloads_immediately());
        assert!(!secs(1).unloads_immediately());
        assert!(!KeepAlive::Forever.unloads_immediately());
    }

    #[test]
    fn negative_values_mean_forever() {
        assert_eq!(KeepAlive::parse("-1").unwrap(), KeepAlive::Forever);
        assert_eq!(KeepAlive::parse("-5m").unwrap(), KeepAlive::Forever);
        assert_eq!(KeepAlive::from_seconds(-1.0).unwrap(), KeepAlive::Forever);
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "", "  ", "abc", "5x", "m", "5m3", "--1", "1h-30m", "NaN", "inf", "1e400",
        ] {
            assert!(
                KeepAlive::parse(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
        assert!(KeepAlive::from_seconds(f64::NAN).is_err());
    }

    #[test]
    fn deserializes_from_json_numbers_and_strings() {
        let parse = |json: &str| serde_json::from_str::<KeepAlive>(json);
        assert_eq!(parse("60").unwrap(), secs(60));
        assert_eq!(parse("-1").unwrap(), KeepAlive::Forever);
        assert_eq!(parse("0").unwrap(), KeepAlive::For(Duration::ZERO));
        assert_eq!(
            parse("0.5").unwrap(),
            KeepAlive::For(Duration::from_millis(500))
        );
        assert_eq!(parse("\"10m\"").unwrap(), secs(600));
        assert!(parse("true").is_err());
        assert!(parse("\"soon\"").is_err());
    }

    #[test]
    fn deserializes_from_toml_string() {
        #[derive(Deserialize)]
        struct Wrapper {
            keep_alive: KeepAlive,
        }
        let w: Wrapper = toml::from_str("keep_alive = \"15m\"").unwrap();
        assert_eq!(w.keep_alive, secs(900));
        let w: Wrapper = toml::from_str("keep_alive = -1").unwrap();
        assert_eq!(w.keep_alive, KeepAlive::Forever);
    }

    #[test]
    fn displays_in_most_compact_unit_and_round_trips() {
        let cases = [
            (secs(300), "5m"),
            (secs(7200), "2h"),
            (secs(90), "90s"),
            (KeepAlive::For(Duration::from_millis(1500)), "1500ms"),
            (KeepAlive::For(Duration::ZERO), "0"),
            (KeepAlive::Forever, "-1"),
        ];
        for (ka, text) in cases {
            assert_eq!(ka.to_string(), text);
            assert_eq!(KeepAlive::parse(text).unwrap(), ka);
            assert_eq!(serde_json::to_string(&ka).unwrap(), format!("\"{text}\""));
        }
    }

    #[test]
    fn deadline_is_offset_from_idle_start() {
        let now = Instant::now();
        assert_eq!(secs(10).deadline(now), Some(now + Duration::from_secs(10)));
        assert_eq!(KeepAlive::For(Duration::ZERO).deadline(now), Some(now));
        assert_eq!(KeepAlive::Forever.deadline(now), None);
    }
}
