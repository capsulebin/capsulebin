//! SQLite implementation of the repository traits.

use crate::domain::{PasteId, VERIFIER_BYTES, Verifier};
use sqlx::{Row as _, SqlitePool};

use super::{BurnedPayload, PasteRepository, RepositoryError};
use crate::domain::{Availability, NewPaste, Paste, PurgeOutcome, PurgePolicy};

#[derive(Debug, Clone)]
pub struct SqlitePasteRepository {
    pool: SqlitePool,
    /// How often a query read a ciphertext blob, shared by every clone.
    #[cfg(test)]
    payload_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl SqlitePasteRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            #[cfg(test)]
            payload_reads: std::sync::Arc::default(),
        }
    }

    /// Number of ciphertext blobs read so far (`find`, `load_ciphertext`,
    /// `retrieve_and_burn`), so a test can prove a path never loads a payload.
    #[cfg(test)]
    pub(crate) fn payload_reads(&self) -> usize {
        self.payload_reads.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Counts one ciphertext read for [`Self::payload_reads`]; does nothing
    /// outside tests.
    fn count_payload_read(&self) {
        #[cfg(test)]
        self.payload_reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Turns a database error from [`create`] into a domain error, recognising a
/// primary-key collision so that the caller can report a conflict rather than a
/// fault. Only used for `INSERT` — a unique violation anywhere else is not an
/// id conflict, so every other method maps through
/// [`RepositoryError::Database`] directly.
fn map_create_error(error: sqlx::Error) -> RepositoryError {
    if let sqlx::Error::Database(ref db) = error
        && db.is_unique_violation()
    {
        return RepositoryError::IdConflict;
    }
    RepositoryError::Database(error)
}

/// Marks a query that this file assembles itself as safe to execute.
///
/// sqlx requires the assertion for every statement that is not a `&'static str`,
/// so that assembled SQL cannot reach the database unexamined. The audit it asks
/// for is one sentence long here: the only thing interpolated below is
/// `SELECT_COLUMNS`, a `const` column list. Every value from a request travels
/// as a bound parameter.
fn assembled(sql: String) -> sqlx::AssertSqlSafe<String> {
    sqlx::AssertSqlSafe(sql)
}

/// Rebuilds a verifier from a stored blob.
///
/// A wrong length here means the row was written by something that is not this
/// code. Treating it as a database error is deliberate: silently padding or
/// truncating would produce a verifier that matches nothing, which would look
/// like a client mistake instead of corruption.
fn verifier_from_blob(blob: Vec<u8>, column: &str) -> Result<Verifier, RepositoryError> {
    let bytes: [u8; VERIFIER_BYTES] = blob.as_slice().try_into().map_err(|_| {
        RepositoryError::Database(sqlx::Error::Decode(
            format!("{column} has {} bytes, expected {VERIFIER_BYTES}", blob.len()).into(),
        ))
    })?;
    Ok(Verifier::from_bytes(bytes))
}

/// Decodes a boolean stored as the integer 0 or 1.
///
/// Anything else is treated as data corruption, consistent with how this file
/// handles wrong verifier lengths and unknown availability statuses.
fn bool_from_integer(value: i64, column: &str) -> Result<bool, RepositoryError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(RepositoryError::Database(sqlx::Error::Decode(
            format!("{column} has invalid boolean value {other}").into(),
        ))),
    }
}

