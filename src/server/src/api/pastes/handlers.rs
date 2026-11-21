//! The four paste handlers: create, descriptor, retrieve, remove.

use crate::domain::{PasteId, Verifier};
use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest as _, Path, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};

use super::codec::{decode, encode, encoded_len, from_json_rejection};
use super::wire::{CreateRequest, CreateResponse, DeleteRequest, DescribeRequest, DescriptorResponse, RetrieveRequest, RetrieveResponse};
use crate::AppState;
use crate::api::problem::{ApiError, Problem};
use crate::api::rate_limit::with_retry_after;
use crate::api::request_id::RequestId;
use crate::config::CreationMode;
use crate::domain::NewPaste;
use crate::repository::{PasteRepository as _, RepositoryError};

/// Checks the creation capability.
///
/// In `token` mode the bearer is hashed and compared against the configured
/// hashes in constant time. The comparison happens even when the header is
/// missing, so that a present-but-wrong token and an absent one take the same
/// path.
fn authorize_creation(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    match state.creation_mode {
        CreationMode::Disabled => Err(ApiError::CreationDisabled),
        CreationMode::Public => Ok(()),
        CreationMode::Token => {
            let presented = headers
                .get(header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.strip_prefix("Bearer "))
                .unwrap_or("");

            let candidate = Verifier::for_creation_token(presented.as_bytes());
            if crate::domain::capability::any_matches(&candidate, &state.creation_tokens) {
                Ok(())
            } else {
                Err(ApiError::Unauthorized)
            }
        }
    }
}

/// Resolves the requested expiry against the configuration.
fn resolve_expiry(state: &AppState, requested: Option<Option<i64>>) -> Result<Option<i64>, ApiError> {
    let now = state.clock.now();
    let maximum = state.pastes.maximum_expiry.as_secs_i64();

    let seconds = match requested {
        // Absent: use the default.
        None => state.pastes.default_expiry.as_secs_i64(),
        // Explicit null: permanent, if allowed.
        Some(None) => {
            return if state.pastes.allow_permanent {
                Ok(None)
            } else {
                Err(ApiError::PermanentNotAllowed)
            };
        }
        Some(Some(seconds)) => seconds,
    };

    if seconds <= 0 {
        return Err(ApiError::BadRequest("expires_in must be positive"));
    }
    if seconds > maximum {
        return Err(ApiError::ExpiryTooLong);
    }

    Ok(Some(now.saturating_add(seconds)))
}

/// Takes the whole request rather than a `Json` argument: an extractor
/// argument is read before the handler runs, so a caller without the creation
/// capability could make the server buffer and parse an upload-sized body. The
/// capability is decided from the head alone, and only then is the body read.
pub(super) async fn create(State(state): State<AppState>, id_extension: Option<Extension<RequestId>>, incoming: Request) -> Response {
    let request_id = request_id(id_extension);

    let outcome = async {
        authorize_creation(&state, incoming.headers())?;

        let Json(request) = Json::<CreateRequest>::from_request(incoming, &state)
            .await
            .map_err(from_json_rejection)?;

        let id = PasteId::parse(&request.id).map_err(|_| ApiError::InvalidId)?;

        if request.format_version <= 0 {
            return Err(ApiError::BadRequest("format_version must be positive"));
        }

        let header_limit = state.pastes.max_public_header_size.as_usize();
        let public_header = decode("public_header", &request.public_header, encoded_len(header_limit))?;
        if public_header.is_empty() {
            return Err(ApiError::BadRequest("public_header must not be empty"));
        }
        if public_header.len() > header_limit {
            return Err(ApiError::PayloadTooLarge);
        }

        let ciphertext = decode("ciphertext", &request.ciphertext, state.max_body_bytes)?;
        if ciphertext.is_empty() {
            return Err(ApiError::BadRequest("ciphertext must not be empty"));
        }

        // The plaintext limit is enforced against the ciphertext, which is the
        // only size the server can observe. AEAD overhead is a fixed additive
        // term, so a generous allowance keeps a maximum-size capsule acceptable
        // without letting an oversized one through.
        let allowance = state.pastes.max_plaintext_size.as_usize().saturating_add(64 * 1024);
        if ciphertext.len() > allowance {
            return Err(ApiError::PayloadTooLarge);
        }

        let reveal_verifier = Verifier::parse(&request.reveal_verifier).map_err(|_| ApiError::BadRequest("reveal_verifier"))?;
        let delete_verifier = Verifier::parse(&request.delete_verifier).map_err(|_| ApiError::BadRequest("delete_verifier"))?;
        let describe_verifier = Verifier::parse(&request.describe_verifier).map_err(|_| ApiError::BadRequest("describe_verifier"))?;

        let expires_at = resolve_expiry(&state, request.expires_in)?;

        let now = state.clock.now();
        let new_paste = NewPaste {
            id,
            format_version: request.format_version,
            public_header,
            ciphertext,
            reveal_verifier,
            delete_verifier,
            describe_verifier,
            created_at: now,
            expires_at,
            burn_after_retrieval: request.burn_after_retrieval,
        };

        let stored = match state.pastes_repo.create(&new_paste).await {
            Ok(stored) => stored,
            // The id is taken. If this is the creator retrying the very same
            // request, answer as the first attempt did (ADR-011); otherwise it
            // stays a conflict.
            Err(RepositoryError::IdConflict) => match state.pastes_repo.find(&new_paste.id).await? {
                Some(existing) if existing.is_replayed_by(&new_paste, now) => existing,
                _ => return Err(ApiError::IdConflict),
            },
            Err(error) => return Err(error.into()),
        };

        Ok(CreateResponse {
            id: stored.id.encode(),
            expires_at: stored.expires_at,
            burn_after_retrieval: stored.burn_after_retrieval,
        })
    }
    .await;

    match outcome {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(error) => problem(&state, request_id, error),
    }
}

