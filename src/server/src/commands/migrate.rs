//! `capsulebin migrate`.

use anyhow::Context as _;
use capsulebin_server::config::Resolved;
use capsulebin_server::db;

pub(crate) async fn migrate(resolved: Resolved) -> anyhow::Result<()> {
    // `connect` applies pending migrations, so opening the pool *is* the migration.
    let pool = db::connect(&resolved.config.database)
        .await
        .context("could not migrate the database")?;

    let version = db::verify(&pool).await.context("schema check failed")?;
    println!("ok: schema at version {version}");
    Ok(())
}
