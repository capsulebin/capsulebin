#![allow(clippy::unwrap_used, clippy::expect_used)]

//! HTTP integration tests for asset delivery and the security boundary.
//!
//! The focus is not feature coverage: the headers must be present on every
//! response, and `GET /p/{id}` and `GET /<language>/p/{id}` must answer
//! identically for every id, whichever language is the default.

mod support;

use axum::http::StatusCode;
use capsulebin_server::config::CreationMode;
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

const MISSING_FRONTEND: &str = "Frontend assets are missing. In development, run `npm run build` in src/server/web/ first.";

/// The asset directory is for development: a rebuilt shell is served without a
/// restart, and a missing one is reported on the page as on `/health/ready`.
#[tokio::test]
async fn the_asset_directory_is_read_on_every_request() {
    let h = harness("shell-directory-live").await;
    let path = format!("/de/p/{SOME_ID}");
    assert!(h.get(&path).await.text().contains("<title>view</title>"));

    std::fs::write(h.dir.join("de/p.html"), "<title>rebuilt %%capsulebin:prefix%%</title>").unwrap();
    assert_eq!(h.get(&path).await.text(), "<title>rebuilt /de</title>");

    std::fs::remove_file(h.dir.join("de/p.html")).unwrap();
    let answer = h.get(&path).await;
    assert_eq!(answer.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(answer.text(), MISSING_FRONTEND);
}

/// The embedded shells are filled once at startup. With a web build in the
/// binary every page is complete and the same for every id; without one every
/// page is the missing-frontend answer and the server is not ready.
#[tokio::test]
async fn embedded_shells_are_served_filled_or_reported_missing() {
    let h = harness_with_embedded_assets("shell-embedded").await;
    let built = h.get("/health/ready").await.json()["assets"] == "ok";

    for (language, prefix) in [("en", ""), ("de", "/de")] {
        for (path, _) in pages(prefix) {
            let answer = h.get(&path).await;
            if !built {
                assert_eq!(answer.status, StatusCode::INTERNAL_SERVER_ERROR, "{path}");
                assert_eq!(answer.text(), MISSING_FRONTEND, "{path}");
                continue;
            }
            assert_eq!(answer.status, StatusCode::OK, "{path}");
            let text = answer.text();
            assert!(text.contains(&format!(r#"<html lang="{language}""#)), "{path}");
            assert!(text.contains(r#"data-default="en""#), "{path}");
            for left in ["%%capsulebin:", "<!--capsulebin:"] {
                assert!(!text.contains(left), "{path} still contains {left}");
            }
        }
        if built {
            let one = h.get(&format!("{prefix}/p/{SOME_ID}")).await;
            let other = h.get(&format!("{prefix}/p/AAAAAAAAAAAAAAAAAAAAAA")).await;
            assert_eq!(one.body, other.body);
            assert_eq!(stable_headers(&one), stable_headers(&other));
        }
    }
}

/// The page paths under `prefix` (`""` or `/de`), each with the shell it must
/// be served with.
fn pages(prefix: &str) -> [(String, &'static str); 4] {
    [
        (format!("{prefix}/"), "create"),
        (format!("{prefix}/p/{SOME_ID}"), "view"),
        (format!("{prefix}/manage/{SOME_ID}"), "manage"),
        (format!("{prefix}/report"), "report"),
    ]
}

/// The security headers are part of the boundary and must appear on **every**
/// response - including the 404 and the redirect, where a naively placed layer
/// loses them.
#[tokio::test]
async fn security_headers_are_on_every_response() {
    for (default, languages) in LAYOUTS {
        let h = harness_with_default_language(&format!("shell-headers-{default}"), default).await;

        let mut paths: Vec<String> = languages
            .iter()
            .flat_map(|(_, prefix)| pages(prefix).map(|(path, _)| path))
            .collect();
        paths.extend(
            [
                "/imprint",
                "/en",
                "/de",
                "/en/",
                "/de/",
                "/en/p/2pS3xQ7Rk9mNpLz4AbCdEA",
                "/de/manage/2pS3xQ7Rk9mNpLz4AbCdEA",
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
                assert_eq!(
                    answer.headers.get(&name),
                    Some(&expected),
                    "{name} missing or wrong on {path} (default {default})"
                );
            }
        }
    }
}

/// The heart of scanner resistance: fetching the shell - or being redirected to
/// it - has no side effects, and the response does not depend on the id. If it
/// differed, an observer could infer existence from the length, the content or
/// a header.
#[tokio::test]
async fn view_shell_is_identical_for_every_id() {
    for (default, languages) in LAYOUTS {
        let h = harness_with_default_language(&format!("shell-identical-{default}"), default).await;

        // One of these ids exists, the other does not. The shell must not care.
        h.post("/api/v1/pastes", create_body(&id(1))).await;
        let present = id(1).encode();
        let absent = "AAAAAAAAAAAAAAAAAAAAAA";

        for (_, prefix) in languages {
            let existing = h.get(&format!("{prefix}/p/{present}")).await;
            let unknown = h.get(&format!("{prefix}/p/{absent}")).await;

            assert_eq!(existing.status, StatusCode::OK, "{prefix}");
            assert_eq!(unknown.status, StatusCode::OK, "{prefix}");
            assert_eq!(existing.body, unknown.body, "{prefix}: shell differs by paste id");
            assert_eq!(
                stable_headers(&existing),
                stable_headers(&unknown),
                "{prefix}: headers differ by paste id"
            );
        }

        // The prefix of the default language: the same redirect for every id.
        let existing = h.get(&format!("/{default}/p/{present}")).await;
        let unknown = h.get(&format!("/{default}/p/{absent}")).await;
        assert_eq!(existing.status, StatusCode::FOUND);
        assert_eq!(unknown.status, StatusCode::FOUND);
        assert_eq!(existing.body, unknown.body, "redirect differs by paste id");
        // The id is copied into the location, so compare everything else.
        assert_eq!(existing.header("location"), Some(format!("/p/{present}")).as_deref());
        assert_eq!(unknown.header("location"), Some(format!("/p/{absent}")).as_deref());
        let mut existing = stable_headers(&existing);
        let mut unknown = stable_headers(&unknown);
        existing.remove("location");
        unknown.remove("location");
        assert_eq!(existing, unknown, "redirect headers differ by paste id");
    }
}

/// Fetching the shell must not consume a retrieval. A link scanner following the
/// URL would otherwise burn a one-time paste - and it must not move
/// `last_retrieved_at` either, because that timestamp drives retention: a crawler
/// would silently keep abandoned capsules alive.
#[tokio::test]
async fn fetching_the_shell_does_not_touch_the_paste() {
    for (default, languages) in LAYOUTS {
        let h = harness_with_default_language(&format!("shell-no-side-effect-{default}"), default).await;
        let mut body = create_body(&id(1));
        body["burn_after_retrieval"] = serde_json::json!(true);
        assert_eq!(h.post("/api/v1/pastes", body).await.status, StatusCode::CREATED);

        let before = h.bookkeeping(&id(1)).await;

        for _ in 0..5 {
            for path in ["/p", "/manage"] {
                assert_eq!(
                    h.get(&format!("/{default}{path}/{}", id(1).encode())).await.status,
                    StatusCode::FOUND
                );
                for (_, prefix) in languages {
                    assert_eq!(h.get(&format!("{prefix}{path}/{}", id(1).encode())).await.status, StatusCode::OK);
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
}

/// The default language has no prefix: a path under its prefix goes to the same
/// path without it, whatever the id and whether or not the page is configured.
#[tokio::test]
async fn the_prefix_of_the_default_language_is_redirected_away() {
    for (default, languages) in LAYOUTS {
        let h = harness_with_default_language(&format!("shell-redirect-{default}"), default).await;

        for (rest, location) in [
            ("", "/"),
            ("/", "/"),
            ("?x=1", "/?x=1"),
            ("/?a=1&b=2", "/?a=1&b=2"),
            ("/p/2pS3xQ7Rk9mNpLz4AbCdEA", "/p/2pS3xQ7Rk9mNpLz4AbCdEA"),
            ("/manage/2pS3xQ7Rk9mNpLz4AbCdEA?x=y", "/manage/2pS3xQ7Rk9mNpLz4AbCdEA?x=y"),
            ("/report?id=2pS3xQ7Rk9mNpLz4AbCdEA", "/report?id=2pS3xQ7Rk9mNpLz4AbCdEA"),
            ("/imprint", "/imprint"),
            ("/privacy", "/privacy"),
            ("/terms", "/terms"),
        ] {
            let path = format!("/{default}{rest}");
            let answer = h.get(&path).await;

            assert_eq!(answer.status, StatusCode::FOUND, "{path}");
            assert_eq!(answer.header("location"), Some(location), "{path}");
            assert_eq!(answer.header("cache-control"), Some("no-store"), "{path}");
            assert_eq!(answer.header("vary"), None, "{path}");
        }

        // Every other language keeps its prefix; without the slash it gets one.
        for (language, prefix) in languages {
            if language == default {
                continue;
            }
            let answer = h.get(&format!("{prefix}?x=1")).await;
            assert_eq!(answer.status, StatusCode::FOUND, "{prefix}");
            assert_eq!(answer.header("location"), Some(format!("{prefix}/?x=1")).as_deref());
            assert_eq!(answer.header("cache-control"), Some("no-store"), "{prefix}");
        }
    }
}

/// The server does not negotiate: whatever `Accept-Language` says, a path is
/// answered by its prefix alone. The browser language is the head script's
/// business.
#[tokio::test]
async fn accept_language_changes_nothing() {
    let h = harness("shell-no-negotiation").await;

    for (path, language) in [("/", "en"), ("/p/2pS3xQ7Rk9mNpLz4AbCdEA", "en"), ("/de/report", "de")] {
        let plain = h.get(path).await;
        assert_eq!(plain.status, StatusCode::OK, "{path}");
        assert!(plain.text().contains(&format!(r#"<html lang="{language}">"#)), "{path}");
        assert_eq!(plain.header("vary"), None, "{path}");

        for accept_language in ["de-AT,de;q=0.9,en;q=0.8", "en", "fr", ";;; q=x, ,"] {
            let answer = h.get_with_headers(path, &[("accept-language", accept_language)]).await;
            assert_eq!(answer.status, plain.status, "{path} {accept_language}");
            assert_eq!(answer.body, plain.body, "{path} {accept_language}");
            assert_eq!(stable_headers(&answer), stable_headers(&plain), "{path} {accept_language}");
        }
    }
}

/// A prefix that is not a known language is an unknown path: same status,
/// same body. So is a shell file requested directly: it would come without its
/// markers.
#[tokio::test]
async fn unknown_prefixes_and_shell_files_are_unknown_paths() {
    for (default, _) in LAYOUTS {
        let h = harness_with_default_language(&format!("shell-unknown-prefix-{default}"), default).await;
        let unknown = h.get("/does-not-exist").await;
        assert_eq!(unknown.status, StatusCode::NOT_FOUND);

        for path in [
            "/fr/",
            "/fr",
            "/xx/p/2pS3xQ7Rk9mNpLz4AbCdEA",
            "/index.html",
            "/p.html",
            "/en/index.html",
            "/de/p.html",
            "/en/legal.html",
            "/en/INDEX.HTML",
        ] {
            let answer = h.get(path).await;
            assert_eq!(answer.status, unknown.status, "{path} (default {default})");
            assert_eq!(answer.body, unknown.body, "{path} (default {default})");
        }
    }
}

fn legal_configuration(config: &mut capsulebin_server::config::Config) {
    config.legal.operator = legal_text("Example GmbH");
    config.legal.terms = Some(capsulebin_server::config::LegalSection {
        url: Some("https://operator.example/terms".to_owned()),
        ..Default::default()
    });
}

/// The labels of the footer links, as the catalogue of `language` has them.
fn labels(language: &str) -> [&'static str; 3] {
    match language {
        "de" => ["Impressum", "Datenschutz", "Nutzungsbedingungen"],
        _ => ["Imprint", "Privacy", "Terms"],
    }
}

/// The labels are the shell's (from the catalogue of its language), the
/// targets the server's: under the shell's prefix - none for the default
/// language - for pages served here, the configured `url` for pages elsewhere.
#[tokio::test]
async fn every_shell_carries_the_configured_legal_links() {
    for (default, languages) in LAYOUTS {
        let h = harness_with(&format!("shell-legal-{default}"), |config| {
            legal_configuration(config);
            config.i18n.default_language = default.to_owned();
        })
        .await;

        for (language, prefix) in languages {
            let [imprint, privacy, terms] = labels(language);
            let mut paths: Vec<String> = pages(prefix).map(|(path, _)| path).into();
            paths.push(format!("{prefix}/imprint"));
            for path in paths {
                let text = h.get(&path).await.text();
                let footer = format!(
                    r#"<footer><a href="{prefix}/imprint">{imprint}</a><a href="{prefix}/privacy">{privacy}</a><a href="https://operator.example/terms">{terms}</a></footer>"#
                );
                assert!(text.contains(&footer), "{path} (default {default}): {text}");
            }
        }
    }
}

/// A page that is not configured has no link; the markers of the others are
/// still replaced, in every language.
#[tokio::test]
async fn links_to_pages_that_are_not_configured_are_omitted() {
    let h = harness_with("shell-legal-omitted", |config| {
        config.legal.privacy = Some(capsulebin_server::config::LegalSection {
            url: Some("https://operator.example/privacy".to_owned()),
            ..Default::default()
        });
    })
    .await;

    for (path, privacy) in [("/report", "Privacy"), ("/de/report", "Datenschutz")] {
        let text = h.get(path).await.text();
        let footer = format!(r#"<footer><a href="https://operator.example/privacy">{privacy}</a></footer>"#);
        assert!(text.contains(&footer), "{path}: {text}");
    }
}

/// Every route serves its shell with what the web build left to the server
/// filled in: the configured default language in the languages `<meta>`, the
/// shell's prefix in its internal links - and no token or marker left over.
#[tokio::test]
async fn every_shell_carries_the_default_language_and_its_prefix() {
    for (default, languages) in LAYOUTS {
        let h = harness_with(&format!("shell-tokens-{default}"), |config| {
            legal_configuration(config);
            config.i18n.default_language = default.to_owned();
        })
        .await;

        for (_, prefix) in languages {
            let mut paths: Vec<String> = pages(prefix).map(|(path, _)| path).into();
            paths.push(format!("{prefix}/privacy"));
            for path in paths {
                let text = h.get(&path).await.text();
                assert!(
                    text.contains(&format!("<head>{}<title>", languages_meta(default))),
                    "{path} (default {default}): {text}"
                );
                assert!(
                    text.contains(&format!("<nav>{}</nav>", report_link(prefix))),
                    "{path} (default {default}): {text}"
                );
                assert!(!text.contains("<!--capsulebin:"), "{path}: a marker is left: {text}");
                assert!(!text.contains("%%capsulebin:"), "{path}: a token is left: {text}");
            }
        }
    }
}

#[tokio::test]
async fn legal_pages_are_served_with_their_title_and_text() {
    for (default, languages) in LAYOUTS {
        let h = harness_with(&format!("legal-pages-{default}"), |config| {
            config.legal.operator = legal_text("Example **GmbH**");
            config.legal.hosting = legal_text("Hoster AG");
            config.i18n.default_language = default.to_owned();
        })
        .await;

        for (language, prefix) in languages {
            let [imprint_label, ..] = labels(language);
            let imprint = h.get(&format!("{prefix}/imprint")).await;
            assert_eq!(imprint.status, StatusCode::OK);
            let text = imprint.text();
            assert!(text.contains(&format!(r#"<html lang="{language}">"#)), "{text}");
            assert!(
                text.contains(&format!(
                    "<title>{imprint_label} · CapsuleBin</title></head><header>{imprint_label}</header>\
                     <main><div class=\"cb-prose cb-legal\"><h1>{imprint_label}</h1><article"
                )),
                "{text}"
            );
            assert!(text.contains("Example <strong>GmbH</strong>"), "{text}");
            assert!(text.contains("<h2>Hosting</h2>"), "{text}");
            assert_eq!(imprint.header("content-type"), Some("text/html; charset=utf-8"));
            assert_eq!(imprint.header("cache-control"), Some("no-store"));
            assert!(imprint.headers.contains_key("content-security-policy"));

            // The built-in texts in the shell's language, with their links
            // under the shell's prefix.
            let [controller, not_share] = match language {
                "de" => ["<h2>Verantwortlicher</h2>", "<h2>Was du nicht teilen darfst</h2>"],
                _ => ["<h2>Controller</h2>", "<h2>What you may not share</h2>"],
            };
            let privacy = h.get(&format!("{prefix}/privacy")).await.text();
            assert!(privacy.contains(controller), "{privacy}");
            assert!(privacy.contains(&format!(r#"href="{prefix}/imprint""#)), "{privacy}");
            let terms = h.get(&format!("{prefix}/terms")).await.text();
            assert!(terms.contains(not_share), "{terms}");
            assert!(terms.contains(&format!(r#"href="{prefix}/report""#)), "{terms}");
        }
    }
}

/// The operator's text is in the default language; in a shell of another
/// language the article says so, for screen readers. The built-in texts are
/// in the shell's language and never marked.
#[tokio::test]
async fn legal_texts_in_another_language_than_the_shell_are_marked() {
    let h = harness_with("legal-lang", legal_configuration).await;

    for (path, article) in [
        ("/de/imprint", r#"<article lang="en">"#),
        ("/imprint", "<article>"),
        ("/de/privacy", "<article>"),
        ("/privacy", "<article>"),
    ] {
        let text = h.get(path).await.text();
        assert!(text.contains(&format!("</h1>{article}")), "{path}: {text}");
    }
}

#[tokio::test]
async fn without_legal_configuration_there_are_no_pages_and_no_links() {
    let h = harness("shell-no-legal").await;

    for prefix in ["", "/de"] {
        assert!(h.get(&format!("{prefix}/")).await.text().contains("<footer></footer>"));
        for page in ["imprint", "privacy", "terms"] {
            let path = format!("{prefix}/{page}");
            let answer = h.get(&path).await;
            assert_eq!(answer.status, StatusCode::NOT_FOUND, "{path}");
            assert_eq!(answer.text(), "Not Found", "{path}");
        }
    }

    // Under the prefix of the default language it is redirected anyway: the
    // redirect depends on the path alone, the 404 comes without the prefix.
    let prefixed = h.get("/en/imprint").await;
    assert_eq!(prefixed.status, StatusCode::FOUND);
    assert_eq!(prefixed.header("location"), Some("/imprint"));
}

#[tokio::test]
async fn every_route_serves_its_own_shell() {
    for (default, languages) in LAYOUTS {
        let h = harness_with_default_language(&format!("shell-routes-{default}"), default).await;

        for (language, prefix) in languages {
            for (path, marker) in pages(prefix) {
                let answer = h.get(&path).await;

                assert_eq!(answer.status, StatusCode::OK, "{path}");
                let text = answer.text();
                assert!(text.contains(&format!("<title>{marker}</title>")), "{path} serves the wrong shell");
                assert!(
                    text.contains(&format!(r#"<html lang="{language}">"#)),
                    "{path} serves the wrong language (default {default})"
                );
                assert_eq!(answer.header("content-type"), Some("text/html; charset=utf-8"));
                // The shell must not be cached.
                assert_eq!(answer.header("cache-control"), Some("no-store"));
            }
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

/// Every `creation.mode` with what the create page's meta says for it.
const MODES: [(CreationMode, &str); 3] = [
    (CreationMode::Token, "token"),
    (CreationMode::Public, "public"),
    (CreationMode::Disabled, "disabled"),
];

/// The meta the web build writes into the head of the create shell.
const CREATION_META: &str = r#"<meta name="capsulebin-creation" content="%%capsulebin:creation%%">"#;

/// The bodies of the pages other than the create page under `prefix`, two
/// view and manage ids included, in a fixed order.
async fn other_pages(h: &Harness, prefix: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<String> = pages(prefix).into_iter().skip(1).map(|(path, _)| path).collect();
    paths.push(format!("{prefix}/p/AAAAAAAAAAAAAAAAAAAAAA"));
    paths.push(format!("{prefix}/privacy"));
    let mut bodies = Vec::new();
    for path in paths {
        let answer = h.get(&path).await;
        assert_eq!(answer.status, StatusCode::OK, "{path}");
        bodies.push((path, answer.body));
    }
    bodies
}

/// The create page names the configured mode once, in every language and
/// whichever language is the default; no other page names it, and the other
/// pages do not change with the mode.
#[tokio::test]
async fn the_create_page_names_the_creation_mode() {
    for (default, languages) in LAYOUTS {
        let mut others = Vec::new();
        for (mode, value) in MODES {
            let h = harness_with(&format!("shell-creation-{default}-{value}"), |config| {
                legal_configuration(config);
                config.i18n.default_language = default.to_owned();
                config.creation.mode = mode;
            })
            .await;
            for (language, _) in languages {
                let file = h.dir.join(language).join("index.html");
                let shell = std::fs::read_to_string(&file).unwrap();
                std::fs::write(&file, shell.replace("<title>", &format!("{CREATION_META}<title>"))).unwrap();
            }

            let mut bodies = Vec::new();
            for (_, prefix) in languages {
                let text = h.get(&format!("{prefix}/")).await.text();
                assert_eq!(text.matches("capsulebin-creation").count(), 1, "{prefix}/ ({value}): {text}");
                assert!(
                    text.contains(&format!(
                        r#"<meta name="capsulebin-creation" content="{value}"><title>create</title>"#
                    )),
                    "{prefix}/ ({value}): {text}"
                );
                let pages = other_pages(&h, prefix).await;
                assert_eq!(pages[0].1, pages[pages.len() - 2].1, "{prefix}/p/{{id}} differs by id");
                for (path, body) in &pages {
                    assert!(!String::from_utf8_lossy(body).contains("capsulebin-creation"), "{path} ({value})");
                }
                bodies.push(pages);
            }
            others.push(bodies);
        }
        assert!(
            others.windows(2).all(|pair| pair[0] == pair[1]),
            "a page other than create depends on the mode"
        );
    }
}

/// The same with the shells of the web build embedded into the binary, if it
/// has run: they are filled once at startup.
#[tokio::test]
async fn the_embedded_create_page_names_the_creation_mode() {
    for (default, languages) in LAYOUTS {
        let mut others = Vec::new();
        for (mode, value) in MODES {
            let h = harness_with_embedded_assets_and(&format!("shell-creation-embedded-{default}-{value}"), |config| {
                legal_configuration(config);
                config.i18n.default_language = default.to_owned();
                config.creation.mode = mode;
            })
            .await;
            if h.get("/health/ready").await.json()["assets"] != "ok" {
                for (_, prefix) in languages {
                    let answer = h.get(&format!("{prefix}/")).await;
                    assert_eq!(answer.status, StatusCode::INTERNAL_SERVER_ERROR, "{prefix}/ ({value})");
                    assert_eq!(answer.text(), MISSING_FRONTEND, "{prefix}/ ({value})");
                }
                continue;
            }

            let mut bodies = Vec::new();
            for (_, prefix) in languages {
                let text = h.get(&format!("{prefix}/")).await.text();
                assert_eq!(text.matches("capsulebin-creation").count(), 1, "{prefix}/ ({value})");
                assert!(
                    text.contains(&format!(r#"<meta name="capsulebin-creation" content="{value}" />"#)),
                    "{prefix}/ ({value}): {text}"
                );
                let pages = other_pages(&h, prefix).await;
                assert_eq!(pages[0].1, pages[pages.len() - 2].1, "{prefix}/p/{{id}} differs by id");
                for (path, body) in &pages {
                    assert!(!String::from_utf8_lossy(body).contains("capsulebin-creation"), "{path} ({value})");
                }
                bodies.push(pages);
            }
            others.push(bodies);
        }
        assert!(
            others.windows(2).all(|pair| pair[0] == pair[1]),
            "a page other than create depends on the mode"
        );
    }
}
