#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance test for `capsulebin token` against the real binary (package
//! 20261121-web-creation-token, AC3 and AC4): today's output comes first and
//! unchanged, an invitation link follows only for a `server.public_url` from
//! `--config` or the environment that passes the `public_url` rules, a refused
//! one is a note on stderr without its value, and an explicit `--config` that
//! cannot be loaded prints no token.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use support::tempdir;

/// The binary with no inherited `CAPSULEBIN__*` variable, so only what a test
/// sets reaches it.
fn capsulebin() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_capsulebin"));
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("CAPSULEBIN__") {
            command.env_remove(key);
        }
    }
    command
}

fn run(command: &mut Command) -> Output {
    command.output().unwrap()
}

fn stdout(output: &Output) -> Vec<String> {
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_ok(what: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{what}: exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        stderr(output)
    );
}

/// Today's seven lines, checked exactly; returns the token from line 2.
fn assert_token_lines(what: &str, lines: &[String]) -> String {
    assert!(lines.len() >= 7, "{what}: too few lines: {lines:?}");
    assert_eq!(lines[0], "token (store this in your client, it is not recoverable):", "{what}");
    let token = lines[1]
        .strip_prefix("  ")
        .unwrap_or_else(|| panic!("{what}: line 2 {:?}", lines[1]));
    assert_eq!(token.len(), 43, "{what}: token {token:?}");
    assert!(token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "{what}");
    assert_eq!(lines[2], "", "{what}");
    assert_eq!(lines[3], "configuration entry:", "{what}");
    assert_eq!(lines[4], "  [creation]", "{what}");
    assert_eq!(lines[5], "  mode = \"token\"", "{what}");
    let hash = lines[6]
        .strip_prefix("  token_hashes = [\"sha256:")
        .and_then(|rest| rest.strip_suffix("\"]"))
        .unwrap_or_else(|| panic!("{what}: hash line {:?}", lines[6]));
    assert!(!hash.is_empty(), "{what}");
    token.to_owned()
}

/// Today's lines followed by exactly the invitation link on `base`.
fn assert_invitation(what: &str, output: &Output, base: &str) {
    assert_ok(what, output);
    let lines = stdout(output);
    let token = assert_token_lines(what, &lines);
    assert_eq!(
        lines[7..],
        [
            String::new(),
            "invitation link (opens the create page and stores the token in the browser):".to_owned(),
            format!("  {base}/#token={token}"),
        ],
        "{what}"
    );
}

/// Today's lines and nothing more, exit 0.
fn assert_no_invitation(what: &str, output: &Output) {
    assert_ok(what, output);
    let lines = stdout(output);
    assert_token_lines(what, &lines);
    assert_eq!(lines.len(), 7, "{what}: {lines:?}");
}

