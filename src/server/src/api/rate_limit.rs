//! Per-endpoint-class, per-client-IP rate limiting.
//!
//! In-memory token buckets. A server restart resets all counters, which is
//! acceptable for a single-instance deployment (horizontal scaling is not a 1.0
//! goal). Every limit here is applied **before** the handler, so a rejected
//! request never reaches a paste lookup, a capability check or a burn — and its
//! `429` cannot depend on whether an id exists.
//!
//! Besides one request bucket per endpoint class there are three
//! abuse-specific limits:
//!
//! - **Create byte quota** (`create.bytes_per_day`): a bucket of bytes, charged
//!   with the request's `Content-Length`, so a few maximum-size uploads cannot
//!   hide behind a requests-per-minute limit.
//! - **Concurrent creates** (`create.max_concurrent`): uploads in flight per IP.
//! - **Password attempts per paste** (`invalid_capability_per_paste`, **on by
//!   default**): retrieves of one paste that prove the link (correct describe
//!   token) but fail the reveal token - a wrong password - are limited
//!   regardless of who sends them. This is the server-side protection of a
//!   password when the link has leaked, and it needs no correct
//!   `trusted_proxies` to work. It is the one exception to "before the
//!   handler": the retrieve handler enforces it through
//!   [`LimiterState::password_attempt`] - on the same limiter the middleware
//!   uses, held in `AppState` - with the paste id it looked up, after the
//!   describe token has been checked. Every spelling of an id in the URI thus
//!   shares one budget, and a caller who knows only the id can neither drain
//!   this bucket nor see its `429`, so it is neither a way to block a capsule
//!   nor an oracle for its existence.
//! - **Invalid capabilities** (`invalid_capability`): every describe, retrieve
//!   or delete that ends in `404` costs a token; with the bucket empty, further
//!   ones from that IP are refused before any lookup. That throttles
//!   guessing ids or capabilities without touching legitimate use.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use serde::Deserialize;

use axum::response::IntoResponse as _;

use crate::api::client_ip::TrustedClientIp;
use crate::api::problem::{ApiError, Problem};
use crate::config::ConfigError;
use crate::config::units::ByteSize;
use crate::domain::PasteId;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimit {
    #[serde(default)]
    pub requests_per_minute: u32,
    /// For classes whose sensible rate is below one per minute (reports). Set
    /// either this or `requests_per_minute`, not both.
    #[serde(default)]
    pub requests_per_hour: u32,
    #[serde(default)]
    pub burst: u32,
}

impl RateLimit {
    fn is_enabled(&self) -> bool {
        self.requests_per_minute > 0 || self.requests_per_hour > 0
    }

    fn spec(&self) -> Spec {
        Spec {
            capacity: f64::from(self.burst),
            refill_per_second: f64::from(self.requests_per_minute) / 60.0 + f64::from(self.requests_per_hour) / 3600.0,
        }
    }

    fn validate(&self, class: &str) -> Result<(), ConfigError> {
        let invalid = |message: String| ConfigError::Invalid(message);
        if self.requests_per_minute > 0 && self.requests_per_hour > 0 {
            return Err(invalid(format!(
                "rate_limits.{class}: set requests_per_minute or requests_per_hour, not both"
            )));
        }
        // A bucket that starts empty and never holds a whole token would reject
        // every request - almost certainly a misconfiguration, not a policy.
        if self.is_enabled() && self.burst == 0 {
            return Err(invalid(format!("rate_limits.{class}.burst must be at least 1 when a rate is set")));
        }
        Ok(())
    }
}

/// The create class: a request rate plus a byte quota and a concurrency cap.
/// Each part is independent and off at `0`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateLimit {
    #[serde(default)]
    pub requests_per_minute: u32,
    #[serde(default)]
    pub requests_per_hour: u32,
    #[serde(default)]
    pub burst: u32,
    /// Request bytes per client IP per day, refilled continuously.
    #[serde(default)]
    pub bytes_per_day: ByteSize,
    /// Create requests in flight per client IP.
    #[serde(default)]
    pub max_concurrent: u32,
}

impl CreateLimit {
    fn rate(&self) -> RateLimit {
        RateLimit {
            requests_per_minute: self.requests_per_minute,
            requests_per_hour: self.requests_per_hour,
            burst: self.burst,
        }
    }

