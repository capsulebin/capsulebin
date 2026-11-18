//! SQLite connection setup and migrations.
//!
//! The pragmas set here are not tuning knobs, they are correctness requirements:
//!
//! - **WAL** so that readers do not block the single writer. It also means the
//!   data directory must be on a local filesystem — WAL over NFS is not reliable.
//! - **`busy_timeout`** because SQLite has exactly one writer. Without a timeout a
//!   concurrent write fails immediately with `SQLITE_BUSY` instead of waiting.
//! - **`synchronous = FULL`** by default. At this data volume the cost is small,
//!   and losing an acknowledged write would be unrecoverable for a one-time paste.
//! - **`foreign_keys = ON`** so that a future reference cannot silently dangle.
//! - **`secure_delete = ON`** so that deleted, burned and purged ciphertext is
//!   overwritten with zeros instead of lingering in free pages until they are
//!   reused. It costs some extra writing on delete; a burn should mean the
//!   bytes are gone from the database file, not merely unlinked.
//! - **`journal_size_limit`** (64 MiB) so that the write-ahead log is truncated
//!   after a checkpoint. Otherwise it stays at its peak size after a burst of
//!   large uploads, still holding old frames with ciphertext the database has
//!   long deleted.
//!
//! Migrations are embedded in the binary, so a deployment is one file and cannot
//! drift from the schema it expects.

use std::time::Duration;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{Row as _, SqlitePool};

use crate::config::DatabaseConfig;

