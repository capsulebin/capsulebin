#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The operator's theme colours (package 20261120-branding-colours). The
//! stylesheet (AC3): served from memory at `/branding/<16 hex digits of
//! SHA-256>.css`, none without colours. Its link (AC4): last in the head of
//! every page in every language, the rest of the page unchanged.

mod support;

use axum::http::StatusCode;
use capsulebin_server::config::Config;
use support::*;

/// Light and dark values, keys in any case and order.
const COLORS: &str = "[light]\ndanger = \"#C00\"\naccent = \"#0f766e\"\n[dark]\naccent = \"#5EEAD4\"\n";

/// The stylesheet [`COLORS`] is expected to give, written out here rather
/// than taken from the server.
const STYLESHEET: &str = "@media not all and (prefers-color-scheme: dark) {
    :root {
        --cb-accent: #0f766e;
        --cb-danger: #cc0000;
    }
}
@media (prefers-color-scheme: dark) {
    :root {
        --cb-accent: #5eead4;
    }
}
:root[data-theme=\"light\"] {
    --cb-accent: #0f766e;
    --cb-danger: #cc0000;
}
:root[data-theme=\"dark\"] {
    --cb-accent: #5eead4;
}
";

/// The stylesheet the web build links at the end of every head.
const BUNDLED_STYLESHEET: &str = r#"<link rel="stylesheet" crossorigin href="/assets/index-0123abcd.css">"#;

fn colored(config: &mut Config) {
    config.branding.colors = toml::from_str(COLORS).unwrap();
}

fn stylesheet_url() -> String {
    branding_url(STYLESHEET.as_bytes(), "css")
}

fn stylesheet_link() -> String {
    format!(r#"<link rel="stylesheet" href="{}">"#, stylesheet_url())
}

async fn assert_served(h: &Harness) {
    let url = stylesheet_url();
    let answer = h.get(&url).await;
    assert_eq!(answer.status, StatusCode::OK, "{url}");
    assert_eq!(answer.text(), STYLESHEET);
    assert_eq!(answer.header("content-type"), Some("text/css; charset=utf-8"));
    assert_eq!(answer.header("cache-control"), Some("public, max-age=31536000, immutable"));
    for (name, expected) in capsulebin_server::api::security_headers::headers() {
        assert_eq!(answer.headers.get(&name), Some(&expected), "{name}");
    }
}

/// Every path in `paths` answers exactly like an unknown path.
async fn assert_generic_404(h: &Harness, paths: &[String]) {
    let unknown = h.get("/no-such-path").await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    for path in paths {
        let answer = h.get(path).await;
        assert_eq!(answer.status, unknown.status, "{path}");
        assert_eq!(answer.body, unknown.body, "{path}");
        assert_eq!(stable_headers(&answer), stable_headers(&unknown), "{path}");
    }
}

/// `.css` paths under `/branding/` that are not the stylesheet.
fn other_css_paths() -> Vec<String> {
    let url = stylesheet_url();
    let stem = url.trim_end_matches(".css");
    vec![
        "/branding/0000000000000000.css".to_owned(),
        "/branding/colors.css".to_owned(),
        "/branding/.css".to_owned(),
        format!("{stem}.CSS"),
        format!("{}.css", &stem[..stem.len() - 1]),
        format!("{url}/"),
        format!("/de{url}"),
        branding_url(b"", "css"),
    ]
}

#[tokio::test]
async fn ac3_the_stylesheet_is_served_from_memory() {
    let h = harness_with("colors-served", colored).await;
    assert_served(&h).await;
    assert_generic_404(&h, &other_css_paths()).await;
}

#[tokio::test]
async fn ac3_the_stylesheet_is_served_with_embedded_assets() {
    let h = harness_with_embedded_assets_and("colors-embedded", colored).await;
    assert_served(&h).await;
}

#[tokio::test]
async fn ac3_without_colours_every_branding_stylesheet_is_the_generic_404() {
    let h = harness_with("colors-none", |_| {}).await;
    let mut paths = other_css_paths();
    paths.push(stylesheet_url());
    assert_generic_404(&h, &paths).await;
}

/// Every page below a language prefix, the legal pages included.
const PAGES: [&str; 7] = [
    "/",
    "/p/AAAAAAAAAAAAAAAAAAAAAA",
    "/manage/AAAAAAAAAAAAAAAAAAAAAA",
    "/report",
    "/imprint",
    "/privacy",
    "/terms",
];

/// A harness whose shells link a bundled stylesheet at the end of the head,
/// as the web build writes them, with the imprint configured, so that all
/// three legal pages exist.
async fn pages_harness(name: &str, adjust: impl FnOnce(&mut Config)) -> Harness {
    let h = harness_with(name, |config| {
        config.legal.operator = legal_text("Example GmbH");
        adjust(config);
    })
    .await;
    for language in capsulebin_server::config::KNOWN_LANGUAGES {
        for file in ["index.html", "p.html", "manage.html", "report.html", "legal.html"] {
            let path = h.dir.join(language).join(file);
            let shell = std::fs::read_to_string(&path).unwrap();
            assert_eq!(shell.matches("</head>").count(), 1, "{}", path.display());
            std::fs::write(&path, shell.replace("</head>", &format!("{BUNDLED_STYLESHEET}\n</head>"))).unwrap();
        }
    }
    h
}

/// Every page in `en` and `de` of both instances.
async fn pages(with: &Harness, without: &Harness) -> Vec<(String, String, String)> {
    let mut pages = Vec::new();
    for prefix in ["", "/de"] {
        for path in PAGES {
            let path = format!("{prefix}{path}");
            let (colored, plain) = (with.get(&path).await, without.get(&path).await);
            assert_eq!(colored.status, StatusCode::OK, "{path}");
            assert_eq!(plain.status, StatusCode::OK, "{path}");
            pages.push((path, colored.text(), plain.text()));
        }
    }
    pages
}

/// AC4: one link, after the bundled stylesheet and right before `</head>`;
/// apart from it the page is the one without colours.
#[tokio::test]
async fn ac4_every_page_links_the_stylesheet_last_in_the_head() {
    let with = pages_harness("colors-pages", colored).await;
    let without = pages_harness("colors-pages-none", |_| {}).await;
    let link = stylesheet_link();
    for (path, colored, plain) in pages(&with, &without).await {
        assert_eq!(colored.matches(&link).count(), 1, "{path}: {colored}");
        assert_eq!(colored.matches("/branding/").count(), 1, "{path}: {colored}");
        let head = colored.split("</head>").next().unwrap();
        assert!(head.ends_with(&format!("{BUNDLED_STYLESHEET}\n{link}")), "{path}: {head}");
        assert_eq!(colored.replacen(&link, "", 1), plain, "{path}");
    }
}

/// AC4: without colours no page links a branding stylesheet; the shell is
/// filled in as before.
#[tokio::test]
async fn ac4_without_colours_no_page_links_a_stylesheet() {
    let h = pages_harness("colors-pages-plain", |_| {}).await;
    for prefix in ["", "/de"] {
        for path in PAGES {
            let path = format!("{prefix}{path}");
            let page = h.get(&path).await.text();
            assert!(!page.contains("/branding/"), "{path}: {page}");
            assert_eq!(page.matches(r#"rel="stylesheet""#).count(), 1, "{path}: {page}");
            assert!(page.contains(&format!("{BUNDLED_STYLESHEET}\n</head>")), "{path}: {page}");
        }
    }
}
