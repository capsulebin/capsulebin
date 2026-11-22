//! Capability verifiers.
//!
//! The server never stores a capability secret. It stores `SHA-256` of the
//! secret and compares that against the hash of what a request presents. A
//! database leak therefore does not hand out reveal or delete rights.
//!
//! Two properties this module exists to guarantee:
//!
//! 1. **Comparison is constant time.** A short-circuiting `==` on a hash leaks,
//!    through timing, how many leading bytes were correct, which turns a
//!    128-bit guess into a byte-at-a-time search.
//! 2. **A secret cannot be logged by accident.** [`Verifier`] has a `Debug`
//!    implementation that prints nothing useful, and the secret types are
//!    consumed rather than stored.

use core::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};
use subtle::{ConditionallySelectable as _, ConstantTimeEq as _};

/// Length of a verifier in bytes.
pub const VERIFIER_BYTES: usize = 32;

/// Domain prefix hashed in front of the delete secret.
const DELETE_VERIFIER_DOMAIN: &[u8] = b"capsule/v1/delete";

/// Domain prefix hashed in front of a creation token.
///
/// Separate from [`DELETE_VERIFIER_DOMAIN`] so that the same bytes cannot serve
/// as both a creation token and a delete secret. Sharing one domain would mean an
/// operator's creation token doubles as a delete capability for any paste whose
/// secret happens to equal it.
const CREATION_TOKEN_DOMAIN: &[u8] = b"capsule/v1/creation";

/// `SHA-256` of a capability secret, as stored by the server.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Verifier([u8; VERIFIER_BYTES]);

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum VerifierError {
    #[error("verifier is not base64url without padding")]
    Encoding,
    #[error("verifier must be {VERIFIER_BYTES} bytes, got {0}")]
    Length(usize),
}

impl Verifier {
    /// Derives the verifier from a secret.
    ///
    /// Domain separation: the delete secret is hashed behind
    /// [`DELETE_VERIFIER_DOMAIN`] so that the same bytes used in another
    /// role cannot produce the same verifier.
    pub fn for_delete_secret(secret: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(DELETE_VERIFIER_DOMAIN);
        hasher.update(secret);
        Self(hasher.finalize().into())
    }

    /// Derives the verifier of an operator creation token.
    ///
    /// Domain-separated from every other capability: a creation token is issued by
    /// the operator, a delete secret by a client, and neither may stand in for the
    /// other.
    pub fn for_creation_token(token: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(CREATION_TOKEN_DOMAIN);
        hasher.update(token);
        Self(hasher.finalize().into())
    }

    /// Derives the verifier of a reveal token.
    ///
    /// The token is already the output of an HKDF step with its own `info`
    /// label, so no second domain prefix is applied here — adding one would put
    /// the same value behind two different verifiers depending on which code path
    /// computed it.
    pub fn for_reveal_token(token: &[u8]) -> Self {
        Self(Sha256::digest(token).into())
    }

    /// Derives the verifier of a describe token. Like the reveal token, the
    /// describe token is an HKDF output with its own `info` label, so a plain
    /// SHA-256 is enough.
    pub fn for_describe_token(token: &[u8]) -> Self {
        Self::for_reveal_token(token)
    }

    /// Parses a verifier a client submitted at creation time.
    pub fn parse(text: &str) -> Result<Self, VerifierError> {
        let decoded = URL_SAFE_NO_PAD.decode(text.as_bytes()).map_err(|_| VerifierError::Encoding)?;

        let bytes: [u8; VERIFIER_BYTES] = decoded.as_slice().try_into().map_err(|_| VerifierError::Length(decoded.len()))?;

        Ok(Self(bytes))
    }

    pub fn from_bytes(bytes: [u8; VERIFIER_BYTES]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; VERIFIER_BYTES] {
        &self.0
    }

    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    /// Constant-time equality.
    ///
    /// Always use this instead of `==` when one side comes from a request.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

/// Prints no bytes. A verifier is not itself a secret, but it is one hash
/// preimage away from one, and it must not appear in logs or error chains.
impl fmt::Debug for Verifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Verifier(redacted)")
    }
}

/// Compares a bearer token against a set of configured hashes in constant time.
///
/// Two details that a naive implementation gets wrong:
///
/// - Every candidate is compared, with no early return on a match, so the time
///   taken does not reveal *which* token matched or how many were configured
///   before it.
/// - An empty candidate set returns `false` rather than allowing everything.
#[must_use]
pub fn any_matches(candidate: &Verifier, allowed: &[Verifier]) -> bool {
    let mut found = subtle::Choice::from(0u8);
    for entry in allowed {
        found |= candidate.0.ct_eq(&entry.0);
    }
    found.into()
}

