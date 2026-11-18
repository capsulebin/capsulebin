//! `application/problem+json` error responses (RFC 9457).
//!
//! Three rules this module enforces, all of them security-relevant:
//!
//! 1. **Stable machine-readable codes.** Clients branch on `code`, never on
//!    `title`, so the wording can change without breaking them.
//! 2. **No secrets, ever.** No token, no ciphertext, no password, no full paste
//!    id. The `Display` text of every variant is a fixed string.
//! 3. **Unavailability is indistinguishable.** Missing, expired, disabled and
//!    deleted all map to the *same* response. A caller cannot learn whether an id
//!    exists, which is what keeps ids unguessable in practice as well as in
//!    theory.
//!
//! The mapping from variant to status and code is one exhaustive `match` with no
//! catch-all arm: adding a variant fails to compile until someone decides how it
//! should surface.

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::repository::RepositoryError;

/// Everything the API can refuse to do.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The one generic answer for "you cannot have this". Covers unknown,
    /// expired, moderated and deleted pastes, and a capability that does not
    /// match. Splitting it would leak existence.
    #[error("not available")]
    NotAvailable,

    /// A burn paste that was already handed out. Distinguishable from
    /// [`Self::NotAvailable`] only because the creator explicitly opted into burn
    /// semantics, so the fact of a prior retrieval is information they chose to
    /// expose to whoever holds the link.
    #[error("already retrieved")]
    AlreadyRetrieved,

    #[error("malformed request")]
    BadRequest(&'static str),

    #[error("paste id is not in the expected format")]
    InvalidId,

    #[error("payload too large")]
    PayloadTooLarge,

    #[error("requested expiry exceeds the configured maximum")]
    ExpiryTooLong,

    #[error("pastes without an expiry are not allowed")]
    PermanentNotAllowed,

    /// Creation requires a bearer token and none, or a wrong one, was supplied.
    #[error("creation requires a valid token")]
    Unauthorized,

    /// Creation is switched off entirely on this instance.
    #[error("creation is disabled on this instance")]
    CreationDisabled,

    /// The client-chosen id is taken. The client retries with a fresh one.
    #[error("paste id already exists")]
    IdConflict,

    /// The operator switched abuse reports off (`abuse.enabled = false`).
    #[error("reports are not accepted on this instance")]
    ReportsDisabled,

    /// A rate limit or quota refused the request before anything was looked up.
    #[error("too many requests")]
    RateLimited,

    /// The create byte quota needs a declared `Content-Length`.
    #[error("a content length is required")]
    LengthRequired,

    #[error("internal error")]
    Internal,
}

/// Stable code and HTTP status for each variant.
///
/// Deliberately exhaustive: no `_ =>` arm. A new variant must be classified here
/// before it can be returned.
impl ApiError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::NotAvailable => StatusCode::NOT_FOUND,
            Self::AlreadyRetrieved => StatusCode::GONE,
            Self::BadRequest(_) | Self::InvalidId => StatusCode::BAD_REQUEST,
            Self::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::ExpiryTooLong | Self::PermanentNotAllowed => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::CreationDisabled => StatusCode::FORBIDDEN,
            Self::IdConflict => StatusCode::CONFLICT,
            Self::ReportsDisabled => StatusCode::NOT_FOUND,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            Self::LengthRequired => StatusCode::LENGTH_REQUIRED,
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The value clients are expected to branch on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAvailable => "not_available",
            Self::AlreadyRetrieved => "already_retrieved",
            Self::BadRequest(_) => "malformed_request",
            Self::InvalidId => "invalid_id",
            Self::PayloadTooLarge => "payload_too_large",
            Self::ExpiryTooLong => "expiry_too_long",
            Self::PermanentNotAllowed => "permanent_not_allowed",
            Self::Unauthorized => "unauthorized",
            Self::CreationDisabled => "creation_disabled",
            Self::IdConflict => "id_conflict",
            Self::ReportsDisabled => "reports_disabled",
            Self::RateLimited => "rate_limited",
            Self::LengthRequired => "length_required",
            Self::Internal => "internal_error",
        }
    }

    /// Human-readable summary. Fixed strings only — never interpolated with
    /// request data.
    pub fn title(&self) -> &'static str {
        match self {
            Self::NotAvailable => "Not available",
            Self::AlreadyRetrieved => "Already retrieved",
            Self::BadRequest(_) => "Malformed request",
            Self::InvalidId => "Invalid paste id",
            Self::PayloadTooLarge => "Payload too large",
            Self::ExpiryTooLong => "Expiry too long",
            Self::PermanentNotAllowed => "Permanent pastes not allowed",
            Self::Unauthorized => "Unauthorized",
            Self::CreationDisabled => "Creation disabled",
            Self::IdConflict => "Paste id already exists",
            Self::ReportsDisabled => "Reports disabled",
            Self::RateLimited => "Too many requests",
            Self::LengthRequired => "Length required",
            Self::Internal => "Internal error",
        }
    }

    /// Additional detail, only where it cannot reveal anything.
    ///
    /// [`Self::BadRequest`] carries a `&'static str` rather than a `String` for
    /// exactly this reason: a static string cannot contain request data.
    pub fn detail(&self) -> Option<&'static str> {
        match self {
            Self::BadRequest(detail) => Some(detail),
            _ => None,
        }
    }
}

