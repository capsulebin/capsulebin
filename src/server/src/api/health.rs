//! Health endpoints.
//!
//! The split is deliberate: `live` only states that the process is running,
//! `ready` states whether it can accept work. A reverse proxy should stop
//! sending traffic when `ready` fails, but must not kill the process — a database
//! that is briefly unreachable is a reason to wait, not to restart.

use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};
use serde::Serialize;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/health/live", get(live)).route("/health/ready", get(ready))
}

#[derive(Serialize)]
struct Live {
    status: &'static str,
}

async fn live() -> impl IntoResponse {
    (StatusCode::OK, Json(Live { status: "ok" }))
}

#[derive(Serialize)]
struct Ready {
    status: &'static str,
    database: &'static str,
    /// Applied migration version, present only when the schema check passed.
    /// Useful during a rollout and harmless to expose: it says nothing about
    /// stored content.
    #[serde(skip_serializing_if = "Option::is_none")]
    schema_version: Option<i64>,
    assets: &'static str,
}

async fn ready(axum::extract::State(state): axum::extract::State<AppState>) -> impl IntoResponse {
    // Readiness means: the shell can be served *and* the database is usable at the
    // expected schema version. Reporting ready while migrations are pending would
    // let a rollout send traffic to a server that fails every write.
    let assets_ok = state.assets_ready();

    let schema = crate::db::verify(&state.pool).await;
    let database_ok = schema.is_ok();

    let ready = assets_ok && database_ok;

    // Every degraded outcome leaves a trace, naming the component that failed.
    // A 503 with nothing in the log gives an operator no starting point - and
    // missing assets in particular produce no error of their own, so without this
    // the most likely cause would be the one that is invisible.
    if !ready {
        tracing::warn!(
            assets_ok,
            database_ok,
            database_error = schema.as_ref().err().map(ToString::to_string),
            "readiness check failed"
        );
    }

    (
        if ready { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE },
        Json(Ready {
            status: if ready { "ok" } else { "degraded" },
            database: if database_ok { "ok" } else { "unavailable" },
            schema_version: schema.ok(),
            assets: if assets_ok { "ok" } else { "missing" },
        }),
    )
}
