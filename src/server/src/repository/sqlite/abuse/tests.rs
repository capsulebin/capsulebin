#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Reports and moderation against a real SQLite file.

use super::*;
use crate::config::{DatabaseConfig, Synchronous};
use crate::domain::{NewPaste, Verifier};
use crate::repository::PasteRepository as _;
use crate::repository::SubmitOutcome::{CeilingReached, NoSuchPaste, Stored};
use crate::repository::sqlite::SqlitePasteRepository;

async fn repositories(name: &str) -> (SqlitePasteRepository, SqliteAbuseRepository) {
    let dir = std::env::temp_dir().join(format!("capsulebin-abuse-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let pool = crate::db::connect(&DatabaseConfig {
        path: dir.join("capsulebin.db"),
        busy_timeout_ms: 5_000,
        synchronous: Synchronous::Full,
    })
    .await
    .expect("connect");

    (SqlitePasteRepository::new(pool.clone()), SqliteAbuseRepository::new(pool))
}

/// The default ceilings, far above what any test here submits.
const ROOMY: ReportCeilings = ReportCeilings {
    per_paste: 10,
    total: 1_000,
};

fn id(byte: u8) -> PasteId {
    PasteId::from_bytes([byte; 16])
}

fn paste(id: PasteId, expires_at: Option<i64>, burn: bool) -> NewPaste {
    NewPaste {
        id,
        format_version: 1,
        public_header: vec![0xAA],
        ciphertext: vec![1, 2, 3],
        reveal_verifier: Verifier::for_reveal_token(b"reveal"),
        delete_verifier: Verifier::for_delete_secret(b"delete"),
        describe_verifier: Verifier::for_describe_token(b"describe"),
        created_at: 1_000,
        expires_at,
        burn_after_retrieval: burn,
    }
}

fn report(report_id: &str, paste_id: PasteId) -> NewReport {
    NewReport::validate(
        report_id.to_owned(),
        paste_id,
        "phishing",
        "fake login page",
        Some("me@example.org"),
        1_100,
    )
    .unwrap()
}

#[tokio::test]
async fn a_report_is_stored_only_for_an_existing_paste() {
    let (pastes, abuse) = repositories("submit").await;
    pastes.create(&paste(id(1), None, false)).await.unwrap();

    assert_eq!(abuse.submit_report(&report("r1", id(1)), ROOMY, 1_100).await.unwrap(), Stored);
    assert_eq!(
        abuse.submit_report(&report("r2", id(2)), ROOMY, 1_100).await.unwrap(),
        NoSuchPaste,
        "unknown id stored"
    );

    let all = abuse.list_reports(None, 100).await.unwrap();
    assert_eq!(all.len(), 1);
    let entry = &all[0];
    assert_eq!(entry.report.id, "r1");
    assert_eq!(entry.report.paste_id, id(1).encode());
    assert_eq!(entry.report.reason, "phishing");
    assert_eq!(entry.report.contact.as_deref(), Some("me@example.org"));
    assert_eq!(entry.report.status, "open");
    assert_eq!(entry.paste_status.as_deref(), Some("active"));
}

#[tokio::test]
async fn open_reports_are_capped_per_paste_and_globally() {
    let (pastes, abuse) = repositories("ceilings").await;
    for byte in 1..=4 {
        pastes.create(&paste(id(byte), None, false)).await.unwrap();
    }
    let ceilings = ReportCeilings { per_paste: 2, total: 3 };
    let submit = |report_id: &'static str, paste_id: PasteId| {
        let abuse = abuse.clone();
        async move { abuse.submit_report(&report(report_id, paste_id), ceilings, 1_100).await.unwrap() }
    };

    // Per paste: the third open report about one paste is dropped.
    assert_eq!(submit("a1", id(1)).await, Stored);
    assert_eq!(submit("a2", id(1)).await, Stored);
    assert_eq!(submit("a3", id(1)).await, CeilingReached);

    // Globally: other pastes take reports until three are open in total.
    assert_eq!(submit("b1", id(2)).await, Stored);
    assert_eq!(submit("c1", id(3)).await, CeilingReached);
    assert_eq!(submit("d1", id(4)).await, CeilingReached);

    // A report under review is still open.
    assert!(abuse.review_report("a1").await.unwrap());
    assert_eq!(submit("c2", id(3)).await, CeilingReached);
    assert_eq!(abuse.count_open_reports().await.unwrap(), 3);

    // Closing one frees exactly one place.
    assert!(abuse.close_report("b1", Resolution::NoAction, 1_200).await.unwrap());
    assert_eq!(abuse.count_open_reports().await.unwrap(), 2);
    assert_eq!(submit("c3", id(3)).await, Stored);
    assert_eq!(submit("d2", id(4)).await, CeilingReached);

    // An unknown id is still not stored, and says so.
    assert_eq!(submit("x1", id(9)).await, NoSuchPaste);

    let mut stored: Vec<_> = abuse
        .list_reports(None, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.report.id)
        .collect();
    stored.sort();
    assert_eq!(stored, ["a1", "a2", "b1", "c3"]);
    assert_eq!(abuse.count_open_reports().await.unwrap(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_report_ceiling_holds_under_concurrency() {
    let (pastes, abuse) = repositories("ceilings-concurrent").await;
    pastes.create(&paste(id(1), None, false)).await.unwrap();
    let ceilings = ReportCeilings {
        per_paste: 3,
        total: 1_000,
    };

    let tasks: Vec<_> = (0..64)
        .map(|n| {
            let abuse = abuse.clone();
            tokio::spawn(async move { abuse.submit_report(&report(&format!("r{n:02}"), id(1)), ceilings, 1_100).await })
        })
        .collect();
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.unwrap().unwrap());
    }

    assert_eq!(outcomes.iter().filter(|outcome| **outcome == Stored).count(), 3);
    assert_eq!(outcomes.iter().filter(|outcome| **outcome == CeilingReached).count(), 61);
    assert_eq!(abuse.list_reports(None, 100).await.unwrap().len(), 3);
}

