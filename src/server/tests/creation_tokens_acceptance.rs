#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance tests for `capsulebin token create|list|revoke` against the real
//! binary (package 20261122-creation-tokens-db): `create` stores one active row
//! and prints token, id, label and invitation link, but no hash (AC12); `list`
//! shows every token with its dates and label, as text or a plain JSON array
//! (AC13); `revoke` is final and tells an already revoked id apart from an
//! unknown one (AC14); a running `serve` honours tokens issued and revoked by
//! other processes without a restart, next to a configured hash (AC15); and
//! neither the server log nor the commands' diagnostics hold a token, its
//! verifier or its label, while the access log keeps the route template (AC17).

mod support;

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE};
use capsulebin_server::config::CreationMode;
use capsulebin_server::domain::{PasteId, Verifier};
use support::*;

/// The `server.public_url` of every configuration; the invitation links end up below it.
const PUBLIC_URL: &str = "https://paste.example/team";
const LABEL: &str = "LABELxMARKERxalice";

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

fn run(config: &Path, args: &[&str]) -> Output {
    capsulebin().arg("--config").arg(config).args(args).output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// stdout of a run that must succeed.
fn stdout(what: &str, output: &Output) -> String {
    assert!(
        output.status.success(),
        "{what}: exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        stderr(output)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// One row of `creation_tokens`, read straight from the table.
#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct Row {
    id: String,
    verifier: Vec<u8>,
    label: String,
    created_at: i64,
    last_used_on: Option<i64>,
    revoked_at: Option<i64>,
}

async fn rows(h: &Harness) -> Vec<Row> {
    sqlx::query_as("SELECT id, verifier, label, created_at, last_used_on, revoked_at FROM creation_tokens ORDER BY created_at, id")
        .fetch_all(&h.pool)
        .await
        .unwrap()
}

/// What `token create` printed.
struct Created {
    token: String,
    id: String,
}

/// Checks `token create`'s stdout line by line; returns token and id.
fn assert_created(what: &str, output: &Output, label: Option<&str>) -> Created {
    let text = stdout(what, output);
    let lines: Vec<&str> = text.lines().collect();
    let expected_len = if label.is_some() { 6 } else { 5 };
    assert_eq!(lines.len(), expected_len, "{what}: {text}");
    assert_eq!(lines[0], "token (store this in your client, it is not recoverable):", "{what}");
    let token = lines[1]
        .strip_prefix("  ")
        .unwrap_or_else(|| panic!("{what}: line 2 {:?}", lines[1]));
    assert_eq!(token.len(), 43, "{what}: {token:?}");
    assert!(token.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'), "{what}");
    let id = lines[2]
        .strip_prefix("id: ")
        .unwrap_or_else(|| panic!("{what}: id line {:?}", lines[2]));
    assert!(ulid::Ulid::from_string(id).is_ok(), "{what}: {id:?}");
    let rest = match label {
        Some(label) => {
            assert_eq!(lines[3], format!("label: {label}"), "{what}");
            &lines[4..]
        }
        None => &lines[3..],
    };
    assert_eq!(
        rest,
        [
            "invitation link (opens the create page and stores the token in the browser):".to_owned(),
            format!("  https://paste.example/team/#token={token}"),
        ],
        "{what}"
    );
    assert!(!text.contains("sha256:"), "{what}: {text}");
    assert!(!text.contains("token_hashes"), "{what}: {text}");
    Created {
        token: token.to_owned(),
        id: id.to_owned(),
    }
}

/// Everything of a token that must not show up where it does not belong: the
/// token, its verifier in every encoding, and its label.
fn traces(token: &str, label: &str) -> Vec<String> {
    let verifier = Verifier::for_creation_token(token.as_bytes());
    let bytes = verifier.as_bytes();
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    [
        token.to_owned(),
        verifier.encode(),
        URL_SAFE.encode(bytes),
        STANDARD.encode(bytes),
        STANDARD_NO_PAD.encode(bytes),
        hex.to_uppercase(),
        hex,
        label.to_owned(),
    ]
    .into_iter()
    .filter(|value| !value.is_empty())
    .collect()
}

fn assert_no_traces(what: &str, text: &str, traces: &[String]) {
    for value in traces {
        assert!(!text.contains(value.as_str()), "{what} shows {value:?}:\n{text}");
    }
}

/// AC12: one active row with the verifier of the printed token and the label;
/// stdout as specified, without a hash or configuration entry.
#[tokio::test]
async fn create_stores_one_active_token_and_prints_it_once() {
    let h = harness_with_mode("tokens-acc-create", CreationMode::Public).await;
    let config = creation_config_file(&h, PUBLIC_URL, "token", &[]);

    let output = run(&config, &["token", "create", "--label", LABEL]);
    let created = assert_created("with label", &output, Some(LABEL));
    assert!(!stderr(&output).contains("note:"), "{}", stderr(&output));
    assert_no_traces("create's stderr", &stderr(&output), &traces(&created.token, ""));

    let stored = rows(&h).await;
    assert_eq!(stored.len(), 1, "{stored:?}");
    let row = &stored[0];
    assert_eq!(row.id, created.id);
    assert_eq!(
        row.verifier,
        Verifier::for_creation_token(created.token.as_bytes()).as_bytes().to_vec()
    );
    assert_eq!(row.label, LABEL);
    assert_eq!((row.last_used_on, row.revoked_at), (None, None));

    // Without `--label` the label is empty and no label line is printed.
    let output = run(&config, &["token", "create"]);
    let unlabelled = assert_created("without label", &output, None);
    let stored = rows(&h).await;
    assert_eq!(stored.len(), 2);
    let row = stored.iter().find(|row| row.id == unlabelled.id).unwrap();
    assert_eq!(row.label, "");
    assert_eq!(row.revoked_at, None);
}

/// AC12: a label above 64 characters or with a control character is refused
/// naming `--label`, and nothing is stored.
#[tokio::test]
async fn create_refuses_an_invalid_label_and_stores_nothing() {
    let h = harness_with_mode("tokens-acc-label", CreationMode::Public).await;
    let config = creation_config_file(&h, PUBLIC_URL, "token", &[]);

    // The longest label is still taken.
    let longest = "ä".repeat(64);
    assert_created(
        "64 characters",
        &run(&config, &["token", "create", "--label", &longest]),
        Some(&longest),
    );
    let before = rows(&h).await;

    for label in [
        "a".repeat(65),
        "tab\there".to_owned(),
        "line\nbreak".to_owned(),
        "esc\u{1b}[2J".to_owned(),
    ] {
        let output = run(&config, &["token", "create", "--label", &label]);
        assert!(!output.status.success(), "{label:?} accepted");
        assert!(output.stdout.is_empty(), "{label:?}: {}", String::from_utf8_lossy(&output.stdout));
        assert!(stderr(&output).contains("--label"), "{label:?}: {}", stderr(&output));
        assert_eq!(rows(&h).await, before, "{label:?} stored something");
    }
}

/// AC12: outside `token` mode the token is still created, with a note that it
/// has no effect in that mode.
#[tokio::test]
async fn create_outside_token_mode_notes_that_it_has_no_effect() {
    let h = harness_with_mode("tokens-acc-other-mode", CreationMode::Public).await;

    for mode in ["public", "disabled"] {
        let output = run(
            &creation_config_file(&h, PUBLIC_URL, mode, &[]),
            &["token", "create", "--label", LABEL],
        );
        let created = assert_created(mode, &output, Some(LABEL));
        let note = stderr(&output);
        let line = note
            .lines()
            .find(|line| line.starts_with("note: "))
            .unwrap_or_else(|| panic!("{mode}: no note:\n{note}"));
        assert!(line.contains("no effect") && line.contains(mode), "{mode}: {line}");
        assert_no_traces(mode, &note, &traces(&created.token, LABEL));
        assert!(rows(&h).await.iter().any(|row| row.id == created.id && row.revoked_at.is_none()));
    }
}

/// AC12: without a usable configuration `token create` fails the way
/// `reports list` does, before anything is stored.
#[tokio::test]
async fn create_without_a_usable_configuration_fails_like_reports() {
    let h = harness_with_mode("tokens-acc-unusable", CreationMode::Public).await;
    let config = h.dir.join("no-public-url.toml");
    std::fs::write(
        &config,
        format!(
            "[database]\npath = \"{}\"\n[creation]\nmode = \"token\"\n",
            h.dir.join("capsulebin.db").display()
        ),
    )
    .unwrap();

    let reports = run(&config, &["reports", "list"]);
    assert!(!reports.status.success());
    for args in [&["token", "create", "--label", LABEL][..], &["token", "create"], &["token", "list"]] {
        let output = run(&config, args);
        assert!(!output.status.success(), "{args:?} succeeded");
        assert!(output.stdout.is_empty(), "{args:?}: {}", String::from_utf8_lossy(&output.stdout));
        assert_eq!(stderr(&output), stderr(&reports), "{args:?}");
        assert!(stderr(&output).contains("server.public_url"), "{args:?}: {}", stderr(&output));
    }
    assert!(rows(&h).await.is_empty());
}

/// AC13: the summary line, one line per token in creation order with every
/// date and the label, and `--json` as a plain array; neither shows a token
/// or a verifier.
#[tokio::test]
async fn list_shows_every_token_without_its_secret() {
    let h = harness_with_mode("tokens-acc-list", CreationMode::Public).await;
    let config = creation_config_file(&h, PUBLIC_URL, "token", &[configured_hash(TOKEN), configured_hash("another")]);

    let empty = stdout("empty list", &run(&config, &["token", "list"]));
    assert_eq!(empty, "active tokens: 0 (configured hashes: 2)\n");
    assert_eq!(stdout("empty json", &run(&config, &["token", "list", "--json"])).trim(), "[]");

    let first = assert_created("first", &run(&config, &["token", "create", "--label", LABEL]), Some(LABEL));
    let second = assert_created("second", &run(&config, &["token", "create"]), None);
    // Fixed dates make the lines predictable; the order stays by creation.
    sqlx::query("UPDATE creation_tokens SET created_at = ?2, last_used_on = ?3, revoked_at = ?4 WHERE id = ?1")
        .bind(&first.id)
        .bind(1_795_434_240_i64)
        .bind(1_795_478_400_i64)
        .bind(1_795_500_000_i64)
        .execute(&h.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE creation_tokens SET created_at = ?2 WHERE id = ?1")
        .bind(&second.id)
        .bind(1_795_434_300_i64)
        .execute(&h.pool)
        .await
        .unwrap();

    let mut secrets = traces(&first.token, "");
    secrets.extend(traces(&second.token, ""));

    let text = stdout("list", &run(&config, &["token", "list"]));
    assert_no_traces("list", &text, &secrets);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines,
        [
            "active tokens: 1 (configured hashes: 2)".to_owned(),
            format!(
                "{}  created 2026-11-23 11:44 UTC  last used 2026-11-24  revoked 2026-11-24 06:00 UTC  {LABEL}",
                first.id
            ),
            format!("{}  created 2026-11-23 11:45 UTC  last used never  revoked -", second.id),
        ]
    );

    let json = stdout("list --json", &run(&config, &["token", "list", "--json"]));
    assert_no_traces("list --json", &json, &secrets);
    let parsed: serde_json::Value = serde_json::from_str(json.trim()).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!([
            {
                "id": first.id,
                "label": LABEL,
                "created_at": 1_795_434_240_i64,
                "last_used_on": 1_795_478_400_i64,
                "revoked_at": 1_795_500_000_i64,
            },
            {
                "id": second.id,
                "label": "",
                "created_at": 1_795_434_300_i64,
                "last_used_on": null,
                "revoked_at": null,
            },
        ])
    );
}

/// AC14: revoke sets `revoked_at`; again is `0` with a message and changes
/// nothing; an unknown or malformed id fails and changes nothing; nothing
/// undoes a revocation.
#[tokio::test]
async fn revoke_is_final() {
    let h = harness_with_mode("tokens-acc-revoke", CreationMode::Public).await;
    let config = creation_config_file(&h, PUBLIC_URL, "token", &[]);
    let kept = assert_created("kept", &run(&config, &["token", "create"]), None);
    let revoked = assert_created("revoked", &run(&config, &["token", "create", "--label", LABEL]), Some(LABEL));

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let output = stdout("revoke", &run(&config, &["token", "revoke", &revoked.id]));
    assert!(output.contains(&revoked.id) && output.contains("revoked"), "{output}");
    let stored = rows(&h).await;
    let row = stored.iter().find(|row| row.id == revoked.id).unwrap();
    let at = row.revoked_at.expect("revoked_at set");
    assert!(at >= before && at <= before + 60, "{at} vs {before}");
    assert_eq!(stored.iter().find(|row| row.id == kept.id).unwrap().revoked_at, None);

    // A recognisable value shows that a second revoke leaves it alone; the
    // id in lower case is the same ULID.
    sqlx::query("UPDATE creation_tokens SET revoked_at = 1000 WHERE id = ?1")
        .bind(&revoked.id)
        .execute(&h.pool)
        .await
        .unwrap();
    let snapshot = rows(&h).await;
    for id in [revoked.id.clone(), revoked.id.to_lowercase()] {
        let again = stdout("revoke again", &run(&config, &["token", "revoke", &id]));
        assert!(again.contains("already revoked"), "{again}");
        assert_eq!(rows(&h).await, snapshot);
    }

    let unknown = ulid::Ulid::generate().to_string();
    for id in [unknown.as_str(), "not-a-token-id", "", &revoked.id[..25]] {
        let output = run(&config, &["token", "revoke", id]);
        assert!(!output.status.success(), "{id:?} accepted");
        assert!(output.stdout.is_empty(), "{id:?}: {}", String::from_utf8_lossy(&output.stdout));
        assert_eq!(rows(&h).await, snapshot, "{id:?} changed something");
    }

    // The only subcommands are create, list and revoke.
    let help = stdout("token --help", &capsulebin().args(["token", "--help"]).output().unwrap());
    let commands: Vec<&str> = help
        .lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| line.starts_with("  "))
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert_eq!(commands, ["create", "list", "revoke", "help"], "{help}");
}

/// Kills the server when the test ends, also on a failed assertion.
struct Server {
    child: std::process::Child,
    port: u16,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    /// `serve` on a free port, returned once `/health/ready` answers `200`.
    fn start(config: &Path) -> Self {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let child = capsulebin()
            .arg("--config")
            .arg(config)
            .arg("serve")
            .env("CAPSULEBIN__SERVER__BIND", format!("127.0.0.1:{port}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut server = Self { child, port };
        let deadline = Instant::now() + Duration::from_secs(20);
        while !server.request("GET /health/ready", &[], None).starts_with("HTTP/1.1 200") {
            if let Some(status) = server.child.try_wait().unwrap() {
                panic!("serve exited with {status}: {}", server.stop());
            }
            assert!(Instant::now() < deadline, "server did not become ready");
            std::thread::sleep(Duration::from_millis(100));
        }
        server
    }

    /// One request over a fresh connection; the whole answer, or `""` when
    /// the server cannot be reached.
    fn request(&self, line: &str, body: &[u8], bearer: Option<&str>) -> String {
        let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", self.port)) else {
            return String::new();
        };
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let mut head = format!("{line} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
        if !body.is_empty() {
            head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len()));
        }
        if let Some(token) = bearer {
            head.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        head.push_str("\r\n");
        if stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body)).is_err() {
            return String::new();
        }
        let mut answer = Vec::new();
        let _ = stream.read_to_end(&mut answer);
        String::from_utf8_lossy(&answer).into_owned()
    }

    /// The status of a create of `paste` with `bearer`.
    fn create(&self, paste: &PasteId, bearer: Option<&str>) -> u16 {
        let answer = self.request("POST /api/v1/pastes", create_body(paste).to_string().as_bytes(), bearer);
        answer
            .split_whitespace()
            .nth(1)
            .and_then(|status| status.parse().ok())
            .unwrap_or_else(|| panic!("no status in {answer:?}"))
    }

    /// Stops the server; returns its log.
    fn stop(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut log = String::new();
        if let Some(mut stderr) = self.child.stderr.take() {
            stderr.read_to_string(&mut log).unwrap();
        }
        log
    }
}

