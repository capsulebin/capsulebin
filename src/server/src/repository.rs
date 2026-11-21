//! Persistence. All SQL lives below this module and nowhere else.
//!
//! The traits exist so the HTTP layer depends on behaviour rather than on SQLite,
//! and so a handler test does not need a database file.

pub mod sqlite;

use crate::domain::abuse::{ModerationAction, ModerationRecord, NewReport, ReasonCode, Report, ReportStatus, Resolution};

use crate::domain::PasteId;

use crate::domain::{NewPaste, Paste, PurgeOutcome, PurgePolicy};

/// A burned paste's public header and ciphertext, handed out exactly once.
type BurnedPayload = (Vec<u8>, Vec<u8>);

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    /// The id is already taken. Surfaced separately because the client generates
    /// ids, so a collision is its problem to retry — not a server fault.
    #[error("paste id already exists")]
    IdConflict,
    #[error("database error: {0}")]
    Database(#[source] sqlx::Error),
}

/// Storage and lifecycle operations on pastes.
pub trait PasteRepository: Send + Sync {
    /// Inserts a paste, failing with [`RepositoryError::IdConflict`] if the id is
    /// taken.
    fn create(&self, paste: &NewPaste) -> impl Future<Output = Result<Paste, RepositoryError>> + Send;

    /// Loads a paste regardless of its availability.
    ///
    /// Returns the row even when expired, disabled or deleted: deciding what a
    /// caller may see is the domain's job, and a repository that hid rows would
    /// make "expired" and "never existed" indistinguishable to the domain too.
    fn find(&self, id: &PasteId) -> impl Future<Output = Result<Option<Paste>, RepositoryError>> + Send;

    /// Like [`Self::find`], but without reading the payload: `ciphertext` is
    /// `Some(empty)` when a payload is stored and `None` when it is gone.
    ///
    /// For every request that has not proven a capability yet (descriptor,
    /// retrieve, delete): reading up to the full plaintext limit from disk for
    /// an unauthorized request would be free I/O amplification.
    fn find_metadata(&self, id: &PasteId) -> impl Future<Output = Result<Option<Paste>, RepositoryError>> + Send;

    /// The payload of a paste that is retrievable at `now`, read after the
    /// caller has verified the reveal capability. `None` if it became
    /// unavailable in the meantime.
    fn load_ciphertext(&self, id: &PasteId, now: i64) -> impl Future<Output = Result<Option<Vec<u8>>, RepositoryError>> + Send;

    /// Records that an authorized retrieval happened by advancing
    /// `last_retrieved_at` - the only retrieval metadata the server keeps.
    ///
    /// The timestamp only moves once per `granularity_secs`; inside the window
    /// this is a no-op. A coarse value is all the inactivity purge needs, and it
    /// keeps retrievals from becoming one write each.
    ///
    /// Call this only after the reveal capability has been accepted and the
    /// ciphertext was handed out. A page view, a rejected token or an unknown id
    /// must leave the timestamp untouched.
    fn track_retrieval(&self, id: &PasteId, now: i64, granularity_secs: i64) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    /// Atomically reads and consumes a burn-after-retrieval paste.
    ///
    /// In one `BEGIN IMMEDIATE` transaction: reads `public_header` and
    /// `ciphertext`, then updates the row with a `WHERE consumed_at IS NULL`
    /// guard, checking the update's affected-row count. `BEGIN IMMEDIATE`
    /// serialises concurrent callers on the write lock instead of racing them
    /// on a stale read snapshot; the affected-row check is the actual
    /// at-most-once guarantee and is kept even so, as the cheap belt-and-braces
    /// check.
    ///
    /// Returns `None` if the paste is not active, already consumed, expired at
    /// `now`, or lost the race to another concurrent call. The caller must have
    /// already verified the reveal capability before calling this — the method
    /// only enforces lifecycle state.
    fn retrieve_and_burn(&self, id: &PasteId, now: i64) -> impl Future<Output = Result<Option<BurnedPayload>, RepositoryError>> + Send;