    fn bytes_spec(&self) -> Option<Spec> {
        let per_day = self.bytes_per_day.as_u64();
        (per_day > 0).then(|| Spec {
            capacity: per_day as f64,
            refill_per_second: per_day as f64 / 86_400.0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EndpointClass {
    Create,
    Descriptor,
    Retrieve,
    Delete,
    Report,
    /// Health checks, static assets and unrecognised paths — never rate-limited.
    Unclassified,
}

/// What a bucket counts. One map holds them all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BucketKind {
    Requests(EndpointClass),
    CreateBytes,
    InvalidCapability,
    InvalidCapabilityPerPaste,
}

/// Who a bucket belongs to: a client, or (for failed capabilities) a paste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BucketKey {
    Ip(IpAddr),
    Paste(PasteId),
}

fn default_per_paste() -> RateLimit {
    RateLimit {
        requests_per_minute: 10,
        requests_per_hour: 0,
        burst: 10,
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RateLimitConfig {
    #[serde(default)]
    pub create: CreateLimit,
    #[serde(default)]
    pub descriptor: RateLimit,
    #[serde(default)]
    pub retrieve: RateLimit,
    #[serde(default)]
    pub delete: RateLimit,
    #[serde(default)]
    pub report: RateLimit,
    /// Retrieves and deletes answered with `404`, per client IP.
    #[serde(default)]
    pub invalid_capability: RateLimit,
    /// Password attempts (link proven, reveal token wrong), per paste id. On by
    /// default.
    #[serde(default = "default_per_paste")]
    pub invalid_capability_per_paste: RateLimit,
}

/// Off by default, except the per-paste failure limit: every other limit keys
/// on the client IP, which is only right once `server.trusted_proxies` matches
/// the deployment. The per-paste limit does not depend on it.
impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            create: CreateLimit::default(),
            descriptor: RateLimit::default(),
            retrieve: RateLimit::default(),
            delete: RateLimit::default(),
            report: RateLimit::default(),
            invalid_capability: RateLimit::default(),
            invalid_capability_per_paste: default_per_paste(),
        }
    }
}

impl RateLimitConfig {
    /// Whether any limit keys on the client address - which the built-in
    /// privacy notice has to mention (`crate::legal`). The per-paste limit
    /// keys on the paste id and does not count.
    pub fn uses_client_ip(&self) -> bool {
        self.create.rate().is_enabled()
            || self.create.bytes_per_day.as_u64() > 0
            || self.create.max_concurrent > 0
            || [
                &self.descriptor,
                &self.retrieve,
                &self.delete,
                &self.report,
                &self.invalid_capability,
            ]
            .into_iter()
            .any(RateLimit::is_enabled)
    }

    pub(crate) fn validate(&self) -> Result<(), ConfigError> {
        self.create.rate().validate("create")?;
        self.descriptor.validate("descriptor")?;
        self.retrieve.validate("retrieve")?;
        self.delete.validate("delete")?;
        self.report.validate("report")?;
        self.invalid_capability.validate("invalid_capability")?;
        self.invalid_capability_per_paste.validate("invalid_capability_per_paste")
    }

    fn request_limit(&self, class: EndpointClass) -> RateLimit {
        match class {
            EndpointClass::Create => self.create.rate(),
            EndpointClass::Descriptor => self.descriptor.clone(),
            EndpointClass::Retrieve => self.retrieve.clone(),
            EndpointClass::Delete => self.delete.clone(),
            EndpointClass::Report => self.report.clone(),
            EndpointClass::Unclassified => RateLimit::default(),
        }
    }

    fn spec(&self, kind: BucketKind) -> Option<Spec> {
        match kind {
            BucketKind::Requests(class) => {
                let limit = self.request_limit(class);
                limit.is_enabled().then(|| limit.spec())
            }
            BucketKind::CreateBytes => self.create.bytes_spec(),
            BucketKind::InvalidCapability => self.invalid_capability.is_enabled().then(|| self.invalid_capability.spec()),
            BucketKind::InvalidCapabilityPerPaste => self
                .invalid_capability_per_paste
                .is_enabled()
                .then(|| self.invalid_capability_per_paste.spec()),
        }
    }
}

/// Size and refill rate of a bucket.
#[derive(Debug, Clone, Copy)]
struct Spec {
    capacity: f64,
    refill_per_second: f64,
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl Bucket {
    fn new(spec: Spec, now: Instant) -> Self {
        Self {
            tokens: spec.capacity,
            last_refill: now,
        }
    }

    fn refill(&mut self, spec: Spec, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * spec.refill_per_second).min(spec.capacity);
        self.last_refill = now;
    }

