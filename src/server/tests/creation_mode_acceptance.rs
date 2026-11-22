#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance tests for the creation mode warnings against the real binary
//! (package 20261122-creation-tokens-db, AC4): `token` mode with neither a
//! configured hash nor an active database token makes `check` print a
//! `warning:` (still `ok:`, exit 0) and `serve` log one and become ready;
//! active database tokens while the mode is `public` or `disabled` are warned
//! about as without effect; a configured hash or an active database token in
//! `token` mode gives neither warning. No warning holds a token, hash or label.

mod support;

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use capsulebin_server::config::CreationMode;
use capsulebin_server::domain::Verifier;
use support::*;

/// The `server.public_url` of every configuration.
const PUBLIC_URL: &str = "https://paste.example";
const DB_TOKEN: &str = "DBxCREATIONxTOKENxMARKER";
const DB_LABEL: &str = "DBxLABELxMARKER";

/// What `check` and `serve` say for `token` mode without any token.
const NO_TOKEN: &str = "no creation token exists";
/// What they say for database tokens outside `token` mode.
const NO_EFFECT: &str = "have no effect";

/// `check`, which must succeed; returns its stderr.
fn check(config: &Path) -> String {
    let output: Output = Command::new(env!("CARGO_BIN_EXE_capsulebin"))
        .arg("--config")
        .arg(config)
        .arg("check")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "check failed: {stdout}\n{stderr}");
    assert!(stdout.starts_with("ok:"), "{stdout}\n{stderr}");
    stderr
}

/// Kills the server when the test ends, also on a failed assertion.
struct Server(std::process::Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Whether `GET /health/ready` answers `200`.
fn ready(port: u16) -> bool {
    let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    if write!(stream, "GET /health/ready HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").is_err() {
        return false;
    }
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    answer.starts_with(b"HTTP/1.1 200")
}

/// `serve` until it is ready, then stopped; returns its log.
fn serve(config: &Path) -> String {
    let port = free_port();
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_capsulebin"))
            .arg("--config")
            .arg(config)
            .arg("serve")
            .env("CAPSULEBIN__SERVER__BIND", format!("127.0.0.1:{port}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready(port) {
        if let Some(status) = server.0.try_wait().unwrap() {
            let mut log = String::new();
            server.0.stderr.take().unwrap().read_to_string(&mut log).unwrap();
            panic!("serve exited with {status}: {log}");
        }
        assert!(Instant::now() < deadline, "server did not become ready");
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = server.0.kill();
    let _ = server.0.wait();
    let mut log = String::new();
    server.0.stderr.take().unwrap().read_to_string(&mut log).unwrap();
    log
}

/// `check`'s stderr and `serve`'s log for `config`, each holding nothing of a
/// token: not the token, its hash or its label.
fn warnings(config: &Path, tokens: &[(&str, &str)]) -> [(&'static str, String); 2] {
    let outputs = [("check", check(config)), ("serve", serve(config))];
    for (command, output) in &outputs {
        for (token, label) in tokens {
            let verifier = Verifier::for_creation_token(token.as_bytes());
            let hex: String = verifier.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect();
            for value in [*token, *label, verifier.encode().as_str(), hex.as_str()]
                .into_iter()
                .filter(|value| !value.is_empty())
            {
                assert!(!output.contains(value), "{command} shows {value:?}:\n{output}");
            }
        }
    }
    outputs
}

/// The one line of `output` holding `text`, which must be a warning.
fn warning_line<'a>(command: &str, output: &'a str, text: &str) -> &'a str {
    let lines: Vec<&str> = output.lines().filter(|line| line.contains(text)).collect();
    assert_eq!(lines.len(), 1, "{command}: expected one line with {text:?}:\n{output}");
    let line = lines[0];
    match command {
        "check" => assert!(line.starts_with("warning: "), "{line}"),
        _ => assert!(line.contains("WARN"), "{line}"),
    }
    line
}

#[tokio::test]
async fn token_mode_without_any_token_warns_and_still_starts() {
    let h = harness_with_mode("creation-mode-acc-none", CreationMode::Public).await;

    for (command, output) in warnings(&creation_config_file(&h, PUBLIC_URL, "token", &[]), &[]) {
        let line = warning_line(command, &output, NO_TOKEN);
        assert!(line.contains("creation.mode"), "{command}: {line}");
        assert!(!output.contains(NO_EFFECT), "{command}:\n{output}");
    }

    // A revoked database token is no token either.
    let revoked = h.issue_token(DB_TOKEN, DB_LABEL).await;
    h.revoke_token(&revoked).await;
    for (command, output) in warnings(&creation_config_file(&h, PUBLIC_URL, "token", &[]), &[(DB_TOKEN, DB_LABEL)]) {
        warning_line(command, &output, NO_TOKEN);
    }
}

#[tokio::test]
async fn database_tokens_outside_token_mode_warn() {
    let h = harness_with_mode("creation-mode-acc-other", CreationMode::Public).await;
    h.issue_token(DB_TOKEN, DB_LABEL).await;

    for mode in ["public", "disabled"] {
        for (command, output) in warnings(&creation_config_file(&h, PUBLIC_URL, mode, &[]), &[(DB_TOKEN, DB_LABEL)]) {
            let line = warning_line(command, &output, NO_EFFECT);
            assert!(line.contains(mode), "{command}, {mode}: {line}");
            assert!(!output.contains(NO_TOKEN), "{command}, {mode}:\n{output}");
        }
    }
}

#[tokio::test]
async fn a_token_in_token_mode_gives_no_warning() {
    let h = harness_with_mode("creation-mode-acc-quiet", CreationMode::Public).await;
    let tokens = [(DB_TOKEN, DB_LABEL), (TOKEN, "")];

    // A configured hash alone.
    let configured = creation_config_file(&h, PUBLIC_URL, "token", &[configured_hash(TOKEN)]);
    // An active database token alone.
    let database = creation_config_file(&h, PUBLIC_URL, "token", &[]);
    // Neither mode without database tokens warns.
    let public = creation_config_file(&h, PUBLIC_URL, "public", &[]);
    let disabled = creation_config_file(&h, PUBLIC_URL, "disabled", &[]);

    for config in [&configured, &public, &disabled] {
        for (command, output) in warnings(config, &tokens) {
            assert!(!output.contains(NO_TOKEN), "{command}, {}:\n{output}", config.display());
            assert!(!output.contains(NO_EFFECT), "{command}, {}:\n{output}", config.display());
        }
    }

    h.issue_token(DB_TOKEN, DB_LABEL).await;
    for (command, output) in warnings(&database, &tokens) {
        assert!(!output.contains(NO_TOKEN), "{command}:\n{output}");
        assert!(!output.contains(NO_EFFECT), "{command}:\n{output}");
    }
}
