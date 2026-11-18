//! Abuse reports and moderation (notice and action, ADR-013).
//!
//! Zero knowledge rules out looking at content, not acting on a concrete paste:
//! a report names a paste id, and the operator can disable, delete or restore
//! that paste without ever decrypting it. Nothing here touches ciphertext
//! beyond dropping it on delete.
//!
//! Reports carry free text from the public — reasons, descriptions, contact
//! addresses. That is personal data with its own retention
//! (`abuse.report_retention`), and it never appears in a log.

use core::fmt;

use super::PasteId;

/// Upper bound for the free-text description, in bytes of UTF-8.
pub const MAX_DESCRIPTION_BYTES: usize = 4_000;
/// Upper bound for the optional contact field, in bytes of UTF-8.
pub const MAX_CONTACT_BYTES: usize = 254;
/// Upper bound for an operator note on a moderation action.
pub const MAX_NOTE_BYTES: usize = 1_000;

macro_rules! code_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }

            pub fn parse(text: &str) -> Option<Self> {
                match text {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

code_enum! {
    /// Why someone reports a paste. Also used as the reason of a moderation
    /// action, so the audit trail speaks the same vocabulary as the reports.
    ReasonCode {
        IllegalContent => "illegal_content",
        Malware => "malware",
        Phishing => "phishing",
        Spam => "spam",
        PersonalData => "personal_data",
        Copyright => "copyright",
        Other => "other",
    }
}

code_enum! {
    /// Lifecycle of a report in the operator's queue.
    ReportStatus {
        Open => "open",
        Reviewing => "reviewing",
        Closed => "closed",
    }
}

code_enum! {
    /// How the operator closed a report.
    Resolution {
        /// The paste was disabled or deleted.
        ActionTaken => "action_taken",
        /// Reviewed, nothing to do.
        NoAction => "no_action",
        /// Another report about the same paste covers it.
        Duplicate => "duplicate",
        /// Not a meaningful report (spam, wrong id, …).
        Invalid => "invalid",
    }
}

code_enum! {
    /// What an operator did to a paste. Recorded in `moderation_actions`.
    ModerationAction {
        /// Blocked; payload kept so the decision can be reversed.
        Disable => "disable",
        /// Payload removed; only a tombstone remains. Irreversible.
        Delete => "delete",
        /// A disabled paste made available again.
        Restore => "restore",
    }
}

/// A validated report, ready to store.
#[derive(Clone)]
pub struct NewReport {
    pub id: String,
    pub paste_id: PasteId,
    pub reason: ReasonCode,
    pub description: String,
    pub contact: Option<String>,
    pub created_at: i64,
}

/// Prints neither the description nor the contact: both are free text from the
/// public and must not end up in a log by way of a `{:?}`.
impl fmt::Debug for NewReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewReport")
            .field("id", &self.id)
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ReportError {
    #[error("unknown reason code")]
    Reason,
    #[error("description must not be empty")]
    EmptyDescription,
    #[error("description exceeds {MAX_DESCRIPTION_BYTES} bytes")]
    DescriptionTooLong,
    #[error("contact exceeds {MAX_CONTACT_BYTES} bytes")]
    ContactTooLong,
    #[error("text contains control characters")]
    ControlCharacters,
    /// A report must never carry the decryption key. A share link's fragment
    /// (`#v1.…`) in the description is refused rather than stored.
    #[error("text contains a link fragment with a key")]
    ContainsKey,
}

/// Rejects text that would be awkward or harmful to show an operator in a
/// terminal (control characters other than line breaks and tabs).
fn check_text(text: &str) -> Result<(), ReportError> {
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) {
        return Err(ReportError::ControlCharacters);
    }
    if text.contains("#v1.") {
        return Err(ReportError::ContainsKey);
    }
    Ok(())
}

impl NewReport {
    /// Validates the public-facing fields. The paste id is already parsed.
    pub fn validate(
        id: String,
        paste_id: PasteId,
        reason: &str,
        description: &str,
        contact: Option<&str>,
        created_at: i64,
    ) -> Result<Self, ReportError> {
        let reason = ReasonCode::parse(reason).ok_or(ReportError::Reason)?;

        let description = description.trim();
        if description.is_empty() {
            return Err(ReportError::EmptyDescription);
        }
        if description.len() > MAX_DESCRIPTION_BYTES {
            return Err(ReportError::DescriptionTooLong);
        }
        check_text(description)?;

        let contact = contact.map(str::trim).filter(|c| !c.is_empty());
        if let Some(contact) = contact {
            if contact.len() > MAX_CONTACT_BYTES {
                return Err(ReportError::ContactTooLong);
            }
            if contact.chars().any(char::is_control) {
                return Err(ReportError::ControlCharacters);
            }
            check_text(contact)?;
        }

        Ok(Self {
            id,
            paste_id,
            reason,
            description: description.to_owned(),
            contact: contact.map(str::to_owned),
            created_at,
        })
    }
}

/// A stored report, as the operator sees it.
#[derive(Clone)]
pub struct Report {
    pub id: String,
    pub paste_id: String,
    pub reason: String,
    pub description: String,
    pub contact: Option<String>,
    pub created_at: i64,
    pub status: String,
    pub handled_at: Option<i64>,
    pub resolution: Option<String>,
}

impl fmt::Debug for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Report")
            .field("id", &self.id)
            .field("reason", &self.reason)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// One entry of the moderation audit trail.
#[derive(Debug, Clone)]
pub struct ModerationRecord {
    pub paste_id: String,
    pub action: String,
    pub reason: Option<String>,
    pub note: Option<String>,
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paste_id() -> PasteId {
        PasteId::from_bytes([1; 16])
    }

    fn validate(reason: &str, description: &str, contact: Option<&str>) -> Result<NewReport, ReportError> {
        NewReport::validate("r1".into(), paste_id(), reason, description, contact, 1_000)
    }

    #[test]
    fn codes_round_trip() {
        for code in ReasonCode::ALL {
            assert_eq!(ReasonCode::parse(code.as_str()), Some(*code));
        }
        for code in Resolution::ALL {
            assert_eq!(Resolution::parse(code.as_str()), Some(*code));
        }
        assert_eq!(ReasonCode::parse("Malware"), None);
    }

    #[test]
    fn accepts_a_plain_report_and_trims_it() {
        let report = validate("phishing", "  fake bank login  ", Some("  ops@example.org ")).unwrap();
        assert_eq!(report.reason, ReasonCode::Phishing);
        assert_eq!(report.description, "fake bank login");
        assert_eq!(report.contact.as_deref(), Some("ops@example.org"));

        let anonymous = validate("spam", "x", Some("   ")).unwrap();
        assert_eq!(anonymous.contact, None);
    }

    #[test]
    fn rejects_bad_fields() {
        assert_eq!(validate("nope", "x", None).err(), Some(ReportError::Reason));
        assert_eq!(validate("spam", "   ", None).err(), Some(ReportError::EmptyDescription));
        let long = "a".repeat(MAX_DESCRIPTION_BYTES + 1);
        assert_eq!(validate("spam", &long, None).err(), Some(ReportError::DescriptionTooLong));
        let contact = "a".repeat(MAX_CONTACT_BYTES + 1);
        assert_eq!(validate("spam", "x", Some(&contact)).err(), Some(ReportError::ContactTooLong));
        assert_eq!(validate("spam", "a\u{1b}[2J", None).err(), Some(ReportError::ControlCharacters));
        assert_eq!(validate("spam", "x", Some("a\nb")).err(), Some(ReportError::ControlCharacters));
    }

    #[test]
    fn line_breaks_are_fine_in_the_description() {
        assert!(validate("other", "line one\nline two\r\n\tindented", None).is_ok());
    }

    #[test]
    fn refuses_a_decryption_key() {
        let text = "see https://paste.example/p/AAAAAAAAAAAAAAAAAAAAAA#v1.c2VjcmV0";
        assert_eq!(validate("other", text, None).err(), Some(ReportError::ContainsKey));
    }

    #[test]
    fn debug_output_hides_free_text() {
        let report = validate("spam", "SECRET-DESCRIPTION", Some("SECRET-CONTACT")).unwrap();
        let debug = format!("{report:?}");
        assert!(!debug.contains("SECRET"), "{debug}");
    }
}
