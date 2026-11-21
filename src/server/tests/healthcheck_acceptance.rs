#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance test for `capsulebin healthcheck` against the real binary
//! (package 20261121-operator-files-restore, AC13): the probe needs only the
//! address. It validates nothing else in the configuration (no `public_url`,
//! a database directory that does not exist) and writes nothing into the
//! server's database directory.

mod support;

use std::collections::BTreeSet;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use support::{tempdir, write_shells};

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

/// `true` once `GET /health/ready` answers `200`.
fn ready(port: u16) -> bool {
    let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let request = format!("GET /health/ready HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response.starts_with("HTTP/1.1 200")
}

fn entries(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

fn assert_ok(what: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{what}: exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// AC13: a server started with `--config <file>` (`public_url` and the
/// database path in the file, `bind` from `CAPSULEBIN__SERVER__BIND`) is
/// reported healthy by `healthcheck` and `healthcheck --live` with nothing but
/// `CAPSULEBIN__SERVER__BIND` (and a database path into a missing directory),
/// by `healthcheck --config <file>` with `bind` only in that file, and by
/// `healthcheck --address` without any configuration. None of them creates an
/// entry in the server's database directory.
#[test]
fn healthcheck_needs_only_the_address() {
    let dir = tempdir("healthcheck-acc-address");
    let assets = dir.join("assets");
    write_shells(&assets);
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let config = dir.join("capsulebin.toml");
    std::fs::write(
        &config,
        format!(
            "[server]\npublic_url = \"https://paste.example\"\nasset_dir = \"{}\"\n\
             [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n",
            assets.display(),
            data.join("capsulebin.db").display()
        ),
    )
    .unwrap();

    let port = free_port();
    let bind = format!("127.0.0.1:{port}");
    let _server = Server(
        capsulebin()
            .arg("--config")
            .arg(&config)
            .arg("serve")
            .env("CAPSULEBIN__SERVER__BIND", &bind)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready(port) {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    let before = entries(&data);
    assert!(before.contains("capsulebin.db"), "{before:?}");

    // Only the bind address: no `public_url`, and a database directory that
    // does not exist. Validation would refuse both.
    let missing = dir.join("missing");
    let database = missing.join("capsulebin.db");
    for args in [&["healthcheck"][..], &["healthcheck", "--live"][..]] {
        let output = capsulebin()
            .args(args)
            .env("CAPSULEBIN__SERVER__BIND", &bind)
            .env("CAPSULEBIN__DATABASE__PATH", &database)
            .output()
            .unwrap();
        assert_ok(&args.join(" "), &output);
    }
    assert!(!missing.exists(), "healthcheck created the database directory");

    // `bind` only from the file given with `--config`.
    let probe = dir.join("probe.toml");
    std::fs::write(&probe, format!("[server]\nbind = \"{bind}\"\n")).unwrap();
    let output = capsulebin().arg("--config").arg(&probe).arg("healthcheck").output().unwrap();
    assert_ok("healthcheck --config", &output);

    // No configuration and no `CAPSULEBIN__*` variable at all.
    let output = capsulebin().args(["healthcheck", "--address", &bind]).output().unwrap();
    assert_ok("healthcheck --address", &output);

    assert_eq!(entries(&data), before, "healthcheck changed the database directory");
}
