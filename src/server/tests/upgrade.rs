#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Upgrading a database written by CapsuleBin 1.0.
//!
//! `fixtures/capsulebin-1.0.sql` is a dump of a database the 1.0 binary wrote:
//! pastes in every state that matters, a report and a moderation action. These
//! tests load it, let this build's migrations run as an upgrade would, and use
//! the data through the HTTP API and the repositories.
//!
//! Two kinds of change fail here:
//!
//! - **An edited migration.** sqlx stores each applied migration's checksum and
//!   refuses to start when an embedded migration no longer matches it, so
//!   changing a released migration would lock every existing deployment out.
//!   Schema changes go into a new migration instead.
//! - **A new migration that loses or breaks 1.0 data**: rows that no longer
//!   load, capabilities that stop matching, a tombstone that comes back to life.
//!
//! The fixture is never regenerated. A later release adds a dump of its own next
//! to it, so every supported starting point stays tested.

mod support;

use axum::http::StatusCode;
use capsulebin_server::config::{DatabaseConfig, Synchronous};
use capsulebin_server::db;
use capsulebin_server::domain::abuse::ModerationAction;
use capsulebin_server::domain::{Availability, PurgePolicy};
use capsulebin_server::domain::{Label, NewPaste, Verifier};
use capsulebin_server::repository::sqlite::SqlitePasteRepository;
use capsulebin_server::repository::sqlite::abuse::SqliteAbuseRepository;
use capsulebin_server::repository::sqlite::creation_tokens::SqliteCreationTokenRepository;
use capsulebin_server::repository::{
    AbuseRepository as _, CreationTokenRepository as _, ModerationOutcome, PasteRepository as _, RepositoryError,
};
use support::*;

const DUMP: &str = include_str!("fixtures/capsulebin-1.0.sql");

/// An hour after the fixture was written, inside its pastes' 7-day expiry.
const NOW: i64 = 1_795_074_774;

const ACTIVE: u8 = 0x10;
const BURNED: u8 = 0x11;
const PASSWORD: u8 = 0x12;
const DISABLED: u8 = 0x13;

/// The fixture's tokens are `<kind>-<id byte in hex>`.
fn token(kind: &str, byte: u8) -> Vec<u8> {
    format!("{kind}-{byte:02x}").into_bytes()
}

fn ciphertext_of(byte: u8) -> Vec<u8> {
    let mut bytes = vec![0x00, 0xFF];
    bytes.extend_from_slice(format!("fixture-ciphertext-{byte:02x}").as_bytes());
    bytes
}

async fn creation_token_count(pool: &sqlx::SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM creation_tokens")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn describe(h: &Harness, byte: u8) -> Answer {
    h.post_with_token(
        &format!("/api/v1/pastes/{}/describe", id(byte).encode()),
        describe_body(&token("describe", byte)),
        None,
    )
    .await
}

async fn retrieve(h: &Harness, byte: u8) -> Answer {
    h.post_with_token(
        &format!("/api/v1/pastes/{}/retrieve", id(byte).encode()),
        retrieve_body_with(&token("describe", byte), &token("reveal", byte)),
        None,
    )
    .await
}

#[tokio::test]
async fn a_1_0_database_migrates_to_the_current_schema() {
    let h = harness_from_dump("upgrade-schema", DUMP, NOW).await;

    assert_eq!(db::verify(&h.pool).await.unwrap(), db::expected_version());
    assert_eq!(h.get("/health/ready").await.status, StatusCode::OK);
    assert_eq!(creation_token_count(&h.pool).await, 0);

    // Running the migrations again on the upgraded file changes nothing.
    db::MIGRATOR.run(&h.pool).await.unwrap();
}

