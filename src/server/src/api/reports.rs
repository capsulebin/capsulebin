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
//! Each capsule and the instance take only a bounded number of open reports
//! (`abuse.max_open_reports_per_paste`, `abuse.max_open_reports`); a report
//! above a ceiling is dropped and gets the same `202`.
//!
//! Whether reports are accepted is decided before a byte of the body is read,
//! and the body is limited to [`MAX_REPORT_BODY_BYTES`].
//!
//! Nothing about a report is logged: its text is personal data, and its paste
//! id must not reach the access log either. A dropped report leaves one
//! warning without either, at most once per [`CEILING_WARNING_INTERVAL`].
//!
//! `GET /api/v1/reports` describes the reporting policy for the report page
//! (whether reports are accepted, the contact address, the reason codes, the
//! limits).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::{FromRequest as _, Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::{Deserialize, Serialize};
use tower_http::limit::RequestBodyLimitLayer;

use crate::AppState;
use crate::api::pastes::codec::from_json_rejection;
use crate::api::problem::{ApiError, Problem};
use crate::api::request_id::RequestId;
use crate::domain::PasteId;
use crate::domain::abuse::{MAX_CONTACT_BYTES, MAX_DESCRIPTION_BYTES, NewReport, ReasonCode, ReportError};
use crate::repository::{AbuseRepository as _, SubmitOutcome};

/// The body limit of `POST /api/v1/reports`. A report holds at most
/// `MAX_DESCRIPTION_BYTES` + `MAX_CONTACT_BYTES` of text; even with every
/// character escaped as `\uXXXX` that stays below 26 KB, so no valid report
/// is refused. Sits inside the global `max_body_bytes` limit.
pub const MAX_REPORT_BODY_BYTES: usize = 32 * 1024;

/// At most one "open report limit" warning per this interval and instance, so a
/// flood of reports cannot flood the log.
pub const CEILING_WARNING_INTERVAL: Duration = Duration::from_secs(60);

/// When the last "open report limit" warning was logged, one per state: the
/// clones of an [`AppState`] share it.
#[derive(Clone, Debug, Default)]
pub struct CeilingWarning(Arc<Mutex<Option<Instant>>>);

impl CeilingWarning {
    /// Logs that a report was dropped at a ceiling, unless that was already
    /// logged within [`CEILING_WARNING_INTERVAL`]. Names neither the paste nor
    /// the report.
    pub fn warn(&self) {
        let now = Instant::now();
        {
            let mut last = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            if last.is_some_and(|at| now.duration_since(at) < CEILING_WARNING_INTERVAL) {
                return;
            }
            *last = Some(now);
        }
        tracing::warn!(
            "report not stored: open report limit reached (abuse.max_open_reports_per_paste or \
             abuse.max_open_reports); see `capsulebin reports list`"
        );
    }
}

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/v1/reports",
        get(policy).merge(post(submit).layer(RequestBodyLimitLayer::new(MAX_REPORT_BODY_BYTES))),
    )
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
        ReportError::ContainsKey => "remove the part after # from any link - it is the decryption key or a creation token",
    })
}

/// Takes the whole request rather than a `Json` argument: an extractor
/// argument is read before the handler runs, and a disabled endpoint must not
/// read the body at all.
async fn submit(State(state): State<AppState>, id_extension: Option<Extension<RequestId>>, request: Request) -> Response {
    let request_id = id_extension.map(|Extension(id)| id.0).unwrap_or_default();

    let outcome = async {
        if !state.abuse.enabled {
            return Err(ApiError::ReportsDisabled);
        }

        let Json(request) = Json::<ReportRequest>::from_request(request, &state)
            .await
            .map_err(from_json_rejection)?;
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

        // What happened to the report is deliberately not part of the answer.
        match state.abuse_repo.submit_report(&report, state.abuse.report_ceilings(), now).await? {
            SubmitOutcome::CeilingReached => state.report_ceiling_warning.warn(),
            SubmitOutcome::Stored | SubmitOutcome::NoSuchPaste => {}
        }
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
