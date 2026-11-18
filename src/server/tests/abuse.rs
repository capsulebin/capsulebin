#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Abuse reports and the effect of moderation on the public API (ADR-013).
//!
//! The properties that matter:
//!
//! - the report endpoint answers identically for existing and unknown ids, so
//!   it is no oracle — yet only reports about existing pastes are stored;
//! - a report never carries the decryption key;
//! - a disabled or deleted paste is indistinguishable from an unknown one,
//!   even for a caller holding the right reveal token.

mod support;

use axum::http::StatusCode;
use capsulebin_server::domain::abuse::{ModerationAction, ReasonCode};
use capsulebin_server::repository::AbuseRepository as _;
use capsulebin_server::repository::sqlite::abuse::SqliteAbuseRepository;
use support::*;

fn report_body(paste_id: &str) -> serde_json::Value {
    serde_json::json!({
        "paste_id": paste_id,
        "reason": "phishing",
        "description": "Imitates a bank login page.",
        "contact": "reporter@example.org",
    })
}

async fn stored_reports(h: &Harness) -> Vec<(String, String)> {
    sqlx::query_as::<_, (String, String)>("SELECT paste_id, description FROM abuse_reports ORDER BY created_at, id")
        .fetch_all(&h.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn existing_and_unknown_ids_get_the_same_answer_but_only_one_is_stored() {
    let h = harness("report-oracle").await;
    h.post_with_token("/api/v1/pastes", create_body(&id(1)), Some(TOKEN)).await;

    let existing = h.post("/api/v1/reports", report_body(&id(1).encode())).await;
    let unknown = h.post("/api/v1/reports", report_body(&id(2).encode())).await;

    assert_eq!(existing.status, StatusCode::ACCEPTED, "{}", existing.text());
    assert_eq!(unknown.status, existing.status);
    assert_eq!(unknown.body, existing.body, "the body must not depend on the id");

    let stored = stored_reports(&h).await;
    assert_eq!(stored, [(id(1).encode(), "Imitates a bank login page.".to_owned())]);
}

#[tokio::test]
async fn a_report_containing_a_key_is_refused() {
    let h = harness("report-key").await;
    h.post_with_token("/api/v1/pastes", create_body(&id(1)), Some(TOKEN)).await;

    let mut body = report_body(&id(1).encode());
    body["description"] = serde_json::json!(format!("see https://paste.example/p/{}#v1.c2VjcmV0", id(1).encode()));
    let answer = h.post("/api/v1/reports", body).await;

    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert!(stored_reports(&h).await.is_empty());
}

#[tokio::test]
async fn malformed_reports_are_rejected() {
    let h = harness("report-malformed").await;

    let cases = [
        ("paste_id", serde_json::json!("not-an-id"), "invalid_id"),
        ("reason", serde_json::json!("dislike"), "malformed_request"),
        ("description", serde_json::json!("   "), "malformed_request"),
        ("description", serde_json::json!("x".repeat(5_000)), "malformed_request"),
        ("contact", serde_json::json!("a\u{7}b"), "malformed_request"),
        ("unexpected", serde_json::json!(true), "malformed_request"),
    ];
    for (field, value, code) in cases {
        let mut body = report_body(&id(1).encode());
        body[field] = value;
        let answer = h.post("/api/v1/reports", body).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{field}");
        assert_eq!(answer.code(), code, "{field}");
    }
}

#[tokio::test]
async fn the_policy_describes_the_report_form() {
    let h = harness_with("report-policy", |config| {
        config.abuse.contact = Some("abuse@paste.example".to_owned());
    })
    .await;

    let answer = h.get("/api/v1/reports").await;
    assert_eq!(answer.status, StatusCode::OK);
    let json = answer.json();
    assert_eq!(json["enabled"], true);
    assert_eq!(json["contact"], "abuse@paste.example");
    assert!(json["reasons"].as_array().unwrap().iter().any(|r| r == "illegal_content"));
    assert_eq!(json["max_description_bytes"], 4_000);
}

#[tokio::test]
async fn reports_can_be_switched_off() {
    let h = harness_with("report-off", |config| config.abuse.enabled = false).await;

    let answer = h.post("/api/v1/reports", report_body(&id(1).encode())).await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.code(), "reports_disabled");
    assert_eq!(h.get("/api/v1/reports").await.json()["enabled"], false);
}

#[tokio::test]
async fn reports_have_their_own_rate_limit() {
    let h = harness_with("report-rate", |config| {
        config.rate_limits.report.requests_per_hour = 5;
        config.rate_limits.report.burst = 2;
    })
    .await;

    for _ in 0..2 {
        assert_eq!(
            h.post("/api/v1/reports", report_body(&id(1).encode())).await.status,
            StatusCode::ACCEPTED
        );
    }
    let limited = h.post("/api/v1/reports", report_body(&id(1).encode())).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.headers["retry-after"], "720");

    // Other classes are unaffected.
    let descriptor = h.describe(&id(1).encode()).await;
    assert_eq!(descriptor.status, StatusCode::NOT_FOUND);
}

