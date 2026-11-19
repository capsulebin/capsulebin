//! Serving the client application.
//!
//! The client is a multi-page application with no client-side router, so every
//! route maps to exactly one HTML document. Every page exists once per offered
//! language, under its prefix (`/en/p/{id}`, `/de/p/{id}`), built from
//! `dist/<language>/<shell>.html`. A page path without a prefix - above all the
//! share and management links - is redirected by `Accept-Language`; a prefix
//! that is not offered is an unknown path.
//!
//! Key property: `GET /<language>/p/{id}` returns the same bytes for **every**
//! syntactically valid id and triggers no retrieval, and so does the redirect
//! from `/p/{id}`. A link scanner following the URL therefore neither burns a
//! one-time paste nor learns from the response whether the id exists.

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::AppState;
use crate::legal::{LegalPages, Which};

/// Where the footer of every HTML shell takes the operator's legal links.
///
/// HTML comments, so that a shell served without substitution (the Vite dev
/// server) still renders, just without the links.
pub const LEGAL_LINKS_MARKER: &str = "<!--capsulebin:legal-links-->";
/// Where `legal.html` takes the page title (in `<title>` and the header).
pub const LEGAL_TITLE_MARKER: &str = "<!--capsulebin:legal-title-->";
/// Where `legal.html` takes the rendered text.
pub const LEGAL_CONTENT_MARKER: &str = "<!--capsulebin:legal-content-->";
/// Where the `<head>` of every HTML shell takes the offered languages and the
/// default, as `<meta name="capsulebin-languages" content="en de" data-default="en">`.
pub const LANGUAGES_MARKER: &str = "<!--capsulebin:languages-->";

/// Page paths without a language prefix. Each is redirected to the same path
/// under the prefix the request prefers.
const UNPREFIXED_PAGES: [&str; 7] = ["/", "/p/{id}", "/manage/{id}", "/report", "/imprint", "/privacy", "/terms"];

/// Longest `Accept-Language` value considered. A longer one is not parsed and
/// gets the default language, like a malformed one.
const MAX_ACCEPT_LANGUAGE_BYTES: usize = 1024;

/// Built frontend assets, embedded into the binary.
///
/// `web/dist` is produced by the frontend build; `build.rs` creates
/// the directory if it is missing so that `cargo build` works without a prior
/// `npm run build`. Missing assets then surface at runtime via `/health/ready`
/// instead of as a compile error.
#[derive(rust_embed::Embed)]
#[folder = "web/dist"]
struct EmbeddedAssets;

/// Which HTML shell a route is served with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    Create,
    View,
    Manage,
    Report,
    /// Imprint, privacy notice and terms: one shell, filled by the server.
    Legal,
}

impl Shell {
    const ALL: [Self; 5] = [Self::Create, Self::View, Self::Manage, Self::Report, Self::Legal];

    fn file(self) -> &'static str {
        match self {
            Self::Create => "index.html",
            Self::View => "p.html",
            Self::Manage => "manage.html",
            Self::Report => "report.html",
            Self::Legal => "legal.html",
        }
    }

    /// The shell of `language` in the build output.
    fn file_for(self, language: &str) -> String {
        format!("{language}/{}", self.file())
    }
}

/// The interface languages as the page routes need them, prepared once at
/// startup from the resolved configuration.
#[derive(Debug)]
pub struct Languages {
    /// Offered languages in configuration order, each with the legal pages and
    /// footer links for its shells.
    offered: Vec<(&'static str, LegalPages)>,
    default: &'static str,
    /// What [`LANGUAGES_MARKER`] is replaced with.
    meta: String,
}

impl Languages {
    pub fn new(offered: Vec<(&'static str, LegalPages)>, default: &'static str) -> Self {
        let codes: Vec<&str> = offered.iter().map(|(code, _)| *code).collect();
        let meta = format!(
            r#"<meta name="capsulebin-languages" content="{}" data-default="{}">"#,
            crate::legal::escape(&codes.join(" ")),
            crate::legal::escape(default)
        );
        Self { offered, default, meta }
    }

    /// The offered language codes, in configuration order.
    pub fn codes(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.offered.iter().map(|(code, _)| *code)
    }

    fn legal(&self, language: &str) -> Option<&LegalPages> {
        self.offered.iter().find_map(|(code, legal)| (*code == language).then_some(legal))
    }

    /// The language a request without a prefix is sent to: the offered one
    /// `Accept-Language` weighs highest, otherwise the default. A missing,
    /// malformed or overlong header is no error, just no preference.
    fn negotiate(&self, accept_language: Option<&HeaderValue>) -> &'static str {
        accept_language
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.len() <= MAX_ACCEPT_LANGUAGE_BYTES)
            .and_then(|value| preferred(value, &self.codes().collect::<Vec<_>>()))
            .unwrap_or(self.default)
    }
}

