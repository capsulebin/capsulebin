#![allow(clippy::unwrap_used, clippy::expect_used)]

//! HTTP integration tests for asset delivery and the security boundary.
//!
//! The focus is not feature coverage: the headers must be present on every
//! response, and `GET /p/{id}` and `GET /<language>/p/{id}` must answer
//! identically for every id.

mod support;

use axum::http::StatusCode;
use capsulebin_server::config::KNOWN_LANGUAGES;
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

/// Every shell of every known language is needed; one missing is a page that
/// would answer `500`.
#[tokio::test]
async fn health_ready_needs_every_shell_of_every_language() {
    let h = harness("shell-ready-partial").await;
    std::fs::remove_file(h.dir.join("de/legal.html")).unwrap();

    let answer = h.get("/health/ready").await;
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(answer.json()["assets"], "missing");
}

const SOME_ID: &str = "2pS3xQ7Rk9mNpLz4AbCdEA";

/// The page paths under `language`, each with the shell it must be served with.
fn pages(language: &str) -> [(String, &'static str); 4] {
    [
        (format!("/{language}/"), "create"),
        (format!("/{language}/p/{SOME_ID}"), "view"),
        (format!("/{language}/manage/{SOME_ID}"), "manage"),
        (format!("/{language}/report"), "report"),
    ]
}

/// The security headers are part of the boundary and must appear on **every**
/// response - including the 404 and the redirect, where a naively placed layer
/// loses them.
#[tokio::test]
async fn security_headers_are_on_every_response() {
    let h = harness("shell-headers").await;

    let mut paths: Vec<String> = KNOWN_LANGUAGES
        .iter()
        .flat_map(|language| pages(language).map(|(path, _)| path))
        .collect();
    paths.extend(
        [
            "/",
            "/p/2pS3xQ7Rk9mNpLz4AbCdEA",
            "/manage/2pS3xQ7Rk9mNpLz4AbCdEA",
            "/report",
            "/imprint",
            "/de",
            "/fr/",
            "/en/index.html",
            "/health/live",
            "/does-not-exist",
        ]
        .map(str::to_owned),
    );
    for path in paths {
        let answer = h.get(&path).await;

        for (name, expected) in capsulebin_server::api::security_headers::headers() {
            assert_eq!(answer.headers.get(&name), Some(&expected), "{name} missing or wrong on {path}");
        }
    }
}

/// The heart of scanner resistance: fetching the shell - or being redirected to
/// it - has no side effects, and the response does not depend on the id. If it
/// differed, an observer could infer existence from the length, the content or
/// a header.
#[tokio::test]
async fn view_shell_is_identical_for_every_id() {
    let h = harness("shell-identical").await;

    // One of these ids exists, the other does not. The shell must not care.
    h.post("/api/v1/pastes", create_body(&id(1))).await;
    let absent = "AAAAAAAAAAAAAAAAAAAAAA";

    for prefix in ["", "/en", "/de"] {
        let existing = h.get(&format!("{prefix}/p/{}", id(1).encode())).await;
        let unknown = h.get(&format!("{prefix}/p/{absent}")).await;

        let expected = if prefix.is_empty() { StatusCode::FOUND } else { StatusCode::OK };
        assert_eq!(existing.status, expected, "{prefix}");
        assert_eq!(unknown.status, expected, "{prefix}");
        assert_eq!(existing.body, unknown.body, "{prefix}: shell differs by paste id");
        if prefix.is_empty() {
            // The id is copied into the location, so compare everything else.
            assert_eq!(existing.header("location"), Some(format!("/en/p/{}", id(1).encode())).as_deref());
            assert_eq!(unknown.header("location"), Some(format!("/en/p/{absent}")).as_deref());
            let mut existing = stable_headers(&existing);
            let mut unknown = stable_headers(&unknown);
            existing.remove("location");
            unknown.remove("location");
            assert_eq!(existing, unknown, "redirect headers differ by paste id");
        } else {
            assert_eq!(
                stable_headers(&existing),
                stable_headers(&unknown),
                "{prefix}: headers differ by paste id"
            );
        }
    }
}

/// Fetching the shell must not consume a retrieval. A link scanner following the
/// URL would otherwise burn a one-time paste - and it must not move
/// `last_retrieved_at` either, because that timestamp drives retention: a crawler
/// would silently keep abandoned capsules alive.
#[tokio::test]
async fn fetching_the_shell_does_not_touch_the_paste() {
    let h = harness("shell-no-side-effect").await;
    let mut body = create_body(&id(1));
    body["burn_after_retrieval"] = serde_json::json!(true);
    assert_eq!(h.post("/api/v1/pastes", body).await.status, StatusCode::CREATED);

    let before = h.bookkeeping(&id(1)).await;

    for _ in 0..5 {
        for path in ["/p", "/manage"] {
            assert_eq!(h.get(&format!("{path}/{}", id(1).encode())).await.status, StatusCode::FOUND);
            for &language in KNOWN_LANGUAGES {
                assert_eq!(h.get(&format!("/{language}{path}/{}", id(1).encode())).await.status, StatusCode::OK);
            }
        }
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

/// A page path without a prefix - a share link above all - goes to the same
/// path under the language the browser prefers.
#[tokio::test]
async fn unprefixed_pages_are_redirected_by_accept_language() {
    let h = harness("shell-redirect").await;

    let overlong = format!("{}de", "x;q=0.1,".repeat(200));
    for (path, accept_language, location) in [
        ("/", None, "/en/"),
        ("/", Some("de-AT,de;q=0.9,en;q=0.8"), "/de/"),
        ("/p/2pS3xQ7Rk9mNpLz4AbCdEA", Some("de"), "/de/p/2pS3xQ7Rk9mNpLz4AbCdEA"),
        ("/p/2pS3xQ7Rk9mNpLz4AbCdEA", Some("fr, en;q=0.5"), "/en/p/2pS3xQ7Rk9mNpLz4AbCdEA"),
        ("/manage/2pS3xQ7Rk9mNpLz4AbCdEA", Some("de"), "/de/manage/2pS3xQ7Rk9mNpLz4AbCdEA"),
        (
            "/report?id=2pS3xQ7Rk9mNpLz4AbCdEA",
            Some("de"),
            "/de/report?id=2pS3xQ7Rk9mNpLz4AbCdEA",
        ),
        ("/imprint", Some("de"), "/de/imprint"),
        ("/privacy", Some("en"), "/en/privacy"),
        ("/terms", None, "/en/terms"),
        ("/", Some("de;q=0, en;q=0.1"), "/en/"),
        ("/", Some("de;q=0"), "/en/"),
        ("/", Some("fr"), "/en/"),
        ("/", Some(";;; q=x, ,"), "/en/"),
        ("/", Some(overlong.as_str()), "/en/"),
        // Not a language range, and a weight with a second parameter.
        ("/", Some("de-"), "/en/"),
        ("/", Some("de;q=0; q=1"), "/en/"),
        // `/<language>` without the slash.
        ("/de", Some("en"), "/de/"),
        ("/en?x=1", None, "/en/?x=1"),
    ] {
        let headers: Vec<(&str, &str)> = accept_language.map(|value| ("accept-language", value)).into_iter().collect();
        let answer = h.get_with_headers(path, &headers).await;

        assert_eq!(answer.status, StatusCode::FOUND, "{path} {accept_language:?}");
        assert_eq!(answer.header("location"), Some(location), "{path} {accept_language:?}");
        assert_eq!(answer.header("vary"), Some("Accept-Language"), "{path}");
        assert_eq!(answer.header("cache-control"), Some("no-store"), "{path}");
    }

    // A header that is not text is no preference either.
    let request = axum::http::Request::builder()
        .uri("/")
        .header("accept-language", axum::http::HeaderValue::from_bytes(b"de\xff").unwrap())
        .body(axum::body::Body::empty())
        .unwrap();
    assert_eq!(h.send(request).await.header("location"), Some("/en/"));

    // Several field lines are one list: the later line may weigh higher.
    let answer = h
        .get_with_headers("/", &[("accept-language", "en;q=0.5"), ("accept-language", "de")])
        .await;
    assert_eq!(answer.header("location"), Some("/de/"));
}

/// A prefix that is not a known language is an unknown path: same status,
/// same body. So is a shell file requested directly: it would come without its
/// markers.
#[tokio::test]
async fn unknown_prefixes_and_shell_files_are_unknown_paths() {
    let h = harness("shell-unknown-prefix").await;
    let unknown = h.get("/does-not-exist").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    for path in [
        "/fr/",
        "/fr",
        "/xx/p/2pS3xQ7Rk9mNpLz4AbCdEA",
        "/en/index.html",
        "/de/p.html",
        "/en/legal.html",
        "/en/INDEX.HTML",
    ] {
        let answer = h.get(path).await;
        assert_eq!(answer.status, unknown.status, "{path}");
        assert_eq!(answer.body, unknown.body, "{path}");
    }
}

fn legal_configuration(config: &mut capsulebin_server::config::Config) {
    config.legal.operator = legal_text("Example GmbH");
    config.legal.terms = Some(capsulebin_server::config::LegalSection {
        url: Some("https://operator.example/terms".to_owned()),
        ..Default::default()
    });
}

/// The labels are the shell's (from the catalogue of its language), the
/// targets the server's: under the prefix for pages served here, the
/// configured `url` for pages elsewhere.
#[tokio::test]
async fn every_shell_carries_the_configured_legal_links() {
    let h = harness_with("shell-legal", legal_configuration).await;

    for (language, [imprint, privacy, terms]) in [
        ("en", ["Imprint", "Privacy", "Terms"]),
        ("de", ["Impressum", "Datenschutz", "Nutzungsbedingungen"]),
    ] {
        let mut paths: Vec<String> = pages(language).map(|(path, _)| path).into();
        paths.push(format!("/{language}/imprint"));
        for path in paths {
            let text = h.get(&path).await.text();
            let footer = format!(
                r#"<footer><a href="/{language}/imprint">{imprint}</a><a href="/{language}/privacy">{privacy}</a><a href="https://operator.example/terms">{terms}</a></footer>"#
            );
            assert!(text.contains(&footer), "{path}: {text}");
        }
    }
}

/// A page that is not configured has no link; the markers of the others are
/// still replaced, under every prefix.
#[tokio::test]
async fn links_to_pages_that_are_not_configured_are_omitted() {
    let h = harness_with("shell-legal-omitted", |config| {
        config.legal.privacy = Some(capsulebin_server::config::LegalSection {
            url: Some("https://operator.example/privacy".to_owned()),
            ..Default::default()
        });
    })
    .await;

    for (language, privacy) in [("en", "Privacy"), ("de", "Datenschutz")] {
        let text = h.get(&format!("/{language}/report")).await.text();
        let footer = format!(r#"<footer><a href="https://operator.example/privacy">{privacy}</a></footer>"#);
        assert!(text.contains(&footer), "/{language}/report: {text}");
    }
}

/// Every route serves its shell with the language list the web build wrote
/// into the head, untouched, and no marker left over.
#[tokio::test]
async fn every_shell_carries_the_language_list() {
    let h = harness_with("shell-languages", legal_configuration).await;

    for &language in KNOWN_LANGUAGES {
        let mut paths: Vec<String> = pages(language).map(|(path, _)| path).into();
        paths.push(format!("/{language}/privacy"));
        for path in paths {
            let text = h.get(&path).await.text();
            assert!(text.contains(&format!("<head>{LANGUAGES_META}<title>")), "{path}: {text}");
            assert!(!text.contains("<!--capsulebin:"), "{path}: a marker is left: {text}");
        }
    }
}

#[tokio::test]
async fn legal_pages_are_served_with_their_title_and_text() {
    let h = harness_with("legal-pages", |config| {
        config.legal.operator = legal_text("Example **GmbH**");
        config.legal.hosting = legal_text("Hoster AG");
    })
    .await;

    for &language in KNOWN_LANGUAGES {
        let imprint = h.get(&format!("/{language}/imprint")).await;
        assert_eq!(imprint.status, StatusCode::OK);
        let text = imprint.text();
        assert!(text.contains(&format!(r#"<html lang="{language}">"#)), "{text}");
        assert!(text.contains("<title>Imprint</title></head><header>Imprint</header>"), "{text}");
        assert!(text.contains("Example <strong>GmbH</strong>"), "{text}");
        assert!(text.contains("<h2>Hosting</h2>"), "{text}");
        assert_eq!(imprint.header("content-type"), Some("text/html; charset=utf-8"));
        assert_eq!(imprint.header("cache-control"), Some("no-store"));
        assert!(imprint.headers.contains_key("content-security-policy"));

        // The built-in texts.
        assert!(h.get(&format!("/{language}/privacy")).await.text().contains("<h2>Controller</h2>"));
        assert!(
            h.get(&format!("/{language}/terms"))
                .await
                .text()
                .contains("<h2>What you may not share</h2>")
        );
    }
}

/// The texts are English under every prefix; in a German shell the article says
/// so, for screen readers.
#[tokio::test]
async fn legal_texts_in_another_language_than_the_shell_are_marked() {
    let h = harness_with("legal-lang", legal_configuration).await;

    for page in ["imprint", "privacy"] {
        let german = h.get(&format!("/de/{page}")).await.text();
        assert!(
            german.contains(r#"<main><article class="cb-prose cb-legal" lang="en">"#),
            "{page}: {german}"
        );

        let english = h.get(&format!("/en/{page}")).await.text();
        assert!(
            english.contains(r#"<main><article class="cb-prose cb-legal">"#),
            "{page}: {english}"
        );
    }
}

#[tokio::test]
async fn without_legal_configuration_there_are_no_pages_and_no_links() {
    let h = harness("shell-no-legal").await;

    for &language in KNOWN_LANGUAGES {
        assert!(h.get(&format!("/{language}/")).await.text().contains("<footer></footer>"));
        for page in ["imprint", "privacy", "terms"] {
            let path = format!("/{language}/{page}");
            let answer = h.get(&path).await;
            assert_eq!(answer.status, StatusCode::NOT_FOUND, "{path}");
            assert_eq!(answer.text(), "Not Found", "{path}");

            // Without a prefix it is redirected anyway: the redirect depends on
            // the path alone, the 404 comes under the prefix.
            let unprefixed = h.get_with_headers(&format!("/{page}"), &[("accept-language", language)]).await;
            assert_eq!(unprefixed.status, StatusCode::FOUND, "/{page}");
            assert_eq!(unprefixed.header("location"), Some(path.as_str()));
        }
    }
}

#[tokio::test]
async fn every_route_serves_its_own_shell() {
    let h = harness("shell-routes").await;

    for &language in KNOWN_LANGUAGES {
        for (path, marker) in pages(language) {
            let answer = h.get(&path).await;

            assert_eq!(answer.status, StatusCode::OK, "{path}");
            let text = answer.text();
            assert!(text.contains(&format!("<title>{marker}</title>")), "{path} serves the wrong shell");
            assert!(
                text.contains(&format!(r#"<html lang="{language}">"#)),
                "{path} serves the wrong language"
            );
            assert_eq!(answer.header("content-type"), Some("text/html; charset=utf-8"));
            // The shell must not be cached.
            assert_eq!(answer.header("cache-control"), Some("no-store"));
        }
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
