#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::rc::Rc;

use super::*;
use crate::config::Synchronous;
use crate::db::sidecars;
use crate::domain::{NewPaste, PasteId, Verifier};
use crate::repository::PasteRepository as _;
use crate::repository::sqlite::SqlitePasteRepository;

type Hook = Box<dyn FnMut(Step, &Path) -> std::io::Result<()>>;

thread_local! {
    /// The hook `step` runs. Per thread: a `#[tokio::test]` runs on one thread,
    /// so parallel tests do not see each other's hooks.
    static HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) };
}

/// Runs the hook set by [`on_step`], if any.
pub(super) fn run_hook(step: Step, path: &Path) -> std::io::Result<()> {
    let Some(mut hook) = HOOK.with(|cell| cell.borrow_mut().take()) else {
        return Ok(());
    };
    let result = hook(step, path);
    HOOK.with(|cell| *cell.borrow_mut() = Some(hook));
    result
}

/// Removes the hook when dropped.
struct HookGuard;

impl Drop for HookGuard {
    fn drop(&mut self) {
        HOOK.with(|cell| *cell.borrow_mut() = None);
    }
}

/// Runs `hook` after every step until the guard is dropped.
fn on_step(hook: impl FnMut(Step, &Path) -> std::io::Result<()> + 'static) -> HookGuard {
    HOOK.with(|cell| *cell.borrow_mut() = Some(Box::new(hook)));
    HookGuard
}

/// Fails `target` with an error of `kind`; the returned flag says whether it fired.
fn fail_at(target: Step, kind: ErrorKind) -> (HookGuard, Rc<RefCell<bool>>) {
    let fired = Rc::new(RefCell::new(false));
    let seen = fired.clone();
    let guard = on_step(move |step, _| {
        if step != target {
            return Ok(());
        }
        *seen.borrow_mut() = true;
        Err(std::io::Error::new(kind, format!("injected failure at {step:?}")))
    });
    (guard, fired)
}

thread_local! {
    /// The `max_page_count` [`carry_page_limit`] hands to `carry_attached`.
    static CARRY_PAGE_LIMIT: RefCell<Option<u32>> = const { RefCell::new(None) };
}

/// The `max_page_count` the carry connection sets on the staged copy after
/// creating the scratch table, if a test asked for one with [`limit_carry_pages`].
pub(super) fn carry_page_limit() -> Option<u32> {
    CARRY_PAGE_LIMIT.with(|cell| *cell.borrow())
}

/// Removes the page limit when dropped.
struct PageLimitGuard;

impl Drop for PageLimitGuard {
    fn drop(&mut self) {
        CARRY_PAGE_LIMIT.with(|cell| *cell.borrow_mut() = None);
    }
}

/// Lets the staged copy grow by no page once the scratch table exists (SQLite
/// raises a smaller `max_page_count` to the current size), so the scratch
/// insert fails writing the copy with `SQLITE_FULL`.
fn limit_carry_pages() -> PageLimitGuard {
    CARRY_PAGE_LIMIT.with(|cell| *cell.borrow_mut() = Some(1));
    PageLimitGuard
}

/// The names in `dir`.
fn entries(dir: &Path) -> BTreeSet<OsString> {
    std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name()).collect()
}

/// The contents of each path, `None` for a missing one.
fn contents(paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
    paths.iter().map(|path| std::fs::read(path).ok()).collect()
}

#[cfg(unix)]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}

fn with_sidecars(path: &Path) -> Vec<PathBuf> {
    let [wal, shm] = sidecars(path);
    vec![path.to_path_buf(), wal, shm]
}

/// A database at `path` with its `-wal` and `-shm`, each holding a marker.
fn old_database(path: &Path) -> Vec<PathBuf> {
    let files = with_sidecars(path);
    for (file, content) in files.iter().zip([&b"old database"[..], b"old wal", b"old shm"]) {
        std::fs::write(file, content).unwrap();
    }
    files
}

/// A database at `path` migrated only up to `version`, with a paste row for
/// each of `ids`, closed without sidecars.
async fn database_at(path: &Path, version: i64, ids: &[&str]) {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(path).create_if_missing(true))
        .await
        .unwrap();
    db::MIGRATOR.run_to(version, &pool).await.unwrap();
    for id in ids {
        sqlx::query(
            "INSERT INTO pastes (id, format_version, public_header, reveal_verifier, delete_verifier, created_at, payload_size)
             VALUES (?1, 1, x'00', x'00', x'00', 1, 1)",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;
    assert!(!sidecars(path)[0].exists());
}

/// A backup at `path` that has only the first migration and one paste row.
async fn first_migration_backup(path: &Path) {
    database_at(path, 1, &["first-migration-paste"]).await;
}

/// Copies the files of the open database `from` (main file, `-wal`, `-shm`) to
/// `to`, as a crash would leave them.
fn copy_open_database(from: &Path, to: &Path) {
    for (from, to) in with_sidecars(from).iter().zip(with_sidecars(to)) {
        std::fs::copy(from, to).unwrap();
    }
}

/// The ids in `table` of the database at `path`.
async fn ids_in(path: &Path, table: &str) -> BTreeSet<String> {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(path).read_only(true))
        .await
        .unwrap();
    let ids = sqlx::query_scalar(AssertSqlSafe(format!("SELECT id FROM {table}")))
        .fetch_all(&pool)
        .await
        .unwrap();
    pool.close().await;
    ids.into_iter().collect()
}

/// The schema of the database at `path`: every entry of `sqlite_master`.
async fn schema_of(path: &Path) -> BTreeSet<(String, String, Option<String>)> {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(path).read_only(true))
        .await
        .unwrap();
    let schema = sqlx::query_as("SELECT type, name, sql FROM sqlite_master")
        .fetch_all(&pool)
        .await
        .unwrap();
    pool.close().await;
    schema.into_iter().collect()
}

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("capsulebin-backup-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(path: PathBuf) -> DatabaseConfig {
    DatabaseConfig {
        path,
        busy_timeout_ms: 5_000,
        synchronous: Synchronous::Full,
    }
}

fn paste(byte: u8, ciphertext: &[u8]) -> NewPaste {
    NewPaste {
        id: PasteId::from_bytes([byte; 16]),
        format_version: 1,
        public_header: vec![0xAA],
        ciphertext: ciphertext.to_vec(),
        reveal_verifier: Verifier::for_reveal_token(b"reveal"),
        delete_verifier: Verifier::for_delete_secret(b"delete"),
        describe_verifier: Verifier::for_describe_token(b"describe"),
        created_at: 1_000,
        expires_at: None,
        burn_after_retrieval: false,
    }
}

async fn database_with_pastes(path: PathBuf) -> SqlitePool {
    let pool = db::connect(&config(path)).await.unwrap();
    let repo = SqlitePasteRepository::new(pool.clone());
    repo.create(&paste(1, b"kept ciphertext")).await.unwrap();
    repo.create(&paste(2, b"DELETED-CIPHERTEXT-MARKER")).await.unwrap();
    repo.delete(&PasteId::from_bytes([2; 16])).await.unwrap();
    pool
}