pub(super) async fn descriptor(
    State(state): State<AppState>,
    Path(id): Path<String>,
    id_extension: Option<Extension<RequestId>>,
    body: Result<Json<DescribeRequest>, JsonRejection>,
) -> Response {
    let request_id = request_id(id_extension);

    let outcome = async {
        let id = PasteId::parse(&id).map_err(|_| ApiError::InvalidId)?;
        let Json(request) = body.map_err(from_json_rejection)?;

        // Which capability, and what it hashes to. The comparison below runs
        // whether or not the paste exists, against a dummy for unknown ids, so
        // an absent paste and a wrong capability cost the same work.
        enum Proof {
            Describe(Verifier),
            Delete(Verifier),
        }
        let proof = match (&request.describe_token, &request.delete_secret) {
            (Some(token), None) => Proof::Describe(Verifier::for_describe_token(&decode("describe_token", token, 1024)?)),
            (None, Some(secret)) => Proof::Delete(Verifier::for_delete_secret(&decode("delete_secret", secret, 1024)?)),
            _ => return Err(ApiError::BadRequest("send either describe_token or delete_secret")),
        };

        let now = state.clock.now();
        let found = state.pastes_repo.find_metadata(&id).await?;

        let absent = Verifier::for_describe_token(b"absent");
        let capability_ok = match (&proof, found.as_ref()) {
            (Proof::Describe(presented), Some(paste)) => presented.matches(&paste.describe_verifier.unwrap_or(absent)),
            (Proof::Delete(presented), Some(paste)) => presented.matches(&paste.delete_verifier),
            (Proof::Describe(presented) | Proof::Delete(presented), None) => presented.matches(&absent),
        };

        let paste = found
            .filter(|paste| capability_ok && paste.is_retrievable(now))
            .ok_or(ApiError::NotAvailable)?;

        Ok(DescriptorResponse {
            id: paste.id.encode(),
            format_version: paste.format_version,
            public_header: encode(&paste.public_header),
            expires_at: paste.expires_at,
            burn_after_retrieval: paste.burn_after_retrieval,
            payload_size: paste.payload_size,
        })
    }
    .await;

    match outcome {
        Ok(response) => (StatusCode::OK, Json(response)).into_response(),
        Err(error) => problem(&state, request_id, error),
    }
}

