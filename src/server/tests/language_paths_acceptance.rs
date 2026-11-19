#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Acceptance tests for the language paths (package 20261119-language-paths),
//! derived from the criteria rather than the implementation: the edges the other
//! HTTP tests leave out.

mod support;

use axum::http::StatusCode;
use support::*;

const LANGUAGES: [&str; 2] = ["en", "de"];

fn text_section(markdown: &str) -> Option<capsulebin_server::config::LegalSection> {
    Some(capsulebin_server::config::LegalSection {
        text: Some(markdown.to_owned()),
        ..Default::default()
    })
}

/// Every legal page configured as text, so all three are served.
fn all_legal_pages(config: &mut capsulebin_server::config::Config) {
    config.legal.operator = text_section("Example GmbH");
    config.legal.terms = text_section("Own terms of use.");
}

fn stable(answer: &Answer) -> axum::http::HeaderMap {
    let mut headers = answer.headers.clone();
    headers.remove("x-request-id");
    headers
}

/// AC2 + AC17: `/L/imprint`, `/L/privacy`, `/L/terms` answer `200`, HTML, `no-store`,
/// with the security headers and `<html lang="L">`; under `/de/` the English text
/// is marked `lang="en"` - for the terms as well.
#[tokio::test]
async fn ac2_ac17_every_legal_page_under_every_prefix() {
    let h = harness_with("acc-legal-all", all_legal_pages).await;

    for language in LANGUAGES {
        for page in ["imprint", "privacy", "terms"] {
            let path = format!("/{language}/{page}");
            let answer = h.get(&path).await;

            assert_eq!(answer.status, StatusCode::OK, "{path}");
            assert_eq!(answer.header("content-type"), Some("text/html; charset=utf-8"), "{path}");
            assert_eq!(answer.header("cache-control"), Some("no-store"), "{path}");
            for (name, expected) in capsulebin_server::api::security_headers::headers() {
                assert_eq!(answer.headers.get(&name), Some(&expected), "{name} on {path}");
            }
            let text = answer.text();
            assert!(text.contains(&format!(r#"<html lang="{language}">"#)), "{path}: {text}");
            assert!(!text.contains("<!--capsulebin:"), "{path}: marker left");

            let marked = text.contains(r#"<article class="cb-prose cb-legal" lang="en">"#);
            assert_eq!(marked, language != "en", "{path}: article lang: {text}");
        }
        assert!(h.get(&format!("/{language}/terms")).await.text().contains("Own terms of use."));
    }
}

/// AC4: the redirect from `/p/{id}` and `/manage/{id}` - also with a German
/// `Accept-Language` - and the shells under each prefix do not depend on whether
/// the id exists, and none of it burns the capsule.
#[tokio::test]
async fn ac4_redirects_and_shells_do_not_depend_on_the_id() {
    let h = harness("acc-id-independent").await;
    let mut body = create_body(&id(1));
    body["burn_after_retrieval"] = serde_json::json!(true);
    assert_eq!(h.post("/api/v1/pastes", body).await.status, StatusCode::CREATED);
    let existing = id(1).encode();
    let absent = "AAAAAAAAAAAAAAAAAAAAAA";

    for page in ["p", "manage"] {
        for accept in [None, Some("de-DE,de;q=0.9"), Some("en-US")] {
            let headers: Vec<(&str, &str)> = accept.map(|value| ("accept-language", value)).into_iter().collect();
            let a = h.get_with_headers(&format!("/{page}/{existing}"), &headers).await;
            let b = h.get_with_headers(&format!("/{page}/{absent}"), &headers).await;

            assert_eq!(a.status, StatusCode::FOUND);
            assert_eq!(a.status, b.status);
            assert_eq!(a.body, b.body, "/{page} {accept:?}: redirect body differs");
            let language = if accept.is_some_and(|v| v.starts_with("de")) { "de" } else { "en" };
            assert_eq!(a.header("location"), Some(format!("/{language}/{page}/{existing}")).as_deref());
            assert_eq!(b.header("location"), Some(format!("/{language}/{page}/{absent}")).as_deref());
            let (mut ha, mut hb) = (stable(&a), stable(&b));
            ha.remove("location");
            hb.remove("location");
            assert_eq!(ha, hb, "/{page} {accept:?}: redirect headers differ");
        }

        for language in LANGUAGES {
            let a = h.get(&format!("/{language}/{page}/{existing}")).await;
            let b = h.get(&format!("/{language}/{page}/{absent}")).await;
            assert_eq!(a.status, StatusCode::OK);
            assert_eq!(a.status, b.status);
            assert_eq!(a.body, b.body, "/{language}/{page}: shell differs");
            assert_eq!(stable(&a), stable(&b), "/{language}/{page}: headers differ");
        }
    }

    let answer = h
        .post(&format!("/api/v1/pastes/{existing}/retrieve"), retrieve_body(REVEAL_TOKEN))
        .await;
    assert_eq!(answer.status, StatusCode::OK, "the burn-after-read capsule was consumed");
}

/// AC3: the redirect carries the security headers as well, and a query survives
/// for every page path, not just the report page.
#[tokio::test]
async fn ac3_redirects_keep_the_query_and_carry_the_security_headers() {
    let h = harness("acc-redirect-query").await;

    for (path, location) in [
        ("/?a=1&b=2", "/de/?a=1&b=2"),
        ("/p/2pS3xQ7Rk9mNpLz4AbCdEA?x=y", "/de/p/2pS3xQ7Rk9mNpLz4AbCdEA?x=y"),
        ("/report?id=2pS3xQ7Rk9mNpLz4AbCdEA", "/de/report?id=2pS3xQ7Rk9mNpLz4AbCdEA"),
        ("/en", "/en/"),
    ] {
        let answer = h.get_with_headers(path, &[("accept-language", "de-CH, en;q=0.5")]).await;
        assert_eq!(answer.status, StatusCode::FOUND, "{path}");
        assert_eq!(answer.header("location"), Some(location), "{path}");
        assert_eq!(answer.header("vary"), Some("Accept-Language"), "{path}");
        assert_eq!(answer.header("cache-control"), Some("no-store"), "{path}");
        for (name, expected) in capsulebin_server::api::security_headers::headers() {
            assert_eq!(answer.headers.get(&name), Some(&expected), "{name} on {path}");
        }
    }

    // The highest q among the offered languages wins regardless of order.
    for (accept, location) in [
        ("en;q=0.3, de;q=0.8", "/de/"),
        ("de;q=0.3, en;q=0.8", "/en/"),
        ("it, de-AT;q=0.2", "/de/"),
    ] {
        let answer = h.get_with_headers("/", &[("accept-language", accept)]).await;
        assert_eq!(answer.header("location"), Some(location), "{accept}");
    }
}
