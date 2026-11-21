//! Backup and restore of the SQLite database.
//!
//! **Backup** uses `VACUUM INTO`, SQLite's way to write a consistent snapshot of
//! a live database to a new file. It runs next to a serving instance (WAL lets
//! the reader proceed while the server writes), never copies a `-wal` file, and
//! writes a compacted file: pages freed by deleted or burned pastes are not
//! carried into the backup.
//!
//! **Restore** puts a verified backup in place of the configured database, so
//! that the configured path holds either the old or the new database, never
//! neither. It copies only the backup's main file — a `-wal` or `-shm` next to it never
//! reaches SQLite — into a private stage directory next to the database,
//! verifies and migrates that copy, and only then moves an existing database
//! (with its own `-wal`/`-shm`, which may hold committed data) aside and renames
//! the copy into place. A failure before or during that cutover undoes every
//! move.
//!
//! Backups hold ciphertext, verifiers and abuse reports with contact details.
//! They are written into a private stage directory with owner-only permissions
//! and must be stored like the database itself (ADR-002). Restore only files you
//! created: the bundled SQLite treats the database as trusted input
//! (RUSTSEC-2026-0459, see `deny.toml`); verification on a private standalone
//! copy is the interim measure.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row as _, SqlitePool};

use crate::config::DatabaseConfig;
use crate::db;

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("{0} already exists; refusing to overwrite it")]
    Exists(PathBuf),
    #[error("{0} does not exist")]
    Missing(PathBuf),
    #[error("the path {0} is not valid UTF-8")]
    NonUtf8Path(PathBuf),
    #[error("the path {0} does not name a file")]
    NoFileName(PathBuf),
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
    #[error("the checkpoint of the staged copy could not complete")]
    CheckpointBusy,
    /// The restore failed, and moving the files back failed as well.
    #[error("{error}; undoing the restore failed as well, move by hand: {}", list_moves(stranded))]
    RollbackFailed {
        error: Box<BackupError>,
        /// `(where the file is now, where it belongs)`.
        stranded: Vec<(PathBuf, PathBuf)>,
    },
    #[error(transparent)]
    Db(#[from] db::DbError),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> BackupError + '_ {
    move |source| BackupError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn list_moves(moves: &[(PathBuf, PathBuf)]) -> String {
    moves
        .iter()
        .map(|(from, to)| format!("{} to {}", from.display(), to.display()))
        .collect::<Vec<_>>()
        .join(", ")
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
/// The snapshot is written and verified inside a new private stage directory
/// next to `output`, then renamed to `output` unless an entry (a dangling
/// symlink included) exists there. The stage is removed on every path.
pub async fn create(pool: &SqlitePool, output: &Path) -> Result<BackupInfo, BackupError> {
    if occupied(output) {
        return Err(BackupError::Exists(output.to_path_buf()));
    }
    let name = output.file_name().ok_or_else(|| BackupError::NoFileName(output.to_path_buf()))?;

    let mut prefix = std::ffi::OsString::from(".");
    prefix.push(name);
    prefix.push(".backup-");
    let stage = Stage::create(db::parent_or_current(output), &prefix)?;
    let staged = stage.path.join(name);
    let target = staged.to_str().ok_or_else(|| BackupError::NonUtf8Path(staged.clone()))?;

    sqlx::query("VACUUM INTO ?1")
        .bind(target)
        .execute(pool)
        .await
        .map_err(BackupError::Database)?;

    restrict_permissions(&staged)?;
    step(Step::BackupWritten, &staged)?;
    let info = verify_file(&staged).await?;

    // `rename` would replace an entry at `output`, so check right before it. The
    // window between the check and the rename is accepted, as in
    // `Plan::check_collisions`: `rename` follows no symlink at the destination,
    // and only someone who can write the output's directory could plant an
    // entry there, who could replace the backup anyway. A rename keeps the
    // backup usable on filesystems without hard links.
    if occupied(output) {
        return Err(BackupError::Exists(output.to_path_buf()));
    }
    std::fs::rename(&staged, output).map_err(io(output))?;
    Ok(info)
}

/// Opens `path` read-only without migrating it and checks that it is an intact
/// CapsuleBin database this build can use (same or older schema).
///
/// SQLite treats the file as trusted input (RUSTSEC-2026-0459): call it only on
/// a file this process wrote, such as a staged copy.
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

/// Whether any entry exists at `path`, a dangling symlink included.
fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Whether a database or one of its sidecars exists at `path`.
fn database_present(path: &Path) -> bool {
    occupied(path) || db::sidecars(path).iter().any(|sidecar| occupied(sidecar))
}

/// The result of [`restore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    /// The backup as verified, before the migration.
    pub info: BackupInfo,
    /// The schema version the installed file was migrated to.
    pub schema_version: i64,
    /// The previous database, if there was one, moved here with its sidecars.
    pub previous: Option<PathBuf>,
}

/// The result of [`restore_dry_run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunOutcome {
    /// The backup as verified, before the migration.
    pub info: BackupInfo,
    /// The schema version the migration of the staged copy reached.
    pub schema_version: i64,
    /// A database exists at the target, and `replace` was not given: the
    /// restore itself would refuse.
    pub needs_replace: bool,
}

