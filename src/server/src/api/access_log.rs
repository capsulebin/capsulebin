//! Access log with a redacted request path.
//!
//! The default `tower-http` trace layer records the full URI. That is unacceptable
//! here: `/api/v1/pastes/<id>` would put a paste id into a file whose retention is
//! longer than the paste's own, and a log archive would become a list of every
//! paste that ever existed.
//!
//! So the path is reduced to its shape before it is recorded:
//! `/api/v1/pastes/AQEB…/retrieve` becomes `/api/v1/pastes/{id}/retrieve`. The
//! endpoint stays visible, which is what an operator needs, and the identifier does
//! not.
//!
//! The reduction works from an **allowlist**: a segment is kept only if it is a
//! known-static part of a route. Anything else is replaced. That way a new route
//! carrying a secret in its path is redacted by default rather than logged until
//! someone notices.

use axum::extract::Request;
use tower_http::trace::{DefaultOnResponse, MakeSpan, TraceLayer};

/// Path segments that are part of the API surface and safe to record.
const STATIC_SEGMENTS: &[&str] = &[
    "api", "v1", "pastes", "retrieve", "describe", "reports", "health", "live", "ready", "manage", "report", "p", "assets", "problems",
    "branding",
];

/// Replaces every non-static path segment with `{id}`.
///
/// Static asset filenames are kept: they contain a dot, are chosen by the build
/// rather than by a client, and are useful when diagnosing a caching problem.
pub fn redact_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut position = 0;
    let mut below_branding = false;

    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }

        out.push('/');

        let branding_file = below_branding && is_branding_filename(segment);
        below_branding = position == 0 && segment == "branding";
        position += 1;

        if branding_file || STATIC_SEGMENTS.contains(&segment) || is_asset_filename(segment) {
            out.push_str(segment);
        } else {
            out.push_str("{id}");
        }
    }

    if out.is_empty() { "/".to_owned() } else { out }
}

/// A build-generated asset name such as `index-DxndYyue.js`.
///
/// Deliberately strict: a dot, a known extension, and no characters that would let
/// a crafted request write arbitrary text into the log.
fn is_asset_filename(segment: &str) -> bool {
    const EXTENSIONS: &[&str] = &["js", "mjs", "css", "html", "json", "svg", "png", "ico", "woff2", "txt", "map"];

    match segment.rsplit_once('.') {
        Some((stem, extension)) => {
            EXTENSIONS.contains(&extension) && !stem.is_empty() && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        }
        None => false,
    }
}

/// A configured branding file directly below `/branding/`, such as
/// `0123456789abcdef.webp`.
///
/// Matches only the shape `crate::branding` gives these URLs (16 lowercase hex
/// digits of a content hash and the extension of a branding file), so `webp` is
/// kept nowhere else: a paste id never has this shape, and every other segment
/// stays redacted as before.
fn is_branding_filename(segment: &str) -> bool {
    const EXTENSIONS: &[&str] = &["png", "webp", "ico", "css"];

    match segment.split_once('.') {
        Some((stem, extension)) => {
            EXTENSIONS.contains(&extension) && stem.len() == 16 && stem.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        }
        None => false,
    }
}

/// Builds the span for one request: method and redacted path, nothing else.
///
/// Explicitly absent: the query string, any header, and the body. The
/// authorization header would carry a bearer token, and a query string is
/// attacker-controlled free text.
#[derive(Debug, Clone, Copy, Default)]
pub struct RedactedSpan;

impl<B> MakeSpan<B> for RedactedSpan {
    fn make_span(&mut self, request: &Request<B>) -> tracing::Span {
        tracing::info_span!(
            "request",
            method = %request.method(),
            path = %redact_path(request.uri().path()),
        )
    }
}

