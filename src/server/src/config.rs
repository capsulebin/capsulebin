//! Configuration: TOML file plus a `CAPSULEBIN__` environment overlay.
//!
//! Two decisions worth knowing before changing anything here.
//!
//! **Unknown fields are an error.** Every table is `deny_unknown_fields`. A typo
//! like `burn_after_retreival` would otherwise be ignored, and the operator would
//! believe a setting is active when it is not. For security-relevant settings
//! that failure mode is unacceptable, so the process refuses to start.
//!
//! **Validation is separate from parsing.** Parsing produces a syntactically
//! valid structure; [`Config::validate`] then rejects combinations that parse but
//! are unsafe or contradictory — a maximum below a default, a token mode without
//! tokens, an unwritable database directory. `check` runs exactly this, so a
//! deployment can be verified without opening a port.

pub mod units;

mod env;
mod model;

pub use model::{
    AbuseConfig, Config, CreationConfig, CreationMode, DEFAULT_LANGUAGE, DatabaseConfig, I18nConfig, KNOWN_LANGUAGES, LegalConfig,
    LegalSection, LegalSectionKind, LegalTexts, PastesConfig, PurgeConfig, ServerConfig, Synchronous, language_prefix,
};

use std::path::{Path, PathBuf};

use crate::domain::Verifier;

use units::ByteSize;

// Re-exported at module-private visibility, `cfg(test)` like `mod tests`
// below, purely so that `config::tests` (a sibling of `env`/`model`, out of
// scope for this split) keeps working via its existing `use super::*;`.
use env::apply_env;
#[cfg(test)]
use env::parse_scalar;
#[cfg(test)]
use units::HumanDuration;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Position and reason only. The parser's own rendering quotes the
    /// offending line, which may hold a secret - a share link pasted without
    /// quotes as `public_url`, a token hash -, and this text reaches stderr and
    /// the service log.
    #[error("{path} is not valid TOML: {reason}")]
    Parse { path: PathBuf, reason: String },
    #[error("environment overlay is not valid: {0}")]
    Env(String),
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

/// Everything the running server needs, derived once at startup so that no
/// request has to re-parse or re-validate.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub config: Config,
    /// Parsed creation token verifiers.
    pub creation_tokens: Vec<Verifier>,
    /// Largest accepted request body, including base64 overhead.
    pub max_body_bytes: usize,
    /// The legal pages and footer links of every interface language, read
    /// and rendered, in the order of [`KNOWN_LANGUAGES`].
    pub legal: Vec<(&'static str, crate::legal::LegalPages)>,
    /// `i18n.default_language`: the language of the pages without a prefix.
    pub default_language: &'static str,
}

/// Where and why `text` is not valid TOML, without quoting any of it.
fn parse_reason(text: &str, error: &toml::de::Error) -> String {
    let message = error.message();
    let Some(before) = error.span().and_then(|span| text.get(..span.start)) else {
        return message.to_owned();
    };
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |last| last.chars().count()) + 1;
    format!("line {line}, column {column}: {message}")
}

impl Config {
    /// Loads the configuration: optional file, then the environment overlay.
    ///
    /// A missing file is not an error — the defaults plus environment variables
    /// are a legitimate way to run in a container. An unreadable or malformed
    /// file is an error.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let mut table = match path {
            Some(path) if path.exists() => {
                let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
                    path: path.to_owned(),
                    source,
                })?;
                toml::from_str::<toml::Table>(&text).map_err(|error| ConfigError::Parse {
                    path: path.to_owned(),
                    reason: parse_reason(&text, &error),
                })?
            }
            Some(path) => {
                return Err(ConfigError::Read {
                    path: path.to_owned(),
                    source: std::io::Error::new(std::io::ErrorKind::NotFound, "configuration file does not exist"),
                });
            }
            None => toml::Table::new(),
        };

        apply_env(&mut table, std::env::vars())?;

        table.try_into::<Self>().map_err(|source| ConfigError::Invalid(source.to_string()))
    }

    /// Rejects configurations that parse but are unsafe or self-contradictory.
    pub fn validate(&self) -> Result<Resolved, ConfigError> {
        self.server.validate()?;
        self.pastes.validate()?;
        self.purge.validate()?;
        self.abuse.validate()?;
        self.rate_limits.validate()?;
        let creation_tokens = self.creation.validate()?;
        // Before the legal pages: their links depend on the default language.
        let default_language = self.i18n.validate()?;
        self.legal.validate(default_language)?;
        let legal = KNOWN_LANGUAGES
            .iter()
            .map(|&language| Ok((language, crate::legal::resolve(self, language)?)))
            .collect::<Result<_, ConfigError>>()?;
        self.database.validate()?;

        Ok(Resolved {
            max_body_bytes: max_body_bytes(self.pastes.max_plaintext_size),
            config: self.clone(),
            creation_tokens,
            legal,
            default_language,
        })
    }
}

/// Request body limit derived from the plaintext limit.
///
/// A creation request carries the ciphertext base64-encoded, which costs 4 bytes
/// per 3 (+33 %), plus the public header, two verifiers and the JSON scaffolding.
/// The limit is therefore the plaintext budget times 4/3, plus a fixed allowance
/// — being generous here is safe, since the exact ciphertext size is checked
/// separately after decoding.
pub fn max_body_bytes(max_plaintext: ByteSize) -> usize {
    const OVERHEAD: u64 = 64 * 1024;
    let base64 = max_plaintext.as_u64().saturating_mul(4) / 3;
    usize::try_from(base64.saturating_add(OVERHEAD)).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests;
