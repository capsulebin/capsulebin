#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Opening a database written by CapsuleBin 1.0.
//!
//! `fixtures/capsulebin-1.0.sql` is a dump of a database the 1.0 binary wrote:
//! pastes in every state that matters, a report and a moderation action, a
//! retired id and creation tokens. These tests load it, let this build's
//! migrations run as an upgrade would, and use the data through the HTTP API
//! and the repositories.
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
//! After the 1.0 tag the fixture is never regenerated. A later release adds a
//! dump of its own next to it, so every supported starting point stays tested.

mod support;

use axum::http::StatusCode;
use capsulebin_server::db;
use capsulebin_server::domain::abuse::ModerationAction;
use capsulebin_server::domain::{Availability, PurgePolicy};
use capsulebin_server::repository::sqlite::SqlitePasteRepository;
use capsulebin_server::repository::sqlite::abuse::SqliteAbuseRepository;
use capsulebin_server::repository::sqlite::creation_tokens::SqliteCreationTokenRepository;
use capsulebin_server::repository::{AbuseRepository as _, CreationTokenRepository as _, ModerationOutcome, PasteRepository as _};
use support::*;

const DUMP: &str = include_str!("fixtures/capsulebin-1.0.sql");

/// An hour after the fixture was written, inside its pastes' 7-day expiry.
const NOW: i64 = 1_795_363_450;

const ACTIVE: u8 = 0x10;
const BURNED: u8 = 0x11;
const PASSWORD: u8 = 0x12;
const DISABLED: u8 = 0x13;
/// Expired and removed by `prune` before the dump: only its id is left.
const RETIRED: u8 = 0x14;

/// The fixture's active database creation token.
const ACTIVE_TOKEN_ID: &str = "01M81WJGCMV3655M9FDWASE284";
const ACTIVE_TOKEN: &str = "4ZB7KtpKEH2_UScS3jlFAieRicESHEACLOPRl0t64yc";
/// The fixture's revoked database creation token.
const REVOKED_TOKEN: &str = "ux393DSUXF2Ulm9n065lWrO-Sdxx87632f45J5xGh0g";

/// The fixture's paste tokens are `<kind>-<id byte in hex>`.
fn token(kind: &str, byte: u8) -> Vec<u8> {
    format!("{kind}-{byte:02x}").into_bytes()
}

fn ciphertext_of(byte: u8) -> Vec<u8> {
    let mut bytes = vec![0x00, 0xFF];
    bytes.extend_from_slice(format!("fixture-ciphertext-{byte:02x}").as_bytes());
    bytes
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

/// The checksum 1.0 recorded for `0001_init.sql` is the checksum of the
/// migration this build embeds. sqlx would refuse the database otherwise; this
/// test says why before any other test here fails on that refusal.
#[tokio::test]
async fn the_fixture_records_the_checksum_of_the_initial_migration() {
    let dir = tempdir("upgrade-checksum");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(dir.join("capsulebin.db"))
        .create_if_missing(true);
    let seed = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::raw_sql(DUMP).execute(&seed).await.unwrap();

    let recorded: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&seed)
        .await
        .unwrap();
    let versions: Vec<i64> = recorded.iter().map(|(version, _)| *version).collect();
    assert_eq!(versions, [1], "the 1.0 fixture records exactly migration 1");

    let embedded = db::MIGRATOR
        .migrations
        .iter()
        .find(|migration| migration.version == 1)
        .expect("this build embeds migration 1");
    assert!(
        recorded[0].1 == embedded.checksum.as_ref(),
        "migrations/0001_init.sql differs from the migration 1.0 applied: every 1.0 database \
         would be refused. Never edit a released migration; put the change into a new one."
    );
}

#[tokio::test]
async fn a_1_0_database_opens_at_the_current_schema() {
    let h = harness_from_dump("upgrade-schema", DUMP, NOW).await;

    assert_eq!(db::verify(&h.pool).await.unwrap(), db::expected_version());
    assert_eq!(h.get("/health/ready").await.status, StatusCode::OK);

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

/// The 1.0 database tokens keep their state: the active one authorizes a
/// create in `token` mode, the revoked one is refused like an unknown token.
#[tokio::test]
async fn creation_tokens_from_1_0_keep_their_state() {
    let h = harness_from_dump("upgrade-creation-tokens", DUMP, NOW).await;

    let active = SqliteCreationTokenRepository::new(h.pool.clone()).active().await.unwrap();
    let active_ids: Vec<&str> = active.iter().map(|token| token.id.as_str()).collect();
    assert_eq!(active_ids, [ACTIVE_TOKEN_ID]);

    let created = h.post_with_token("/api/v1/pastes", create_body(&id(1)), Some(ACTIVE_TOKEN)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());

    let refused = h.post_with_token("/api/v1/pastes", create_body(&id(2)), Some(REVOKED_TOKEN)).await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED, "{}", refused.text());
    assert_eq!(refused.code(), "unauthorized");
}

