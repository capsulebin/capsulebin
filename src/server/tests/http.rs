#![allow(clippy::unwrap_used, clippy::expect_used)]

//! HTTP integration tests for asset delivery and the security boundary.
//!
//! The focus is not feature coverage: the headers must be present on every
//! response, and `GET /p/{id}` must answer byte-identically for every id.

mod support;

use axum::http::StatusCode;
use support::*;

#[tokio::test]
async fn health_live_returns_ok() {
    let h = harness("shell-live").await;
    let answer = h.get("/health/live").await;

    assert_eq!(answer.status, StatusCode::OK);
    assert_eq!(answer.text(), r#"{"status":"ok"}"#);
}

#[tokio::test]
async fn health_ready_is_degraded_without_assets() {
    let h = harness_without_assets("shell-ready-empty").await;
    let answer = h.get("/health/ready").await;

    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    let json = answer.json();
    assert_eq!(json["assets"], "missing");
    // The database is fine, so readiness must fail on the assets alone. Reporting
    // both as broken would misdirect whoever is debugging it.
    assert_eq!(json["database"], "ok");
}

#[tokio::test]
async fn health_ready_is_ok_with_assets() {
    let h = harness("shell-ready-full").await;
    assert_eq!(h.get("/health/ready").await.status, StatusCode::OK);
}

/// The security headers are part of the boundary and must appear on **every**
/// response - including the 404, where a naively placed layer loses them.
#[tokio::test]
async fn security_headers_are_on_every_response() {
    let h = harness("shell-headers").await;

    for path in [
        "/",
        "/p/2pS3xQ7Rk9mNpLz4AbCdEA",
        "/manage/2pS3xQ7Rk9mNpLz4AbCdEA",
        "/report",
        "/health/live",
        "/does-not-exist",
    ] {
        let answer = h.get(path).await;

        for (name, expected) in capsulebin_server::api::security_headers::headers() {
            assert_eq!(answer.headers.get(&name), Some(&expected), "{name} missing or wrong on {path}");
        }
    }
}

/// The heart of scanner resistance: fetching the shell has no side effects and the
/// response does not depend on the id. If it differed, an observer could infer
/// existence from the length or the content.
#[tokio::test]
async fn view_shell_is_identical_for_every_id() {
    let h = harness("shell-identical").await;

    // One of these ids exists, the other does not. The shell must not care.
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let existing = h.get(&format!("/p/{}", id(1).encode())).await;
    let absent = h.get("/p/AAAAAAAAAAAAAAAAAAAAAA").await;

    assert_eq!(existing.status, StatusCode::OK);
    assert_eq!(absent.status, StatusCode::OK);
    assert_eq!(existing.body, absent.body, "shell differs by paste id");
}

/// Fetching the shell must not consume a retrieval. A link scanner following the
/// URL would otherwise burn a one-time paste - and it must not move
/// `last_retrieved_at` either, because that timestamp drives retention: a crawler
/// would silently keep abandoned capsules alive.
#[tokio::test]
async fn fetching_the_shell_does_not_touch_the_paste() {
    let h = harness("shell-no-side-effect").await;
    h.post("/api/v1/pastes", create_body(&id(1))).await;

    let before = h.bookkeeping(&id(1)).await;

    for _ in 0..5 {
        assert_eq!(h.get(&format!("/p/{}", id(1).encode())).await.status, StatusCode::OK);
    }

    assert_eq!(h.bookkeeping(&id(1)).await, before, "fetching the shell changed the stored row");
    assert_eq!(before.last_retrieved_at, None);
    assert_eq!(before.consumed_at, None);

    // Still retrievable, so nothing was consumed.
    let answer = h
        .post(&format!("/api/v1/pastes/{}/retrieve", id(1).encode()), retrieve_body(REVEAL_TOKEN))
        .await;
    assert_eq!(answer.status, StatusCode::OK);
}

fn legal_text(markdown: &str) -> Option<capsulebin_server::config::LegalSection> {
    Some(capsulebin_server::config::LegalSection {
        text: Some(markdown.to_owned()),
        ..Default::default()
    })
}

#[tokio::test]
async fn every_shell_carries_the_configured_legal_links() {
    let h = harness_with("shell-legal", |config| {
        config.legal.operator = legal_text("Example GmbH");
        config.legal.terms = Some(capsulebin_server::config::LegalSection {
            url: Some("https://operator.example/terms".to_owned()),
            ..Default::default()
        });
    })
    .await;

    for path in [
        "/",
        "/p/2pS3xQ7Rk9mNpLz4AbCdEA",
        "/manage/2pS3xQ7Rk9mNpLz4AbCdEA",
        "/report",
        "/imprint",
    ] {
        let text = h.get(path).await.text();
        assert!(
            text.contains(
                r#"<footer><a href="/imprint">Imprint</a><a href="/privacy">Privacy</a><a href="https://operator.example/terms">Terms</a></footer>"#
            ),
            "{path}: {text}"
        );
    }
}

#[tokio::test]
async fn legal_pages_are_served_with_their_title_and_text() {
    let h = harness_with("legal-pages", |config| {
        config.legal.operator = legal_text("Example **GmbH**");
        config.legal.hosting = legal_text("Hoster AG");
    })
    .await;

    let imprint = h.get("/imprint").await;
    assert_eq!(imprint.status, StatusCode::OK);
    let text = imprint.text();
    assert!(text.contains("<title>Imprint</title><header>Imprint</header>"), "{text}");
    assert!(text.contains("Example <strong>GmbH</strong>"), "{text}");
    assert!(text.contains("<h2>Hosting</h2>"), "{text}");
    assert_eq!(
        imprint
            .headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    assert!(imprint.headers.contains_key("content-security-policy"));

    // The built-in texts.
    assert!(h.get("/privacy").await.text().contains("<h2>Controller</h2>"));
    assert!(h.get("/terms").await.text().contains("<h2>What you may not share</h2>"));
}

#[tokio::test]
async fn without_legal_configuration_there_are_no_pages_and_no_links() {
    let h = harness("shell-no-legal").await;

    assert!(h.get("/").await.text().contains("<footer></footer>"));
    for path in ["/imprint", "/privacy", "/terms"] {
        let answer = h.get(path).await;
        assert_eq!(answer.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(answer.text(), "Not Found", "{path}");
    }
}

#[tokio::test]
async fn every_route_serves_its_own_shell() {
    let h = harness("shell-routes").await;

    for (path, marker) in [
        ("/", "create"),
        ("/p/2pS3xQ7Rk9mNpLz4AbCdEA", "view"),
        ("/manage/2pS3xQ7Rk9mNpLz4AbCdEA", "manage"),
        ("/report", "report"),
    ] {
        let answer = h.get(path).await;

        assert_eq!(answer.status, StatusCode::OK, "{path}");
        assert!(answer.text().contains(marker), "{path} serves the wrong shell");
        assert_eq!(
            answer
                .headers
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/html; charset=utf-8")
        );
        // The shell must not be cached.
        assert_eq!(
            answer
                .headers
                .get(axum::http::header::CACHE_CONTROL)
                .and_then(|value| value.to_str().ok()),
            Some("no-store")
        );
    }
}

#[tokio::test]
async fn unknown_path_is_generic() {
    let h = harness("shell-notfound").await;
    let answer = h.get("/etc/passwd").await;

    assert_eq!(answer.status, StatusCode::NOT_FOUND);
    // No hint about existing files or the directory structure.
    assert_eq!(answer.text(), "Not Found");
}
