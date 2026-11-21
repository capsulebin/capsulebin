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
//!
//! Buckets live in two tables, one keyed by client (request classes, create
//! bytes, invalid capabilities) and one keyed by paste (password attempts),
//! each capped at 100 000 entries (`CLIENT_CEILING`, `PASTE_CEILING`). The
//! work per request is bounded by a constant, whatever their size: a new key
//! lets a sweep cursor look at a few entries and drop those that have refilled
//! completely (they carry no information), and a table at its ceiling then
//! decides by its own rule. The client table evicts its least recently used
//! bucket - that address starts afresh, which costs nothing an attacker with
//! many addresses does not already have. The paste table never evicts a
//! budget: a wrong password on a paste it does not track is refused, a correct
//! one passes (see [`LimiterState::password_attempt`]). A saturated table logs
//! a warning at most once a minute, naming the table and its ceiling, never an
//! address or an id.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

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

/// What a bucket counts. `InvalidCapabilityPerPaste` buckets are keyed by the
/// paste and live in the paste table, all others in the client table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum BucketKind {
    Requests(EndpointClass),
    CreateBytes,
    InvalidCapability,
    InvalidCapabilityPerPaste,
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

impl Spec {
    /// Whole seconds until `missing` tokens have been refilled, at least one.
    fn seconds_to_refill(self, missing: f64) -> u64 {
        if self.refill_per_second <= 0.0 {
            return u64::MAX;
        }
        // Saturating float-to-int cast.
        ((missing / self.refill_per_second).ceil() as u64).max(1)
    }
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
        spec.seconds_to_refill(cost - self.tokens)
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

/// Most client buckets (request classes, create bytes, invalid capabilities)
/// the limiter keeps. An entry (bucket, recency links, index) takes about a
/// hundred bytes, so a full table stays near 10 MB.
const CLIENT_CEILING: usize = 100_000;

/// Most paste buckets (`invalid_capability_per_paste`) the limiter keeps. A
/// table of its own, so a flood of client addresses cannot push out password
/// budgets.
const PASTE_CEILING: usize = 100_000;

/// Entries the reclaim sweep looks at for each new key. Together with the
/// lookup and at most one eviction, this bounds the work done under the lock
/// per admission, whatever the size of the table.
const SWEEP_STEP: usize = 8;

/// A saturated table logs at most one warning in this interval.
const SATURATION_WARNING_INTERVAL: Duration = Duration::from_secs(60);

/// The end of the recency list.
const NONE: usize = usize::MAX;

/// The limiter's monotonic time source: `Instant::now` in production, a clock
/// moved by hand in the tests.
type Now = Arc<dyn Fn() -> Instant + Send + Sync>;

/// What a table at its ceiling does with a new key once the sweep freed nothing.
#[derive(Debug, Clone, Copy)]
enum WhenFull {
    /// Drop the least recently used bucket. Right for client buckets: an
    /// attacker with many addresses gets fresh budgets anyway.
    EvictOldest,
    /// Track nothing new. Right for paste buckets: evicting one would reset a
    /// password budget, so flooding the table would unlock new guesses.
    Refuse,
}

struct Entry<K> {
    key: K,
    bucket: Bucket,
    spec: Spec,
    /// Neighbours in the recency list, [`NONE`] at either end.
    newer: usize,
    older: usize,
}

/// Buckets with a ceiling and bounded work per admission.
///
/// The entries sit in a dense vector, indexed by a map from key to slot and
/// threaded on a doubly linked list from the most to the least recently used
/// one. Each new key first lets a sweep cursor walk [`SWEEP_STEP`] slots and
/// drop the buckets that have refilled completely (they carry no
/// information). So full buckets are reclaimed before anything is evicted, and
/// the table shrinks back after a burst without a full scan. Only at the
/// ceiling does [`WhenFull`] decide.
struct Table<K> {
    /// For the saturation warning, never a key.
    name: &'static str,
    ceiling: usize,
    when_full: WhenFull,
    slots: HashMap<K, usize>,
    entries: Vec<Entry<K>>,
    newest: usize,
    oldest: usize,
    cursor: usize,
    last_warning: Option<Instant>,
    warning_due: bool,
    /// Entries visited by the last admission.
    #[cfg(test)]
    visits: usize,
}

impl<K: Copy + Eq + Hash> Table<K> {
    fn new(name: &'static str, ceiling: usize, when_full: WhenFull) -> Self {
        Self {
            name,
            ceiling,
            when_full,
            slots: HashMap::new(),
            entries: Vec::new(),
            newest: NONE,
            oldest: NONE,
            cursor: 0,
            last_warning: None,
            warning_due: false,
            #[cfg(test)]
            visits: 0,
        }
    }