/// Embedded migrations. The path is relative to this crate's manifest.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("could not open the database: {0}")]
    Connect(#[source] sqlx::Error),
    #[error("migrations failed: {0}")]
    Migrate(#[source] sqlx::migrate::MigrateError),
    #[error("database query failed: {0}")]
    Query(#[source] sqlx::Error),
    /// The binary expects a newer schema than the file provides. Serving anyway
    /// would risk writing rows the schema cannot represent.
    #[error("schema is at version {found:?} but this build expects {expected}")]
    SchemaMismatch { found: Option<i64>, expected: i64 },
}

/// Upper bound in bytes the write-ahead log is truncated to after a checkpoint.
const JOURNAL_SIZE_LIMIT: i64 = 64 * 1024 * 1024;

/// Opens the pool and applies pending migrations.
pub async fn connect(config: &DatabaseConfig) -> Result<SqlitePool, DbError> {
    let options = SqliteConnectOptions::new()
        .filename(&config.path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_millis(config.busy_timeout_ms))
        .synchronous(match config.synchronous {
            crate::config::Synchronous::Off => SqliteSynchronous::Off,
            crate::config::Synchronous::Normal => SqliteSynchronous::Normal,
            crate::config::Synchronous::Full => SqliteSynchronous::Full,
        })
        .foreign_keys(true)
        .pragma("secure_delete", "ON")
        .pragma("journal_size_limit", JOURNAL_SIZE_LIMIT.to_string());

    // A small pool: WAL allows concurrent readers, but there is still only one
    // writer, so more connections buy contention rather than throughput.
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .map_err(DbError::Connect)?;

    MIGRATOR.run(&pool).await.map_err(DbError::Migrate)?;

    Ok(pool)
}

/// Highest migration version this build carries.
pub fn expected_version() -> i64 {
    MIGRATOR.migrations.iter().map(|migration| migration.version).max().unwrap_or(0)
}

/// Highest migration version applied to the file.
async fn applied_version(pool: &SqlitePool) -> Result<Option<i64>, DbError> {
    // The table exists once the migrator has run at least once. Treating a
    // missing table as "no migrations" keeps the readiness check honest on a
    // database that was never migrated.
    let row = sqlx::query("SELECT MAX(version) AS version FROM _sqlx_migrations")
        .fetch_optional(pool)
        .await;

    match row {
        Ok(Some(row)) => Ok(row.try_get::<Option<i64>, _>("version").unwrap_or(None)),
        Ok(None) => Ok(None),
        Err(sqlx::Error::Database(_)) => Ok(None),
        Err(error) => Err(DbError::Query(error)),
    }
}

/// Verifies that the connection works and the schema is at the expected version.
///
/// Used by `/health/ready` and by the `check` subcommand. Both need the same
/// answer, so they share one implementation.
pub async fn verify(pool: &SqlitePool) -> Result<i64, DbError> {
    sqlx::query("SELECT 1").fetch_one(pool).await.map_err(DbError::Query)?;

    let expected = expected_version();
    match applied_version(pool).await? {
        Some(found) if found == expected => Ok(found),
        found => Err(DbError::SchemaMismatch { found, expected }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Synchronous;

    fn config(name: &str) -> DatabaseConfig {
        let dir = std::env::temp_dir().join(format!("capsulebin-db-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        DatabaseConfig {
            path: dir.join("capsulebin.db"),
            busy_timeout_ms: 5_000,
            synchronous: Synchronous::Full,
        }
    }

    #[tokio::test]
    async fn migrates_a_fresh_database_and_verifies() {
        let pool = connect(&config("fresh")).await.expect("connect");
        let version = verify(&pool).await.expect("schema is current");
        assert_eq!(version, expected_version());
    }

    /// Running twice must be a no-op. A migrator that reapplied its migrations
    /// would destroy data on every restart.
    #[tokio::test]
    async fn migrating_twice_is_idempotent() {
        let config = config("twice");

        let first = connect(&config).await.expect("first connect");
        sqlx::query("INSERT INTO pastes (id, format_version, public_header, reveal_verifier, delete_verifier, created_at, payload_size) VALUES ('x', 1, x'00', x'00', x'00', 1, 1)")
            .execute(&first)
            .await
            .expect("insert");
        first.close().await;

        let second = connect(&config).await.expect("second connect");
        let count: i64 = sqlx::query("SELECT COUNT(*) AS n FROM pastes")
            .fetch_one(&second)
            .await
            .expect("count")
            .try_get("n")
            .expect("column");

        assert_eq!(count, 1, "migration ran again and lost the row");
    }

    /// The pragmas are the reason this module exists, so they are asserted
    /// rather than assumed.
    #[tokio::test]
    async fn applies_the_required_pragmas() {
        let pool = connect(&config("pragmas")).await.expect("connect");

        let journal: String = sqlx::query("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .expect("journal_mode")
            .try_get(0)
            .expect("value");
        assert_eq!(journal.to_lowercase(), "wal");

        let foreign_keys: i64 = sqlx::query("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .expect("foreign_keys")
            .try_get(0)
            .expect("value");
        assert_eq!(foreign_keys, 1);

        let synchronous: i64 = sqlx::query("PRAGMA synchronous")
            .fetch_one(&pool)
            .await
            .expect("synchronous")
            .try_get(0)
            .expect("value");
        // 2 == FULL
        assert_eq!(synchronous, 2);

        let secure_delete: i64 = sqlx::query("PRAGMA secure_delete")
            .fetch_one(&pool)
            .await
            .expect("secure_delete")
            .try_get(0)
            .expect("value");
        assert_eq!(secure_delete, 1);

        let journal_size_limit: i64 = sqlx::query("PRAGMA journal_size_limit")
            .fetch_one(&pool)
            .await
            .expect("journal_size_limit")
            .try_get(0)
            .expect("value");
        assert_eq!(journal_size_limit, JOURNAL_SIZE_LIMIT);
    }

    /// The schema check has to fail on a database that was never migrated,
    /// otherwise readiness would report a server that cannot serve.
    #[tokio::test]
    async fn verify_rejects_an_unmigrated_database() {
        let dir = std::env::temp_dir().join("capsulebin-db-unmigrated");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let options = SqliteConnectOptions::new().filename(dir.join("empty.db")).create_if_missing(true);
        let pool = SqlitePoolOptions::new().connect_with(options).await.expect("connect");

        let error = verify(&pool).await.expect_err("must reject");
        assert!(matches!(error, DbError::SchemaMismatch { found: None, .. }));
    }

    #[tokio::test]
    async fn indexes_exist() {
        let pool = connect(&config("indexes")).await.expect("connect");

        let rows = sqlx::query("SELECT name FROM sqlite_master WHERE type = 'index'")
            .fetch_all(&pool)
            .await
            .expect("query indexes");

        let names: Vec<String> = rows.iter().filter_map(|row| row.try_get::<String, _>("name").ok()).collect();

        for expected in [
            "idx_pastes_expires_at",
            "idx_pastes_created_at",
            "idx_pastes_last_retrieved_at",
            "idx_abuse_reports_status",
        ] {
            assert!(names.iter().any(|name| name == expected), "{expected} missing");
        }
    }
}