#[tokio::test]
async fn expired_consumed_and_deleted_pastes_take_no_reports() {
    let (pastes, abuse) = repositories("submit-gone").await;

    pastes.create(&paste(id(1), Some(1_050), false)).await.unwrap();
    assert_eq!(
        abuse.submit_report(&report("expired", id(1)), ROOMY, 1_100).await.unwrap(),
        NoSuchPaste
    );

    pastes.create(&paste(id(2), None, true)).await.unwrap();
    pastes.retrieve_and_burn(&id(2), 1_050).await.unwrap().expect("burn");
    assert_eq!(
        abuse.submit_report(&report("consumed", id(2)), ROOMY, 1_100).await.unwrap(),
        NoSuchPaste
    );

    pastes.create(&paste(id(3), None, false)).await.unwrap();
    pastes.delete(&id(3)).await.unwrap();
    assert_eq!(
        abuse.submit_report(&report("deleted", id(3)), ROOMY, 1_100).await.unwrap(),
        NoSuchPaste
    );

    // A disabled paste still takes reports: it may need deleting.
    pastes.create(&paste(id(4), None, false)).await.unwrap();
    abuse.moderate(&id(4), ModerationAction::Disable, None, None, 1_050).await.unwrap();
    assert_eq!(abuse.submit_report(&report("disabled", id(4)), ROOMY, 1_100).await.unwrap(), Stored);
}

#[tokio::test]
async fn reports_move_through_review_to_closed() {
    let (pastes, abuse) = repositories("lifecycle").await;
    pastes.create(&paste(id(1), None, false)).await.unwrap();
    abuse.submit_report(&report("r1", id(1)), ROOMY, 1_100).await.unwrap();
    abuse.submit_report(&report("r2", id(1)), ROOMY, 1_101).await.unwrap();

    assert!(abuse.review_report("r1").await.unwrap());
    assert!(!abuse.review_report("r1").await.unwrap(), "reviewing twice");

    assert!(abuse.close_report("r1", Resolution::ActionTaken, 1_200).await.unwrap());
    assert!(
        !abuse.close_report("r1", Resolution::NoAction, 1_300).await.unwrap(),
        "closed twice"
    );
    assert!(!abuse.close_report("missing", Resolution::NoAction, 1_300).await.unwrap());

    let closed = abuse.find_report("r1").await.unwrap().unwrap().report;
    assert_eq!(closed.status, "closed");
    assert_eq!(closed.handled_at, Some(1_200));
    assert_eq!(closed.resolution.as_deref(), Some("action_taken"));

    let open = abuse.list_reports(Some(ReportStatus::Open), 100).await.unwrap();
    assert_eq!(open.iter().map(|e| e.report.id.as_str()).collect::<Vec<_>>(), ["r2"]);
}