fn row_to_paste(row: &sqlx::sqlite::SqliteRow) -> Result<Paste, RepositoryError> {
    let decode = |message: String| RepositoryError::Database(sqlx::Error::Decode(message.into()));

    let id_text: String = row.try_get("id").map_err(RepositoryError::Database)?;
    let id = PasteId::parse(&id_text).map_err(|source| decode(format!("stored id is not in canonical form: {source}")))?;

    let availability_text: String = row.try_get("availability_status").map_err(RepositoryError::Database)?;
    let availability =
        Availability::parse(&availability_text).ok_or_else(|| decode(format!("unknown availability_status {availability_text:?}")))?;

    Ok(Paste {
        id,
        format_version: row.try_get("format_version").map_err(RepositoryError::Database)?,
        public_header: row.try_get("public_header").map_err(RepositoryError::Database)?,
        ciphertext: row.try_get("ciphertext").map_err(RepositoryError::Database)?,
        reveal_verifier: verifier_from_blob(
            row.try_get("reveal_verifier").map_err(RepositoryError::Database)?,
            "reveal_verifier",
        )?,
        delete_verifier: verifier_from_blob(
            row.try_get("delete_verifier").map_err(RepositoryError::Database)?,
            "delete_verifier",
        )?,
        describe_verifier: row
            .try_get::<Option<Vec<u8>>, _>("describe_verifier")
            .map_err(RepositoryError::Database)?
            .map(|blob| verifier_from_blob(blob, "describe_verifier"))
            .transpose()?,
        created_at: row.try_get("created_at").map_err(RepositoryError::Database)?,
        last_retrieved_at: row.try_get("last_retrieved_at").map_err(RepositoryError::Database)?,
        expires_at: row.try_get("expires_at").map_err(RepositoryError::Database)?,
        burn_after_retrieval: bool_from_integer(
            row.try_get("burn_after_retrieval").map_err(RepositoryError::Database)?,
            "burn_after_retrieval",
        )?,
        consumed_at: row.try_get("consumed_at").map_err(RepositoryError::Database)?,
        payload_size: row.try_get("payload_size").map_err(RepositoryError::Database)?,
        availability,
    })
}

const SELECT_COLUMNS: &str = "id, format_version, public_header, ciphertext, \
     reveal_verifier, delete_verifier, describe_verifier, created_at, last_retrieved_at, expires_at, \
     burn_after_retrieval, consumed_at, payload_size, availability_status";

