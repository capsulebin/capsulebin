//! SQLite implementation of [`AbuseRepository`].

use sqlx::{Row as _, SqlitePool};

use crate::domain::abuse::{ModerationAction, ModerationRecord, NewReport, ReasonCode, Report, ReportStatus, Resolution};
use crate::domain::{Availability, PasteId};
use crate::repository::{AbusePurgeOutcome, AbuseRepository, ModerationOutcome, ReportEntry, RepositoryError};

#[derive(Debug, Clone)]
pub struct SqliteAbuseRepository {
    pool: SqlitePool,
}

impl SqliteAbuseRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn db(error: sqlx::Error) -> RepositoryError {
    RepositoryError::Database(error)
}

/// The report columns plus the named paste's current status (NULL once purged).
const REPORT_SELECT: &str = "SELECT r.id, r.paste_id, r.reason_code, r.description, r.reporter_contact, \
            r.created_at, r.status, r.handled_at, r.resolution, p.availability_status AS paste_status \
       FROM abuse_reports r LEFT JOIN pastes p ON p.id = r.paste_id";

fn row_to_entry(row: &sqlx::sqlite::SqliteRow) -> Result<ReportEntry, RepositoryError> {
    Ok(ReportEntry {
        report: Report {
            id: row.try_get("id").map_err(db)?,
            paste_id: row.try_get("paste_id").map_err(db)?,
            reason: row.try_get("reason_code").map_err(db)?,
            description: row.try_get("description").map_err(db)?,
            contact: row.try_get("reporter_contact").map_err(db)?,
            created_at: row.try_get("created_at").map_err(db)?,
            status: row.try_get("status").map_err(db)?,
            handled_at: row.try_get("handled_at").map_err(db)?,
            resolution: row.try_get("resolution").map_err(db)?,
        },
        paste_status: row.try_get("paste_status").map_err(db)?,
    })
}

impl AbuseRepository for SqliteAbuseRepository {
    async fn submit_report(&self, report: &NewReport, now: i64) -> Result<bool, RepositoryError> {
        // "Exists" means: something an operator could still act on. A disabled
        // paste counts (it may need deleting); an expired, consumed or deleted one
        // does not — there is nothing left to report.
        let result = sqlx::query(
            "INSERT INTO abuse_reports (id, paste_id, reason_code, description, reporter_contact, created_at)
             SELECT ?1, ?2, ?3, ?4, ?5, ?6
              WHERE EXISTS (
                SELECT 1 FROM pastes
                 WHERE id = ?2
                   AND ciphertext IS NOT NULL
                   AND (availability_status = 'disabled'
                        OR (availability_status = 'active'
                            AND consumed_at IS NULL
                            AND (expires_at IS NULL OR expires_at > ?7)))
              )",
        )
        .bind(&report.id)
        .bind(report.paste_id.encode())
        .bind(report.reason.as_str())
        .bind(&report.description)
        .bind(report.contact.as_deref())
        .bind(report.created_at)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db)?;