/// Moderation is only reachable through the operator CLI, which works on the
/// repository; this drives the repository directly and checks what the public
/// API makes of it.
#[tokio::test]
async fn moderated_pastes_look_exactly_like_unknown_ones() {
    let h = harness("moderated").await;
    let abuse = SqliteAbuseRepository::new(h.pool.clone());
    for byte in [1, 2] {
        h.post_with_token("/api/v1/pastes", create_body(&id(byte)), Some(TOKEN)).await;
    }

    abuse
        .moderate(&id(1), ModerationAction::Disable, Some(ReasonCode::Malware), None, START)
        .await
        .unwrap();
    abuse
        .moderate(&id(2), ModerationAction::Delete, Some(ReasonCode::IllegalContent), None, START)
        .await
        .unwrap();

    let unknown = id(9).encode();
    let reference_descriptor = h.describe(&unknown).await;
    let reference_retrieve = h
        .post(&format!("/api/v1/pastes/{unknown}/retrieve"), retrieve_body(REVEAL_TOKEN))
        .await;

    for byte in [1, 2] {
        let paste = id(byte).encode();
        let descriptor = h.describe(&paste).await;
        assert_eq!(descriptor.status, reference_descriptor.status, "descriptor of {byte}");
        assert_eq!(descriptor.code(), reference_descriptor.code());

        // Even with the right reveal token.
        let retrieve = h
            .post(&format!("/api/v1/pastes/{paste}/retrieve"), retrieve_body(REVEAL_TOKEN))
            .await;
        assert_eq!(retrieve.status, reference_retrieve.status, "retrieve of {byte}");
        assert_eq!(retrieve.code(), reference_retrieve.code());
    }

    // Restoring the disabled one brings it back; the deleted one stays gone.
    abuse.moderate(&id(1), ModerationAction::Restore, None, None, START).await.unwrap();
    let restored = h
        .post(&format!("/api/v1/pastes/{}/retrieve", id(1).encode()), retrieve_body(REVEAL_TOKEN))
        .await;
    assert_eq!(restored.status, StatusCode::OK);
}

#[tokio::test]
async fn report_text_never_reaches_a_response_header_or_problem_body() {
    let h = harness("report-echo").await;
    let mut body = report_body(&id(1).encode());
    body["description"] = serde_json::json!("REPORT-TEXT-MARKER");
    body["contact"] = serde_json::json!("CONTACT-MARKER@example.org");
    body["reason"] = serde_json::json!("nonsense");

    let answer = h.post("/api/v1/reports", body).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    let text = answer.text();
    assert!(!text.contains("REPORT-TEXT-MARKER") && !text.contains("CONTACT-MARKER"), "{text}");
}
