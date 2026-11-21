//! `capsulebin token`.

use std::path::Path;

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use capsulebin_server::config::{Config, ServerConfig};
use capsulebin_server::domain::Verifier;

/// Generates a creation token and prints it together with its configured hash,
/// followed by an invitation link when `server.public_url` is known.
///
/// The token is printed once and not stored anywhere. The hash is what goes into
/// the configuration file, which therefore never has to hold the secret.
pub(crate) fn token(config: Option<&Path>) -> anyhow::Result<()> {
    // Before the token: an explicit `--config` that cannot be loaded is an
    // error, and no token is printed for it.
    let public_url = public_url(config)?;

    // 32 bytes straight from the OS. No PRNG in between, so there is no seeding
    // question to get wrong.
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).context("could not read random bytes from the operating system")?;

    let token = URL_SAFE_NO_PAD.encode(bytes);
    let hash = Verifier::for_creation_token(token.as_bytes()).encode();

    // Line 2 is the token and the `sha256:` line follows: scripts (and
    // `scripts/smoke.sh`) parse them, so anything new goes after them.
    println!("token (store this in your client, it is not recoverable):");
    println!("  {token}");
    println!();
    println!("configuration entry:");
    println!("  [creation]");
    println!("  mode = \"token\"");
    println!("  token_hashes = [\"sha256:{hash}\"]");

    if let Some(public_url) = public_url {
        // The token sits in the fragment, which browsers never send.
        println!();
        println!("invitation link (opens the create page and stores the token in the browser):");
        println!("  {public_url}/#token={token}");
    }

    Ok(())
}

/// `server.public_url` from `--config` or the environment, if it is set and
/// passes the rules `serve` applies to it. Nothing else in the configuration is
/// validated: `token` is run before the instance is configured.
///
/// A configuration that cannot be loaded is an error with `--config`. Without
/// it, and for a `public_url` that breaks a rule, the token is still printed,
/// without the link, after a note on stderr that never repeats the value.
fn public_url(path: Option<&Path>) -> anyhow::Result<Option<String>> {
    let config = match Config::load(path) {
        Ok(config) => config,
        Err(error) if path.is_some() => return Err(error).context("could not load the configuration"),
        Err(error) => {
            eprintln!("note: no invitation link: could not load the configuration: {error}");
            return Ok(None);
        }
    };

    let Some(public_url) = config.server.public_url else {
        return Ok(None);
    };
    match ServerConfig::check_public_url(&public_url) {
        Ok(()) => Ok(Some(public_url)),
        Err(error) => {
            eprintln!("note: no invitation link: {error}");
            Ok(None)
        }
    }
}