/// Every `path` the access log recorded: its span renders as
/// `request{method=POST path=/api/v1/pastes}: …`.
fn logged_paths(log: &str) -> Vec<&str> {
    log.split("request{method=")
        .skip(1)
        .filter_map(|rest| rest.split_once(" path=")?.1.split('}').next())
        .collect()
}

/// AC15 and AC17: one running server, tokens issued and revoked by other
/// processes, a configured hash accepted throughout, and nothing of a token in
/// the server log or the commands' diagnostics.
async fn live(name: &str, configured: bool) {
    let h = harness_with_mode(name, CreationMode::Public).await;
    let hashes = if configured { vec![configured_hash(TOKEN)] } else { Vec::new() };
    let config = creation_config_file(&h, PUBLIC_URL, "token", &hashes);
    let mut server = Server::start(&config);
    let mut next = 0_u8;
    let mut paste = || {
        next += 1;
        id(next)
    };
    let expect_configured = |server: &Server, paste: &PasteId| {
        let expected = if configured { 201 } else { 401 };
        assert_eq!(server.create(paste, Some(TOKEN)), expected, "configured token");
    };

    assert_eq!(server.create(&paste(), None), 401, "no token");
    expect_configured(&server, &paste());

    let create = run(&config, &["token", "create", "--label", LABEL]);
    let issued = assert_created("create", &create, Some(LABEL));
    let secrets = traces(&issued.token, LABEL);
    assert_eq!(server.create(&paste(), Some(&issued.token)), 201, "issued token");
    expect_configured(&server, &paste());

    let list = run(&config, &["token", "list"]);
    assert!(stdout("list", &list).contains(&issued.id));
    let revoke = run(&config, &["token", "revoke", &issued.id]);
    stdout("revoke", &revoke);
    assert_eq!(server.create(&paste(), Some(&issued.token)), 401, "revoked token");
    expect_configured(&server, &paste());

    // The server was started once and is still the same process.
    assert!(server.child.try_wait().unwrap().is_none(), "the server stopped");
    let log = server.stop();

    // AC17: the server log, and every diagnostic of the commands, hold nothing
    // of the token; the access log names the route template only.
    assert_no_traces("server log", &log, &secrets);
    for (what, output) in [("create", &create), ("list", &list), ("revoke", &revoke)] {
        assert_no_traces(what, &stderr(output), &secrets);
    }
    assert_no_traces("list", &String::from_utf8_lossy(&list.stdout), &traces(&issued.token, ""));
    assert_no_traces("revoke", &String::from_utf8_lossy(&revoke.stdout), &secrets);
    let paths = logged_paths(&log);
    assert!(paths.contains(&"/api/v1/pastes"), "no create in the log:\n{log}");
    for path in paths {
        assert!(
            ["/api/v1/pastes", "/health/ready"].contains(&path),
            "logged path {path:?} is no route template:\n{log}"
        );
    }
}

#[tokio::test]
async fn a_running_server_follows_issued_and_revoked_tokens() {
    live("tokens-acc-live", false).await;
}

#[tokio::test]
async fn a_configured_hash_works_next_to_database_tokens() {
    live("tokens-acc-live-configured", true).await;
}
