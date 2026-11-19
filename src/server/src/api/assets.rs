//! Serving the client application.
//!
//! The client is a multi-page application with no client-side router, so every
//! route maps to exactly one HTML document. Every page exists once per known
//! language, built from `dist/<language>/<shell>.html`: in the default language
//! (`i18n.default_language`) at the path without a prefix (`/p/{id}`), in every
//! other one under its prefix (`/de/p/{id}`). The prefix of the default
//! language is redirected to the path without it; a prefix that is not a known
//! language is an unknown path. The server does not look at `Accept-Language`:
//! the browser's language is applied by the head script of the shell.
//!
//! Key property: `GET /p/{id}` and `GET /<language>/p/{id}` return the same
//! bytes for **every** syntactically valid id and trigger no retrieval, and so
//! does the redirect. A link scanner following the URL therefore neither burns
//! a one-time paste nor learns from the response whether the id exists.

use axum::{
    Router,
    extract::State,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::AppState;
use crate::config::{KNOWN_LANGUAGES, language_prefix};
use crate::legal::{LegalPages, Which};

/// Start of the footer marker of one legal link in every HTML shell:
/// `<!--capsulebin:legal-link:<page>:<label>-->`, `<page>` as in
/// [`Which::path`]. The web build writes the label from the catalogue of the
/// shell's language, HTML-escaped, so it holds no `>` and cannot end the
/// comment early; the server only decides whether the link appears and where
/// it points.
///
/// HTML comments, so that a shell served without substitution still renders,
/// just without the links.
pub const LEGAL_LINK_MARKER: &str = "<!--capsulebin:legal-link:";
/// Where `legal.html` takes the page title (in `<title>` and the header).
pub const LEGAL_TITLE_MARKER: &str = "<!--capsulebin:legal-title-->";
/// Where `legal.html` takes the rendered text.
pub const LEGAL_CONTENT_MARKER: &str = "<!--capsulebin:legal-content-->";
/// What the web build writes in front of every internal link of a shell
/// (`href="%%capsulebin:prefix%%/report"`): the language prefix of the shell,
/// which depends on the configured default language.
pub const PREFIX_TOKEN: &str = "%%capsulebin:prefix%%";
/// What the web build writes where a shell names the default language
/// (`data-default` of the languages `<meta>`).
pub const DEFAULT_TOKEN: &str = "%%capsulebin:default%%";

/// What a page route serves.
#[derive(Clone, Copy)]
enum Page {
    Shell(Shell),
    Legal(Which),
}

/// Every page path, below the language prefix.
const PAGES: [(&str, Page); 7] = [
    ("/", Page::Shell(Shell::Create)),
    ("/p/{id}", Page::Shell(Shell::View)),
    ("/manage/{id}", Page::Shell(Shell::Manage)),
    ("/report", Page::Shell(Shell::Report)),
    ("/imprint", Page::Legal(Which::Imprint)),
    ("/privacy", Page::Legal(Which::Privacy)),
    ("/terms", Page::Legal(Which::Terms)),
];

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

/// The legal pages and footer links of every interface language, prepared
/// once at startup from the resolved configuration.
#[derive(Debug)]
pub struct LegalByLanguage {
    legal: Vec<(&'static str, LegalPages)>,
}

impl LegalByLanguage {
    pub fn new(legal: Vec<(&'static str, LegalPages)>) -> Self {
        Self { legal }
    }

    fn get(&self, language: &str) -> Option<&LegalPages> {
        self.legal.iter().find_map(|(code, legal)| (*code == language).then_some(legal))
    }
}

pub fn routes(default_language: &'static str) -> Router<AppState> {
    let mut router = Router::new();
    for &language in KNOWN_LANGUAGES {
        let prefix = language_prefix(default_language, language);
        for (path, which) in PAGES {
            router = router.route(&format!("{prefix}{path}"), get(move |s| page(s, language, which)));
        }
        if language == default_language {
            // The default language has no prefix: `/en/p/{id}` is `/p/{id}`.
            let without_prefix = move |uri: Uri| async move { found(&unprefixed(&uri, language)) };
            router = router.route(&format!("/{language}"), get(without_prefix));
            for (path, _) in PAGES {
                router = router.route(&format!("/{language}{path}"), get(without_prefix));
            }
        } else {
            router = router.route(
                &format!("/{language}"),
                get(move |uri| async move { found(&format!("/{language}/{}", query(&uri))) }),
            );
        }
    }
    router.fallback(get(static_file))
}

/// The path and query of `uri` without the prefix of `language`. The rest is
/// only copied, never inspected, so the answer for `/en/p/{id}` does not depend
/// on whether the id exists; the fragment with the key never reaches the server
/// and stays in the browser.
fn unprefixed(uri: &Uri, language: &str) -> String {
    let path = uri
        .path()
        .strip_prefix('/')
        .and_then(|path| path.strip_prefix(language))
        .filter(|rest| rest.starts_with('/'))
        .unwrap_or("/");
    format!("{path}{}", query(uri))
}

/// `?` and the query of `uri`, or nothing.
fn query(uri: &Uri) -> String {
    uri.query().map(|query| format!("?{query}")).unwrap_or_default()
}

/// A `302` to a path on this host. Not cached, so that a changed default
/// language takes effect at once.
fn found(location: &str) -> Response {
    let Ok(location) = HeaderValue::from_str(location) else {
        return not_found();
    };
    let mut response = StatusCode::FOUND.into_response();
    let headers = response.headers_mut();
    headers.insert(header::LOCATION, location);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Serves a page in `language`. The path id is deliberately **not** inspected:
/// reading it would be the first step towards a response that depends on
/// whether it exists.
///
/// A legal page is the `legal.html` shell with the page put in. Not configured,
/// or configured as a link elsewhere: the same `404` as any unknown path.
async fn page(State(state): State<AppState>, language: &'static str, which: Page) -> Response {
    let Some(legal) = state.legal.get(language) else {
        return not_found();
    };
    let (shell, legal_page) = match which {
        Page::Shell(shell) => (shell, None),
        Page::Legal(which) => match legal.page(which) {
            Some(page) => (Shell::Legal, Some(page)),
            None => return not_found(),
        },
    };
    let file = shell.file_for(language);
    let Some(bytes) = state.assets.load(&file) else {
        return missing_frontend(&file);
    };
    let mut document = render(bytes, legal, language, state.default_language);
    // After the tokens, so that the operator's text is put in as it is.
    if let Some(page) = legal_page {
        document = document
            .replace(LEGAL_TITLE_MARKER, &crate::legal::escape(page.title))
            .replace(LEGAL_CONTENT_MARKER, &page.html);
    }
    html(document.into_bytes())
}

/// Fills in what a shell of `language` leaves to the server: the prefix of its
/// internal links, the default language and the legal links. All fixed at
/// startup and the same for every request, so the shell still does not depend
/// on the path id. The shells are UTF-8; a lossy conversion only matters for a
/// broken build, which `/health/ready` reports.
fn render(bytes: Vec<u8>, legal: &LegalPages, language: &str, default_language: &str) -> String {
    let shell = String::from_utf8_lossy(&bytes)
        .replace(PREFIX_TOKEN, &language_prefix(default_language, language))
        .replace(DEFAULT_TOKEN, default_language);
    Which::ALL
        .into_iter()
        .fold(shell, |shell, which| with_legal_link(&shell, which, legal.footer.href(which)))
}

/// Replaces every footer marker of `which` with a link to `href` labelled as
/// the marker says, or removes it when the page is not configured. A marker
/// without its end is left as it is.
fn with_legal_link(shell: &str, which: Which, href: Option<&str>) -> String {
    let start = format!("{LEGAL_LINK_MARKER}{}:", which.path());
    let mut out = String::with_capacity(shell.len());
    let mut rest = shell;
    while let Some(at) = rest.find(&start) {
        let after = &rest[at + start.len()..];
        let Some(end) = after.find("-->") else { break };
        out.push_str(&rest[..at]);
        if let Some(href) = href {
            // The label is already escaped by the web build.
            out.push_str(&format!(r#"<a href="{}">{}</a>"#, crate::legal::escape(href), &after[..end]));
        }
        rest = &after[end + "-->".len()..];
    }
    out.push_str(rest);
    out
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

    /// Whether every shell exists for each known language; without them there
    /// is no application. Used by `/health/ready`.
    pub fn has_shells(&self) -> bool {
        KNOWN_LANGUAGES
            .iter()
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
    use crate::legal::FooterLinks;

    #[test]
    fn every_route_maps_to_one_shell() {
        assert_eq!(Shell::Create.file(), "index.html");
        assert_eq!(Shell::View.file(), "p.html");
        assert_eq!(Shell::Manage.file(), "manage.html");
        assert_eq!(Shell::Report.file(), "report.html");
        assert_eq!(Shell::Legal.file(), "legal.html");
    }

    #[test]
    fn the_footer_markers_become_links_or_disappear() {
        let legal = LegalPages {
            footer: FooterLinks {
                imprint: Some("/de/imprint".to_owned()),
                privacy: None,
                terms: Some("https://operator.example/terms?a=1&b=2".to_owned()),
            },
            ..LegalPages::default()
        };
        let shell = "<footer><!--capsulebin:legal-link:imprint:Impressum-->\
                     <!--capsulebin:legal-link:privacy:Datenschutz-->\
                     <!--capsulebin:legal-link:terms:Nutzungs&amp;bedingungen--></footer>";
        assert_eq!(
            render(shell.as_bytes().to_vec(), &legal, "de", "en"),
            r#"<footer><a href="/de/imprint">Impressum</a><a href="https://operator.example/terms?a=1&amp;b=2">Nutzungs&amp;bedingungen</a></footer>"#
        );
    }

    /// A broken build: what is not a complete marker stays.
    #[test]
    fn shells_without_complete_markers_are_left_alone() {
        let legal = LegalPages {
            footer: FooterLinks {
                imprint: Some("/imprint".to_owned()),
                ..FooterLinks::default()
            },
            ..LegalPages::default()
        };
        for shell in ["<footer></footer>", "<footer><!--capsulebin:legal-link:imprint:Imprint</footer>"] {
            assert_eq!(render(shell.as_bytes().to_vec(), &legal, "de", "en"), shell);
        }
    }

    #[test]
    fn the_tokens_follow_the_default_language() {
        let shell = r#"<meta data-default="%%capsulebin:default%%"><a href="%%capsulebin:prefix%%/"></a><a href="%%capsulebin:prefix%%/report"></a>"#;
        let render = |language, default| render(shell.as_bytes().to_vec(), &LegalPages::default(), language, default);

        let unprefixed = |default| format!(r#"<meta data-default="{default}"><a href="/"></a><a href="/report"></a>"#);
        assert_eq!(render("en", "en"), unprefixed("en"));
        assert_eq!(render("de", "de"), unprefixed("de"));
        assert_eq!(
            render("de", "en"),
            r#"<meta data-default="en"><a href="/de/"></a><a href="/de/report"></a>"#
        );
        assert_eq!(
            render("en", "de"),
            r#"<meta data-default="de"><a href="/en/"></a><a href="/en/report"></a>"#
        );
    }

    #[test]
    fn the_prefix_of_the_default_language_is_dropped_from_the_path() {
        for (uri, expected) in [
            ("/en", "/"),
            ("/en/", "/"),
            ("/en?x=1", "/?x=1"),
            ("/en/p/abc", "/p/abc"),
            ("/en/report?id=abc", "/report?id=abc"),
            ("/en/p/%2F%2Fhost.example", "/p/%2F%2Fhost.example"),
        ] {
            assert_eq!(unprefixed(&Uri::from_static(uri), "en"), expected, "{uri}");
        }
    }

    /// The template a shell is generated from, once per language, with its path.
    fn template(shell: Shell) -> (std::path::PathBuf, String) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web").join(shell.file());
        let source = std::fs::read_to_string(&path).unwrap_or_default();
        (path, source)
    }

    /// The substitution only works if every shell has the markers in its
    /// footer, each with its label from the catalogue.
    #[test]
    fn every_shell_source_carries_the_legal_link_markers() {
        for shell in Shell::ALL {
            let (path, source) = template(shell);
            assert_eq!(source.matches(LEGAL_LINK_MARKER).count(), 3, "{}", path.display());
            for which in Which::ALL {
                let marker = format!("{LEGAL_LINK_MARKER}{0}:{{{{shell.{0}}}}}-->", which.path());
                assert_eq!(source.matches(&marker).count(), 1, "{} must contain {marker} once", path.display());
            }
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
