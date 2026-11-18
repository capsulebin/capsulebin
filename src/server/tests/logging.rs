#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Acceptance test for the logging rules.
//!
//! Logs outlive pastes. A retention period measured in weeks, applied to a file
//! that accidentally contains a reveal token or a paste id, would defeat the whole
//! design — so this is a security test, not a tidiness one.
//!
//! It captures the real `tracing` output of a full request cycle (create, describe,
//! retrieve, delete, plus the failing variants, which is where careless logging
//! usually hides) and asserts that none of the following ever appears: payload
//! bytes, a capability secret, a bearer token, or a full paste id.
//!
//! **One test, one binary, one global subscriber.** A `tracing` subscriber set with
//! `set_default` is thread-local, and a multi-threaded runtime moves tasks between
//! threads, so a thread-local subscriber would capture an arbitrary subset. A
//! global subscriber can be installed only once per process, which is why this file
//! contains exactly one test and lives apart from the other suites.

mod support;

use std::io;
use std::sync::{Arc, Mutex};

use support::*;

/// A `tracing` writer that collects everything into a buffer.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_request_cycle_logs_no_secrets_but_stays_diagnosable() {
    let capture = Capture::default();

    // TRACE level, so nothing is hidden behind a filter that production might
    // apply. If a secret is logged at any level, this test must see it.
    tracing::subscriber::set_global_default(
        tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .finish(),
    )
    .expect("no other subscriber may be installed in this binary");

    let h = harness("logging").await;
    let paste = id(1).encode();

    // --- a complete, successful cycle ---
    let created = h.post("/api/v1/pastes", create_body(&id(1))).await;
    assert_eq!(created.status, axum::http::StatusCode::CREATED);

    h.describe(&paste).await;
    h.post(&format!("/api/v1/pastes/{paste}/retrieve"), retrieve_body(REVEAL_TOKEN))
        .await;

    // --- failure paths ---
    h.post(&format!("/api/v1/pastes/{paste}/retrieve"), retrieve_body(b"wrong-token"))
        .await;
    h.post_with_token("/api/v1/pastes", create_body(&id(2)), Some("wrong-bearer-token"))
        .await;
    h.post("/api/v1/pastes", serde_json::json!({ "nonsense": true })).await;
    h.describe(&id(9).encode()).await;
    h.delete(&format!("/api/v1/pastes/{paste}"), delete_body(b"wrong-secret")).await;
    h.delete(&format!("/api/v1/pastes/{paste}"), delete_body(DELETE_SECRET)).await;

    // --- abuse reports: free text from the public, personal data ---
    let report = |description: &str| {
        serde_json::json!({
            "paste_id": id(3).encode(),
            "reason": "spam",
            "description": description,
            "contact": "REPORT-CONTACT-MARKER@example.org",
        })
    };
    h.post("/api/v1/pastes", create_body(&id(3))).await;
    h.post("/api/v1/reports", report("REPORT-DESCRIPTION-MARKER")).await;
    h.post("/api/v1/reports", report("REPORT-DESCRIPTION-MARKER with a key #v1.c2VjcmV0"))
        .await;

    // --- the shell routes, which also carry an id in the path ---
    h.get(&format!("/p/{paste}")).await;
    h.get(&format!("/manage/{paste}")).await;

    // --- a degraded readiness check, so the diagnostic assertion has something
    //     to find ---
    let degraded = harness_without_assets("logging-degraded").await;
    degraded.get("/health/ready").await;

    let logs = capture.contents();
    assert!(!logs.is_empty(), "nothing was logged - the capture is not wired up");

    // --- nothing sensitive ---

    let forbidden: [(&str, String); 10] = [
        ("payload marker", String::from_utf8_lossy(MARKER).into_owned()),
        ("base64 ciphertext", b64(&ciphertext())),
        ("reveal token", String::from_utf8_lossy(REVEAL_TOKEN).into_owned()),
        ("encoded reveal token", b64(REVEAL_TOKEN)),
        ("delete secret", String::from_utf8_lossy(DELETE_SECRET).into_owned()),
        ("creation token", TOKEN.to_owned()),
        // The one that the default trace layer got wrong: a full paste id.
        // Shortened forms are permitted, the whole value is not.
        ("full paste id", paste.clone()),
        ("reported paste id", id(3).encode()),
        ("report description", "REPORT-DESCRIPTION-MARKER".to_owned()),
        ("report contact", "REPORT-CONTACT-MARKER".to_owned()),
    ];

    for (label, needle) in &forbidden {
        assert!(
            !logs.contains(needle.as_str()),
            "the log contains the {label} ({needle:?})\n\ncaptured:\n{logs}"
        );
    }

    assert!(
        !logs.to_lowercase().contains("bearer"),
        "the log mentions a bearer token\n\ncaptured:\n{logs}"
    );

    assert!(
        !logs.contains(&capsulebin_server::domain::Verifier::for_delete_secret(DELETE_SECRET).encode()),
        "the log contains a delete verifier\n\ncaptured:\n{logs}"
    );

    // --- but still useful ---
    //
    // A logger that records nothing passes every assertion above and is worthless
    // during an incident, so the diagnostic signal is asserted too.

    assert!(
        logs.contains("/api/v1/pastes/{id}"),
        "the redacted route shape is missing, so the log cannot show which endpoint \
         was called\n\ncaptured:\n{logs}"
    );
    assert!(
        logs.contains("status=201") && logs.contains("status=404"),
        "response statuses are missing\n\ncaptured:\n{logs}"
    );
    assert!(
        logs.contains("readiness"),
        "a failed readiness check left no trace\n\ncaptured:\n{logs}"
    );
}
