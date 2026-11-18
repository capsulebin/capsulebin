//! Retention rules for the purge job.
//!
//! Three rules, evaluated independently, each removing whole rows:
//!
//! 1. **Expired.** An explicit expiry always wins. It is the creator's own
//!    statement about how long the capsule should live, so it is applied
//!    regardless of how recently the capsule was used.
//! 2. **Never retrieved.** A capsule nobody ever fetched is measured from
//!    `created_at`. Most of these are mistakes or abandoned drafts.
//! 3. **Inactive.** A capsule that was fetched but not for a long time is measured
//!    from `last_retrieved_at`.
//!
//! A capsule without an expiry is *not* exempt from rules 2 and 3. "No fixed
//! expiry" means "no deadline the creator chose", not "kept forever" — an
//! accountless service that accumulates content indefinitely becomes a moderation
//! and storage problem.
//!
//! Consumed burn capsules need no rule of their own: the burn transaction sets
//! `last_retrieved_at`, so rule 3 already covers them.

/// Retention thresholds in seconds, resolved from the configuration once at
/// startup so that no purge run has to re-parse a duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PurgePolicy {
    /// Age after `created_at` at which a never-retrieved capsule is removed.
    pub never_retrieved_after: i64,
    /// Age after `last_retrieved_at` at which a retrieved capsule is removed.
    pub inactive_after: i64,
}

/// What one purge run removed, split by rule.
///
/// Reported separately because the aggregate hides the interesting case: a
/// deployment where the inactivity rule removes far more than expected has its
/// thresholds wrong, and a single total would not show that.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PurgeOutcome {
    pub expired: u64,
    pub never_retrieved: u64,
    pub inactive: u64,
}

impl PurgeOutcome {
    pub fn total(self) -> u64 {
        self.expired + self.never_retrieved + self.inactive
    }

    /// Whether the run removed nothing, which is the signal to stop batching.
    pub fn is_empty(self) -> bool {
        self.total() == 0
    }

    pub fn add(&mut self, other: Self) {
        self.expired += other.expired;
        self.never_retrieved += other.never_retrieved;
        self.inactive += other.inactive;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_accumulate_per_rule() {
        let mut total = PurgeOutcome::default();
        assert!(total.is_empty());

        total.add(PurgeOutcome {
            expired: 2,
            never_retrieved: 1,
            inactive: 0,
        });
        total.add(PurgeOutcome {
            expired: 1,
            never_retrieved: 0,
            inactive: 4,
        });

        assert_eq!(total.expired, 3);
        assert_eq!(total.never_retrieved, 1);
        assert_eq!(total.inactive, 4);
        assert_eq!(total.total(), 8);
        assert!(!total.is_empty());
    }
}
