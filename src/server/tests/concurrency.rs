#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Races through the whole HTTP stack against a real SQLite file.
//!
//! The repository has its own burn-race test; these go through the handlers,
//! where the capability check, the lifecycle check and the burn transaction are
//! separate steps that concurrent requests interleave.

mod support;

use axum::http::StatusCode;
use support::*;

const CALLERS: usize = 32;

async fn race(h: &Harness, path: String, body: serde_json::Value) -> Vec<StatusCode> {
    let mut handles = Vec::with_capacity(CALLERS);
    for _ in 0..CALLERS {
        let app = h.app.clone();
        let path = path.clone();
        let body = body.clone();
        handles.push(tokio::spawn(async move {
            use tower::ServiceExt as _;
            let request = axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap();
            app.oneshot(request).await.unwrap().status()
        }));
    }
    let mut statuses = Vec::with_capacity(CALLERS);
    for handle in handles {
        statuses.push(handle.await.unwrap());
    }
    statuses
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_retrievals_of_a_burn_paste_hand_it_out_exactly_once() {
    let h = harness("race-burn").await;
    let mut body = create_body(&id(1));
    body["burn_after_retrieval"] = serde_json::json!(true);
    assert_eq!(h.post("/api/v1/pastes", body).await.status, StatusCode::CREATED);

    let path = format!("/api/v1/pastes/{}/retrieve", id(1).encode());
    let statuses = race(&h, path, retrieve_body(REVEAL_TOKEN)).await;

    let winners = statuses.iter().filter(|s| **s == StatusCode::OK).count();
    assert_eq!(winners, 1, "{statuses:?}");
    // Everyone else gets "not available" or, if they lost inside the burn
    // transaction, "already retrieved" - never a server error.
    assert!(
        statuses
            .iter()
            .all(|s| matches!(*s, StatusCode::OK | StatusCode::NOT_FOUND | StatusCode::GONE)),
        "{statuses:?}"
    );

    let bookkeeping = h.bookkeeping(&id(1)).await;
    assert!(bookkeeping.consumed_at.is_some());
    assert_eq!(bookkeeping.last_retrieved_at, bookkeeping.consumed_at);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_retrievals_of_a_normal_paste_all_succeed_and_are_recorded() {
    let h = harness("race-normal").await;
    assert_eq!(h.post("/api/v1/pastes", create_body(&id(1))).await.status, StatusCode::CREATED);

    let path = format!("/api/v1/pastes/{}/retrieve", id(1).encode());
    let statuses = race(&h, path, retrieve_body(REVEAL_TOKEN)).await;

    assert!(statuses.iter().all(|s| *s == StatusCode::OK), "{statuses:?}");
    assert_eq!(h.bookkeeping(&id(1)).await.last_retrieved_at, Some(START));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_delete_racing_retrievals_leaves_a_consistent_tombstone() {
    // Up to 32 failed retrievals of one id: about the race, not the per-paste
    // failure limit (tests/rate_limits.rs), so that limit is off here.
    let h = harness_with("race-delete", |config| {
        config.rate_limits.invalid_capability_per_paste = Default::default();
    })
    .await;
    assert_eq!(h.post("/api/v1/pastes", create_body(&id(1))).await.status, StatusCode::CREATED);

    let retrieve = {
        let path = format!("/api/v1/pastes/{}/retrieve", id(1).encode());
        let h_app = h.app.clone();
        tokio::spawn(async move {
            let mut statuses = Vec::new();
            for _ in 0..CALLERS {
                use tower::ServiceExt as _;
                let request = axum::http::Request::builder()
                    .method("POST")
                    .uri(path.clone())
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(retrieve_body(REVEAL_TOKEN).to_string()))
                    .unwrap();
                statuses.push(h_app.clone().oneshot(request).await.unwrap().status());
            }
            statuses
        })
    };
    let deleted = h
        .delete(&format!("/api/v1/pastes/{}", id(1).encode()), delete_body(DELETE_SECRET))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let statuses = retrieve.await.unwrap();

    // Before the delete: 200; after: 404. Never anything else, and once a 404
    // was seen, no later request may succeed again.
    assert!(
        statuses.iter().all(|s| matches!(*s, StatusCode::OK | StatusCode::NOT_FOUND)),
        "{statuses:?}"
    );
    if let Some(first_gone) = statuses.iter().position(|s| *s == StatusCode::NOT_FOUND) {
        assert!(statuses[first_gone..].iter().all(|s| *s == StatusCode::NOT_FOUND), "{statuses:?}");
    }

    let (status, ciphertext): (String, Option<Vec<u8>>) =
        sqlx::query_as("SELECT availability_status, ciphertext FROM pastes WHERE id = ?1")
            .bind(id(1).encode())
            .fetch_one(&h.pool)
            .await
            .unwrap();
    assert_eq!(status, "deleted");
    assert!(ciphertext.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_creates_with_distinct_ids_all_land() {
    let h = harness("race-create").await;
    let mut handles = Vec::new();
    for byte in 0..CALLERS as u8 {
        let app = h.app.clone();
        handles.push(tokio::spawn(async move {
            use tower::ServiceExt as _;
            let body = create_body(&id(100 + byte)).to_string();
            let request = axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/pastes")
                .header("content-type", "application/json")
                .header("content-length", body.len())
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(axum::body::Body::from(body))
                .unwrap();
            app.oneshot(request).await.unwrap().status()
        }));
    }
    for handle in handles {
        assert_eq!(handle.await.unwrap(), StatusCode::CREATED);
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pastes").fetch_one(&h.pool).await.unwrap();
    assert_eq!(count, CALLERS as i64);
}

/// Wrong passwords fired in parallel - reveal tokens precomputed offline from a
/// password list - must not slip past the per-paste limit between its check and
/// its charge. With the default burst of 10, exactly ten get through.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn parallel_password_guesses_cannot_exceed_the_per_paste_burst() {
    const GUESSES: usize = 50;
    let h = harness("race-password-attempts").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;
    let path = format!("/api/v1/pastes/{}/retrieve", id(1).encode());

    let start = std::sync::Arc::new(tokio::sync::Barrier::new(GUESSES));
    let mut handles = Vec::with_capacity(GUESSES);
    for n in 0..GUESSES {
        let app = h.app.clone();
        let path = path.clone();
        let start = start.clone();
        let body = retrieve_body(format!("guess-{n}").as_bytes());
        handles.push(tokio::spawn(async move {
            use tower::ServiceExt as _;
            let request = axum::http::Request::builder()
                .method("POST")
                .uri(path)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .unwrap();
            start.wait().await;
            app.oneshot(request).await.unwrap().status()
        }));
    }
    let mut statuses = Vec::with_capacity(GUESSES);
    for handle in handles {
        statuses.push(handle.await.unwrap());
    }

    let not_found = statuses.iter().filter(|s| **s == StatusCode::NOT_FOUND).count();
    let limited = statuses.iter().filter(|s| **s == StatusCode::TOO_MANY_REQUESTS).count();
    assert_eq!((not_found, limited), (10, GUESSES - 10), "{statuses:?}");
}
