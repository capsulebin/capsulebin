//! Configuration structure: tables, defaults and per-table validation.
//!
//! Each table validates itself; [`super::Config::validate`] only sequences the
//! calls and folds the results into a [`super::Resolved`].

use std::path::{Path, PathBuf};

use crate::domain::Verifier;
use serde::Deserialize;

use super::ConfigError;
use super::units::{ByteSize, HumanDuration};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub pastes: PastesConfig,
    #[serde(default)]
    pub creation: CreationConfig,
    #[serde(default)]
    pub purge: PurgeConfig,
    #[serde(default)]
    pub rate_limits: crate::api::rate_limit::RateLimitConfig,
    #[serde(default)]
    pub abuse: AbuseConfig,
    #[serde(default)]
    pub legal: LegalConfig,
    #[serde(default)]
    pub i18n: I18nConfig,
}

fn default_bind() -> String {
    "127.0.0.1:8080".to_owned()
}

fn default_database_path() -> PathBuf {
    PathBuf::from("capsulebin.db")
}

fn default_busy_timeout_ms() -> u64 {
    5_000
}

fn default_synchronous() -> Synchronous {
    Synchronous::Full
}

fn default_expiry() -> HumanDuration {
    HumanDuration(std::time::Duration::from_secs(604_800))
}

fn default_maximum_expiry() -> HumanDuration {
    HumanDuration(std::time::Duration::from_secs(7_776_000))
}

fn default_max_plaintext_size() -> ByteSize {
    ByteSize(10 * 1024 * 1024)
}

fn default_max_public_header_size() -> ByteSize {
    ByteSize(2 * 1024)
}

/// Hard ceiling for `pastes.max_public_header_size`.
///
/// The public header holds a format version, a nonce, a salt and an iteration
/// count — a few dozen bytes. Room for a few future fields is useful; room for
/// kilobytes of operator-configured slack is not, because the header is stored
/// for every paste and returned on every descriptor request.
const MAX_PUBLIC_HEADER_CEILING: u64 = 4 * 1024;

fn default_never_retrieved_after() -> HumanDuration {
    HumanDuration(std::time::Duration::from_secs(2_592_000))
}

fn default_inactive_after() -> HumanDuration {
    HumanDuration(std::time::Duration::from_secs(7_776_000))
}

fn default_retrieval_timestamp_granularity() -> HumanDuration {
    HumanDuration(std::time::Duration::from_secs(3_600))
}

fn default_purge_batch_size() -> i64 {
    500
}

fn default_report_retention() -> HumanDuration {
    HumanDuration(std::time::Duration::from_secs(7_776_000))
}

fn default_true() -> bool {
    true
}

fn default_creation_mode() -> CreationMode {
    CreationMode::Token
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Absolute, externally reachable base URL. Required because it appears in
    /// `problem+json` `type` URIs and in generated share links; guessing it from
    /// a `Host` header would let a request rewrite what the operator publishes.
    pub public_url: Option<String>,
    /// Directory to serve assets from instead of using the embedded ones.
    /// Development only.
    pub asset_dir: Option<PathBuf>,
    /// CIDR ranges of reverse proxies whose `X-Forwarded-For` header is trusted.
    /// An empty list means only the TCP peer IP is used.
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            public_url: None,
            asset_dir: None,
            trusted_proxies: Vec::new(),
        }
    }
}

impl ServerConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: String| ConfigError::Invalid(message);

        self.bind
            .parse::<std::net::SocketAddr>()
            .map_err(|_| invalid(format!("server.bind is not a socket address: {}", self.bind)))?;

        let public_url = self
            .public_url
            .as_deref()
            .ok_or_else(|| invalid("server.public_url is required".to_owned()))?;

        if !(public_url.starts_with("https://") || public_url.starts_with("http://")) {
            return Err(invalid(format!(
                "server.public_url must be an absolute http(s) URL, got {public_url:?}"
            )));
        }

        // A trailing slash would produce "//problems/..." in problem type URIs.
        if public_url.ends_with('/') {
            return Err(invalid("server.public_url must not end with a slash".to_owned()));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    #[serde(default = "default_database_path")]
    pub path: PathBuf,
    #[serde(default = "default_busy_timeout_ms")]
    pub busy_timeout_ms: u64,
    #[serde(default = "default_synchronous")]
    pub synchronous: Synchronous,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_database_path(),
            busy_timeout_ms: default_busy_timeout_ms(),
            synchronous: default_synchronous(),
        }
    }
}

