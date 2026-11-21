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
//! - **`journal_size_limit`** (64 MiB) caps the size the write-ahead log is
//!   truncated to on a WAL reset. Without it the log stays at its peak size
//!   after a burst of large uploads.
//!
//! Removing ciphertext happens in four distinct steps:
//!
//! 1. **Logically** it is unavailable at once: the delete, burn or purge
//!    commits and no request can read it again.
//! 2. **The database file** has the old pages overwritten by `secure_delete`
//!    only once a checkpoint has copied the zeroed pages from the WAL into it.
//! 3. **WAL frames** written before the delete can keep the old ciphertext
//!    after that checkpoint, until the frames are reused by later writes or
//!    the WAL is truncated. An ordinary checkpoint does neither.
//! 4. **Truncation** of the WAL happens only on a `wal_checkpoint(TRUNCATE)`,
//!    when the last connection closes, or down to `journal_size_limit` when
//!    the WAL is reset - so up to 64 MiB of old frames can remain.
//!
//! Migrations are embedded in the binary, so a deployment is one file and cannot
//! drift from the schema it expects.
//!
//! The database file is created here, not by SQLite: exclusively and owner-only
//! (`0600` on Unix) whatever the umask, because SQLite gives `-wal` and `-shm`
//! the mode of the main file, and a `chmod` afterwards could not revoke a
//! descriptor opened in between. A missing file next to what an interrupted
//! restore left behind is refused instead of being replaced by an empty one.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::path::{Path, PathBuf};
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
    #[error("could not create the database file {}: {source}", path.display())]
    Create {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not inspect the database file {}: {source}", path.display())]
    InspectFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not look for restore leftovers in {}: {source}", directory.display())]
    Inspect {
        directory: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The database is missing, but a restore was interrupted next to it.
    /// Creating an empty database would hide the operator's data behind a
    /// server that looks healthy.
    #[error(
        "the database {} is missing, but an interrupted restore left {} behind; no new database \
         was created. Run the restore again, or move the old database back to {}",
        path.display(),
        list_paths(leftovers),
        path.display()
    )]
    RestoreLeftovers { path: PathBuf, leftovers: Vec<PathBuf> },
    /// The binary expects a newer schema than the file provides. Serving anyway
    /// would risk writing rows the schema cannot represent.
    #[error("schema is at version {found:?} but this build expects {expected}")]
    SchemaMismatch { found: Option<i64>, expected: i64 },
}

/// Upper bound in bytes the write-ahead log is truncated to when it is reset;
/// frames below it are kept and may hold old ciphertext until reused.
const JOURNAL_SIZE_LIMIT: i64 = 64 * 1024 * 1024;

fn list_paths(paths: &[PathBuf]) -> String {
    paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ")
}

/// Opens the pool and applies pending migrations.
///
/// A missing database file is refused if a restore leftover lies next to it
/// (`<file>.pre-restore-*`, a directory `.<file>.restore-*`), and otherwise created owner-only before SQLite
/// opens it.
pub async fn connect(config: &DatabaseConfig) -> Result<SqlitePool, DbError> {
    let in_memory = config.path == Path::new(IN_MEMORY);
    if !in_memory {
        prepare_file(&config.path, config.directory())?;
    }

    let options = SqliteConnectOptions::new()
        .filename(&config.path)
        // The file exists by now; SQLite must not create one with its own mode.
        .create_if_missing(in_memory)
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

/// SQLite's name for a database that lives only in memory, with no file to
/// create or protect.
const IN_MEMORY: &str = ":memory:";

/// Refuses a missing database next to restore leftovers, otherwise creates a
/// missing database file owner-only.
fn prepare_file(path: &Path, directory: &Path) -> Result<(), DbError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(DbError::InspectFile {
                path: path.to_owned(),
                source,
            });
        }
    }

    let leftovers = restore_leftovers(path, directory).map_err(|source| DbError::Inspect {
        directory: directory.to_owned(),
        source,
    })?;
    if !leftovers.is_empty() {
        return Err(DbError::RestoreLeftovers {
            path: path.to_owned(),
            leftovers,
        });
    }

    match create_private_file(path) {
        Ok(_) => Ok(()),
        // Created by someone else in between: opened as an existing database.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(source) => Err(DbError::Create {
            path: path.to_owned(),
            source,
        }),
    }
}