        Ok(result.rows_affected() == 1)
    }

    async fn list_reports(&self, status: Option<ReportStatus>, limit: i64) -> Result<Vec<ReportEntry>, RepositoryError> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{REPORT_SELECT} WHERE (?1 IS NULL OR r.status = ?1) ORDER BY r.created_at, r.id LIMIT ?2"
        )))
        .bind(status.map(ReportStatus::as_str))
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        rows.iter().map(row_to_entry).collect()
    }

    async fn find_report(&self, id: &str) -> Result<Option<ReportEntry>, RepositoryError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("{REPORT_SELECT} WHERE r.id = ?1")))
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db)?;

        row.as_ref().map(row_to_entry).transpose()
    }

    async fn review_report(&self, id: &str) -> Result<bool, RepositoryError> {
        let result = sqlx::query("UPDATE abuse_reports SET status = 'reviewing' WHERE id = ?1 AND status = 'open'")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        Ok(result.rows_affected() == 1)
    }

    async fn close_report(&self, id: &str, resolution: Resolution, now: i64) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "UPDATE abuse_reports SET status = 'closed', handled_at = ?2, resolution = ?3
              WHERE id = ?1 AND status != 'closed'",
        )
        .bind(id)
        .bind(now)
        .bind(resolution.as_str())
        .execute(&self.pool)
        .await
        .map_err(db)?;
        Ok(result.rows_affected() == 1)
    }

    async fn moderate(
        &self,
        paste_id: &PasteId,
        action: ModerationAction,
        reason: Option<ReasonCode>,
        note: Option<&str>,
        now: i64,
    ) -> Result<ModerationOutcome, RepositoryError> {
        let id = paste_id.encode();
        let reason = reason.map(ReasonCode::as_str);

        // `BEGIN IMMEDIATE` takes the write lock before the status is read, so the
        // check and the update cannot interleave with a concurrent burn, delete or
        // second moderation command (same reasoning as `retrieve_and_burn`).
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await.map_err(db)?;

        let current: Option<String> = sqlx::query_scalar("SELECT availability_status FROM pastes WHERE id = ?1")
            .bind(&id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
        let Some(current) = current else {
            return Ok(ModerationOutcome::NotFound);
        };
        let current = Availability::parse(&current)
            .ok_or_else(|| db(sqlx::Error::Decode(format!("unknown availability_status {current:?}").into())))?;

        let statement = match (action, current) {
            (ModerationAction::Disable, Availability::Active) => {
                "UPDATE pastes SET availability_status = 'disabled', moderation_reason = ?2, moderated_at = ?3
                  WHERE id = ?1 AND availability_status = 'active'"
            }
            // Deleting drops the payload. From `disabled` as well as `active`:
            // disabling first and deleting after review is the expected path.
            (ModerationAction::Delete, Availability::Active | Availability::Disabled) => {
                "UPDATE pastes SET availability_status = 'deleted', ciphertext = NULL,
                        moderation_reason = ?2, moderated_at = ?3
                  WHERE id = ?1 AND availability_status != 'deleted'"
            }
            // `?2` (the reason) is bound but unused here: a restore clears the
            // stored reason, and the audit entry below keeps why it happened.
            (ModerationAction::Restore, Availability::Disabled) => {
                "UPDATE pastes SET availability_status = 'active', moderation_reason = NULL, moderated_at = ?3
                  WHERE id = ?1 AND availability_status = 'disabled'"
            }
            _ => return Ok(ModerationOutcome::NotApplicable(current)),
        };

        let changed = sqlx::query(statement)
            .bind(&id)
            .bind(reason)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(db)?
            .rows_affected();
        if changed != 1 {
            return Ok(ModerationOutcome::NotApplicable(current));
        }

        sqlx::query("INSERT INTO moderation_actions (paste_id, action, reason_code, note, created_at) VALUES (?1, ?2, ?3, ?4, ?5)")
            .bind(&id)
            .bind(action.as_str())
            .bind(reason)
            .bind(note)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(db)?;

        tx.commit().await.map_err(db)?;
        Ok(ModerationOutcome::Applied)
    }

    async fn moderation_history(&self, paste_id: &PasteId) -> Result<Vec<ModerationRecord>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT paste_id, action, reason_code, note, created_at FROM moderation_actions
              WHERE paste_id = ?1 ORDER BY created_at, id",
        )
        .bind(paste_id.encode())
        .fetch_all(&self.pool)
        .await
        .map_err(db)?;

        rows.iter()
            .map(|row| {
                Ok(ModerationRecord {
                    paste_id: row.try_get("paste_id").map_err(db)?,
                    action: row.try_get("action").map_err(db)?,
                    reason: row.try_get("reason_code").map_err(db)?,
                    note: row.try_get("note").map_err(db)?,
                    created_at: row.try_get("created_at").map_err(db)?,
                })
            })
            .collect()
    }

    async fn purge_abuse(&self, cutoff: i64) -> Result<AbusePurgeOutcome, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(db)?;

        let reports = sqlx::query("DELETE FROM abuse_reports WHERE status = 'closed' AND handled_at <= ?1")
            .bind(cutoff)
            .execute(&mut *tx)
            .await
            .map_err(db)?
            .rows_affected();

        let actions = sqlx::query("DELETE FROM moderation_actions WHERE created_at <= ?1")
            .bind(cutoff)
            .execute(&mut *tx)
            .await
            .map_err(db)?
            .rows_affected();

        tx.commit().await.map_err(db)?;
        Ok(AbusePurgeOutcome { reports, actions })
    }
}

#[cfg(test)]
mod tests;