/// Replaces the database `target` with the backup at `input`.
///
/// The server must be stopped: SQLite cannot tell whether another process will
/// open the file next. With `replace = false` an existing database is an error;
/// with `replace = true` it is moved to `<file>.pre-restore-<now>` together
/// with its `-wal`/`-shm`, never deleted.
///
/// Only the backup's main file is copied, into a private stage directory next
/// to the database, and verified and migrated there like `serve` would. Then
/// the old files move aside and the copy is renamed into place. Any failure
/// undoes the moves made so far and removes the stage, so the old database and
/// its sidecars stay exactly as they were.
pub async fn restore(input: &Path, target: &DatabaseConfig, replace: bool, now: i64) -> Result<RestoreOutcome, BackupError> {
    let plan = Plan::new(input, &target.path, now)?;
    if plan.existing && !replace {
        return Err(BackupError::Exists(target.path.clone()));
    }
    plan.check_collisions()?;

    let stage = Stage::for_restore(target)?;
    let staged = stage.path.join(file_name(&target.path)?);
    let (info, schema_version) = stage_copy(input, &staged, target).await?;

    plan.check_collisions()?;
    plan.cutover(&staged)?;
    Ok(RestoreOutcome {
        info,
        schema_version,
        previous: plan.existing.then_some(plan.aside),
    })
}

/// Runs [`restore`] up to and including the migration of the staged copy,
/// then removes the stage. Nothing outside the stage is created or changed.
pub async fn restore_dry_run(input: &Path, target: &DatabaseConfig, replace: bool, now: i64) -> Result<DryRunOutcome, BackupError> {
    let plan = Plan::new(input, &target.path, now)?;
    let needs_replace = plan.existing && !replace;
    if !needs_replace {
        plan.check_collisions()?;
    }

    let stage = Stage::for_restore(target)?;
    let staged = stage.path.join(file_name(&target.path)?);
    let (info, schema_version) = stage_copy(input, &staged, target).await?;
    Ok(DryRunOutcome {
        info,
        schema_version,
        needs_replace,
    })
}

/// The files a restore moves: the target and its sidecars, and the names they
/// move to.
struct Plan {
    target: PathBuf,
    aside: PathBuf,
    /// The target or one of its sidecars exists and has to move aside.
    existing: bool,
}

impl Plan {
    fn new(input: &Path, target: &Path, now: i64) -> Result<Self, BackupError> {
        if let (Ok(a), Ok(b)) = (std::fs::canonicalize(input), std::fs::canonicalize(target))
            && a == b
        {
            return Err(BackupError::SameFile);
        }

        let mut aside = db::pre_restore_prefix(target);
        aside.push(now.to_string());
        Ok(Self {
            target: target.to_path_buf(),
            aside: target.with_file_name(aside),
            existing: database_present(target),
        })
    }

    /// `(from, to)` for the target and each sidecar.
    fn moves(&self) -> [(PathBuf, PathBuf); 3] {
        let [wal, shm] = db::sidecars(&self.target);
        let [aside_wal, aside_shm] = db::sidecars(&self.aside);
        [(self.target.clone(), self.aside.clone()), (wal, aside_wal), (shm, aside_shm)]
    }

    /// Refuses when a name the cutover writes to is taken: the target if
    /// nothing moves aside, else any of the aside names.
    ///
    /// `rename` in [`Plan::cutover`] replaces an existing destination, so an
    /// entry created between the last check and the rename would be lost. That
    /// window is accepted: a restore requires a stopped server, and planting an
    /// entry there needs write access to the database directory, which already
    /// allows replacing the database itself. Publishing by hard link instead
    /// would make restore depend on hard links on the database volume.
    fn check_collisions(&self) -> Result<(), BackupError> {
        if !self.existing {
            if database_present(&self.target) {
                return Err(BackupError::Exists(self.target.clone()));
            }
            return Ok(());
        }
        match self.moves().into_iter().find(|(_, to)| occupied(to)) {
            Some((_, to)) => Err(BackupError::Exists(to)),
            None => Ok(()),
        }
    }

