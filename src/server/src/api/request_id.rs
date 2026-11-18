//! Request identifiers.
//!
//! Every response carries one, including error responses, so that an operator can
//! correlate a user's report with a log line without the user having to send
//! anything sensitive. That is the point: a request id is safe to paste into a
//! support ticket, unlike a paste id or a URL.
//!
//! ULIDs rather than UUIDv4 because they sort by creation time, which makes a log
//! grep for "around that id" useful.

use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;

/// Header the id is echoed in.
pub const HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// A generated request id, put into the request extensions so handlers can read
/// it without regenerating one.
#[derive(Debug, Clone)]
pub struct RequestId(pub String);

impl RequestId {
    fn generate() -> Self {
        Self(ulid::Ulid::generate().to_string())
    }
}

/// Assigns an id and echoes it on the response.
///
/// A client-supplied `X-Request-Id` is deliberately **not** trusted: it would let
/// a caller choose what appears in the logs, including forged ids that collide
/// with someone else's request or that contain log-injection payloads.
pub async fn middleware(mut request: Request, next: Next) -> Response {
    let id = RequestId::generate();
    request.extensions_mut().insert(id.clone());

    let mut response = next.run(request).await;

    // A ULID is 26 characters of Crockford base32, so this cannot fail; if it
    // ever did, omitting the header beats panicking inside the middleware.
    if let Ok(value) = HeaderValue::from_str(&id.0) {
        response.headers_mut().insert(HEADER, value);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::{Router, extract::Extension};
    use tower::ServiceExt as _;

    fn app() -> Router {
        Router::new()
            .route("/", get(|Extension(id): Extension<RequestId>| async move { id.0 }))
            .layer(axum::middleware::from_fn(middleware))
    }

    async fn call(app: Router, header: Option<&str>) -> (StatusCode, Option<String>, String) {
        let mut builder = axum::http::Request::builder().uri("/");
        if let Some(value) = header {
            builder = builder.header("x-request-id", value);
        }

        let response = app.oneshot(builder.body(Body::empty()).expect("request")).await.expect("response");

        let status = response.status();
        let echoed = response
            .headers()
            .get(HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = axum::body::to_bytes(response.into_body(), 4096).await.expect("body");

        (status, echoed, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn every_response_carries_an_id_that_the_handler_also_sees() {
        let (status, echoed, body) = call(app(), None).await;

        assert_eq!(status, StatusCode::OK);
        let echoed = echoed.expect("header present");
        assert_eq!(echoed.len(), 26, "expected a ULID");
        assert_eq!(echoed, body, "handler and header must agree");
    }

    #[tokio::test]
    async fn ids_differ_between_requests() {
        let (_, first, _) = call(app(), None).await;
        let (_, second, _) = call(app(), None).await;

        assert_ne!(first, second);
    }

    /// A caller must not be able to choose its own id: that would let it forge log
    /// entries or make two unrelated requests share an identifier.
    ///
    /// Control characters need no test — `http` refuses to build a header value
    /// containing them, so log injection through this header is blocked one layer
    /// lower. What is testable, and what matters here, is that a *well-formed*
    /// client-supplied value is still discarded.
    #[tokio::test]
    async fn a_client_supplied_id_is_ignored() {
        let forged = "attacker-chosen-identifier";
        let (_, echoed, body) = call(app(), Some(forged)).await;

        let echoed = echoed.expect("header present");
        assert_ne!(echoed, forged, "the client's value was echoed back");
        assert_eq!(echoed.len(), 26);
        assert!(echoed.chars().all(|c| c.is_ascii_alphanumeric()));
        // The handler sees the generated id too, not the forged one.
        assert_eq!(body, echoed);
    }

    /// ULIDs sort by time, which is the reason for choosing them over UUIDv4.
    ///
    /// The guarantee is at millisecond granularity: the first ten characters encode
    /// a 48-bit timestamp, the remaining sixteen are random. Two ids generated
    /// within the same millisecond are therefore *not* ordered relative to each
    /// other, and asserting that they were would make this test flaky — it would
    /// pass only when the two calls happened to straddle a millisecond boundary.
    ///
    /// Strict monotonicity is available from a stateful generator, and deliberately
    /// not used: it would put a mutex on the path of every request to buy an
    /// ordering that log correlation does not need.
    #[tokio::test]
    async fn ids_sort_by_creation_time() {
        let (_, first, _) = call(app(), None).await;
        let (_, second, _) = call(app(), None).await;

        let (first, second) = (first.expect("first"), second.expect("second"));

        let (first_time, second_time) = (&first[..10], &second[..10]);
        assert!(
            first_time <= second_time,
            "timestamp component went backwards: {first} then {second}"
        );

        // The random component makes the full ids differ even inside one millisecond.
        assert_ne!(first, second);
    }
}
