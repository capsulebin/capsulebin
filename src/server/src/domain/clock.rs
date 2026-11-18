//! Time as an injected dependency.
//!
//! Every lifecycle rule in this project is a comparison against "now": expiry,
//! retention, rate limit windows. Reading the system clock directly inside those
//! rules would make them testable only by sleeping, and a test that sleeps for a
//! 90-day expiry is a test nobody writes.
//!
//! So time enters through this trait. Production uses [`SystemClock`]; tests use
//! [`FixedClock`] and move time by hand.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

/// Source of the current time, as UTC Unix seconds.
///
/// Seconds rather than a richer type because that is what the database stores;
/// converting at the boundary would just move the truncation somewhere less
/// visible.
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        // A clock before 1970 would mean the host is badly misconfigured. Falling
        // back to 0 keeps this infallible, and every expiry then reads as long
        // past, which fails closed rather than handing out unbounded lifetimes.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// A clock the caller controls. Test support, but not `#[cfg(test)]`: the
/// integration tests live in another crate and need it too.
#[derive(Debug, Clone)]
pub struct FixedClock(Arc<AtomicI64>);

impl FixedClock {
    pub fn at(seconds: i64) -> Self {
        Self(Arc::new(AtomicI64::new(seconds)))
    }

    pub fn advance(&self, seconds: i64) {
        self.0.fetch_add(seconds, Ordering::SeqCst);
    }

    pub fn set(&self, seconds: i64) {
        self.0.store(seconds, Ordering::SeqCst);
    }
}

impl Clock for FixedClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_plausible() {
        // 2020-01-01, chosen as a lower bound that will not age badly.
        assert!(SystemClock.now() > 1_577_836_800);
    }

    #[test]
    fn fixed_clock_moves_only_when_told() {
        let clock = FixedClock::at(1_000);
        assert_eq!(clock.now(), 1_000);
        assert_eq!(clock.now(), 1_000, "reading must not advance time");

        clock.advance(60);
        assert_eq!(clock.now(), 1_060);

        clock.set(5);
        assert_eq!(clock.now(), 5);
    }

    /// Clones share the same instant, so a clock handed to the repository and one
    /// kept by the test stay in step.
    #[test]
    fn clones_share_state() {
        let clock = FixedClock::at(0);
        let handle = clock.clone();
        clock.advance(10);
        assert_eq!(handle.now(), 10);
    }
}