/// Creates a new file that only its owner can read and write (`0600` on Unix,
/// independent of the umask). Fails if any entry exists at `path`, a symlink
/// included, so it never opens a file someone else placed there.
pub fn create_private_file(path: &Path) -> std::io::Result<File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// `prefix` followed by 16 random hex digits from the operating system: a
/// scratch name nobody can predict and plant an entry under beforehand.
pub fn random_name(prefix: &OsStr) -> std::io::Result<OsString> {
    let mut suffix = [0_u8; 8];
    getrandom::fill(&mut suffix).map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut name = prefix.to_owned();
    name.push(suffix.iter().map(|byte| format!("{byte:02x}")).collect::<String>());
    Ok(name)
}

/// The directory `path` lies in; `.` for a bare file name.
pub fn parent_or_current(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// The `-wal` and `-shm` files SQLite keeps next to the database `path`.
pub fn sidecars(path: &Path) -> [PathBuf; 2] {
    let with = |suffix: &str| {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    [with("-wal"), with("-shm")]
}

/// The file name prefix of the entries `restore` moves the old database and
/// its `-wal`/`-shm` to: `<file>.pre-restore-` for the database `<dir>/<file>`.
/// The full name appends the time of the restore.
pub fn pre_restore_prefix(database: &Path) -> OsString {
    let mut prefix = file_name(database).to_owned();
    prefix.push(".pre-restore-");
    prefix
}

/// The name prefix of the private directories `restore` stages a backup in:
/// `.<file>.restore-` for the database `<dir>/<file>`. The full name appends a
/// random suffix.
pub fn restore_stage_prefix(database: &Path) -> OsString {
    let mut prefix = OsString::from(".");
    prefix.push(file_name(database));
    prefix.push(".restore-");
    prefix
}

fn file_name(database: &Path) -> &OsStr {
    database.file_name().unwrap_or(database.as_os_str())
}

/// The entries of `directory` an interrupted restore of `database` left
/// behind: any `<file>.pre-restore-*` and any directory `.<file>.restore-*`,
/// sorted by path.
fn restore_leftovers(database: &Path, directory: &Path) -> std::io::Result<Vec<PathBuf>> {
    let aside = pre_restore_prefix(database);
    let stage = restore_stage_prefix(database);
    let mut leftovers = Vec::new();

    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.as_encoded_bytes();
        let is_aside = name.starts_with(aside.as_encoded_bytes());
        let is_stage = name.starts_with(stage.as_encoded_bytes()) && entry.file_type()?.is_dir();
        if is_aside || is_stage {
            leftovers.push(entry.path());
        }
    }

    leftovers.sort();
    Ok(leftovers)
}

/// Warnings about database files other local users can access: the database
/// and an existing `-wal`/`-shm` with any group or other permission bit, and
/// the database directory with a group or other write bit. Each names the
/// path and its octal mode. Only reads metadata; nothing on disk is changed,
/// and a file that does not exist is skipped. Always empty outside Unix.
pub fn permission_warnings(config: &DatabaseConfig) -> Vec<String> {
    #[cfg(unix)]
    {
        unix_permission_warnings(&config.path, config.directory())
    }
    #[cfg(not(unix))]
    {
        let _ = config;
        Vec::new()
    }
}

#[cfg(unix)]
fn unix_permission_warnings(database: &Path, directory: &Path) -> Vec<String> {
    use std::os::unix::fs::PermissionsExt as _;

    let mode = |path: &Path| std::fs::metadata(path).ok().map(|metadata| metadata.permissions().mode() & 0o7777);
    let [wal, shm] = sidecars(database);

    let mut warnings = Vec::new();
    for path in [database.to_owned(), wal, shm] {
        if let Some(mode) = mode(&path).filter(|mode| mode & 0o077 != 0) {
            let access = match (mode & 0o044 != 0, mode & 0o022 != 0) {
                (true, true) => "readable and writable",
                (true, false) => "readable",
                (false, true) => "writable",
                (false, false) => "accessible",
            };
            warnings.push(format!(
                "database file {} has mode {mode:04o} and is {access} by other users",
                path.display()
            ));
        }
    }
    if let Some(mode) = mode(directory).filter(|mode| mode & 0o022 != 0) {
        warnings.push(format!(
            "database directory {} has mode {mode:04o} and is writable by other users",
            directory.display()
        ));
    }
    warnings
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

    #[test]
    fn leftover_prefixes_follow_the_file_name() {
        let database = Path::new("/data/capsulebin.db");
        assert_eq!(pre_restore_prefix(database), "capsulebin.db.pre-restore-");
        assert_eq!(restore_stage_prefix(database), ".capsulebin.db.restore-");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn creates_a_missing_database_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let config = config("private");
        let pool = connect(&config).await.expect("connect");
        for suffix in ["", "-wal", "-shm"] {
            let mut path = config.path.as_os_str().to_owned();
            path.push(suffix);
            let mode = std::fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{suffix}");
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn refuses_a_missing_database_next_to_restore_leftovers() {
        let aside = config("leftover-aside");
        std::fs::write(aside.directory().join("capsulebin.db.pre-restore-1700000000"), b"old").expect("aside");
        let stage = config("leftover-stage");
        std::fs::create_dir(stage.directory().join(".capsulebin.db.restore-0123")).expect("stage");

        for (config, leftover) in [
            (&aside, "capsulebin.db.pre-restore-1700000000"),
            (&stage, ".capsulebin.db.restore-0123"),
        ] {
            let error = connect(config).await.expect_err("must refuse");
            assert!(matches!(error, DbError::RestoreLeftovers { .. }), "{error}");
            assert!(error.to_string().contains(leftover), "{error}");
            assert!(!config.path.exists(), "database created next to {leftover}");
        }

        // With the database present, the leftovers do not matter.
        std::fs::remove_dir(stage.directory().join(".capsulebin.db.restore-0123")).expect("remove stage");
        connect(&stage).await.expect("connect without leftovers").close().await;
        std::fs::create_dir(stage.directory().join(".capsulebin.db.restore-0123")).expect("stage");
        connect(&stage).await.expect("connect next to leftovers").close().await;
    }

    /// A regular file with the stage prefix is not a stage directory.
    #[tokio::test]
    async fn a_file_named_like_a_stage_is_no_leftover() {
        let config = config("stage-file");
        std::fs::write(config.directory().join(".capsulebin.db.restore-file"), b"").expect("file");
        connect(&config).await.expect("connect").close().await;
    }

    #[tokio::test]
    async fn connects_to_an_in_memory_database() {
        let config = DatabaseConfig {
            path: PathBuf::from(IN_MEMORY),
            busy_timeout_ms: 5_000,
            synchronous: Synchronous::Full,
        };
        connect(&config).await.expect("connect").close().await;
    }

    #[cfg(unix)]
    #[test]
    fn permission_warnings_name_path_and_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let config = config("permissions");
        let directory = config.directory().to_owned();
        let set = |path: &Path, mode: u32| std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
        let wal = directory.join("capsulebin.db-wal");
        std::fs::write(&config.path, b"").expect("db");
        std::fs::write(&wal, b"").expect("wal");

        for mode in [0o700, 0o755] {
            set(&directory, mode);
            set(&config.path, 0o600);
            set(&wal, 0o600);
            assert!(permission_warnings(&config).is_empty(), "{mode:o}");
        }

        set(&directory, 0o770);
        set(&config.path, 0o640);
        set(&wal, 0o602);
        assert_eq!(
            permission_warnings(&config),
            [
                format!(
                    "database file {} has mode 0640 and is readable by other users",
                    config.path.display()
                ),
                format!("database file {} has mode 0602 and is writable by other users", wal.display()),
                format!(
                    "database directory {} has mode 0770 and is writable by other users",
                    directory.display()
                ),
            ]
        );
        set(&directory, 0o700);
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