pub fn routes(languages: &Languages) -> Router<AppState> {
    let mut router = Router::new();
    for path in UNPREFIXED_PAGES {
        router = router.route(path, get(redirect));
    }
    for language in languages.codes() {
        router = router
            .route(
                &format!("/{language}"),
                get(move |uri| async move { found(&format!("/{language}/{}", query(&uri))) }),
            )
            .route(&format!("/{language}/"), get(move |s| shell(s, language, Shell::Create)))
            .route(&format!("/{language}/p/{{id}}"), get(move |s| shell(s, language, Shell::View)))
            .route(
                &format!("/{language}/manage/{{id}}"),
                get(move |s| shell(s, language, Shell::Manage)),
            )
            .route(&format!("/{language}/report"), get(move |s| shell(s, language, Shell::Report)))
            .route(&format!("/{language}/imprint"), get(move |s| legal(s, language, Which::Imprint)))
            .route(&format!("/{language}/privacy"), get(move |s| legal(s, language, Which::Privacy)))
            .route(&format!("/{language}/terms"), get(move |s| legal(s, language, Which::Terms)));
    }
    router.fallback(get(static_file))
}

/// Sends a page path without a prefix to the same path under the preferred
/// language. Only the path is copied, never inspected, so the answer for
/// `/p/{id}` does not depend on whether the id exists; the fragment with the
/// key never reaches the server and stays in the browser.
async fn redirect(State(state): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    let language = state.languages.negotiate(headers.get(header::ACCEPT_LANGUAGE));
    found(&format!("/{language}{}{}", uri.path(), query(&uri)))
}

/// `?` and the query of `uri`, or nothing.
fn query(uri: &Uri) -> String {
    uri.query().map(|query| format!("?{query}")).unwrap_or_default()
}