#[tokio::test]
async fn an_active_1_0_paste_opens_and_deletes_with_its_capabilities() {
    let h = harness_from_dump("upgrade-active", DUMP, NOW).await;

    let descriptor = describe(&h, ACTIVE).await;
    assert_eq!(descriptor.status, StatusCode::OK, "{}", descriptor.text());
    assert_eq!(descriptor.json()["burn_after_retrieval"], false);

    let retrieved = retrieve(&h, ACTIVE).await;
    assert_eq!(retrieved.status, StatusCode::OK, "{}", retrieved.text());
    assert_eq!(retrieved.json()["ciphertext"], b64(&ciphertext_of(ACTIVE)));

    let path = format!("/api/v1/pastes/{}", id(ACTIVE).encode());
    let deleted = h.delete(&path, delete_body(&token("delete", ACTIVE))).await;
    assert!(deleted.status.is_success(), "{}", deleted.text());
    assert_eq!(retrieve(&h, ACTIVE).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_burned_1_0_paste_stays_burned() {
    let h = harness_from_dump("upgrade-burned", DUMP, NOW).await;

    assert_eq!(retrieve(&h, BURNED).await.status, StatusCode::NOT_FOUND);
    assert_eq!(describe(&h, BURNED).await.status, StatusCode::NOT_FOUND);

    let tombstone = SqlitePasteRepository::new(h.pool.clone())
        .find(&id(BURNED))
        .await
        .unwrap()
        .expect("the tombstone must survive the upgrade");
    assert!(tombstone.burn_after_retrieval);
    assert!(tombstone.consumed_at.is_some());
    assert_eq!(tombstone.ciphertext, None);
}

#[tokio::test]
async fn a_1_0_password_header_is_returned_byte_for_byte() {
    let h = harness_from_dump("upgrade-password", DUMP, NOW).await;

    let stored = SqlitePasteRepository::new(h.pool.clone())
        .find(&id(PASSWORD))
        .await
        .unwrap()
        .expect("the password paste must load");
    let header = String::from_utf8(stored.public_header.clone()).unwrap();
    assert!(
        header
            .ends_with(r#""password":{"required":true,"kdf":"PBKDF2-HMAC-SHA-256","salt":"ICEiIyQlJicoKSorLC0uLw","iterations":600000}}"#),
        "{header}"
    );

    // The header is AES-GCM additional data: any change to its bytes would make
    // the capsule impossible to decrypt, so it has to come back exactly.
    let descriptor = describe(&h, PASSWORD).await;
    assert_eq!(descriptor.status, StatusCode::OK, "{}", descriptor.text());
    assert_eq!(descriptor.json()["public_header"], b64(&stored.public_header));

    let retrieved = retrieve(&h, PASSWORD).await;
    assert_eq!(retrieved.status, StatusCode::OK, "{}", retrieved.text());
    assert_eq!(retrieved.json()["public_header"], b64(&stored.public_header));
    assert_eq!(retrieved.json()["ciphertext"], b64(&ciphertext_of(PASSWORD)));
}

#[tokio::test]
async fn moderation_records_and_reports_from_1_0_are_kept() {
    let h = harness_from_dump("upgrade-moderation", DUMP, NOW).await;
    let abuse = SqliteAbuseRepository::new(h.pool.clone());

    assert_eq!(retrieve(&h, DISABLED).await.status, StatusCode::NOT_FOUND);

    let reports = abuse.list_reports(None, 10).await.unwrap();
    assert_eq!(reports.len(), 1);
    let entry = &reports[0];
    assert_eq!(entry.report.paste_id, id(DISABLED).encode());
    assert_eq!(entry.report.reason, "spam");
    assert_eq!(entry.report.status, "closed");
    assert_eq!(entry.report.resolution.as_deref(), Some("action_taken"));
    assert_eq!(entry.paste_status.as_deref(), Some("disabled"));

    let history = abuse.moderation_history(&id(DISABLED)).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].action, "disable");
    assert_eq!(history[0].reason.as_deref(), Some("spam"));

    // A 1.0 decision can still be reversed, and the payload is still there.
    let outcome = abuse
        .moderate(&id(DISABLED), ModerationAction::Restore, None, None, NOW)
        .await
        .unwrap();
    assert!(matches!(outcome, ModerationOutcome::Applied), "{outcome:?}");
    let paste = SqlitePasteRepository::new(h.pool.clone())
        .find(&id(DISABLED))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(paste.availability, Availability::Active);

    let retrieved = retrieve(&h, DISABLED).await;
    assert_eq!(retrieved.status, StatusCode::OK, "{}", retrieved.text());
    assert_eq!(retrieved.json()["ciphertext"], b64(&ciphertext_of(DISABLED)));
}

#[tokio::test]
async fn an_upgraded_database_accepts_new_pastes() {
    let h = harness_from_dump("upgrade-create", DUMP, NOW).await;

    let created = h.post("/api/v1/pastes", create_body(&id(1))).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    assert_eq!(h.describe(&id(1).encode()).await.status, StatusCode::OK);
}

/// Migration `0004` starts with no retired ids (rows purged before it ran left
/// no record), keeps refusing the 1.0 tombstones' ids and retires every 1.0 row
/// a purge removes.
#[tokio::test]
async fn purged_fixture_ids_stay_retired() {
    let h = harness_from_dump("upgrade-retired-ids", DUMP, NOW).await;
    let fixture = [ACTIVE, BURNED, PASSWORD, DISABLED];

    assert_eq!(db::verify(&h.pool).await.unwrap(), db::expected_version());
    let retired = || async {
        sqlx::query_scalar::<_, String>("SELECT id FROM retired_ids ORDER BY id")
            .fetch_all(&h.pool)
            .await
            .unwrap()
    };
    assert!(retired().await.is_empty());

    let tombstone = h.post("/api/v1/pastes", create_body(&id(BURNED))).await;
    assert_eq!(tombstone.status, StatusCode::CONFLICT, "{}", tombstone.text());
    assert_eq!(tombstone.code(), "id_conflict");

    let policy = PurgePolicy {
        never_retrieved_after: 86_400,
        inactive_after: 86_400,
    };
    let outcome = SqlitePasteRepository::new(h.pool.clone())
        .purge(NOW + 365 * 86_400, &policy, 100)
        .await
        .unwrap();
    assert_eq!(outcome.expired + outcome.never_retrieved + outcome.inactive, fixture.len() as u64);

    let mut expected: Vec<String> = fixture.iter().map(|&byte| id(byte).encode()).collect();
    expected.sort();
    assert_eq!(retired().await, expected);

    for byte in fixture {
        let answer = h.post("/api/v1/pastes", create_body(&id(byte))).await;
        assert_eq!(answer.status, StatusCode::CONFLICT, "{byte:#x}: {}", answer.text());
        assert_eq!(answer.code(), "id_conflict");
    }
}

/// Migration `0005` on a database at schema 4: the 1.0 fixture migrated up to
/// `0004`, used there (a purge retires ids), then opened by this build. The
/// table of creation tokens starts empty, the retired ids stay retired, the
/// remaining pastes keep their capabilities, and tokens can be stored.
#[tokio::test]
async fn a_schema_4_database_migrates_to_schema_5() {
    let dir = tempdir("upgrade-schema-4");
    let config = DatabaseConfig {
        path: dir.join("capsulebin.db"),
        busy_timeout_ms: 5_000,
        synchronous: Synchronous::Full,
    };

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&config.path)
        .create_if_missing(true);
    let seed = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::raw_sql(DUMP).execute(&seed).await.unwrap();
    db::MIGRATOR.run_to(4, &seed).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT MAX(version) FROM _sqlx_migrations")
            .fetch_one(&seed)
            .await
            .unwrap(),
        4
    );
    // Used at schema 4: a retention rule deleted the burned tombstone, so
    // `0004`'s trigger retired its id.
    sqlx::query("DELETE FROM pastes WHERE id = ?1")
        .bind(id(BURNED).encode())
        .execute(&seed)
        .await
        .unwrap();
    seed.close().await;

    let pool = db::connect(&config)
        .await
        .expect("migration 0005 must apply to a schema-4 database");
    // Pinned to 5: this test covers migration 0005 only; a later migration
    // needs its own test.
    assert_eq!(db::expected_version(), 5, "a migration after 0005 needs its own upgrade test");
    assert_eq!(db::verify(&pool).await.unwrap(), 5);
    assert_eq!(creation_token_count(&pool).await, 0);

    let retired: Vec<String> = sqlx::query_scalar("SELECT id FROM retired_ids").fetch_all(&pool).await.unwrap();
    assert_eq!(retired, [id(BURNED).encode()]);

    let pastes = SqlitePasteRepository::new(pool.clone());
    let active = pastes.find(&id(ACTIVE)).await.unwrap().expect("the active paste must load");
    assert!(
        active
            .reveal_verifier
            .matches(&Verifier::for_reveal_token(&token("reveal", ACTIVE)))
    );
    assert!(
        active
            .delete_verifier
            .matches(&Verifier::for_delete_secret(&token("delete", ACTIVE)))
    );
    assert_eq!(active.ciphertext, Some(ciphertext_of(ACTIVE)));
    let reused = NewPaste {
        id: id(BURNED),
        format_version: 1,
        public_header: vec![0xAA],
        ciphertext: vec![1, 2, 3],
        reveal_verifier: Verifier::for_reveal_token(b"reveal"),
        delete_verifier: Verifier::for_delete_secret(b"delete"),
        describe_verifier: Verifier::for_describe_token(b"describe"),
        created_at: NOW,
        expires_at: None,
        burn_after_retrieval: false,
    };
    assert!(matches!(pastes.create(&reused).await, Err(RepositoryError::IdConflict)));

    let reports = SqliteAbuseRepository::new(pool.clone()).list_reports(None, 10).await.unwrap();
    assert_eq!(reports.len(), 1);

    let tokens = SqliteCreationTokenRepository::new(pool.clone());
    let label = Label::parse("after the upgrade").unwrap();
    tokens.create(&Verifier::for_creation_token(b"token"), &label, NOW).await.unwrap();
    assert_eq!(tokens.active().await.unwrap().len(), 1);

    // Running the migrations again on the upgraded file changes nothing.
    db::MIGRATOR.run(&pool).await.unwrap();
}

