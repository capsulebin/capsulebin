//! `capsulebin token`.

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use capsulebin_server::domain::Verifier;

/// Generates a creation token and prints it together with its configured hash.
///
/// The token is printed once and not stored anywhere. The hash is what goes into
/// the configuration file, which therefore never has to hold the secret.
pub(crate) fn token() -> anyhow::Result<()> {
    // 32 bytes straight from the OS. No PRNG in between, so there is no seeding
    // question to get wrong.
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).context("could not read random bytes from the operating system")?;

    let token = URL_SAFE_NO_PAD.encode(bytes);
    let hash = Verifier::for_creation_token(token.as_bytes()).encode();

    println!("token (store this in your client, it is not recoverable):");
    println!("  {token}");
    println!();
    println!("configuration entry:");
    println!("  [creation]");
    println!("  mode = \"token\"");
    println!("  token_hashes = [\"sha256:{hash}\"]");

    Ok(())
}
