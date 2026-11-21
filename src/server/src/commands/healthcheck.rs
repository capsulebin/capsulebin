//! `capsulebin healthcheck`.

use std::time::Duration;

use capsulebin_server::healthcheck::probe;

/// Below the usual container health check timeout, so a hanging server is
/// reported by this command rather than killed by the runtime.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Probes `address` (`host:port`). Takes the address rather than a validated
/// configuration: the probe needs nothing else, and validating would fail on
/// settings the probe never uses (`public_url`, a writable database directory).
pub(crate) async fn healthcheck(address: &str, live: bool) -> anyhow::Result<()> {
    let path = if live { "/health/live" } else { "/health/ready" };

    probe(address, path, TIMEOUT).await?;

    println!("{path} ok");
    Ok(())
}