    fn visit(&mut self) {
        #[cfg(test)]
        {
            self.visits += 1;
        }
    }

    /// The slot of `key`, now the most recently used one. Starts an admission.
    fn find(&mut self, key: &K) -> Option<usize> {
        #[cfg(test)]
        {
            self.visits = 0;
        }
        self.visit();
        let slot = *self.slots.get(key)?;
        if slot != self.newest {
            self.unlink(slot);
            self.push_newest(slot);
        }
        Some(slot)
    }

    /// The slot of `key`, with a fresh bucket if it had none. `None` only when
    /// a [`WhenFull::Refuse`] table is at its ceiling.
    fn admit(&mut self, key: K, spec: Spec, now: Instant) -> Option<usize> {
        match self.find(&key) {
            Some(slot) => Some(slot),
            None => self.insert(key, spec, now),
        }
    }

    /// Adds a bucket for `key`, which has none.
    fn insert(&mut self, key: K, spec: Spec, now: Instant) -> Option<usize> {
        self.reclaim(now);
        if self.entries.len() >= self.ceiling {
            self.note_saturation(now);
            match self.when_full {
                WhenFull::Refuse => return None,
                WhenFull::EvictOldest => {
                    if self.oldest == NONE {
                        return None;
                    }
                    self.visit();
                    self.remove(self.oldest);
                }
            }
        }
        let slot = self.entries.len();
        self.entries.push(Entry {
            key,
            bucket: Bucket::new(spec, now),
            spec,
            newer: NONE,
            older: NONE,
        });
        self.slots.insert(key, slot);
        self.push_newest(slot);
        Some(slot)
    }

    fn bucket(&mut self, slot: usize) -> &mut Bucket {
        &mut self.entries[slot].bucket
    }

    /// Walks the sweep cursor [`SWEEP_STEP`] slots on and drops every bucket
    /// on the way that has refilled completely.
    fn reclaim(&mut self, now: Instant) {
        for _ in 0..SWEEP_STEP {
            if self.entries.is_empty() {
                return;
            }
            if self.cursor >= self.entries.len() {
                self.cursor = 0;
            }
            self.visit();
            let entry = &self.entries[self.cursor];
            if entry.bucket.is_full(entry.spec, now) {
                // The last entry moves into this slot and is looked at next.
                self.remove(self.cursor);
            } else {
                self.cursor += 1;
            }
        }
    }

    fn note_saturation(&mut self, now: Instant) {
        if self
            .last_warning
            .is_none_or(|at| now.saturating_duration_since(at) >= SATURATION_WARNING_INTERVAL)
        {
            self.last_warning = Some(now);
            self.warning_due = true;
        }
    }

    /// The warning noted by the last admission, to be logged once the lock is
    /// released.
    fn take_warning(&mut self) -> Option<SaturationWarning> {
        std::mem::take(&mut self.warning_due).then_some(SaturationWarning {
            table: self.name,
            ceiling: self.ceiling,
            when_full: self.when_full,
        })
    }

    fn push_newest(&mut self, slot: usize) {
        self.entries[slot].newer = NONE;
        self.entries[slot].older = self.newest;
        if self.newest == NONE {
            self.oldest = slot;
        } else {
            self.entries[self.newest].newer = slot;
        }
        self.newest = slot;
    }

    fn unlink(&mut self, slot: usize) {
        let Entry { newer, older, .. } = self.entries[slot];
        if newer == NONE {
            self.newest = older;
        } else {
            self.entries[newer].older = older;
        }
        if older == NONE {
            self.oldest = newer;
        } else {
            self.entries[older].newer = newer;
        }
    }