pub(super) async fn retrieve(
    State(state): State<AppState>,
    Path(id): Path<String>,
    id_extension: Option<Extension<RequestId>>,
    body: Result<Json<RetrieveRequest>, JsonRejection>,
) -> Response {
    let request_id = request_id(id_extension);
    let mut retry_after = None;

    let outcome = async {
        let id = PasteId::parse(&id).map_err(|_| ApiError::InvalidId)?;
        let Json(request) = body.map_err(from_json_rejection)?;

        let presented_describe = Verifier::for_describe_token(&decode("describe_token", &request.describe_token, 1024)?);
        let presented_reveal = Verifier::for_reveal_token(&decode("reveal_token", &request.reveal_token, 1024)?);

        let now = state.clock.now();
        // Metadata only: the payload is read once the capability has passed.
        let found = state.pastes_repo.find_metadata(&id).await?;

        // Both capabilities are checked against *something* in every case. For
        // an unknown id those are dummy verifiers, so the hashes and the
        // constant-time comparisons still run - an absent paste and wrong tokens
        // then cost the same work.
        let (stored_describe, stored_reveal, paste) = match found.as_ref() {
            Some(paste) => (
                paste.describe_verifier.unwrap_or(Verifier::for_describe_token(b"absent")),
                paste.reveal_verifier,
                Some(paste),
            ),
            None => (Verifier::for_describe_token(b"absent"), Verifier::for_reveal_token(b"absent"), None),
        };
        let holds_link = presented_describe.matches(&stored_describe);
        let reveal_ok = presented_reveal.matches(&stored_reveal);

        let paste = paste.ok_or(ApiError::NotAvailable)?;
        if !holds_link {
            return Err(ApiError::NotAvailable);
        }

        // The caller holds the link, so a wrong reveal token from here on is a
        // wrong password. Those are limited per paste, from any address; a
        // caller with only the id never gets this far, so it can neither use up
        // the attempts nor observe the `429`. Keyed on `id`, the one the paste
        // was looked up by, so every spelling of it in the URI shares a budget.
        if let Err(seconds) = state.limiter.password_attempt(id, !reveal_ok) {
            retry_after = Some(seconds);
            return Err(ApiError::RateLimited);
        }
        if !reveal_ok {
            return Err(ApiError::NotAvailable);
        }

        if !paste.is_retrievable(now) {
            return Err(ApiError::NotAvailable);
        }

        // Burn: atomically read and consume. Two concurrent retrievals will
        // see exactly one success — the repository's `BEGIN IMMEDIATE`
        // guarantees at-most-once.
        if paste.burn_after_retrieval {
            let result = state.pastes_repo.retrieve_and_burn(&id, now).await?;
            match result {
                Some((public_header, ciphertext)) => {
                    return Ok(RetrieveResponse {
                        public_header: encode(&public_header),
                        ciphertext: encode(&ciphertext),
                    });
                }
                None => {
                    return Err(ApiError::AlreadyRetrieved);
                }
            }
        }

        // Normal paste: authorized, so now the payload is worth reading.
        let ciphertext = state.pastes_repo.load_ciphertext(&id, now).await?.ok_or(ApiError::NotAvailable)?;
        let response = RetrieveResponse {
            public_header: encode(&paste.public_header),
            ciphertext: encode(&ciphertext),
        };

        let granularity = state.purge.retrieval_timestamp_granularity.as_secs_i64();
        if paste.needs_retrieval_timestamp(now, granularity) {
            state.pastes_repo.track_retrieval(&id, now, granularity).await?;
        }

        Ok(response)
    }
    .await;

    match (outcome, retry_after) {
        (Ok(response), _) => (StatusCode::OK, Json(response)).into_response(),
        (Err(error), Some(seconds)) => with_retry_after(problem(&state, request_id, error), seconds),
        (Err(error), None) => problem(&state, request_id, error),
    }
}

pub(super) async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
    id_extension: Option<Extension<RequestId>>,
    body: Result<Json<DeleteRequest>, JsonRejection>,
) -> Response {
    let request_id = request_id(id_extension);

    let outcome = async {
        let id = PasteId::parse(&id).map_err(|_| ApiError::InvalidId)?;
        let Json(request) = body.map_err(from_json_rejection)?;

        let secret = decode("delete_secret", &request.delete_secret, 1024)?;
        let presented = Verifier::for_delete_secret(&secret);

        let found = state.pastes_repo.find_metadata(&id).await?;

        let (stored_verifier, exists) = match found.as_ref() {
            Some(paste) => (paste.delete_verifier, true),
            None => (Verifier::for_delete_secret(b"absent"), false),
        };
        let capability_ok = presented.matches(&stored_verifier);

        if !exists || !capability_ok {
            return Err(ApiError::NotAvailable);
        }

        // The return value is ignored on purpose: deleting an already-deleted
        // paste answers exactly like deleting a live one, so a caller cannot
        // probe state by repeating the request.
        let _ = state.pastes_repo.delete(&id).await?;

        Ok(())
    }
    .await;

    match outcome {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => problem(&state, request_id, error),
    }
}

/// Reads the id the middleware put into the request extensions.
///
/// Not from a request header: the middleware sets the header on the *response*,
/// and trusting an incoming one would let a caller choose its own log identifier.
fn request_id(extension: Option<Extension<RequestId>>) -> String {
    extension.map(|Extension(id)| id.0).unwrap_or_default()
}

fn problem(state: &AppState, request_id: String, error: ApiError) -> Response {
    Problem {
        error,
        request_id,
        base_url: state.public_url.clone(),
    }
    .into_response()
}
