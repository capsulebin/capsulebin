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
//! lacks are retired in the copy, so a restore never frees an id; the backup's
//! capsules whose ids it has retired (purged after the backup) are removed from
//! the copy, so a restore never brings back a purged capsule; and its creation
//! tokens and revocations are carried into the copy, so a restore never brings
//! back a revoked token or loses one issued after the backup.
//!
//! Backups hold ciphertext, verifiers and abuse reports with contact details.
//! They are written into a private stage directory with owner-only permissions
//! and must be stored like the database itself (ADR-002). Restore only files you
//! created: SQLite treats the database as trusted input, so verification runs
//! on a private standalone copy of the main file, never on the input.

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
/// SQLite treats the file as trusted input: call it only on
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
    /// Creation tokens of the previous database the backup lacks, matched by
    /// verifier, now inserted into the restored database.
    pub carried_tokens: i64,
    /// Creation tokens revoked in the previous database but active in the
    /// backup, now revoked with the previous database's `revoked_at`.
    pub carried_revocations: i64,
    /// Creation tokens in both whose `last_used_on` was later in the previous
    /// database, now set to that later day.
    pub carried_last_uses: i64,
    /// Capsules in both that the previous database holds burned, deleted or
    /// disabled while the backup holds them in an earlier state, now in the
    /// previous database's state (see [`CARRY_STATES`]).
    pub carried_states: i64,
    /// Capsules of the backup whose ids the previous database has retired
    /// (purged after the backup), now removed from the restored database with
    /// their ciphertext; their ids stay retired.
    pub carried_purges: i64,
    /// Problems that did not stop the restore, such as a previous database
    /// whose ids, capsule states and creation tokens could not be read.
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
    /// [`RestoreOutcome::carried_tokens`] the restore would carry over.
    pub carried_tokens: i64,
    /// [`RestoreOutcome::carried_revocations`] the restore would carry over.
    pub carried_revocations: i64,
    /// [`RestoreOutcome::carried_last_uses`] the restore would carry over.
    pub carried_last_uses: i64,
    /// [`RestoreOutcome::carried_states`] the restore would carry over.
    pub carried_states: i64,
    /// [`RestoreOutcome::carried_purges`] the restore would remove.
    pub carried_purges: i64,
    /// The warnings the restore would report.
    pub warnings: Vec<String>,
}

/// Replaces the database `target` with the backup at `input`.
///
/// The server must be stopped: SQLite cannot tell whether another process will
/// open the file next. With `replace = false` an existing database is an error;
/// with `replace = true` it is moved to `<file>.pre-restore-<now>` together
/// with its `-wal`/`-shm`, never deleted. A missing database next to restore
/// leftovers is refused before anything is created, like `serve` does.
///
/// Only the backup's main file is copied, into a private stage directory next
/// to the database, and verified and migrated there like `serve` would. Then
/// the old files move aside and the copy is renamed into place. Any failure
/// undoes the moves made so far and removes the stage, so the old database and
/// its sidecars stay exactly as they were.
///
/// Before the cutover, every id of the old database that the backup does not
/// hold as a row is retired in the staged copy, so the
/// restore frees no id; capsules the old database burned, deleted or disabled
/// keep that state; capsules whose ids it has retired (purged after the
/// backup) are removed; and its creation tokens and revocations are carried
/// into the copy (see [`carry_from_replaced`]). An old database that cannot be read only
/// adds a warning: corruption is the typical reason to restore.
pub async fn restore(input: &Path, target: &DatabaseConfig, replace: bool, now: i64) -> Result<RestoreOutcome, BackupError> {
    let plan = Plan::new(input, &target.path, now)?;
    refuse_leftovers(&plan, target)?;
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
        carried_ids: copy.carried.ids,
        carried_tokens: copy.carried.tokens.inserted,
        carried_revocations: copy.carried.tokens.revoked,
        carried_last_uses: copy.carried.tokens.last_used,
        carried_states: copy.carried.states,
        carried_purges: copy.carried.purges,
        warnings: copy.carried.warning.into_iter().collect(),
    })
}

