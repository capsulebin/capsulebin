//! `capsulebin serve`.

use std::time::Duration;

use anyhow::Context as _;
use capsulebin_server::config::Resolved;
use capsulebin_server::domain::{Clock as _, PurgeOutcome, SystemClock};
use capsulebin_server::repository::sqlite::SqlitePasteRepository;
use capsulebin_server::repository::sqlite::abuse::SqliteAbuseRepository;
use capsulebin_server::repository::{AbuseRepository as _, PasteRepository as _};
use capsulebin_server::router;
use tokio::sync::watch;

pub(crate) async fn serve(resolved: Resolved, bind_override: Option<String>) -> anyhow::Result<()> {
    let state = super::state_for(&resolved).await?;

    if !state.assets_ready() {
        tracing::warn!(
            "frontend assets are missing - run `npm run build` in src/server/web/. \
             The server starts, but /health/ready stays degraded."
        );
    }

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // Daily purge job. Runs immediately on start, then every 24 hours. It also
    // applies `abuse.report_retention`, which is personal-data retention and
    // therefore not something `purge.enabled = false` should switch off.
    {
        let enabled = state.purge.enabled;
        let repo = state.pastes_repo.clone();
        let abuse_repo = state.abuse_repo.clone();
        let report_retention = state.abuse.report_retention.as_secs_i64();
        let purge_config = state.purge.clone();
        let mut rx = shutdown_rx.clone();

        tokio::spawn(async move {
            let policy = purge_config.policy();
            let batch_size = purge_config.batch_size;
            let clock = SystemClock;

            loop {
                if enabled {
                    purge_run(&repo, &policy, batch_size, &clock).await;
                }
                abuse_retention_run(&abuse_repo, report_retention, &clock).await;

                let sleep = tokio::time::sleep(Duration::from_secs(86_400));
                tokio::pin!(sleep);
                tokio::select! {
                    () = &mut sleep => {},
                    _ = rx.changed() => {
                        if *rx.borrow() { break; }
                    },
                }
            }
        });
    }

    let bind = bind_override.unwrap_or_else(|| resolved.config.server.bind.clone());
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .with_context(|| format!("could not bind to {bind}"))?;

    let local = listener.local_addr().context("could not determine the local address")?;
    tracing::info!(%local, mode = ?resolved.config.creation.mode, "CapsuleBin is listening");

    // With connect info: the client IP (and with it every rate limit) comes from
    // the TCP peer address. Without it, all clients would share one bucket.
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal(shutdown_tx))
    .await
    .context("server aborted")
}

async fn purge_run(repo: &SqlitePasteRepository, policy: &capsulebin_server::domain::PurgePolicy, batch_size: i64, clock: &SystemClock) {
    let start = std::time::Instant::now();
    let now = clock.now();

    const MAX_ROUNDS: u32 = 10_000;
    let mut total = PurgeOutcome::default();
    let mut rounds = 0;

    loop {
        match repo.purge(now, policy, batch_size).await {
            Ok(outcome) => {
                if outcome.is_empty() {
                    break;
                }
                total.add(outcome);
                rounds += 1;

                if rounds >= MAX_ROUNDS {
                    tracing::error!("purge did not converge after {MAX_ROUNDS} rounds; stopping to avoid an endless loop");
                    break;
                }
            }
            Err(error) => {
                tracing::error!(%error, "purge run failed");
                break;
            }
        }
    }

    let elapsed = start.elapsed();
    tracing::info!(
        total = total.total(),
        expired = total.expired,
        never_retrieved = total.never_retrieved,
        inactive = total.inactive,
        rounds,
        duration_ms = elapsed.as_millis(),
        "daily purge complete"
    );
}

async fn abuse_retention_run(repo: &SqliteAbuseRepository, retention_secs: i64, clock: &SystemClock) {
    let cutoff = clock.now().saturating_sub(retention_secs);
    match repo.purge_abuse(cutoff).await {
        Ok(outcome) => tracing::info!(
            reports = outcome.reports,
            moderation_records = outcome.actions,
            "report retention applied"
        ),
        Err(error) => tracing::error!(%error, "report retention failed"),
    }
}

/// Graceful shutdown: stop accepting new requests, let running ones finish.
async fn shutdown_signal(tx: watch::Sender<bool>) {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(%error, "could not install the SIGTERM handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    let _ = tx.send(true);
    tracing::info!("shutdown requested - finishing in-flight requests");
}
