//! `capsulebin prune`.

use anyhow::Context as _;
use capsulebin_server::config::Resolved;
use capsulebin_server::domain::PurgeOutcome;
use capsulebin_server::repository::{AbuseRepository as _, PasteRepository as _};

/// Applies the retention policy once and exits, including the retention for
/// closed abuse reports and moderation records.
///
/// Runs the same rules as the scheduled job, so it doubles as the way to force a
/// purge after changing the thresholds.
pub(crate) async fn prune(resolved: Resolved, batch_size: Option<i64>) -> anyhow::Result<()> {
    let state = super::state_for(&resolved).await?;
    let now = state.clock.now();
    let policy = resolved.config.purge.policy();
    let batch_size = batch_size.unwrap_or(resolved.config.purge.batch_size);
    anyhow::ensure!(batch_size >= 1, "--batch-size must be at least 1");

    // Batching until a run removes nothing, rather than doing one batch and
    // stopping: a single batch would leave the operator to guess how many times to
    // repeat the command after a long outage.
    //
    // The upper bound is a safety net, not a policy. Without it a bug that makes a
    // rule match rows it cannot delete would spin here forever.
    const MAX_ROUNDS: u32 = 10_000;
    let mut total = PurgeOutcome::default();
    let mut rounds = 0;

    loop {
        let outcome = state
            .pastes_repo
            .purge(now, &policy, batch_size)
            .await
            .context("could not apply the retention policy")?;

        if outcome.is_empty() {
            break;
        }

        total.add(outcome);
        rounds += 1;

        if rounds >= MAX_ROUNDS {
            anyhow::bail!("purge did not converge after {MAX_ROUNDS} rounds; stopping to avoid an endless loop");
        }
    }

    println!(
        "ok: {} capsule(s) removed ({} expired, {} never retrieved, {} inactive)",
        total.total(),
        total.expired,
        total.never_retrieved,
        total.inactive
    );

    let cutoff = now.saturating_sub(resolved.config.abuse.report_retention.as_secs_i64());
    let abuse = state
        .abuse_repo
        .purge_abuse(cutoff)
        .await
        .context("could not apply the report retention")?;
    println!(
        "ok: {} closed report(s) and {} moderation record(s) past abuse.report_retention removed",
        abuse.reports, abuse.actions
    );
    Ok(())
}
