#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance tests for `[branding]` against the real binary (package
//! 20261120-branding-identity): a configuration `Config::validate` refuses
//! stops `check` and `serve` with a non-zero exit code and a message naming
//! the key (AC1, AC2, AC3); every key works as `CAPSULEBIN__BRANDING__<KEY>`
//! and a configured file is served by the started binary (AC1, AC4).

mod support;

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use support::{ICO, PNG, WEBP, branding_url, tempdir, write_shells};

/// A configuration file with test shells from `asset_dir`, `branding`
/// appended.
fn config_file(dir: &Path, name: &str, branding: &str) -> PathBuf {
    let assets = dir.join("assets");
    write_shells(&assets);
    let path = dir.join(name);
    std::fs::write(
        &path,
        format!(
            "[server]\npublic_url = \"https://paste.example\"\nasset_dir = \"{}\"\n\
             [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n{branding}",
            assets.display(),
            dir.join(format!("{name}.db")).display()
        ),
    )
    .unwrap();
    path
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

fn check(config: &Path, env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_capsulebin"))
        .arg("--config")
        .arg(config)
        .arg("check")
        .envs(env.iter().copied())
        .output()
        .unwrap()
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

fn serve_command(config: &Path, port: u16, env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_capsulebin"));
    command
        .arg("--config")
        .arg(config)
        .arg("serve")
        .env("CAPSULEBIN__SERVER__BIND", format!("127.0.0.1:{port}"))
        .envs(env.iter().copied());
    command
}

/// `serve` with a configuration it must refuse: it exits on its own, non-zero,
/// and its stderr is returned. A server still running after 20 s fails.
fn serve_refused(config: &Path, env: &[(&str, &str)]) -> String {
    let child = serve_command(config, free_port(), env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut server = Server(child);
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = server.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "serve started despite the configuration");
        std::thread::sleep(Duration::from_millis(50));
    };
    let mut stderr = String::new();
    server.0.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    assert!(!status.success(), "serve exited with success: {stderr}");
    stderr
}

/// Both `check` and `serve` refuse the configuration, naming every one of
/// `named` and none of `absent`.
fn assert_refused(config: &Path, env: &[(&str, &str)], named: &[&str], absent: &[&str]) {
    let output = check(config, env);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(!output.status.success(), "check accepted {env:?}: {stderr}");
    let served = serve_refused(config, env);
    for (command, message) in [("check", &stderr), ("serve", &served)] {
        for name in named {
            assert!(message.contains(name), "{command} {env:?}: {name} not named: {message}");
        }
        for text in absent {
            assert!(!message.contains(text), "{command} {env:?}: {text} in message: {message}");
        }
    }
}

/// AC1: an unknown key in `[branding]`, from the file or the environment,
/// stops `check` and `serve`.
#[test]
fn ac1_unknown_branding_keys_stop_check_and_serve() {
    let dir = tempdir("branding-acc-unknown");
    let typo = config_file(&dir, "typo.toml", "[branding]\nlogo_light = \"logo.png\"\n");
    assert_refused(&typo, &[], &["logo_light"], &[]);

    let plain = config_file(&dir, "plain.toml", "");
    let output = check(&plain, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_refused(&plain, &[("CAPSULEBIN__BRANDING__COLOR", "red")], &["color"], &[]);
}

/// AC2: every refused value stops `check` and `serve`, naming the key, also
/// when it comes from the environment.
#[test]
fn ac2_refused_values_stop_check_and_serve() {
    let dir = tempdir("branding-acc-values");
    let logo = write(&dir, "logo.png", PNG);
    let logo = logo.to_str().unwrap();
    let plain = config_file(&dir, "plain.toml", "");
    let long = "x".repeat(65);

    for (env, key) in [
        (vec![("CAPSULEBIN__BRANDING__NAME", "")], "branding.name"),
        (vec![("CAPSULEBIN__BRANDING__NAME", "   ")], "branding.name"),
        (vec![("CAPSULEBIN__BRANDING__NAME", long.as_str())], "branding.name"),
        (vec![("CAPSULEBIN__BRANDING__NAME", "Team\u{1b}Paste")], "branding.name"),
        (vec![("CAPSULEBIN__BRANDING__HIDE_NAME", "true")], "branding.hide_name"),
        (vec![("CAPSULEBIN__BRANDING__LOGO_DARK", logo)], "branding.logo_dark"),
        (vec![("CAPSULEBIN__BRANDING__SOURCE_URL", "/source")], "branding.source_url"),
        (
            vec![("CAPSULEBIN__BRANDING__SOURCE_URL", "ftp://git.example/capsulebin")],
            "branding.source_url",
        ),
        (
            vec![("CAPSULEBIN__BRANDING__SOURCE_URL", "https://me:pw@git.example/capsulebin")],
            "branding.source_url",
        ),
    ] {
        assert_refused(&plain, &env, &[key], &["pw@"]);
    }

    // The edge of the name limit and the dependent keys with a logo pass.
    let edge = "ä".repeat(64);
    let output = check(
        &plain,
        &[
            ("CAPSULEBIN__BRANDING__NAME", edge.as_str()),
            ("CAPSULEBIN__BRANDING__LOGO", logo),
            ("CAPSULEBIN__BRANDING__LOGO_DARK", logo),
            ("CAPSULEBIN__BRANDING__HIDE_NAME", "true"),
        ],
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

/// AC3: an unusable file stops `check` and `serve`, naming key and path and
/// never the content.
#[test]
fn ac3_unusable_files_stop_check_and_serve() {
    let dir = tempdir("branding-acc-files");
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'><script>ACCEPTANCE-SECRET</script></svg>";
    let mut large = PNG.to_vec();
    large.resize(256 * 1024 + 1, 0);
    let plain = config_file(&dir, "plain.toml", "");

    for (key, env_key, file, bytes, expected) in [
        ("branding.logo", "CAPSULEBIN__BRANDING__LOGO", "missing.png", None, "cannot be read"),
        (
            "branding.logo",
            "CAPSULEBIN__BRANDING__LOGO",
            "large.png",
            Some(&large[..]),
            "256 KiB",
        ),
        (
            "branding.logo",
            "CAPSULEBIN__BRANDING__LOGO",
            "logo.svg",
            Some(&svg[..]),
            "only PNG or WebP",
        ),
        (
            "branding.logo",
            "CAPSULEBIN__BRANDING__LOGO",
            "logo.jpg",
            Some(PNG),
            "only PNG or WebP",
        ),
        (
            "branding.logo",
            "CAPSULEBIN__BRANDING__LOGO",
            "fake.webp",
            Some(ICO),
            "WebP signature",
        ),
        (
            "branding.favicon",
            "CAPSULEBIN__BRANDING__FAVICON",
            "favicon.svg",
            Some(&svg[..]),
            "only PNG or ICO",
        ),
        (
            "branding.favicon",
            "CAPSULEBIN__BRANDING__FAVICON",
            "fake.png",
            Some(&svg[..]),
            "PNG signature",
        ),
    ] {
        let path = match bytes {
            Some(bytes) => write(&dir, file, bytes),
            None => dir.join(file),
        };
        let path = path.to_str().unwrap();
        assert_refused(
            &plain,
            &[(env_key, path)],
            &[key, path, expected],
            &["ACCEPTANCE-SECRET", "<svg", "<script"],
        );
    }
}

/// AC3: a file that exists but cannot be read (no permission) is a startup
/// error naming key and path. Skipped when running as root, which reads it
/// anyway.
#[cfg(unix)]
#[test]
fn ac3_an_unreadable_file_stops_check_and_serve() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempdir("branding-acc-unreadable");
    let path = write(&dir, "locked.png", PNG);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&path).is_ok() {
        eprintln!("skipped: the file stays readable (root)");
        return;
    }
    let plain = config_file(&dir, "plain.toml", "");
    let path = path.to_str().unwrap();
    assert_refused(&plain, &[("CAPSULEBIN__BRANDING__LOGO", path)], &["branding.logo", path], &[]);
}

/// `GET path` over a real socket: status line and body.
fn get(port: u16, path: &str) -> Option<(String, Vec<u8>)> {
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").ok()?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).ok()?;
    let split = answer.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&answer[..split]).into_owned();
    Some((head, answer[split + 4..].to_vec()))
}

/// AC1, AC4: every key set through the environment reaches the started
/// binary: its files are served at their hashed URLs with MIME type and
/// immutable caching, and an unknown file is the generic 404.
#[test]
fn ac1_ac4_every_key_from_the_environment_reaches_serve() {
    let dir = tempdir("branding-acc-serve");
    let logo = write(&dir, "logo.png", PNG);
    let dark = write(&dir, "dark.webp", WEBP);
    let favicon = write(&dir, "favicon.ico", ICO);
    let plain = config_file(&dir, "plain.toml", "");
    let port = free_port();
    let env = [
        ("CAPSULEBIN__BRANDING__NAME", "Env Paste"),
        ("CAPSULEBIN__BRANDING__HIDE_NAME", "true"),
        ("CAPSULEBIN__BRANDING__LOGO", logo.to_str().unwrap()),
        ("CAPSULEBIN__BRANDING__LOGO_DARK", dark.to_str().unwrap()),
        ("CAPSULEBIN__BRANDING__FAVICON", favicon.to_str().unwrap()),
        ("CAPSULEBIN__BRANDING__SOURCE_URL", "https://git.example/fork"),
    ];
    let _server = Server(
        serve_command(&plain, port, &env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );

    let deadline = Instant::now() + Duration::from_secs(20);
    let png = branding_url(PNG, "png");
    let (head, body) = loop {
        if let Some(answer) = get(port, &png) {
            break answer;
        }
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head.to_ascii_lowercase().contains("content-type: image/png"), "{head}");
    assert_eq!(body, PNG);

    for (path, bytes, content_type) in [
        (branding_url(WEBP, "webp"), WEBP, "image/webp"),
        (branding_url(ICO, "ico"), ICO, "image/x-icon"),
    ] {
        let (head, body) = get(port, &path).unwrap();
        assert!(head.starts_with("HTTP/1.1 200"), "{path}: {head}");
        assert!(
            head.to_ascii_lowercase().contains(&format!("content-type: {content_type}")),
            "{path}: {head}"
        );
        assert!(
            head.to_ascii_lowercase()
                .contains("cache-control: public, max-age=31536000, immutable"),
            "{path}: {head}"
        );
        assert_eq!(body, bytes, "{path}");
    }

    let (unknown, unknown_body) = get(port, "/no-such-path").unwrap();
    let (other, other_body) = get(port, "/branding/0123456789abcdef.png").unwrap();
    assert!(unknown.starts_with("HTTP/1.1 404"), "{unknown}");
    assert!(other.starts_with("HTTP/1.1 404"), "{other}");
    assert_eq!(other_body, unknown_body);
}