/// Runs [`restore`] up to and including the migration of the staged copy and
/// counts the ids, capsule states, purged capsules and creation tokens it
/// would carry over, then removes the stage. Nothing outside the stage is
/// created or changed.
pub async fn restore_dry_run(input: &Path, target: &DatabaseConfig, replace: bool, now: i64) -> Result<DryRunOutcome, BackupError> {
    let plan = Plan::new(input, &target.path, now)?;
    refuse_leftovers(&plan, target)?;
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
        carried_ids: copy.carried.ids,
        carried_tokens: copy.carried.tokens.inserted,
        carried_revocations: copy.carried.tokens.revoked,
        carried_last_uses: copy.carried.tokens.last_used,
        carried_states: copy.carried.states,
        carried_purges: copy.carried.purges,
        warnings: copy.carried.warning.into_iter().collect(),
    })
}

/// Refuses a restore over a missing database next to restore leftovers, with
/// the same rule as `serve` (`db::refuse_restore_leftovers`): the old database
/// of an interrupted restore lies aside, and a restore now would carry none of
/// its ids, capsule states or revocations. Whether the database is there is
/// [`Plan::replaced`]'s answer, so refusal and carry-over cannot disagree.
/// Runs before [`Stage::for_restore`] creates this restore's own stage, which
/// has a leftover's name.
fn refuse_leftovers(plan: &Plan, target: &DatabaseConfig) -> Result<(), BackupError> {
    if plan.replaced().is_some() {
        return Ok(());
    }
    Ok(db::refuse_restore_leftovers(&target.path, target.directory())?)
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

    /// The database the restore replaces, whose ids, capsule states and creation tokens it
    /// carries over: the target, if a file is there (sidecars alone hold no
    /// readable database).
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
    carried: Carried,
}

/// Steps 3 to 6 of a restore: copies only the main file of `input` to
/// `staged`, verifies the copy, migrates it like `serve` would and checkpoints
/// it, so the staged file is self-contained. Then carries the ids and creation
/// tokens of `replaced` over (see [`carry_from_replaced`]) and checkpoints again.
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
            Some(replaced) => carry_from_replaced(&pool, replaced, carry).await?,
            None => Carried::default(),
        };
        checkpoint(&pool).await?;
        step(Step::Carry, staged)?;
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

/// What [`carry_from_replaced`] does with the ids, capsule states and creation
/// tokens it finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Carry {
    /// Count them only (dry run).
    Count,
    /// Count them and write them into the staged copy.
    Insert,
}

/// The outcome of [`carry_from_replaced`]. The counts are what was carried over (or, for
/// [`Carry::Count`], what would be).
#[derive(Debug, Default)]
struct Carried {
    /// Ids retired in the copy.
    ids: i64,
    /// Capsules of the copy moved to `old`'s later state, see [`CARRY_STATES`].
    states: i64,
    /// Capsules of the copy removed because `old` has retired their ids, see
    /// [`CARRY_PURGES`].
    purges: i64,
    tokens: CarriedTokens,
    /// Why the replaced database could not be read.
    warning: Option<String>,
}

/// The creation tokens part of [`Carried`], see [`carry_tokens`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct CarriedTokens {
    /// Tokens inserted because the copy lacks their verifier.
    inserted: i64,
    /// Tokens of the copy revoked because they are revoked in `old`.
    revoked: i64,
    /// Tokens of the copy whose `last_used_on` was moved to `old`'s later day.
    last_used: i64,
}

impl Carried {
    fn unreadable(reason: impl std::fmt::Display) -> Self {
        Self {
            warning: Some(format!(
                "the ids, capsule states and creation tokens of the replaced database could not be carried over \
                 ({reason}); its ids that are not in the backup can be used again, capsules it burned, deleted, \
                 disabled or purged after the backup are available again, its creation tokens that are not in the \
                 backup are lost, and tokens it revoked are active again if the backup holds them unrevoked"
            )),
            ..Self::default()
        }
    }
}

/// The ids of the attached replaced database (`old`): its rows and its retired
/// ids. Constant SQL, put into the statements of [`carry_attached`] with
/// `format!`; nothing from outside goes into them. `UNION ALL`: the scratch
/// table's primary key with `INSERT OR IGNORE` already removes duplicates.
const REPLACED_IDS: &str = "SELECT id FROM old.pastes UNION ALL SELECT id FROM old.retired_ids";