impl DatabaseConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: String| ConfigError::Invalid(message);

        let directory = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));

        if !directory.is_dir() {
            return Err(invalid(format!("database directory {} does not exist", directory.display())));
        }

        // Checked by writing, not by reading permission bits: those lie under
        // containers, on read-only mounts and with unusual ownership.
        let probe = directory.join(".capsulebin-write-probe");
        match std::fs::write(&probe, b"") {
            Ok(()) => {
                let _ = std::fs::remove_file(&probe);
            }
            Err(source) => {
                return Err(invalid(format!(
                    "database directory {} is not writable: {source}",
                    directory.display()
                )));
            }
        }

        if self.busy_timeout_ms == 0 {
            return Err(invalid(
                "database.busy_timeout_ms must be greater than zero, otherwise a \
                 concurrent write fails immediately"
                    .to_owned(),
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Synchronous {
    /// Fastest, and the only setting that can lose an acknowledged write.
    Off,
    Normal,
    Full,
}

impl Synchronous {
    pub fn as_pragma(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Normal => "NORMAL",
            Self::Full => "FULL",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
/// Limits the server can actually enforce. Anything about what is *inside* a
/// capsule — item count, file types — is invisible to it by design and therefore
/// not configurable here: a limit a different client could ignore is not a limit.
pub struct PastesConfig {
    #[serde(default = "default_expiry")]
    pub default_expiry: HumanDuration,
    #[serde(default = "default_maximum_expiry")]
    pub maximum_expiry: HumanDuration,
    #[serde(default)]
    pub allow_permanent: bool,
    /// Limit on the plaintext a client may pack into one capsule. The server
    /// cannot check this directly — it never sees plaintext — so it is enforced
    /// against the ciphertext size, which is the plaintext plus a bounded
    /// overhead.
    #[serde(default = "default_max_plaintext_size")]
    pub max_plaintext_size: ByteSize,
    /// Limit on the opaque public header. Small on purpose: the header is the one
    /// client-supplied blob the server hands back on an unauthenticated descriptor
    /// request, so it should not be a place to park kilobytes.
    #[serde(default = "default_max_public_header_size")]
    pub max_public_header_size: ByteSize,
}

impl Default for PastesConfig {
    fn default() -> Self {
        Self {
            default_expiry: default_expiry(),
            maximum_expiry: default_maximum_expiry(),
            allow_permanent: false,
            max_plaintext_size: default_max_plaintext_size(),
            max_public_header_size: default_max_public_header_size(),
        }
    }
}

impl PastesConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: String| ConfigError::Invalid(message);

        if self.maximum_expiry.as_secs() == 0 {
            return Err(invalid("pastes.maximum_expiry must be greater than zero".to_owned()));
        }

        if self.default_expiry.as_secs() == 0 {
            return Err(invalid("pastes.default_expiry must be greater than zero".to_owned()));
        }

        if self.default_expiry.as_secs() > self.maximum_expiry.as_secs() {
            return Err(invalid(format!(
                "pastes.default_expiry ({}s) exceeds pastes.maximum_expiry ({}s)",
                self.default_expiry.as_secs(),
                self.maximum_expiry.as_secs()
            )));
        }

        if self.max_plaintext_size.as_u64() == 0 {
            return Err(invalid("pastes.max_plaintext_size must be greater than zero".to_owned()));
        }

        // An upper bound so that a typo cannot turn into a memory exhaustion
        // vector: every request body is buffered before it is decoded.
        const MAX_ALLOWED: u64 = 128 * 1024 * 1024;
        if self.max_plaintext_size.as_u64() > MAX_ALLOWED {
            return Err(invalid(format!(
                "pastes.max_plaintext_size is {} bytes; more than {MAX_ALLOWED} is not supported",
                self.max_plaintext_size.as_u64()
            )));
        }

        if self.max_public_header_size.as_u64() == 0 {
            return Err(invalid("pastes.max_public_header_size must be greater than zero".to_owned()));
        }

        if self.max_public_header_size.as_u64() > MAX_PUBLIC_HEADER_CEILING {
            return Err(invalid(format!(
                "pastes.max_public_header_size is {} bytes; more than {MAX_PUBLIC_HEADER_CEILING} is not supported",
                self.max_public_header_size.as_u64()
            )));
        }

        Ok(())
    }
}

/// Retention for the purge job.
///
/// **There is no interval setting, and that is deliberate.** The job runs once
/// after startup and then once a day. An operator who could shorten the interval
/// would be tuning write amplification against retention accuracy without the
/// information to do it well, and one who could lengthen it would silently defeat
/// the retention promise the instance makes. What *is* configurable is what gets
/// removed, not how often the question is asked.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Age after creation at which a capsule nobody ever retrieved is removed.
    #[serde(default = "default_never_retrieved_after")]
    pub never_retrieved_after: HumanDuration,
    /// Age after the last authorized retrieval at which a capsule is removed.
    #[serde(default = "default_inactive_after")]
    pub inactive_after: HumanDuration,
    /// How coarsely `last_retrieved_at` is advanced. Larger means fewer writes and
    /// a less precise inactivity measurement.
    #[serde(default = "default_retrieval_timestamp_granularity")]
    pub retrieval_timestamp_granularity: HumanDuration,
    /// Rows removed per rule per statement. The job keeps batching until a run
    /// removes nothing, so this bounds the size of a single write, not the total.
    #[serde(default = "default_purge_batch_size")]
    pub batch_size: i64,
}

impl Default for PurgeConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            never_retrieved_after: default_never_retrieved_after(),
            inactive_after: default_inactive_after(),
            retrieval_timestamp_granularity: default_retrieval_timestamp_granularity(),
            batch_size: default_purge_batch_size(),
        }
    }
}