/// A repository failure must never surface as itself: the message could name a
/// column or a file path. Only the conflict is meaningful to a client.
impl From<RepositoryError> for ApiError {
    fn from(error: RepositoryError) -> Self {
        match error {
            RepositoryError::IdConflict => Self::IdConflict,
            RepositoryError::Database(source) => {
                // Logged once, here, at the boundary - with the cause, which the
                // response will not carry.
                tracing::error!(error = %source, "repository failure");
                Self::Internal
            }
        }
    }
}

/// The wire format.
#[derive(Debug, Serialize)]
struct ProblemBody {
    /// Absolute URI identifying the problem type.
    #[serde(rename = "type")]
    problem_type: String,
    title: &'static str,
    status: u16,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'static str>,
    request_id: String,
}

/// An [`ApiError`] plus the context needed to render it.
///
/// Handlers return `ApiError`; the layer below turns it into this. Keeping them
/// separate means a handler cannot forget the request id, because it never
/// constructs the response.
pub struct Problem {
    pub error: ApiError,
    pub request_id: String,
    pub base_url: String,
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let status = self.error.status();

        let body = ProblemBody {
            problem_type: format!("{}/problems/{}", self.base_url, self.error.code()),
            title: self.error.title(),
            status: status.as_u16(),
            code: self.error.code(),
            detail: self.error.detail(),
            request_id: self.request_id,
        };

        // Serialisation of this struct cannot fail; if it somehow did, an empty
        // body with the right status is still a correct-enough answer and better
        // than a panic inside a handler.
        let json = serde_json::to_vec(&body).unwrap_or_default();

