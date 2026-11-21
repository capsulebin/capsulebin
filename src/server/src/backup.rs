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
//! move. Before the cutover, the ids of the replaced database that the backup
//! lacks are retired in the copy, so a restore never frees an id.
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
use sqlx::{AssertSqlSafe, Row as _, SqlitePool};

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
    /// Ids of the previous database the backup does not hold as a row, now
    /// retired in the restored database so they are never used again.
    pub carried_ids: i64,
    /// Problems that did not stop the restore, such as a previous database
    /// whose ids could not be read.
    pub warnings: Vec<String>,
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
    /// The number of ids the restore would carry over, as in
    /// [`RestoreOutcome::carried_ids`].
    pub carried_ids: i64,
    /// The warnings the restore would report.
    pub warnings: Vec<String>,
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
///
/// Before the cutover, every id of the old database that the backup does not
/// hold as a row is retired in the staged copy, so the
/// restore frees no id. An old database that cannot be read only adds a
/// warning: corruption is the typical reason to restore.
pub async fn restore(input: &Path, target: &DatabaseConfig, replace: bool, now: i64) -> Result<RestoreOutcome, BackupError> {
    let plan = Plan::new(input, &target.path, now)?;
    if plan.existing && !replace {
        return Err(BackupError::Exists(target.path.clone()));
    }
    plan.check_collisions()?;

    let stage = Stage::for_restore(target)?;
    let staged = stage.path.join(file_name(&target.path)?);
    let copy = stage_copy(input, &staged, target, plan.replaced(), Carry::Insert).await?;

    plan.check_collisions()?;
    plan.cutover(&staged)?;
    Ok(RestoreOutcome {
        info: copy.info,
        schema_version: copy.schema_version,
        previous: plan.existing.then_some(plan.aside),
        carried_ids: copy.carried.count,
        warnings: copy.carried.warning.into_iter().collect(),
    })
}

