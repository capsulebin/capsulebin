// Every test starts the binary through `sh` or checks Unix file modes.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance tests for the database files against the real binary (package
//! 20261121-operator-files-restore): a new database and its `-wal`/`-shm` are
//! owner-only whatever the umask (AC1); permissive existing files are reported
//! by `check` and `serve` but left alone (AC2); the write probe follows no
//! planted entry (AC3); a missing database next to restore leftovers is
//! refused, not recreated (AC4).

mod support;

use std::io::{Read as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use support::{tempdir, write_shells};

const BINARY: &str = env!("CARGO_BIN_EXE_capsulebin");

/// A configuration for `<dir>/data/capsulebin.db` with test shells, so
/// `check` finds its assets. Returns the configuration and the database path.
fn setup(name: &str) -> (PathBuf, PathBuf) {
    let dir = tempdir(name);
    let assets = dir.join("assets");
    write_shells(&assets);
    let data = dir.join("data");
    std::fs::create_dir(&data).unwrap();
    let database = data.join("capsulebin.db");
    let config = dir.join("capsulebin.toml");
    std::fs::write(
        &config,
        format!(
            "[server]\npublic_url = \"https://paste.example\"\nasset_dir = \"{}\"\n\
             [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n",
            assets.display(),
            database.display()
        ),
    )
    .unwrap();
    (config, database)
}

fn run(config: &Path, command: &str) -> Output {
    Command::new(BINARY).arg("--config").arg(config).arg(command).output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_ok(output: &Output) {
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).starts_with("ok:"),
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        stderr(output)
    );
}

