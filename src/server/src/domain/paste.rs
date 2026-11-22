//! The paste as the domain sees it.
//!
//! The server treats `public_header` and `ciphertext` as opaque bytes. It knows
//! how large they are, when they expire and who may fetch them — never what they
//! contain.

use subtle::ConstantTimeEq as _;

use super::{PasteId, Verifier};

/// Availability as stored in the database.
///
/// Deliberately separate from "is this retrievable right now": expiry is a
/// function of the clock, not a stored state, so a paste can be `Active` and
/// still unavailable. Materialising expiry into this enum would require a
/// background job to be correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Active,
    /// Blocked by moderation. The payload is still present, so the decision can
    /// be reversed.
    Disabled,
    /// Payload removed. Only a tombstone remains.
    Deleted,
}

impl Availability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "active" => Some(Self::Active),
            "disabled" => Some(Self::Disabled),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

/// What a creation request turns into after validation.
#[derive(Debug, Clone)]
pub struct NewPaste {
    pub id: PasteId,
    pub format_version: i64,
    pub public_header: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub reveal_verifier: Verifier,
    pub delete_verifier: Verifier,
    pub describe_verifier: Verifier,
    /// Taken from the [`Clock`](super::Clock) rather than from SQLite's
    /// `unixepoch()`, so that a test can place a paste at any point in time.
    pub created_at: i64,
    /// `None` means no expiry, which the configuration may forbid.
    pub expires_at: Option<i64>,
    pub burn_after_retrieval: bool,
}

/// A stored paste.
#[derive(Debug, Clone)]
pub struct Paste {
    pub id: PasteId,
    pub format_version: i64,
    pub public_header: Vec<u8>,
    /// `None` once the payload has been removed — after a burn or a delete.
    pub ciphertext: Option<Vec<u8>>,
    pub reveal_verifier: Verifier,
    pub delete_verifier: Verifier,
    /// `None` for rows written before the describe capability existed: they
    /// have no descriptor anybody can read.
    pub describe_verifier: Option<Verifier>,
    pub created_at: i64,
    /// Most recent authorized ciphertext handout, or `None` if there has not been
    /// one. Coarse by design and never exposed: the purge policy is its only
    /// consumer. See [`crate::repository::PasteRepository::track_retrieval`].
    pub last_retrieved_at: Option<i64>,
    pub expires_at: Option<i64>,
    pub burn_after_retrieval: bool,
    pub consumed_at: Option<i64>,
    pub payload_size: i64,
    pub availability: Availability,
}

impl Paste {
    /// Whether the paste is past its expiry at `now`.
    ///
    /// The comparison is `>=` so that a paste is gone in the same second its
    /// expiry names, rather than lingering for one second past it.
    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|expiry| now >= expiry)
    }

    /// Whether a burn paste has already been handed out.
    pub fn is_consumed(&self) -> bool {
        self.consumed_at.is_some()
    }

    /// Whether the ciphertext may still be served at `now`.
    ///
    /// Every reason for "no" is collapsed here on purpose. The HTTP layer turns a
    /// `false` into one generic response, so that an observer cannot distinguish
    /// "expired" from "disabled" from "never existed".
    pub fn is_retrievable(&self, now: i64) -> bool {
        self.availability == Availability::Active && !self.is_expired(now) && !self.is_consumed() && self.ciphertext.is_some()
    }

    /// Whether a retrieval at `now` has to move `last_retrieved_at`: only once
    /// a full `granularity_secs` window has passed since the stored value.
    ///
    /// Inside the window a retrieval is a pure read and never takes SQLite's
    /// single write lock.
    pub fn needs_retrieval_timestamp(&self, now: i64, granularity_secs: i64) -> bool {
        self.last_retrieved_at
            .is_none_or(|last| last <= now.saturating_sub(granularity_secs))
    }

    /// Whether `presented` is this paste's describe capability, compared in
    /// constant time.
    ///
    /// A row without a describe verifier (written before
    /// `migrations/0002_describe_verifier.sql`) has no describe capability:
    /// the comparison still runs, against a dummy, so it costs the same work as
    /// for any other row, but it never matches - also not for a caller who
    /// presents the dummy's preimage.
    #[must_use]
    pub fn describe_matches(&self, presented: &Verifier) -> bool {
        let stored = self.describe_verifier.unwrap_or_else(|| Verifier::for_describe_token(b"absent"));
        presented.matches(&stored) & self.describe_verifier.is_some()
    }

    /// Whether `request` can still be a re-submission of the create request
    /// that produced this paste — a client retrying after a timeout or a
    /// dropped connection, not knowing whether its first attempt landed.
    ///
    /// The client-generated id doubles as the idempotency key (ADR-011). A
    /// replay is answered with the original result instead of `id_conflict`,
    /// but only when the request proves it comes from the creator: all three
    /// verifiers, the public header and the ciphertext must be identical, and
    /// the lifecycle options must resolve to the same paste. Anything else
    /// stays a conflict, so a replay cannot be used to probe, extend or revive
    /// a paste:
    ///
    /// - the paste must still be retrievable at `now` (not expired, consumed,
    ///   disabled or deleted);
    /// - the requested lifetime (`expires_at - created_at`) must be the
    ///   original one, so a retry does not silently move the expiry.
    ///
    /// This decides everything but the ciphertext bytes, from the metadata
    /// alone (a row from `find_metadata` will do): the stored `payload_size`
    /// stands in for the ciphertext's length. Only when it answers `true` is
    /// the stored ciphertext worth loading and handing to
    /// [`NewPaste::carries_ciphertext`]; a request from someone who knows only
    /// the id never gets that far, so it cannot make the server read a payload.
    ///
    /// The byte comparisons run in constant time and every comparison is
    /// evaluated, so the time taken does not reveal which field differed.
    #[must_use]
    pub fn may_be_replayed_by(&self, request: &NewPaste, now: i64) -> bool {
        let mut same = subtle::Choice::from(u8::from(self.reveal_verifier.matches(&request.reveal_verifier)));
        same &= subtle::Choice::from(u8::from(self.delete_verifier.matches(&request.delete_verifier)));
        same &= subtle::Choice::from(u8::from(self.describe_matches(&request.describe_verifier)));
        same &= self.public_header.as_slice().ct_eq(request.public_header.as_slice());

        let lifetime = |expires_at: Option<i64>, created_at: i64| expires_at.map(|at| at.saturating_sub(created_at));
        let same_size = usize::try_from(self.payload_size).is_ok_and(|size| size == request.ciphertext.len());
        let same_lifecycle = (self.format_version == request.format_version)
            & (self.burn_after_retrieval == request.burn_after_retrieval)
            & (lifetime(self.expires_at, self.created_at) == lifetime(request.expires_at, request.created_at));

        bool::from(same) & same_size & same_lifecycle & self.is_retrievable(now)
    }
}