fn write_config(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

/// AC3: without any configuration the output is exactly today's.
#[test]
fn without_configuration_prints_only_the_token_and_hash() {
    let output = run(capsulebin().arg("token"));
    assert_no_invitation("no configuration", &output);
    assert!(!stderr(&output).contains("invitation"), "{}", stderr(&output));

    // A configuration without `public_url` adds nothing either.
    let dir = tempdir("token-no-public-url");
    let config = write_config(&dir, "capsulebin.toml", "[server]\nbind = \"127.0.0.1:9\"\n");
    let output = run(capsulebin().arg("--config").arg(&config).arg("token"));
    assert_no_invitation("configuration without public_url", &output);
}

/// AC3: `public_url` from the environment or from `--config` adds the link,
/// with the token of line 2; nothing else in the configuration is validated.
#[test]
fn public_url_adds_the_invitation_link() {
    let output = run(capsulebin()
        .env("CAPSULEBIN__SERVER__PUBLIC_URL", "https://paste.example")
        .arg("token"));
    assert_invitation("environment", &output, "https://paste.example");

    let dir = tempdir("token-config");
    // Not usable for `serve`: a database directory that does not exist and
    // token mode without any token hash.
    let config = write_config(
        &dir,
        "capsulebin.toml",
        "[server]\npublic_url = \"https://paste.example/team\"\n\n\
         [database]\npath = \"/nonexistent/capsulebin-token-test/capsulebin.db\"\n\n\
         [creation]\nmode = \"token\"\n",
    );
    let output = run(capsulebin().arg("--config").arg(&config).arg("token"));
    assert_invitation("--config", &output, "https://paste.example/team");

    // The environment overrides the file, as for every other command.
    let output = run(capsulebin()
        .env("CAPSULEBIN__SERVER__PUBLIC_URL", "https://other.example")
        .arg("--config")
        .arg(&config)
        .arg("token"));
    assert_invitation("--config and environment", &output, "https://other.example");
}

/// AC4: a `public_url` that breaks a rule prints the token and hash without
/// the link, exit 0, and a note naming the key and the reason, not the value.
#[test]
fn refused_public_url_is_a_note_without_the_value() {
    let cases = [
        ("https://refused-host.example/team?x=1", "query or fragment"),
        ("https://refused-host.example/team#v1.secret", "query or fragment"),
        ("https://refused-host.example/team/", "must not end with a slash"),
        ("https://refused-host.example/team/de", "language code \"de\""),
        ("ftp://refused-host.example", "absolute http(s) URL"),
        ("https://user:pw@refused-host.example", "user information"),
    ];
    for (value, reason) in cases {
        let output = run(capsulebin().env("CAPSULEBIN__SERVER__PUBLIC_URL", value).arg("token"));
        assert_no_invitation(value, &output);
        let note = stderr(&output);
        assert!(note.contains("server.public_url"), "{value}: {note}");
        assert!(note.contains(reason), "{value}: {note}");
        assert!(!note.contains("refused-host"), "{value}: the note repeats the value: {note}");
        assert!(!String::from_utf8_lossy(&output.stdout).contains("refused-host"), "{value}");
    }

    let dir = tempdir("token-refused-config");
    let config = write_config(
        &dir,
        "capsulebin.toml",
        "[server]\npublic_url = \"https://refused-host.example/de/\"\n",
    );
    let output = run(capsulebin().arg("--config").arg(&config).arg("token"));
    assert_no_invitation("--config", &output);
    let note = stderr(&output);
    assert!(note.contains("server.public_url") && note.contains("language code"), "{note}");
    assert!(!note.contains("refused-host"), "{note}");
}

/// AC4: an explicit `--config` that cannot be loaded is an error and prints
/// no token.
#[test]
fn unloadable_config_prints_no_token() {
    let dir = tempdir("token-unloadable");
    let missing = dir.join("missing.toml");
    let unparseable = write_config(&dir, "broken.toml", "[server\npublic_url = \"https://paste.example\"\n");
    let unknown_key = write_config(
        &dir,
        "unknown.toml",
        "[server]\npublic_url = \"https://paste.example\"\npublic_ulr = \"x\"\n",
    );
    for (what, config) in [("missing", &missing), ("unparseable", &unparseable), ("unknown key", &unknown_key)] {
        let output = run(capsulebin().arg("--config").arg(config).arg("token"));
        assert!(!output.status.success(), "{what}: exit {:?}", output.status);
        assert!(output.stdout.is_empty(), "{what}: {}", String::from_utf8_lossy(&output.stdout));
        assert!(
            stderr(&output).contains("could not load the configuration"),
            "{what}: {}",
            stderr(&output)
        );
    }
}

/// AC11 (package 20261122-creation-tokens-db): plain `token` stays a pure
/// generator next to `token create`: with a configuration that `serve` would
/// accept it opens no database and writes nothing, and its output is today's.
#[test]
fn plain_token_stores_nothing() {
    let dir = tempdir("token-stores-nothing");
    let config = write_config(
        &dir,
        "capsulebin.toml",
        &format!(
            "[server]\npublic_url = \"https://paste.example\"\n\n\
             [database]\npath = \"{}\"\n\n\
             [creation]\nmode = \"token\"\n",
            dir.join("capsulebin.db").display()
        ),
    );
    let output = run(capsulebin().arg("--config").arg(&config).arg("token"));
    assert_invitation("valid configuration", &output, "https://paste.example");

    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().file_name()).collect();
    assert_eq!(entries, ["capsulebin.toml"], "plain `token` wrote into the database directory");
}