/// `path` with `suffix` appended to its file name (`-wal`, `-shm`).
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Kills the server when the test ends, also on a failed assertion.
struct Server(std::process::Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Server {
    /// Kills the server and returns what it logged.
    fn stop(mut self) -> String {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let mut log = String::new();
        if let Some(mut stderr) = self.0.stderr.take() {
            stderr.read_to_string(&mut log).unwrap();
        }
        log
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// `serve` started through `sh` with umask `022`, as a typical service would.
fn serve_command(config: &Path, port: u16) -> Command {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("umask 022; exec \"$0\" \"$@\"")
        .arg(BINARY)
        .arg("--config")
        .arg(config)
        .arg("serve")
        .env("CAPSULEBIN__SERVER__BIND", format!("127.0.0.1:{port}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

fn live(port: u16) -> bool {
    let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    if stream
        .write_all(b"GET /health/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut response = String::new();
    let _ = stream.read_to_string(&mut response);
    response.starts_with("HTTP/1.1 200")
}

/// Starts `serve` and waits until it answers.
fn serve(config: &Path) -> Server {
    let port = free_port();
    let mut server = Server(serve_command(config, port).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !live(port) {
        if let Some(status) = server.0.try_wait().unwrap() {
            panic!("serve exited with {status}: {}", server.stop());
        }
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    server
}

/// `serve` that must refuse to start: it exits on its own, non-zero, and its
/// stderr is returned.
fn serve_refused(config: &Path) -> String {
    let mut server = Server(serve_command(config, free_port()).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = server.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "serve started despite the restore leftovers");
        std::thread::sleep(Duration::from_millis(50));
    };
    let log = server.stop();
    assert!(!status.success(), "serve exited with success: {log}");
    log
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

fn set_mode(path: &Path, mode: u32) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// AC1: under umask `022`, the database `serve` creates and its `-wal`/`-shm`
/// are `0600` while the server runs.
#[test]
fn new_database_files_are_private() {
    let (config, database) = setup("db-files-private");
    assert!(!database.exists());

    let server = serve(&config);
    for path in [database.clone(), sidecar(&database, "-wal"), sidecar(&database, "-shm")] {
        assert!(path.exists(), "{} missing while serving", path.display());
        assert_eq!(mode(&path), 0o600, "{}", path.display());
    }
    let log = server.stop();
    assert!(!log.contains("by other users"), "{log}");
}

/// AC2: permissive database files and a directory others can write to are
/// reported, one line each, by `check` (stderr, still `ok:`) and by `serve`
/// (log, still serving). Nothing is changed; `0700` and `0755` directories
/// with `0600` files give no warning.
#[test]
fn permissive_database_files_are_reported() {
    let (config, database) = setup("db-files-permissive");
    let directory = database.parent().unwrap().to_owned();
    let wal = sidecar(&database, "-wal");
    let shm = sidecar(&database, "-shm");

    // A running server keeps the sidecars in place for `check` to see.
    let server = serve(&config);
    let modes = [(&database, 0o640), (&wal, 0o602), (&shm, 0o660), (&directory, 0o770)];
    for (path, permissive) in modes {
        set_mode(path, permissive);
    }

    let output = run(&config, "check");
    assert_ok(&output);
    let log = stderr(&output);
    let warnings: Vec<&str> = log.lines().filter(|line| line.starts_with("warning: ")).collect();
    assert_eq!(warnings.len(), 4, "{log}");
    for (path, expected) in [
        (&database, "0640 and is readable by other users"),
        (&wal, "0602 and is writable by other users"),
        (&shm, "0660 and is readable and writable by other users"),
        (&directory, "0770 and is writable by other users"),
    ] {
        let line = format!("{} has mode {expected}", path.display());
        assert_eq!(
            warnings.iter().filter(|warning| warning.ends_with(&line)).count(),
            1,
            "{line}: {log}"
        );
    }
    for (path, permissive) in modes {
        assert_eq!(mode(path), permissive, "{} changed by check", path.display());
    }
    server.stop();

    // `serve` on the same files logs the same findings as warnings and serves.
    let log = serve(&config).stop();
    for path in [&database, &directory] {
        let lines: Vec<&str> = log
            .lines()
            .filter(|line| line.contains(&format!("{} has mode", path.display())))
            .collect();
        assert_eq!(lines.len(), 1, "{}: {log}", path.display());
        assert!(lines[0].contains("WARN") && lines[0].contains("by other users"), "{}", lines[0]);
    }
    for (path, permissive) in modes {
        if path.exists() {
            assert_eq!(mode(path), permissive, "{} changed by serve", path.display());
        }
    }

    // Owner-only files in a private or a world-readable directory: no warning.
    for path in [&database, &wal, &shm] {
        if path.exists() {
            set_mode(path, 0o600);
        }
    }
    for directory_mode in [0o700, 0o755] {
        set_mode(&directory, directory_mode);
        let output = run(&config, "check");
        assert_ok(&output);
        assert!(!stderr(&output).contains("warning:"), "{directory_mode:o}: {}", stderr(&output));
    }
}

/// AC3: a symlink at the old fixed probe name is not followed, and `check`
/// leaves nothing but the database files behind. An unwritable directory
/// stays an error saying so.
#[test]
fn write_probe_follows_no_planted_entry() {
    let (config, database) = setup("db-files-probe");
    let directory = database.parent().unwrap().to_owned();
    let sentinel = database.parent().unwrap().parent().unwrap().join("sentinel");
    std::fs::write(&sentinel, b"sentinel").unwrap();
    let planted = directory.join(".capsulebin-write-probe");
    std::os::unix::fs::symlink(&sentinel, &planted).unwrap();

    assert_ok(&run(&config, "check"));

    assert_eq!(std::fs::read(&sentinel).unwrap(), b"sentinel");
    assert!(std::fs::symlink_metadata(&planted).unwrap().file_type().is_symlink());
    for name in names(&directory) {
        assert!(
            name == ".capsulebin-write-probe" || name.starts_with("capsulebin.db"),
            "check left {name} behind"
        );
    }

    set_mode(&directory, 0o500);
    // Root writes regardless of the mode bits; nothing to check then.
    let root = std::fs::File::create(directory.join("root-check")).is_ok();
    let output = run(&config, "check");
    set_mode(&directory, 0o700);
    if !root {
        assert!(!output.status.success(), "unwritable directory accepted");
        assert!(stderr(&output).contains("is not writable"), "{}", stderr(&output));
    }
}

/// AC4: a missing database next to a `<file>.pre-restore-*` entry or a
/// `.<file>.restore-*` directory stops `serve`, `check` and `migrate`, naming
/// the leftovers and the way out, and creates no database file. Without
/// leftovers the database is created; with it present, leftovers do not block.
#[test]
fn missing_database_with_restore_leftovers_is_not_recreated() {
    for (case, leftover, is_dir) in [
        ("aside", "capsulebin.db.pre-restore-1700000000", false),
        ("aside-wal", "capsulebin.db.pre-restore-1700000000-wal", false),
        ("stage", ".capsulebin.db.restore-0f1e2d3c", true),
    ] {
        let (config, database) = setup(&format!("db-files-leftover-{case}"));
        let directory = database.parent().unwrap().to_owned();
        let path = directory.join(leftover);
        if is_dir {
            std::fs::create_dir(&path).unwrap();
        } else {
            std::fs::write(&path, b"old database").unwrap();
        }

        let check = run(&config, "check");
        let migrate = run(&config, "migrate");
        for (command, status, message) in [
            ("check", Some(check.status), stderr(&check)),
            ("migrate", Some(migrate.status), stderr(&migrate)),
            ("serve", None, serve_refused(&config)),
        ] {
            if let Some(status) = status {
                assert!(!status.success(), "{case}: {command} succeeded");
            }
            assert!(message.contains(&path.display().to_string()), "{case}: {command}: {message}");
            assert!(message.contains("restore again"), "{case}: {command}: {message}");
            assert!(message.contains("move the old database back"), "{case}: {command}: {message}");
        }
        assert_eq!(names(&directory), [leftover], "{case}: database files created");

        // Once the database is back, the leftover no longer blocks anything.
        let (fresh_config, fresh) = setup(&format!("db-files-leftover-{case}-fresh"));
        assert_ok(&run(&fresh_config, "check"));
        assert!(fresh.exists(), "{case}: no database created without leftovers");
        // `check` may exit before its last connection checkpoints, leaving
        // pages in the `-wal`: copy the sidecars too, or the copy is torn.
        std::fs::copy(&fresh, &database).unwrap();
        for suffix in ["-wal", "-shm"] {
            if sidecar(&fresh, suffix).exists() {
                std::fs::copy(sidecar(&fresh, suffix), sidecar(&database, suffix)).unwrap();
            }
        }
        assert_ok(&run(&config, "check"));
        assert_ok(&run(&config, "migrate"));
        serve(&config).stop();
        assert!(path.exists(), "{case}: leftover removed");
    }
}