#[tokio::test]
async fn backs_up_a_live_database_and_verifies_it() {
    let dir = dir("create");
    let pool = database_with_pastes(dir.join("live.db")).await;
    let output = dir.join("backup.db");

    let info = create(&pool, &output).await.unwrap();
    assert_eq!(info.schema_version, db::expected_version());
    assert_eq!(info.pastes, 2, "the deleted paste's tombstone is part of the data");
    assert_eq!(info.reports, 0);

    // Not a WAL database, no sidecars to forget.
    for sidecar in sidecars(&output) {
        assert!(!sidecar.exists(), "{} next to the backup", sidecar.display());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&output).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "backup is readable by others");
    }

    // VACUUM INTO compacts: the payload of the deleted paste is not in the file.
    let bytes = std::fs::read(&output).unwrap();
    let marker = b"DELETED-CIPHERTEXT-MARKER";
    assert!(!bytes.windows(marker.len()).any(|w| w == marker), "freed pages were copied");
}

#[tokio::test]
async fn verify_rejects_garbage_and_foreign_databases() {
    let dir = dir("verify");

    std::fs::write(dir.join("garbage.db"), b"this is not a database at all, not even close").unwrap();
    assert!(verify_file(&dir.join("garbage.db")).await.is_err());

    let foreign = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(dir.join("foreign.db")).create_if_missing(true))
        .await
        .unwrap();
    sqlx::query("CREATE TABLE notes (x TEXT)").execute(&foreign).await.unwrap();
    foreign.close().await;
    let error = verify_file(&dir.join("foreign.db")).await.unwrap_err();
    assert!(matches!(error, BackupError::NotCapsuleBin), "{error}");

    let error = verify_file(&dir.join("absent.db")).await.unwrap_err();
    assert!(matches!(error, BackupError::Missing(_)), "{error}");
}

#[tokio::test]
async fn verify_rejects_a_newer_schema() {
    let dir = dir("too-new");
    let pool = database_with_pastes(dir.join("live.db")).await;
    let output = dir.join("backup.db");
    create(&pool, &output).await.unwrap();

    let writable = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(&output))
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time)
         VALUES (999999, 'future', CURRENT_TIMESTAMP, 1, x'00', 0)",
    )
    .execute(&writable)
    .await
    .unwrap();
    writable.close().await;

    let error = verify_file(&output).await.unwrap_err();
    assert!(matches!(error, BackupError::TooNew { found: 999_999, .. }), "{error}");
}

#[cfg(unix)]
#[tokio::test]
async fn backup_stages_privately_and_cleans_up_on_failure() {
    let dir = dir("backup-stage");
    let pool = database_with_pastes(dir.join("live.db")).await;
    let out = dir.join("out");
    std::fs::create_dir(&out).unwrap();
    let output = out.join("backup.db");

    let observed = Rc::new(RefCell::new(false));
    {
        let (stage_parent, published, observed) = (out.clone(), output.clone(), observed.clone());
        let _hook = on_step(move |step, path| {
            if step != Step::BackupWritten {
                return Ok(());
            }
            let stage = path.parent().unwrap();
            assert_eq!(stage.parent().unwrap(), stage_parent, "the stage is not next to the output");
            assert_eq!(mode(stage), 0o700, "the stage is reachable by others");
            assert_eq!(mode(path), 0o600, "the snapshot is readable by others");
            assert!(!occupied(&published), "published before it was verified");
            *observed.borrow_mut() = true;
            Err(std::io::Error::other("injected"))
        });
        create(&pool, &output).await.unwrap_err();
    }
    assert!(*observed.borrow(), "the hook never saw the snapshot");
    assert!(entries(&out).is_empty(), "left behind: {:?}", entries(&out));

    create(&pool, &output).await.unwrap();
    assert_eq!(entries(&out), BTreeSet::from([OsString::from("backup.db")]));
    assert_eq!(mode(&output), 0o600);
}

#[cfg(unix)]
#[tokio::test]
async fn backup_never_replaces_or_follows_an_existing_entry() {
    let dir = dir("backup-existing");
    let pool = database_with_pastes(dir.join("live.db")).await;
    let sentinel = dir.join("sentinel");
    std::fs::write(&sentinel, b"precious").unwrap();
    let dangling = dir.join("never-created");

    type Plant<'a> = &'a dyn Fn(&Path);
    let cases: [(&str, Plant); 4] = [
        ("file", &|path| std::fs::write(path, b"precious").unwrap()),
        ("directory", &|path| std::fs::create_dir(path).unwrap()),
        ("symlink", &|path| std::os::unix::fs::symlink(&sentinel, path).unwrap()),
        ("dangling", &|path| std::os::unix::fs::symlink(&dangling, path).unwrap()),
    ];
    for (name, plant) in cases {
        let out = dir.join(name);
        std::fs::create_dir(&out).unwrap();
        let output = out.join("backup.db");
        plant(&output);
        let before = std::fs::symlink_metadata(&output).unwrap();

        let error = create(&pool, &output).await.unwrap_err();
        assert!(matches!(error, BackupError::Exists(ref path) if path == &output), "{name}: {error}");
        assert!(error.to_string().contains("already exists"), "{name}: {error}");

        let after = std::fs::symlink_metadata(&output).unwrap();
        assert_eq!(before.file_type(), after.file_type(), "{name}");
        assert_eq!(before.len(), after.len(), "{name}");
        assert_eq!(entries(&out), BTreeSet::from([OsString::from("backup.db")]), "{name}: stage left");
    }
    assert_eq!(std::fs::read(dir.join("file/backup.db")).unwrap(), b"precious");
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"precious");
    assert!(!occupied(&dangling), "the dangling symlink was followed");

    // An entry that appears while the snapshot is written is not replaced either.
    let out = dir.join("late");
    std::fs::create_dir(&out).unwrap();
    let output = out.join("backup.db");
    {
        let (planted, dangling) = (output.clone(), dangling.clone());
        let _hook = on_step(move |step, _| {
            if step == Step::BackupWritten {
                std::os::unix::fs::symlink(&dangling, &planted)?;
            }
            Ok(())
        });
        let error = create(&pool, &output).await.unwrap_err();
        assert!(matches!(error, BackupError::Exists(ref path) if path == &output), "late: {error}");
    }
    assert_eq!(std::fs::read_link(&output).unwrap(), dangling);
    assert_eq!(entries(&out), BTreeSet::from([OsString::from("backup.db")]), "late: stage left");
    assert!(!occupied(&dangling), "the late dangling symlink was followed");
}

#[tokio::test]
async fn restores_into_an_empty_place() {
    let dir = dir("restore-empty");
    let pool = database_with_pastes(dir.join("live.db")).await;
    create(&pool, &dir.join("backup.db")).await.unwrap();

    let target = config(dir.join("restored.db"));
    let outcome = restore(&dir.join("backup.db"), &target, false, 5_000).await.unwrap();
    assert_eq!(outcome.previous, None);
    assert_eq!(outcome.info.pastes, 2);
    assert_eq!(outcome.schema_version, db::expected_version());
    assert_eq!(outcome.carried_ids, 0, "nothing to carry over");
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);

    let reopened = db::connect(&target).await.unwrap();
    assert_eq!(db::verify(&reopened).await.unwrap(), db::expected_version());
    let repo = SqlitePasteRepository::new(reopened);
    let kept = repo.find(&PasteId::from_bytes([1; 16])).await.unwrap().unwrap();
    assert_eq!(kept.ciphertext.as_deref(), Some(&b"kept ciphertext"[..]));
}