    /// Moves the existing files aside and renames `staged` to the target. On a
    /// failure every move made so far is undone, in reverse order.
    fn cutover(&self, staged: &Path) -> Result<(), BackupError> {
        let mut planned: Vec<(Step, PathBuf, PathBuf)> = [Step::MoveDatabase, Step::MoveWal, Step::MoveShm]
            .into_iter()
            .zip(self.moves())
            .filter(|(_, (from, _))| occupied(from))
            .map(|(step, (from, to))| (step, from, to))
            .collect();
        planned.push((Step::Install, staged.to_path_buf(), self.target.clone()));

        let mut done = Vec::new();
        for (step_name, from, to) in planned {
            if let Err(error) = std::fs::rename(&from, &to) {
                return Err(undo(done, io(&from)(error)));
            }
            done.push((from, to.clone()));
            if let Err(error) = step(step_name, &to) {
                return Err(undo(done, error));
            }
        }
        Ok(())
    }
}

/// Moves every `(from, to)` in `done` back, last first. Returns `error`, or
/// [`BackupError::RollbackFailed`] naming what could not be moved back.
fn undo(done: Vec<(PathBuf, PathBuf)>, error: BackupError) -> BackupError {
    let mut stranded = Vec::new();
    for (from, to) in done.into_iter().rev() {
        if std::fs::rename(&to, &from).is_err() {
            stranded.push((to, from));
        }
    }
    if stranded.is_empty() {
        return error;
    }
    BackupError::RollbackFailed {
        error: Box::new(error),
        stranded,
    }
}

/// Steps 3 to 5 of a restore: copies only the main file of `input` to
/// `staged`, verifies the copy, migrates it like `serve` would and checkpoints
/// it, so the staged file is self-contained. Returns the verified contents
/// and the schema version reached.
async fn stage_copy(input: &Path, staged: &Path, target: &DatabaseConfig) -> Result<(BackupInfo, i64), BackupError> {
    copy_main_file(input, staged)?;
    step(Step::Copy, staged)?;
    let info = verify_file(staged).await?;
    step(Step::Verify, staged)?;

    let config = DatabaseConfig {
        path: staged.to_path_buf(),
        ..target.clone()
    };
    let pool = db::connect(&config).await?;
    let result = async {
        step(Step::Migrate, staged)?;
        let version = db::verify(&pool).await?;
        let busy: i64 = sqlx::query_scalar("PRAGMA wal_checkpoint(TRUNCATE)")
            .fetch_one(&pool)
            .await
            .map_err(BackupError::Database)?;
        if busy != 0 {
            return Err(BackupError::CheckpointBusy);
        }
        step(Step::Checkpoint, staged)?;
        Ok(version)
    }
    .await;
    pool.close().await;

    Ok((info, result?))
}

/// Copies `input` to the new owner-only file `staged`.
fn copy_main_file(input: &Path, staged: &Path) -> Result<(), BackupError> {
    let mut source = std::fs::File::open(input).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => BackupError::Missing(input.to_path_buf()),
        _ => BackupError::Io {
            path: input.to_path_buf(),
            source: error,
        },
    })?;
    let mut copy = db::create_private_file(staged).map_err(io(staged))?;
    std::io::copy(&mut source, &mut copy).map_err(io(staged))?;
    copy.sync_all().map_err(io(staged))
}

fn file_name(path: &Path) -> Result<&OsStr, BackupError> {
    path.file_name().ok_or_else(|| BackupError::NoFileName(path.to_path_buf()))
}

/// A private directory (`0700` on Unix) with a random name, created
/// exclusively and removed with everything in it when dropped.
struct Stage {
    path: PathBuf,
}

impl Stage {
    /// The stage of a restore of `target`, next to the database.
    fn for_restore(target: &DatabaseConfig) -> Result<Self, BackupError> {
        Self::create(target.directory(), &db::restore_stage_prefix(&target.path))
    }

    fn create(directory: &Path, prefix: &OsStr) -> Result<Self, BackupError> {
        let name = db::random_name(prefix).map_err(io(directory))?;
        let path = directory.join(name);

        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&path).map_err(io(&path))?;
        Ok(Self { path })
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The points a test can make an operation fail at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// The backup snapshot is written into its stage.
    BackupWritten,
    Copy,
    Verify,
    Migrate,
    Checkpoint,
    MoveDatabase,
    MoveWal,
    MoveShm,
    Install,
}

/// Called after each step succeeded, with the file it produced. Does nothing
/// outside tests; in tests it runs the hook a test has set.
fn step(step: Step, path: &Path) -> Result<(), BackupError> {
    #[cfg(test)]
    tests::run_hook(step, path).map_err(io(path))?;
    #[cfg(not(test))]
    let _ = (step, path);
    Ok(())
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
