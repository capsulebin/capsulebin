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
use capsulebin_server::db;
use capsulebin_server::domain::Availability;
use capsulebin_server::domain::abuse::ModerationAction;
use capsulebin_server::repository::sqlite::SqlitePasteRepository;
use capsulebin_server::repository::sqlite::abuse::SqliteAbuseRepository;
use capsulebin_server::repository::{AbuseRepository as _, ModerationOutcome, PasteRepository as _};
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
