#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance tests for the open report ceilings against the real binary
//! (package 20261120-body-report-limits): `capsulebin reports list` starts its
//! text output with the open reports and both ceilings, before the table, also
//! without any open report, and says when the global ceiling is reached;
//! `--json` stays the plain array (AC11). Both ceilings are read from the file
//! and from `CAPSULEBIN__ABUSE__…`, and `0` or a negative value stops the binary
//! naming the key (AC6).

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use axum::http::StatusCode;
use capsulebin_server::domain::PasteId;
use support::*;

/// A configuration for the harness's database, with `abuse` appended.
fn config_file(h: &Harness, abuse: &str) -> PathBuf {
    let path = h.dir.join("capsulebin.toml");
    std::fs::write(
        &path,
        format!(
            "[server]\npublic_url = \"https://paste.example\"\nasset_dir = \"{}\"\n\
             [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n[abuse]\n{abuse}",
            h.dir.display(),
            h.dir.join("capsulebin.db").display()
        ),
    )
    .unwrap();
    path
}

fn run(config: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_capsulebin"))
        .arg("--config")
        .arg(config)
        .args(args)
        .envs(env.iter().copied())
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn report(paste_id: &PasteId) -> serde_json::Value {
    serde_json::json!({
        "paste_id": paste_id.encode(),
        "reason": "phishing",
        "description": "Imitates a bank login page.",
    })
}

#[tokio::test]
async fn reports_list_starts_with_the_open_reports_and_both_ceilings() {
    let h = harness_with("ceilings-list", |config| {
        config.abuse.max_open_reports_per_paste = 1;
        config.abuse.max_open_reports = 2;
    })
    .await;
    let config = config_file(&h, "max_open_reports_per_paste = 1\nmax_open_reports = 2\n");

    // No open report yet: the line is printed all the same.
    let empty = stdout(&run(&config, &["reports", "list"], &[]));
    let mut lines = empty.lines();
    assert_eq!(lines.next(), Some("open reports: 0 of 2 (at most 1 per capsule)"), "{empty}");
    assert!(!empty.contains("new reports are not stored"), "{empty}");

    for byte in 1..=3 {
        assert_eq!(
            h.post_with_token("/api/v1/pastes", create_body(&id(byte)), Some(TOKEN))
                .await
                .status,
            StatusCode::CREATED
        );
    }
    h.post("/api/v1/reports", report(&id(1))).await;
    let below = stdout(&run(&config, &["reports", "list"], &[]));
    assert!(below.starts_with("open reports: 1 of 2 (at most 1 per capsule)\n"), "{below}");
    assert!(!below.contains("new reports are not stored"), "{below}");

    // The second report fills the global ceiling, the third is dropped.
    h.post("/api/v1/reports", report(&id(2))).await;
    h.post("/api/v1/reports", report(&id(3))).await;
    let full = stdout(&run(&config, &["reports", "list"], &[]));
    assert!(full.starts_with("open reports: 2 of 2 (at most 1 per capsule)\n"), "{full}");
    let notice = full.find("new reports are not stored").expect("notice at the global ceiling");
    let table = full.find("REPORT ").expect("the table header");
    assert!(notice < table, "the summary must come before the table:\n{full}");
    assert!(full.contains(&id(1).encode()) && full.contains(&id(2).encode()), "{full}");
    assert!(!full.contains(&id(3).encode()), "a dropped report is listed:\n{full}");

    // `--json` stays the plain array, without the summary.
    let json = stdout(&run(&config, &["reports", "list", "--json"], &[]));
    assert!(!json.contains("open reports"), "{json}");
    let parsed: serde_json::Value = serde_json::from_str(json.trim()).expect("JSON");
    assert_eq!(parsed.as_array().map(Vec::len), Some(2), "{json}");

    // The ceilings from the environment replace the file's.
    let from_env = stdout(&run(
        &config,
        &["reports", "list"],
        &[
            ("CAPSULEBIN__ABUSE__MAX_OPEN_REPORTS", "50"),
            ("CAPSULEBIN__ABUSE__MAX_OPEN_REPORTS_PER_PASTE", "5"),
        ],
    ));
    assert!(
        from_env.starts_with("open reports: 2 of 50 (at most 5 per capsule)\n"),
        "{from_env}"
    );
    assert!(!from_env.contains("new reports are not stored"), "{from_env}");
}

#[tokio::test]
async fn a_report_ceiling_below_one_stops_the_binary() {
    let h = harness("ceilings-refused").await;

    for key in ["max_open_reports_per_paste", "max_open_reports"] {
        for bad in ["0", "-1", "2.5"] {
            let config = config_file(&h, &format!("{key} = {bad}\n"));
            let output = run(&config, &["check"], &[]);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{key} = {bad} accepted from the file");
            assert!(stderr.contains(key), "{key} = {bad}: {stderr}");
        }

        let config = config_file(&h, "");
        let env_key = format!("CAPSULEBIN__ABUSE__{}", key.to_uppercase());
        for bad in ["0", "-1", "ten"] {
            let output = run(&config, &["check"], &[(&env_key, bad)]);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(!output.status.success(), "{env_key}={bad} accepted");
            assert!(stderr.contains(key), "{env_key}={bad}: {stderr}");
        }
        let output = run(&config, &["check"], &[(&env_key, "3")]);
        assert!(
            output.status.success(),
            "{env_key}=3 refused: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
