#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Rate limits through the real middleware stack.
//!
//! What matters beyond "a limit limits":
//!
//! - a `429` happens before any lookup, so it cannot burn a paste or reveal
//!   whether an id exists;
//! - failed capability checks are throttled per client, and only failures count;
//! - wrong passwords are throttled per paste, but only for callers who hold the
//!   link - knowing an id is not enough to block a capsule or to see the limit;
//! - the create byte quota is charged by request size;
//! - a spoofed `X-Forwarded-For` does not buy a fresh bucket unless the peer is a
//!   configured proxy.

mod support;

use axum::http::StatusCode;
use support::*;

fn retrieve_path(byte: u8) -> String {
    format!("/api/v1/pastes/{}/retrieve", id(byte).encode())
}

#[tokio::test]
async fn failed_capability_checks_are_throttled_before_any_lookup() {
    let h = harness_with("invalid-capability", |config| {
        config.rate_limits.invalid_capability.requests_per_minute = 1;
        config.rate_limits.invalid_capability.burst = 3;
    })
    .await;
    let mut burn = create_body(&id(1));
    burn["burn_after_retrieval"] = serde_json::json!(true);
    assert_eq!(h.post("/api/v1/pastes", burn).await.status, StatusCode::CREATED);

    // Successful requests cost nothing.
    for _ in 0..3 {
        assert_eq!(h.describe(&id(1).encode()).await.status, StatusCode::OK);
    }

    // Three failures - wrong token, unknown id, wrong delete secret - use up the bucket.
    assert_eq!(
        h.post(&retrieve_path(1), retrieve_body(b"wrong")).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.post(&retrieve_path(9), retrieve_body(REVEAL_TOKEN)).await.status,
        StatusCode::NOT_FOUND
    );
    let wrong_delete = h.delete(&format!("/api/v1/pastes/{}", id(1).encode()), delete_body(b"wrong")).await;
    assert_eq!(wrong_delete.status, StatusCode::NOT_FOUND);

    // Now even the right token is refused - before the burn transaction.
    let refused = h.post(&retrieve_path(1), retrieve_body(REVEAL_TOKEN)).await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(refused.code(), "rate_limited");
    assert_eq!(refused.headers["retry-after"], "60");
    let untouched = h.bookkeeping(&id(1)).await;
    assert_eq!(untouched.consumed_at, None, "a throttled request burned the paste");
    assert_eq!(untouched.last_retrieved_at, None);

    // An unknown id gets the identical answer.
    let unknown = h.post(&retrieve_path(9), retrieve_body(REVEAL_TOKEN)).await;
    assert_eq!(unknown.status, refused.status);
    assert_eq!(unknown.code(), refused.code());
    assert_eq!(unknown.headers["retry-after"], refused.headers["retry-after"]);
    // Like every other error: problem+json with a request id.
    assert_eq!(refused.headers["content-type"], "application/problem+json");
    assert!(!refused.json()["request_id"].as_str().unwrap_or_default().is_empty());

    // The descriptor checks a capability too, so it is refused as well -
    // otherwise it would be a way around the limit.
    assert_eq!(h.describe(&id(1).encode()).await.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn the_create_byte_quota_is_charged_by_size() {
    let body_len = create_body(&id(1)).to_string().len() as u64;
    let h = harness_with("byte-quota", move |config| {
        // Room for exactly two requests of this size per day.
        config.rate_limits.create.bytes_per_day = capsulebin_server::config::units::ByteSize(body_len * 2);
    })
    .await;

    assert_eq!(h.post("/api/v1/pastes", create_body(&id(1))).await.status, StatusCode::CREATED);
    assert_eq!(h.post("/api/v1/pastes", create_body(&id(2))).await.status, StatusCode::CREATED);
    let third = h.post("/api/v1/pastes", create_body(&id(3))).await;
    assert_eq!(third.status, StatusCode::TOO_MANY_REQUESTS);
    // Refilled over a day: one request's worth takes half a day.
    let retry: u64 = third.headers["retry-after"].to_str().unwrap().parse().unwrap();
    assert!((43_000..=43_300).contains(&retry), "retry-after {retry}");
}

#[tokio::test]
async fn the_byte_quota_needs_a_declared_length() {
    let h = harness_with("byte-quota-length", |config| {
        config.rate_limits.create.bytes_per_day = capsulebin_server::config::units::ByteSize(1024 * 1024);
    })
    .await;

    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/api/v1/pastes")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(axum::body::Body::from(create_body(&id(1)).to_string()))
        .unwrap();
    let answer = h.send(request).await;
    assert_eq!(answer.status, StatusCode::LENGTH_REQUIRED);
    assert_eq!(answer.code(), "length_required");
}

#[tokio::test]
async fn a_spoofed_forwarded_for_does_not_reset_the_limit() {
    let h = harness_with("spoofed-xff", |config| {
        config.rate_limits.descriptor.requests_per_minute = 1;
        config.rate_limits.descriptor.burst = 1;
    })
    .await;
    let path = format!("/api/v1/pastes/{}/describe", id(1).encode());
    let get = |forwarded: &'static str| {
        let request = axum::http::Request::builder()
            .method("POST")
            .uri(path.clone())
            .header("content-type", "application/json")
            .header("x-forwarded-for", forwarded)
            .body(axum::body::Body::from(describe_body(DESCRIBE_TOKEN).to_string()))
            .unwrap();
        h.send(request)
    };

    assert_eq!(get("198.51.100.1").await.status, StatusCode::NOT_FOUND);
    // The peer is not a trusted proxy, so the header is ignored.
    assert_eq!(get("198.51.100.2").await.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn behind_a_trusted_proxy_each_client_has_its_own_bucket() {
    let h = harness_with("trusted-xff", |config| {
        config.server.trusted_proxies = vec!["127.0.0.1/32".to_owned()];
        config.rate_limits.descriptor.requests_per_minute = 1;
        config.rate_limits.descriptor.burst = 1;
    })
    .await;
    let path = format!("/api/v1/pastes/{}/describe", id(1).encode());
    let get = |forwarded: &'static str| {
        let request = axum::http::Request::builder()
            .method("POST")
            .uri(path.clone())
            .header("content-type", "application/json")
            .header("x-forwarded-for", forwarded)
            .body(axum::body::Body::from(describe_body(DESCRIBE_TOKEN).to_string()))
            .unwrap();
        h.send(request)
    };

    assert_eq!(get("198.51.100.1").await.status, StatusCode::NOT_FOUND);
    assert_eq!(get("198.51.100.1").await.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        get("198.51.100.2").await.status,
        StatusCode::NOT_FOUND,
        "a different client shares the bucket"
    );
    // A client cannot escape by prepending its own value: the right-most
    // untrusted address is the one the proxy saw.
    assert_eq!(get("203.0.113.9, 198.51.100.1").await.status, StatusCode::TOO_MANY_REQUESTS);
}

/// On by default and independent of the client address: the server-side
/// protection of a password once the link has leaked.
#[tokio::test]
async fn password_attempts_on_one_paste_are_limited_by_default_from_any_address() {
    let h = harness("per-paste").await;
    let mut body = create_body(&id(1));
    body["burn_after_retrieval"] = serde_json::json!(true);
    assert_eq!(h.post("/api/v1/pastes", body).await.status, StatusCode::CREATED);

    // Ten wrong passwords - the right describe token, a wrong reveal token -
    // each from a different (untrusted) forwarded address.
    for n in 0..10 {
        let forwarded = format!("198.51.100.{n}");
        let answer = h
            .post_with_headers(
                &retrieve_path(1),
                retrieve_body(format!("guess-{n}").as_bytes()),
                None,
                &[("x-forwarded-for", forwarded.as_str())],
            )
            .await;
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "guess {n}");
    }

    // The eleventh attempt - even with the right token - is refused before the
    // burn transaction, so the burn paste survives.
    let refused = h.post(&retrieve_path(1), retrieve_body(REVEAL_TOKEN)).await;
    assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(refused.code(), "rate_limited");
    assert_eq!(refused.headers["retry-after"], "6");
    assert_eq!(h.bookkeeping(&id(1)).await.consumed_at, None);

    // Only link holders see the limit: with just the id, the paste still looks
    // like an unknown one.
    let id_only = h.post(&retrieve_path(1), retrieve_body_with(b"guessed", REVEAL_TOKEN)).await;
    assert_eq!(id_only.status, StatusCode::NOT_FOUND);
    // Describing and deleting do not involve a password and stay available.
    assert_eq!(h.describe(&id(1).encode()).await.status, StatusCode::OK);

    // Other pastes are unaffected.
    assert_eq!(h.post("/api/v1/pastes", create_body(&id(2))).await.status, StatusCode::CREATED);
    assert_eq!(h.post(&retrieve_path(2), retrieve_body(REVEAL_TOKEN)).await.status, StatusCode::OK);
}

/// The scenario from ADR-001: the id leaks (a proxy log, the report
/// form), the link does not. Knowing the id must not be enough to block it.
#[tokio::test]
async fn a_caller_with_only_the_id_cannot_block_a_capsule() {
    let h = harness("per-paste-id-only").await;
    assert_eq!(h.post("/api/v1/pastes", create_body(&id(1))).await.status, StatusCode::CREATED);
    let paste = id(1).encode();

    // Far more failures than the per-paste burst, through every endpoint that
    // takes a capability, from many addresses.
    for n in 0..30 {
        let forwarded = format!("198.51.100.{n}");
        let headers = [("x-forwarded-for", forwarded.as_str())];
        let guess = format!("guess-{n}");
        let retrieve = h
            .post_with_headers(
                &retrieve_path(1),
                retrieve_body_with(guess.as_bytes(), REVEAL_TOKEN),
                None,
                &headers,
            )
            .await;
        assert_eq!(retrieve.status, StatusCode::NOT_FOUND);
        let describe = h
            .post_with_headers(
                &format!("/api/v1/pastes/{paste}/describe"),
                describe_body(guess.as_bytes()),
                None,
                &headers,
            )
            .await;
        assert_eq!(describe.status, StatusCode::NOT_FOUND);
        let delete = h.delete(&format!("/api/v1/pastes/{paste}"), delete_body(guess.as_bytes())).await;
        assert_eq!(delete.status, StatusCode::NOT_FOUND);
    }

    // The recipient is not affected.
    assert_eq!(h.post(&retrieve_path(1), retrieve_body(REVEAL_TOKEN)).await.status, StatusCode::OK);
}

#[tokio::test]
async fn successful_retrievals_do_not_count_against_the_per_paste_limit() {
    let h = harness("per-paste-success").await;
    assert_eq!(h.post("/api/v1/pastes", create_body(&id(1))).await.status, StatusCode::CREATED);
    for _ in 0..15 {
        assert_eq!(h.post(&retrieve_path(1), retrieve_body(REVEAL_TOKEN)).await.status, StatusCode::OK);
    }
}

#[tokio::test]
async fn the_per_paste_limit_can_be_switched_off() {
    let h = harness_with("per-paste-off", |config| {
        config.rate_limits.invalid_capability_per_paste = Default::default();
    })
    .await;
    assert_eq!(h.post("/api/v1/pastes", create_body(&id(1))).await.status, StatusCode::CREATED);
    for _ in 0..15 {
        assert_eq!(h.post(&retrieve_path(1), retrieve_body(b"x")).await.status, StatusCode::NOT_FOUND);
    }
    assert_eq!(h.post(&retrieve_path(1), retrieve_body(REVEAL_TOKEN)).await.status, StatusCode::OK);
}