    /// Seconds until `cost` tokens are available, at least one.
    fn wait_for(&self, spec: Spec, cost: f64) -> u64 {
        let missing = cost - self.tokens;
        if spec.refill_per_second <= 0.0 {
            return u64::MAX;
        }
        // Saturating float-to-int cast.
        ((missing / spec.refill_per_second).ceil() as u64).max(1)
    }

    /// Takes `cost` tokens, or returns how many whole seconds until they are
    /// available. A cost above the capacity can never be paid.
    fn try_consume(&mut self, spec: Spec, cost: f64, now: Instant) -> Result<(), u64> {
        self.refill(spec, now);
        if cost > spec.capacity {
            return Err(86_400);
        }
        if self.tokens >= cost {
            self.tokens -= cost;
            Ok(())
        } else {
            Err(self.wait_for(spec, cost))
        }
    }

    /// Like [`Self::try_consume`] with a cost of one, but without taking it.
    fn check(&mut self, spec: Spec, now: Instant) -> Result<(), u64> {
        self.refill(spec, now);
        if self.tokens >= 1.0 {
            Ok(())
        } else {
            Err(self.wait_for(spec, 1.0))
        }
    }

    /// Whether the bucket has refilled completely — then it carries no
    /// information and can be dropped.
    fn is_full(&self, spec: Spec, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last_refill).as_secs_f64();
        self.tokens + elapsed * spec.refill_per_second >= spec.capacity
    }
}

/// Above this many buckets, full ones are swept out before a new one is
/// added. Without it, a stream of distinct client addresses (trivial with
/// IPv6) would grow the map without bound.
const SWEEP_THRESHOLD: usize = 10_000;

#[derive(Clone)]
pub struct LimiterState {
    inner: std::sync::Arc<LimiterInner>,
}

struct LimiterInner {
    /// For the `problem+json` type URI, as everywhere else.
    base_url: String,
    buckets: Mutex<HashMap<(BucketKind, BucketKey), Bucket>>,
    in_flight: Mutex<HashMap<IpAddr, u32>>,
    config: RateLimitConfig,
}

impl LimiterState {
    pub fn new(config: RateLimitConfig, base_url: String) -> Self {
        Self {
            inner: std::sync::Arc::new(LimiterInner {
                base_url,
                buckets: Mutex::new(HashMap::new()),
                in_flight: Mutex::new(HashMap::new()),
                config,
            }),
        }
    }

    #[cfg(test)]
    fn new_for_tests(config: RateLimitConfig) -> Self {
        Self::new(config, "https://paste.example".to_owned())
    }

    /// Runs `op` on the bucket for `kind`/`key`, if that kind is enabled.
    fn with_bucket(
        &self,
        kind: BucketKind,
        key: BucketKey,
        op: impl FnOnce(&mut Bucket, Spec, Instant) -> Result<(), u64>,
    ) -> Result<(), u64> {
        let Some(spec) = self.inner.config.spec(kind) else {
            return Ok(());
        };
        let now = Instant::now();
        // A panic elsewhere while the lock is held must not turn every future
        // request into a panic too — recovering the guard is safer than
        // propagating the poison into the request path.
        let mut buckets = self.inner.buckets.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if buckets.len() >= SWEEP_THRESHOLD && !buckets.contains_key(&(kind, key)) {
            let config = &self.inner.config;
            buckets.retain(|(kind, _), bucket| config.spec(*kind).is_some_and(|spec| !bucket.is_full(spec, now)));
        }
        let bucket = buckets.entry((kind, key)).or_insert_with(|| Bucket::new(spec, now));
        op(bucket, spec, now)
    }

    /// Claims an in-flight create slot, released when the guard drops.
    fn claim_slot(&self, ip: IpAddr) -> Option<SlotGuard> {
        let max = self.inner.config.create.max_concurrent;
        if max == 0 {
            return Some(SlotGuard(None));
        }
        let mut in_flight = self.inner.in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let count = in_flight.entry(ip).or_insert(0);
        if *count >= max {
            return None;
        }
        *count += 1;
        Some(SlotGuard(Some((self.clone(), ip))))
    }