#[cfg(unix)]
#[tokio::test]
async fn restore_migrates_a_private_copy_before_the_cutover() {
    let dir = dir("restore-migrate");
    let backup = dir.join("backup.db");
    first_migration_backup(&backup).await;
    let input = std::fs::read(&backup).unwrap();
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    database_with_pastes(target.path.clone()).await.close().await;

    let seen = Rc::new(RefCell::new(Vec::new()));
    let outcome = {
        let seen = seen.clone();
        let _hook = on_step(move |step, path| {
            seen.borrow_mut().push((step, path.to_path_buf()));
            Ok(())
        });
        restore(&backup, &target, true, 5_000).await.unwrap()
    };

    let stage_prefix = db::restore_stage_prefix(&target.path);
    for (step, path) in seen
        .borrow()
        .iter()
        .filter(|(step, _)| matches!(step, Step::Verify | Step::Migrate))
    {
        let stage = path.parent().unwrap();
        assert_eq!(stage.parent().unwrap(), data, "{step:?} not next to the target");
        let name = stage.file_name().unwrap().as_encoded_bytes();
        assert!(name.starts_with(stage_prefix.as_encoded_bytes()), "{step:?} on {}", path.display());
    }
    assert_eq!(seen.borrow().iter().filter(|(step, _)| *step == Step::Migrate).count(), 1);
    assert_eq!(std::fs::read(&backup).unwrap(), input, "the input was changed");

    assert_eq!(outcome.info.schema_version, 1);
    assert_eq!(outcome.schema_version, db::expected_version());
    assert_eq!(mode(&target.path), 0o600);
    let wal = &sidecars(&target.path)[0];
    assert!(
        !wal.exists() || std::fs::metadata(wal).unwrap().len() == 0,
        "the restored file needs its WAL"
    );
    assert!(
        entries(&data)
            .iter()
            .all(|name| !name.as_encoded_bytes().starts_with(stage_prefix.as_encoded_bytes())),
        "stage left: {:?}",
        entries(&data)
    );

    let restored = verify_file(&target.path).await.unwrap();
    assert_eq!(restored.schema_version, db::expected_version());
    assert_eq!(restored.pastes, 1, "not the backup's rows");
}

#[tokio::test]
async fn restore_failure_at_every_step_keeps_the_old_database() {
    let dir = dir("restore-fail");
    let backup = dir.join("backup.db");
    first_migration_backup(&backup).await;

    // Garbage, and a real database left by a crash (rows in its WAL) whose
    // ids are carried over.
    let garbage = dir.join("garbage");
    std::fs::create_dir(&garbage).unwrap();
    old_database(&garbage.join("live.db"));
    let crashed = dir.join("crashed");
    std::fs::create_dir(&crashed).unwrap();
    let source = dir.join("source.db");
    let pool = db::connect(&config(source.clone())).await.unwrap();
    let repo = SqlitePasteRepository::new(pool.clone());
    repo.create(&paste(1, b"in the main file")).await.unwrap();
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&pool).await.unwrap();
    repo.create(&paste(2, b"only in the wal")).await.unwrap();
    copy_open_database(&source, &crashed.join("live.db"));
    pool.close().await;

    for data in [garbage, crashed] {
        let target = config(data.join("live.db"));
        let files = with_sidecars(&target.path);
        let before = contents(&files);
        let listed = entries(&data);

        for (step, kind) in [
            (Step::Copy, ErrorKind::Other),
            (Step::Copy, ErrorKind::StorageFull),
            (Step::Verify, ErrorKind::Other),
            (Step::Migrate, ErrorKind::Other),
            (Step::Checkpoint, ErrorKind::Other),
            (Step::Carry, ErrorKind::Other),
            (Step::MoveDatabase, ErrorKind::Other),
            (Step::MoveWal, ErrorKind::Other),
            (Step::MoveShm, ErrorKind::Other),
            (Step::Install, ErrorKind::Other),
        ] {
            let (hook, fired) = fail_at(step, kind);
            let error = restore(&backup, &target, true, 5_000).await.unwrap_err();
            drop(hook);

            assert!(*fired.borrow(), "{step:?} was never reached: {error}");
            assert!(
                matches!(&error, BackupError::Io { source, .. } if source.kind() == kind),
                "{step:?}: {error}"
            );
            assert_eq!(contents(&files), before, "{step:?} changed the old database");
            assert_eq!(entries(&data), listed, "{step:?} left entries behind");
        }
    }
}

#[tokio::test]
async fn replacing_moves_the_old_database_and_its_wal_aside() {
    let dir = dir("restore-replace");
    let backup_source = database_with_pastes(dir.join("source.db")).await;
    create(&backup_source, &dir.join("backup.db")).await.unwrap();

    let target = config(dir.join("live.db"));
    let files = old_database(&target.path);
    let before = contents(&files);

    let error = restore(&dir.join("backup.db"), &target, false, 5_000).await.unwrap_err();
    assert!(matches!(error, BackupError::Exists(_)), "{error}");
    assert_eq!(contents(&files), before, "touched without --force");

    let outcome = restore(&dir.join("backup.db"), &target, true, 5_000).await.unwrap();
    let previous = outcome.previous.unwrap();
    assert_eq!(previous, dir.join("live.db.pre-restore-5000"));
    assert_eq!(contents(&with_sidecars(&previous)), before);
    for sidecar in sidecars(&target.path) {
        assert!(!sidecar.exists(), "{} would be replayed onto the restored file", sidecar.display());
    }
    assert_eq!(verify_file(&target.path).await.unwrap().pastes, 2);
}

#[tokio::test]
async fn restore_refuses_an_existing_aside_name() {
    let dir = dir("restore-aside-taken");
    let source = database_with_pastes(dir.join("source.db")).await;
    let backup = dir.join("backup.db");
    create(&source, &backup).await.unwrap();
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    let files = old_database(&target.path);

    for taken in with_sidecars(&data.join("live.db.pre-restore-5000")) {
        std::fs::write(&taken, b"earlier").unwrap();
        let before = contents(&files);
        let listed = entries(&data);

        let steps = Rc::new(RefCell::new(Vec::new()));
        let error = {
            let steps = steps.clone();
            let _hook = on_step(move |step, _| {
                steps.borrow_mut().push(step);
                Ok(())
            });
            restore(&backup, &target, true, 5_000).await.unwrap_err()
        };
        assert!(matches!(&error, BackupError::Exists(path) if path == &taken), "{error}");
        assert!(steps.borrow().is_empty(), "went on to {:?}", steps.borrow());
        assert_eq!(contents(&files), before);
        assert_eq!(std::fs::read(&taken).unwrap(), b"earlier");
        assert_eq!(entries(&data), listed);
        std::fs::remove_file(&taken).unwrap();
    }
}