impl PurgeConfig {
    /// The thresholds the repository works with, in seconds.
    pub fn policy(&self) -> crate::domain::PurgePolicy {
        crate::domain::PurgePolicy {
            never_retrieved_after: self.never_retrieved_after.as_secs_i64(),
            inactive_after: self.inactive_after.as_secs_i64(),
        }
    }

    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: String| ConfigError::Invalid(message);

        for (name, value) in [
            ("purge.never_retrieved_after", self.never_retrieved_after),
            ("purge.inactive_after", self.inactive_after),
            ("purge.retrieval_timestamp_granularity", self.retrieval_timestamp_granularity),
        ] {
            if value.as_secs() == 0 {
                return Err(invalid(format!("{name} must be greater than zero")));
            }
        }

        // A granularity coarser than the inactivity window would let the timestamp
        // stay behind the very threshold it is measured against, so the purge would
        // remove capsules that are still being retrieved.
        if self.retrieval_timestamp_granularity.as_secs() > self.inactive_after.as_secs() {
            return Err(invalid(format!(
                "purge.retrieval_timestamp_granularity ({}s) exceeds purge.inactive_after ({}s); \
                 capsules would be purged while still in use",
                self.retrieval_timestamp_granularity.as_secs(),
                self.inactive_after.as_secs()
            )));
        }