impl NewPaste {
    /// Whether this request carries exactly `stored`, the ciphertext of the
    /// paste it collided with. The last step of a replay check, after
    /// [`Paste::may_be_replayed_by`]; compared in constant time.
    #[must_use]
    pub fn carries_ciphertext(&self, stored: &[u8]) -> bool {
        bool::from(stored.ct_eq(self.ciphertext.as_slice()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paste() -> Paste {
        Paste {
            id: PasteId::from_bytes([7; 16]),
            format_version: 1,
            public_header: vec![1, 2, 3],
            ciphertext: Some(vec![4, 5, 6]),
            reveal_verifier: Verifier::for_reveal_token(b"reveal"),
            delete_verifier: Verifier::for_delete_secret(b"delete"),
            describe_verifier: Some(Verifier::for_describe_token(b"describe")),
            created_at: 1_000,
            last_retrieved_at: None,
            expires_at: Some(2_000),
            burn_after_retrieval: false,
            consumed_at: None,
            payload_size: 3,
            availability: Availability::Active,
        }
    }

    fn replay_of(paste: &Paste, now: i64) -> NewPaste {
        NewPaste {
            id: paste.id,
            format_version: paste.format_version,
            public_header: paste.public_header.clone(),
            ciphertext: paste.ciphertext.clone().unwrap_or_default(),
            reveal_verifier: paste.reveal_verifier,
            delete_verifier: paste.delete_verifier,
            describe_verifier: paste.describe_verifier.unwrap_or_else(|| Verifier::for_describe_token(b"x")),
            created_at: now,
            expires_at: paste.expires_at.map(|at| at - paste.created_at + now),
            burn_after_retrieval: paste.burn_after_retrieval,
        }
    }

    #[test]
    fn the_retrieval_timestamp_moves_once_per_window() {
        let mut stored = paste();
        assert!(stored.needs_retrieval_timestamp(1_500, 3_600), "never retrieved");
        stored.last_retrieved_at = Some(10_000);
        assert!(!stored.needs_retrieval_timestamp(10_000, 3_600));
        assert!(!stored.needs_retrieval_timestamp(13_599, 3_600));
        assert!(stored.needs_retrieval_timestamp(13_600, 3_600));
        // A clock that went backwards does not write either.
        assert!(!stored.needs_retrieval_timestamp(9_000, 3_600));
    }

    /// The row `find_metadata` returns: the payload replaced by an empty
    /// marker, its length only in `payload_size`.
    fn metadata(paste: &Paste) -> Paste {
        Paste {
            ciphertext: paste.ciphertext.as_ref().map(|_| Vec::new()),
            ..paste.clone()
        }
    }

    #[test]
    fn an_identical_request_is_a_replay() {
        let stored = paste();
        let request = replay_of(&stored, 1_500);
        assert!(metadata(&stored).may_be_replayed_by(&request, 1_500));
        assert!(request.carries_ciphertext(stored.ciphertext.as_deref().unwrap_or_default()));
    }

    #[test]
    fn any_difference_in_the_metadata_makes_it_a_conflict() {
        let stored = metadata(&paste());
        type Mutation = (&'static str, fn(&mut NewPaste));
        let mutations: [Mutation; 10] = [
            ("reveal verifier", |r| r.reveal_verifier = Verifier::for_reveal_token(b"other")),
            ("delete verifier", |r| r.delete_verifier = Verifier::for_delete_secret(b"other")),
            ("describe verifier", |r| {
                r.describe_verifier = Verifier::for_describe_token(b"other")
            }),
            ("public header", |r| r.public_header.push(0)),
            ("public header byte", |r| r.public_header[0] ^= 1),
            ("ciphertext length", |r| r.ciphertext.push(0)),
            ("burn flag", |r| r.burn_after_retrieval = true),
            ("lifetime", |r| r.expires_at = r.expires_at.map(|at| at + 1)),
            ("permanence", |r| r.expires_at = None),
            ("format version", |r| r.format_version = 2),
        ];
        for (what, mutate) in mutations {
            let mut request = replay_of(&paste(), 1_500);
            mutate(&mut request);
            assert!(
                !stored.may_be_replayed_by(&request, 1_500),
                "a different {what} was accepted as a possible replay"
            );
        }
    }

    /// Same length, other bytes: only the ciphertext step can tell, and it does.
    #[test]
    fn a_different_ciphertext_of_the_same_length_is_a_conflict() {
        let stored = paste();
        let mut request = replay_of(&stored, 1_500);
        request.ciphertext[0] ^= 1;

        assert!(metadata(&stored).may_be_replayed_by(&request, 1_500));
        assert!(!request.carries_ciphertext(stored.ciphertext.as_deref().unwrap_or_default()));
        assert!(!request.carries_ciphertext(&[]));
    }

    #[test]
    fn a_paste_that_is_no_longer_retrievable_cannot_be_replayed() {
        let request = replay_of(&paste(), 1_500);

        let expired = metadata(&paste());
        assert!(!expired.may_be_replayed_by(&request, 2_000));

        let mut consumed = paste();
        consumed.consumed_at = Some(1_200);
        consumed.ciphertext = None;
        assert!(!metadata(&consumed).may_be_replayed_by(&request, 1_500));

        let mut disabled = metadata(&paste());
        disabled.availability = Availability::Disabled;
        assert!(!disabled.may_be_replayed_by(&request, 1_500));

        let mut deleted = paste();
        deleted.availability = Availability::Deleted;
        deleted.ciphertext = None;
        assert!(!metadata(&deleted).may_be_replayed_by(&request, 1_500));
    }

    #[test]
    fn availability_round_trips() {
        for value in [Availability::Active, Availability::Disabled, Availability::Deleted] {
            assert_eq!(Availability::parse(value.as_str()), Some(value));
        }
        assert_eq!(Availability::parse("nonsense"), None);
    }

    /// The boundary second is the one that gets implemented wrong. A paste that
    /// expires at 2000 must be gone at 2000, not at 2001.
    #[test]
    fn expiry_boundary_is_inclusive() {
        let paste = paste();

        assert!(!paste.is_expired(1_999));
        assert!(paste.is_expired(2_000));
        assert!(paste.is_expired(2_001));
    }

    #[test]
    fn a_paste_without_expiry_never_expires() {
        let mut paste = paste();
        paste.expires_at = None;

        assert!(!paste.is_expired(i64::MAX));
        assert!(paste.is_retrievable(i64::MAX));
    }

    /// Each condition alone must make the paste unavailable. This is the single
    /// most important behaviour in the module, because the HTTP layer relies on
    /// one boolean to keep its responses indistinguishable.
    #[test]
    fn every_condition_blocks_retrieval_on_its_own() {
        assert!(paste().is_retrievable(1_500));

        let mut expired = paste();
        expired.expires_at = Some(1_400);
        assert!(!expired.is_retrievable(1_500));

        let mut disabled = paste();
        disabled.availability = Availability::Disabled;
        assert!(!disabled.is_retrievable(1_500));

        let mut deleted = paste();
        deleted.availability = Availability::Deleted;
        assert!(!deleted.is_retrievable(1_500));

        let mut consumed = paste();
        consumed.consumed_at = Some(1_450);
        assert!(!consumed.is_retrievable(1_500));

        let mut payloadless = paste();
        payloadless.ciphertext = None;
        assert!(!payloadless.is_retrievable(1_500));
    }
}
