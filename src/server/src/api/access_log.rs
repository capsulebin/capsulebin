//! Access log that records route templates, never request paths.
//!
//! The default `tower-http` trace layer records the full URI. That is unacceptable
//! here: `/api/v1/pastes/<id>` would put a paste id into a file whose retention is
//! longer than the paste's own, and a log archive would become a list of every
//! paste that ever existed.
//!
//! So the span's `path` is never taken from the request. It is one of:
//!
//! - the template of the matched route (`MatchedPath`), e.g.
//!   `/api/v1/pastes/{id}/retrieve` or `/de/p/{id}`: chosen by the code, so no
//!   request can put text into it, and the endpoint stays visible, which is what
//!   an operator needs;
//! - for a request that falls through to the asset fallback, the path of the file
//!   it served ([`ServedAsset`]), which the build or the operator chose;
//! - otherwise the fixed label [`UNMATCHED`], with nothing from the request.
//!
//! [`record_path`] runs after routing, so it sees `MatchedPath`, and wraps every
//! layer that may reject a request (limiter, body limits), so those rejections
//! carry the template too.

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use tower_http::trace::{DefaultOnResponse, MakeSpan, TraceLayer};

/// The `path` of every request that matched no route and served no asset.
pub const UNMATCHED: &str = "{unmatched}";

/// The path of an asset the fallback served, set by it as a response extension
/// only after the file was found in the asset source.
#[derive(Debug, Clone)]
pub(crate) struct ServedAsset(pub(crate) String);

/// Builds the span for one request: the method and an empty `path`, filled by
/// [`record_path`]. Nothing else.
///
/// Explicitly absent: the request path, the query string, any header, and the
/// body. The authorization header would carry a bearer token, and path and
/// query are attacker-controlled free text.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestSpan;

impl<B> MakeSpan<B> for RequestSpan {
    fn make_span(&mut self, request: &Request<B>) -> tracing::Span {
        tracing::info_span!("request", method = %request.method(), path = tracing::field::Empty)
    }
}

/// The trace layer to install on the router, outside [`record_path`].
pub fn layer() -> TraceLayer<tower_http::classify::SharedClassifier<tower_http::classify::ServerErrorsAsFailures>, RequestSpan> {
    TraceLayer::new_for_http()
        .make_span_with(RequestSpan)
        .on_response(DefaultOnResponse::new().level(tracing::Level::INFO))
}

/// Records the span's `path`, exactly once: the route template before the
/// request is handled, so that everything logged while handling it carries it;
/// without a matched route, after the answer, the served asset or
/// [`UNMATCHED`].
pub async fn record_path(request: Request, next: Next) -> Response {
    // `display`, so the value is logged as it is, not quoted as a string.
    let record = |path: &str| {
        tracing::Span::current().record("path", tracing::field::display(path));
    };
    if let Some(template) = request.extensions().get::<MatchedPath>() {
        record(template.as_str());
        return next.run(request).await;
    }

    let response = next.run(request).await;
    match response.extensions().get::<ServedAsset>() {
        Some(ServedAsset(path)) => record(path),
        None => record(UNMATCHED),
    }
    response
}
