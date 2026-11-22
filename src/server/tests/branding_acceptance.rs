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

/// AC1, AC15: a language table the configuration must refuse stops `check`
/// and `serve`, naming the table: an unknown language, the default language,
/// an unknown key in a language table (file or environment), and a tagline
/// too long or with a control character (also from the environment).
#[test]
fn ac1_ac15_refused_language_tables_stop_check_and_serve() {
    let dir = tempdir("branding-acc-languages");
    let long = "x".repeat(81);
    let long_toml = format!("[branding]\ntagline = \"{long}\"\n");
    for (name, branding, env, key) in [
        ("fr.toml", "[branding.fr]\ntagline = \"x\"\n", vec![], "branding.fr"),
        ("en.toml", "[branding.en]\ntagline = \"x\"\n", vec![], "branding.en"),
        (
            "de-default.toml",
            "[i18n]\ndefault_language = \"de\"\n[branding.de]\ntagline = \"x\"\n",
            vec![],
            "branding.de",
        ),
        ("slogan.toml", "[branding.de]\nslogan = \"x\"\n", vec![], "branding.de"),
        ("env-key.toml", "", vec![("CAPSULEBIN__BRANDING__DE__NAME", "x")], "branding.de"),
        ("long.toml", long_toml.as_str(), vec![], "branding.tagline"),
        (
            "env-long.toml",
            "",
            vec![("CAPSULEBIN__BRANDING__TAGLINE", long.as_str())],
            "branding.tagline",
        ),
        (
            "env-control.toml",
            "",
            vec![("CAPSULEBIN__BRANDING__DE__TAGLINE", "Paste\u{7}safely")],
            "branding.de.tagline",
        ),
    ] {
        let config = config_file(&dir, name, branding);
        assert_refused(&config, &env, &[key], &[]);
    }

    // The limits themselves pass: 80 characters, an empty tagline, and a
    // table for a language other than the default one.
    let fine = config_file(
        &dir,
        "fine.toml",
        &format!("[branding]\ntagline = \"{}\"\n[branding.de]\ntagline = \"\"\n", "ä".repeat(80)),
    );
    let output = check(&fine, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let en = config_file(
        &dir,
        "en-other.toml",
        "[i18n]\ndefault_language = \"de\"\n[branding.en]\ntagline = \"x\"\n",
    );
    let output = check(&en, &[]);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

/// The test shell of the create page with the tagline region around the
/// built-in slogan, as the web build writes it.
fn write_tagline_shells(assets: &Path) {
    for (language, builtin) in [("en", "BUILT-IN-EN"), ("de", "BUILT-IN-DE")] {
        std::fs::write(
            assets.join(language).join("index.html"),
            format!(
                "<!doctype html><html lang=\"{language}\"><head><title>create</title></head><body><header>\n\
                 <!--capsulebin:tagline-->\n<span class=\"cb-claim\">{builtin}</span>\n<!--/capsulebin:tagline-->\n\
                 </header></body></html>"
            ),
        )
        .unwrap();
    }
}

/// Starts `serve` and returns the body of `GET path` once it answers.
fn serve_and_get(config: &Path, env: &[(&str, &str)], paths: &[&str]) -> Vec<String> {
    let port = free_port();
    let _server = Server(
        serve_command(config, port, env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    paths
        .iter()
        .map(|path| {
            loop {
                if let Some((head, body)) = get(port, path) {
                    assert!(head.starts_with("HTTP/1.1 200"), "{path}: {head}");
                    break String::from_utf8(body).unwrap();
                }
                assert!(Instant::now() < deadline, "server did not start");
                std::thread::sleep(Duration::from_millis(100));
            }
        })
        .collect()
}

/// AC1, AC15: `CAPSULEBIN__BRANDING__TAGLINE` and
/// `CAPSULEBIN__BRANDING__DE__TAGLINE` reach the started binary: the German
/// page shows the language's tagline, the English one `branding.tagline`,
/// both escaped; an empty language tagline removes the slogan element.
#[test]
fn ac1_ac15_taglines_from_the_environment_reach_serve() {
    let dir = tempdir("branding-acc-tagline-env");
    let config = config_file(&dir, "plain.toml", "");
    write_tagline_shells(&dir.join("assets"));

    let pages = serve_and_get(
        &config,
        &[
            ("CAPSULEBIN__BRANDING__TAGLINE", "Ours & <b>yours</b>"),
            ("CAPSULEBIN__BRANDING__DE__TAGLINE", "Unser \"Slogan\""),
        ],
        &["/", "/de/"],
    );
    let (en, de) = (&pages[0], &pages[1]);
    assert!(
        en.contains(r#"<span class="cb-claim">Ours &amp; &lt;b&gt;yours&lt;/b&gt;</span>"#),
        "{en}"
    );
    assert!(!en.contains("BUILT-IN-EN") && !en.contains("<b>"), "{en}");
    assert!(de.contains(r#"<span class="cb-claim">Unser &quot;Slogan&quot;</span>"#), "{de}");
    assert!(!de.contains("BUILT-IN-DE"), "{de}");

    let pages = serve_and_get(&config, &[("CAPSULEBIN__BRANDING__DE__TAGLINE", "")], &["/", "/de/"]);
    let (en, de) = (&pages[0], &pages[1]);
    assert!(en.contains(r#"<span class="cb-claim">BUILT-IN-EN</span>"#), "{en}");
    assert!(!de.contains("cb-claim") && !de.contains("BUILT-IN-DE"), "{de}");
}

/// Package 20261120-branding-colours, AC1/AC2: an unknown key below
/// `[branding.colors]` or a colour that is not `#rgb`/`#rrggbb`, from the file
/// or the environment, stops `check` and `serve`, naming the full key.
#[test]
fn colors_refused_keys_and_values_stop_check_and_serve() {
    let dir = tempdir("branding-acc-colors");
    for (name, branding, named) in [
        (
            "unknown.toml",
            "[branding.colors.light]\nshadow_card = \"#fff\"\n",
            &["branding.colors.light.shadow_card"][..],
        ),
        ("dim.toml", "[branding.colors.dim]\nbg = \"#fff\"\n", &["branding.colors.dim"][..]),
        (
            "red.toml",
            "[branding.colors.light]\naccent = \"red\"\n",
            &["branding.colors.light.accent", "#rgb or #rrggbb"][..],
        ),
        (
            "integer.toml",
            "[branding.colors.dark]\nbg = 123\n",
            &["branding.colors.dark.bg", "#rgb or #rrggbb"][..],
        ),
    ] {
        let config = config_file(&dir, name, branding);
        assert_refused(&config, &[], named, &[]);
    }

    let plain = config_file(&dir, "plain.toml", "");
    for (env, named) in [
        (
            ("CAPSULEBIN__BRANDING__COLORS__DARK__ACCENT_HOVER", "#5eead"),
            &["branding.colors.dark.accent_hover", "#rgb or #rrggbb"][..],
        ),
        (
            ("CAPSULEBIN__BRANDING__COLORS__LIGHT__TEXT", " #fff"),
            &["branding.colors.light.text", "#rgb or #rrggbb"][..],
        ),
        (
            ("CAPSULEBIN__BRANDING__COLORS__LIGHT__SHADOW_CARD", "#fff"),
            &["branding.colors.light.shadow_card"][..],
        ),
    ] {
        assert_refused(&plain, &[env], named, &[]);
    }
}

/// The contrast warnings of the colours below, as `check` and `serve` word
/// them: the light text on the three grounds and the four tints, and the dark
/// accent text on the dark accent configured from the environment.
const CONTRAST_WARNINGS: [&str; 8] = [
    "light theme: branding.colors.light.text on branding.colors.light.bg has a contrast of 1.09:1, below 4.5:1",
    "light theme: branding.colors.light.text on branding.colors.light.surface has a contrast of 1.00:1, below 4.5:1",
    "light theme: branding.colors.light.text on branding.colors.light.surface_sunken has a contrast of 1.10:1, below 4.5:1",
    "light theme: branding.colors.light.text on branding.colors.light.tint_accent has a contrast of 1.15:1, below 4.5:1",
    "light theme: branding.colors.light.text on branding.colors.light.tint_danger has a contrast of 1.14:1, below 4.5:1",
    "light theme: branding.colors.light.text on branding.colors.light.tint_warning has a contrast of 1.10:1, below 4.5:1",
    "light theme: branding.colors.light.text on branding.colors.light.tint_success has a contrast of 1.13:1, below 4.5:1",
    "dark theme: branding.colors.dark.accent_text on branding.colors.dark.accent has a contrast of 1.42:1, below 4.5:1",
];

const LOW_CONTRAST: &str = "[branding.colors.light]\ntext = \"#FFF\"\n";
const LOW_CONTRAST_ENV: [(&str, &str); 1] = [("CAPSULEBIN__BRANDING__COLORS__DARK__ACCENT_TEXT", "#3ad")];

/// Package 20261120-branding-colours, AC6: `check` prints each contrast
/// warning to stderr, still prints its `ok:` line and exits 0; without colours
/// it warns about nothing.
#[test]
fn colors_contrast_warnings_in_check() {
    let dir = tempdir("branding-acc-contrast-check");
    let config = config_file(&dir, "low.toml", LOW_CONTRAST);
    let output = check(&config, &LOW_CONTRAST_ENV);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stdout.starts_with("ok: ") && stdout.lines().count() == 1, "{stdout}");
    for warning in CONTRAST_WARNINGS {
        assert_eq!(stderr.matches(warning).count(), 1, "{warning}: {stderr}");
    }
    assert_eq!(stderr.matches("warning: ").count(), CONTRAST_WARNINGS.len(), "{stderr}");

    let plain = config_file(&dir, "plain.toml", "");
    let output = check(&plain, &[]);
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("contrast of"));
}

/// Package 20261120-branding-colours, AC6: `serve` starts despite the
/// warnings and logs each once at WARN level.
#[test]
fn colors_contrast_warnings_in_the_log_of_serve() {
    let dir = tempdir("branding-acc-contrast-serve");
    let config = config_file(&dir, "low.toml", LOW_CONTRAST);
    let port = free_port();
    let mut server = Server(
        serve_command(&config, port, &LOW_CONTRAST_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    while get(port, "/health/live").is_none() {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = server.0.kill();
    let _ = server.0.wait();
    let mut log = String::new();
    server.0.stderr.take().unwrap().read_to_string(&mut log).unwrap();

    for warning in CONTRAST_WARNINGS {
        let lines: Vec<&str> = log.lines().filter(|line| line.contains(warning)).collect();
        assert_eq!(lines.len(), 1, "{warning}: {log}");
        assert!(lines[0].contains("WARN"), "{}", lines[0]);
    }
    assert_eq!(log.matches("contrast of").count(), CONTRAST_WARNINGS.len(), "{log}");
}

/// The 18 keys of `[branding.colors.<theme>]`, in the order of `tokens.css`,
/// written out from the acceptance criterion rather than taken from the server.
const COLOR_KEYS: [&str; 18] = [
    "bg",
    "surface",
    "surface_sunken",
    "border",
    "border_strong",
    "text",
    "text_muted",
    "accent",
    "accent_hover",
    "accent_text",
    "danger",
    "warning",
    "success",
    "focus_ring",
    "tint_accent",
    "tint_danger",
    "tint_warning",
    "tint_success",
];

/// Package 20261120-branding-colours, AC1, AC3, AC4: every colour key of both
/// themes set as `CAPSULEBIN__BRANDING__COLORS__<THEME>__<KEY>` (upper-case
/// hex, `#rgb` and `#rrggbb`) reaches the started binary next to a
/// `[branding.de] tagline` from the file: the pages in `en` and `de` link the
/// stylesheet at its content hash, which holds every token normalised to
/// lowercase `#rrggbb` in the four blocks.
#[test]
fn colors_every_key_from_the_environment_reaches_serve() {
    let dir = tempdir("branding-acc-colors-env");
    let config = config_file(&dir, "de.toml", "[branding.de]\ntagline = \"Unser Slogan\"\n");
    let names: Vec<(String, String)> = COLOR_KEYS
        .iter()
        .flat_map(|key| {
            [
                (
                    format!("CAPSULEBIN__BRANDING__COLORS__LIGHT__{}", key.to_ascii_uppercase()),
                    "#ABC".to_owned(),
                ),
                (
                    format!("CAPSULEBIN__BRANDING__COLORS__DARK__{}", key.to_ascii_uppercase()),
                    "#1A2B3C".to_owned(),
                ),
            ]
        })
        .collect();
    let env: Vec<(&str, &str)> = names.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();

    let declarations = |indent: &str, value: &str| -> String {
        COLOR_KEYS
            .iter()
            .map(|key| format!("{indent}--cb-{}: {value};\n", key.replace('_', "-")))
            .collect()
    };
    let expected = format!(
        "@media not all and (prefers-color-scheme: dark) {{\n    :root {{\n{}    }}\n}}\n\
         @media (prefers-color-scheme: dark) {{\n    :root {{\n{}    }}\n}}\n\
         :root[data-theme=\"light\"] {{\n{}}}\n\
         :root[data-theme=\"dark\"] {{\n{}}}\n",
        declarations("        ", "#aabbcc"),
        declarations("        ", "#1a2b3c"),
        declarations("    ", "#aabbcc"),
        declarations("    ", "#1a2b3c"),
    );
    let url = branding_url(expected.as_bytes(), "css");
    let link = format!(r#"<link rel="stylesheet" href="{url}">"#);

    let bodies = serve_and_get(&config, &env, &["/", "/de/", "/de/report", &url]);
    for (path, page) in ["/", "/de/", "/de/report"].iter().zip(&bodies) {
        assert_eq!(page.matches(&link).count(), 1, "{path}: {page}");
        assert!(
            page.contains(&format!("{link}</head>")),
            "{path}: link not right before </head>: {page}"
        );
    }
    assert_eq!(bodies[3], expected);
}
