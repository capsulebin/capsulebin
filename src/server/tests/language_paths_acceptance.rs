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

/// AC2 + AC17: `/L/imprint`, `/L/privacy`, `/L/terms` answer `200`, HTML, `no-store`,
/// with the security headers and `<html lang="L">`; under `/de/` the English text
/// is marked `lang="en"` - for the terms as well.
#[tokio::test]
async fn ac2_ac17_every_legal_page_under_every_prefix() {
    let h = harness_with("acc-legal-all", all_legal_pages).await;

    for &language in KNOWN_LANGUAGES {
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
            let (mut ha, mut hb) = (stable_headers(&a), stable_headers(&b));
            ha.remove("location");
            hb.remove("location");
            assert_eq!(ha, hb, "/{page} {accept:?}: redirect headers differ");
        }

        for &language in KNOWN_LANGUAGES {
            let a = h.get(&format!("/{language}/{page}/{existing}")).await;
            let b = h.get(&format!("/{language}/{page}/{absent}")).await;
            assert_eq!(a.status, StatusCode::OK);
            assert_eq!(a.status, b.status);
            assert_eq!(a.body, b.body, "/{language}/{page}: shell differs");
            assert_eq!(stable_headers(&a), stable_headers(&b), "/{language}/{page}: headers differ");
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

/// AC5 (N2): every known language is offered, so no known prefix is a 404 any
/// more; a prefix that is not a known language is the generic 404 on every
/// page path, status and body identical to an unknown path.
#[tokio::test]
async fn ac5_only_unknown_language_prefixes_are_unknown_paths() {
    let h = harness_with("acc-unknown-prefix", all_legal_pages).await;
    let unknown = h.get("/does-not-exist").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    for &language in KNOWN_LANGUAGES {
        assert_eq!(h.get(&format!("/{language}/")).await.status, StatusCode::OK, "/{language}/");
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

/// AC6 (N2): the languages meta is written by the web build; the server
/// substitutes no languages marker. A shell that still carried one would be
/// served with it untouched, the language list is not the server's business.
#[tokio::test]
async fn ac6_the_server_substitutes_no_languages_marker() {
    let h = harness("acc-no-languages-marker").await;
    for &language in KNOWN_LANGUAGES {
        std::fs::write(
            h.dir.join(language).join("index.html"),
            format!(
                "<!doctype html><html lang=\"{language}\"><head><!--capsulebin:languages--><title>create</title></head>\
                 <footer><!--capsulebin:legal-links--></footer>"
            ),
        )
        .unwrap();

        let text = h.get(&format!("/{language}/")).await.text();
        assert!(text.contains("<head><!--capsulebin:languages--><title>"), "/{language}/: {text}");
        assert!(!text.contains("capsulebin-languages"), "/{language}/: {text}");
    }
}

/// AC23 (N2): `capsulebin check` rejects an `[i18n]` table and the
/// `CAPSULEBIN__I18N__*` variables like any other unknown key, while the same
/// configuration without them is valid.
#[test]
fn ac23_check_rejects_an_i18n_table_and_its_env_variables() {
    let dir = tempdir("acc-i18n-check");
    let valid = format!(
        "[server]\npublic_url = \"https://paste.example\"\n[database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n",
        dir.join("capsulebin.db").display()
    );
    let ok = dir.join("ok.toml");
    std::fs::write(&ok, &valid).unwrap();
    let with_table = dir.join("i18n.toml");
    std::fs::write(
        &with_table,
        format!("{valid}[i18n]\ndefault_language = \"en\"\nlanguages = [\"en\", \"de\"]\n"),
    )
    .unwrap();

    let check = |config: &std::path::Path, env: &[(&str, &str)]| {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_capsulebin"));
        command.arg("--config").arg(config).arg("check");
        for (key, value) in env {
            command.env(key, value);
        }
        command.output().unwrap()
    };

    let output = check(&ok, &[]);
    assert!(
        output.status.success(),
        "valid configuration rejected: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for (config, env) in [
        (&with_table, &[][..]),
        (&ok, &[("CAPSULEBIN__I18N__LANGUAGES", r#"["en"]"#)][..]),
        (&ok, &[("CAPSULEBIN__I18N__DEFAULT_LANGUAGE", "de")][..]),
    ] {
        let output = check(config, env);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{config:?} {env:?} accepted");
        assert!(stderr.contains("i18n"), "{config:?} {env:?}: error does not name the key: {stderr}");
    }
}
