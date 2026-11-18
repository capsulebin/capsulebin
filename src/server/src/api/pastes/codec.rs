//! Base64url encoding and JSON-rejection classification shared by every
//! handler.

use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::api::problem::ApiError;

/// Decodes a base64url field, rejecting it if the *encoded* text is longer than
/// `limit`.
///
/// The bound is on the encoded form because that is what has already been read
/// into memory; a limit on the decoded size cannot prevent the allocation it is
/// meant to prevent. Callers that care about the decoded size check it separately,
/// with [`encoded_len`] converting between the two.
pub(super) fn decode(field: &'static str, text: &str, limit: usize) -> Result<Vec<u8>, ApiError> {
    if text.len() > limit {
        return Err(ApiError::PayloadTooLarge);
    }
    URL_SAFE_NO_PAD.decode(text.as_bytes()).map_err(|_| ApiError::BadRequest(field))
}

/// Length of `bytes` bytes in unpadded base64url: four characters per three bytes,
/// rounded up.
pub(super) fn encoded_len(bytes: usize) -> usize {
    bytes.div_ceil(3).saturating_mul(4)
}

pub(super) fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Classifies a rejected JSON body.
///
/// The body limit layer sits below the extractor, so an oversized request surfaces
/// here as a rejection rather than as a size error. Mapping every rejection to
/// "malformed" would report 400 for a request that is well-formed but too large,
/// which tells the client to fix the wrong thing. The rejection's own status is
/// the reliable signal - matching on variant names would break silently when axum
/// adds one.
pub(crate) fn from_json_rejection(rejection: JsonRejection) -> ApiError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::PayloadTooLarge
    } else {
        ApiError::BadRequest("expected a JSON object")
    }
}
