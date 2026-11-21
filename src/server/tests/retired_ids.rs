#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Ids are never freed (`migrations/0004_retired_ids.sql`, REL-005).
//!
//! The client chooses the id (ADR-011). If a purge freed it, anyone holding the
//! full link could create a new capsule under the same id and link secret, and
//! the old link would open different content. Reports and moderation, which
//! name a paste by its id, would attach to that later capsule. These tests purge
//! through the repository with a chosen `now` and then try both over HTTP.

mod support;

use axum::http::StatusCode;
use capsulebin_server::domain::abuse::ModerationAction;
use capsulebin_server::domain::{PasteId, PurgeOutcome, PurgePolicy};
use capsulebin_server::repository::sqlite::SqlitePasteRepository;
use capsulebin_server::repository::sqlite::abuse::SqliteAbuseRepository;
use capsulebin_server::repository::{AbuseRepository as _, ModerationOutcome, PasteRepository as _};
use support::*;

const HOUR: i64 = 3_600;

/// Thresholds no test reaches unless it means to.
const NEVER: i64 = 100 * 365 * 86_400;

/// A create for `id` with the harness's link secret (the same describe, reveal
/// and delete verifiers) but a different public header and ciphertext: what a
/// link holder re-seating the link with new content would send.
fn reseating_body(id: &PasteId) -> serde_json::Value {
    let mut body = create_body(id);
    body["public_header"] = serde_json::json!(b64(&[0x11, 0x22, 0x33]));
    body["ciphertext"] = serde_json::json!(b64(b"different content"));
    body
}

/// A problem body without its request id, which is unique per request.
fn without_request_id(answer: &Answer) -> serde_json::Value {
    let mut json = answer.json();
    json["request_id"] = serde_json::json!("");
    json
}

async fn purge(h: &Harness, now: i64, policy: PurgePolicy) -> PurgeOutcome {
    SqlitePasteRepository::new(h.pool.clone())
        .purge(now, &policy, 100)
        .await
        .expect("purge")
}

#[tokio::test]
async fn a_purged_id_cannot_be_created_again() {
    // (rule, expires_in, retrieved, policy, expected outcome)
    let cases = [
        (
            "expired",
            HOUR,
            false,
            PurgePolicy {
                never_retrieved_after: NEVER,
                inactive_after: NEVER,
            },
            PurgeOutcome {
                expired: 1,
                never_retrieved: 0,
                inactive: 0,
            },
        ),
        (
            "never-retrieved",
            86_400,
            false,
            PurgePolicy {
                never_retrieved_after: HOUR,
                inactive_after: NEVER,
            },
            PurgeOutcome {
                expired: 0,
                never_retrieved: 1,
                inactive: 0,
            },
        ),
        (
            "inactive",
            86_400,
            true,
            PurgePolicy {
                never_retrieved_after: NEVER,
                inactive_after: HOUR,
            },
            PurgeOutcome {
                expired: 0,
                never_retrieved: 0,
                inactive: 1,
            },
        ),
    ];

    for (rule, expires_in, retrieved, policy, expected) in cases {
        let h = harness(&format!("retired-ids-{rule}")).await;

        // The reference: re-seating an id whose row is still there.
        let live = id(0x50);
        assert_eq!(h.post("/api/v1/pastes", create_body(&live)).await.status, StatusCode::CREATED);
        let live_conflict = h.post("/api/v1/pastes", reseating_body(&live)).await;
        assert_eq!(live_conflict.status, StatusCode::CONFLICT, "{rule}: {}", live_conflict.text());
        assert_eq!(live_conflict.code(), "id_conflict");

        let purged = id(0x51);
        let mut body = create_body(&purged);
        body["expires_in"] = serde_json::json!(expires_in);
        let created = h.post("/api/v1/pastes", body).await;
        assert_eq!(created.status, StatusCode::CREATED, "{rule}: {}", created.text());
        if retrieved {
            let path = format!("/api/v1/pastes/{}/retrieve", purged.encode());
            let answer = h.post_with_token(&path, retrieve_body(REVEAL_TOKEN), None).await;
            assert_eq!(answer.status, StatusCode::OK, "{rule}: {}", answer.text());
        }

        // The reference paste is purged by the same run; keep it out of the count.
        sqlx::query("UPDATE pastes SET expires_at = NULL, last_retrieved_at = ?2 WHERE id = ?1")
            .bind(live.encode())
            .bind(START + HOUR)
            .execute(&h.pool)
            .await
            .unwrap();
        assert_eq!(purge(&h, START + HOUR, policy).await, expected, "{rule}");

        let conflict = h.post("/api/v1/pastes", reseating_body(&purged)).await;
        assert_eq!(conflict.status, StatusCode::CONFLICT, "{rule}: {}", conflict.text());
        assert_eq!(without_request_id(&conflict), without_request_id(&live_conflict), "{rule}");
        assert_eq!(stable_headers(&conflict), stable_headers(&live_conflict), "{rule}");

        // Neither a replay of the original request nor anything else brings it back.
        let replay = h.post("/api/v1/pastes", create_body(&purged)).await;
        assert_eq!(replay.status, StatusCode::CONFLICT, "{rule}: {}", replay.text());

        let old_link = h.describe(&purged.encode()).await;
        let unknown = h.describe(&id(0x52).encode()).await;
        assert_eq!(old_link.status, StatusCode::NOT_FOUND, "{rule}: {}", old_link.text());
        assert_eq!(without_request_id(&old_link), without_request_id(&unknown), "{rule}");
    }
}

#[tokio::test]
async fn a_report_on_a_purged_paste_never_reaches_a_new_capsule() {
    let h = harness("retired-ids-report").await;
    let abuse = SqliteAbuseRepository::new(h.pool.clone());
    let paste = id(0x60);

    let report = serde_json::json!({
        "paste_id": paste.encode(),
        "reason": "phishing",
        "description": "Imitates a bank login page.",
    });
    assert_eq!(h.post("/api/v1/pastes", create_body(&paste)).await.status, StatusCode::CREATED);
    assert_eq!(h.post("/api/v1/reports", report.clone()).await.status, StatusCode::ACCEPTED);

    let policy = PurgePolicy {
        never_retrieved_after: HOUR,
        inactive_after: NEVER,
    };
    assert_eq!(purge(&h, START + HOUR, policy).await.never_retrieved, 1);

    let recreated = h.post("/api/v1/pastes", reseating_body(&paste)).await;
    assert_eq!(recreated.status, StatusCode::CONFLICT, "{}", recreated.text());
    assert_eq!(recreated.code(), "id_conflict");

    // `reports list` and `reports show` print a missing paste status as `gone`.
    let reports = abuse.list_reports(None, 10).await.unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].report.paste_id, paste.encode());
    assert_eq!(reports[0].paste_status, None);
    let shown = abuse.find_report(&reports[0].report.id).await.unwrap().expect("the report is kept");
    assert_eq!(shown.paste_status, None);

    for action in [ModerationAction::Disable, ModerationAction::Delete] {
        let outcome = abuse.moderate(&paste, action, None, None, START + HOUR).await.unwrap();
        assert_eq!(outcome, ModerationOutcome::NotFound, "{action:?}");
    }
    assert!(abuse.moderation_history(&paste).await.unwrap().is_empty());
    let actions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM moderation_actions")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(actions, 0);

    let again = h.post("/api/v1/reports", report).await;
    assert_eq!(again.status, StatusCode::ACCEPTED, "{}", again.text());
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM abuse_reports")
        .fetch_one(&h.pool)
        .await
        .unwrap();
    assert_eq!(stored, 1, "a report on a retired id is not stored");
}