#[tokio::test]
async fn disable_blocks_retrieval_and_restore_reverses_it() {
    let (pastes, abuse) = repositories("disable").await;
    pastes.create(&paste(id(1), None, false)).await.unwrap();

    let outcome = abuse
        .moderate(
            &id(1),
            ModerationAction::Disable,
            Some(ReasonCode::Malware),
            Some("ticket 42"),
            1_100,
        )
        .await
        .unwrap();
    assert_eq!(outcome, ModerationOutcome::Applied);

    let disabled = pastes.find(&id(1)).await.unwrap().unwrap();
    assert_eq!(disabled.availability, Availability::Disabled);
    assert!(!disabled.is_retrievable(1_100));
    assert!(disabled.ciphertext.is_some(), "disable must keep the payload");

    assert_eq!(
        abuse.moderate(&id(1), ModerationAction::Disable, None, None, 1_150).await.unwrap(),
        ModerationOutcome::NotApplicable(Availability::Disabled)
    );

    assert_eq!(
        abuse.moderate(&id(1), ModerationAction::Restore, None, None, 1_200).await.unwrap(),
        ModerationOutcome::Applied
    );
    assert!(pastes.find(&id(1)).await.unwrap().unwrap().is_retrievable(1_200));

    let history = abuse.moderation_history(&id(1)).await.unwrap();
    let actions: Vec<_> = history.iter().map(|r| r.action.as_str()).collect();
    assert_eq!(actions, ["disable", "restore"]);
    assert_eq!(history[0].reason.as_deref(), Some("malware"));
    assert_eq!(history[0].note.as_deref(), Some("ticket 42"));
}

#[tokio::test]
async fn delete_drops_the_payload_and_cannot_be_restored() {
    let (pastes, abuse) = repositories("delete").await;
    pastes.create(&paste(id(1), None, false)).await.unwrap();
    abuse.moderate(&id(1), ModerationAction::Disable, None, None, 1_100).await.unwrap();

    assert_eq!(
        abuse
            .moderate(&id(1), ModerationAction::Delete, Some(ReasonCode::IllegalContent), None, 1_200)
            .await
            .unwrap(),
        ModerationOutcome::Applied
    );
    let deleted = pastes.find(&id(1)).await.unwrap().unwrap();
    assert_eq!(deleted.availability, Availability::Deleted);
    assert!(deleted.ciphertext.is_none());

    for action in [ModerationAction::Restore, ModerationAction::Disable, ModerationAction::Delete] {
        assert_eq!(
            abuse.moderate(&id(1), action, None, None, 1_300).await.unwrap(),
            ModerationOutcome::NotApplicable(Availability::Deleted),
            "{action} on a deleted paste"
        );
    }
}

#[tokio::test]
async fn restore_only_applies_to_disabled_and_unknown_ids_are_reported() {
    let (pastes, abuse) = repositories("restore").await;
    pastes.create(&paste(id(1), None, false)).await.unwrap();

    assert_eq!(
        abuse.moderate(&id(1), ModerationAction::Restore, None, None, 1_100).await.unwrap(),
        ModerationOutcome::NotApplicable(Availability::Active)
    );
    assert_eq!(
        abuse.moderate(&id(9), ModerationAction::Disable, None, None, 1_100).await.unwrap(),
        ModerationOutcome::NotFound
    );
    assert!(
        abuse.moderation_history(&id(1)).await.unwrap().is_empty(),
        "a refused action was recorded"
    );
}

#[tokio::test]
async fn retention_removes_closed_reports_and_old_audit_entries_only() {
    let (pastes, abuse) = repositories("purge").await;
    pastes.create(&paste(id(1), None, false)).await.unwrap();
    abuse.submit_report(&report("old-closed", id(1)), ROOMY, 1_100).await.unwrap();
    abuse.submit_report(&report("old-open", id(1)), ROOMY, 1_100).await.unwrap();
    abuse.submit_report(&report("new-closed", id(1)), ROOMY, 1_100).await.unwrap();
    abuse.close_report("old-closed", Resolution::NoAction, 2_000).await.unwrap();
    abuse.close_report("new-closed", Resolution::NoAction, 9_000).await.unwrap();
    abuse.moderate(&id(1), ModerationAction::Disable, None, None, 2_000).await.unwrap();
    abuse.moderate(&id(1), ModerationAction::Restore, None, None, 9_000).await.unwrap();

    let outcome = abuse.purge_abuse(5_000).await.unwrap();
    assert_eq!(outcome, AbusePurgeOutcome { reports: 1, actions: 1 });

    // Same `created_at`, so the queue order falls back to the id.
    let left: Vec<_> = abuse
        .list_reports(None, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.report.id)
        .collect();
    assert_eq!(left, ["new-closed", "old-open"]);
    assert_eq!(abuse.moderation_history(&id(1)).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_report_outlives_its_purged_paste_with_no_status() {
    let (pastes, abuse) = repositories("orphan").await;
    pastes.create(&paste(id(1), Some(2_000), false)).await.unwrap();
    abuse.submit_report(&report("r1", id(1)), ROOMY, 1_100).await.unwrap();

    let policy = crate::domain::PurgePolicy {
        never_retrieved_after: 1_000_000,
        inactive_after: 1_000_000,
    };
    pastes.purge(3_000, &policy, 100).await.unwrap();

    let entry = abuse.find_report("r1").await.unwrap().unwrap();
    assert_eq!(entry.paste_status, None);
}
