//! `capsulebin check`.

use anyhow::Context as _;
use capsulebin_server::config::Resolved;
use capsulebin_server::db;

/// Runs exactly the checks `/health/ready` runs, so a deployment can be verified
/// before a port is opened.
pub(crate) async fn check(resolved: Resolved) -> anyhow::Result<()> {
    let state = super::state_for(&resolved).await?;

    let version = db::verify(&state.pool).await.context("database is not ready")?;

    anyhow::ensure!(
        state.assets_ready(),
        "frontend assets are missing: run `npm run build` in src/server/web/ first"
    );

    println!("ok: configuration valid, schema at version {version}, assets present");
    Ok(())
}
