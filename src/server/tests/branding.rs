#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! The operator's branding (package 20261120-branding-identity). The files
//! (AC4): served from memory at `/branding/<16 hex digits of SHA-256>.<ext>`,
//! every other `/branding/` path the generic `404` of an unknown path. The
//! pages (AC5, AC6, AC7, AC9, AC10, AC15): name, logo, favicon, tagline and
//! attribution link filled into every shell in every language, with test
//! shells written like the web build writes the real ones.

mod support;

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use capsulebin_server::branding::Tagline;
use capsulebin_server::config::{BrandingTexts, Config};
use support::*;

/// Writes the three files and returns their paths.
fn files(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = tempdir(&format!("{name}-files"));
    let write = |file: &str, bytes: &[u8]| {
        let path = dir.join(file);
        std::fs::write(&path, bytes).unwrap();
        path
    };
    (write("logo.png", PNG), write("logo-dark.webp", WEBP), write("favicon.ico", ICO))
}

fn branded(name: &str) -> impl FnOnce(&mut Config) {
    let (logo, logo_dark, favicon) = files(name);
    move |config| {
        config.branding.logo = Some(logo);
        config.branding.logo_dark = Some(logo_dark);
        config.branding.favicon = Some(favicon);
    }
}

async fn assert_served(h: &Harness) {
    let urls = [branding_url(PNG, "png"), branding_url(WEBP, "webp"), branding_url(ICO, "ico")];
    assert_eq!(
        urls.iter().collect::<std::collections::BTreeSet<_>>().len(),
        3,
        "different files, different URLs"
    );

    for (path, bytes, content_type) in [
        (&urls[0], PNG, "image/png"),
        (&urls[1], WEBP, "image/webp"),
        (&urls[2], ICO, "image/x-icon"),
    ] {
        let answer = h.get(path).await;
        assert_eq!(answer.status, StatusCode::OK, "{path}");
        assert_eq!(answer.body, bytes, "{path}: body");
        assert_eq!(answer.header("content-type"), Some(content_type), "{path}");
        assert_eq!(
            answer.header("cache-control"),
            Some("public, max-age=31536000, immutable"),
            "{path}"
        );
        for (name, expected) in capsulebin_server::api::security_headers::headers() {
            assert_eq!(answer.headers.get(&name), Some(&expected), "{name} on {path}");
        }
    }
}

/// Every `/branding/` path in `paths` answers exactly like an unknown path.
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

/// Paths under `/branding/` that are not a configured file: near misses of
/// the real URLs and paths that never were one.
fn other_paths() -> Vec<String> {
    let png = branding_url(PNG, "png");
    let stem = png.trim_end_matches(".png");
    let hash = stem.trim_start_matches("/branding/");
    vec![
        "/branding".to_owned(),
        "/branding/".to_owned(),
        "/branding/0000000000000000.png".to_owned(),
        "/branding/logo.png".to_owned(),
        format!("{stem}.webp"),
        format!("{stem}.ico"),
        format!("{stem}.PNG"),
        stem.to_owned(),
        format!("/branding/{}.png", hash.to_uppercase()),
        format!("/branding/{}.png", &hash[..15]),
        format!("{png}/"),
        format!("{png}/x"),
        format!("/de{png}"),
    ]
}

#[tokio::test]
async fn ac4_branding_files_are_served_from_memory() {
    let h = harness_with("branding-served", branded("branding-served")).await;
    assert_served(&h).await;
}

/// The files come from the configuration, not from the assets: the same in
/// the embedded mode.
#[tokio::test]
async fn ac4_branding_files_are_served_with_embedded_assets() {
    let h = harness_with_embedded_assets_and("branding-embedded", branded("branding-embedded")).await;
    assert_served(&h).await;
}

/// A file read at startup is served as it was then: replacing it takes a
/// restart.
#[tokio::test]
async fn ac4_a_file_is_read_once_at_startup() {
    let (logo, _, _) = files("branding-once");
    let h = harness_with("branding-once", |config| config.branding.logo = Some(logo.clone())).await;
    std::fs::write(&logo, b"\x89PNG\r\n\x1a\n replaced").unwrap();

    assert_eq!(h.get(&branding_url(PNG, "png")).await.body, PNG);
    assert_generic_404(&h, &[branding_url(b"\x89PNG\r\n\x1a\n replaced", "png")]).await;
}