#[tokio::test]
async fn restore_recovers_next_to_an_old_aside() {
    let dir = dir("restore-recover");
    let source = database_with_pastes(dir.join("source.db")).await;
    let backup = dir.join("backup.db");
    create(&source, &backup).await.unwrap();
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    let old_aside = data.join("live.db.pre-restore-1000");
    std::fs::write(&old_aside, b"interrupted earlier").unwrap();
    assert!(db::connect(&target).await.is_err(), "the guard should refuse first");

    let outcome = restore(&backup, &target, false, 5_000).await.unwrap();
    assert_eq!(outcome.previous, None);
    assert_eq!(std::fs::read(&old_aside).unwrap(), b"interrupted earlier");
    let pool = db::connect(&target).await.unwrap();
    assert_eq!(db::verify(&pool).await.unwrap(), db::expected_version());
    pool.close().await;
}

#[tokio::test]
async fn restore_ignores_a_valid_sibling_wal() {
    let dir = dir("restore-sibling-wal");
    let live = dir.join("live.db");
    let pool = db::connect(&config(live.clone())).await.unwrap();
    let repo = SqlitePasteRepository::new(pool.clone());
    repo.create(&paste(1, b"in the main file")).await.unwrap();
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&pool).await.unwrap();
    repo.create(&paste(2, b"only in the wal")).await.unwrap();

    // A copy of the open database: main file, WAL with the second row, SHM.
    let backup = dir.join("backup.db");
    copy_open_database(&live, &backup);
    pool.close().await;
    let check = dir.join("check.db");
    std::fs::copy(&backup, &check).unwrap();
    std::fs::copy(&sidecars(&backup)[0], &sidecars(&check)[0]).unwrap();
    assert_eq!(verify_file(&check).await.unwrap().pastes, 2, "the WAL does not hold the second row");

    let backup_files = with_sidecars(&backup);
    let before = contents(&backup_files);
    let target = config(dir.join("data").join("restored.db"));
    std::fs::create_dir(target.directory()).unwrap();

    let dry_run = restore_dry_run(&backup, &target, false, 5_000).await.unwrap();
    assert_eq!(dry_run.info.pastes, 1);
    let outcome = restore(&backup, &target, false, 5_000).await.unwrap();
    assert_eq!(outcome.info.pastes, 1);
    assert_eq!(verify_file(&target.path).await.unwrap().pastes, 1);
    assert_eq!(contents(&backup_files), before, "the backup's files were changed");
}

#[cfg(unix)]
#[tokio::test]
async fn restore_follows_no_planted_scratch_entry() {
    let dir = dir("restore-planted");
    let source = database_with_pastes(dir.join("source.db")).await;
    let backup = dir.join("backup.db");
    create(&source, &backup).await.unwrap();
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    old_database(&target.path);
    let sentinel = dir.join("sentinel");
    std::fs::write(&sentinel, b"sentinel").unwrap();
    let planted = data.join("live.db.restoring");
    std::os::unix::fs::symlink(&sentinel, &planted).unwrap();

    restore(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"sentinel");
    assert!(std::fs::symlink_metadata(&planted).unwrap().file_type().is_symlink());
    assert_eq!(verify_file(&target.path).await.unwrap().pastes, 2);
}

#[tokio::test]
async fn dry_run_migrates_a_private_copy_and_changes_nothing() {
    let dir = dir("restore-dry-run");
    let backup = dir.join("backup.db");
    first_migration_backup(&backup).await;
    let input = std::fs::read(&backup).unwrap();
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    let files = old_database(&target.path);
    let before = contents(&files);
    let listed = entries(&data);

    let outcome = restore_dry_run(&backup, &target, false, 5_000).await.unwrap();
    assert_eq!(outcome.info.schema_version, 1);
    assert_eq!(outcome.info.pastes, 1);
    assert_eq!(outcome.schema_version, db::expected_version());
    assert!(outcome.needs_replace);
    assert_eq!(std::fs::read(&backup).unwrap(), input);
    assert_eq!(contents(&files), before);
    assert_eq!(entries(&data), listed);

    let (hook, fired) = fail_at(Step::Migrate, ErrorKind::Other);
    restore_dry_run(&backup, &target, true, 5_000).await.unwrap_err();
    drop(hook);
    assert!(*fired.borrow());
    assert_eq!(std::fs::read(&backup).unwrap(), input);
    assert_eq!(contents(&files), before);
    assert_eq!(entries(&data), listed);
}

#[tokio::test]
async fn refuses_to_restore_a_file_onto_itself() {
    let dir = dir("restore-self");
    let pool = database_with_pastes(dir.join("live.db")).await;
    create(&pool, &dir.join("backup.db")).await.unwrap();

    let error = restore(&dir.join("backup.db"), &config(dir.join("backup.db")), true, 1)
        .await
        .unwrap_err();
    assert!(matches!(error, BackupError::SameFile), "{error}");
}

/// AC17: the published output is the verified staged snapshot itself, not a
/// copy of it, and no second name for it is left behind.
#[cfg(unix)]
#[tokio::test]
async fn backup_publishes_the_staged_snapshot_itself() {
    use std::os::unix::fs::MetadataExt as _;

    let dir = dir("backup-rename");
    let pool = database_with_pastes(dir.join("live.db")).await;
    let out = dir.join("out");
    std::fs::create_dir(&out).unwrap();
    let output = out.join("backup.db");

    let staged_inode = Rc::new(RefCell::new(None));
    {
        let staged_inode = staged_inode.clone();
        let _hook = on_step(move |step, path| {
            if step == Step::BackupWritten {
                *staged_inode.borrow_mut() = Some(std::fs::metadata(path)?.ino());
            }
            Ok(())
        });
        create(&pool, &output).await.unwrap();
    }
    // The output carries the staged file's inode, so it is not a copy (a copy
    // gets a new inode). A link count of 1 means no second name for it is left
    // behind. Whether a hard link was used on the way is not observable here:
    // the stage is gone once `create` returns, so a hard link followed by
    // removing the stage looks the same as a rename (telling them apart would
    // need a filesystem without hard links).
    let published = std::fs::symlink_metadata(&output).unwrap();
    assert!(published.file_type().is_file());
    assert_eq!(Some(published.ino()), *staged_inode.borrow(), "the output is not the staged file");
    assert_eq!(published.nlink(), 1, "the output has another name left");
    assert_eq!(mode(&output), 0o600);
    assert_eq!(entries(&out), BTreeSet::from([OsString::from("backup.db")]));
    verify_file(&output).await.unwrap();
}