        const MAX_PURGE_BATCH: i64 = 10_000;
        if self.batch_size < 1 || self.batch_size > MAX_PURGE_BATCH {
            return Err(invalid(format!(
                "purge.batch_size must be between 1 and {MAX_PURGE_BATCH}, got {}",
                self.batch_size
            )));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreationMode {
    /// Read-only instance: existing pastes stay retrievable, nothing new is
    /// accepted.
    Disabled,
    /// A bearer token from `token_hashes` is required.
    Token,
    /// Anyone may create. Only sensible behind another access control.
    Public,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreationConfig {
    #[serde(default = "default_creation_mode")]
    pub mode: CreationMode,
    /// Entries in the form `sha256:<base64url>`. The plain token is never
    /// configured, so the file does not have to be treated as a secret store.
    #[serde(default)]
    pub token_hashes: Vec<String>,
}

impl Default for CreationConfig {
    fn default() -> Self {
        // Token mode by default: an instance that is reachable before it is
        // configured should not be an open write target.
        Self {
            mode: default_creation_mode(),
            token_hashes: Vec::new(),
        }
    }
}

impl CreationConfig {
    pub(super) fn validate(&self) -> Result<Vec<Verifier>, ConfigError> {
        let invalid = |message: String| ConfigError::Invalid(message);

        let mut creation_tokens = Vec::new();
        for entry in &self.token_hashes {
            let hash = entry
                .strip_prefix("sha256:")
                .ok_or_else(|| invalid("creation.token_hashes entries must start with \"sha256:\"".to_owned()))?;
            let verifier =
                Verifier::parse(hash).map_err(|source| invalid(format!("creation.token_hashes contains an invalid hash: {source}")))?;
            creation_tokens.push(verifier);
        }

        // Token mode without tokens accepts nothing, which looks like a broken
        // server rather than a configuration mistake. Fail loudly instead.
        if self.mode == CreationMode::Token && creation_tokens.is_empty() {
            return Err(invalid("creation.mode is \"token\" but creation.token_hashes is empty".to_owned()));
        }

        if self.mode != CreationMode::Token && !creation_tokens.is_empty() {
            return Err(invalid(format!(
                "creation.token_hashes is set but creation.mode is {:?}; \
                 the tokens would have no effect",
                self.mode
            )));
        }

        Ok(creation_tokens)
    }
}

/// Notice and action (ADR-013): the report endpoint and its retention.
///
/// There is deliberately no switch that makes a moderated paste look different
/// from any other unavailable one — disabled and deleted answer exactly like
/// unknown and expired (invariant 5).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbuseConfig {
    /// Accept reports at `POST /api/v1/reports`. Moderation commands work
    /// regardless.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Shown on the report page as an alternative channel (an address or URL).
    #[serde(default)]
    pub contact: Option<String>,
    /// How long closed reports and moderation audit entries are kept. Reports
    /// carry free text from the public and are personal data; open ones are kept
    /// until an operator closes them.
    #[serde(default = "default_report_retention")]
    pub report_retention: HumanDuration,
}

impl Default for AbuseConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            contact: None,
            report_retention: default_report_retention(),
        }
    }
}

impl AbuseConfig {
    pub(super) fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |message: String| ConfigError::Invalid(message);

        if self.report_retention.as_secs() == 0 {
            return Err(invalid("abuse.report_retention must be greater than zero".to_owned()));
        }
        if let Some(contact) = &self.contact
            && (contact.trim().is_empty() || contact.len() > 254 || contact.chars().any(char::is_control))
        {
            return Err(invalid(
                "abuse.contact must be a non-empty address or URL of at most 254 bytes without control characters".to_owned(),
            ));
        }
        Ok(())
    }
}

