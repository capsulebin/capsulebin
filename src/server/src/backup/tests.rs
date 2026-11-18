#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::*;
use crate::config::{DatabaseConfig, Synchronous};
use crate::domain::{NewPaste, PasteId, Verifier};
use crate::repository::PasteRepository as _;
use crate::repository::sqlite::SqlitePasteRepository;

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
async fn never_overwrites_an_existing_file() {
    let dir = dir("exists");
    let pool = database_with_pastes(dir.join("live.db")).await;
    std::fs::write(dir.join("backup.db"), b"precious").unwrap();

    let error = create(&pool, &dir.join("backup.db")).await.unwrap_err();
    assert!(matches!(error, BackupError::Exists(_)), "{error}");
    assert_eq!(std::fs::read(dir.join("backup.db")).unwrap(), b"precious");
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

#[tokio::test]
async fn restores_into_an_empty_place() {
    let dir = dir("restore-empty");
    let pool = database_with_pastes(dir.join("live.db")).await;
    create(&pool, &dir.join("backup.db")).await.unwrap();

    let target = dir.join("restored.db");
    let outcome = restore(&dir.join("backup.db"), &target, false, 5_000).await.unwrap();
    assert_eq!(outcome.previous, None);
    assert_eq!(outcome.info.pastes, 2);

    let reopened = db::connect(&config(target)).await.unwrap();
    assert_eq!(db::verify(&reopened).await.unwrap(), db::expected_version());
    let repo = SqlitePasteRepository::new(reopened);
    let kept = repo.find(&PasteId::from_bytes([1; 16])).await.unwrap().unwrap();
    assert_eq!(kept.ciphertext.as_deref(), Some(&b"kept ciphertext"[..]));
}

#[tokio::test]
async fn replacing_moves_the_old_database_and_its_wal_aside() {
    let dir = dir("restore-replace");
    let backup_source = database_with_pastes(dir.join("source.db")).await;
    create(&backup_source, &dir.join("backup.db")).await.unwrap();

    let target = dir.join("live.db");
    std::fs::write(&target, b"old database").unwrap();
    std::fs::write(sidecars(&target)[0].clone(), b"old wal with committed data").unwrap();

    let error = restore(&dir.join("backup.db"), &target, false, 5_000).await.unwrap_err();
    assert!(matches!(error, BackupError::Exists(_)), "{error}");
    assert_eq!(std::fs::read(&target).unwrap(), b"old database", "touched without --force");

    let outcome = restore(&dir.join("backup.db"), &target, true, 5_000).await.unwrap();
    let previous = outcome.previous.unwrap();
    assert_eq!(previous, dir.join("live.db.pre-restore-5000"));
    assert_eq!(std::fs::read(&previous).unwrap(), b"old database");
    assert_eq!(
        std::fs::read(sidecars(&previous)[0].clone()).unwrap(),
        b"old wal with committed data"
    );
    assert!(
        !sidecars(&target)[0].exists(),
        "the old WAL would be replayed onto the restored file"
    );
    assert_eq!(verify_file(&target).await.unwrap().pastes, 2);
}

#[tokio::test]
async fn ignores_a_wal_file_next_to_the_backup() {
    let dir = dir("restore-wal");
    let pool = database_with_pastes(dir.join("live.db")).await;
    let backup = dir.join("backup.db");
    create(&pool, &backup).await.unwrap();
    std::fs::write(sidecars(&backup)[0].clone(), b"untrusted wal").unwrap();

    let target = dir.join("restored.db");
    restore(&backup, &target, false, 1).await.unwrap();
    assert!(!sidecars(&target)[0].exists());
}

#[tokio::test]
async fn refuses_to_restore_a_file_onto_itself() {
    let dir = dir("restore-self");
    let pool = database_with_pastes(dir.join("live.db")).await;
    create(&pool, &dir.join("backup.db")).await.unwrap();

    let error = restore(&dir.join("backup.db"), &dir.join("backup.db"), true, 1).await.unwrap_err();
    assert!(matches!(error, BackupError::SameFile), "{error}");
}
