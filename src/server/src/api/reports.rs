//! Abuse reports (notice and action, ADR-013).
//!
//! `POST /api/v1/reports` takes a paste id, a reason code, a description and
//! an optional contact. It never asks for — and refuses — the link fragment that
//! holds the key.
//!
//! **The answer does not depend on the id.** Every well-formed report gets the
//! same `202`, whether the paste exists or not; only reports about a paste that
//! currently exists are stored (see
//! [`AbuseRepository::submit_report`](crate::repository::AbuseRepository::submit_report)).
//! The endpoint is therefore no oracle for paste ids, and junk reports about
//! random ids do not reach the operator's queue.
//!
//! Nothing about a report is logged: its text is personal data, and its paste
//! id must not reach the access log either.
//!
//! `GET /api/v1/reports` describes the reporting policy for the report page
//! (whether reports are accepted, the contact address, the reason codes, the
//! limits).

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::api::pastes::codec::from_json_rejection;
use crate::api::problem::{ApiError, Problem};
use crate::api::request_id::RequestId;
use crate::domain::PasteId;
use crate::domain::abuse::{MAX_CONTACT_BYTES, MAX_DESCRIPTION_BYTES, NewReport, ReasonCode, ReportError};
use crate::repository::AbuseRepository as _;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/v1/reports", get(policy).post(submit))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReportRequest {
    paste_id: String,
    reason: String,
    description: String,
    #[serde(default)]
    contact: Option<String>,
}

#[derive(Debug, Serialize)]
struct ReportResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct PolicyResponse {
    enabled: bool,
    contact: Option<String>,
    reasons: Vec<&'static str>,
    max_description_bytes: usize,
    max_contact_bytes: usize,
}

async fn policy(State(state): State<AppState>) -> Json<PolicyResponse> {
    Json(PolicyResponse {
        enabled: state.abuse.enabled,
        contact: state.abuse.contact.clone(),
        reasons: ReasonCode::ALL.iter().map(|code| code.as_str()).collect(),
        max_description_bytes: MAX_DESCRIPTION_BYTES,
        max_contact_bytes: MAX_CONTACT_BYTES,
    })
}

fn report_error(error: ReportError) -> ApiError {
    ApiError::BadRequest(match error {
        ReportError::Reason => "unknown reason",
        ReportError::EmptyDescription => "description must not be empty",
        ReportError::DescriptionTooLong => "description is too long",
        ReportError::ContactTooLong => "contact is too long",
        ReportError::ControlCharacters => "text contains control characters",
        ReportError::ContainsKey => "remove the part after # from any link - it is the decryption key",
    })
}

async fn submit(
    State(state): State<AppState>,
    id_extension: Option<Extension<RequestId>>,
    body: Result<Json<ReportRequest>, JsonRejection>,
) -> Response {
    let request_id = id_extension.map(|Extension(id)| id.0).unwrap_or_default();

    let outcome = async {
        if !state.abuse.enabled {
            return Err(ApiError::ReportsDisabled);
        }

        let Json(request) = body.map_err(from_json_rejection)?;
        let paste_id = PasteId::parse(&request.paste_id).map_err(|_| ApiError::InvalidId)?;

        let now = state.clock.now();
        let report = NewReport::validate(
            ulid::Ulid::generate().to_string(),
            paste_id,
            &request.reason,
            &request.description,
            request.contact.as_deref(),
            now,
        )
        .map_err(report_error)?;

        // Whether it was stored is deliberately not part of the answer.
        let _stored = state.abuse_repo.submit_report(&report, now).await?;
        Ok(())
    }
    .await;

    match outcome {
        Ok(()) => (StatusCode::ACCEPTED, Json(ReportResponse { status: "received" })).into_response(),
        Err(error) => Problem {
            error,
            request_id,
            base_url: state.public_url.clone(),
        }
        .into_response(),
    }
}
