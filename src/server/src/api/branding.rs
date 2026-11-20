//! Serving the operator's branding files (`[branding]`, see [`crate::branding`]).
//!
//! Each configured file, and the stylesheet generated from the configured
//! colours, gets exactly one route, at the URL derived from its content; the
//! bytes were read or generated at startup and are served from memory, in
//! both asset modes. Every other path under `/branding/` has no route and
//! falls through to the asset fallback, which answers the same generic `404`
//! as any unknown path.

use axum::{
    Router,
    body::Bytes,
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};

use crate::AppState;

/// The URL changes with the content, so a file never changes under its URL.
const CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

pub fn routes(state: &AppState) -> Router<AppState> {
    let mut router = Router::new();
    for file in state.branding.files() {
        let body = Bytes::from(file.bytes.clone());
        let content_type = file.content_type;
        router = router.route(&file.url, get(move || async move { serve(body, content_type) }));
    }
    router
}

fn serve(body: Bytes, content_type: &'static str) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE_CONTROL));
    response
}