/// AC1 (package 20261122-creation-tokens-db): after the upgrade from the 1.0
/// fixture, `creation_tokens` has exactly the specified columns, and
/// `verifier` is `NOT NULL` and unique: a second row with the same verifier,
/// or one without a verifier, is refused.
#[tokio::test]
async fn creation_tokens_has_exactly_the_specified_columns() {
    let h = harness_from_dump("upgrade-token-columns", DUMP, NOW).await;

    let columns: Vec<(String, i64)> = sqlx::query_as("SELECT name, \"notnull\" FROM pragma_table_info('creation_tokens') ORDER BY cid")
        .fetch_all(&h.pool)
        .await
        .unwrap();
    let names: Vec<&str> = columns.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["id", "verifier", "label", "created_at", "last_used_on", "revoked_at"]);
    let verifier_not_null = columns.iter().find(|(name, _)| name == "verifier").unwrap().1;
    assert_eq!(verifier_not_null, 1, "verifier must be NOT NULL");

    // A unique index covering exactly `verifier`.
    let unique_indexes: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_index_list('creation_tokens') WHERE \"unique\" = 1")
        .fetch_all(&h.pool)
        .await
        .unwrap();
    let mut verifier_unique = false;
    for index in unique_indexes {
        let indexed: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_index_info(?1)")
            .bind(&index)
            .fetch_all(&h.pool)
            .await
            .unwrap();
        verifier_unique |= indexed == ["verifier"];
    }
    assert!(verifier_unique, "no unique index on verifier alone");

    // The constraints hold in practice too.
    let insert = |id: &'static str, verifier: Option<Vec<u8>>| {
        sqlx::query("INSERT INTO creation_tokens (id, verifier, label, created_at) VALUES (?1, ?2, '', 1)")
            .bind(id)
            .bind(verifier)
            .execute(&h.pool)
    };
    insert("01ARZ3NDEKTSV4RRFFQ69G5FAV", Some(vec![7; 32])).await.unwrap();
    assert!(
        insert("01ARZ3NDEKTSV4RRFFQ69G5FAW", Some(vec![7; 32])).await.is_err(),
        "duplicate verifier accepted"
    );
    assert!(
        insert("01ARZ3NDEKTSV4RRFFQ69G5FAX", None).await.is_err(),
        "missing verifier accepted"
    );
    assert_eq!(creation_token_count(&h.pool).await, 1);
}