/// The trace layer to install on the router.
pub fn layer() -> TraceLayer<tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>, RedactedSpan> {
    TraceLayer::new_for_http()
        .make_span_with(RedactedSpan)
        .on_response(DefaultOnResponse::new().level(tracing::Level::INFO))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reason this module exists.
    #[test]
    fn paste_ids_are_replaced() {
        assert_eq!(redact_path("/api/v1/pastes/AQEBAQEBAQEBAQEBAQEBAQ"), "/api/v1/pastes/{id}");
        assert_eq!(
            redact_path("/api/v1/pastes/AQEBAQEBAQEBAQEBAQEBAQ/retrieve"),
            "/api/v1/pastes/{id}/retrieve"
        );
        assert_eq!(redact_path("/p/AQEBAQEBAQEBAQEBAQEBAQ"), "/p/{id}");
        assert_eq!(redact_path("/manage/AQEBAQEBAQEBAQEBAQEBAQ"), "/manage/{id}");
    }

    #[test]
    fn static_routes_survive_unchanged() {
        for path in ["/api/v1/pastes", "/api/v1/reports", "/health/live", "/health/ready", "/report"] {
            assert_eq!(redact_path(path), path);
        }

        // The root has no segments and must not collapse to an empty string.
        assert_eq!(redact_path("/"), "/");
        assert_eq!(redact_path(""), "/");
    }

    #[test]
    fn asset_names_survive() {
        assert_eq!(redact_path("/assets/index-DxndYyue.js"), "/assets/index-DxndYyue.js");
        assert_eq!(redact_path("/theme-init.js"), "/theme-init.js");
        assert_eq!(redact_path("/assets/shell-CakMXSuP.css"), "/assets/shell-CakMXSuP.css");
    }

    /// A configured branding file is served at `/branding/<16 hex>.<ext>`; its URL
    /// is a content hash, not a secret, and useful when diagnosing caching.
    #[test]
    fn branding_files_survive() {
        for extension in ["png", "webp", "ico", "css"] {
            let path = format!("/branding/0123456789abcdef.{extension}");
            assert_eq!(redact_path(&path), path);
        }
    }

    /// Only the branding file shape is new; anything else below `/branding/` and
    /// `webp` anywhere else is redacted as before.
    #[test]
    fn other_branding_segments_are_redacted() {
        for (input, expected) in [
            ("/branding/AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEB", "/branding/{id}"),
            ("/branding", "/branding"),
            ("/branding/0123456789ABCDEF.webp", "/branding/{id}"),
            ("/branding/0123456789abcde.webp", "/branding/{id}"),
            ("/branding/0123456789abcdef0.webp", "/branding/{id}"),
            ("/branding/0123456789abcdef.svg.webp", "/branding/{id}"),
            ("/branding/0123456789abcdef.webp/x", "/branding/0123456789abcdef.webp/{id}"),
            ("/branding/x/0123456789abcdef.webp", "/branding/{id}/{id}"),
            ("/p/0123456789abcdef.webp", "/p/{id}"),
            ("/p/AQEBAQEBAQEBAQEBAQEBAQ.webp", "/p/{id}"),
            ("/assets/branding/0123456789abcdef.webp", "/assets/branding/{id}"),
        ] {
            assert_eq!(redact_path(input), expected, "for {input}");
        }
    }

    /// `/branding/colors.css` is not a branding file (no content hash), but it has
    /// the shape of a build asset name, so the general asset rule keeps it, as
    /// anywhere else in a path.
    #[test]
    fn asset_shaped_names_below_branding_survive() {
        assert_eq!(redact_path("/branding/colors.css"), "/branding/colors.css");
    }

    /// The allowlist has to fail closed: anything unexpected is redacted, so a
    /// crafted path cannot inject text into the log.
    #[test]
    fn unknown_and_hostile_segments_are_redacted() {
        for (input, expected) in [
            ("/wp-admin", "/{id}"),
            ("/api/v1/pastes/../../etc/passwd", "/api/v1/pastes/{id}/{id}/{id}/{id}"),
            ("/p/a%0aINJECTED", "/p/{id}"),
            ("/p/very-long-secret-looking-value", "/p/{id}"),
            ("/evil.sh", "/{id}"),
            ("/.env", "/{id}"),
        ] {
            assert_eq!(redact_path(input), expected, "for {input}");
        }
    }

    /// The query string never reaches the span, but the redactor is only given a
    /// path anyway. This pins that assumption.
    #[test]
    fn only_the_path_is_considered() {
        assert_eq!(redact_path("/health/live"), "/health/live");
    }
}