/// AC7: ids of the replaced database that the backup does not hold as a row
/// are retired by the restore, read from its rows, its `retired_ids` and its
/// WAL, also from a database at schema version 3 (no `retired_ids`).
#[tokio::test]
async fn restore_retires_ids_missing_from_the_backup() {
    let dir = dir("restore-carry");
    let (a, b, c) = (
        PasteId::from_bytes([1; 16]),
        PasteId::from_bytes([2; 16]),
        PasteId::from_bytes([3; 16]),
    );

    // A before the backup; B and C after it, C purged (retired). B and C are
    // only in the WAL of the database a crash left behind.
    let source = dir.join("source.db");
    let pool = db::connect(&config(source.clone())).await.unwrap();
    let repo = SqlitePasteRepository::new(pool.clone());
    repo.create(&paste(1, b"a")).await.unwrap();
    let backup = dir.join("backup.db");
    create(&pool, &backup).await.unwrap();
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&pool).await.unwrap();
    repo.create(&paste(2, b"b")).await.unwrap();
    repo.create(&NewPaste {
        expires_at: Some(1_500),
        ..paste(3, b"c")
    })
    .await
    .unwrap();
    let policy = crate::domain::PurgePolicy {
        never_retrieved_after: 86_400,
        inactive_after: 86_400,
    };
    assert_eq!(repo.purge(2_000, &policy, 100).await.unwrap().expired, 1);

    // `?`, `#` and `%` in the path must stay part of the attached file's name.
    let data = dir.join("data ?#%");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    copy_open_database(&source, &target.path);
    pool.close().await;
    let files = with_sidecars(&target.path);
    assert!(contents(&files).iter().all(Option::is_some), "not a crashed database");
    let before = contents(&files);
    let listed = entries(&data);

    let dry_run = restore_dry_run(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(dry_run.carried_ids, 2);
    assert!(dry_run.warnings.is_empty(), "{:?}", dry_run.warnings);
    assert_eq!(contents(&files), before, "the dry run changed the old database");
    assert_eq!(entries(&data), listed, "the dry run left an entry");

    let outcome = restore(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(outcome.carried_ids, 2);
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(contents(&with_sidecars(&outcome.previous.unwrap())), before);

    assert_eq!(ids_in(&target.path, "pastes").await, BTreeSet::from([a.encode()]));
    assert_eq!(ids_in(&target.path, "retired_ids").await, BTreeSet::from([b.encode(), c.encode()]));
    let repo = SqlitePasteRepository::new(db::connect(&target).await.unwrap());
    for byte in [2, 3] {
        let error = repo.create(&paste(byte, b"again")).await.unwrap_err();
        assert!(matches!(error, crate::repository::RepositoryError::IdConflict), "{byte}: {error}");
    }

    // A replaced database at schema version 3 has no `retired_ids`: its rows
    // are carried over.
    let data = dir.join("version-3");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    database_at(&target.path, 3, &[&a.encode(), "only-in-version-3"]).await;
    let outcome = restore(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(outcome.carried_ids, 1);
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(ids_in(&target.path, "pastes").await, BTreeSet::from([a.encode()]));
    assert_eq!(
        ids_in(&target.path, "retired_ids").await,
        BTreeSet::from(["only-in-version-3".to_owned()])
    );
}

/// AC8: a replaced database that cannot be read does not stop the restore; the
/// outcome warns that its ids were not carried over, and it is kept unchanged.
/// The same for a database whose `-wal` could only be read by creating a
/// `-shm`, and for one whose schema can be read but whose tables cannot.
/// (`restore_acceptance.rs` checks the `warning:` line on stderr.)
#[tokio::test]
async fn restore_with_an_unreadable_old_database_warns() {
    let dir = dir("restore-unreadable");
    let source = database_with_pastes(dir.join("source.db")).await;
    let backup = dir.join("backup.db");
    create(&source, &backup).await.unwrap();
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&source).await.unwrap();
    SqlitePasteRepository::new(source.clone())
        .create(&paste(9, b"only in the wal"))
        .await
        .unwrap();

    let garbage = dir.join("garbage");
    std::fs::create_dir(&garbage).unwrap();
    std::fs::write(garbage.join("live.db"), b"this is not a database at all, not even close").unwrap();
    let no_shm = dir.join("no-shm");
    std::fs::create_dir(&no_shm).unwrap();
    copy_open_database(&dir.join("source.db"), &no_shm.join("live.db"));
    std::fs::remove_file(&sidecars(&no_shm.join("live.db"))[1]).unwrap();
    source.close().await;
    // A readable schema over unreadable tables: ATTACH and the schema query
    // succeed, reading the ids fails.
    let corrupt = dir.join("corrupt");
    std::fs::create_dir(&corrupt).unwrap();
    database_at(&corrupt.join("live.db"), 4, &["lost-1", "lost-2"]).await;
    let mut bytes = std::fs::read(corrupt.join("live.db")).unwrap();
    let page_size = usize::from(u16::from_be_bytes([bytes[16], bytes[17]]));
    assert!(bytes.len() > page_size, "the tables live behind the schema page");
    bytes[page_size..].fill(0xFF);
    std::fs::write(corrupt.join("live.db"), bytes).unwrap();

    for data in [garbage, no_shm, corrupt] {
        let target = config(data.join("live.db"));
        let files = with_sidecars(&target.path);
        let before = contents(&files);
        let listed = entries(&data);

        let dry_run = restore_dry_run(&backup, &target, true, 5_000).await.unwrap();
        assert_eq!(dry_run.carried_ids, 0);
        assert_eq!(dry_run.warnings.len(), 1, "{data:?}: {:?}", dry_run.warnings);
        assert_eq!(contents(&files), before, "{data:?}: the dry run changed the old database");
        assert_eq!(entries(&data), listed, "{data:?}: the dry run left an entry");

        let outcome = restore(&backup, &target, true, 5_000).await.unwrap();
        assert_eq!(outcome.carried_ids, 0);
        assert_eq!(outcome.warnings, dry_run.warnings);
        assert!(
            outcome.warnings[0].contains("could not be carried over"),
            "{data:?}: {:?}",
            outcome.warnings
        );
        assert!(!outcome.warnings[0].contains("not even close"), "file content in the warning");
        assert!(outcome.warnings[0].contains("creation tokens"), "{data:?}: {:?}", outcome.warnings);
        let previous = outcome.previous.unwrap();
        assert_eq!(previous, data.join("live.db.pre-restore-5000"));
        assert_eq!(contents(&with_sidecars(&previous)), before, "{data:?}: the old database changed");
        assert_eq!(
            verify_file(&target.path).await.unwrap().pastes,
            2,
            "{data:?}: not the backup's rows"
        );
        assert!(ids_in(&target.path, "retired_ids").await.is_empty());
    }
}

/// AC12: ids are carried over inside SQLite, also many of them.
#[tokio::test]
async fn restore_carries_many_ids_over() {
    let dir = dir("restore-many");
    let source = database_with_pastes(dir.join("source.db")).await;
    let backup = dir.join("backup.db");
    create(&source, &backup).await.unwrap();
    source.close().await;

    // 60 000 rows and 40 000 retired ids, none of them in the backup.
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    let pool = db::connect(&target).await.unwrap();
    let mut transaction = pool.begin().await.unwrap();
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 60000)
         INSERT INTO pastes (id, format_version, public_header, reveal_verifier, delete_verifier, created_at, payload_size)
         SELECT 'row-' || i, 1, x'00', x'00', x'00', 1, 1 FROM n",
    )
    .execute(&mut *transaction)
    .await
    .unwrap();
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 40000)
         INSERT INTO retired_ids (id) SELECT 'retired-' || i FROM n",
    )
    .execute(&mut *transaction)
    .await
    .unwrap();
    transaction.commit().await.unwrap();
    pool.close().await;

    let dry_run = restore_dry_run(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(dry_run.carried_ids, 100_000);
    assert!(dry_run.warnings.is_empty(), "{:?}", dry_run.warnings);

    let outcome = restore(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(outcome.carried_ids, 100_000);
    let retired = ids_in(&target.path, "retired_ids").await;
    assert_eq!(retired.len(), 100_000);
    assert!(retired.contains("row-60000") && retired.contains("retired-40000"));
    assert_eq!(verify_file(&target.path).await.unwrap().pastes, 2, "not the backup's rows");

    // The scratch table in the staged copy does not reach the installed
    // database: its schema is that of a freshly migrated one.
    let schema = schema_of(&target.path).await;
    assert!(
        !schema.iter().any(|(_, name, _)| name.contains("restore_carried_ids")),
        "{schema:?}"
    );
    let fresh = dir.join("fresh.db");
    db::connect(&config(fresh.clone())).await.unwrap().close().await;
    assert_eq!(schema, schema_of(&fresh).await);
}

/// A write failure on the staged copy while the ids are carried over is an
/// error, not a warning: the restore and the dry run stop before the cutover,
/// and the old database, its sidecars and its directory stay as they were.
#[tokio::test]
async fn restore_stops_when_the_copy_cannot_take_the_carried_ids() {
    let dir = dir("restore-carry-full");
    let source = database_with_pastes(dir.join("source.db")).await;
    let backup = dir.join("backup.db");
    create(&source, &backup).await.unwrap();
    source.close().await;

    // 10 000 rows, none of them in the backup: far more than the scratch
    // table's first page holds.
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    let pool = db::connect(&target).await.unwrap();
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 10000)
         INSERT INTO pastes (id, format_version, public_header, reveal_verifier, delete_verifier, created_at, payload_size)
         SELECT 'row-' || i, 1, x'00', x'00', x'00', 1, 1 FROM n",
    )
    .execute(&pool)
    .await
    .unwrap();
    // Keep a `-wal` and `-shm` next to it, as a stopped server may leave them.
    let copy = dir.join("copy.db");
    copy_open_database(&target.path, &copy);
    pool.close().await;
    for (from, to) in with_sidecars(&copy).iter().zip(with_sidecars(&target.path)) {
        std::fs::rename(from, to).unwrap();
    }
    let files = with_sidecars(&target.path);
    assert!(contents(&files).iter().all(Option::is_some), "no sidecars");
    let before = contents(&files);
    let listed = entries(&data);

    let is_full = |error: &BackupError| matches!(error, BackupError::Database(sqlx::Error::Database(source)) if source.code().as_deref() == Some("13"));

    let limit = limit_carry_pages();
    let error = restore_dry_run(&backup, &target, true, 5_000).await.unwrap_err();
    assert!(is_full(&error), "{error:?}");
    assert_eq!(contents(&files), before, "the dry run changed the old database");
    assert_eq!(entries(&data), listed, "the dry run left an entry");

    let error = restore(&backup, &target, true, 5_000).await.unwrap_err();
    assert!(is_full(&error), "{error:?}");
    assert_eq!(contents(&files), before, "the restore changed the old database");
    assert_eq!(entries(&data), listed, "the restore left an entry");
    drop(limit);

    // Without the limit the same restore carries the ids over.
    let outcome = restore(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(outcome.carried_ids, 10_000);
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(contents(&with_sidecars(&outcome.previous.unwrap())), before);
}

