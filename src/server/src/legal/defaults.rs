//! The built-in privacy notice and terms of use, in every interface language
//! (`en.rs`, `de.rs`).
//!
//! What CapsuleBin does with data is fixed by the software and its
//! configuration, so these texts state it concretely - the retention periods
//! and whether client addresses are processed come from the running
//! configuration, not from a template the operator has to keep in sync. What
//! the software cannot know (the operator's identity, a hosting provider, the
//! reverse proxy's logs, backups) comes from `[legal]`: the operator and the
//! hosting provider are named in the imprint, everything else goes into
//! `extra`.
//!
//! Every language states the same facts under the same headings with the same
//! links; the `[Facts]` are language-neutral and each language words them,
//! durations included.
//!
//! The texts are a careful default, not legal advice; the operator stays
//! responsible for what the instance publishes (docs/privacy-notice.md).

mod de;
mod en;

use crate::config::Config;

/// What the texts state about the running instance.
#[derive(Debug, Clone)]
pub struct Facts {
    /// Where the imprint is; `None` when there is no operator.
    pub imprint_href: Option<String>,
    /// Where the privacy notice is, for the terms to refer to.
    pub privacy_href: String,
    /// Where the report form is, for the terms to refer to.
    pub report_href: String,
    pub hosting: bool,
    /// Whether any limit keyed by the client address is on.
    pub client_ip_limits: bool,
    pub purge: bool,
    pub never_retrieved_after: u64,
    pub inactive_after: u64,
    pub maximum_expiry: u64,
    pub allow_permanent: bool,
    pub retrieval_granularity: u64,
    pub reports: bool,
    pub report_retention: u64,
}

impl Facts {
    /// `prefix` is the language prefix of the shell the texts are shown in
    /// (`/de`, empty for the default language), for the links to pages served
    /// here.
    pub fn from_config(config: &Config, prefix: &str, imprint_href: Option<String>, hosting: bool) -> Self {
        Self {
            imprint_href,
            privacy_href: format!("{prefix}/privacy"),
            report_href: format!("{prefix}/report"),
            hosting,
            client_ip_limits: config.rate_limits.uses_client_ip(),
            purge: config.purge.enabled,
            never_retrieved_after: config.purge.never_retrieved_after.as_secs(),
            inactive_after: config.purge.inactive_after.as_secs(),
            maximum_expiry: config.pastes.maximum_expiry.as_secs(),
            allow_permanent: config.pastes.allow_permanent,
            retrieval_granularity: config.purge.retrieval_timestamp_granularity.as_secs(),
            reports: config.abuse.enabled,
            report_retention: config.abuse.report_retention.as_secs(),
        }
    }

    fn imprint(&self) -> &str {
        self.imprint_href.as_deref().unwrap_or("/imprint")
    }
}

/// A built-in text in two parts around the place of the operator's `extra`:
/// before the last section ("Your rights" in the privacy notice, "Changes" in
/// the terms) and that section itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltIn {
    pub before_extra: String,
    pub after_extra: String,
}

impl BuiltIn {
    fn new(before_extra: String, after_extra: &str) -> Self {
        Self {
            before_extra,
            after_extra: after_extra.to_owned(),
        }
    }

    /// The whole text as one Markdown document, with `extra` (trimmed) as its
    /// own paragraphs between the two parts.
    pub fn join(&self, extra: Option<&str>) -> String {
        match extra {
            Some(extra) => format!("{}\n\n{}\n\n{}", self.before_extra, extra.trim(), self.after_extra),
            None => format!("{}\n\n{}", self.before_extra, self.after_extra),
        }
    }
}

/// The largest unit a duration is a whole number of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Day,
    Hour,
    Minute,
    Second,
}

/// `seconds` as a count of the largest whole unit: 90 days, 1 hour. Each
/// language words the result.
fn unit(seconds: u64) -> (u64, Unit) {
    if seconds.is_multiple_of(86_400) {
        (seconds / 86_400, Unit::Day)
    } else if seconds.is_multiple_of(3_600) {
        (seconds / 3_600, Unit::Hour)
    } else if seconds.is_multiple_of(60) {
        (seconds / 60, Unit::Minute)
    } else {
        (seconds, Unit::Second)
    }
}

/// The built-in privacy notice in `language`, a code from
/// [`crate::config::model::KNOWN_LANGUAGES`]; any other code gets the English
/// text (the configuration never passes one).
pub fn privacy(f: &Facts, language: &str) -> BuiltIn {
    match language {
        "de" => de::privacy(f),
        _ => en::privacy(f),
    }
}

/// The built-in terms of use in `language`, as [`privacy`].
pub fn terms(f: &Facts, language: &str) -> BuiltIn {
    match language {
        "de" => de::terms(f),
        _ => en::terms(f),
    }
}

#[cfg(test)]
mod tests;
