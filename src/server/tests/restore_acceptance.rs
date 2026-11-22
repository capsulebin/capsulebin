#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance tests for `capsulebin restore` against the real binary (package
//! 20261121-operator-files-restore): `restore --dry-run` prints the backup's
//! contents and the schema version the staged copy was migrated to, exits 0
//! and changes nothing (AC12); a successful `restore --force` keeps the old
//! database with its sidecars aside (AC9).
//! Package 20261122-creation-tokens-db (AC16): creation tokens issued and
//! revoked after the backup survive `restore --force`.
//! Package 20261122-restore-protection (AC2): a capsule of the backup purged
//! from the current database is removed by `restore --force`.
//! A database path that is a symbolic link is refused by `restore`
//! and `restore --dry-run`, the link and the linked files left unchanged.

mod support;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use support::{tempdir, write_shells};

const BINARY: &str = env!("CARGO_BIN_EXE_capsulebin");

/// A configuration for `<dir>/data/capsulebin.db`. Returns the configuration,
/// the data directory and the database path.
fn setup(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = tempdir(name);
    let assets = dir.join("assets");
    write_shells(&assets);
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let database = data.join("capsulebin.db");
    let config = dir.join("capsulebin.toml");
    std::fs::write(
        &config,
        format!(
            "[server]\npublic_url = \"https://paste.example\"\nasset_dir = \"{}\"\n\
             [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n",
            assets.display(),
            database.display()
        ),
    )
    .unwrap();
    (config, data, database)
}

fn run(config: &Path, args: &[&str]) -> Output {
    Command::new(BINARY).arg("--config").arg(config).args(args).output().unwrap()
}

