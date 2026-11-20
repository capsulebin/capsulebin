#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Acceptance tests for the language paths (package 20261119-language-paths),
//! derived from the criteria rather than the implementation: the edges the other
//! HTTP tests leave out.

mod support;

use axum::http::StatusCode;
use capsulebin_server::config::KNOWN_LANGUAGES;
use support::*;

/// Every legal page configured as text, so all three are served.
fn all_legal_pages(config: &mut capsulebin_server::config::Config) {
    config.legal.operator = legal_text("Example GmbH");
    config.legal.terms = legal_text("Own terms of use.");
}

/// AC2 + AC17: the legal pages answer `200`, HTML, `no-store`, with the security
/// headers and `<html lang>` of their language - without a prefix for the
/// default language, under `/L/` for every other one; in a German shell the
/// English text is marked `lang="en"` - for the terms as well.
#[tokio::test]
async fn ac2_ac17_every_legal_page_in_every_language() {
    for (default, languages) in LAYOUTS {
        let h = harness_with(&format!("acc-legal-all-{default}"), |config| {
            all_legal_pages(config);
            config.i18n.default_language = default.to_owned();
        })
        .await;

        for (language, prefix) in languages {
            for page in ["imprint", "privacy", "terms"] {
                let path = format!("{prefix}/{page}");
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
                assert!(!text.contains("%%capsulebin:"), "{path}: token left");

                let marked = text.contains(r#"<article class="cb-prose cb-legal" lang="en">"#);
                assert_eq!(marked, language != "en", "{path}: article lang: {text}");
            }
            assert!(h.get(&format!("{prefix}/terms")).await.text().contains("Own terms of use."));
        }
    }
}

/// AC4: the shells of `/p/{id}` and `/manage/{id}` in every language and the
/// redirect from the default language's prefix do not depend on whether the id
/// exists - nor on `Accept-Language` - and none of it burns the capsule.
#[tokio::test]
async fn ac4_redirects_and_shells_do_not_depend_on_the_id() {
    for (default, languages) in LAYOUTS {
        let h = harness_with_default_language(&format!("acc-id-independent-{default}"), default).await;
        let mut body = create_body(&id(1));
        body["burn_after_retrieval"] = serde_json::json!(true);
        assert_eq!(h.post("/api/v1/pastes", body).await.status, StatusCode::CREATED);
        let existing = id(1).encode();
        let absent = "AAAAAAAAAAAAAAAAAAAAAA";

        for page in ["p", "manage"] {
            for accept in [None, Some("de-DE,de;q=0.9"), Some("en-US")] {
                let headers: Vec<(&str, &str)> = accept.map(|value| ("accept-language", value)).into_iter().collect();
                let a = h.get_with_headers(&format!("/{default}/{page}/{existing}"), &headers).await;
                let b = h.get_with_headers(&format!("/{default}/{page}/{absent}"), &headers).await;

                assert_eq!(a.status, StatusCode::FOUND);
                assert_eq!(a.status, b.status);
                assert_eq!(a.body, b.body, "/{page} {accept:?}: redirect body differs");
                assert_eq!(a.header("location"), Some(format!("/{page}/{existing}")).as_deref());
                assert_eq!(b.header("location"), Some(format!("/{page}/{absent}")).as_deref());
                let (mut ha, mut hb) = (stable_headers(&a), stable_headers(&b));
                ha.remove("location");
                hb.remove("location");
                assert_eq!(ha, hb, "/{page} {accept:?}: redirect headers differ");

                for (_, prefix) in languages {
                    let a = h.get_with_headers(&format!("{prefix}/{page}/{existing}"), &headers).await;
                    let b = h.get_with_headers(&format!("{prefix}/{page}/{absent}"), &headers).await;
                    assert_eq!(a.status, StatusCode::OK);
                    assert_eq!(a.status, b.status);
                    assert_eq!(a.body, b.body, "{prefix}/{page}: shell differs");
                    assert_eq!(stable_headers(&a), stable_headers(&b), "{prefix}/{page}: headers differ");
                }
            }
        }

        let answer = h
            .post(&format!("/api/v1/pastes/{existing}/retrieve"), retrieve_body(REVEAL_TOKEN))
            .await;
        assert_eq!(answer.status, StatusCode::OK, "the burn-after-read capsule was consumed");
    }
}

/// AC3: a path under the default language's prefix goes to the same path
/// without it - query kept, `no-store`, security headers, no `Vary` - whatever
/// `Accept-Language` says; the path without a prefix is the page itself.
#[tokio::test]
async fn ac3_the_default_prefix_redirects_without_negotiation() {
    for (default, _) in LAYOUTS {
        let h = harness_with_default_language(&format!("acc-redirect-query-{default}"), default).await;

        for (rest, location) in [
            ("", "/"),
            ("/?a=1&b=2", "/?a=1&b=2"),
            ("/p/2pS3xQ7Rk9mNpLz4AbCdEA?x=y", "/p/2pS3xQ7Rk9mNpLz4AbCdEA?x=y"),
            ("/report?id=2pS3xQ7Rk9mNpLz4AbCdEA", "/report?id=2pS3xQ7Rk9mNpLz4AbCdEA"),
        ] {
            let path = format!("/{default}{rest}");
            for accept in ["de-CH, en;q=0.5", "en;q=0.3, de;q=0.8", "it"] {
                let answer = h.get_with_headers(&path, &[("accept-language", accept)]).await;
                assert_eq!(answer.status, StatusCode::FOUND, "{path}");
                assert_eq!(answer.header("location"), Some(location), "{path} {accept}");
                assert_eq!(answer.header("vary"), None, "{path}");
                assert_eq!(answer.header("cache-control"), Some("no-store"), "{path}");
                for (name, expected) in capsulebin_server::api::security_headers::headers() {
                    assert_eq!(answer.headers.get(&name), Some(&expected), "{name} on {path}");
                }

                let page = h.get_with_headers(location, &[("accept-language", accept)]).await;
                assert_eq!(page.status, StatusCode::OK, "{location} {accept}");
                assert!(page.text().contains(&format!(r#"<html lang="{default}">"#)), "{location} {accept}");
                assert_eq!(page.header("vary"), None, "{location}");
            }
        }
    }
}

/// AC5 (N2): every known language is offered, so no known prefix is a 404 any
/// more; a prefix that is not a known language is the generic 404 on every
/// page path, status and body identical to an unknown path.
#[tokio::test]
async fn ac5_only_unknown_language_prefixes_are_unknown_paths() {
    let h = harness_with("acc-unknown-prefix", all_legal_pages).await;
    let unknown = h.get("/does-not-exist").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    for prefix in ["", "/de"] {
        assert_eq!(h.get(&format!("{prefix}/")).await.status, StatusCode::OK, "{prefix}/");
    }
    for path in [
        "/fr/",
        "/fr/p/2pS3xQ7Rk9mNpLz4AbCdEA",
        "/xx/manage/2pS3xQ7Rk9mNpLz4AbCdEA",
        "/zz/report?id=2pS3xQ7Rk9mNpLz4AbCdEA",
        "/it/imprint",
        "/es/privacy",
        "/nl/terms",
    ] {
        let answer = h.get(path).await;
        assert_eq!(answer.status, unknown.status, "{path}");
        assert_eq!(answer.body, unknown.body, "{path}");
    }
}

/// AC6: the language list in the languages meta is written by the web build;
/// the server only fills in the default language and substitutes no languages
/// marker. A shell that still carried one would be served with it untouched.
#[tokio::test]
async fn ac6_the_server_substitutes_no_languages_marker() {
    let h = harness("acc-no-languages-marker").await;
    for (language, prefix) in [("en", ""), ("de", "/de")] {
        std::fs::write(
            h.dir.join(language).join("index.html"),
            format!(
                "<!doctype html><html lang=\"{language}\"><head><!--capsulebin:languages--><title>create</title></head>\
                 <footer></footer>"
            ),
        )
        .unwrap();

        let text = h.get(&format!("{prefix}/")).await.text();
        assert!(text.contains("<head><!--capsulebin:languages--><title>"), "{prefix}/: {text}");
        assert!(!text.contains("capsulebin-languages"), "{prefix}/: {text}");
    }
}

/// AC23: `capsulebin check` accepts `[i18n] default_language` with a known
/// language, from the file and from the environment, and rejects an unknown
/// code and every other key in `[i18n]`, naming the key. The shells come from
/// `asset_dir`, so `check` does not depend on the embedded web build.
#[test]
fn ac23_check_accepts_only_a_known_default_language_in_i18n() {
    let dir = tempdir("acc-i18n-check");
    let assets = dir.join("assets");
    write_shells(&assets);
    let valid = format!(
        "[server]\npublic_url = \"https://paste.example\"\nasset_dir = \"{}\"\n\
         [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n",
        assets.display(),
        dir.join("capsulebin.db").display()
    );
    let config_with = |i18n: &str, name: &str| {
        let path = dir.join(name);
        std::fs::write(&path, format!("{valid}{i18n}")).unwrap();
        path
    };
    let ok = config_with("", "ok.toml");

    let check = |config: &std::path::Path, env: &[(&str, &str)]| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_capsulebin"));
        command.arg("--config").arg(config).arg("check");
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().unwrap()
    };

    for (config, env) in [
        (ok.clone(), &[][..]),
        (config_with("[i18n]\n", "empty.toml"), &[][..]),
        (config_with("[i18n]\ndefault_language = \"en\"\n", "en.toml"), &[][..]),
        (config_with("[i18n]\ndefault_language = \"de\"\n", "de.toml"), &[][..]),
        (ok.clone(), &[("CAPSULEBIN__I18N__DEFAULT_LANGUAGE", "de")][..]),
    ] {
        let output = check(&config, env);
        assert!(
            output.status.success(),
            "{config:?} {env:?} rejected: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    for (config, env, key) in [
        (
            config_with("[i18n]\ndefault_language = \"fr\"\n", "fr.toml"),
            &[][..],
            "i18n.default_language",
        ),
        (
            ok.clone(),
            &[("CAPSULEBIN__I18N__DEFAULT_LANGUAGE", "fr")][..],
            "i18n.default_language",
        ),
        (
            config_with(
                "[i18n]\ndefault_language = \"en\"\nlanguages = [\"en\", \"de\"]\n",
                "languages.toml",
            ),
            &[][..],
            "languages",
        ),
        (ok.clone(), &[("CAPSULEBIN__I18N__LANGUAGES", r#"["en"]"#)][..], "languages"),
    ] {
        let output = check(&config, env);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{config:?} {env:?} accepted");
        assert!(stderr.contains(key), "{config:?} {env:?}: error does not name the key: {stderr}");
    }
}

/// AC25 (N4): a `public_url` whose path ends in a known language code is a
/// startup error naming the key - from the file and from the environment -
/// while a code elsewhere in the path or in the host is fine.
#[test]
fn ac25_check_rejects_a_public_url_ending_in_a_language_code() {
    let dir = tempdir("acc-public-url-language");
    let assets = dir.join("assets");
    write_shells(&assets);
    let config_with = |url: &str, name: &str| {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(
                "[server]\npublic_url = \"{url}\"\nasset_dir = \"{}\"\n\
                 [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n",
                assets.display(),
                dir.join("capsulebin.db").display()
            ),
        )
        .unwrap();
        path
    };
    let check = |config: &std::path::Path, env: &[(&str, &str)]| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_capsulebin"));
        command.arg("--config").arg(config).arg("check");
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().unwrap()
    };

    for (index, url) in [
        "https://paste.example/team",
        "https://de.paste.example",
        "https://paste.example/de/team",
    ]
    .into_iter()
    .enumerate()
    {
        let output = check(&config_with(url, &format!("ok-{index}.toml")), &[]);
        assert!(
            output.status.success(),
            "{url} rejected: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let ok = config_with("https://paste.example", "ok.toml");
    for &language in KNOWN_LANGUAGES {
        for (config, env) in [
            (
                config_with(&format!("https://paste.example/team/{language}"), &format!("bad-{language}.toml")),
                None,
            ),
            (ok.clone(), Some(format!("https://paste.example/{language}"))),
        ] {
            let env: Vec<(&str, &str)> = env
                .as_deref()
                .map(|url| ("CAPSULEBIN__SERVER__PUBLIC_URL", url))
                .into_iter()
                .collect();
            let output = check(&config, &env);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{config:?} {env:?} accepted");
            assert!(stderr.contains("server.public_url"), "{config:?} {env:?}: key not named: {stderr}");
        }
    }
}

/// AC25 (fourth review): paths are appended to `public_url` as text, so a base
/// with a query or a fragment - also an empty one - is a startup error naming
/// the key, from the file and from the environment, even when a language code
/// only appears behind the `?` or `#`. A percent-encoded `?` or `#` is path
/// and stays accepted.
#[test]
fn ac25_check_rejects_a_public_url_with_a_query_or_fragment() {
    let dir = tempdir("acc-public-url-query");
    let assets = dir.join("assets");
    write_shells(&assets);
    let config_with = |url: &str, name: &str| {
        let path = dir.join(name);
        std::fs::write(
            &path,
            format!(
                "[server]\npublic_url = \"{url}\"\nasset_dir = \"{}\"\n\
                 [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n",
                assets.display(),
                dir.join("capsulebin.db").display()
            ),
        )
        .unwrap();
        path
    };
    let check = |config: &std::path::Path, env: Option<&str>| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_capsulebin"));
        command.arg("--config").arg(config).arg("check");
        if let Some(url) = env {
            command.env("CAPSULEBIN__SERVER__PUBLIC_URL", url);
        }
        command.output().unwrap()
    };
    let ok = config_with("https://paste.example", "ok.toml");

    for (index, url) in ["https://paste.example/team%3Fx=1", "https://paste.example/team%23de"]
        .into_iter()
        .enumerate()
    {
        for output in [check(&config_with(url, &format!("ok-{index}.toml")), None), check(&ok, Some(url))] {
            assert!(
                output.status.success(),
                "{url} rejected: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    for (index, url) in [
        "https://paste.example/team?lang=/de",
        "https://paste.example/team#/de",
        "https://paste.example/team?x=1",
        "https://paste.example/team?",
        "https://paste.example/team#",
        "https://paste.example?x=1",
        "https://paste.example#top",
    ]
    .into_iter()
    .enumerate()
    {
        for output in [check(&config_with(url, &format!("bad-{index}.toml")), None), check(&ok, Some(url))] {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{url} accepted");
            assert!(stderr.contains("server.public_url"), "{url}: key not named: {stderr}");
            assert!(stderr.contains("query or fragment"), "{url}: reason not named: {stderr}");
        }
    }
}

/// AC3 + AC5: only a path segment that is exactly a known language code is a
/// prefix. A segment that merely starts with one, carries a region or differs
/// in case is neither redirected (default language) nor served as a page (any
/// other language): it is the generic 404, whichever language is the default.
#[tokio::test]
async fn ac3_ac5_segments_that_only_resemble_a_language_are_unknown_paths() {
    for (default, _) in LAYOUTS {
        let h = harness_with_default_language(&format!("acc-near-prefix-{default}"), default).await;
        let unknown = h.get("/does-not-exist").await;
        assert_eq!(unknown.status, StatusCode::NOT_FOUND);

        for &language in KNOWN_LANGUAGES {
            for segment in [
                format!("{language}x"),
                format!("{language}-{}", language.to_uppercase()),
                format!("x{language}"),
                format!("{language}{language}"),
            ] {
                for rest in ["", "/", "/p/2pS3xQ7Rk9mNpLz4AbCdEA", "/report?id=2pS3xQ7Rk9mNpLz4AbCdEA"] {
                    let path = format!("/{segment}{rest}");
                    let answer = h.get(&path).await;
                    assert_eq!(answer.status, unknown.status, "{path} (default {default})");
                    assert_eq!(answer.body, unknown.body, "{path} (default {default})");
                    assert_eq!(answer.header("location"), None, "{path} (default {default})");
                }
            }
        }

        // A language code further down the path is no prefix either.
        for path in [
            format!("/p/{default}"),
            format!("/report/{default}"),
            format!("/{default}/{default}/"),
        ] {
            let answer = h.get(&path).await;
            assert_ne!(
                answer.header("location"),
                Some("/"),
                "{path} (default {default}) is treated as the bare prefix"
            );
        }
    }
}

/// AC3: the redirect away from the default language's prefix stays on this
/// host. Whatever follows the prefix, the answer is either the generic 404 or a
/// `302` whose `Location` is one absolute path - never a protocol-relative
/// (`//host`) or backslash form a browser would follow to another origin.
#[tokio::test]
async fn ac3_the_redirect_never_leaves_the_host() {
    for (default, _) in LAYOUTS {
        let h = harness_with_default_language(&format!("acc-redirect-host-{default}"), default).await;

        for rest in [
            "//evil.example",
            "//evil.example/",
            "//evil.example/p/2pS3xQ7Rk9mNpLz4AbCdEA",
            "/p//evil.example",
            "/%2F%2Fevil.example",
            "/p/%2F%2Fevil.example",
            "/%5Cevil.example",
            "/p/%5C%5Cevil.example",
            "/report//evil.example",
            "?//evil.example",
            "/?next=//evil.example",
            "/report?id=https://evil.example/",
        ] {
            let path = format!("/{default}{rest}");
            let answer = h.get(&path).await;

            match answer.status {
                StatusCode::NOT_FOUND => assert_eq!(answer.header("location"), None, "{path}"),
                StatusCode::FOUND => {
                    let location = answer.header("location").expect("a redirect has a location");
                    let target = location.split('?').next().unwrap_or_default();
                    assert!(target.starts_with('/'), "{path} -> {location}");
                    assert!(!target.starts_with("//"), "{path} -> {location}");
                    assert!(!target.contains('\\'), "{path} -> {location}");
                    assert!(!target.contains("://"), "{path} -> {location}");
                    // Nothing but the prefix is taken away.
                    let expected = if rest.starts_with('/') {
                        rest.to_owned()
                    } else {
                        format!("/{rest}")
                    };
                    assert_eq!(location, expected, "{path}");
                }
                other => assert_eq!(other, StatusCode::NOT_FOUND, "{path}"),
            }
        }
    }
}

/// AC2 + AC3: only the page paths exist under a language prefix. The API, the
/// health endpoints and static files have none - neither under the prefix of
/// another language (no page) nor under the default language's (no redirect) -
/// and a page path with a missing or surplus segment is no page. A shell file
/// stays unreachable however its name is spelled. All of it is the generic 404.
#[tokio::test]
async fn ac2_ac3_only_page_paths_exist_under_a_language_prefix() {
    for (default, _) in LAYOUTS {
        let h = harness_with_default_language(&format!("acc-prefix-pages-only-{default}"), default).await;
        std::fs::write(h.dir.join("language-init.js"), "/* head script */").unwrap();
        let unknown = h.get("/does-not-exist").await;
        assert_eq!(unknown.status, StatusCode::NOT_FOUND);

        // Without a prefix they are there.
        assert_eq!(h.get("/language-init.js").await.status, StatusCode::OK);
        assert_eq!(h.get("/health/live").await.status, StatusCode::OK);
        assert_eq!(h.get("/api/v1/reports").await.status, StatusCode::OK);

        for &language in KNOWN_LANGUAGES {
            for rest in [
                "/language-init.js",
                "/health/live",
                "/health/ready",
                "/api/v1/reports",
                "/assets/index.js",
                "/p",
                "/p/",
                "/p/2pS3xQ7Rk9mNpLz4AbCdEA/extra",
                "/manage",
                "/manage/",
                "/report/",
                "/report/2pS3xQ7Rk9mNpLz4AbCdEA",
                "/legal",
                "/index",
                "/index.html/",
                "/index%2Ehtml",
                "/index.htm%6C",
                "/index.html;x",
                "/index.html.",
                "//index.html",
            ] {
                let path = format!("/{language}{rest}");
                let answer = h.get(&path).await;
                assert_eq!(answer.status, unknown.status, "{path} (default {default})");
                assert_eq!(answer.body, unknown.body, "{path} (default {default})");
                assert_eq!(answer.header("location"), None, "{path} (default {default})");
            }
        }

        // A second prefix is no page and no redirect either.
        for path in ["/en/de/", "/de/en/", "/de/de/p/2pS3xQ7Rk9mNpLz4AbCdEA", "/en/en/report"] {
            let answer = h.get(path).await;
            assert_eq!(answer.status, unknown.status, "{path} (default {default})");
            assert_eq!(answer.body, unknown.body, "{path} (default {default})");
        }
    }
}
