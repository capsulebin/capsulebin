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

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/pastes", post(handlers::create))
        .route("/api/v1/pastes/{id}", delete(handlers::remove))
        .route("/api/v1/pastes/{id}/describe", post(handlers::descriptor))
        .route("/api/v1/pastes/{id}/retrieve", post(handlers::retrieve))
}
