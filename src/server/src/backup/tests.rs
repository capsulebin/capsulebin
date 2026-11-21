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

/// A backup at `path` that has only the first migration and one paste row.
async fn first_migration_backup(path: &Path) {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(path).create_if_missing(true))
        .await
        .unwrap();
    db::MIGRATOR.run_to(1, &pool).await.unwrap();
    sqlx::query(
        "INSERT INTO pastes (id, format_version, public_header, reveal_verifier, delete_verifier, created_at, payload_size)
         VALUES ('first-migration-paste', 1, x'00', x'00', x'00', 1, 1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    assert!(!sidecars(path)[0].exists());
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
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let target = config(data.join("live.db"));
    let files = old_database(&target.path);
    let before = contents(&files);
    let listed = entries(&data);

    for (step, kind) in [
        (Step::Copy, ErrorKind::Other),
        (Step::Copy, ErrorKind::StorageFull),
        (Step::Verify, ErrorKind::Other),
        (Step::Migrate, ErrorKind::Other),
        (Step::Checkpoint, ErrorKind::Other),
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
    for (from, to) in with_sidecars(&live).iter().zip(with_sidecars(&backup)) {
        std::fs::copy(from, to).unwrap();
    }
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