fn describe(output: &Output) -> String {
    format!(
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn entries(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn with_sidecars(path: &Path) -> Vec<PathBuf> {
    ["", "-wal", "-shm"]
        .iter()
        .map(|suffix| {
            let mut name = path.as_os_str().to_owned();
            name.push(suffix);
            PathBuf::from(name)
        })
        .collect()
}

fn contents(paths: &[PathBuf]) -> Vec<Option<Vec<u8>>> {
    paths.iter().map(|path| std::fs::read(path).ok()).collect()
}

/// A backup on the current schema with two paste rows.
async fn two_paste_backup(path: &Path) {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(path).create_if_missing(true))
        .await
        .unwrap();
    capsulebin_server::db::MIGRATOR.run(&pool).await.unwrap();
    for id in ["first-row", "second-row"] {
        sqlx::query(
            "INSERT INTO pastes (id, format_version, public_header, reveal_verifier, delete_verifier, describe_verifier, created_at, payload_size)
             VALUES (?, 1, x'00', x'00', x'00', x'00', 1, 1)",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;
}

/// A database at `path` with its `-wal` and `-shm`, each holding a marker.
fn old_database(path: &Path) -> Vec<PathBuf> {
    let files = with_sidecars(path);
    for (file, content) in files.iter().zip([&b"old database"[..], b"old wal", b"old shm"]) {
        std::fs::write(file, content).unwrap();
    }
    files
}

/// AC12: the dry run reports the backup (its schema version, its rows) and the
/// version the private copy was migrated to, exits 0, and leaves input,
/// target, its sidecars and the target directory as they were. A backup that
/// cannot be used fails the dry run with the same state.
#[tokio::test]
async fn dry_run_reports_the_migrated_version_and_changes_nothing() {
    let (config, data, database) = setup("restore-acc-dry-run");
    let backup = data.parent().unwrap().join("backup.db");
    two_paste_backup(&backup).await;
    let input = std::fs::read(&backup).unwrap();
    let files = old_database(&database);
    let before = contents(&files);
    let listed = entries(&data);
    assert_eq!(capsulebin_server::db::expected_version(), 1);

    for force in [false, true] {
        let mut args = vec!["restore", "--input", backup.to_str().unwrap(), "--dry-run"];
        if force {
            args.push("--force");
        }
        let output = run(&config, &args);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "force={force}: {}", describe(&output));
        assert!(stdout.starts_with("ok:"), "force={force}: {}", describe(&output));
        assert!(stdout.contains("schema version 1,"), "backup's version missing: {stdout}");
        assert!(stdout.contains("2 paste row(s)"), "backup's rows missing: {stdout}");
        assert!(stdout.contains("migrated to schema version 1"), "reached version missing: {stdout}");

        assert_eq!(std::fs::read(&backup).unwrap(), input, "force={force}: input changed");
        assert_eq!(contents(&files), before, "force={force}: target changed");
        assert_eq!(entries(&data), listed, "force={force}: new entry in the target directory");
    }

    // Edge case: an unusable backup fails the dry run and changes nothing.
    let garbage = data.parent().unwrap().join("garbage.db");
    std::fs::write(&garbage, b"this is not a database at all, not even close").unwrap();
    let output = run(&config, &["restore", "--input", garbage.to_str().unwrap(), "--dry-run", "--force"]);
    assert!(!output.status.success(), "garbage accepted: {}", describe(&output));
    assert_eq!(contents(&files), before, "garbage dry run changed the target");
    assert_eq!(entries(&data), listed, "garbage dry run left an entry");
}

/// AC9 through the binary: `restore --force` keeps the old database and both
/// sidecars byte-identical under `<file>.pre-restore-<now>`, names it, and the
/// target then holds the backup's rows at the current schema version.
#[tokio::test]
async fn forced_restore_keeps_the_old_database_with_its_sidecars() {
    let (config, data, database) = setup("restore-acc-force");
    let backup = data.parent().unwrap().join("backup.db");
    two_paste_backup(&backup).await;
    let files = old_database(&database);
    let before = contents(&files);

    // Without --force nothing is touched.
    let output = run(&config, &["restore", "--input", backup.to_str().unwrap()]);
    assert!(!output.status.success(), "{}", describe(&output));
    assert_eq!(contents(&files), before);

    let output = run(&config, &["restore", "--input", backup.to_str().unwrap(), "--force"]);
    assert!(output.status.success(), "{}", describe(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    // The old database is garbage: its ids cannot be carried over, which is
    // a warning on stderr, not a failure.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .lines()
            .any(|line| line.starts_with("warning: ") && line.contains("could not be carried over")),
        "no warning: {}",
        describe(&output)
    );
    assert!(stdout.contains("0 id(s) of the previous database"), "count missing: {stdout}");

    let aside: Vec<String> = entries(&data)
        .into_iter()
        .filter(|name| name.starts_with("capsulebin.db.pre-restore-") && !name.ends_with("-wal") && !name.ends_with("-shm"))
        .collect();
    assert_eq!(aside.len(), 1, "{:?}", entries(&data));
    let aside = data.join(&aside[0]);
    assert!(stdout.contains(&aside.display().to_string()), "aside name not reported: {stdout}");
    assert_eq!(contents(&with_sidecars(&aside)), before, "old database not kept as a full set");
    assert!(
        entries(&data).iter().all(|name| !name.starts_with(".capsulebin.db.restore-")),
        "stage left: {:?}",
        entries(&data)
    );

    let output = run(&config, &["check"]);
    assert!(output.status.success(), "{}", describe(&output));
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(&database).read_only(true))
        .await
        .unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pastes").fetch_one(&pool).await.unwrap();
    assert_eq!(rows, 2, "not the backup's rows");
    let version: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(version, capsulebin_server::db::expected_version());
    pool.close().await;
}

/// Ids of the database at `path` in `table`.
async fn ids_in(path: &Path, table: &str) -> BTreeSet<String> {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(path))
        .await
        .unwrap();
    let ids: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT id FROM {table}")))
        .fetch_all(&pool)
        .await
        .unwrap();
    pool.close().await;
    ids.into_iter().collect()
}

/// Package 20261121-retired-ids, AC7 through the binary: the replaced database
/// holds A (also in the backup), B (live, not in the backup) and C (retired, not
/// in the backup). `restore --dry-run` prints that 2 ids would be retired, warns
/// about nothing and changes nothing; `restore --force` prints the same count,
/// and afterwards B and C are retired while the backup's rows are installed.
/// A, deleted in the replaced database after the backup, stays deleted (both
/// runs print one capsule that keeps its state).
#[tokio::test]
async fn restore_reports_and_retires_the_ids_missing_from_the_backup() {
    let (config, data, database) = setup("restore-acc-carry");
    let backup = data.parent().unwrap().join("backup.db");
    two_paste_backup(&backup).await;

    let migrated = run(&config, &["migrate"]);
    assert!(migrated.status.success(), "{}", describe(&migrated));
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(&database))
        .await
        .unwrap();
    for id in ["first-row", "live-b"] {
        sqlx::query(
            "INSERT INTO pastes (id, format_version, public_header, reveal_verifier, delete_verifier, describe_verifier, created_at, payload_size)
             VALUES (?, 1, x'00', x'00', x'00', x'00', 1, 1)",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO retired_ids (id) VALUES ('retired-c')")
        .execute(&pool)
        .await
        .unwrap();
    // Deleted after the backup was taken, which holds it active.
    sqlx::query("UPDATE pastes SET availability_status = 'deleted' WHERE id = 'first-row'")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let files = with_sidecars(&database);
    let before = contents(&files);
    let listed = entries(&data);

    let output = run(&config, &["restore", "--input", backup.to_str().unwrap(), "--dry-run", "--force"]);
    assert!(output.status.success(), "{}", describe(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 id(s) of the current database not in the backup would be retired"),
        "count missing: {}",
        describe(&output)
    );
    assert!(
        stdout.contains("1 capsule(s) burned, deleted or disabled in the current database would keep that state"),
        "state count missing: {}",
        describe(&output)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("warning:"),
        "{}",
        describe(&output)
    );
    assert_eq!(contents(&files), before, "the dry run changed the database");
    assert_eq!(entries(&data), listed, "the dry run left an entry");

    let output = run(&config, &["restore", "--input", backup.to_str().unwrap(), "--force"]);
    assert!(output.status.success(), "{}", describe(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("2 id(s) of the previous database not in the backup were retired"),
        "count missing: {}",
        describe(&output)
    );
    assert!(
        stdout.contains("1 capsule(s) burned, deleted or disabled in the previous database kept that state"),
        "state count missing: {}",
        describe(&output)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("warning:"),
        "{}",
        describe(&output)
    );
    assert_eq!(
        ids_in(&database, "pastes").await,
        BTreeSet::from(["first-row".to_owned(), "second-row".to_owned()])
    );
    assert_eq!(
        ids_in(&database, "retired_ids").await,
        BTreeSet::from(["live-b".to_owned(), "retired-c".to_owned()])
    );
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(&database))
        .await
        .unwrap();
    let status: String = sqlx::query_scalar("SELECT availability_status FROM pastes WHERE id = 'first-row'")
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(status, "deleted", "the deletion after the backup was undone");
}

/// Runs `sql` once per entry of `rows`, binding its values, on the database at `path`.
async fn execute(path: &Path, sql: &str, rows: &[&[&str]]) {
    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(path))
        .await
        .unwrap();
    for row in rows {
        let mut query = sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()));
        for value in *row {
            query = query.bind(*value);
        }
        query.execute(&pool).await.unwrap();
    }
    pool.close().await;
}