/// Retires, in the staged copy behind `pool`, every id of `replaced` that the
/// copy does not hold as a row, removes the copy's capsules whose ids
/// `replaced` has retired (see [`CARRY_PURGES`]), carries its later capsule
/// states (see [`carry_states`]) and its creation tokens over (see
/// [`carry_tokens`]); `Carry::Count` only counts them.
///
/// Everything stays inside SQLite: `replaced` is attached read-only to one
/// connection of the copy, so millions of ids never pass through this
/// process. A replaced database that cannot be attached, or that the read
/// probe of [`carry_attached`] cannot read, is not an error: the copy stays
/// unchanged and the result carries a warning. Every failure after the probe
/// is an error that stops the restore before the cutover.
async fn carry_from_replaced(pool: &SqlitePool, replaced: &Path, carry: Carry) -> Result<Carried, BackupError> {
    let uri = match read_only_uri(replaced) {
        Ok(uri) => uri,
        Err(reason) => return Ok(Carried::unreadable(reason)),
    };
    let mut connection = pool.acquire().await.map_err(BackupError::Database)?;
    // Bound, never formatted into the statement: the path is not SQL.
    if let Err(error) = sqlx::query("ATTACH DATABASE ?1 AS old").bind(uri).execute(&mut *connection).await {
        return Ok(Carried::unreadable(error));
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

/// The part of [`carry_from_replaced`] that runs while `old` is attached.
///
/// First a read probe: a count over the same ids, capsule states and tokens
/// the writes read. It writes nothing into the copy, so its failure can only
/// come from `old`; it is the only failure here that becomes the warning, and
/// then neither ids nor states nor tokens are carried. A database without
/// `retired_ids` or `creation_tokens` is not one this binary wrote, so a
/// missing table fails the probe like any other unreadable database. Then, in
/// one transaction, the ids are copied into a scratch table in the copy
/// ([`SCRATCH_TABLE`]), counted and inserted, the capsules `old` has purged
/// removed ([`CARRY_PURGES`]), the capsule states carried over
/// ([`carry_states`]) and the tokens carried over. Any failure there is an
/// error (the transaction rolls back), also a read failure of `old` that shows
/// up only then: it cannot be told apart from a failure writing the copy, and
/// installing the backup without the ids would free them, without the
/// revocations would revive tokens.
async fn carry_attached(connection: &mut sqlx::SqliteConnection, carry: Carry) -> Result<Carried, BackupError> {
    use sqlx::Connection as _;

    let probe = async {
        sqlx::query_scalar::<_, i64>(AssertSqlSafe(format!("SELECT count(*) FROM ({REPLACED_IDS})")))
            .fetch_one(&mut *connection)
            .await?;
        // The lifecycle columns [`CARRY_STATES`] reads, so the rows themselves
        // are scanned, not only the primary key index.
        sqlx::query(
            "SELECT count(*), max(consumed_at), max(last_retrieved_at), max(availability_status), \
             max(moderation_reason), max(moderated_at) FROM old.pastes",
        )
        .fetch_one(&mut *connection)
        .await?;
        // Every column, so the table itself is scanned: `count(*)` alone may
        // be answered from the smaller verifier index.
        sqlx::query(
            "SELECT count(*), max(id), max(verifier), max(label), max(created_at), max(last_used_on), \
             max(revoked_at) FROM old.creation_tokens",
        )
        .fetch_one(&mut *connection)
        .await
    }
    .await;
    if let Err(error) = probe {
        return Ok(Carried::unreadable(error));
    }

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
    sqlx::query(AssertSqlSafe(format!("INSERT OR IGNORE INTO {SCRATCH_TABLE} (id) {REPLACED_IDS}")))
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
    }
    let purges = carry_purges(&mut transaction, carry).await.map_err(BackupError::Database)?;
    let states = carry_states(&mut transaction, carry).await.map_err(BackupError::Database)?;
    let tokens = carry_tokens(&mut transaction, carry).await.map_err(BackupError::Database)?;
    if carry == Carry::Insert {
        sqlx::query(AssertSqlSafe(format!("DROP TABLE {SCRATCH_TABLE}")))
            .execute(&mut *transaction)
            .await
            .map_err(BackupError::Database)?;
        transaction.commit().await.map_err(BackupError::Database)?;
    } else {
        // Also drops the scratch table.
        transaction.rollback().await.map_err(BackupError::Database)?;
    }
    Ok(Carried {
        ids: count,
        states,
        purges,
        tokens,
        warning: None,
    })
}

/// The capsules [`carry_purges`] removes: rows of the copy whose id `old` has
/// retired, so `old` purged that capsule after the backup was taken. Matches
/// the `WHERE` clause of [`CARRY_PURGES`].
const COUNT_PURGES: &str = "SELECT count(*) FROM main.pastes WHERE id IN (SELECT id FROM old.retired_ids)";

/// The write of [`carry_purges`]: removes the rows [`COUNT_PURGES`] counts,
/// whatever their state. Purged is the latest state a capsule can reach, so a
/// restore only moves it forward. The copy's own `AFTER DELETE` trigger
/// retires the ids, and `secure_delete` (set by [`db::connect`]) zeroes the
/// freed pages, so the ciphertext does not reach the installed file.
const CARRY_PURGES: &str = "DELETE FROM main.pastes WHERE id IN (SELECT id FROM old.retired_ids)";

/// Removes the capsules of the copy that the attached `old` purged after the
/// backup, inside the caller's transaction (`Carry::Count`: only counts), see
/// [`CARRY_PURGES`]. Without it a restore would bring back a capsule burned or
/// deleted and then purged after the backup was taken, with its ciphertext.
/// `old.retired_ids` and `old.pastes` never share an id, so these rows are
/// never also changed by [`carry_states`].
async fn carry_purges(connection: &mut sqlx::SqliteConnection, carry: Carry) -> Result<i64, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(COUNT_PURGES).fetch_one(&mut *connection).await?;
    if carry == Carry::Insert {
        sqlx::query(CARRY_PURGES).execute(&mut *connection).await?;
    }
    Ok(count)
}

