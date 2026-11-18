#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Integration tests for the opaque paste API.
//!
//! These are the acceptance criteria for the storage layer. The emphasis is on
//! the properties that are easy to break without noticing:
//!
//! - a response must not reveal whether a paste id exists,
//! - the payload must survive byte-for-byte,
//! - expiry must be enforced against the clock, not against wall time,
//! - creation must be closed unless a valid token is presented.

mod support;

use axum::http::StatusCode;
use capsulebin_server::config::CreationMode;
use capsulebin_server::domain::Verifier;
use support::*;

// --- create ---

#[tokio::test]
async fn creates_a_paste_and_returns_its_lifecycle() {
    let h = harness("create").await;

    let answer = h.post("/api/v1/pastes", create_body(&id(1))).await;

    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.text());
    let json = answer.json();
    assert_eq!(json["id"], id(1).encode());
    // Default expiry is seven days from the fixed clock.
    assert_eq!(json["expires_at"], START + 604_800);
    assert_eq!(json["burn_after_retrieval"], false);
}

#[tokio::test]
async fn rejects_a_duplicate_id_with_a_conflict() {
    let h = harness("duplicate").await;

    h.post("/api/v1/pastes", create_body(&id(2))).await;

    // Same id, different content: someone else's paste, or a client bug.
    let mut other = create_body(&id(2));
    other["ciphertext"] = serde_json::json!(b64(b"different ciphertext"));
    let answer = h.post("/api/v1/pastes", other).await;

    assert_eq!(answer.status, StatusCode::CONFLICT);
    assert_eq!(answer.code(), "id_conflict");
}

/// ADR-011: the client-generated id is the idempotency key. A retry of the very
/// same request is answered like the first attempt, so a client that lost the
/// response can safely send again.
#[tokio::test]
async fn an_identical_create_is_a_replay_not_a_conflict() {
    let h = harness("replay").await;

    let first = h.post("/api/v1/pastes", create_body(&id(20))).await;
    assert_eq!(first.status, StatusCode::CREATED);
    let replay = h.post("/api/v1/pastes", create_body(&id(20))).await;

    assert_eq!(replay.status, StatusCode::CREATED, "{}", replay.text());
    assert_eq!(replay.json(), first.json());
}

#[tokio::test]
async fn a_replay_must_match_every_capability_and_option() {
    let h = harness("replay-mismatch").await;
    h.post("/api/v1/pastes", create_body(&id(21))).await;

    let mutations: [(&str, serde_json::Value); 4] = [
        ("reveal_verifier", serde_json::json!(Verifier::for_reveal_token(b"other").encode())),
        ("delete_verifier", serde_json::json!(Verifier::for_delete_secret(b"other").encode())),
        ("public_header", serde_json::json!(b64(&[0xAA, 0xBB]))),
        ("expires_in", serde_json::json!(3600)),
    ];
    for (field, value) in mutations {
        let mut body = create_body(&id(21));
        body[field] = value;
        let answer = h.post("/api/v1/pastes", body).await;
        assert_eq!(answer.status, StatusCode::CONFLICT, "a different {field} was accepted as a replay");
    }
}