/// The same file for logo and favicon is one route (and no panic from a
/// duplicate route).
#[tokio::test]
async fn ac4_the_same_file_twice_is_one_url() {
    let (logo, _, _) = files("branding-twice");
    let h = harness_with("branding-twice", |config| {
        config.branding.logo = Some(logo.clone());
        config.branding.logo_dark = Some(logo.clone());
        config.branding.favicon = Some(logo);
    })
    .await;
    assert_eq!(h.get(&branding_url(PNG, "png")).await.status, StatusCode::OK);
}

#[tokio::test]
async fn ac4_other_branding_paths_are_the_generic_404() {
    let h = harness_with("branding-404", branded("branding-404")).await;
    assert_generic_404(&h, &other_paths()).await;
}

#[tokio::test]
async fn ac4_without_branding_files_every_branding_path_is_the_generic_404() {
    let h = harness_with("branding-none", |_| {}).await;
    let mut paths = other_paths();
    paths.extend([branding_url(PNG, "png"), branding_url(WEBP, "webp"), branding_url(ICO, "ico")]);
    assert_generic_404(&h, &paths).await;
}

// --- the pages ---

/// A test shell's header as the web build writes it: the built-in symbol and
/// the name, each between its markers over several lines. The slogan or page
/// title follows the link.
const HEADER: &str = "<header><a class=\"cb-logo\" href=\"%%capsulebin:prefix%%/\">
<!--capsulebin:logo-->
<svg width=\"18\" height=\"20\" aria-hidden=\"true\">
<path d=\"M9 1.2 16.4 5.4\" />
</svg>
<!--/capsulebin:logo-->
<!--capsulebin:wordmark-->
<span class=\"cb-logo-name\">%%capsulebin:name%%</span>
<!--/capsulebin:wordmark-->
</a>";

const NAME_TOKEN: &str = "%%capsulebin:name%%";
const DEFAULT_SOURCE: &str = "https://github.com/capsulebin/capsulebin";

const SOURCE_TOKEN: &str = "%%capsulebin:source%%";

/// Every page below a language prefix: its path, the shell it is built from,
/// the catalogue key of its title (`None`: the name alone), of the catalogue
/// text that names the service, if it has one, and of the text next to the
/// logo, if it has one (the create page's slogan, the others' page title).
type Page = (
    &'static str,
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    Option<&'static str>,
);
const PAGES: [Page; 7] = [
    ("/", "index.html", None, Some("create.noscript"), Some("create.claim")),
    (
        "/p/AAAAAAAAAAAAAAAAAAAAAA",
        "p.html",
        Some("view.title"),
        Some("view.noscript"),
        None,
    ),
    (
        "/manage/AAAAAAAAAAAAAAAAAAAAAA",
        "manage.html",
        Some("manage.title"),
        None,
        Some("manage.claim"),
    ),
    (
        "/report",
        "report.html",
        Some("shell.reportContent"),
        Some("report.back"),
        Some("shell.reportContent"),
    ),
    ("/imprint", "legal.html", Some("shell.imprint"), None, Some("shell.imprint")),
    ("/privacy", "legal.html", Some("shell.privacy"), None, Some("shell.privacy")),
    ("/terms", "legal.html", Some("shell.terms"), None, Some("shell.terms")),
];

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// A catalogue text as the web build puts it into a shell: escaped, with the
/// name placeholder turned into the name token.
fn text(language: &str, key: &str) -> String {
    escape(catalogue(language)[key].as_str().unwrap()).replace("{name}", NAME_TOKEN)
}