/// Runs [`restore`] up to and including the migration of the staged copy and
/// counts the ids it would carry over, then removes the stage. Nothing outside
/// the stage is created or changed.
pub async fn restore_dry_run(input: &Path, target: &DatabaseConfig, replace: bool, now: i64) -> Result<DryRunOutcome, BackupError> {
    let plan = Plan::new(input, &target.path, now)?;
    let needs_replace = plan.existing && !replace;
    if !needs_replace {
        plan.check_collisions()?;
    }

    let stage = Stage::for_restore(target)?;
    let staged = stage.path.join(file_name(&target.path)?);
    let copy = stage_copy(input, &staged, target, plan.replaced(), Carry::Count).await?;
    Ok(DryRunOutcome {
        info: copy.info,
        schema_version: copy.schema_version,
        needs_replace,
        carried_ids: copy.carried.count,
        warnings: copy.carried.warning.into_iter().collect(),
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

    /// The database the restore replaces, whose ids it carries over: the
    /// target, if a file is there (sidecars alone hold no readable database).
    fn replaced(&self) -> Option<&Path> {
        occupied(&self.target).then_some(self.target.as_path())
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

/// The staged copy, ready for the cutover.
struct StagedCopy {
    /// The backup as verified, before the migration.
    info: BackupInfo,
    /// The schema version the migration reached.
    schema_version: i64,
    carried: CarriedIds,
}

/// Steps 3 to 6 of a restore: copies only the main file of `input` to
/// `staged`, verifies the copy, migrates it like `serve` would and checkpoints
/// it, so the staged file is self-contained. Then carries the ids of
/// `replaced` over (see [`carry_ids`]) and checkpoints again.
async fn stage_copy(
    input: &Path,
    staged: &Path,
    target: &DatabaseConfig,
    replaced: Option<&Path>,
    carry: Carry,
) -> Result<StagedCopy, BackupError> {
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
        let schema_version = db::verify(&pool).await?;
        checkpoint(&pool).await?;
        step(Step::Checkpoint, staged)?;

        let carried = match replaced {
            Some(replaced) => carry_ids(&pool, replaced, carry).await?,
            None => CarriedIds::default(),
        };
        checkpoint(&pool).await?;
        step(Step::CarryIds, staged)?;
        Ok(StagedCopy {
            info,
            schema_version,
            carried,
        })
    }
    .await;
    pool.close().await;
    result
}

/// Moves every page of the WAL into the database file and truncates the WAL.
async fn checkpoint(pool: &SqlitePool) -> Result<(), BackupError> {
    let busy: i64 = sqlx::query_scalar("PRAGMA wal_checkpoint(TRUNCATE)")
        .fetch_one(pool)
        .await
        .map_err(BackupError::Database)?;
    if busy != 0 {
        return Err(BackupError::CheckpointBusy);
    }
    Ok(())
}

/// What [`carry_ids`] does with the ids it finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Carry {
    /// Count them only (dry run).
    Count,
    /// Count them and insert them into the staged copy's `retired_ids`.
    Insert,
}

/// The outcome of [`carry_ids`].
#[derive(Debug, Default)]
struct CarriedIds {
    /// The ids carried over (or, for [`Carry::Count`], to carry over).
    count: i64,
    /// Why the ids of the replaced database could not be read.
    warning: Option<String>,
}

impl CarriedIds {
    fn unreadable(reason: impl std::fmt::Display) -> Self {
        Self {
            count: 0,
            warning: Some(format!(
                "the ids of the replaced database could not be carried over ({reason}); its ids that are not in the \
                 backup can be used again"
            )),
        }
    }
}

/// The ids of the attached replaced database (`old`): its rows and, from schema
/// version 4 on, its retired ids. Constant SQL, put into the statements of
/// [`carry_attached`] with `format!`; nothing from outside goes into them.
/// `UNION ALL`: the scratch table's primary key with `INSERT OR IGNORE`
/// already removes duplicates.
const OLD_IDS: &str = "SELECT id FROM old.pastes";
const OLD_IDS_WITH_RETIRED: &str = "SELECT id FROM old.pastes UNION ALL SELECT id FROM old.retired_ids";

/// Retires, in the staged copy behind `pool`, every id of `replaced` that the
/// copy does not hold as a row (`Carry::Count`: only counts them).
///
/// Everything stays inside SQLite: `replaced` is attached read-only to one
/// connection of the copy, so millions of ids never pass through this
/// process. A replaced database that cannot be attached, or that the read
/// probe of [`carry_attached`] cannot read, is not an error: the copy stays
/// unchanged and the result carries a warning. Every failure after the probe
/// is an error that stops the restore before the cutover.
async fn carry_ids(pool: &SqlitePool, replaced: &Path, carry: Carry) -> Result<CarriedIds, BackupError> {
    let uri = match read_only_uri(replaced) {
        Ok(uri) => uri,
        Err(reason) => return Ok(CarriedIds::unreadable(reason)),
    };
    let mut connection = pool.acquire().await.map_err(BackupError::Database)?;
    // Bound, never formatted into the statement: the path is not SQL.
    if let Err(error) = sqlx::query("ATTACH DATABASE ?1 AS old").bind(uri).execute(&mut *connection).await {
        return Ok(CarriedIds::unreadable(error));
    }

    let carried = carry_attached(&mut connection, carry).await;
    let detached = sqlx::query("DETACH DATABASE old").execute(&mut *connection).await;
    let carried = carried?;
    detached.map_err(BackupError::Database)?;
    Ok(carried)
}

/// The scratch table in the staged copy that holds the ids read from `old`.
/// Created, filled and dropped within one transaction of [`carry_attached`],
/// so it never reaches the installed database. Not `TEMP`: the bundled SQLite
/// keeps temporary tables in memory, and a temporary file would lie outside
/// the private stage.
const SCRATCH_TABLE: &str = "main.restore_carried_ids";

/// The part of [`carry_ids`] that runs while `old` is attached.
///
/// First a read probe: whether `old` has `retired_ids`, and a count over the
/// same ids the insert reads. It writes nothing into the copy, so its failure
/// can only come from `old`; it is the only failure here that becomes the
/// warning. Then, in one transaction, the ids are copied into a scratch table
/// in the copy ([`SCRATCH_TABLE`]), counted and inserted. Any failure there
/// is an error (the transaction rolls back), also a read failure of `old`
/// that shows up only then: it cannot be told apart from a failure writing
/// the copy, and installing the backup without the ids would free them.
async fn carry_attached(connection: &mut sqlx::SqliteConnection, carry: Carry) -> Result<CarriedIds, BackupError> {
    use sqlx::Connection as _;

    let probe = async {
        let has_retired: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM old.sqlite_master WHERE type = 'table' AND name = 'retired_ids')")
                .fetch_one(&mut *connection)
                .await?;
        let old_ids = if has_retired { OLD_IDS_WITH_RETIRED } else { OLD_IDS };
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!("SELECT count(*) FROM ({old_ids})")))
            .fetch_one(&mut *connection)
            .await?;
        Ok::<_, sqlx::Error>(old_ids)
    }
    .await;
    let old_ids = match probe {
        Ok(old_ids) => old_ids,
        Err(error) => return Ok(CarriedIds::unreadable(error)),
    };

    let mut transaction = connection.begin().await.map_err(BackupError::Database)?;
    sqlx::query(AssertSqlSafe(format!(
        "CREATE TABLE {SCRATCH_TABLE} (id TEXT PRIMARY KEY) WITHOUT ROWID"
    )))
    .execute(&mut *transaction)
    .await
    .map_err(BackupError::Database)?;
    // Tests can let the copy grow no further, so that the insert below fails
    // writing it (`SQLITE_FULL`).
    #[cfg(test)]
    if let Some(pages) = tests::carry_page_limit() {
        sqlx::query(AssertSqlSafe(format!("PRAGMA main.max_page_count = {pages}")))
            .execute(&mut *transaction)
            .await
            .map_err(BackupError::Database)?;
    }
    sqlx::query(AssertSqlSafe(format!("INSERT OR IGNORE INTO {SCRATCH_TABLE} (id) {old_ids}")))
        .execute(&mut *transaction)
        .await
        .map_err(BackupError::Database)?;

    let count: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT COUNT(*) FROM {SCRATCH_TABLE} \
         WHERE id NOT IN (SELECT id FROM main.pastes) AND id NOT IN (SELECT id FROM main.retired_ids)"
    )))
    .fetch_one(&mut *transaction)
    .await
    .map_err(BackupError::Database)?;
    if carry == Carry::Insert {
        sqlx::query(AssertSqlSafe(format!(
            "INSERT OR IGNORE INTO main.retired_ids (id) \
             SELECT id FROM {SCRATCH_TABLE} WHERE id NOT IN (SELECT id FROM main.pastes)"
        )))
        .execute(&mut *transaction)
        .await
        .map_err(BackupError::Database)?;
        sqlx::query(AssertSqlSafe(format!("DROP TABLE {SCRATCH_TABLE}")))
            .execute(&mut *transaction)
            .await
            .map_err(BackupError::Database)?;
        transaction.commit().await.map_err(BackupError::Database)?;
    } else {
        // Also drops the scratch table.
        transaction.rollback().await.map_err(BackupError::Database)?;
    }
    Ok(CarriedIds { count, warning: None })
}