/// A creation token row: id, verifier (32 times this byte), label, created_at,
/// last_used_on, revoked_at.
type TokenRow = (&'static str, u8, &'static str, i64, Option<i64>, Option<i64>);

/// A [`TokenRow`] as read back by [`tokens_in`].
type StoredToken = (String, u8, String, i64, Option<i64>, Option<i64>);

/// Inserts `rows` into `creation_tokens` of the database at `path`.
async fn add_tokens(path: &Path, rows: &[TokenRow]) {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(path))
        .await
        .unwrap();
    for (id, verifier, label, created_at, last_used_on, revoked_at) in rows {
        sqlx::query(
            "INSERT INTO creation_tokens (id, verifier, label, created_at, last_used_on, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .bind(id)
        .bind(vec![*verifier; 32])
        .bind(label)
        .bind(created_at)
        .bind(last_used_on)
        .bind(revoked_at)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;
}

/// Every row of `creation_tokens` in the database at `path`, the verifier as
/// its first byte (the rows of these tests repeat one byte).
async fn tokens_in(path: &Path) -> BTreeSet<StoredToken> {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(path).read_only(true))
        .await
        .unwrap();
    let rows = sqlx::query("SELECT id, verifier, label, created_at, last_used_on, revoked_at FROM creation_tokens")
        .fetch_all(&pool)
        .await
        .unwrap();
    pool.close().await;
    rows.iter()
        .map(|row| {
            let id: String = row.get("id");
            let verifier: Vec<u8> = row.get("verifier");
            assert!(verifier.iter().all(|byte| *byte == verifier[0]), "{id}");
            (
                id,
                verifier[0],
                row.get("label"),
                row.get("created_at"),
                row.get("last_used_on"),
                row.get("revoked_at"),
            )
        })
        .collect()
}

fn owned(rows: &[TokenRow]) -> BTreeSet<StoredToken> {
    rows.iter()
        .map(|(id, verifier, label, created, used, revoked)| ((*id).to_owned(), *verifier, (*label).to_owned(), *created, *used, *revoked))
        .collect()
}

const DAY: i64 = 86_400;

