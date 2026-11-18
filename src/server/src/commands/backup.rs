//! `capsulebin backup` and `capsulebin restore`.

use std::path::PathBuf;

use anyhow::Context as _;
use capsulebin_server::backup::{self, BackupInfo};
use capsulebin_server::config::Resolved;
use capsulebin_server::db;
use capsulebin_server::domain::{Clock as _, SystemClock};

fn describe(info: &BackupInfo) -> String {
    format!(
        "schema version {}, {} paste row(s), {} report(s), {} bytes",
        info.schema_version, info.pastes, info.reports, info.bytes
    )
}

/// Writes a consistent snapshot of the configured database. Safe to run while
/// the server is serving.
pub(crate) async fn backup(resolved: Resolved, output: PathBuf) -> anyhow::Result<()> {
    let state = super::state_for(&resolved).await?;
    let info = backup::create(&state.pool, &output)
        .await
        .with_context(|| format!("could not write the backup to {}", output.display()))?;

    println!("ok: backup written to {} ({})", output.display(), describe(&info));
    println!(
        "    It contains ciphertext, verifiers and abuse reports with contact details - store it as \
         carefully as the database itself."
    );
    Ok(())
}

/// Puts a verified backup in place of the configured database. The server must
/// be stopped.
pub(crate) async fn restore(resolved: Resolved, input: PathBuf, force: bool, dry_run: bool) -> anyhow::Result<()> {
    let target = resolved.config.database.path.clone();

    if dry_run {
        let info = backup::verify_file(&input)
            .await
            .with_context(|| format!("{} is not a usable backup", input.display()))?;
        println!("ok: {} is a usable backup ({})", input.display(), describe(&info));
        return Ok(());
    }

    let outcome = backup::restore(&input, &target, force, SystemClock.now())
        .await
        .map_err(|error| match error {
            backup::BackupError::Exists(path) if path == target => anyhow::anyhow!(
                "{} already exists; stop the server and pass --force to replace it (the current database is kept as \
                 <file>.pre-restore-<timestamp>)",
                path.display()
            ),
            other => anyhow::Error::new(other).context(format!("could not restore {}", input.display())),
        })?;

    // Open it the way `serve` does: applies migrations an older backup lacks,
    // then checks the schema.
    let pool = db::connect(&resolved.config.database)
        .await
        .context("the restored database cannot be opened")?;
    let version = db::verify(&pool).await.context("the restored database is not usable")?;
    pool.close().await;

    println!(
        "ok: restored {} to {} ({})",
        input.display(),
        target.display(),
        describe(&outcome.info)
    );
    if let Some(previous) = outcome.previous {
        println!("    The previous database was moved to {}.", previous.display());
    }
    if version != outcome.info.schema_version {
        println!("    Migrated from schema version {} to {version}.", outcome.info.schema_version);
    }
    Ok(())
}
