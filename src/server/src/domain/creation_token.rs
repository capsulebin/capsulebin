//! Creation tokens kept in the database.
//!
//! The operator issues and revokes them while the server runs. Per token only
//! its verifier, a label, the creation and revocation time and the day of last
//! use are stored; no capsule is ever linked to the token that created it.

use core::fmt;

use super::Verifier;

/// Longest label, in characters.
pub const MAX_LABEL_CHARS: usize = 64;

/// Seconds in a UTC day; `last_used_on` is stored at this granularity.
const DAY_SECS: i64 = 86_400;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum LabelError {
    #[error("label exceeds {MAX_LABEL_CHARS} characters")]
    TooLong,
    #[error("label contains control characters")]
    ControlCharacters,
}

/// The operator's note on a token. Empty means none.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Label(String);

impl Label {
    /// Accepts at most [`MAX_LABEL_CHARS`] characters and no control
    /// characters. The empty label is allowed and stands for "none".
    pub fn parse(text: &str) -> Result<Self, LabelError> {
        if text.chars().count() > MAX_LABEL_CHARS {
            return Err(LabelError::TooLong);
        }
        if text.chars().any(char::is_control) {
            return Err(LabelError::ControlCharacters);
        }
        Ok(Self(text.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Prints no text: a label is operator input and stays out of logs.
impl fmt::Debug for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Label(redacted)")
    }
}

/// A stored token as the operator sees it, without its verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationToken {
    /// ULID, not secret.
    pub id: String,
    pub label: Label,
    pub created_at: i64,
    /// Unix seconds of 00:00 UTC of the day of last use, see [`utc_day_start`].
    pub last_used_on: Option<i64>,
    pub revoked_at: Option<i64>,
}

/// What authorization needs of a token that is not revoked: its verifier to
/// compare against, and its id and day of last use to record a use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveCreationToken {
    pub id: String,
    pub verifier: Verifier,
    pub last_used_on: Option<i64>,
}

/// 00:00 UTC of the day `now` (Unix seconds) falls on, also before 1970.
#[must_use]
pub fn utc_day_start(now: i64) -> i64 {
    now - now.rem_euclid(DAY_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_label_means_none() {
        assert_eq!(Label::parse("").unwrap().as_str(), "");
        assert_eq!(Label::default(), Label::parse("").unwrap());
    }

    #[test]
    fn a_label_counts_characters_not_bytes() {
        let longest = "ä".repeat(MAX_LABEL_CHARS);
        assert_eq!(Label::parse(&longest).unwrap().as_str(), longest);
        assert_eq!(Label::parse(&"a".repeat(MAX_LABEL_CHARS + 1)), Err(LabelError::TooLong));
    }

    #[test]
    fn a_label_refuses_control_characters() {
        for text in ["a\nb", "tab\there", "\u{1b}[2J", "del\u{7f}", "c1\u{85}"] {
            assert_eq!(Label::parse(text), Err(LabelError::ControlCharacters), "{text:?}");
        }
    }

    #[test]
    fn debug_hides_the_label() {
        let label = Label::parse("alice@example.org").unwrap();
        assert_eq!(format!("{label:?}"), "Label(redacted)");
    }

    #[test]
    fn day_start_rounds_down_to_midnight_utc() {
        assert_eq!(utc_day_start(0), 0);
        assert_eq!(utc_day_start(86_399), 0);
        assert_eq!(utc_day_start(86_400), 86_400);
        assert_eq!(utc_day_start(1_795_074_774), 1_795_046_400);
        assert_eq!(utc_day_start(-1), -86_400);
    }
}