/// The capsules [`carry_states`] changes: rows the copy and `old` both hold
/// (same id) where `old` is in a later lifecycle state. Matches the `WHERE`
/// clauses of [`CARRY_STATES`].
const COUNT_STATES: &str = "SELECT count(*) FROM main.pastes AS m JOIN old.pastes AS o ON o.id = m.id \
     WHERE (o.consumed_at IS NOT NULL AND m.consumed_at IS NULL) \
     OR (o.availability_status = 'deleted' AND m.availability_status != 'deleted') \
     OR (o.availability_status = 'disabled' AND m.availability_status = 'active')";

/// The writes of [`carry_states`], in order. Each moves a capsule of the copy
/// only forward, to the state `old` holds it in, and only ever clears its
/// ciphertext, never copies one from `old`:
/// - consumed in `old` (burned after retrieval): `consumed_at` and
///   `last_retrieved_at` from `old`, ciphertext dropped;
/// - deleted in `old` (by its owner or by moderation): deleted, ciphertext
///   dropped, the moderation reason and time from `old`;
/// - disabled in `old` but active in the copy: disabled with `old`'s reason
///   and time.
///
/// A capsule `old` holds in an earlier state than the copy (re-enabled with
/// `moderation restore` after the backup) keeps the copy's state: the copy
/// never becomes more available than either database.
const CARRY_STATES: [&str; 3] = [
    "UPDATE main.pastes AS m SET consumed_at = o.consumed_at, \
     last_retrieved_at = coalesce(o.last_retrieved_at, m.last_retrieved_at), ciphertext = NULL \
     FROM old.pastes AS o WHERE o.id = m.id AND o.consumed_at IS NOT NULL AND m.consumed_at IS NULL",
    "UPDATE main.pastes AS m SET availability_status = 'deleted', ciphertext = NULL, \
     moderation_reason = o.moderation_reason, moderated_at = o.moderated_at \
     FROM old.pastes AS o WHERE o.id = m.id AND o.availability_status = 'deleted' \
     AND m.availability_status != 'deleted'",
    "UPDATE main.pastes AS m SET availability_status = 'disabled', \
     moderation_reason = o.moderation_reason, moderated_at = o.moderated_at \
     FROM old.pastes AS o WHERE o.id = m.id AND o.availability_status = 'disabled' \
     AND m.availability_status = 'active'",
];