/// AC16: creation tokens of the replaced database are carried into the
/// restored one, matched by verifier: a token the backup lacks is inserted
/// with all its fields (active and revoked), a revocation the backup lacks is
/// applied with the replaced database's `revoked_at`, `last_used_on` becomes
/// the later of both in either direction, and a token only in the backup, or
/// revoked only in the backup, stays as it is. The dry run counts the same and
/// changes nothing.
#[tokio::test]
async fn restore_carries_creation_tokens_matched_by_verifier() {
    let dir = dir("restore-tokens");
    let version = db::expected_version();
    let backup = dir.join("backup.db");
    database_at(&backup, version, &[]).await;
    add_tokens(
        &backup,
        &[
            ("same", 1, "unchanged", 10, Some(DAY), None),
            ("revoked-later", 2, "revoked after the backup", 20, None, None),
            ("used-later-old", 3, "", 30, None, None),
            ("used-later-backup", 4, "", 40, Some(5 * DAY), None),
            ("backup-only", 5, "only in the backup", 50, Some(DAY), None),
            ("revoked-in-backup", 6, "", 60, None, Some(600)),
        ],
    )
    .await;

    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    database_at(&target.path, version, &[]).await;
    add_tokens(
        &target.path,
        &[
            ("same", 1, "unchanged", 10, Some(DAY), None),
            // Another id for the same verifier: matched by verifier, so no
            // second row; the backup's id stays.
            ("other-id", 2, "revoked after the backup", 20, None, Some(7_000)),
            ("used-later-old", 3, "", 30, Some(2 * DAY), None),
            ("used-later-backup", 4, "", 40, Some(DAY), None),
            ("revoked-in-backup", 6, "", 60, None, None),
            ("new-active", 7, "issued after the backup", 70, Some(3 * DAY), None),
            ("new-revoked", 8, "issued and revoked", 80, None, Some(8_000)),
        ],
    )
    .await;
    let files = with_sidecars(&target.path);
    let before = contents(&files);
    let listed = entries(&data);

    let dry_run = restore_dry_run(&backup, &target, true, 9_000).await.unwrap();
    assert_eq!(
        (dry_run.carried_tokens, dry_run.carried_revocations, dry_run.carried_last_uses),
        (2, 1, 1)
    );
    assert_eq!(dry_run.carried_ids, 0);
    assert!(dry_run.warnings.is_empty(), "{:?}", dry_run.warnings);
    assert_eq!(contents(&files), before, "the dry run changed the old database");
    assert_eq!(entries(&data), listed, "the dry run left an entry");

    let outcome = restore(&backup, &target, true, 9_000).await.unwrap();
    assert_eq!(
        (outcome.carried_tokens, outcome.carried_revocations, outcome.carried_last_uses),
        (2, 1, 1)
    );
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(contents(&with_sidecars(&outcome.previous.unwrap())), before);
    assert_eq!(
        tokens_in(&target.path).await,
        owned(&[
            ("same", 1, "unchanged", 10, Some(DAY), None),
            ("revoked-later", 2, "revoked after the backup", 20, None, Some(7_000)),
            ("used-later-old", 3, "", 30, Some(2 * DAY), None),
            ("used-later-backup", 4, "", 40, Some(5 * DAY), None),
            ("backup-only", 5, "only in the backup", 50, Some(DAY), None),
            ("revoked-in-backup", 6, "", 60, None, Some(600)),
            ("new-active", 7, "issued after the backup", 70, Some(3 * DAY), None),
            ("new-revoked", 8, "issued and revoked", 80, None, Some(8_000)),
        ])
    );
}

