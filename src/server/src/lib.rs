//! CapsuleBin server.
//!
//! The server is content-blind: it stores ciphertext, enforces limits and drives
//! the lifecycle. Plaintext, keys and passwords never reach it.
//!
//! The opaque storage API (create, describe, retrieve, delete, burn), abuse
//! reports and moderation, the retention job and the security boundary live
//! here. All cryptography happens in the clients.

pub mod api;
pub mod backup;
pub mod config;
pub mod db;
pub mod domain;
pub mod healthcheck;
pub mod legal;
pub mod repository;

use std::sync::Arc;

use axum::Router;
use sqlx::SqlitePool;
use tower_http::set_header::SetResponseHeaderLayer;

use api::assets::{AssetSource, Languages};
use config::{AbuseConfig, CreationMode, PastesConfig, PurgeConfig, Resolved};
use domain::{Clock, Verifier};
use repository::sqlite::SqlitePasteRepository;
use repository::sqlite::abuse::SqliteAbuseRepository;

/// Shared state.
///
/// The repository is held as its concrete type rather than behind `dyn`: the
/// trait's methods return `impl Future`, which is not dyn-compatible. Swapping
/// the implementation is a type change here, not a runtime decision - acceptable,
/// because there is exactly one production backend by design.
#[derive(Clone)]
pub struct AppState {
    pub assets: Arc<AssetSource>,
    pub clock: Arc<dyn Clock>,
    pub pastes_repo: Arc<SqlitePasteRepository>,
    pub abuse_repo: Arc<SqliteAbuseRepository>,
    pub pool: SqlitePool,
    /// Base URL for `problem+json` type URIs. Comes from configuration, never
    /// from a request header.
    pub public_url: String,
    pub creation_mode: CreationMode,
    pub creation_tokens: Arc<Vec<Verifier>>,
    pub pastes: PastesConfig,
    /// Held by the request path only for `retrieval_timestamp_granularity`; the
    /// retention thresholds belong to the purge job.
    pub purge: PurgeConfig,
    pub max_body_bytes: usize,
    pub trusted_proxies: Vec<String>,
    pub rate_limits: crate::api::rate_limit::RateLimitConfig,
    pub abuse: AbuseConfig,
    /// The offered interface languages with the operator's legal pages and
    /// footer links for each, rendered once from `[i18n]` and `[legal]` (see
    /// [`legal`] and [`api::assets`]).
    pub languages: Arc<Languages>,
}

impl AppState {
    pub fn new(resolved: &Resolved, pool: SqlitePool, assets: AssetSource, clock: Arc<dyn Clock>) -> Self {
        Self {
            assets: Arc::new(assets),
            clock,
            pastes_repo: Arc::new(SqlitePasteRepository::new(pool.clone())),
            abuse_repo: Arc::new(SqliteAbuseRepository::new(pool.clone())),
            pool,
            public_url: resolved
                .config
                .server
                .public_url
                .clone()
                .unwrap_or_else(|| "http://localhost:8080".to_owned()),
            creation_mode: resolved.config.creation.mode,
            creation_tokens: Arc::new(resolved.creation_tokens.clone()),
            pastes: resolved.config.pastes.clone(),
            purge: resolved.config.purge.clone(),
            max_body_bytes: resolved.max_body_bytes,
            trusted_proxies: resolved.config.server.trusted_proxies.clone(),
            rate_limits: resolved.config.rate_limits.clone(),
            abuse: resolved.config.abuse.clone(),
            languages: Arc::new(Languages::new(resolved.languages.clone(), resolved.default_language)),
        }
    }

    /// Whether the application can be served, used by `/health/ready`: every
    /// shell of every offered language.
    pub fn assets_ready(&self) -> bool {
        self.assets.has_shells(self.languages.codes())
    }
}

/// Builds the router.
///
/// Layer order matters and is not cosmetic:
///
/// - The security headers and the request id sit **outermost**, so they reach
///   every response including rejections produced by the layers below. A header
///   set only on the success path is not a security boundary.
/// - The client IP is resolved before the rate limiter runs, and the server
///   must be started with connect info (`into_make_service_with_connect_info`),
///   otherwise every request looks like it came from one address.
/// - The body limit sits below them, so an oversized request is rejected before a
///   handler allocates anything - but still gets the headers.
/// - `DefaultBodyLimit::disable()` turns off axum's own per-extractor cap, which
///   defaults to a hard-coded 2 MiB independently of any layer here. Without it,
///   every `Json<T>` extraction in `api::pastes` would reject bodies above 2 MiB
///   regardless of `max_body_bytes`, silently overriding `pastes.max_plaintext_size`.
///   `RequestBodyLimitLayer` above remains the one real limit.
pub fn router(state: AppState) -> Router {
    let max_body = state.max_body_bytes;

    let mut app = Router::new()
        .merge(api::health::routes())
        .merge(api::pastes::routes())
        .merge(api::reports::routes())
        .merge(api::assets::routes(&state.languages))
        // `.layer()` wraps, so the *later* call runs *first*: the client IP is
        // resolved before the rate limiter reads it. In the opposite order every
        // client shared one bucket (`tests/rate_limits.rs` pins this down).
        .layer(axum::middleware::from_fn_with_state(
            api::rate_limit::LimiterState::new(state.rate_limits.clone(), state.public_url.clone()),
            api::rate_limit::middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            api::client_ip::CidrList::parse(&state.trusted_proxies),
            api::client_ip::middleware,
        ))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(max_body))
        .layer(axum::middleware::from_fn_with_state(
            state.public_url.clone(),
            api::problem::body_limit_problem,
        ))
        .layer(axum::extract::DefaultBodyLimit::disable());

    for (name, value) in api::security_headers::headers() {
        app = app.layer(SetResponseHeaderLayer::overriding(name, value));
    }

    app.layer(axum::middleware::from_fn(api::request_id::middleware))
        // Records method and a redacted path only - never the full URI, which
        // would put paste ids into the access log. See `api::access_log`.
        .layer(api::access_log::layer())
        .with_state(state)
}