/// A `302` to a path on this host. It depends on `Accept-Language`, so it must
/// neither be cached nor be shared between clients by a cache.
fn found(location: &str) -> Response {
    let Ok(location) = HeaderValue::from_str(location) else {
        return not_found();
    };
    let mut response = StatusCode::FOUND.into_response();
    let headers = response.headers_mut();
    headers.insert(header::LOCATION, location);
    headers.insert(header::VARY, HeaderValue::from_static("Accept-Language"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The offered language with the highest weight in an `Accept-Language` value
/// (RFC 9110), matched by its primary subtag (`de-AT` is `de`). `q=0` rules a
/// language out; malformed entries are skipped; on equal weight the earlier
/// entry wins.
fn preferred(accept_language: &str, offered: &[&'static str]) -> Option<&'static str> {
    let mut best: Option<(u16, &'static str)> = None;
    for entry in accept_language.split(',') {
        let Some((primary, weight)) = parse_entry(entry) else {
            continue;
        };
        let Some(code) = offered.iter().copied().find(|code| code.eq_ignore_ascii_case(primary)) else {
            continue;
        };
        if weight > 0 && best.is_none_or(|(highest, _)| weight > highest) {
            best = Some((weight, code));
        }
    }
    best.map(|(_, code)| code)
}

/// One `Accept-Language` entry, `de-AT;q=0.8`, as its primary subtag and its
/// weight in thousandths (`("de", 800)`). `None` if it is malformed or `*`.
fn parse_entry(entry: &str) -> Option<(&str, u16)> {
    let mut parts = entry.split(';');
    let tag = parts.next()?.trim();
    let primary = tag.split('-').next()?;
    if primary.is_empty() || primary.len() > 8 || !primary.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let mut weight = 1000;
    for parameter in parts {
        let (name, value) = parameter.split_once('=')?;
        if name.trim().eq_ignore_ascii_case("q") {
            weight = parse_weight(value.trim())?;
        }
    }
    Some((primary, weight))
}

/// A `qvalue` (`0`, `0.5`, `1.000`) in thousandths; `None` if it is not one.
fn parse_weight(value: &str) -> Option<u16> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let thousandths = fraction
        .bytes()
        .chain(std::iter::repeat(b'0'))
        .take(3)
        .fold(0u16, |sum, digit| sum * 10 + u16::from(digit - b'0'));
    match whole {
        "0" => Some(thousandths),
        "1" if thousandths == 0 => Some(1000),
        _ => None,
    }
}

/// Serves an HTML shell. The path id is deliberately **not** inspected: reading
/// it would be the first step towards a response that depends on whether it
/// exists.
async fn shell(State(state): State<AppState>, language: &'static str, which: Shell) -> Response {
    let Some(legal) = state.languages.legal(language) else {
        return not_found();
    };
    let file = which.file_for(language);
    match state.assets.load(&file) {
        Some(bytes) => html(render(bytes, &state.languages, legal).into_bytes()),
        None => missing_frontend(&file),
    }
}

/// A legal page: the `legal.html` shell with the page put in. Not configured,
/// or configured as a link elsewhere: the same `404` as any unknown path.
async fn legal(State(state): State<AppState>, language: &'static str, which: Which) -> Response {
    let Some(legal) = state.languages.legal(language) else {
        return not_found();
    };
    let Some(page) = legal.page(which) else {
        return not_found();
    };
    let file = Shell::Legal.file_for(language);
    match state.assets.load(&file) {
        Some(bytes) => html(
            render(bytes, &state.languages, legal)
                .replace(LEGAL_TITLE_MARKER, &crate::legal::escape(page.title))
                .replace(LEGAL_CONTENT_MARKER, &page.html)
                .into_bytes(),
        ),
        None => missing_frontend(&file),
    }
}

/// Puts the offered languages and the legal links into a shell. The same for
/// every request, so the shell still does not depend on the path id. The
/// shells are UTF-8; a lossy conversion only matters for a broken build, which
/// `/health/ready` reports.
fn render(bytes: Vec<u8>, languages: &Languages, legal: &LegalPages) -> String {
    String::from_utf8_lossy(&bytes)
        .replace(LANGUAGES_MARKER, &languages.meta)
        .replace(LEGAL_LINKS_MARKER, &legal.footer)
}

/// Static assets (JS, CSS, icons). No directory listing and no path traversal:
/// `rust-embed` only knows the embedded names, and the development mode
/// validates the path. No HTML either: a shell is only served through its
/// route, with the markers replaced.
async fn static_file(State(state): State<AppState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if path.to_ascii_lowercase().ends_with(".html") {
        return not_found();
    }

    match state.assets.load(path) {
        Some(bytes) => {
            let mime = mime_for(path);
            let mut response = (StatusCode::OK, bytes).into_response();
            response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            // Hashed Vite assets are immutable; HTML is never cached.
            if path.starts_with("assets/") {
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                );
            }
            response
        }
        // Unknown path: generic, with no hint about what does exist.
        None => not_found(),
    }
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not Found").into_response()
}

fn html(bytes: Vec<u8>) -> Response {
    let mut response = (StatusCode::OK, bytes).into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    // The shell must not be cached, so that a deployment takes effect at once.
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn missing_frontend(file: &str) -> Response {
    tracing::error!(file, "frontend asset missing - was `npm run build` run in src/server/web/?");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Frontend assets are missing. In development, run `npm run build` in src/server/web/ first.",
    )
        .into_response()
}

fn mime_for(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, ext)| ext) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Where assets are read from.
///
/// Production uses the binary exclusively. The directory mode exists only for
/// local development and is enabled through `CAPSULEBIN__SERVER__ASSET_DIR`.
#[derive(Clone, Debug)]
pub enum AssetSource {
    Embedded,
    Directory(std::path::PathBuf),
}

impl AssetSource {
    pub fn from_env() -> Self {
        match std::env::var_os("CAPSULEBIN__SERVER__ASSET_DIR") {
            Some(dir) => {
                let path = std::path::PathBuf::from(dir);
                tracing::warn!(
                    path = %path.display(),
                    "serving assets from a directory - development only"
                );
                Self::Directory(path)
            }
            None => Self::Embedded,
        }
    }

    /// Whether every shell exists for each of `languages`; without them there
    /// is no application. Used by `/health/ready`.
    pub fn has_shells<'a>(&self, languages: impl IntoIterator<Item = &'a str>) -> bool {
        languages
            .into_iter()
            .all(|language| Shell::ALL.iter().all(|shell| self.exists(&shell.file_for(language))))
    }

    /// Whether an asset exists, without loading it.
    pub fn exists(&self, path: &str) -> bool {
        match self {
            Self::Embedded => EmbeddedAssets::get(path).is_some(),
            Self::Directory(_) => self.load(path).is_some(),
        }
    }

    fn load(&self, path: &str) -> Option<Vec<u8>> {
        match self {
            Self::Embedded => EmbeddedAssets::get(path).map(|file| file.data.into_owned()),
            Self::Directory(root) => {
                // Rule out path traversal: accept plain segments only.
                if path.is_empty()
                    || path
                        .split('/')
                        .any(|segment| segment.is_empty() || segment == "." || segment == ".." || segment.contains('\\'))
                {
                    return None;
                }
                std::fs::read(root.join(path)).ok()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_route_maps_to_one_shell() {
        assert_eq!(Shell::Create.file(), "index.html");
        assert_eq!(Shell::View.file(), "p.html");
        assert_eq!(Shell::Manage.file(), "manage.html");
        assert_eq!(Shell::Report.file(), "report.html");
        assert_eq!(Shell::Legal.file(), "legal.html");
    }

    #[test]
    fn the_language_and_footer_markers_are_replaced() {
        let legal = LegalPages {
            footer: "<a>x</a>".to_owned(),
            ..LegalPages::default()
        };
        let languages = Languages::new(vec![("de", LegalPages::default()), ("en", LegalPages::default())], "en");
        let shell = format!("<head>{LANGUAGES_MARKER}</head><footer>{LEGAL_LINKS_MARKER}</footer>").into_bytes();
        assert_eq!(
            render(shell, &languages, &legal),
            r#"<head><meta name="capsulebin-languages" content="de en" data-default="en"></head><footer><a>x</a></footer>"#
        );
    }

    #[test]
    fn accept_language_picks_the_offered_language_weighed_highest() {
        let offered = ["en", "de"];
        for (header, expected) in [
            ("de", Some("de")),
            ("de-AT", Some("de")),
            ("DE-at", Some("de")),
            ("fr, de;q=0.5, en;q=0.4", Some("de")),
            ("en;q=0.4, de;q=0.5", Some("de")),
            ("de;q=0.5, en;q=0.5", Some("de")),
            ("en-US,en;q=0.9,de;q=0.8", Some("en")),
            ("de;q=0, en;q=0.1", Some("en")),
            ("de;q=0.000", None),
            ("de;q=1.0", Some("de")),
            ("de ; q = 0.7", Some("de")),
            ("fr, it", None),
            ("*", None),
            ("", None),
            ("de;q=2", None),
            ("de;q=1.5", None),
            ("de;q=0.5555", None),
            ("de;q=abc", None),
            ("de;q", None),
            ("123", None),
            ("x-klingon", None),
            ("deutsch-de", None),
        ] {
            assert_eq!(preferred(header, &offered), expected, "{header:?}");
        }
    }

    #[test]
    fn a_missing_malformed_or_overlong_header_gets_the_default() {
        let languages = Languages::new(vec![("en", LegalPages::default()), ("de", LegalPages::default())], "de");
        let value = |text: &str| HeaderValue::from_str(text).ok();

        assert_eq!(languages.negotiate(None), "de");
        assert_eq!(languages.negotiate(value("en").as_ref()), "en");
        assert_eq!(languages.negotiate(value(";;;,,,q=").as_ref()), "de");
        let overlong = format!("{}en", "x;q=0.1,".repeat(200));
        assert_eq!(languages.negotiate(value(&overlong).as_ref()), "de");
        let not_text = HeaderValue::from_bytes(b"en\xff").ok();
        assert_eq!(languages.negotiate(not_text.as_ref()), "de");
    }

    /// The template a shell is generated from, once per language, with its path.
    fn template(shell: Shell) -> (std::path::PathBuf, String) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web").join(shell.file());
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        (path, source)
    }

    /// The substitution only works if every shell has the marker in its footer.
    #[test]
    fn every_shell_source_carries_the_legal_links_marker() {
        for shell in Shell::ALL {
            let (path, source) = template(shell);
            assert_eq!(
                source.matches(LEGAL_LINKS_MARKER).count(),
                1,
                "{} must contain the legal links marker exactly once",
                path.display()
            );
        }
    }

    /// `language-init.js` reads the offered languages synchronously from the
    /// `<meta>` that replaces the marker, so the marker must be in the `<head>`
    /// before the script.
    #[test]
    fn every_shell_source_carries_the_languages_marker_before_the_head_script() {
        for shell in Shell::ALL {
            let (path, source) = template(shell);
            assert_eq!(
                source.matches(LANGUAGES_MARKER).count(),
                1,
                "{} must contain the languages marker exactly once",
                path.display()
            );
            let marker = source.find(LANGUAGES_MARKER);
            let script = source.find(r#"<script src="/language-init.js"></script>"#);
            let head_end = source.find("</head>");
            assert!(
                matches!((marker, script, head_end), (Some(marker), Some(script), Some(head_end)) if marker < script && script < head_end),
                "{}: the languages marker must be in the <head>, before language-init.js",
                path.display()
            );
        }
    }

    #[test]
    fn the_legal_shell_carries_its_title_and_content_markers() {
        let (_, source) = template(Shell::Legal);
        assert_eq!(source.matches(LEGAL_TITLE_MARKER).count(), 2, "<title> and header");
        assert_eq!(source.matches(LEGAL_CONTENT_MARKER).count(), 1);
    }

    #[test]
    fn directory_mode_rejects_path_traversal() {
        let source = AssetSource::Directory(std::path::PathBuf::from("/nonexistent-root"));

        for hostile_path in ["../Cargo.toml", "a/../../etc/passwd", "..", "./x", "", "a//b", "a\\b"] {
            assert!(source.load(hostile_path).is_none(), "{hostile_path} should have been rejected");
        }
    }
}