/// The position of the first entry in `allowed` that equals `candidate`, in
/// constant time.
///
/// Like [`any_matches`], every entry is compared and none is skipped: neither
/// the time taken nor the memory accessed depends on where (or whether) the
/// candidate matched. The position is accumulated with
/// [`subtle::ConditionallySelectable`], so a match does not branch. Only the
/// final answer is turned into an `Option`; it is what the caller acts on
/// anyway. An empty set matches nothing.
#[must_use]
pub fn matching_index(candidate: &Verifier, allowed: &[Verifier]) -> Option<usize> {
    let mut found = subtle::Choice::from(0u8);
    let mut position = 0u64;
    for (index, entry) in allowed.iter().enumerate() {
        let first_hit = candidate.0.ct_eq(&entry.0) & !found;
        position.conditional_assign(&(index as u64), first_hit);
        found |= first_hit;
    }
    Option::<u64>::from(subtle::CtOption::new(position, found)).and_then(|position| usize::try_from(position).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_round_trips_through_base64url() {
        let verifier = Verifier::for_delete_secret(b"secret");
        let parsed = Verifier::parse(&verifier.encode()).expect("valid verifier");
        assert!(verifier.matches(&parsed));
    }

    #[test]
    fn rejects_wrong_length_and_bad_encoding() {
        // Decodes cleanly but to the wrong number of bytes.
        assert_eq!(Verifier::parse("AAAA"), Err(VerifierError::Length(3)));
        // Not valid base64url at all.
        assert_eq!(Verifier::parse("not base64!!"), Err(VerifierError::Encoding));
        // Correct alphabet, but a length base64 cannot produce.
        assert_eq!(Verifier::parse("short"), Err(VerifierError::Encoding));
    }

    #[test]
    fn different_secrets_give_different_verifiers() {
        let a = Verifier::for_delete_secret(b"one");
        let b = Verifier::for_delete_secret(b"two");
        assert!(!a.matches(&b));
    }

    /// The delete verifier is domain-separated, the reveal verifier is not.
    /// Feeding identical bytes to both must therefore not collide - otherwise a
    /// reveal token could be replayed as a delete secret.
    #[test]
    fn all_capability_domains_are_separated() {
        let same = b"identical bytes";
        let derived = [
            Verifier::for_delete_secret(same),
            Verifier::for_reveal_token(same),
            Verifier::for_creation_token(same),
        ];

        for (i, a) in derived.iter().enumerate() {
            for b in derived.iter().skip(i + 1) {
                assert!(!a.matches(b), "two capability kinds produced the same verifier for identical bytes");
            }
        }
    }

    #[test]
    fn any_matches_finds_a_configured_token() {
        let allowed = [Verifier::for_creation_token(b"a"), Verifier::for_creation_token(b"b")];

        assert!(any_matches(&Verifier::for_creation_token(b"a"), &allowed));
        assert!(any_matches(&Verifier::for_creation_token(b"b"), &allowed));
        assert!(!any_matches(&Verifier::for_creation_token(b"c"), &allowed));
        // A delete secret with identical bytes must not authorise creation.
        assert!(!any_matches(&Verifier::for_delete_secret(b"a"), &allowed));
    }

    /// An empty allowlist must deny. Getting this backwards would turn a
    /// misconfigured token mode into an open server.
    #[test]
    fn empty_allowlist_denies() {
        assert!(!any_matches(&Verifier::for_creation_token(b"a"), &[]));
    }

    fn tokens(names: &[&str]) -> Vec<Verifier> {
        names.iter().map(|name| Verifier::for_creation_token(name.as_bytes())).collect()
    }

    #[test]
    fn matching_index_finds_no_match() {
        let allowed = tokens(&["a", "b", "c"]);
        assert_eq!(matching_index(&Verifier::for_creation_token(b"d"), &allowed), None);
        // Identical bytes in another capability domain are no match either.
        assert_eq!(matching_index(&Verifier::for_delete_secret(b"a"), &allowed), None);
    }

    #[test]
    fn matching_index_finds_the_first_middle_and_last_entry() {
        let allowed = tokens(&["a", "b", "c", "d", "e"]);
        assert_eq!(matching_index(&Verifier::for_creation_token(b"a"), &allowed), Some(0));
        assert_eq!(matching_index(&Verifier::for_creation_token(b"c"), &allowed), Some(2));
        assert_eq!(matching_index(&Verifier::for_creation_token(b"e"), &allowed), Some(4));
    }

    #[test]
    fn matching_index_reports_the_first_of_equal_entries() {
        let allowed = tokens(&["a", "b", "a"]);
        assert_eq!(matching_index(&Verifier::for_creation_token(b"a"), &allowed), Some(0));
    }

    /// Like [`empty_allowlist_denies`]: no candidates means no match.
    #[test]
    fn matching_index_of_an_empty_set_is_none() {
        assert_eq!(matching_index(&Verifier::for_creation_token(b"a"), &[]), None);
    }

    #[test]
    fn debug_reveals_nothing() {
        let verifier = Verifier::for_delete_secret(b"secret");
        assert_eq!(format!("{verifier:?}"), "Verifier(redacted)");
    }
}