/// The attribution as the web build writes it: the catalogue text, escaped,
/// with only the project's name linked to the source token.
fn attribution(language: &str) -> String {
    let link = format!(r#"<a href="{SOURCE_TOKEN}" target="_blank" rel="noopener noreferrer">CapsuleBin</a>"#);
    format!("<span>{}</span>", text(language, "shell.poweredBy").replace("{project}", &link))
}

/// Replaces the shells of the harness with ones that carry everything the
/// branding fills in: the name in the title, the header and a catalogue text,
/// the favicon link, the logo, wordmark and (create page) tagline regions and
/// the attribution after the legal footer markers. `legal.html` titled like
/// the real one.
fn write_branding_shells(dir: &Path) {
    for language in capsulebin_server::config::KNOWN_LANGUAGES {
        for (_, file, title, named, claim) in PAGES {
            let title = match (file, title) {
                ("legal.html", _) => format!("<!--capsulebin:legal-title--> · {NAME_TOKEN}"),
                (_, Some(key)) => format!("{} · {NAME_TOKEN}", text(language, key)),
                (_, None) => NAME_TOKEN.to_owned(),
            };
            let main = match (file, named) {
                ("legal.html", _) => "<h1><!--capsulebin:legal-title--></h1><!--capsulebin:legal-content-->".to_owned(),
                (_, Some(key)) => format!("<p data-named>{}</p>", text(language, key)),
                (_, None) => String::new(),
            };
            let claim = match (file, claim) {
                ("index.html", Some(key)) => format!(
                    "\n<!--capsulebin:tagline-->\n<span class=\"cb-claim\">{}</span>\n<!--/capsulebin:tagline-->\n",
                    text(language, key)
                ),
                ("legal.html", _) => "<span class=\"cb-claim\"><!--capsulebin:legal-title--></span>".to_owned(),
                (_, Some(key)) => format!("<span class=\"cb-claim\">{}</span>", text(language, key)),
                (_, None) => String::new(),
            };
            let shell = format!(
                "<!doctype html><html lang=\"{language}\"><head><title>{title}</title>\n\
                 <link rel=\"icon\" href=\"%%capsulebin:favicon%%\" />\n</head><body>{HEADER}{claim}</header><main>{main}</main>\
                 <footer>{}\n{}</footer></body></html>",
                legal_link_markers(language),
                attribution(language),
            );
            std::fs::write(dir.join(language).join(file), shell).unwrap();
        }
    }
}

/// A harness on the branding test shells, with `adjust` applied and the
/// imprint configured, so that all three legal pages exist.
async fn pages_harness(name: &str, adjust: impl FnOnce(&mut Config)) -> Harness {
    let h = harness_with(name, |config| {
        config.legal.operator = legal_text("Example GmbH");
        adjust(config);
    })
    .await;
    write_branding_shells(&h.dir);
    h
}

/// What every page of an instance is expected to show.
struct Expected {
    /// The name, escaped.
    name: String,
    favicon: String,
    /// `source_url`, escaped.
    source: String,
    /// The images in place of the built-in symbol, or `None` for the symbol.
    logo: Option<String>,
    /// Whether the name is shown next to the logo.
    wordmark: bool,
    /// The slogan of the create page per language, a text escaped; a
    /// language not listed has the built-in one.
    taglines: Vec<(&'static str, Tagline)>,
}

impl Expected {
    fn tagline(&self, language: &str) -> &Tagline {
        self.taglines
            .iter()
            .find_map(|(code, tagline)| (*code == language).then_some(tagline))
            .unwrap_or(&Tagline::BuiltIn)
    }
}

impl Default for Expected {
    fn default() -> Self {
        Self {
            name: "CapsuleBin".to_owned(),
            favicon: "/favicon.svg".to_owned(),
            source: DEFAULT_SOURCE.to_owned(),
            logo: None,
            wordmark: true,
            taglines: Vec::new(),
        }
    }
}

/// Checks every page in `en` and `de` against `expected`.
async fn assert_pages(h: &Harness, expected: &Expected) {
    for (language, prefix) in [("en", ""), ("de", "/de")] {
        for (path, file, title, named, claim) in PAGES {
            let path = format!("{prefix}{path}");
            let answer = h.get(&path).await;
            assert_eq!(answer.status, StatusCode::OK, "{path}");
            let page = answer.text();
            let name = &expected.name;

            let title = match title {
                Some(key) => format!("{} · {name}", text(language, key)),
                None => name.clone(),
            };
            assert!(page.contains(&format!("<title>{title}</title>")), "{path}: title: {page}");
            if let Some(key) = named {
                let text = text(language, key).replace(NAME_TOKEN, name);
                assert!(page.contains(&format!("<p data-named>{text}</p>")), "{path}: {key}: {page}");
            }
            assert_eq!(
                page.matches(&format!(r#"<link rel="icon" href="{}" />"#, expected.favicon)).count(),
                1,
                "{path}: favicon: {page}"
            );

            let header = page
                .split("<header>")
                .nth(1)
                .and_then(|rest| rest.split("</header>").next())
                .unwrap();
            assert!(
                header.starts_with(&format!(r#"<a class="cb-logo" href="{prefix}/">"#)),
                "{path}: {header}"
            );
            match &expected.logo {
                None => assert!(header.contains("<svg") && !header.contains("<img"), "{path}: symbol: {header}"),
                Some(images) => {
                    assert!(header.contains(images.as_str()), "{path}: logo: {header}");
                    assert!(!header.contains("<svg"), "{path}: symbol left: {header}");
                    assert_eq!(header.matches("<img").count(), images.matches("<img").count(), "{path}: {header}");
                }
            }
            let wordmark = format!(r#"<span class="cb-logo-name">{name}</span>"#);
            assert_eq!(header.contains(&wordmark), expected.wordmark, "{path}: wordmark: {header}");
            assert_eq!(header.contains("cb-logo-name"), expected.wordmark, "{path}: wordmark: {header}");

            // The create page has the tagline next to the logo, the others
            // keep their page title there.
            let tagline = expected.tagline(language);
            let claim = match (file, tagline, claim) {
                ("index.html", Tagline::Text(text), _) => Some(text.clone()),
                ("index.html", Tagline::Hidden, _) | (_, _, None) => None,
                (_, _, Some(key)) => Some(text(language, key)),
            };
            if let Some(claim) = &claim {
                let span = format!(r#"<span class="cb-claim">{claim}</span>"#);
                assert!(header.contains(&span), "{path}: claim: {header}");
            }
            assert_eq!(
                header.matches("cb-claim").count(),
                usize::from(claim.is_some()),
                "{path}: claim: {header}"
            );

            // The attribution keeps the project's name, whatever the instance's
            // is, and only that name is the link.
            let before = if language == "de" { "Betrieben mit " } else { "Powered by " };
            let link = format!(
                r#"<a href="{}" target="_blank" rel="noopener noreferrer">CapsuleBin</a>"#,
                expected.source
            );
            assert_eq!(
                page.matches(&format!("<span>{before}{link}</span>")).count(),
                1,
                "{path}: attribution: {page}"
            );
            assert_eq!(page.matches(before.trim_end()).count(), 1, "{path}: attribution: {page}");
            assert_eq!(page.matches(r#"target="_blank""#).count(), 1, "{path}: attribution: {page}");
            assert_eq!(page.matches(">CapsuleBin</a>").count(), 1, "{path}: attribution: {page}");

            // Nothing of the build is left, apart from what the name and the
            // tagline bring.
            let mut rest = page.replace(name.as_str(), "");
            if let Some(claim) = &claim {
                rest = rest.replace(claim.as_str(), "");
            }
            assert!(!rest.contains("%%capsulebin:"), "{path}: a token is left: {page}");
            assert!(!rest.contains("<!--capsulebin:"), "{path}: a marker is left: {page}");
        }
    }
}

/// AC5: without `[branding]` every page names CapsuleBin, shows the built-in
/// symbol and links the built-in favicon and the source code.
#[tokio::test]
async fn ac5_without_branding_every_page_names_capsulebin() {
    let h = pages_harness("branding-pages-default", |_| {}).await;
    assert_pages(&h, &Expected::default()).await;
}

/// AC6: the name replaces CapsuleBin everywhere, as text.
#[tokio::test]
async fn ac6_the_name_is_escaped_on_every_page() {
    let h = pages_harness("branding-pages-name", |config| {
        config.branding.name = r#"A&B <i>x</i> "q""#.to_owned();
    })
    .await;
    assert_pages(
        &h,
        &Expected {
            name: "A&amp;B &lt;i&gt;x&lt;/i&gt; &quot;q&quot;".to_owned(),
            ..Expected::default()
        },
    )
    .await;
    for path in ["/", "/de/report", "/de/imprint", "/terms"] {
        assert!(!h.get(path).await.text().contains("<i>"), "{path}");
    }
}

/// AC6: a name that looks like a token or a marker of the build is put in
/// as it is, not interpreted.
#[tokio::test]
async fn ac6_a_name_like_a_token_or_a_marker_stays_literal() {
    let h = pages_harness("branding-pages-literal", |config| {
        config.branding.name = "%%capsulebin:prefix%% <!--capsulebin:legal-link:imprint:x-->".to_owned();
    })
    .await;
    assert_pages(
        &h,
        &Expected {
            name: "%%capsulebin:prefix%% &lt;!--capsulebin:legal-link:imprint:x--&gt;".to_owned(),
            ..Expected::default()
        },
    )
    .await;
    for path in ["/", "/de/p/AAAAAAAAAAAAAAAAAAAAAA", "/imprint", "/de/privacy"] {
        let page = h.get(path).await.text();
        assert!(!page.contains(">x</a>"), "{path}: the name became a link: {page}");
        assert!(!page.contains("<title>/de "), "{path}: the name became a prefix: {page}");
    }
}

/// AC7: a logo takes the place of the symbol; the name stays next to it, so
/// the image has an empty `alt`.
#[tokio::test]
async fn ac7_a_logo_replaces_the_symbol() {
    let (logo, _, _) = files("branding-pages-logo");
    let h = pages_harness("branding-pages-logo", |config| config.branding.logo = Some(logo)).await;
    assert_pages(
        &h,
        &Expected {
            logo: Some(format!(r#"<img class="cb-logo-img" src="{}" alt="">"#, branding_url(PNG, "png"))),
            ..Expected::default()
        },
    )
    .await;
}

/// AC7: with `hide_name` the logo names the instance, in its `alt`; the
/// title still has the name.
#[tokio::test]
async fn ac7_hide_name_leaves_the_name_to_the_logo() {
    let (logo, _, _) = files("branding-pages-hide");
    let h = pages_harness("branding-pages-hide", |config| {
        config.branding.name = r#"A&B <i>x</i> "q""#.to_owned();
        config.branding.logo = Some(logo);
        config.branding.hide_name = true;
    })
    .await;
    let name = "A&amp;B &lt;i&gt;x&lt;/i&gt; &quot;q&quot;";
    assert_pages(
        &h,
        &Expected {
            name: name.to_owned(),
            logo: Some(format!(
                r#"<img class="cb-logo-img" src="{}" alt="{name}">"#,
                branding_url(PNG, "png")
            )),
            wordmark: false,
            ..Expected::default()
        },
    )
    .await;
}

/// AC7, AC9, AC10, AC15 with every key of `[branding]` set: two logos for the
/// two themes, the configured favicon (which answers), the taglines and the
/// attribution link to the escaped `source_url`.
#[tokio::test]
async fn ac9_ac10_every_key_set() {
    let h = pages_harness("branding-pages-all", |config| {
        branded("branding-pages-all")(config);
        config.branding.name = "Team Paste".to_owned();
        config.branding.hide_name = true;
        config.branding.source_url = "https://example.org/src?a=1&b=2".to_owned();
        config.branding.tagline = Some("Ours".to_owned());
        config.branding.languages.insert("de".to_owned(), tagline("Unser"));
    })
    .await;
    let (png, webp, ico) = (branding_url(PNG, "png"), branding_url(WEBP, "webp"), branding_url(ICO, "ico"));
    assert_pages(
        &h,
        &Expected {
            name: "Team Paste".to_owned(),
            favicon: ico.clone(),
            source: "https://example.org/src?a=1&amp;b=2".to_owned(),
            logo: Some(format!(
                r#"<img class="cb-logo-img cb-logo-img-light" src="{png}" alt="Team Paste"><img class="cb-logo-img cb-logo-img-dark" src="{webp}" alt="Team Paste">"#
            )),
            wordmark: false,
            taglines: vec![("en", Tagline::Text("Ours".to_owned())), ("de", Tagline::Text("Unser".to_owned()))],
        },
    )
    .await;
    let favicon = h.get(&ico).await;
    assert_eq!(favicon.status, StatusCode::OK);
    assert_eq!(favicon.body, ICO);
}

/// AC9 without a logo: only the favicon link changes.
#[tokio::test]
async fn ac9_a_favicon_alone() {
    let (_, _, favicon) = files("branding-pages-favicon");
    let h = pages_harness("branding-pages-favicon", |config| config.branding.favicon = Some(favicon)).await;
    let ico = branding_url(ICO, "ico");
    assert_pages(
        &h,
        &Expected {
            favicon: ico.clone(),
            ..Expected::default()
        },
    )
    .await;
    assert_eq!(h.get(&ico).await.status, StatusCode::OK);
}

/// `[branding.<lang>]` with only a tagline.
fn tagline(text: &str) -> BrandingTexts {
    BrandingTexts {
        tagline: Some(text.to_owned()),
    }
}

/// The expected taglines, unescaped texts.
fn taglines(en: Tagline, de: Tagline) -> Expected {
    Expected {
        taglines: vec![("en", en), ("de", de)],
        ..Expected::default()
    }
}

fn configured(text: &str) -> Tagline {
    Tagline::Text(text.to_owned())
}

/// AC15: `tagline` alone is the slogan of every language.
#[tokio::test]
async fn ac15_the_tagline_applies_to_every_language() {
    let h = pages_harness("branding-tagline-all", |config| {
        config.branding.tagline = Some("Our pastes".to_owned());
    })
    .await;
    assert_pages(&h, &taglines(configured("Our pastes"), configured("Our pastes"))).await;
}

/// AC15: `[branding.de]` overrides `tagline` for German only.
#[tokio::test]
async fn ac15_a_language_table_overrides_the_tagline() {
    let h = pages_harness("branding-tagline-override", |config| {
        config.branding.tagline = Some("Our pastes".to_owned());
        config.branding.languages.insert("de".to_owned(), tagline("Unsere Pastes"));
    })
    .await;
    assert_pages(&h, &taglines(configured("Our pastes"), configured("Unsere Pastes"))).await;
}

/// AC15: with only `[branding.de]`, English keeps the built-in slogan.
#[tokio::test]
async fn ac15_only_a_language_table() {
    let h = pages_harness("branding-tagline-de", |config| {
        config.branding.languages.insert("de".to_owned(), tagline("Unsere Pastes"));
    })
    .await;
    assert_pages(&h, &taglines(Tagline::BuiltIn, configured("Unsere Pastes"))).await;
}

/// AC15: an empty tagline removes the slogan element of the create page, in
/// every language it applies to; the other pages keep their title.
#[tokio::test]
async fn ac15_an_empty_tagline_removes_the_slogan() {
    let h = pages_harness("branding-tagline-empty", |config| {
        config.branding.tagline = Some(String::new());
    })
    .await;
    assert_pages(&h, &taglines(Tagline::Hidden, Tagline::Hidden)).await;

    let h = pages_harness("branding-tagline-empty-de", |config| {
        config.branding.tagline = Some("Our pastes".to_owned());
        config.branding.languages.insert("de".to_owned(), tagline(""));
    })
    .await;
    assert_pages(&h, &taglines(configured("Our pastes"), Tagline::Hidden)).await;
}

/// AC15: the tagline is escaped and put in literally, like the name.
#[tokio::test]
async fn ac15_the_tagline_is_escaped_and_stays_literal() {
    let h = pages_harness("branding-tagline-literal", |config| {
        config.branding.tagline = Some(r#"A&B <i>x</i> "q" %%capsulebin:prefix%%"#.to_owned());
        config.branding.languages.insert(
            "de".to_owned(),
            tagline("<!--capsulebin:legal-link:imprint:x--> %%capsulebin:name%%"),
        );
    })
    .await;
    assert_pages(
        &h,
        &taglines(
            configured("A&amp;B &lt;i&gt;x&lt;/i&gt; &quot;q&quot; %%capsulebin:prefix%%"),
            configured("&lt;!--capsulebin:legal-link:imprint:x--&gt; %%capsulebin:name%%"),
        ),
    )
    .await;
    for path in ["/", "/de/"] {
        let page = h.get(path).await.text();
        assert!(!page.contains("<i>"), "{path}: {page}");
        assert!(!page.contains(">x</a>"), "{path}: the tagline became a link: {page}");
    }
}