/// A `file:` URI that opens the database `path` without writing to it, its
/// `-wal` or its `-shm`, and without creating either sidecar.
///
/// `mode=ro` alone is not enough: SQLite would still create a missing `-wal`
/// and `-shm` and rewrite an existing `-shm` (the WAL index). So:
/// - without a `-wal`, or with an empty one, the database file holds every
///   committed page: `immutable=1` reads it with no locks and no sidecar;
/// - with a non-empty `-wal` and a `-shm`, `readonly_shm=1` opens the `-shm`
///   read-only and reads the WAL into private memory, so its commits count;
/// - a non-empty `-wal` without a `-shm` cannot be read without creating one,
///   so that database is reported as unreadable.
///
/// The path is made absolute and percent-encoded by `url`, so `?`, `#` and `%`
/// in it stay part of the path.
fn read_only_uri(path: &Path) -> Result<String, &'static str> {
    let [wal, shm] = db::sidecars(path);
    let query = match std::fs::metadata(&wal) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => "mode=ro&immutable=1",
        Ok(metadata) if metadata.len() == 0 => "mode=ro&immutable=1",
        _ if occupied(&shm) => "mode=ro&readonly_shm=1",
        _ => return Err("its -wal file has no -shm file, which reading it would create"),
    };
    let absolute = std::path::absolute(path).map_err(|_| "its path cannot be made absolute")?;
    let mut uri = url::Url::from_file_path(&absolute).map_err(|()| "its path cannot be written as a URI")?;
    uri.set_query(Some(query));
    Ok(uri.into())
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
    /// The ids of the replaced database are carried over and checkpointed.
    CarryIds,
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
