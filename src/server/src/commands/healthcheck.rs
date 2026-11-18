//! `capsulebin healthcheck`.

use std::time::Duration;

use capsulebin_server::config::Resolved;
use capsulebin_server::healthcheck::{probe, probe_address};

/// Below the usual container health check timeout, so a hanging server is
/// reported by this command rather than killed by the runtime.
const TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn healthcheck(resolved: &Resolved, live: bool, address: Option<String>) -> anyhow::Result<()> {
    let address = address.unwrap_or_else(|| probe_address(&resolved.config.server.bind));
    let path = if live { "/health/live" } else { "/health/ready" };

    probe(&address, path, TIMEOUT).await?;

    println!("{path} ok");
    Ok(())
}