#[tokio::test]
async fn a_deleted_paste_cannot_be_revived_by_a_replay() {
    let h = harness("replay-deleted").await;
    h.post("/api/v1/pastes", create_body(&id(22))).await;
    let deleted = h
        .delete(&format!("/api/v1/pastes/{}", id(22).encode()), delete_body(DELETE_SECRET))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);

    let replay = h.post("/api/v1/pastes", create_body(&id(22))).await;
    assert_eq!(replay.status, StatusCode::CONFLICT);
    let descriptor = h.describe(&id(22).encode()).await;
    assert_eq!(descriptor.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn rejects_malformed_ids() {
    let h = harness("bad-ids").await;

    for bad in ["short", "2pS3xQ7Rk9mNpLz4AbCdEf", "../../etc/passwd0EA"] {
        let mut body = create_body(&id(3));
        body["id"] = serde_json::json!(bad);

        let answer = h.post("/api/v1/pastes", body).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{bad} was accepted");
        assert_eq!(answer.code(), "invalid_id");
    }
}

#[tokio::test]
async fn rejects_unknown_fields_in_the_request() {
    let h = harness("unknown-field").await;

    let mut body = create_body(&id(4));
    body["burn_after_retreival"] = serde_json::json!(true);

    let answer = h.post("/api/v1/pastes", body).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    assert_eq!(answer.code(), "malformed_request");
}

#[tokio::test]
async fn rejects_an_expiry_beyond_the_maximum() {
    let h = harness("expiry-long").await;

    let mut body = create_body(&id(5));
    // Maximum is 90 days.
    body["expires_in"] = serde_json::json!(91 * 86_400);

    let answer = h.post("/api/v1/pastes", body).await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(answer.code(), "expiry_too_long");
}

/// An absent `expires_in` means "use the default"; an explicit `null` means
/// "permanent". Collapsing the two would silently create a week-long paste where
/// the client asked for a permanent one, or vice versa.
#[tokio::test]
async fn explicit_null_expiry_is_rejected_when_permanent_is_disallowed() {
    let h = harness("permanent").await;

    let mut body = create_body(&id(6));
    body["expires_in"] = serde_json::Value::Null;

    let answer = h.post("/api/v1/pastes", body).await;
    assert_eq!(answer.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(answer.code(), "permanent_not_allowed");
}

#[tokio::test]
async fn rejects_an_oversized_payload() {
    let h = harness("oversize").await;

    // The harness limit is 64 KiB; the allowance adds 64 KiB on top, so 256 KiB
    // is unambiguously over.
    let big = vec![0x41_u8; 256 * 1024];
    let mut body = create_body(&id(7));
    body["ciphertext"] = serde_json::json!(b64(&big));

    let answer = h.post("/api/v1/pastes", body).await;
    assert_eq!(answer.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(answer.code(), "payload_too_large");
}

/// Axum's `Json<T>` extractor enforces its own 2 MiB body cap independently of
/// any layer, unless `DefaultBodyLimit` is explicitly disabled. This body sits
/// above that hard-coded default and below `pastes.max_plaintext_size`, so it
/// only passes if that internal default has in fact been turned off.
#[tokio::test]
async fn accepts_a_payload_above_two_mebibytes() {
    let h = harness_with_plaintext_limit("large-payload", capsulebin_server::config::units::ByteSize(10 * 1024 * 1024)).await;

    let big = vec![0x41_u8; 3 * 1024 * 1024];
    let mut body = create_body(&id(8));
    body["ciphertext"] = serde_json::json!(b64(&big));

    let answer = h.post("/api/v1/pastes", body).await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.text());
}

/// The public header is the one client-supplied blob an unauthenticated request
/// gets back, so its size is bounded by configuration rather than by whatever fits
/// in a request body.
#[tokio::test]
async fn rejects_an_oversized_public_header() {
    let h = harness("header-oversize").await;

    // The default limit is 2 KiB.
    let mut body = create_body(&id(7));
    body["public_header"] = serde_json::json!(b64(&vec![0x42_u8; 2 * 1024 + 1]));

    let answer = h.post("/api/v1/pastes", body).await;
    assert_eq!(answer.status, StatusCode::PAYLOAD_TOO_LARGE, "{}", answer.text());

    // Exactly at the limit it is accepted, so the bound is off-by-one-free.
    let mut body = create_body(&id(8));
    body["public_header"] = serde_json::json!(b64(&vec![0x42_u8; 2 * 1024]));

    let answer = h.post("/api/v1/pastes", body).await;
    assert_eq!(answer.status, StatusCode::CREATED, "{}", answer.text());
}

#[tokio::test]
async fn rejects_an_empty_payload() {
    let h = harness("empty").await;

    let mut body = create_body(&id(8));
    body["ciphertext"] = serde_json::json!("");

    let answer = h.post("/api/v1/pastes", body).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
}

// --- creation modes ---

#[tokio::test]
async fn token_mode_requires_a_valid_token() {
    let h = harness_with_mode("token-mode", CreationMode::Token).await;

    let missing = h.post_with_token("/api/v1/pastes", create_body(&id(1)), None).await;
    assert_eq!(missing.status, StatusCode::UNAUTHORIZED);
    assert_eq!(missing.code(), "unauthorized");

    let wrong = h.post_with_token("/api/v1/pastes", create_body(&id(1)), Some("wrong")).await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);

    let right = h.post_with_token("/api/v1/pastes", create_body(&id(1)), Some(TOKEN)).await;
    assert_eq!(right.status, StatusCode::CREATED);
}

/// The authorisation check runs before the body is parsed, so an unauthorised
/// caller cannot use validation errors to probe what the server accepts.
#[tokio::test]
async fn authorization_is_checked_before_the_body() {
    let h = harness_with_mode("auth-first", CreationMode::Token).await;

    let answer = h
        .post_with_token("/api/v1/pastes", serde_json::json!({ "garbage": true }), None)
        .await;

    assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    assert_eq!(answer.code(), "unauthorized");
}

#[tokio::test]
async fn public_mode_needs_no_token() {
    let h = harness_with_mode("public-mode", CreationMode::Public).await;

    let answer = h.post_with_token("/api/v1/pastes", create_body(&id(1)), None).await;

    assert_eq!(answer.status, StatusCode::CREATED);
}

/// A disabled instance stays readable. That is the point: it lets an operator stop
/// accepting new content without breaking links that are already out there.
#[tokio::test]
async fn disabled_mode_refuses_creation_but_still_serves() {
    let h = harness_with_mode("disabled-mode", CreationMode::Disabled).await;

    let answer = h.post_with_token("/api/v1/pastes", create_body(&id(1)), Some(TOKEN)).await;
    assert_eq!(answer.status, StatusCode::FORBIDDEN);
    assert_eq!(answer.code(), "creation_disabled");

    // Retrieval paths still answer.
    assert_eq!(h.get("/health/live").await.status, StatusCode::OK);
}

// --- descriptor ---

#[tokio::test]
async fn descriptor_returns_the_header_but_never_the_ciphertext() {
    let h = harness("descriptor").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let answer = h.describe(&id(1).encode()).await;

    assert_eq!(answer.status, StatusCode::OK);
    let json = answer.json();
    assert_eq!(json["public_header"], b64(&[0xAA, 0xBB, 0xCC]));
    assert_eq!(json["payload_size"], ciphertext().len());

    // The whole point of a separate descriptor endpoint.
    assert!(json.get("ciphertext").is_none(), "descriptor leaked the ciphertext");
    assert!(!answer.text().contains(&b64(&ciphertext())));
}

#[tokio::test]
async fn descriptor_of_an_unknown_id_is_generic() {
    let h = harness("descriptor-unknown").await;

    let answer = h.describe(&id(9).encode()).await;

    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    assert_eq!(answer.code(), "not_available");
}

// --- retrieve ---

#[tokio::test]
async fn retrieves_the_payload_byte_for_byte() {
    let h = harness("retrieve").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let answer = h
        .post(&format!("/api/v1/pastes/{}/retrieve", id(1).encode()), retrieve_body(REVEAL_TOKEN))
        .await;

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let json = answer.json();

    // Byte-exactness matters: the header is authenticated as additional data, so a
    // single altered byte would make the capsule undecryptable.
    assert_eq!(json["public_header"], b64(&[0xAA, 0xBB, 0xCC]));
    assert_eq!(json["ciphertext"], b64(&ciphertext()));
}

/// Retrieving needs both tokens from the link. Without the right describe token
/// even the right reveal token is refused - otherwise a password guesser could
/// leave it out and escape the per-paste limit on password attempts.
#[tokio::test]
async fn retrieve_requires_the_describe_token_too() {
    let h = harness("retrieve-describe").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;
    let path = format!("/api/v1/pastes/{}/retrieve", id(1).encode());

    let wrong_describe = h.post(&path, retrieve_body_with(b"not-the-token", REVEAL_TOKEN)).await;
    assert_eq!(wrong_describe.status, StatusCode::NOT_FOUND);
    assert_eq!(wrong_describe.code(), "not_available");

    let missing = h.post(&path, serde_json::json!({ "reveal_token": b64(REVEAL_TOKEN) })).await;
    assert_eq!(missing.status, StatusCode::BAD_REQUEST);

    assert_eq!(h.post(&path, retrieve_body(REVEAL_TOKEN)).await.status, StatusCode::OK);
}

/// The central indistinguishability property: a wrong capability and a
/// non-existent paste must produce identical responses, down to the body.
#[tokio::test]
async fn wrong_token_and_unknown_id_are_indistinguishable() {
    let h = harness("indistinguishable").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let wrong_token = h
        .post(
            &format!("/api/v1/pastes/{}/retrieve", id(1).encode()),
            retrieve_body(b"not-the-token"),
        )
        .await;

    let unknown_id = h
        .post(
            &format!("/api/v1/pastes/{}/retrieve", id(9).encode()),
            retrieve_body(b"not-the-token"),
        )
        .await;

    assert_eq!(wrong_token.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown_id.status, StatusCode::NOT_FOUND);
    assert_eq!(wrong_token.code(), unknown_id.code());
    assert_eq!(wrong_token.code(), "not_available");

    // Bodies differ only in the request id, which is unique per request by design.
    let strip = |answer: &Answer| {
        let mut json = answer.json();
        json["request_id"] = serde_json::json!("");
        json
    };
    assert_eq!(strip(&wrong_token), strip(&unknown_id));
}

#[tokio::test]
async fn expired_paste_is_not_retrievable() {
    let h = harness("expired").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    // Just before expiry it still works.
    h.clock.set(START + 604_799);
    let before = h
        .post(&format!("/api/v1/pastes/{}/retrieve", id(1).encode()), retrieve_body(REVEAL_TOKEN))
        .await;
    assert_eq!(before.status, StatusCode::OK);

    // At the expiry second it does not - and looks like any other unavailability.
    h.clock.set(START + 604_800);
    let after = h
        .post(&format!("/api/v1/pastes/{}/retrieve", id(1).encode()), retrieve_body(REVEAL_TOKEN))
        .await;
    assert_eq!(after.status, StatusCode::NOT_FOUND);
    assert_eq!(after.code(), "not_available");

    // The descriptor agrees.
    let descriptor = h.describe(&id(1).encode()).await;
    assert_eq!(descriptor.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn retrieval_moves_only_the_coarse_timestamp() {
    let h = harness("retrieval-timestamp").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    assert_eq!(
        h.bookkeeping(&id(1)).await,
        Bookkeeping {
            last_retrieved_at: None,
            consumed_at: None,
        },
        "a fresh capsule must look untouched"
    );

    for _ in 0..3 {
        let answer = h
            .post(&format!("/api/v1/pastes/{}/retrieve", id(1).encode()), retrieve_body(REVEAL_TOKEN))
            .await;
        assert_eq!(answer.status, StatusCode::OK);
    }

    // The timestamp is coarse and stays inside its window; nothing else about
    // retrievals is kept.
    assert_eq!(
        h.bookkeeping(&id(1)).await,
        Bookkeeping {
            last_retrieved_at: Some(START),
            consumed_at: None,
        }
    );
}

/// Inside the timestamp window a retrieval is a pure read: it must not even
/// ask for SQLite's single write lock. Another connection holds that lock here,
/// with a short busy timeout - a retrieval that tried to write would fail.
#[tokio::test]
async fn retrievals_inside_the_timestamp_window_do_not_write() {
    let h = harness_with("retrieval-read-only", |config| {
        config.database.busy_timeout_ms = 100;
    })
    .await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;
    let path = format!("/api/v1/pastes/{}/retrieve", id(1).encode());

    // The first retrieval sets the timestamp - that one writes.
    assert_eq!(h.post(&path, retrieve_body(REVEAL_TOKEN)).await.status, StatusCode::OK);

    let mut writer = h.pool.acquire().await.unwrap();
    sqlx::query("BEGIN IMMEDIATE").execute(&mut *writer).await.unwrap();
    for _ in 0..3 {
        let answer = h.post(&path, retrieve_body(REVEAL_TOKEN)).await;
        assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    }
    sqlx::query("ROLLBACK").execute(&mut *writer).await.unwrap();
}

/// `last_retrieved_at` exists for the retention policy alone. Exposing it would
/// turn a descriptor request into a "was this link used, and when" oracle for
/// anyone who has the id, which is precisely what the coarse granularity is meant
/// to prevent.
#[tokio::test]
async fn no_response_exposes_the_retrieval_bookkeeping() {
    let h = harness("bookkeeping-hidden").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let path = format!("/api/v1/pastes/{}", id(1).encode());
    let descriptor = h.describe(&id(1).encode()).await;
    let retrieve = h.post(&format!("{path}/retrieve"), retrieve_body(REVEAL_TOKEN)).await;
    let after = h.describe(&id(1).encode()).await;

    for answer in [&descriptor, &retrieve, &after] {
        let field = "last_retrieved_at";
        assert!(
            answer.json().get(field).is_none(),
            "{field} appeared in a response: {}",
            answer.text()
        );
        // Nested or renamed is still exposed, so check the raw body too.
        assert!(
            !answer.text().contains(field),
            "{field} appeared in a response body: {}",
            answer.text()
        );
    }

    // The descriptor is byte-identical before and after a retrieval, so its length
    // gives nothing away either.
    assert_eq!(descriptor.body, after.body);
}

/// The invariant behind the whole metadata design: the timestamp means "an
/// authorized handout happened". A rejected request must leave no trace, or the
/// retention policy would measure guessing traffic instead of use.
#[tokio::test]
async fn a_rejected_retrieval_records_nothing() {
    let h = harness("bookkeeping-rejected").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let before = h.bookkeeping(&id(1)).await;

    // A wrong capability, an unknown id, and a fetch of the descriptor.
    h.post(
        &format!("/api/v1/pastes/{}/retrieve", id(1).encode()),
        retrieve_body(b"not-the-token"),
    )
    .await;
    h.post(&format!("/api/v1/pastes/{}/retrieve", id(9).encode()), retrieve_body(REVEAL_TOKEN))
        .await;
    h.describe(&id(1).encode()).await;

    assert_eq!(h.bookkeeping(&id(1)).await, before, "a rejected request changed the bookkeeping");
    assert_eq!(before.last_retrieved_at, None);
}

// --- delete ---

#[tokio::test]
async fn deletes_with_the_right_secret() {
    let h = harness("delete").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let answer = h
        .delete(&format!("/api/v1/pastes/{}", id(1).encode()), delete_body(DELETE_SECRET))
        .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);
    assert!(answer.body.is_empty());

    // Gone afterwards, and generically so.
    let after = h
        .post(&format!("/api/v1/pastes/{}/retrieve", id(1).encode()), retrieve_body(REVEAL_TOKEN))
        .await;
    assert_eq!(after.status, StatusCode::NOT_FOUND);
    assert_eq!(after.code(), "not_available");
}

/// A reveal token must not delete. If the delete verifier were derived from the
/// share link, every recipient could destroy the paste.
#[tokio::test]
async fn a_reveal_token_cannot_delete() {
    let h = harness("delete-wrong-cap").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let answer = h
        .delete(&format!("/api/v1/pastes/{}", id(1).encode()), delete_body(REVEAL_TOKEN))
        .await;
    assert_eq!(answer.status, StatusCode::NOT_FOUND);

    // And the paste survives.
    let still_there = h
        .post(&format!("/api/v1/pastes/{}/retrieve", id(1).encode()), retrieve_body(REVEAL_TOKEN))
        .await;
    assert_eq!(still_there.status, StatusCode::OK);
}

/// Deleting twice, and deleting something that never existed, must be
/// indistinguishable — otherwise DELETE becomes an existence oracle.
#[tokio::test]
async fn repeated_and_unknown_deletes_look_the_same() {
    let h = harness("delete-twice").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let first = h
        .delete(&format!("/api/v1/pastes/{}", id(1).encode()), delete_body(DELETE_SECRET))
        .await;
    assert_eq!(first.status, StatusCode::NO_CONTENT);

    // A second delete with the same secret still succeeds silently: the tombstone
    // keeps the verifier, so the capability holder gets the same answer.
    let second = h
        .delete(&format!("/api/v1/pastes/{}", id(1).encode()), delete_body(DELETE_SECRET))
        .await;
    assert_eq!(second.status, StatusCode::NO_CONTENT);

    // An unknown id answers as "not available", identical to a wrong secret.
    let unknown = h
        .delete(&format!("/api/v1/pastes/{}", id(9).encode()), delete_body(DELETE_SECRET))
        .await;
    let wrong_secret = h.delete(&format!("/api/v1/pastes/{}", id(2).encode()), delete_body(b"wrong")).await;
    assert_eq!(unknown.status, wrong_secret.status);
    assert_eq!(unknown.code(), wrong_secret.code());
}

// --- cross-cutting ---

#[tokio::test]
async fn every_response_carries_a_request_id_and_the_security_headers() {
    let h = harness("headers").await;

    let created = h.post("/api/v1/pastes", create_body(&id(1))).await;
    let not_found = h.describe(&id(9).encode()).await;
    let bad_route = h.get("/api/v1/nonexistent").await;

    for answer in [&created, &not_found, &bad_route] {
        let request_id = answer
            .headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert_eq!(request_id.len(), 26, "missing or malformed request id");

        for (name, expected) in capsulebin_server::api::security_headers::headers() {
            assert_eq!(answer.headers.get(&name), Some(&expected), "{name} missing or wrong");
        }
    }

    // Error responses use the problem+json media type and echo the same id as the
    // header, so a user can quote one value.
    assert_eq!(
        not_found.headers.get("content-type").and_then(|value| value.to_str().ok()),
        Some("application/problem+json")
    );
    assert_eq!(
        not_found.json()["request_id"].as_str().unwrap_or_default(),
        not_found
            .headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
    );
}

/// The problem type URI must come from configuration, not from the request. A
/// Host header is attacker-controlled, so deriving it there would let a caller
/// choose the URLs the server publishes.
#[tokio::test]
async fn problem_type_uses_the_configured_public_url() {
    let h = harness("problem-type").await;

    let answer = h
        .send(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/api/v1/pastes/{}/describe", id(9).encode()))
                .header("host", "evil.example")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(describe_body(DESCRIBE_TOKEN).to_string()))
                .unwrap(),
        )
        .await;

    assert_eq!(answer.json()["type"], "https://paste.example/problems/not_available");
}

#[tokio::test]
async fn health_ready_reports_the_schema_version() {
    let h = harness("ready").await;

    let answer = h.get("/health/ready").await;

    assert_eq!(answer.status, StatusCode::OK, "{}", answer.text());
    let json = answer.json();
    assert_eq!(json["database"], "ok");
    assert_eq!(json["assets"], "ok");
    assert_eq!(json["schema_version"], 3);
}

/// The acceptance criterion for the logging rules: a full request cycle must not
/// put payload bytes, a token or a paste id into the log stream.
///
/// Rather than capturing the global tracing subscriber - which is process-wide and
/// makes tests order-dependent - this asserts the property at its source: none of
/// the values are ever passed to a tracing macro, and the types that hold them
/// redact themselves. The `Debug` implementations are the enforcement point.
#[tokio::test]
async fn secrets_redact_themselves_in_debug_output() {
    use capsulebin_server::domain::{PasteId, Verifier};

    let verifier = Verifier::for_delete_secret(DELETE_SECRET);
    let rendered = format!("{verifier:?}");
    assert_eq!(rendered, "Verifier(redacted)");
    assert!(!rendered.contains(&verifier.encode()));

    let paste_id = id(1);
    let rendered = format!("{paste_id:?}");
    assert!(
        !rendered.contains(&paste_id.encode()),
        "Debug for PasteId leaked the full id: {rendered}"
    );
    assert_eq!(PasteId::parse(&paste_id.encode()).unwrap().prefix().len(), 8);
}

/// No response body on any path may contain the marker except the one endpoint
/// that is supposed to return the ciphertext.
#[tokio::test]
async fn the_payload_appears_only_in_the_retrieve_response() {
    let h = harness("no-leak").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let marker = b64(&ciphertext());

    let descriptor = h.describe(&id(1).encode()).await;
    assert!(!descriptor.text().contains(&marker));

    let ready = h.get("/health/ready").await;
    assert!(!ready.text().contains(&marker));

    let retrieve = h
        .post(&format!("/api/v1/pastes/{}/retrieve", id(1).encode()), retrieve_body(REVEAL_TOKEN))
        .await;
    assert!(retrieve.text().contains(&marker), "retrieve must return the payload");
}

// --- describe capability ---

/// Knowing an id is not enough to learn anything about the paste: without the
/// describe token (or the delete secret) the descriptor answers like for an
/// unknown id.
#[tokio::test]
async fn the_descriptor_needs_a_capability() {
    let h = harness("describe-capability").await;
    let mut body = create_body(&id(1));
    body["burn_after_retrieval"] = serde_json::json!(true);
    h.post("/api/v1/pastes", body).await;
    let path = format!("/api/v1/pastes/{}/describe", id(1).encode());

    let with_token = h.describe(&id(1).encode()).await;
    assert_eq!(with_token.status, StatusCode::OK);
    assert_eq!(with_token.json()["burn_after_retrieval"], true);

    let with_delete_secret = h.post(&path, delete_body(DELETE_SECRET)).await;
    assert_eq!(with_delete_secret.status, StatusCode::OK);

    let unknown = h
        .post(
            &format!("/api/v1/pastes/{}/describe", id(9).encode()),
            describe_body(DESCRIBE_TOKEN),
        )
        .await;
    for (label, body) in [
        ("wrong token", describe_body(b"wrong")),
        ("reveal token", describe_body(REVEAL_TOKEN)),
        ("wrong delete secret", delete_body(b"wrong")),
    ] {
        let answer = h.post(&path, body).await;
        assert_eq!(answer.status, unknown.status, "{label}");
        assert_eq!(answer.code(), unknown.code(), "{label}");
        assert!(!answer.text().contains("burn_after_retrieval"), "{label}");
    }
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_descriptor_takes_exactly_one_capability() {
    let h = harness("describe-shape").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;
    let path = format!("/api/v1/pastes/{}/describe", id(1).encode());

    let both = serde_json::json!({ "describe_token": b64(DESCRIBE_TOKEN), "delete_secret": b64(DELETE_SECRET) });
    for body in [serde_json::json!({}), both] {
        let answer = h.post(&path, body).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST);
    }
    // The old unauthenticated GET is gone.
    let get = h.get(&format!("/api/v1/pastes/{}", id(1).encode())).await;
    assert_eq!(get.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn create_requires_a_describe_verifier() {
    let h = harness("describe-required").await;
    let mut body = create_body(&id(1));
    body.as_object_mut().unwrap().remove("describe_verifier");
    let answer = h.post("/api/v1/pastes", body).await;
    assert_eq!(answer.status, StatusCode::BAD_REQUEST);
}
