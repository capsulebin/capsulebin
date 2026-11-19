//! Serving the client application.
//!
//! The client is a multi-page application with no client-side router, so every
//! route maps to exactly one HTML document.
//!
//! Key property: `GET /p/{id}` returns the same bytes for **every**
//! syntactically valid id and triggers no retrieval. A link scanner following
//! the URL therefore neither burns a one-time paste nor learns from the
//! response whether the id exists.

use axum::{
    Router,
    http::{HeaderValue, StatusCode, Uri, header},
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
    fn file(self) -> &'static str {
        match self {
            Self::Create => "index.html",
            Self::View => "p.html",
            Self::Manage => "manage.html",
            Self::Report => "report.html",
            Self::Legal => "legal.html",
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(|s| shell(s, Shell::Create)))
        .route("/p/{id}", get(|s| shell(s, Shell::View)))
        .route("/manage/{id}", get(|s| shell(s, Shell::Manage)))
        .route("/report", get(|s| shell(s, Shell::Report)))
        .route("/imprint", get(|s| legal(s, Which::Imprint)))
        .route("/privacy", get(|s| legal(s, Which::Privacy)))
        .route("/terms", get(|s| legal(s, Which::Terms)))
        .fallback(get(static_file))
}

/// Serves an HTML shell. The path id is deliberately **not** inspected: reading
/// it would be the first step towards a response that depends on whether it
/// exists.
async fn shell(axum::extract::State(state): axum::extract::State<AppState>, which: Shell) -> Response {
    match state.assets.load(which.file()) {
        Some(bytes) => html(with_legal_links(bytes, &state.legal).into_bytes()),
        None => missing_frontend(which.file()),
    }
}

/// A legal page: the `legal.html` shell with the page put in. Not configured,
/// or configured as a link elsewhere: the same `404` as any unknown path.
async fn legal(axum::extract::State(state): axum::extract::State<AppState>, which: Which) -> Response {
    let Some(page) = state.legal.page(which) else {
        return not_found();
    };
    match state.assets.load(Shell::Legal.file()) {
        Some(bytes) => html(
            with_legal_links(bytes, &state.legal)
                .replace(LEGAL_TITLE_MARKER, &crate::legal::escape(page.title))
                .replace(LEGAL_CONTENT_MARKER, &page.html)
                .into_bytes(),
        ),
        None => missing_frontend(Shell::Legal.file()),
    }
}

/// Puts the legal links into a shell. The same for every request, so the shell
/// still does not depend on the path id. The shells are UTF-8; a lossy
/// conversion only matters for a broken build, which `/health/ready` reports.
fn with_legal_links(bytes: Vec<u8>, legal: &LegalPages) -> String {
    String::from_utf8_lossy(&bytes).replace(LEGAL_LINKS_MARKER, &legal.footer)
}

/// Static assets (JS, CSS, icons). No directory listing and no path traversal:
/// `rust-embed` only knows the embedded names, and the development mode
/// validates the path.
async fn static_file(axum::extract::State(state): axum::extract::State<AppState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

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
    fn the_footer_marker_is_replaced() {
        let legal = LegalPages {
            footer: "<a>x</a>".to_owned(),
            ..LegalPages::default()
        };
        let shell = format!("<footer>{LEGAL_LINKS_MARKER}</footer>").into_bytes();
        assert_eq!(with_legal_links(shell, &legal), "<footer><a>x</a></footer>");
    }

    /// The substitution only works if every shell has the marker in its footer.
    #[test]
    fn every_shell_source_carries_the_legal_links_marker() {
        for shell in [Shell::Create, Shell::View, Shell::Manage, Shell::Report, Shell::Legal] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web").join(shell.file());
            let source = std::fs::read_to_string(&path).unwrap_or_default();
            assert_eq!(
                source.matches(LEGAL_LINKS_MARKER).count(),
                1,
                "{} must contain the legal links marker exactly once",
                path.display()
            );
        }
    }

    #[test]
    fn the_legal_shell_carries_its_title_and_content_markers() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/legal.html");
        let source = std::fs::read_to_string(&path).unwrap_or_default();
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