/// AC16: a replaced database at schema version 4 has no `creation_tokens`:
/// no token is carried, no warning, its ids still are, and the backup's tokens
/// stay as they are.
#[tokio::test]
async fn restore_over_a_database_without_creation_tokens_carries_none() {
    let dir = dir("restore-tokens-v4");
    let backup = dir.join("backup.db");
    database_at(&backup, db::expected_version(), &[]).await;
    let rows: [TokenRow; 1] = [("backup-token", 1, "kept", 10, None, None)];
    add_tokens(&backup, &rows).await;

    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    database_at(&target.path, 4, &["only-in-version-4"]).await;

    let dry_run = restore_dry_run(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(
        (dry_run.carried_ids, dry_run.carried_tokens, dry_run.carried_revocations),
        (1, 0, 0)
    );
    assert!(dry_run.warnings.is_empty(), "{:?}", dry_run.warnings);

    let outcome = restore(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(
        (
            outcome.carried_ids,
            outcome.carried_tokens,
            outcome.carried_revocations,
            outcome.carried_last_uses
        ),
        (1, 0, 0, 0)
    );
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(tokens_in(&target.path).await, owned(&rows));
}

/// AC16: when the replaced database's `creation_tokens` cannot be read (its
/// pages are corrupt, the ids are readable), the probe fails: the restore
/// warns once, carries neither ids nor tokens, and installs the backup.
#[tokio::test]
async fn restore_with_unreadable_creation_tokens_carries_neither_ids_nor_tokens() {
    let dir = dir("restore-tokens-unreadable");
    let version = db::expected_version();
    let backup = dir.join("backup.db");
    database_at(&backup, version, &[]).await;
    let rows: [TokenRow; 1] = [("backup-token", 1, "kept", 10, None, None)];
    add_tokens(&backup, &rows).await;

    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    database_at(&target.path, version, &["lost-id"]).await;
    add_tokens(&target.path, &[("lost-token", 2, "", 20, None, None)]).await;
    let root: i64 = {
        let pool = SqlitePoolOptions::new()
            .connect_with(SqliteConnectOptions::new().filename(&target.path).read_only(true))
            .await
            .unwrap();
        let root = sqlx::query_scalar("SELECT rootpage FROM sqlite_master WHERE name = 'creation_tokens'")
            .fetch_one(&pool)
            .await
            .unwrap();
        pool.close().await;
        root
    };
    let mut bytes = std::fs::read(&target.path).unwrap();
    let page_size = usize::from(u16::from_be_bytes([bytes[16], bytes[17]]));
    let start = (usize::try_from(root).unwrap() - 1) * page_size;
    bytes[start..start + page_size].fill(0xFF);
    std::fs::write(&target.path, bytes).unwrap();
    let files = with_sidecars(&target.path);
    let before = contents(&files);

    let dry_run = restore_dry_run(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!((dry_run.carried_ids, dry_run.carried_tokens), (0, 0));
    assert_eq!(dry_run.warnings.len(), 1, "{:?}", dry_run.warnings);

    let outcome = restore(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(
        (
            outcome.carried_ids,
            outcome.carried_tokens,
            outcome.carried_revocations,
            outcome.carried_last_uses
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(outcome.warnings, dry_run.warnings);
    assert!(
        outcome.warnings[0].contains("ids, capsule states and creation tokens")
            && outcome.warnings[0].contains("could not be carried over"),
        "{:?}",
        outcome.warnings
    );
    assert_eq!(contents(&with_sidecars(&outcome.previous.unwrap())), before);
    assert!(ids_in(&target.path, "retired_ids").await.is_empty());
    assert_eq!(tokens_in(&target.path).await, owned(&rows));
}

/// AC16: a write failure on the staged copy while the tokens are carried over
/// stops the restore before the cutover, with the old database, its sidecars
/// and its directory unchanged. So does a carried token whose id the backup
/// uses for another verifier (primary key).
#[tokio::test]
async fn restore_stops_when_the_copy_cannot_take_the_carried_tokens() {
    let dir = dir("restore-tokens-full");
    let version = db::expected_version();
    let backup = dir.join("backup.db");
    database_at(&backup, version, &[]).await;

    // 2 000 tokens, none in the backup, and no ids to carry: the scratch
    // table stays empty, so only the token insert needs new pages.
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    let pool = db::connect(&target).await.unwrap();
    sqlx::query(
        "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
         INSERT INTO creation_tokens (id, verifier, label, created_at)
         SELECT 'token-' || i, CAST(printf('%032d', i) AS BLOB), 'label ' || i, i FROM n",
    )
    .execute(&pool)
    .await
    .unwrap();
    let copy = dir.join("copy.db");
    copy_open_database(&target.path, &copy);
    pool.close().await;
    for (from, to) in with_sidecars(&copy).iter().zip(with_sidecars(&target.path)) {
        std::fs::rename(from, to).unwrap();
    }
    let files = with_sidecars(&target.path);
    assert!(contents(&files).iter().all(Option::is_some), "no sidecars");
    let before = contents(&files);
    let listed = entries(&data);

    let is_full = |error: &BackupError| matches!(error, BackupError::Database(sqlx::Error::Database(source)) if source.code().as_deref() == Some("13"));
    let limit = limit_carry_pages();
    let error = restore(&backup, &target, true, 5_000).await.unwrap_err();
    assert!(is_full(&error), "{error:?}");
    assert_eq!(contents(&files), before, "the restore changed the old database");
    assert_eq!(entries(&data), listed, "the restore left an entry");
    drop(limit);

    // The same id for another verifier in the backup fails on the primary key.
    let clash = dir.join("clash.db");
    database_at(&clash, version, &[]).await;
    add_tokens(&clash, &[("token-1", 0xEE, "", 1, None, None)]).await;
    let error = restore(&clash, &target, true, 5_000).await.unwrap_err();
    assert!(
        matches!(&error, BackupError::Database(sqlx::Error::Database(source)) if source.is_unique_violation()),
        "{error:?}"
    );
    assert_eq!(contents(&files), before, "the clash changed the old database");
    assert_eq!(entries(&data), listed, "the clash left an entry");

    // Without the limit the same restore carries the tokens over.
    let outcome = restore(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!(outcome.carried_tokens, 2_000);
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(contents(&with_sidecars(&outcome.previous.unwrap())), before);
}

/// The lifecycle columns of the capsule `byte` in the database at `path`:
/// `(availability_status, consumed_at, moderation_reason, has ciphertext)`.
async fn state_of(path: &Path, byte: u8) -> (String, Option<i64>, Option<String>, bool) {
    let pool = db::connect(&config(path.to_path_buf())).await.unwrap();
    let row = sqlx::query("SELECT availability_status, consumed_at, moderation_reason, ciphertext IS NOT NULL FROM pastes WHERE id = ?1")
        .bind(PasteId::from_bytes([byte; 16]).encode())
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    (row.get(0), row.get(1), row.get(2), row.get(3))
}

/// A capsule the replaced database burned, deleted (by its owner or by
/// moderation) or disabled after the backup was taken keeps that state in the
/// restored database, and its ciphertext is gone from the installed file; a
/// capsule re-enabled after the backup keeps the backup's (disabled) state, an
/// untouched one stays as it is. The dry run counts the same and changes
/// nothing.
#[tokio::test]
async fn restore_keeps_burns_deletions_and_moderation_after_the_backup() {
    use crate::domain::abuse::{ModerationAction, ReasonCode};
    use crate::repository::AbuseRepository as _;
    use crate::repository::sqlite::abuse::SqliteAbuseRepository;

    let dir = dir("restore-states");
    let source = dir.join("source.db");
    let pool = db::connect(&config(source.clone())).await.unwrap();
    let repo = SqlitePasteRepository::new(pool.clone());
    let abuse = SqliteAbuseRepository::new(pool.clone());
    let id = |byte: u8| PasteId::from_bytes([byte; 16]);
    repo.create(&NewPaste {
        burn_after_retrieval: true,
        ..paste(1, b"BURNED-CIPHERTEXT-MARKER")
    })
    .await
    .unwrap();
    repo.create(&paste(2, b"OWNER-DELETED-MARKER")).await.unwrap();
    repo.create(&paste(3, b"disabled later")).await.unwrap();
    repo.create(&paste(4, b"MODERATION-DELETED-MARKER")).await.unwrap();
    repo.create(&paste(5, b"re-enabled later")).await.unwrap();
    repo.create(&paste(6, b"untouched")).await.unwrap();
    abuse
        .moderate(&id(5), ModerationAction::Disable, Some(ReasonCode::Spam), None, 1_100)
        .await
        .unwrap();
    let backup = dir.join("backup.db");
    create(&pool, &backup).await.unwrap();

    assert!(repo.retrieve_and_burn(&id(1), 2_000).await.unwrap().is_some());
    assert!(repo.delete(&id(2)).await.unwrap());
    abuse
        .moderate(&id(3), ModerationAction::Disable, Some(ReasonCode::Malware), None, 2_100)
        .await
        .unwrap();
    abuse
        .moderate(&id(4), ModerationAction::Delete, Some(ReasonCode::IllegalContent), None, 2_200)
        .await
        .unwrap();
    abuse.moderate(&id(5), ModerationAction::Restore, None, None, 2_300).await.unwrap();

    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    copy_open_database(&source, &target.path);
    pool.close().await;
    let files = with_sidecars(&target.path);
    let before = contents(&files);
    let listed = entries(&data);

    let dry_run = restore_dry_run(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!((dry_run.carried_states, dry_run.carried_ids), (4, 0));
    assert!(dry_run.warnings.is_empty(), "{:?}", dry_run.warnings);
    assert_eq!(contents(&files), before, "the dry run changed the old database");
    assert_eq!(entries(&data), listed, "the dry run left an entry");

    let outcome = restore(&backup, &target, true, 5_000).await.unwrap();
    assert_eq!((outcome.carried_states, outcome.carried_ids), (4, 0));
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    assert_eq!(contents(&with_sidecars(&outcome.previous.unwrap())), before);

    assert_eq!(state_of(&target.path, 1).await, ("active".to_owned(), Some(2_000), None, false));
    assert_eq!(state_of(&target.path, 2).await, ("deleted".to_owned(), None, None, false));
    assert_eq!(
        state_of(&target.path, 3).await,
        ("disabled".to_owned(), None, Some("malware".to_owned()), true)
    );
    assert_eq!(
        state_of(&target.path, 4).await,
        ("deleted".to_owned(), None, Some("illegal_content".to_owned()), false)
    );
    assert_eq!(
        state_of(&target.path, 5).await,
        ("disabled".to_owned(), None, Some("spam".to_owned()), true),
        "re-enabling after the backup is not carried over"
    );
    assert_eq!(state_of(&target.path, 6).await, ("active".to_owned(), None, None, true));

    let repo = SqlitePasteRepository::new(db::connect(&target).await.unwrap());
    for byte in [1, 2, 3, 4, 5] {
        assert_eq!(repo.load_ciphertext(&id(byte), 5_000).await.unwrap(), None, "{byte}");
    }
    assert_eq!(
        repo.load_ciphertext(&id(6), 5_000).await.unwrap().as_deref(),
        Some(&b"untouched"[..])
    );
    drop(repo);

    let installed: Vec<u8> = with_sidecars(&target.path)
        .iter()
        .filter_map(|path| std::fs::read(path).ok())
        .flatten()
        .collect();
    for marker in [
        &b"BURNED-CIPHERTEXT-MARKER"[..],
        b"OWNER-DELETED-MARKER",
        b"MODERATION-DELETED-MARKER",
    ] {
        assert!(
            !installed.windows(marker.len()).any(|window| window == marker),
            "{} left in the installed database",
            String::from_utf8_lossy(marker)
        );
    }
}