impl PasteRepository for SqlitePasteRepository {
    async fn create(&self, paste: &NewPaste) -> Result<Paste, RepositoryError> {
        // payload_size records the ciphertext length, which is what the server
        // can actually measure. The plaintext size stays unknown to it.
        let payload_size = i64::try_from(paste.ciphertext.len()).unwrap_or(i64::MAX);

        // A retired id (`migrations/0004_retired_ids.sql`) is refused in the
        // same statement as the insert: checked separately, a purge committing in
        // between could retire the id after the check and before the insert. A
        // retired id inserts nothing, an occupied one is a unique violation; both
        // are the same conflict.
        let row = sqlx::query(assembled(format!(
            "INSERT INTO pastes (
                 id, format_version, public_header, ciphertext,
                 reveal_verifier, delete_verifier, describe_verifier, created_at, expires_at,
                 burn_after_retrieval, payload_size
             ) SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11
                WHERE NOT EXISTS (SELECT 1 FROM retired_ids WHERE id = ?1)
             RETURNING {SELECT_COLUMNS}"
        )))
        .bind(paste.id.encode())
        .bind(paste.format_version)
        .bind(paste.public_header.as_slice())
        .bind(paste.ciphertext.as_slice())
        .bind(paste.reveal_verifier.as_bytes().as_slice())
        .bind(paste.delete_verifier.as_bytes().as_slice())
        .bind(paste.describe_verifier.as_bytes().as_slice())
        .bind(paste.created_at)
        .bind(paste.expires_at)
        .bind(i64::from(paste.burn_after_retrieval))
        .bind(payload_size)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_create_error)?
        .ok_or(RepositoryError::IdConflict)?;

        row_to_paste(&row)
    }

    async fn find(&self, id: &PasteId) -> Result<Option<Paste>, RepositoryError> {
        self.count_payload_read();
        let row = sqlx::query(assembled(format!("SELECT {SELECT_COLUMNS} FROM pastes WHERE id = ?1")))
            .bind(id.encode())
            .fetch_optional(&self.pool)
            .await
            .map_err(RepositoryError::Database)?;

        row.as_ref().map(row_to_paste).transpose()
    }

    async fn find_metadata(&self, id: &PasteId) -> Result<Option<Paste>, RepositoryError> {
        // The payload column is replaced by an empty blob that only marks its
        // presence; everything else is the full row.
        let columns = SELECT_COLUMNS.replace("ciphertext,", "CASE WHEN ciphertext IS NULL THEN NULL ELSE X'' END AS ciphertext,");
        let row = sqlx::query(assembled(format!("SELECT {columns} FROM pastes WHERE id = ?1")))
            .bind(id.encode())
            .fetch_optional(&self.pool)
            .await
            .map_err(RepositoryError::Database)?;

        row.as_ref().map(row_to_paste).transpose()
    }

    async fn load_ciphertext(&self, id: &PasteId, now: i64) -> Result<Option<Vec<u8>>, RepositoryError> {
        self.count_payload_read();
        let ciphertext: Option<Option<Vec<u8>>> = sqlx::query_scalar(
            "SELECT ciphertext FROM pastes
              WHERE id = ?1
                AND availability_status = 'active'
                AND consumed_at IS NULL
                AND (expires_at IS NULL OR expires_at > ?2)",
        )
        .bind(id.encode())
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(RepositoryError::Database)?;

        Ok(ciphertext.flatten())
    }

    async fn track_retrieval(&self, id: &PasteId, now: i64, granularity_secs: i64) -> Result<(), RepositoryError> {
        // The window condition is in the `WHERE`, not decided in Rust from an
        // earlier read: two concurrent retrievals could each see a stale value,
        // and the coarse timestamp would then depend on interleaving rather than
        // on the clock. Callers skip this call entirely while the stored value is
        // still inside the window (`Paste::needs_retrieval_timestamp`), so most
        // retrievals never take the write lock; this guard covers the race.
        sqlx::query(
            "UPDATE pastes
                SET last_retrieved_at = ?2
              WHERE id = ?1
                AND availability_status = 'active'
                AND (last_retrieved_at IS NULL OR last_retrieved_at <= ?2 - ?3)",
        )
        .bind(id.encode())
        .bind(now)
        .bind(granularity_secs)
        .execute(&self.pool)
        .await
        .map_err(RepositoryError::Database)?;

        Ok(())
    }

    async fn retrieve_and_burn(&self, id: &PasteId, now: i64) -> Result<Option<BurnedPayload>, RepositoryError> {
        self.count_payload_read();
        // `BEGIN IMMEDIATE`: a deferred `BEGIN` takes its read snapshot at the
        // first statement and only asks for the write lock at the UPDATE. In
        // WAL mode, if another connection commits a conflicting write in
        // between, that UPDATE fails with `SQLITE_BUSY_SNAPSHOT` - a busy
        // variant `busy_timeout` does not retry, since retrying the same
        // statement on a stale snapshot cannot help. Taking the write lock
        // upfront serialises concurrent callers instead of racing them, so the
        // loser waits (governed by `busy_timeout`) and then simply observes
        // `consumed_at IS NOT NULL`. The `rows_affected()` check below stays as
        // the actual source of truth either way.
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(RepositoryError::Database)?;

        let row = sqlx::query(
            "SELECT public_header, ciphertext
               FROM pastes
              WHERE id = ?1
                AND availability_status = 'active'
                AND consumed_at IS NULL
                AND (expires_at IS NULL OR expires_at > ?2)",
        )
        .bind(id.encode())
        .bind(now)
        .fetch_optional(&mut *tx)
        .await
        .map_err(RepositoryError::Database)?;

        let Some(row) = row else {
            tx.commit().await.map_err(RepositoryError::Database)?;
            return Ok(None);
        };

        let public_header: Vec<u8> = row.try_get("public_header").map_err(RepositoryError::Database)?;
        let ciphertext: Vec<u8> = row.try_get("ciphertext").map_err(RepositoryError::Database)?;

        let affected = sqlx::query(
            "UPDATE pastes
                SET consumed_at = ?2,
                    last_retrieved_at = ?2,
                    ciphertext = NULL
              WHERE id = ?1
                AND availability_status = 'active'
                AND consumed_at IS NULL",
        )
        .bind(id.encode())
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(RepositoryError::Database)?
        .rows_affected();

        tx.commit().await.map_err(RepositoryError::Database)?;

        if affected == 1 {
            Ok(Some((public_header, ciphertext)))
        } else {
            Ok(None)
        }
    }

    async fn delete(&self, id: &PasteId) -> Result<bool, RepositoryError> {
        // The row survives as a tombstone; only the payload goes. Keeping the id
        // is what lets a burn paste answer "already retrieved" without letting
        // anyone enumerate which ids ever existed - the tombstone is only ever
        // reachable by someone who already holds the id.
        let result = sqlx::query(
            "UPDATE pastes
                SET ciphertext = NULL,
                    availability_status = 'deleted'
              WHERE id = ?1 AND availability_status != 'deleted'",
        )
        .bind(id.encode())
        .execute(&self.pool)
        .await
        .map_err(RepositoryError::Database)?;

        Ok(result.rows_affected() > 0)
    }

    async fn purge(&self, now: i64, policy: &PurgePolicy, limit: i64) -> Result<PurgeOutcome, RepositoryError> {
        // Cutoffs are computed here rather than in SQL so that a small `now` - which
        // every test has - cannot wrap around into the future.
        let never_retrieved_before = now.saturating_sub(policy.never_retrieved_after);
        let inactive_before = now.saturating_sub(policy.inactive_after);

        // One transaction for the whole run: a partially applied batch that still
        // reported success would leave the caller believing it had converged.
        let mut tx = self.pool.begin().await.map_err(RepositoryError::Database)?;

        // `DELETE ... LIMIT` needs a SQLite compile-time option that cannot be
        // assumed, so each rule selects its ids first. An `ORDER BY` makes the
        // choice deterministic (oldest first, tie-broken by id) so that repeated
        // runs produce reproducible results and tests stay stable.
        //
        // Every deleted row - live capsule or tombstone - leaves its id in
        // `retired_ids`, so `create` never hands it out again. No code here does
        // that: the `AFTER DELETE` trigger `retire_deleted_paste_id` in
        // `migrations/0004_retired_ids.sql` inserts it, in the same transaction.

        const DELETE_EXPIRED: &str = "\
            DELETE FROM pastes \
              WHERE id IN ( \
                SELECT id FROM pastes \
                 WHERE expires_at IS NOT NULL AND expires_at <= ?1 \
                 ORDER BY expires_at, id \
                 LIMIT ?2 \
              )";

        const DELETE_NEVER_RETRIEVED: &str = "\
            DELETE FROM pastes \
              WHERE id IN ( \
                SELECT id FROM pastes \
                 WHERE last_retrieved_at IS NULL AND created_at <= ?1 \
                 ORDER BY created_at, id \
                 LIMIT ?2 \
              )";

        const DELETE_INACTIVE: &str = "\
            DELETE FROM pastes \
              WHERE id IN ( \
                SELECT id FROM pastes \
                 WHERE last_retrieved_at IS NOT NULL AND last_retrieved_at <= ?1 \
                 ORDER BY last_retrieved_at, id \
                 LIMIT ?2 \
              )";

        let expired = sqlx::query(DELETE_EXPIRED)
            .bind(now)
            .bind(limit)
            .execute(&mut *tx)
            .await
            .map_err(RepositoryError::Database)?
            .rows_affected();

        let never_retrieved = sqlx::query(DELETE_NEVER_RETRIEVED)
            .bind(never_retrieved_before)
            .bind(limit)
            .execute(&mut *tx)
            .await
            .map_err(RepositoryError::Database)?
            .rows_affected();

        let inactive = sqlx::query(DELETE_INACTIVE)
            .bind(inactive_before)
            .bind(limit)
            .execute(&mut *tx)
            .await
            .map_err(RepositoryError::Database)?
            .rows_affected();

        tx.commit().await.map_err(RepositoryError::Database)?;

        Ok(PurgeOutcome {
            expired,
            never_retrieved,
            inactive,
        })
    }
}

pub mod abuse;

#[cfg(test)]
mod tests;