/// The operator's legal pages: imprint, privacy notice and terms of use.
///
/// CapsuleBin serves them itself at `/imprint`, `/privacy` and `/terms`, from
/// Markdown given inline (`text`) or in a file (`file`), or links to pages
/// hosted elsewhere (`url`). The privacy notice and the terms have built-in
/// defaults that describe what the software does with the configured retention
/// and limits; they need an operator, because a privacy notice without a
/// controller is worse than none. Without `[legal.operator]` and without
/// explicit privacy or terms there are no legal pages and no footer links.
///
/// Resolved, read and rendered once at startup by [`crate::legal::resolve`].
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegalConfig {
    /// Who runs the instance: the imprint (e.g. the German "Impressum").
    #[serde(default)]
    pub operator: Option<LegalSection>,
    /// Who hosts it, if that is someone else: a second block in the imprint,
    /// and named as processor in the built-in privacy notice.
    #[serde(default)]
    pub hosting: Option<LegalSection>,
    /// Replaces or extends (`extra`) the built-in privacy notice.
    #[serde(default)]
    pub privacy: Option<LegalSection>,
    /// Replaces or extends (`extra`) the built-in terms of use.
    #[serde(default)]
    pub terms: Option<LegalSection>,
}

/// One legal text. At most one of `text`, `file` and `url`; `extra` and
/// `extra_file` add Markdown to a built-in text and are only valid without
/// them. Which combinations a section accepts is checked by
/// [`crate::legal::resolve`], which knows the section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegalSection {
    /// Markdown, inline.
    #[serde(default)]
    pub text: Option<String>,
    /// Markdown, read from this file at startup.
    #[serde(default)]
    pub file: Option<PathBuf>,
    /// A page hosted elsewhere; the footer links there.
    #[serde(default)]
    pub url: Option<String>,
    /// Markdown appended to the built-in text, inline.
    #[serde(default)]
    pub extra: Option<String>,
    /// Markdown appended to the built-in text, read from this file at startup.
    #[serde(default)]
    pub extra_file: Option<PathBuf>,
}

/// Interface languages the server knows: a shell is built for each of them
/// (`src/server/web/src/i18n/<code>.json`). The same list lives in
/// `@capsulebin/core` (`languages.ts`); a test keeps the three in step.
pub const KNOWN_LANGUAGES: &[&str] = &["en", "de"];

fn default_language() -> String {
    "en".to_owned()
}

fn default_languages() -> Vec<String> {
    KNOWN_LANGUAGES.iter().map(|code| (*code).to_owned()).collect()
}

/// Which interface languages the instance offers, each under its path prefix
/// (`/en/`, `/de/`), and which one a request without a usable
/// `Accept-Language` is sent to.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct I18nConfig {
    #[serde(default = "default_language")]
    pub default_language: String,
    /// Offered languages, in the order the language switcher lists them.
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
}

impl Default for I18nConfig {
    fn default() -> Self {
        Self {
            default_language: default_language(),
            languages: default_languages(),
        }
    }
}

impl I18nConfig {
    /// The offered languages in configuration order, as entries of
    /// [`KNOWN_LANGUAGES`], and the default among them.
    pub(super) fn validate(&self) -> Result<(Vec<&'static str>, &'static str), ConfigError> {
        let invalid = |message: String| ConfigError::Invalid(message);

        if self.languages.is_empty() {
            return Err(invalid("i18n.languages must name at least one language".to_owned()));
        }
        let mut offered: Vec<&'static str> = Vec::with_capacity(self.languages.len());
        for code in &self.languages {
            let known = known_language(code).ok_or_else(|| {
                invalid(format!(
                    "i18n.languages contains {code:?}, which is not a known language ({})",
                    KNOWN_LANGUAGES.join(", ")
                ))
            })?;
            if offered.contains(&known) {
                return Err(invalid(format!("i18n.languages contains {code:?} more than once")));
            }
            offered.push(known);
        }
        let default = offered.iter().copied().find(|code| *code == self.default_language).ok_or_else(|| {
            invalid(format!(
                "i18n.default_language {:?} is not one of i18n.languages ({})",
                self.default_language,
                self.languages.join(", ")
            ))
        })?;
        Ok((offered, default))
    }
}

/// The entry of [`KNOWN_LANGUAGES`] equal to `code`.
fn known_language(code: &str) -> Option<&'static str> {
    KNOWN_LANGUAGES.iter().copied().find(|known| *known == code)
}
