//! HTTP security headers.
//!
//! These headers are part of the security boundary, not cosmetics. They are in
//! place on the empty application already, so that no later feature can ship
//! without them by accident.

use axum::http::{HeaderName, HeaderValue, header};

/// Content Security Policy.
///
/// `default-src 'none'` as the base, then only what is explicitly needed.
/// Deliberate choices:
///
/// - No `'unsafe-inline'` for `script-src`/`style-src`, which makes inline
///   scripts and inline styles impossible. The theme toggle and every state
///   change therefore work through classes and `data-*` attributes.
/// - `img-src 'self' data:` allows locally generated previews but no third
///   party resources. Markdown images from foreign origins are blocked at the
///   transport layer, independently of how the sanitizer is configured.
/// - `require-trusted-types-for 'script'` keeps rendered Markdown from turning
///   into script execution via `innerHTML`.
/// - `frame-ancestors 'none'` rather than relying on `X-Frame-Options`, which
///   is no longer honoured everywhere.
const CONTENT_SECURITY_POLICY: &str = "default-src 'none'; \
script-src 'self'; \
style-src 'self'; \
connect-src 'self'; \
img-src 'self' data:; \
font-src 'self'; \
object-src 'none'; \
base-uri 'none'; \
form-action 'self'; \
frame-ancestors 'none'; \
require-trusted-types-for 'script'";

/// Send no referrer. Keeps navigation out of the application from leaking the
/// paste URL - and with it, potentially, the fragment - to third parties.
const REFERRER_POLICY: &str = "no-referrer";

/// Search engines should neither index nor archive pastes.
const ROBOTS_TAG: &str = "noindex, nofollow, noarchive";

const PERMISSIONS_POLICY: &str = "camera=(), microphone=(), geolocation=()";

/// All headers as pairs, so that the router and the tests draw from one source.
/// A test that restates its own expectation would be edited together with the
/// bug it is supposed to catch.
pub fn headers() -> Vec<(HeaderName, HeaderValue)> {
    // Every value is a constant from this module and therefore a valid header
    // value; `from_static` verifies that without an error path.
    vec![
        (header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CONTENT_SECURITY_POLICY)),
        (header::REFERRER_POLICY, HeaderValue::from_static(REFERRER_POLICY)),
        (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
        (
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static(PERMISSIONS_POLICY),
        ),
        (HeaderName::from_static("x-robots-tag"), HeaderValue::from_static(ROBOTS_TAG)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CSP is the single most important line in the project. This test
    /// spells out what must not get lost from it.
    #[test]
    fn csp_allows_no_third_party_and_no_inline_code() {
        let csp = CONTENT_SECURITY_POLICY;

        assert!(csp.starts_with("default-src 'none'"));
        assert!(csp.contains("require-trusted-types-for 'script'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(csp.contains("base-uri 'none'"));
        assert!(csp.contains("object-src 'none'"));

        // Inline code and third party origins are ruled out.
        assert!(!csp.contains("unsafe-inline"));
        assert!(!csp.contains("unsafe-eval"));
        assert!(!csp.contains("http://"));
        assert!(!csp.contains("https://"));
        assert!(!csp.contains('*'));
    }

    #[test]
    fn every_header_is_present() {
        let names: Vec<String> = headers().into_iter().map(|(name, _)| name.as_str().to_owned()).collect();

        for expected in [
            "content-security-policy",
            "referrer-policy",
            "x-content-type-options",
            "x-frame-options",
            "permissions-policy",
            "x-robots-tag",
        ] {
            assert!(names.contains(&expected.to_owned()), "{expected} missing");
        }
    }
}
