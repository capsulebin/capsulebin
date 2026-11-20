//! Request and response bodies for the paste API. Pure wire shapes — no
//! behaviour beyond what serde needs to parse and reject them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreateRequest {
    pub(super) id: String,
    pub(super) format_version: i64,
    pub(super) public_header: String,
    pub(super) ciphertext: String,
    pub(super) reveal_verifier: String,
    pub(super) delete_verifier: String,
    pub(super) describe_verifier: String,
    /// Seconds from now. Absent means "use the configured default"; explicit
    /// `null` means "no expiry", which the configuration may forbid.
    #[serde(default, deserialize_with = "deserialize_optional_expiry")]
    pub(super) expires_in: Option<Option<i64>>,
    #[serde(default)]
    pub(super) burn_after_retrieval: bool,
}

/// Distinguishes an absent `expires_in` from an explicit `null`.
///
/// The two mean different things — default versus permanent — and collapsing them
/// would silently turn a rejected permanent paste into a week-long one.
fn deserialize_optional_expiry<'de, D>(deserializer: D) -> Result<Option<Option<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<i64>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Serialize)]
pub(super) struct CreateResponse {
    pub(super) id: String,
    pub(super) expires_at: Option<i64>,
    pub(super) burn_after_retrieval: bool,
}

/// Lifecycle metadata plus the public header. Never the ciphertext, and only
/// for a caller holding the describe token or the delete secret.
///
/// What is *absent* matters as much as what is present: no `last_retrieved_at`.
/// It is server-side bookkeeping for the purge job, and exposing it would turn a
/// descriptor request into an oracle for whether and when someone opened a link.
#[derive(Debug, Serialize)]
pub(super) struct DescriptorResponse {
    pub(super) id: String,
    pub(super) format_version: i64,
    pub(super) public_header: String,
    pub(super) expires_at: Option<i64>,
    pub(super) burn_after_retrieval: bool,
    pub(super) payload_size: i64,
}

/// Proof of a capability that may read the descriptor: the describe token
/// (from the share link) or the delete secret (from the management link).
/// Exactly one of them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DescribeRequest {
    #[serde(default)]
    pub(super) describe_token: Option<String>,
    #[serde(default)]
    pub(super) delete_secret: Option<String>,
}

/// Both tokens from the share link. The describe token proves the caller holds
/// the link; only then does a wrong reveal token count as a password attempt
/// against the paste (see `rate_limit::LimiterState::password_attempt`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetrieveRequest {
    pub(super) describe_token: String,
    pub(super) reveal_token: String,
}

#[derive(Debug, Serialize)]
pub(super) struct RetrieveResponse {
    pub(super) public_header: String,
    pub(super) ciphertext: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteRequest {
    pub(super) delete_secret: String,
}