        let mut response = (status, json).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

/// Turns the body-limit layer's bare `413` into the same `problem+json` every
/// other error uses.
///
/// `RequestBodyLimitLayer` rejects a request whose declared `Content-Length`
/// exceeds the limit before any handler runs, with a plain-text body. Real
/// clients always declare the length, so without this they would never see
/// `payload_too_large`. Use with `from_fn_with_state(public_url, …)`, outside the
/// body limit.
pub async fn body_limit_problem(
    axum::extract::State(base_url): axum::extract::State<String>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let request_id = request
        .extensions()
        .get::<crate::api::request_id::RequestId>()
        .map(|id| id.0.clone())
        .unwrap_or_default();

    let response = next.run(request).await;

    let is_problem = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes().starts_with(b"application/problem+json"));
    if response.status() == StatusCode::PAYLOAD_TOO_LARGE && !is_problem {
        return Problem {
            error: ApiError::PayloadTooLarge,
            request_id,
            base_url,
        }
        .into_response();
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant, so that a newly added one shows up here as a missing entry
    /// rather than being forgotten.
    fn all() -> Vec<ApiError> {
        vec![
            ApiError::NotAvailable,
            ApiError::AlreadyRetrieved,
            ApiError::BadRequest("expected a JSON object"),
            ApiError::InvalidId,
            ApiError::PayloadTooLarge,
            ApiError::ExpiryTooLong,
            ApiError::PermanentNotAllowed,
            ApiError::Unauthorized,
            ApiError::CreationDisabled,
            ApiError::IdConflict,
            ApiError::ReportsDisabled,
            ApiError::RateLimited,
            ApiError::LengthRequired,
            ApiError::Internal,
        ]
    }

    #[test]
    fn codes_are_unique_and_snake_case() {
        let mut seen = std::collections::BTreeSet::new();

        for error in all() {
            let code = error.code();
            assert!(seen.insert(code), "duplicate code {code}");
            assert!(code.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{code} is not snake_case");
        }
    }

    /// The messages end up in logs, so none may carry a value that came from a
    /// request.
    ///
    /// The structural guarantee is the type: `detail` returns `&'static str` and so
    /// *cannot* hold runtime data — that is why [`ApiError::BadRequest`] takes a
    /// `&'static str` rather than a `String`. This test guards the remaining risk,
    /// which is a new variant interpolating a value through `Display`.
    #[test]
    fn no_variant_carries_request_data() {
        // Values a request might plausibly supply. None may surface.
        let secrets = [
            "AQEBAQEBAQEBAQEBAQEBAQ",
            "test-creation-token",
            "reveal-token-bytes",
            "PLAINTEXT-MARKER",
        ];

        for error in all() {
            let rendered = format!("{error}|{}|{}", error.title(), error.detail().unwrap_or(""));

            for secret in secrets {
                assert!(!rendered.contains(secret), "{rendered:?} contains the request value {secret:?}");
            }

            // A brace in a rendered message is a sign that a format argument slipped
            // in, which is how request data would get here.
            assert!(
                !rendered.contains('{') && !rendered.contains('}'),
                "{rendered:?} looks like it interpolates a value"
            );
        }
    }

    /// The core indistinguishability property. If this test ever needs changing,
    /// the change is a security decision, not a refactor.
    #[test]
    fn unavailability_has_exactly_one_representation() {
        let problem = ApiError::NotAvailable;
        assert_eq!(problem.status(), StatusCode::NOT_FOUND);
        assert_eq!(problem.code(), "not_available");
        assert_eq!(problem.detail(), None, "detail could distinguish causes");
    }

    #[test]
    fn statuses_match_their_meaning() {
        assert_eq!(ApiError::NotAvailable.status(), StatusCode::NOT_FOUND);
        assert_eq!(ApiError::AlreadyRetrieved.status(), StatusCode::GONE);
        assert_eq!(ApiError::PayloadTooLarge.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(ApiError::Unauthorized.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(ApiError::CreationDisabled.status(), StatusCode::FORBIDDEN);
        assert_eq!(ApiError::IdConflict.status(), StatusCode::CONFLICT);
        assert_eq!(ApiError::Internal.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn database_errors_become_internal_and_lose_their_message() {
        let error = ApiError::from(RepositoryError::Database(sqlx::Error::RowNotFound));

        assert!(matches!(error, ApiError::Internal));
        assert_eq!(error.detail(), None);
        assert!(!format!("{error}").contains("RowNotFound"));
    }

    #[test]
    fn conflicts_survive_the_conversion() {
        let error = ApiError::from(RepositoryError::IdConflict);
        assert!(matches!(error, ApiError::IdConflict));
    }

    #[tokio::test]
    async fn renders_the_documented_shape() {
        let problem = Problem {
            error: ApiError::PayloadTooLarge,
            request_id: "01ABCDEF".to_owned(),
            base_url: "https://paste.example".to_owned(),
        };

        let response = problem.into_response();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok()),
            Some("application/problem+json")
        );

        let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await.expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");

        assert_eq!(json["type"], "https://paste.example/problems/payload_too_large");
        assert_eq!(json["title"], "Payload too large");
        assert_eq!(json["status"], 413);
        assert_eq!(json["code"], "payload_too_large");
        assert_eq!(json["request_id"], "01ABCDEF");
    }

    /// `detail` is omitted rather than sent as null, so a client can rely on its
    /// presence meaning something.
    #[tokio::test]
    async fn detail_is_absent_when_there_is_none() {
        let problem = Problem {
            error: ApiError::NotAvailable,
            request_id: "01ABCDEF".to_owned(),
            base_url: "https://paste.example".to_owned(),
        };

        let body = axum::body::to_bytes(problem.into_response().into_body(), 64 * 1024)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("valid JSON");

        assert!(json.get("detail").is_none(), "detail should be omitted");
    }
}
