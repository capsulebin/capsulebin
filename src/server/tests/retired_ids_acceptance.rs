#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Acceptance tests for never freeing ids against the real binary (package
//! 20261121-retired-ids, AC5): a capsule is created and reported, then the
//! binary's own `prune` removes it. Afterwards `reports list` and `reports show`
//! print its status as `gone`, `moderation disable`/`delete` refuse it as "no
//! paste" without writing a moderation record, re-creating the id is the plain
//! `409 id_conflict` and a new report is answered `202` but not stored.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use axum::http::StatusCode;
use capsulebin_server::domain::PasteId;
use support::*;

/// A configuration for the harness's database. The harness clock sits at
/// `START` (2023), so the binary's `prune` with the default 30-day
/// never-retrieved rule removes the capsule; the report retention is long
/// enough to keep the report.
fn config_file(h: &Harness) -> PathBuf {
    let path = h.dir.join("capsulebin.toml");
    std::fs::write(
        &path,
        format!(
            "[server]\npublic_url = \"https://paste.example\"\nasset_dir = \"{}\"\n\
             [database]\npath = \"{}\"\n[creation]\nmode = \"public\"\n\
             [abuse]\nreport_retention = \"36500d\"\n",
            h.dir.display(),
            h.dir.join("capsulebin.db").display()
        ),
    )
    .unwrap();
    path
}

fn run(config: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_capsulebin"))
        .arg("--config")
        .arg(config)
        .args(args)
        .output()
        .unwrap()
}

fn describe(output: &Output) -> String {
    format!(
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn stdout(output: &Output) -> String {
    assert!(output.status.success(), "{}", describe(output));
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn report(paste_id: &PasteId) -> serde_json::Value {
    serde_json::json!({
        "paste_id": paste_id.encode(),
        "reason": "phishing",
        "description": "Imitates a bank login page.",
    })
}

async fn count(h: &Harness, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
        .fetch_one(&h.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_capsule_pruned_by_the_binary_stays_gone_for_reports_and_moderation() {
    let h = harness("retired-ids-acceptance").await;
    let config = config_file(&h);
    let paste = id(0x70);

    let created = h.post("/api/v1/pastes", create_body(&paste)).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", created.text());
    assert_eq!(h.post("/api/v1/reports", report(&paste)).await.status, StatusCode::ACCEPTED);
    assert_eq!(count(&h, "abuse_reports").await, 1);

    let pruned = run(&config, &["prune"]);
    assert!(pruned.status.success(), "{}", describe(&pruned));
    assert_eq!(count(&h, "pastes").await, 0, "prune did not remove the capsule");
    let retired: Vec<String> = sqlx::query_scalar("SELECT id FROM retired_ids").fetch_all(&h.pool).await.unwrap();
    assert_eq!(retired, [paste.encode()]);

    // Re-creating the id, with the original request or other content.
    for body in [create_body(&paste), {
        let mut body = create_body(&paste);
        body["ciphertext"] = serde_json::json!(b64(b"different content"));
        body
    }] {
        let again = h.post("/api/v1/pastes", body).await;
        assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.text());
        assert_eq!(again.code(), "id_conflict");
    }
    assert_eq!(count(&h, "pastes").await, 0);

    // `reports list` (text and JSON) and `reports show` print the status as `gone`.
    let list = stdout(&run(&config, &["reports", "list"]));
    let row = list
        .lines()
        .find(|line| line.contains(&paste.encode()))
        .unwrap_or_else(|| panic!("report not listed:\n{list}"));
    assert!(row.trim_end().ends_with("gone"), "{row}");

    let json = stdout(&run(&config, &["reports", "list", "--json"]));
    let parsed: serde_json::Value = serde_json::from_str(json.trim()).expect("JSON");
    let entries = parsed.as_array().expect("an array");
    assert_eq!(entries.len(), 1, "{json}");
    assert_eq!(entries[0]["paste_id"], paste.encode());
    assert_eq!(entries[0]["paste_status"], "gone");
    let report_id = entries[0]["id"].as_str().expect("report id").to_owned();

    let shown = stdout(&run(&config, &["reports", "show", &report_id]));
    assert!(shown.contains(&format!("Paste       {} (gone)", paste.encode())), "{shown}");

    // Moderation answers "no paste" and records nothing.
    for action in ["disable", "delete"] {
        let output = run(&config, &["moderation", action, &paste.encode(), "--reason", "phishing"]);
        assert!(!output.status.success(), "moderation {action} succeeded: {}", describe(&output));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("no paste"), "moderation {action}: {}", describe(&output));
    }
    assert_eq!(count(&h, "moderation_actions").await, 0);
    assert_eq!(count(&h, "pastes").await, 0, "moderation brought a row back");

    // A new report is answered like any other and not stored.
    let again = h.post("/api/v1/reports", report(&paste)).await;
    assert_eq!(again.status, StatusCode::ACCEPTED, "{}", again.text());
    assert_eq!(count(&h, "abuse_reports").await, 1, "a report on a retired id is stored");
}
