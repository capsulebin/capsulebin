//! The opaque paste API.
//!
//! Every handler here obeys two rules that shape the code more than anything
//! else:
//!
//! **The server never sees plaintext.** Bodies carry base64url of ciphertext and
//! of an opaque public header. Nothing is decoded beyond base64, nothing is
//! inspected, nothing is logged.
//!
//! **A response must not reveal whether an id exists.** Unknown, expired,
//! moderated, deleted and wrong-capability all produce the identical
//! [`ApiError::NotAvailable`]. The capability check therefore runs even when the
//! paste is already known to be unavailable — see [`retrieve`] — so that timing
//! does not become the oracle the status code is not.
//!
//! [`ApiError::NotAvailable`]: crate::api::problem::ApiError::NotAvailable
//! [`retrieve`]: handlers::retrieve

pub(crate) mod codec;
mod handlers;
mod wire;

use axum::Router;
use axum::routing::{delete, post};
use tower_http::limit::RequestBodyLimitLayer;

use crate::AppState;

/// Body limit of describe, retrieve and delete. Their bodies are one or two
/// base64url tokens of a few dozen characters; 4 KiB leaves room for any JSON
/// formatting without letting a proof request carry an upload-sized body.
/// Create keeps the global limit (`max_body_bytes`).
const PROOF_BODY_LIMIT: usize = 4 * 1024;

pub fn routes() -> Router<AppState> {
    // Inside the global limit layer of `router()`, so the smaller one wins;
    // `body_limit_problem` sits outside both and turns either `413` into
    // `problem+json`.
    let proof_limit = || RequestBodyLimitLayer::new(PROOF_BODY_LIMIT);

    Router::new()
        .route("/api/v1/pastes", post(handlers::create))
        .route("/api/v1/pastes/{id}", delete(handlers::remove).layer(proof_limit()))
        .route("/api/v1/pastes/{id}/describe", post(handlers::descriptor).layer(proof_limit()))
        .route("/api/v1/pastes/{id}/retrieve", post(handlers::retrieve).layer(proof_limit()))
}
