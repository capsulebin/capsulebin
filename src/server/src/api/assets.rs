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
//!
//! Nothing in a page depends on the request, so the pages of the embedded
//! assets are put together once, when the routes are built, and served from
//! memory. Only the directory mode reads its shells on every request.
//!
//! The operator's branding (`[branding]`) is filled in the same way: name,
//! favicon, source link and logo come from the resolved configuration, never
//! from the request.

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::AppState;
use crate::branding::Branding;
use crate::config::{KNOWN_LANGUAGES, language_prefix};
use crate::legal::{LegalPages, Which, escape};

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
/// Where `legal.html` takes the page title (in `<title>`, the header and the
/// `<h1>`): the label of the page's own footer marker, so the title is the
/// catalogue's, in the shell's language.
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
/// Where a shell names the instance: `<title>`, the header wordmark and the
/// catalogue texts that name the service. Filled with the escaped
/// `branding.name`.
pub const NAME_TOKEN: &str = "%%capsulebin:name%%";
/// The `href` of the shell's `<link rel="icon">`.
pub const FAVICON_TOKEN: &str = "%%capsulebin:favicon%%";
/// The `href` of the attribution link: the escaped `branding.source_url`.
pub const SOURCE_TOKEN: &str = "%%capsulebin:source%%";
/// What every token above starts with.
const TOKEN_START: &str = "%%capsulebin:";
/// Around the built-in symbol in the header, replaced by the operator's logo.
pub const LOGO_REGION: (&str, &str) = ("<!--capsulebin:logo-->", "<!--/capsulebin:logo-->");
/// Around the name in the header, removed with `branding.hide_name`.
pub const WORDMARK_REGION: (&str, &str) = ("<!--capsulebin:wordmark-->", "<!--/capsulebin:wordmark-->");
/// The favicon of the web build, linked when none is configured.
const BUILT_IN_FAVICON: &str = "/favicon.svg";

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

/// Built frontend assets, embedded into the binary in every profile
/// (`debug-embed`): a debug build serves the web build it was compiled with,
/// like a release build, not whatever is in `web/dist` at the moment.
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

