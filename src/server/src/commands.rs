//! The subcommands behind the `capsulebin` binary: `serve`, `check`,
//! `healthcheck`, `migrate`, `prune`, `token`, `backup`, `restore`, and the
//! operator's `reports` and `moderation`.

pub(crate) mod abuse;
mod backup;
mod check;
mod healthcheck;
mod migrate;
mod prune;
mod serve;
mod token;

pub(crate) use backup::{backup, restore};
pub(crate) use check::check;
pub(crate) use healthcheck::healthcheck;
pub(crate) use migrate::migrate;
pub(crate) use prune::prune;
pub(crate) use serve::serve;
pub(crate) use token::token;

use std::sync::Arc;

use anyhow::Context as _;
use capsulebin_server::config::Resolved;
use capsulebin_server::domain::SystemClock;
use capsulebin_server::{AppState, api::assets::AssetSource, db};

async fn state_for(resolved: &Resolved) -> anyhow::Result<AppState> {
    let pool = db::connect(&resolved.config.database)
        .await
        .context("could not open the database")?;

    let assets = match resolved.config.server.asset_dir.clone() {
        Some(dir) => {
            tracing::warn!(path = %dir.display(), "serving assets from a directory - development only");
            AssetSource::Directory(dir)
        }
        None => AssetSource::from_env(),
    };

    Ok(AppState::new(resolved, pool, assets, Arc::new(SystemClock)))
}