/// The id 1.0's `prune` removed stays retired, the 1.0 tombstones keep
/// refusing their ids, and every 1.0 row a purge removes is retired as well.
#[tokio::test]
async fn purged_fixture_ids_stay_retired() {
    let h = harness_from_dump("upgrade-retired-ids", DUMP, NOW).await;
    let fixture = [ACTIVE, BURNED, PASSWORD, DISABLED];

    let retired = || async {
        sqlx::query_scalar::<_, String>("SELECT id FROM retired_ids ORDER BY id")
            .fetch_all(&h.pool)
            .await
            .unwrap()
    };
    assert_eq!(retired().await, [id(RETIRED).encode()]);

    for byte in [RETIRED, BURNED] {
        let answer = h.post("/api/v1/pastes", create_body(&id(byte))).await;
        assert_eq!(answer.status, StatusCode::CONFLICT, "{byte:#x}: {}", answer.text());
        assert_eq!(answer.code(), "id_conflict");
    }

    let policy = PurgePolicy {
        never_retrieved_after: 86_400,
        inactive_after: 86_400,
    };
    let outcome = SqlitePasteRepository::new(h.pool.clone())
        .purge(NOW + 365 * 86_400, &policy, 100)
        .await
        .unwrap();
    assert_eq!(outcome.expired + outcome.never_retrieved + outcome.inactive, fixture.len() as u64);

    let mut expected: Vec<String> = fixture.iter().chain([&RETIRED]).map(|&byte| id(byte).encode()).collect();
    expected.sort();
    assert_eq!(retired().await, expected);

    for byte in fixture {
        let answer = h.post("/api/v1/pastes", create_body(&id(byte))).await;
        assert_eq!(answer.status, StatusCode::CONFLICT, "{byte:#x}: {}", answer.text());
        assert_eq!(answer.code(), "id_conflict");
    }
}

/// AC7: the fixture holds what the 1.0 binary was asked to write, read
/// straight from its rows: exactly migration 1, the four pastes in their
/// states, the pruned paste only as a retired id, one closed report, and one
/// active and one revoked database creation token.
#[tokio::test]
async fn the_fixture_holds_every_state_it_was_written_with() {
    let dir = tempdir("upgrade-fixture-contents");
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(dir.join("capsulebin.db"))
        .create_if_missing(true);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::raw_sql(DUMP).execute(&pool).await.unwrap();

    let versions: Vec<i64> = sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(versions, [1]);

    type Row = (String, String, i64, Option<i64>, bool, bool);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, availability_status, burn_after_retrieval, consumed_at, \
         ciphertext IS NOT NULL, instr(CAST(public_header AS TEXT), '\"password\"') > 0 \
         FROM pastes ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let state = |byte: u8| {
        rows.iter()
            .find(|row| row.0 == id(byte).encode())
            .unwrap_or_else(|| panic!("no row for {byte:#x}"))
            .clone()
    };
    assert_eq!(rows.len(), 4, "{rows:?}");
    let (_, status, burn, consumed, payload, password) = state(ACTIVE);
    assert_eq!(
        (status.as_str(), burn, consumed, payload, password),
        ("active", 0, None, true, false)
    );
    let (_, status, burn, consumed, payload, _) = state(BURNED);
    assert_eq!((status.as_str(), burn, payload), ("active", 1, false));
    assert!(consumed.is_some(), "the burned paste is a tombstone");
    let (_, status, _, _, payload, password) = state(PASSWORD);
    assert_eq!((status.as_str(), payload, password), ("active", true, true));
    let (_, status, _, _, payload, _) = state(DISABLED);
    assert_eq!((status.as_str(), payload), ("disabled", true));

    let retired: Vec<String> = sqlx::query_scalar("SELECT id FROM retired_ids").fetch_all(&pool).await.unwrap();
    assert_eq!(retired, [id(RETIRED).encode()]);
    assert!(rows.iter().all(|row| row.0 != id(RETIRED).encode()), "the retired id has no row");

    let reports: Vec<(String, String)> = sqlx::query_as("SELECT paste_id, status FROM abuse_reports")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(reports, [(id(DISABLED).encode(), "closed".to_owned())]);

    let tokens: Vec<(String, bool)> =
        sqlx::query_as("SELECT id, revoked_at IS NOT NULL FROM creation_tokens ORDER BY revoked_at IS NOT NULL")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(tokens.len(), 2, "{tokens:?}");
    assert_eq!(tokens[0], (ACTIVE_TOKEN_ID.to_owned(), false));
    assert!(tokens[1].1, "the second token is revoked");
}