pub fn routes(state: &AppState) -> Router<AppState> {
    let default_language = state.default_language;
    let mut router = Router::new();
    for &language in KNOWN_LANGUAGES {
        let prefix = language_prefix(default_language, language);
        for (path, which) in PAGES {
            // The embedded assets cannot change while the server runs.
            let prepared = match *state.assets {
                AssetSource::Embedded => Some(Document::build(state, language, which)),
                AssetSource::Directory(_) => None,
            };
            router = router.route(
                &format!("{prefix}{path}"),
                get(move |State(state): State<AppState>| async move {
                    prepared.unwrap_or_else(|| Document::build(&state, language, which)).into_response()
                }),
            );
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

/// What a page route answers with: the same for every request, whatever the
/// path id.
#[derive(Clone)]
enum Document {
    Html(Bytes),
    /// A legal page that is not configured, or configured as a link elsewhere:
    /// the same `404` as any unknown path.
    NotFound,
    /// The shell file that is not in the build output.
    Missing(String),
}

impl Document {
    /// Puts a page in `language` together. The path id is deliberately **not**
    /// an input: reading it would be the first step towards a response that
    /// depends on whether it exists.
    ///
    /// A legal page is the `legal.html` shell with the page put in, titled
    /// with the label of its footer marker.
    fn build(state: &AppState, language: &str, which: Page) -> Self {
        let Some(legal) = state.legal.get(language) else {
            return Self::NotFound;
        };
        let (shell, legal_page) = match which {
            Page::Shell(shell) => (shell, None),
            Page::Legal(which) => match legal.page(which) {
                Some(page) => (Shell::Legal, Some((which, page))),
                None => return Self::NotFound,
            },
        };
        let file = shell.file_for(language);
        let Some(bytes) = state.assets.load(&file) else {
            return Self::Missing(file);
        };
        let shell = String::from_utf8_lossy(&bytes);
        let mut document = render(&shell, legal, language, state.default_language, &state.branding);
        // After the tokens, so that the operator's text is put in as it is.
        if let Some((which, page)) = legal_page {
            // Read before `render` replaced the footer markers; already escaped.
            let title = legal_marker(&shell, which).map_or("", |(_, label, _)| label);
            document = document
                .replace(LEGAL_TITLE_MARKER, title)
                .replace(LEGAL_CONTENT_MARKER, &page.html);
        }
        Self::Html(document.into())
    }
}

impl IntoResponse for Document {
    fn into_response(self) -> Response {
        match self {
            Self::Html(bytes) => html(bytes),
            Self::NotFound => not_found(),
            Self::Missing(file) => missing_frontend(&file),
        }
    }
}

/// Fills in what a shell of `language` leaves to the server: the logo and
/// wordmark regions of the header, the tokens (prefix of its internal links,
/// default language, name, favicon, source link) and the legal links. All
/// fixed at startup and the same for every request, so the shell does not
/// depend on the path id. The shells are UTF-8; a lossy conversion only
/// matters for a broken build, which `/health/ready` reports.
///
/// Operator values are never read as markup of the shell: the tokens are
/// filled in one pass, so a value is not searched for tokens again, and every
/// operator value is escaped, so it holds no `<` and cannot form a marker.
/// The legal links (and the legal page in [`Document::build`]) come after the
/// tokens, so their operator text is not searched for tokens either.
fn render(shell: &str, legal: &LegalPages, language: &str, default_language: &str, branding: &Branding) -> String {
    let shell = with_branding_regions(shell, branding);
    let favicon = branding.favicon.as_ref().map_or(BUILT_IN_FAVICON, |favicon| &favicon.url);
    let shell = fill_tokens(
        &shell,
        &[
            (PREFIX_TOKEN, &language_prefix(default_language, language)),
            (DEFAULT_TOKEN, default_language),
            (NAME_TOKEN, &escape(&branding.name)),
            (FAVICON_TOKEN, &escape(favicon)),
            (SOURCE_TOKEN, &escape(&branding.source_url)),
        ],
    );
    Which::ALL
        .into_iter()
        .fold(shell, |shell, which| with_legal_link(&shell, which, legal.footer.href(which)))
}

/// Replaces every token of `tokens` in `shell` with its value, in a single
/// pass: what a value brings in is not searched again. Text that only looks
/// like the start of a token stays as it is.
fn fill_tokens(shell: &str, tokens: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(shell.len());
    let mut rest = shell;
    while let Some(at) = rest.find(TOKEN_START) {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        match tokens.iter().find(|(token, _)| rest.starts_with(token)) {
            Some((token, value)) => {
                out.push_str(value);
                rest = &rest[token.len()..];
            }
            None => {
                out.push_str(TOKEN_START);
                rest = &rest[TOKEN_START.len()..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The header as `[branding]` wants it: the configured logo in place of the
/// built-in symbol, and no name next to it with `hide_name`. Without a logo the
/// symbol stays, and with the name the wordmark; only their markers go. The
/// images name the instance through [`NAME_TOKEN`], filled afterwards like
/// every other token; their URLs are derived from the files' hashes and hold
/// nothing to escape.
fn with_branding_regions(shell: &str, branding: &Branding) -> String {
    let alt = if branding.hide_name { NAME_TOKEN } else { "" };
    let image = |url: &str, class: &str| format!(r#"<img class="{class}" src="{url}" alt="{alt}">"#);
    let logo = branding.logo.as_ref().map(|logo| match &branding.logo_dark {
        None => image(&logo.url, "cb-logo-img"),
        Some(dark) => image(&logo.url, "cb-logo-img cb-logo-img-light") + &image(&dark.url, "cb-logo-img cb-logo-img-dark"),
    });
    let shell = replace_regions(shell, LOGO_REGION, logo.as_deref());
    replace_regions(&shell, WORDMARK_REGION, branding.hide_name.then_some(""))
}

/// Replaces every complete region from `start` to `end`, markers included and
/// whatever it spans, with `replacement`, or with the text between the markers
/// if there is none. A start without its end is left as it is.
fn replace_regions(shell: &str, (start, end): (&str, &str), replacement: Option<&str>) -> String {
    let mut out = String::with_capacity(shell.len());
    let mut rest = shell;
    while let Some(at) = rest.find(start) {
        let Some(length) = rest[at + start.len()..].find(end) else {
            break;
        };
        let inner = &rest[at + start.len()..at + start.len() + length];
        out.push_str(&rest[..at]);
        out.push_str(replacement.unwrap_or(inner));
        rest = &rest[at + start.len() + length + end.len()..];
    }
    out.push_str(rest);
    out
}

/// The first complete footer marker of `which` in `shell`: the text before it,
/// its label (as the web build wrote it, escaped) and the text after it.
fn legal_marker(shell: &str, which: Which) -> Option<(&str, &str, &str)> {
    let start = format!("{LEGAL_LINK_MARKER}{}:", which.path());
    let at = shell.find(&start)?;
    let after = &shell[at + start.len()..];
    let end = after.find("-->")?;
    Some((&shell[..at], &after[..end], &after[end + "-->".len()..]))
}

/// Replaces every footer marker of `which` with a link to `href` labelled as
/// the marker says, or removes it when the page is not configured. A marker
/// without its end is left as it is.
fn with_legal_link(shell: &str, which: Which, href: Option<&str>) -> String {
    let mut out = String::with_capacity(shell.len());
    let mut rest = shell;
    while let Some((before, label, after)) = legal_marker(rest, which) {
        out.push_str(before);
        if let Some(href) = href {
            // The label is already escaped by the web build.
            out.push_str(&format!(r#"<a href="{}">{label}</a>"#, escape(href)));
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// Static assets (JS, CSS, icons). No directory listing and no path traversal:
/// `rust-embed` only knows the embedded names, and the directory mode
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

fn html(bytes: Bytes) -> Response {
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
    use crate::branding::BrandingFile;
    use crate::legal::FooterLinks;

    /// `[branding]` as an instance without the table has it.
    fn default_branding() -> Branding {
        crate::branding::resolve(&crate::config::BrandingConfig::default()).unwrap()
    }

    fn image(url: &str) -> Option<BrandingFile> {
        Some(BrandingFile {
            url: url.to_owned(),
            content_type: "image/png",
            bytes: Vec::new(),
        })
    }

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
            render(shell, &legal, "de", "en", &default_branding()),
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
            assert_eq!(render(shell, &legal, "de", "en", &default_branding()), shell);
        }
    }

    #[test]
    fn the_tokens_follow_the_default_language() {
        let shell = r#"<meta data-default="%%capsulebin:default%%"><a href="%%capsulebin:prefix%%/"></a><a href="%%capsulebin:prefix%%/report"></a>"#;
        let branding = default_branding();
        let render = |language, default| render(shell, &LegalPages::default(), language, default, &branding);

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

    /// What a shell names the instance with, as the web build writes it: in
    /// the title, the header (logo and wordmark regions over several lines), a
    /// catalogue text, the favicon link and the attribution link.
    const BRANDED_SHELL: &str = "<title>%%capsulebin:name%%</title><link rel=\"icon\" href=\"%%capsulebin:favicon%%\">\
        <a class=\"cb-logo\" href=\"%%capsulebin:prefix%%/\">\n<!--capsulebin:logo-->\n<svg>\n</svg>\n<!--/capsulebin:logo-->\n\
        <!--capsulebin:wordmark-->\n<span class=\"cb-logo-name\">%%capsulebin:name%%</span>\n<!--/capsulebin:wordmark-->\n</a>\
        <noscript>%%capsulebin:name%% needs JavaScript.</noscript>\
        <footer><!--capsulebin:legal-link:imprint:Imprint--><a href=\"%%capsulebin:source%%\">Powered by CapsuleBin</a></footer>";

    fn render_branded(branding: &Branding) -> String {
        let legal = LegalPages {
            footer: FooterLinks {
                imprint: Some("/de/imprint".to_owned()),
                ..FooterLinks::default()
            },
            ..LegalPages::default()
        };
        render(BRANDED_SHELL, &legal, "de", "en", branding)
    }

    #[test]
    fn without_branding_the_shell_names_capsulebin_and_keeps_the_symbol() {
        assert_eq!(
            render_branded(&default_branding()),
            "<title>CapsuleBin</title><link rel=\"icon\" href=\"/favicon.svg\">\
             <a class=\"cb-logo\" href=\"/de/\">\n\n<svg>\n</svg>\n\n\n<span class=\"cb-logo-name\">CapsuleBin</span>\n\n</a>\
             <noscript>CapsuleBin needs JavaScript.</noscript>\
             <footer><a href=\"/de/imprint\">Imprint</a><a href=\"https://github.com/capsulebin/capsulebin\">Powered by CapsuleBin</a></footer>"
        );
    }

    /// The operator's values are put in escaped and are never read as a token
    /// or a marker, wherever they end up.
    #[test]
    fn operator_values_are_escaped_and_stay_literal() {
        let branding = Branding {
            name: r#"A&B <i>x</i> "q" %%capsulebin:prefix%% <!--capsulebin:legal-link:imprint:x--> %%capsulebin:name%%"#.to_owned(),
            hide_name: true,
            logo: image("/branding/0123456789abcdef.png"),
            source_url: "https://example.org/src?a=1&b=%%capsulebin:default%%".to_owned(),
            ..default_branding()
        };
        let name = "A&amp;B &lt;i&gt;x&lt;/i&gt; &quot;q&quot; %%capsulebin:prefix%% \
                    &lt;!--capsulebin:legal-link:imprint:x--&gt; %%capsulebin:name%%";
        assert_eq!(
            render_branded(&branding),
            format!(
                "<title>{name}</title><link rel=\"icon\" href=\"/favicon.svg\">\
                 <a class=\"cb-logo\" href=\"/de/\">\n<img class=\"cb-logo-img\" src=\"/branding/0123456789abcdef.png\" alt=\"{name}\">\n\n</a>\
                 <noscript>{name} needs JavaScript.</noscript>\
                 <footer><a href=\"/de/imprint\">Imprint</a>\
                 <a href=\"https://example.org/src?a=1&amp;b=%%capsulebin:default%%\">Powered by CapsuleBin</a></footer>"
            )
        );
    }

    #[test]
    fn a_logo_replaces_the_symbol_and_keeps_the_name() {
        let branding = Branding {
            logo: image("/branding/0123456789abcdef.png"),
            favicon: image("/branding/fedcba9876543210.ico"),
            ..default_branding()
        };
        let page = render_branded(&branding);
        assert!(
            page.contains("\n<img class=\"cb-logo-img\" src=\"/branding/0123456789abcdef.png\" alt=\"\">\n\n<span class=\"cb-logo-name\">CapsuleBin</span>\n\n</a>"),
            "{page}"
        );
        assert!(!page.contains("<svg>") && !page.contains(LOGO_REGION.0), "{page}");
        assert!(
            page.contains("<link rel=\"icon\" href=\"/branding/fedcba9876543210.ico\">"),
            "{page}"
        );
    }

    #[test]
    fn a_dark_logo_adds_a_second_image() {
        let branding = Branding {
            logo: image("/branding/0123456789abcdef.png"),
            logo_dark: image("/branding/fedcba9876543210.webp"),
            ..default_branding()
        };
        let page = render_branded(&branding);
        assert!(
            page.contains(
                "<img class=\"cb-logo-img cb-logo-img-light\" src=\"/branding/0123456789abcdef.png\" alt=\"\">\
                 <img class=\"cb-logo-img cb-logo-img-dark\" src=\"/branding/fedcba9876543210.webp\" alt=\"\">"
            ),
            "{page}"
        );
        assert_eq!(page.matches("<img").count(), 2, "{page}");
    }

    /// A broken build: a region without its end is left as it is.
    #[test]
    fn incomplete_regions_are_left_alone() {
        let branding = Branding {
            hide_name: true,
            logo: image("/branding/0123456789abcdef.png"),
            ..default_branding()
        };
        let shell = "<!--capsulebin:logo--><svg></svg><!--capsulebin:wordmark-->x %%capsulebin:unknown%%";
        assert_eq!(render(shell, &LegalPages::default(), "en", "en", &branding), shell);
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

    /// Every shell names the instance through the token in its title and
    /// header, links the favicon and the source code through their tokens, and
    /// has the logo and wordmark regions in its header, each complete once.
    #[test]
    fn every_shell_source_carries_the_branding_tokens_and_regions() {
        for shell in Shell::ALL {
            let (path, source) = template(shell);
            let path = path.display();
            assert!(
                source.contains(" · {{name}}</title>") || source.contains("<title>{{name}}</title>"),
                "{path}: <title>"
            );
            assert_eq!(
                source.matches(r#"<link rel="icon" href="{{favicon}}" />"#).count(),
                1,
                "{path}: favicon"
            );
            assert_eq!(
                source.matches(r#"<a href="{{source}}">{{shell.poweredBy}}</a>"#).count(),
                1,
                "{path}: attribution"
            );
            for (start, end) in [LOGO_REGION, WORDMARK_REGION] {
                assert_eq!(source.matches(start).count(), 1, "{path}: {start}");
                assert_eq!(source.matches(end).count(), 1, "{path}: {end}");
                let region = source
                    .split(start)
                    .nth(1)
                    .and_then(|rest| rest.split(end).next())
                    .unwrap_or_default();
                assert!(!region.is_empty(), "{path}: {start} before {end}");
            }
            let wordmark = source
                .split(WORDMARK_REGION.0)
                .nth(1)
                .and_then(|rest| rest.split(WORDMARK_REGION.1).next());
            assert_eq!(
                wordmark.map(str::trim),
                Some(r#"<span class="cb-logo-name">{{name}}</span>"#),
                "{path}: wordmark"
            );
            let logo = source.split(LOGO_REGION.0).nth(1).and_then(|rest| rest.split(LOGO_REGION.1).next());
            assert!(logo.is_some_and(|logo| logo.trim().starts_with("<svg")), "{path}: symbol");
        }
    }

    #[test]
    fn the_legal_shell_carries_its_title_and_content_markers() {
        let (_, source) = template(Shell::Legal);
        assert_eq!(source.matches(LEGAL_TITLE_MARKER).count(), 3, "<title>, header and <h1>");
        // In the shell, not in the content, whose `<article>` may carry another `lang`.
        assert_eq!(source.matches(&format!("<h1>{LEGAL_TITLE_MARKER}</h1>")).count(), 1, "<h1>");
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
