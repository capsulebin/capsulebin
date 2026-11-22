//! `capsulebin token` and `capsulebin token create|list|revoke`.
//!
//! Plain `token` only generates: it runs before the instance is configured and
//! stores nothing. The subcommands keep tokens in the database (ADR-019), so
//! they need the same configuration as `serve` and can run while it is up; a
//! change applies from the server's next create request.
//!
//! Labels are printed as stored. Control characters are refused when a label
//! is parsed, on `create` and again when it is read back, so printing one to a
//! terminal cannot inject escape sequences.

use std::path::Path;

use anyhow::{Context as _, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use capsulebin_server::config::{Config, CreationMode, Resolved, ServerConfig};
use capsulebin_server::domain::{Clock as _, CreationToken, Label, SystemClock, Verifier};
use capsulebin_server::repository::{CreationTokenRepository as _, RevokeOutcome};

use super::abuse::{format_utc, format_utc_date};

/// Generates a creation token and prints it together with its configured hash,
/// followed by an invitation link when `server.public_url` is known.
///
/// The token is printed once and not stored anywhere. The hash is what goes into
/// the configuration file, which therefore never has to hold the secret.
pub(crate) fn token(config: Option<&Path>) -> anyhow::Result<()> {
    // Before the token: an explicit `--config` that cannot be loaded is an
    // error, and no token is printed for it.
    let public_url = public_url(config)?;

    let token = generate()?;
    let hash = Verifier::for_creation_token(token.as_bytes()).encode();

    // Line 2 is the token and the `sha256:` line follows: scripts (and
    // `scripts/smoke.sh`) parse them, so anything new goes after them.
    print_token(&token);
    println!();
    println!("configuration entry:");
    println!("  [creation]");
    println!("  mode = \"token\"");
    println!("  token_hashes = [\"sha256:{hash}\"]");

    if let Some(public_url) = public_url {
        println!();
        print_invitation(&public_url, &token);
    }

    Ok(())
}

/// A new creation token: 32 bytes straight from the OS, base64url. No PRNG in
/// between, so there is no seeding question to get wrong.
fn generate() -> anyhow::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).context("could not read random bytes from the operating system")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The first two lines of both `token` and `token create`: scripts take the
/// token from line 2.
fn print_token(token: &str) {
    println!("token (store this in your client, it is not recoverable):");
    println!("  {token}");
}

fn print_invitation(public_url: &str, token: &str) {
    // The token sits in the fragment, which browsers never send.
    println!("invitation link (opens the create page and stores the token in the browser):");
    println!("  {public_url}/#token={token}");
}

/// `token create`: stores a new token's verifier with `label` and prints the
/// token, its id and the invitation link. Neither the hash nor a configuration
/// entry is printed: a database token needs neither.
pub(crate) async fn token_create(resolved: Resolved, label: Option<String>) -> anyhow::Result<()> {
    // Before anything is opened or stored. The error names the rule, not the
    // text, which may hold the control character it is refused for.
    let label = Label::parse(label.as_deref().unwrap_or_default()).map_err(|error| anyhow::anyhow!("--label: {error}"))?;

    let state = super::state_for(&resolved).await?;
    let token = generate()?;
    let created = state
        .creation_tokens_repo
        .create(&Verifier::for_creation_token(token.as_bytes()), &label, SystemClock.now())
        .await
        .context("could not store the creation token")?;

    print_token(&token);
    println!("id: {}", created.id);
    if !label.as_str().is_empty() {
        println!("label: {}", label.as_str());
    }
    print_invitation(&state.public_url, &token);

    // Tokens may be prepared before the mode is switched, so this is no error.
    if state.creation_mode != CreationMode::Token {
        eprintln!(
            "note: the token has no effect while creation.mode is \"{}\"; it applies once the mode is \"token\"",
            state.creation_mode.as_str()
        );
    }
    Ok(())
}

/// One `token list` line: id, created, last used, revoked, label.
fn token_line(token: &CreationToken) -> String {
    let line = format!(
        "{}  created {}  last used {}  revoked {}  {}",
        token.id,
        format_utc(token.created_at),
        token.last_used_on.map_or_else(|| "never".to_owned(), format_utc_date),
        token.revoked_at.map_or_else(|| "-".to_owned(), format_utc),
        token.label.as_str(),
    );
    // No trailing blanks for a token without a label.
    line.trim_end().to_owned()
}

fn token_json(token: &CreationToken) -> serde_json::Value {
    serde_json::json!({
        "id": token.id,
        "label": token.label.as_str(),
        "created_at": token.created_at,
        "last_used_on": token.last_used_on,
        "revoked_at": token.revoked_at,
    })
}

/// `token list`: every database token, active and revoked, in creation order.
/// Nothing of a token's verifier is shown.
pub(crate) async fn token_list(resolved: Resolved, json: bool) -> anyhow::Result<()> {
    let state = super::state_for(&resolved).await?;
    let tokens = state
        .creation_tokens_repo
        .list()
        .await
        .context("could not read the creation tokens")?;

    if json {
        println!("{}", serde_json::Value::Array(tokens.iter().map(token_json).collect()));
        return Ok(());
    }

    let active = tokens.iter().filter(|token| token.revoked_at.is_none()).count();
    println!("active tokens: {active} (configured hashes: {})", state.creation_tokens.len());
    for token in &tokens {
        println!("{}", token_line(token));
    }
    Ok(())
}

/// `token revoke`: final; there is no command to undo it.
pub(crate) async fn token_revoke(resolved: Resolved, id: String) -> anyhow::Result<()> {
    // Ids are ULIDs; the canonical (upper-case) form is what is stored.
    let id = ulid::Ulid::from_string(&id)
        .map_err(|_| anyhow::anyhow!("not a token id: expected a ULID (26 characters) as printed by `token list`"))?
        .to_string();

    let state = super::state_for(&resolved).await?;
    match state
        .creation_tokens_repo
        .revoke(&id, SystemClock.now())
        .await
        .context("could not revoke the creation token")?
    {
        RevokeOutcome::Revoked => println!("ok: token {id} revoked"),
        RevokeOutcome::AlreadyRevoked => println!("ok: token {id} was already revoked"),
        RevokeOutcome::NotFound => bail!("no token {id}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn token(label: &str, last_used_on: Option<i64>, revoked_at: Option<i64>) -> CreationToken {
        CreationToken {
            id: "01KAPS0000000000000000TEST".to_owned(),
            label: Label::parse(label).unwrap(),
            created_at: 1_795_434_240,
            last_used_on,
            revoked_at,
        }
    }

    #[test]
    fn a_list_line_names_every_date_and_the_label() {
        assert_eq!(
            token_line(&token("alice", Some(1_795_392_000), Some(1_795_500_000))),
            "01KAPS0000000000000000TEST  created 2026-11-23 11:44 UTC  last used 2026-11-23  \
             revoked 2026-11-24 06:00 UTC  alice"
        );
        assert_eq!(
            token_line(&token("", None, None)),
            "01KAPS0000000000000000TEST  created 2026-11-23 11:44 UTC  last used never  revoked -"
        );
    }

    #[test]
    fn json_holds_exactly_the_listed_fields() {
        let json = token_json(&token("", None, Some(5)));
        assert_eq!(
            json,
            serde_json::json!({
                "id": "01KAPS0000000000000000TEST",
                "label": "",
                "created_at": 1_795_434_240,
                "last_used_on": null,
                "revoked_at": 5,
            })
        );
    }
}
