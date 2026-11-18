//! Backup and restore of the SQLite database.
//!
//! **Backup** uses `VACUUM INTO`, SQLite's way to write a consistent snapshot of
//! a live database to a new file. It runs next to a serving instance (WAL lets
//! the reader proceed while the server writes), never copies a `-wal` file, and
//! writes a compacted file: pages freed by deleted or burned pastes are not
//! carried into the backup.
//!
//! **Restore** puts a verified backup in place of the configured database. It
//! only ever copies the backup's main file — a `-wal` or `-shm` next to it is
//! ignored — and moves an existing database (with its own `-wal`/`-shm`, which
//! may hold committed data) aside instead of deleting it.
//!
//! Backups hold ciphertext, verifiers and abuse reports with contact details.
//! They are written with owner-only permissions and must be stored like the
//! database itself (ADR-002). Restore only files you created: the bundled SQLite
//! treats the database as trusted input (RUSTSEC-2026-0459, see `deny.toml`).

use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row as _, SqlitePool};

use crate::db;

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("{0} already exists; refusing to overwrite it")]
    Exists(PathBuf),
    #[error("{0} does not exist")]
    Missing(PathBuf),
    #[error("the path {0} is not valid UTF-8")]
    NonUtf8Path(PathBuf),
    #[error("the backup and the database are the same file")]
    SameFile,
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("database error: {0}")]
    Database(#[source] sqlx::Error),
    #[error("integrity check failed: {0}")]
    Integrity(String),
    #[error("not a CapsuleBin database (no migration table)")]
    NotCapsuleBin,
    #[error("the file is at schema version {found}, newer than this build ({expected}); use a newer binary")]
    TooNew { found: i64, expected: i64 },
    #[error(transparent)]
    Db(#[from] db::DbError),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> BackupError + '_ {
    move |source| BackupError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// What a verified backup file contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupInfo {
    pub schema_version: i64,
    pub pastes: i64,
    pub reports: i64,
    pub bytes: u64,
}

/// Writes a consistent, compacted snapshot of `pool`'s database to `output`.
///
/// Refuses to overwrite. The file gets owner-only permissions and is verified
/// after writing.
pub async fn create(pool: &SqlitePool, output: &Path) -> Result<BackupInfo, BackupError> {
    if output.exists() {
        return Err(BackupError::Exists(output.to_path_buf()));
    }
    let target = output.to_str().ok_or_else(|| BackupError::NonUtf8Path(output.to_path_buf()))?;

    sqlx::query("VACUUM INTO ?1")
        .bind(target)
        .execute(pool)
        .await
        .map_err(BackupError::Database)?;

    restrict_permissions(output)?;
    verify_file(output).await
}

/// Opens `path` read-only without migrating it and checks that it is an intact
/// CapsuleBin database this build can use (same or older schema).
pub async fn verify_file(path: &Path) -> Result<BackupInfo, BackupError> {
    let metadata = std::fs::metadata(path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => BackupError::Missing(path.to_path_buf()),
        _ => BackupError::Io {
            path: path.to_path_buf(),
            source: error,
        },
    })?;

    let options = SqliteConnectOptions::new().filename(path).read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(BackupError::Database)?;

    let result = async {
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&pool)
            .await
            .map_err(BackupError::Database)?;
        if integrity != "ok" {
            return Err(BackupError::Integrity(integrity));
        }

        let version: Option<i64> = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&pool)
            .await
            .map_err(|_| BackupError::NotCapsuleBin)?;
        let version = version.ok_or(BackupError::NotCapsuleBin)?;
        let expected = db::expected_version();
        if version > expected {
            return Err(BackupError::TooNew { found: version, expected });
        }

        let counts = sqlx::query("SELECT (SELECT COUNT(*) FROM pastes) AS pastes, (SELECT COUNT(*) FROM abuse_reports) AS reports")
            .fetch_one(&pool)
            .await
            .map_err(BackupError::Database)?;

        Ok(BackupInfo {
            schema_version: version,
            pastes: counts.try_get("pastes").map_err(BackupError::Database)?,
            reports: counts.try_get("reports").map_err(BackupError::Database)?,
            bytes: metadata.len(),
        })
    }
    .await;

    pool.close().await;
    result
}

/// The files SQLite keeps next to a database.
fn sidecars(path: &Path) -> [PathBuf; 2] {
    let with = |suffix: &str| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    [with("-wal"), with("-shm")]
}

/// Where an existing database was moved by [`restore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub info: BackupInfo,
    /// The previous database, if there was one, moved here with its sidecars.
    pub previous: Option<PathBuf>,
}

/// Replaces the database at `target` with the backup at `input`.
///
/// The server must be stopped: SQLite cannot tell whether another process will
/// open the file next. With `replace = false` an existing database is an error;
/// with `replace = true` it is moved to `<target>.pre-restore-<now>` together
/// with its `-wal`/`-shm`, never deleted. The caller then opens the restored file
/// with [`db::connect`], which applies any migrations an older backup lacks.
pub async fn restore(input: &Path, target: &Path, replace: bool, now: i64) -> Result<RestoreOutcome, BackupError> {
    let info = verify_file(input).await?;

    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(input), std::fs::canonicalize(target))
        && a == b
    {
        return Err(BackupError::SameFile);
    }

    let mut previous = None;
    let existing = target.exists() || sidecars(target).iter().any(|p| p.exists());
    if existing {
        if !replace {
            return Err(BackupError::Exists(target.to_path_buf()));
        }
        let mut aside = target.as_os_str().to_owned();
        aside.push(format!(".pre-restore-{now}"));
        let aside = PathBuf::from(aside);
        if aside.exists() {
            return Err(BackupError::Exists(aside));
        }
        if target.exists() {
            std::fs::rename(target, &aside).map_err(io(target))?;
        }
        for (from, to) in sidecars(target).iter().zip(sidecars(&aside)) {
            if from.exists() {
                std::fs::rename(from, &to).map_err(io(from))?;
            }
        }
        previous = Some(aside);
    }

    // Copy to a temporary name, then rename: a crash halfway leaves no
    // half-written file under the database's name.
    let mut temporary = target.as_os_str().to_owned();
    temporary.push(".restoring");
    let temporary = PathBuf::from(temporary);
    std::fs::copy(input, &temporary).map_err(io(&temporary))?;
    restrict_permissions(&temporary)?;
    std::fs::rename(&temporary, target).map_err(io(target))?;

    Ok(RestoreOutcome { info, previous })
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), BackupError> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(io(path))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<(), BackupError> {
    Ok(())
}

#[cfg(test)]
mod tests;