    /// Removes the entry in `slot`; the last entry takes its place.
    fn remove(&mut self, slot: usize) {
        self.unlink(slot);
        let removed = self.entries.swap_remove(slot);
        self.slots.remove(&removed.key);
        if slot < self.entries.len() {
            let Entry { key, newer, older, .. } = self.entries[slot];
            self.slots.insert(key, slot);
            if newer == NONE {
                self.newest = slot;
            } else {
                self.entries[newer].older = slot;
            }
            if older == NONE {
                self.oldest = slot;
            } else {
                self.entries[older].newer = slot;
            }
        }
    }
}

/// A table reached its ceiling. Names the table, never a key.
struct SaturationWarning {
    table: &'static str,
    ceiling: usize,
    when_full: WhenFull,
}

impl SaturationWarning {
    fn log(&self) {
        let Self { table, ceiling, .. } = *self;
        match self.when_full {
            WhenFull::EvictOldest => tracing::warn!(
                table,
                ceiling,
                "rate limiter table full: the least recently used bucket makes room for each new one"
            ),
            WhenFull::Refuse => tracing::warn!(
                table,
                ceiling,
                "rate limiter table full: wrong passwords on capsules without a bucket are refused until buckets refill"
            ),
        }
    }
}

/// Recovers the guard of a poisoned lock: a panic elsewhere while it was held
/// must not turn every future request into a panic too.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub struct LimiterState {
    inner: Arc<LimiterInner>,
}

struct LimiterInner {
    /// For the `problem+json` type URI, as everywhere else.
    base_url: String,
    clients: Mutex<Table<(BucketKind, IpAddr)>>,
    pastes: Mutex<Table<PasteId>>,
    in_flight: Mutex<HashMap<IpAddr, u32>>,
    config: RateLimitConfig,
    now: Now,
}

impl LimiterState {
    pub fn new(config: RateLimitConfig, base_url: String) -> Self {
        Self::build(config, base_url, CLIENT_CEILING, PASTE_CEILING, Arc::new(Instant::now))
    }

    fn build(config: RateLimitConfig, base_url: String, client_ceiling: usize, paste_ceiling: usize, now: Now) -> Self {
        Self {
            inner: Arc::new(LimiterInner {
                base_url,
                clients: Mutex::new(Table::new("client", client_ceiling, WhenFull::EvictOldest)),
                pastes: Mutex::new(Table::new("paste", paste_ceiling, WhenFull::Refuse)),
                in_flight: Mutex::new(HashMap::new()),
                config,
                now,
            }),
        }
    }

    #[cfg(test)]
    fn new_for_tests(config: RateLimitConfig) -> Self {
        Self::new(config, "https://paste.example".to_owned())
    }

    /// A limiter with small ceilings on a clock the test moves.
    #[cfg(test)]
    fn bounded_for_tests(config: RateLimitConfig, client_ceiling: usize, paste_ceiling: usize, now: Now) -> Self {
        Self::build(config, "https://paste.example".to_owned(), client_ceiling, paste_ceiling, now)
    }