/// Package 20261122-creation-tokens-db, AC16 through the binary: token A is in
/// the backup and revoked after it, token B is issued after it. The dry run
/// prints that one token and one revocation would be carried over and changes
/// nothing; `restore --force` prints the same, and afterwards A is revoked and
/// B is present with its id and label.
#[tokio::test]
async fn restore_keeps_creation_tokens_and_revocations_made_after_the_backup() {
    let (config, data, database) = setup("restore-acc-tokens");
    let backup = data.parent().unwrap().join("backup.db");

    let migrated = run(&config, &["migrate"]);
    assert!(migrated.status.success(), "{}", describe(&migrated));
    let insert = "INSERT INTO creation_tokens (id, verifier, label, created_at) VALUES (?1, CAST(?2 AS BLOB), ?3, 100)";
    execute(&database, insert, &[&["token-a", "verifier-a-0123456789abcdef0123", "alice"]]).await;
    let output = run(&config, &["backup", "--output", backup.to_str().unwrap()]);
    assert!(output.status.success(), "{}", describe(&output));

    execute(&database, insert, &[&["token-b", "verifier-b-0123456789abcdef0123", "bob"]]).await;
    execute(
        &database,
        "UPDATE creation_tokens SET revoked_at = 200 WHERE id = ?1",
        &[&["token-a"]],
    )
    .await;
    let files = with_sidecars(&database);
    let before = contents(&files);
    let listed = entries(&data);

    let output = run(&config, &["restore", "--input", backup.to_str().unwrap(), "--dry-run", "--force"]);
    assert!(output.status.success(), "{}", describe(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "1 creation token(s), 1 revocation(s) and 0 last-use date(s) of the current database not in the backup \
             would be carried over"
        ),
        "counts missing: {}",
        describe(&output)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("warning:"),
        "{}",
        describe(&output)
    );
    assert_eq!(contents(&files), before, "the dry run changed the database");
    assert_eq!(entries(&data), listed, "the dry run left an entry");

    let output = run(&config, &["restore", "--input", backup.to_str().unwrap(), "--force"]);
    assert!(output.status.success(), "{}", describe(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "1 creation token(s), 1 revocation(s) and 0 last-use date(s) of the previous database not in the backup \
             were carried over"
        ),
        "counts missing: {}",
        describe(&output)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("warning:"),
        "{}",
        describe(&output)
    );

    let pool = SqlitePoolOptions::new()
        .connect_with(SqliteConnectOptions::new().filename(&database).read_only(true))
        .await
        .unwrap();
    let tokens: Vec<(String, String, Option<i64>)> = sqlx::query_as("SELECT id, label, revoked_at FROM creation_tokens ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    pool.close().await;
    assert_eq!(
        tokens,
        [
            ("token-a".to_owned(), "alice".to_owned(), Some(200)),
            ("token-b".to_owned(), "bob".to_owned(), None)
        ]
    );
}

/// Package 20261122-restore-protection, AC2 through the binary: the backup
/// holds the capsule `first-row`, which the current database has since purged
/// (only its id is left in `retired_ids`). `restore --dry-run` prints that one
/// purged capsule would be removed, warns about nothing and changes nothing;
/// `restore --force` prints that it was removed, and afterwards `first-row`
/// has no row and stays retired.
#[tokio::test]
async fn restore_removes_the_backup_capsules_purged_from_the_current_database() {
    let (config, data, database) = setup("restore-acc-purged");
    let backup = data.parent().unwrap().join("backup.db");
    two_paste_backup(&backup).await;

    let migrated = run(&config, &["migrate"]);
    assert!(migrated.status.success(), "{}", describe(&migrated));
    execute(
        &database,
        "INSERT INTO pastes (id, format_version, public_header, reveal_verifier, delete_verifier, describe_verifier, created_at, payload_size)
         VALUES (?1, 1, x'00', x'00', x'00', x'00', 1, 1)",
        &[&["second-row"]],
    )
    .await;
    execute(&database, "INSERT INTO retired_ids (id) VALUES (?1)", &[&["first-row"]]).await;
    let files = with_sidecars(&database);
    let before = contents(&files);
    let listed = entries(&data);

    let output = run(&config, &["restore", "--input", backup.to_str().unwrap(), "--dry-run", "--force"]);
    assert!(output.status.success(), "{}", describe(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 capsule(s) of the backup purged from the current database would be removed."),
        "purge count missing: {}",
        describe(&output)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("warning:"),
        "{}",
        describe(&output)
    );
    assert_eq!(contents(&files), before, "the dry run changed the database");
    assert_eq!(entries(&data), listed, "the dry run left an entry");

    let output = run(&config, &["restore", "--input", backup.to_str().unwrap(), "--force"]);
    assert!(output.status.success(), "{}", describe(&output));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("1 capsule(s) of the backup purged from the previous database were removed."),
        "purge count missing: {}",
        describe(&output)
    );
    assert!(
        !String::from_utf8_lossy(&output.stderr).contains("warning:"),
        "{}",
        describe(&output)
    );
    assert_eq!(ids_in(&database, "pastes").await, BTreeSet::from(["second-row".to_owned()]));
    assert_eq!(ids_in(&database, "retired_ids").await, BTreeSet::from(["first-row".to_owned()]));
}

/// SQLite keeps the `-wal` of a symlinked database next to the file
/// the link points to, so a restore through the link could miss commits in it.
/// `restore`, `restore --force` and `restore --dry-run` refuse the link, name
/// `database.path`, and change neither the link nor the linked files.
#[cfg(unix)]
#[tokio::test]
async fn restore_refuses_a_symlinked_database() {
    let (config, data, database) = setup("restore-symlink");
    let backup = data.parent().unwrap().join("backup.db");
    two_paste_backup(&backup).await;
    let real = data.parent().unwrap().join("real");
    std::fs::create_dir(&real).unwrap();
    let linked = real.join("capsulebin.db");
    let files = old_database(&linked);
    std::os::unix::fs::symlink(&linked, &database).unwrap();
    let (data_before, real_before) = (entries(&data), entries(&real));
    let before = contents(&files);

    let input = backup.to_str().unwrap();
    for args in [
        &["restore", "--input", input][..],
        &["restore", "--force", "--input", input],
        &["restore", "--dry-run", "--input", input],
        &["restore", "--dry-run", "--force", "--input", input],
    ] {
        let output = run(&config, args);
        assert!(!output.status.success(), "{args:?}: {}", describe(&output));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("is a symbolic link"), "{args:?}: {}", describe(&output));
        assert!(stderr.contains("database.path"), "{args:?}: {}", describe(&output));
        assert!(std::fs::symlink_metadata(&database).unwrap().file_type().is_symlink(), "{args:?}");
        assert_eq!(entries(&data), data_before, "{args:?}");
        assert_eq!(entries(&real), real_before, "{args:?}");
        assert_eq!(contents(&files), before, "{args:?}");
    }
}
