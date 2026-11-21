//! `capsulebin check`.

use anyhow::Context as _;
use capsulebin_server::config::Resolved;
use capsulebin_server::db;

/// Runs exactly the checks `/health/ready` runs, so a deployment can be verified
/// before a port is opened.
pub(crate) async fn check(resolved: Resolved) -> anyhow::Result<()> {
    let state = super::state_for(&resolved).await?;

    // Warnings, not failures: on stderr, so stdout stays the one `ok:` line.
    for warning in resolved.branding.colors.contrast_warnings() {
        eprintln!("warning: {warning}");
    }
    for warning in db::permission_warnings(&resolved.config.database) {
        eprintln!("warning: {warning}");
    }

    let version = db::verify(&state.pool).await.context("database is not ready")?;

    anyhow::ensure!(
        state.assets_ready(),
        "frontend assets are missing: run `npm run build` in src/server/web/ first"
    );

    println!("ok: configuration valid, schema at version {version}, assets present");
    Ok(())
}