    /// The per-paste limit on password attempts: admits one attempt on `paste`
    /// by a link holder, and uses up a token if it `failed` (wrong password).
    /// `Err(seconds)` when the paste has no attempts left; always `Ok` when the
    /// limit is switched off.
    ///
    /// Called by the retrieve handler with the id it looked up, never by the
    /// middleware: only the handler knows whether the caller proved the link.
    /// Checking it before the lookup would let anyone who knows an id see a
    /// `429` for an existing capsule, and charging every `404` would let them
    /// block it. And only the handler's id is the one the paste was found by -
    /// a key taken from the raw URI would give every spelling of the id
    /// (`%41…` for `A…`) a budget of its own.
    ///
    /// Checking and charging happen under one lock: as two steps, parallel
    /// guesses - reveal tokens precomputed offline from a password list - could
    /// all pass the check before any of them was charged.
    pub fn password_attempt(&self, paste: PasteId, failed: bool) -> Result<(), u64> {
        self.with_bucket(
            BucketKind::InvalidCapabilityPerPaste,
            BucketKey::Paste(paste),
            |bucket, spec, now| {
                bucket.check(spec, now)?;
                if failed {
                    bucket.tokens -= 1.0;
                }
                Ok(())
            },
        )
    }
}

/// Releases an in-flight create slot when the request is done — also when the
/// handler panics or the client disconnects and the future is dropped.
struct SlotGuard(Option<(LimiterState, IpAddr)>);

impl Drop for SlotGuard {
    fn drop(&mut self) {
        if let Some((state, ip)) = self.0.take() {
            let mut in_flight = state.inner.in_flight.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(count) = in_flight.get_mut(&ip) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    in_flight.remove(&ip);
                }
            }
        }
    }
}

fn problem(state: &LimiterState, request_id: &str, error: ApiError) -> Response {
    Problem {
        error,
        request_id: request_id.to_owned(),
        base_url: state.inner.base_url.clone(),
    }
    .into_response()
}

fn too_many_requests(state: &LimiterState, request_id: &str, retry_after: u64) -> Response {
    with_retry_after(problem(state, request_id, ApiError::RateLimited), retry_after)
}

/// Takes one token, never below zero: used after the fact, when the request
/// has already been answered.
fn charge_one(bucket: &mut Bucket, spec: Spec, now: Instant) -> Result<(), u64> {
    bucket.refill(spec, now);
    bucket.tokens = (bucket.tokens - 1.0).max(0.0);
    Ok(())
}

/// Adds `Retry-After` to a `429` built elsewhere.
pub fn with_retry_after(mut response: Response, retry_after: u64) -> Response {
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, axum::http::HeaderValue::from(retry_after));
    response
}

/// The middleware handler. Use with `axum::middleware::from_fn_with_state`.
pub async fn middleware(State(state): State<LimiterState>, request: Request, next: Next) -> Response {
    let class = classify(&request);
    if class == EndpointClass::Unclassified {
        return next.run(request).await;
    }

    let ip = request
        .extensions()
        .get::<TrustedClientIp>()
        .map_or(IpAddr::from([0, 0, 0, 0]), |ip| ip.0);

    let request_id = request
        .extensions()
        .get::<crate::api::request_id::RequestId>()
        .map(|id| id.0.clone())
        .unwrap_or_default();

    // Every endpoint that checks a capability: failures count against the
    // per-client failure limit.
    let guards_capabilities = matches!(class, EndpointClass::Descriptor | EndpointClass::Retrieve | EndpointClass::Delete);

    // Too many failed capability checks from this client: refuse before
    // anything is looked up.
    if guards_capabilities
        && let Err(retry_after) = state.with_bucket(BucketKind::InvalidCapability, BucketKey::Ip(ip), |bucket, spec, now| {
            bucket.check(spec, now)
        })
    {
        return too_many_requests(&state, &request_id, retry_after);
    }

    if let Err(retry_after) = state.with_bucket(BucketKind::Requests(class), BucketKey::Ip(ip), |bucket, spec, now| {
        bucket.try_consume(spec, 1.0, now)
    }) {
        return too_many_requests(&state, &request_id, retry_after);
    }

    let mut _slot = None;
    if class == EndpointClass::Create {
        if state.inner.config.create.bytes_spec().is_some() {
            // The body is not read yet; its declared length is what the quota
            // is charged with. Without one, the cost is unknown up front.
            let Some(length) = request
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
            else {
                return problem(&state, &request_id, ApiError::LengthRequired);
            };
            if let Err(retry_after) = state.with_bucket(BucketKind::CreateBytes, BucketKey::Ip(ip), |bucket, spec, now| {
                bucket.try_consume(spec, length as f64, now)
            }) {
                return too_many_requests(&state, &request_id, retry_after);
            }
        }

        match state.claim_slot(ip) {
            Some(slot) => _slot = Some(slot),
            None => return too_many_requests(&state, &request_id, 1),
        }
    }

    let response = next.run(request).await;

    if guards_capabilities && response.status() == StatusCode::NOT_FOUND {
        // Charged after the fact: the answer is the same for every reason a
        // paste is unavailable, so this does not single out any of them.
        let _ = state.with_bucket(BucketKind::InvalidCapability, BucketKey::Ip(ip), charge_one);
    }

    response
}