    /// Runs `op` on the bucket for `kind` of client `ip`, if that kind is
    /// enabled.
    fn with_bucket(&self, kind: BucketKind, ip: IpAddr, op: impl FnOnce(&mut Bucket, Spec, Instant) -> Result<(), u64>) -> Result<(), u64> {
        let Some(spec) = self.inner.config.spec(kind) else {
            return Ok(());
        };
        let now = (self.inner.now)();
        let (result, warning) = {
            let mut clients = lock(&self.inner.clients);
            // The client table evicts at its ceiling, so it always has a slot.
            let result = clients
                .admit((kind, ip), spec, now)
                .map_or(Ok(()), |slot| op(clients.bucket(slot), spec, now));
            (result, clients.take_warning())
        };
        if let Some(warning) = warning {
            warning.log();
        }
        result
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
    ///
    /// A paste without a bucket has its whole budget, so a correct password
    /// creates none. With the paste table at its ceiling and nothing to
    /// reclaim, a wrong password on such a paste is refused: no budget is ever
    /// evicted to make room, and admitting it without one would be an unlimited
    /// guessing path. Correct passwords still pass, so whoever fills the table
    /// with pastes of their own cannot lock others out. The refusal's
    /// `Retry-After` is the time one token takes to refill (6 s at the
    /// default), not a full refill: that is when a tracked paste would admit
    /// the next attempt. A slot in the table may free up only later.
    pub fn password_attempt(&self, paste: PasteId, failed: bool) -> Result<(), u64> {
        let Some(spec) = self.inner.config.spec(BucketKind::InvalidCapabilityPerPaste) else {
            return Ok(());
        };
        let now = (self.inner.now)();
        let (result, warning) = {
            let mut pastes = lock(&self.inner.pastes);
            let slot = match pastes.find(&paste) {
                Some(slot) => Some(slot),
                None if !failed => return Ok(()),
                None => pastes.insert(paste, spec, now),
            };
            let result = match slot {
                Some(slot) => {
                    let bucket = pastes.bucket(slot);
                    bucket.check(spec, now).map(|()| {
                        if failed {
                            bucket.tokens -= 1.0;
                        }
                    })
                }
                None => Err(spec.seconds_to_refill(1.0)),
            };
            (result, pastes.take_warning())
        };
        if let Some(warning) = warning {
            warning.log();
        }
        result
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
        && let Err(retry_after) = state.with_bucket(BucketKind::InvalidCapability, ip, |bucket, spec, now| bucket.check(spec, now))
    {
        return too_many_requests(&state, &request_id, retry_after);
    }

    if let Err(retry_after) = state.with_bucket(BucketKind::Requests(class), ip, |bucket, spec, now| {
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
            if let Err(retry_after) = state.with_bucket(BucketKind::CreateBytes, ip, |bucket, spec, now| {
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
        let _ = state.with_bucket(BucketKind::InvalidCapability, ip, charge_one);
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
    use crate::domain::ID_BYTES;

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

    /// A monotonic clock the test moves by hand.
    #[derive(Clone)]
    struct FakeClock(Arc<Mutex<Instant>>);

    impl FakeClock {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Instant::now())))
        }

        fn advance(&self, by: Duration) {
            *lock(&self.0) += by;
        }

        fn source(&self) -> Now {
            let at = Arc::clone(&self.0);
            Arc::new(move || *lock(&at))
        }
    }

    fn bounded(config: RateLimitConfig, client_ceiling: usize, paste_ceiling: usize) -> (LimiterState, FakeClock) {
        let clock = FakeClock::new();
        let state = LimiterState::bounded_for_tests(config, client_ceiling, paste_ceiling, clock.source());
        (state, clock)
    }

    const DESCRIBE: BucketKind = BucketKind::Requests(EndpointClass::Descriptor);

    fn describe_limit(limit: RateLimit) -> RateLimitConfig {
        RateLimitConfig {
            descriptor: limit,
            ..RateLimitConfig::default()
        }
    }

    fn ipv4(n: u32) -> IpAddr {
        IpAddr::from(n.to_be_bytes())
    }

    /// `2001:db8::<n>`.
    fn ipv6(n: u32) -> IpAddr {
        IpAddr::from(std::net::Ipv6Addr::from((0x2001_0db8_u128 << 96) | u128::from(n)))
    }

    fn paste(n: u32) -> PasteId {
        let mut bytes = [0x5a; ID_BYTES];
        bytes[..4].copy_from_slice(&n.to_be_bytes());
        PasteId::from_bytes(bytes)
    }

    /// One descriptor request from `ip`.
    fn request(state: &LimiterState, ip: IpAddr) -> Result<(), u64> {
        state.with_bucket(DESCRIBE, ip, |bucket, spec, now| bucket.try_consume(spec, 1.0, now))
    }

    /// Entries in the client and the paste table.
    fn sizes(state: &LimiterState) -> (usize, usize) {
        (lock(&state.inner.clients).entries.len(), lock(&state.inner.pastes).entries.len())
    }

    /// Entries visited by the last admission to the client and the paste table.
    fn visits(state: &LimiterState) -> (usize, usize) {
        (lock(&state.inner.clients).visits, lock(&state.inner.pastes).visits)
    }

    #[test]
    fn client_buckets_are_capped() {
        assert_eq!((CLIENT_CEILING, PASTE_CEILING), (100_000, 100_000));
        let (state, _clock) = bounded(describe_limit(limit(60, 0, 5)), 100, 100);
        for n in 0..5_000 {
            for ip in [ipv4(n), ipv6(n)] {
                // Each client leaves its bucket partly used, so none is reclaimable.
                assert_eq!(request(&state, ip), Ok(()));
                assert!(sizes(&state).0 <= 100, "the client table outgrew its ceiling");
            }
        }
        assert_eq!(sizes(&state).0, 100);
    }

    /// The most entries one admission visits, on tables filled to `ceiling`
    /// with buckets that are not full - for new keys and for existing ones.
    fn most_visits_at_ceiling(ceiling: usize) -> usize {
        let (state, _clock) = bounded(describe_limit(limit(60, 0, 5)), ceiling, ceiling);
        let n = u32::try_from(ceiling).unwrap();
        for i in 0..n {
            assert_eq!(request(&state, ipv4(i)), Ok(()));
            assert_eq!(state.password_attempt(paste(i), true), Ok(()));
        }
        assert_eq!(sizes(&state), (ceiling, ceiling));

        let mut most = 0;
        // Fifty new keys (each evicts a client, each paste is refused), then
        // existing ones: from the middle, the newest of the fill, the newest.
        for i in (n..n + 50).chain([n / 2, n - 1, n + 49]) {
            let _ = request(&state, ipv4(i));
            let (clients, _) = visits(&state);
            let _ = state.password_attempt(paste(i), true);
            let (_, pastes) = visits(&state);
            most = most.max(clients).max(pastes);
        }
        most
    }

    #[test]
    fn admission_work_is_bounded() {
        let small = most_visits_at_ceiling(1_000);
        assert!(small <= 16, "one admission visited {small} entries");
        assert_eq!(most_visits_at_ceiling(10_000), small, "the work grew with the ceiling");
    }

    #[test]
    fn saturation_evicts_the_least_recently_used_client() {
        let (state, _clock) = bounded(describe_limit(limit(60, 0, 1)), 10, 10);
        for n in 0..10 {
            assert_eq!(request(&state, ipv4(n)), Ok(()));
        }
        // Client 0 comes back just before the table overflows: it is refused,
        // and now the most recently used client.
        assert_eq!(request(&state, ipv4(0)), Err(1));

        assert_eq!(request(&state, ipv4(100)), Ok(()));
        assert_eq!(request(&state, ipv4(0)), Err(1), "a recently used client lost its bucket");
        // Client 1 was the least recently used one: it starts afresh.
        assert_eq!(request(&state, ipv4(1)), Ok(()), "the least recently used client was kept");
        assert_eq!(sizes(&state).0, 10);
    }

    #[test]
    fn password_budgets_survive_saturation() {
        // The default per-paste limit: 10 a minute, burst 10.
        let (state, clock) = bounded(describe_limit(limit(60, 0, 1)), 50, 50);
        let target = paste(u32::MAX);
        for _ in 0..10 {
            assert_eq!(state.password_attempt(target, true), Ok(()));
        }
        assert!(state.password_attempt(target, true).is_err());

        for n in 0..5_000 {
            let _ = request(&state, ipv4(n));
            let _ = request(&state, ipv6(n));
            let _ = state.password_attempt(paste(n), true);
        }
        assert_eq!(sizes(&state), (50, 50));

        let Err(wait) = state.password_attempt(target, true) else {
            panic!("a wrong password passed after the tables were flooded");
        };
        assert!(wait > 0);
        assert!(
            state.password_attempt(target, false).is_err(),
            "a correct password passed a used-up budget"
        );
        clock.advance(Duration::from_secs(wait));
        assert_eq!(state.password_attempt(target, false), Ok(()));
        assert_eq!(state.password_attempt(target, true), Ok(()));
    }

    #[test]
    fn untracked_pastes_refuse_wrong_passwords_when_saturated() {
        let (state, _clock) = bounded(RateLimitConfig::default(), 10, 10);
        for n in 0..10 {
            assert_eq!(state.password_attempt(paste(n), true), Ok(()));
        }
        let untracked = paste(1_000);

        assert_eq!(state.password_attempt(untracked, false), Ok(()));
        assert!(
            state.password_attempt(untracked, true).is_err_and(|wait| wait > 0),
            "a wrong password on an untracked paste passed a full table"
        );
        assert_eq!(sizes(&state), (0, 10));
        assert!(!lock(&state.inner.pastes).slots.contains_key(&untracked));
    }

    #[test]
    fn full_buckets_are_reclaimed_first() {
        let (state, clock) = bounded(describe_limit(limit(60, 0, 5)), 100, 100);
        for n in 0..100 {
            assert_eq!(request(&state, ipv4(n)), Ok(()));
            assert_eq!(state.password_attempt(paste(n), true), Ok(()));
        }
        assert_eq!(sizes(&state), (100, 100));

        // Every bucket refills: one request a second, one password every six.
        clock.advance(Duration::from_secs(60));
        let fresh = 1_000..1_050;
        for n in fresh.clone() {
            assert_eq!(request(&state, ipv4(n)), Ok(()));
            // At the ceiling, this would be refused without a reclaimed slot.
            assert_eq!(state.password_attempt(paste(n), true), Ok(()));
            let (clients, pastes) = visits(&state);
            assert!(clients <= 16 && pastes <= 16, "an admission visited {clients}/{pastes} entries");
        }

        // No partly used bucket was evicted, and the full ones are gone.
        let clients = lock(&state.inner.clients);
        assert!(fresh.clone().all(|n| clients.slots.contains_key(&(DESCRIBE, ipv4(n)))));
        assert_eq!(clients.entries.len(), fresh.len());
        let pastes = lock(&state.inner.pastes);
        assert!(fresh.clone().all(|n| pastes.slots.contains_key(&paste(n))));
        assert_eq!(pastes.entries.len(), fresh.len());
    }

    /// A `tracing` writer that collects everything into a buffer.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn text(&self) -> String {
            String::from_utf8_lossy(&lock(&self.0)).into_owned()
        }

        fn warnings(&self, table: &str) -> usize {
            let field = format!("table=\"{table}\"");
            self.text()
                .lines()
                .filter(|line| line.contains("WARN") && line.contains(&field))
                .count()
        }
    }

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            lock(&self.0).extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn saturation_warns_once_a_minute_without_identifiers() {
        // An hourly refill: no bucket becomes full while the clock moves a minute.
        let config = RateLimitConfig {
            invalid_capability_per_paste: limit(0, 1, 5),
            ..describe_limit(limit(0, 1, 5))
        };
        let (state, clock) = bounded(config.clone(), 5, 5);
        let ids = |n: u32| paste(0xc0ff_ee00 + n);
        let flood = |state: &LimiterState, round: u32| {
            for n in round * 20..round * 20 + 20 {
                let _ = request(state, IpAddr::from([203, 0, 113, u8::try_from(n).unwrap()]));
                let _ = request(state, ipv6(n));
                let _ = state.password_attempt(ids(n), true);
            }
        };
        let counts = |capture: &Capture| (capture.warnings("client"), capture.warnings("paste"));
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt().with_writer(capture.clone()).with_ansi(false).finish();
        tracing::subscriber::with_default(subscriber, || {
            // `tracing` caches per call site whether anyone listens. Another
            // test that saturates a table first, before this subscriber
            // existed, may have cached "nobody": register both warnings here,
            // then rebuild the cache with this subscriber in place.
            flood(&bounded(config, 1, 1).0, 0);
            tracing::callsite::rebuild_interest_cache();
            lock(&capture.0).clear();

            flood(&state, 0);
            assert_eq!(counts(&capture), (1, 1));
            clock.advance(Duration::from_secs(59));
            flood(&state, 1);
            assert_eq!(counts(&capture), (1, 1));
            clock.advance(Duration::from_secs(1));
            flood(&state, 2);
            assert_eq!(counts(&capture), (2, 2));
        });

        let text = capture.text();
        assert!(text.contains("ceiling=5"), "the warning does not name the ceiling: {text}");
        assert!(
            !text.contains("203.0.113") && !text.contains("2001:db8"),
            "an address was logged: {text}"
        );
        for n in 0..60 {
            let id = ids(n);
            assert!(
                !text.contains(&id.encode()) && !text.contains(&id.prefix()),
                "a paste id was logged: {text}"
            );
        }
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
