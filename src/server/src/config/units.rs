//! Human-readable durations and byte sizes for the configuration file.
//!
//! `default_expiry = "7d"` is far easier to review than `604800`, and a reviewer
//! who cannot check a value at a glance is a reviewer who misses a wrong one.
//! Both types deserialise from a string and reject anything they do not fully
//! understand — a silently ignored suffix would be worse than a parse error.

use std::time::Duration;

use serde::{Deserialize, Deserializer};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum UnitError {
    #[error("empty value")]
    Empty,
    #[error("{0}: expected a number followed by a unit")]
    Malformed(String),
    #[error("{value}: unknown unit {unit:?}")]
    UnknownUnit { value: String, unit: String },
    #[error("{0}: value out of range")]
    Overflow(String),
}

/// Splits `"10MiB"` into `(10, "MiB")`, tolerating whitespace between the two.
fn split(text: &str) -> Result<(u64, &str), UnitError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(UnitError::Empty);
    }

    let digits = trimmed.find(|c: char| !c.is_ascii_digit()).unwrap_or(trimmed.len());

    if digits == 0 {
        return Err(UnitError::Malformed(trimmed.to_owned()));
    }

    let number = trimmed[..digits]
        .parse::<u64>()
        .map_err(|_| UnitError::Overflow(trimmed.to_owned()))?;

    Ok((number, trimmed[digits..].trim()))
}

/// A duration written as `30s`, `15m`, `24h` or `90d`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HumanDuration(pub Duration);

impl HumanDuration {
    pub fn parse(text: &str) -> Result<Self, UnitError> {
        let (number, unit) = split(text)?;

        let seconds_per = match unit {
            "s" | "sec" | "secs" => 1_u64,
            "m" | "min" | "mins" => 60,
            "h" | "hour" | "hours" => 3_600,
            "d" | "day" | "days" => 86_400,
            "" => return Err(UnitError::Malformed(text.trim().to_owned())),
            other => {
                return Err(UnitError::UnknownUnit {
                    value: text.trim().to_owned(),
                    unit: other.to_owned(),
                });
            }
        };

        let seconds = number
            .checked_mul(seconds_per)
            .ok_or_else(|| UnitError::Overflow(text.trim().to_owned()))?;

        Ok(Self(Duration::from_secs(seconds)))
    }

    pub fn as_secs(self) -> u64 {
        self.0.as_secs()
    }

    /// Seconds as `i64`, the representation used for timestamps in the database.
    pub fn as_secs_i64(self) -> i64 {
        i64::try_from(self.0.as_secs()).unwrap_or(i64::MAX)
    }
}

impl<'de> Deserialize<'de> for HumanDuration {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// A byte size written as `512`, `64KiB`, `10MiB` or `1GiB`.
///
/// Only binary units are accepted. `MB` is rejected rather than guessed at,
/// because a limit that is off by 4.9 % is a limit someone will have to debug.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ByteSize(pub u64);

impl ByteSize {
    pub fn parse(text: &str) -> Result<Self, UnitError> {
        let (number, unit) = split(text)?;

        let multiplier = match unit {
            "" | "B" => 1_u64,
            "KiB" => 1 << 10,
            "MiB" => 1 << 20,
            "GiB" => 1 << 30,
            other => {
                return Err(UnitError::UnknownUnit {
                    value: text.trim().to_owned(),
                    unit: other.to_owned(),
                });
            }
        };

        let bytes = number
            .checked_mul(multiplier)
            .ok_or_else(|| UnitError::Overflow(text.trim().to_owned()))?;

        Ok(Self(bytes))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn as_usize(self) -> usize {
        usize::try_from(self.0).unwrap_or(usize::MAX)
    }
}

impl<'de> Deserialize<'de> for ByteSize {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_durations() {
        assert_eq!(HumanDuration::parse("30s").expect("30s").as_secs(), 30);
        assert_eq!(HumanDuration::parse("15m").expect("15m").as_secs(), 900);
        assert_eq!(HumanDuration::parse("24h").expect("24h").as_secs(), 86_400);
        assert_eq!(HumanDuration::parse("7d").expect("7d").as_secs(), 604_800);
        assert_eq!(HumanDuration::parse("90d").expect("90d").as_secs(), 7_776_000);
        // Whitespace is tolerated, since a config file is written by hand.
        assert_eq!(HumanDuration::parse(" 7 d ").expect("spaced").as_secs(), 604_800);
    }

    /// A bare number is rejected on purpose: "expiry = 7" is ambiguous, and
    /// guessing seconds would silently turn a week into a moment.
    #[test]
    fn rejects_durations_without_a_unit() {
        assert!(HumanDuration::parse("7").is_err());
    }

    #[test]
    fn rejects_bad_durations() {
        for bad in ["", "d", "7x", "7 weeks", "-1d", "abc", "7d5h"] {
            assert!(HumanDuration::parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn parses_byte_sizes() {
        assert_eq!(ByteSize::parse("512").expect("512").as_u64(), 512);
        assert_eq!(ByteSize::parse("512B").expect("512B").as_u64(), 512);
        assert_eq!(ByteSize::parse("64KiB").expect("64KiB").as_u64(), 65_536);
        assert_eq!(ByteSize::parse("10MiB").expect("10MiB").as_u64(), 10_485_760);
        assert_eq!(ByteSize::parse("1GiB").expect("1GiB").as_u64(), 1_073_741_824);
    }

    /// Decimal units are rejected rather than approximated. Accepting "MB" as
    /// 1_000_000 next to "MiB" as 1_048_576 invites a limit that is subtly wrong.
    #[test]
    fn rejects_decimal_and_unknown_units() {
        for bad in ["10MB", "10mb", "10 gigabytes", "10KB", "10Kib", "10mib"] {
            assert!(ByteSize::parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn rejects_overflow() {
        assert!(ByteSize::parse("99999999999999999999GiB").is_err());
        assert!(HumanDuration::parse("99999999999999999999d").is_err());
    }
}