/// Carries the later lifecycle state of the capsules both the copy and the
/// attached `old` hold into the copy, inside the caller's transaction
/// (`Carry::Count`: only counts), see [`CARRY_STATES`]. Without it a restore
/// would bring back a capsule burned, deleted or moderated after the backup
/// was taken, with its ciphertext.
async fn carry_states(connection: &mut sqlx::SqliteConnection, carry: Carry) -> Result<i64, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(COUNT_STATES).fetch_one(&mut *connection).await?;
    if carry == Carry::Insert {
        for statement in CARRY_STATES {
            sqlx::query(statement).execute(&mut *connection).await?;
        }
    }
    Ok(count)
}

/// Counts of [`carry_tokens`], each over the same rows its statement in
/// [`CARRY_TOKENS`] changes. Rows are matched by verifier, never by id: the
/// verifier is what authorizes a create, so it is the token's identity.
const COUNT_TOKENS: &str = "SELECT \
     (SELECT count(*) FROM old.creation_tokens o \
      WHERE o.verifier NOT IN (SELECT verifier FROM main.creation_tokens)) AS inserted, \
     (SELECT count(*) FROM old.creation_tokens o JOIN main.creation_tokens m ON m.verifier = o.verifier \
      WHERE o.revoked_at IS NOT NULL AND m.revoked_at IS NULL) AS revoked, \
     (SELECT count(*) FROM old.creation_tokens o JOIN main.creation_tokens m ON m.verifier = o.verifier \
      WHERE o.last_used_on IS NOT NULL AND (m.last_used_on IS NULL OR m.last_used_on < o.last_used_on)) AS last_used";

/// The writes of [`carry_tokens`], in order: revocations and later last-use
/// days onto the copy's rows, then the rows the copy lacks. An inserted row
/// whose id the copy already uses for another verifier fails on the primary
/// key, which stops the restore.
const CARRY_TOKENS: [&str; 3] = [
    "UPDATE main.creation_tokens AS m SET revoked_at = o.revoked_at FROM old.creation_tokens AS o \
     WHERE o.verifier = m.verifier AND o.revoked_at IS NOT NULL AND m.revoked_at IS NULL",
    "UPDATE main.creation_tokens AS m SET last_used_on = o.last_used_on FROM old.creation_tokens AS o \
     WHERE o.verifier = m.verifier AND o.last_used_on IS NOT NULL \
     AND (m.last_used_on IS NULL OR m.last_used_on < o.last_used_on)",
    "INSERT INTO main.creation_tokens (id, verifier, label, created_at, last_used_on, revoked_at) \
     SELECT id, verifier, label, created_at, last_used_on, revoked_at FROM old.creation_tokens \
     WHERE verifier NOT IN (SELECT verifier FROM main.creation_tokens)",
];

/// Carries the creation tokens of the attached `old` into the copy, matched by
/// verifier, inside the caller's transaction (`Carry::Count`: only counts):
/// a token the copy lacks is inserted with its id, label, dates and revocation;
/// a token revoked in `old` but active in the copy is revoked with `old`'s
/// `revoked_at`; `last_used_on` becomes the later of both (`NULL` is earlier).
/// Tokens only in the copy stay as they are. The verifiers are compared inside
/// SQLite: both sides are stored hashes, no presented token is involved.
async fn carry_tokens(connection: &mut sqlx::SqliteConnection, carry: Carry) -> Result<CarriedTokens, sqlx::Error> {
    let counts = sqlx::query(COUNT_TOKENS).fetch_one(&mut *connection).await?;
    let carried = CarriedTokens {
        inserted: counts.try_get("inserted")?,
        revoked: counts.try_get("revoked")?,
        last_used: counts.try_get("last_used")?,
    };
    if carry == Carry::Insert {
        for statement in CARRY_TOKENS {
            sqlx::query(statement).execute(&mut *connection).await?;
        }
    }
    Ok(carried)
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
    /// The ids, capsule states, purges and creation tokens of the replaced database are
    /// carried over and checkpointed.
    Carry,
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