fn classify(request: &Request) -> EndpointClass {
    let path = request.uri().path();
    let method = request.method().as_str();

    if path == "/api/v1/pastes" && method == "POST" {
        EndpointClass::Create
    } else if path == "/api/v1/reports" && method == "POST" {
        EndpointClass::Report
    } else if path.starts_with("/api/v1/pastes/") && path.ends_with("/retrieve") && method == "POST" {
        EndpointClass::Retrieve
    } else if path.starts_with("/api/v1/pastes/") && path.ends_with("/describe") && method == "POST" {
        EndpointClass::Descriptor
    } else if path.starts_with("/api/v1/pastes/") && method == "DELETE" {
        EndpointClass::Delete
    } else {
        EndpointClass::Unclassified
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec(per_minute: u32, per_hour: u32, burst: u32) -> Spec {
        RateLimit {
            requests_per_minute: per_minute,
            requests_per_hour: per_hour,
            burst,
        }
        .spec()
    }

    fn limit(per_minute: u32, per_hour: u32, burst: u32) -> RateLimit {
        RateLimit {
            requests_per_minute: per_minute,
            requests_per_hour: per_hour,
            burst,
        }
    }

    #[test]
    fn a_bucket_allows_the_burst_then_refuses_with_a_wait() {
        let spec = spec(60, 0, 2);
        let start = Instant::now();
        let mut bucket = Bucket::new(spec, start);

        assert_eq!(bucket.try_consume(spec, 1.0, start), Ok(()));
        assert_eq!(bucket.try_consume(spec, 1.0, start), Ok(()));
        // One per second refill: the next token is one second away.
        assert_eq!(bucket.try_consume(spec, 1.0, start), Err(1));
        assert_eq!(bucket.try_consume(spec, 1.0, start + Duration::from_secs(1)), Ok(()));
    }

    #[test]
    fn an_hourly_rate_reports_an_hourly_wait() {
        let spec = spec(0, 5, 1);
        let start = Instant::now();
        let mut bucket = Bucket::new(spec, start);

        assert_eq!(bucket.try_consume(spec, 1.0, start), Ok(()));
        // Five per hour: one token every 720 seconds.
        assert_eq!(bucket.try_consume(spec, 1.0, start), Err(720));
        assert_eq!(bucket.try_consume(spec, 1.0, start + Duration::from_secs(360)), Err(360));
        assert_eq!(bucket.try_consume(spec, 1.0, start + Duration::from_secs(720)), Ok(()));
    }

    #[test]
    fn the_bucket_never_holds_more_than_the_burst() {
        let spec = spec(60, 0, 2);
        let start = Instant::now();
        let mut bucket = Bucket::new(spec, start);
        let later = start + Duration::from_secs(3600);

        assert_eq!(bucket.try_consume(spec, 1.0, later), Ok(()));
        assert_eq!(bucket.try_consume(spec, 1.0, later), Ok(()));
        assert!(bucket.try_consume(spec, 1.0, later).is_err());
    }

    #[test]
    fn a_byte_quota_charges_the_size_and_refills_over_the_day() {
        let quota = CreateLimit {
            bytes_per_day: ByteSize(86_400),
            ..CreateLimit::default()
        };
        let spec = quota.bytes_spec().unwrap_or(Spec {
            capacity: 0.0,
            refill_per_second: 0.0,
        });
        let start = Instant::now();
        let mut bucket = Bucket::new(spec, start);

        assert_eq!(bucket.try_consume(spec, 80_000.0, start), Ok(()));
        // 6,400 left at one byte per second: 10,000 bytes are 3,600 s away.
        assert_eq!(bucket.try_consume(spec, 10_000.0, start), Err(3_600));
        assert_eq!(bucket.try_consume(spec, 10_000.0, start + Duration::from_secs(3_600)), Ok(()));
        // More than a whole day's quota can never be paid.
        assert_eq!(
            bucket.try_consume(spec, 100_000.0, start + Duration::from_secs(86_400 * 2)),
            Err(86_400)
        );
    }

    #[test]
    fn check_does_not_consume() {
        let spec = spec(60, 0, 1);
        let start = Instant::now();
        let mut bucket = Bucket::new(spec, start);
        assert_eq!(bucket.check(spec, start), Ok(()));
        assert_eq!(bucket.check(spec, start), Ok(()));
        assert_eq!(bucket.try_consume(spec, 1.0, start), Ok(()));
        assert_eq!(bucket.check(spec, start), Err(1));
    }

    #[test]
    fn full_buckets_are_swept_once_the_map_is_large() {
        let state = LimiterState::new_for_tests(RateLimitConfig {
            descriptor: limit(60, 0, 1),
            ..RateLimitConfig::default()
        });
        let kind = BucketKind::Requests(EndpointClass::Descriptor);
        for i in 0..SWEEP_THRESHOLD as u32 {
            let ip = IpAddr::from(i.to_be_bytes());
            // Untouched buckets are full right away.
            let _ = state.with_bucket(kind, BucketKey::Ip(ip), |_, _, _| Ok(()));
        }
        let _ = state.with_bucket(kind, BucketKey::Ip(IpAddr::from([255, 255, 255, 255])), |_, _, _| Ok(()));
        let size = state.inner.buckets.lock().map(|b| b.len()).unwrap_or_default();
        assert_eq!(size, 1, "full buckets were not swept");
    }

    #[test]
    fn concurrent_create_slots_are_capped_and_released() {
        let state = LimiterState::new_for_tests(RateLimitConfig {
            create: CreateLimit {
                max_concurrent: 2,
                ..CreateLimit::default()
            },
            ..RateLimitConfig::default()
        });
        let ip = IpAddr::from([10, 0, 0, 1]);
        let other = IpAddr::from([10, 0, 0, 2]);

        let first = state.claim_slot(ip);
        let second = state.claim_slot(ip);
        assert!(first.is_some() && second.is_some());
        assert!(state.claim_slot(ip).is_none(), "a third upload was allowed");
        assert!(state.claim_slot(other).is_some(), "the cap is per IP");

        drop(first);
        assert!(state.claim_slot(ip).is_some(), "the slot was not released");
    }

    #[test]
    fn requests_are_classified_by_method_and_path() {
        let classify_one = |method: &str, path: &str| {
            let request = Request::builder()
                .method(method)
                .uri(path)
                .body(axum::body::Body::empty())
                .unwrap_or_default();
            classify(&request)
        };
        let id = "AAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(classify_one("POST", "/api/v1/pastes"), EndpointClass::Create);
        assert_eq!(
            classify_one("POST", &format!("/api/v1/pastes/{id}/describe")),
            EndpointClass::Descriptor
        );
        assert_eq!(classify_one("GET", &format!("/api/v1/pastes/{id}")), EndpointClass::Unclassified);
        assert_eq!(
            classify_one("POST", &format!("/api/v1/pastes/{id}/retrieve")),
            EndpointClass::Retrieve
        );
        assert_eq!(classify_one("DELETE", &format!("/api/v1/pastes/{id}")), EndpointClass::Delete);
        assert_eq!(classify_one("POST", "/api/v1/reports"), EndpointClass::Report);
        assert_eq!(classify_one("GET", "/api/v1/reports"), EndpointClass::Unclassified);
        assert_eq!(classify_one("GET", "/report"), EndpointClass::Unclassified);
    }

    #[test]
    fn validation_rejects_both_rates_and_a_rate_without_burst() {
        assert!(limit(0, 0, 0).validate("x").is_ok(), "disabled is fine");
        assert!(limit(5, 0, 2).validate("x").is_ok());
        assert!(limit(0, 5, 1).validate("x").is_ok());
        assert!(limit(5, 5, 1).validate("x").is_err());
        assert!(limit(5, 0, 0).validate("x").is_err());
    }
}