    /// Logically deletes a paste: drops the ciphertext, keeps a tombstone.
    ///
    /// Returns `false` if nothing was there to delete, so the caller can stay
    /// silent about which of the two happened.
    ///
    /// No timestamp is recorded. `moderated_at` means "an operator acted", and a
    /// creator exercising their own delete capability is not moderation; there is
    /// no column for the other case, and the purge rules reach the tombstone
    /// through `created_at` anyway.
    fn delete(&self, id: &PasteId) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Applies the retention policy, removing at most `limit` rows per rule.
    ///
    /// Rows are deleted outright rather than reduced to tombstones: the retention
    /// metadata is part of what has to disappear, and a row kept forever with its
    /// payload nulled would still record when a capsule existed and when it was
    /// last used. A tombstone exists for a *deletion*, which is a different event
    /// with a different lifetime — see [`Self::delete`].
    fn purge(&self, now: i64, policy: &PurgePolicy, limit: i64) -> impl Future<Output = Result<PurgeOutcome, RepositoryError>> + Send;
}

/// What a moderation command did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModerationOutcome {
    /// Applied, and recorded in the audit trail.
    Applied,
    /// No paste with this id (never existed, or purged).
    NotFound,
    /// The paste is in a state the action does not apply to — e.g. restoring an
    /// active paste, or anything on a deleted one. Carries the current state.
    NotApplicable(crate::domain::Availability),
}

/// Reports removed and audit entries removed by a retention run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AbusePurgeOutcome {
    pub reports: u64,
    pub actions: u64,
}

/// A report together with the current state of the paste it names, for the
/// operator's queue. `paste_status` is `None` once the paste row is gone.
#[derive(Debug, Clone)]
pub struct ReportEntry {
    pub report: Report,
    pub paste_status: Option<String>,
}

/// The ceilings on open reports (`open` or `reviewing`):
/// `abuse.max_open_reports_per_paste` and `abuse.max_open_reports`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReportCeilings {
    pub per_paste: i64,
    pub total: i64,
}

/// What [`AbuseRepository::submit_report`] did with a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    Stored,
    /// The paste does not exist (any more), or cannot take reports.
    NoSuchPaste,
    /// The paste exists, but it or the instance already has as many open
    /// reports as a ceiling allows.
    CeilingReached,
}

/// Abuse reports and moderation (ADR-013).
pub trait AbuseRepository: Send + Sync {
    /// Stores a report — but only if the paste it names currently exists
    /// (active and not expired or consumed, or disabled) and neither it nor the
    /// instance has reached its ceiling of open reports.
    ///
    /// The caller answers the reporter identically for every outcome, so the
    /// endpoint is no oracle for paste ids. The existence check, both counts and
    /// the insert are one statement: SQLite serializes writers, so the ceilings
    /// hold under concurrency. Only when nothing was stored does a second read
    /// tell "no such paste" from "ceiling reached"; it races with concurrent
    /// writes, so it may only inform a log line, never the answer.
    fn submit_report(
        &self,
        report: &NewReport,
        ceilings: ReportCeilings,
        now: i64,
    ) -> impl Future<Output = Result<SubmitOutcome, RepositoryError>> + Send;

    /// The number of open reports (`open` or `reviewing`) on the instance.
    fn count_open_reports(&self) -> impl Future<Output = Result<i64, RepositoryError>> + Send;

    /// Reports, oldest first, optionally filtered by status.
    fn list_reports(
        &self,
        status: Option<ReportStatus>,
        limit: i64,
    ) -> impl Future<Output = Result<Vec<ReportEntry>, RepositoryError>> + Send;

    fn find_report(&self, id: &str) -> impl Future<Output = Result<Option<ReportEntry>, RepositoryError>> + Send;

    /// Moves a report to `reviewing`. Returns `false` if it does not exist or is
    /// not open.
    fn review_report(&self, id: &str) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Closes a report. Returns `false` if it does not exist or is already closed.
    fn close_report(&self, id: &str, resolution: Resolution, now: i64) -> impl Future<Output = Result<bool, RepositoryError>> + Send;

    /// Disables, deletes or restores a paste and records the action, in one
    /// transaction.
    fn moderate(
        &self,
        paste_id: &PasteId,
        action: ModerationAction,
        reason: Option<ReasonCode>,
        note: Option<&str>,
        now: i64,
    ) -> impl Future<Output = Result<ModerationOutcome, RepositoryError>> + Send;

    /// The audit trail for one paste, oldest first.
    fn moderation_history(&self, paste_id: &PasteId) -> impl Future<Output = Result<Vec<ModerationRecord>, RepositoryError>> + Send;

    /// Applies `abuse.report_retention`: removes closed reports handled at or
    /// before `cutoff` and audit entries recorded at or before it. Open and
    /// reviewing reports are kept until an operator closes them.
    fn purge_abuse(&self, cutoff: i64) -> impl Future<Output = Result<AbusePurgeOutcome, RepositoryError>> + Send;
}
