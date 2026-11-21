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
//! **One global subscriber, for one test.** A `tracing` subscriber set with
//! `set_default` is thread-local, and a multi-threaded runtime moves tasks between
//! threads, so a thread-local subscriber would capture an arbitrary subset. The
//! full cycle therefore runs on a multi-threaded runtime under the global
//! subscriber, which can be installed only once per process, and this file lives
//! apart from the other suites.
//!
//! The tests of the logged `path` (route templates, served assets, `{unmatched}`)
//! run on a single-threaded runtime under a thread-local subscriber instead: the
//! whole request runs on the test's thread, so the capture holds exactly that
//! test's requests, one at a time.

mod support;

use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use capsulebin_server::api::access_log::UNMATCHED;
use capsulebin_server::config::{KNOWN_LANGUAGES, language_prefix};
use support::*;

/// A `tracing` writer that collects everything into a buffer.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Capture {
    fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }

    /// A capture installed as this thread's subscriber, at TRACE level, for as
    /// long as the guard lives.
    fn local() -> (Self, tracing::subscriber::DefaultGuard) {
        let capture = Self::default();
        let guard = tracing::subscriber::set_default(
            tracing_subscriber::fmt()
                .with_writer(capture.clone())
                .with_max_level(tracing::Level::TRACE)
                .with_ansi(false)
                .finish(),
        );
        (capture, guard)
    }
}

/// One request as the access log saw it.
struct Logged {
    answer: Answer,
    /// Every `path` value in the captured lines, in order.
    paths: Vec<String>,
    /// Everything captured during the request.
    text: String,
}

impl Logged {
    /// The single `path` the request was logged with.
    fn path(&self) -> &str {
        let distinct: BTreeSet<&str> = self.paths.iter().map(String::as_str).collect();
        assert_eq!(
            distinct.len(),
            1,
            "expected exactly one logged path, got {:?}\n\ncaptured:\n{}",
            self.paths,
            self.text
        );
        &self.paths[0]
    }
}

/// Sends `request` with an emptied `capture` (installed on this thread) and
/// collects what was logged for it.
async fn logged(h: &Harness, capture: &Capture, request: Request<Body>) -> Logged {
    capture.clear();
    let answer = h.send(request).await;
    let text = capture.contents();
    // The span renders as `request{method=GET path=/p/{id}}: …`; a path holds no
    // whitespace, and the span's closing `}:` follows it.
    let paths: Vec<String> = text
        .split("path=")
        .skip(1)
        .map(|rest| {
            let value = rest.split_whitespace().next().unwrap_or_default();
            value.strip_suffix("}:").unwrap_or(value).to_owned()
        })
        .collect();
    assert!(!paths.is_empty(), "the request logged no path\n\ncaptured:\n{text}");
    Logged { answer, paths, text }
}

fn get(path: &str) -> Request<Body> {
    Request::builder().uri(path).body(Body::empty()).unwrap()
}

fn post(path: &str, body: Vec<u8>, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("content-length", body.len());
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::from(body)).unwrap()
}

/// Files the asset fallback serves from the harness's asset directory, as the
/// web build puts them into `dist`.
const ASSETS: [&str; 3] = ["/assets/index-Dx7ndYyu.js", "/favicon.svg", "/theme-init.js"];

fn write_assets(h: &Harness) {
    for asset in ASSETS {
        let path = h.dir.join(asset.trim_start_matches('/'));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"/* asset */").unwrap();
    }
}

#[tokio::test]
async fn matched_routes_log_their_template() {
    let (capture, _guard) = Capture::local();
    let logo = tempdir("logging-templates-files").join("logo.png");
    std::fs::write(&logo, PNG).unwrap();
    let h = harness_with("logging-templates", |config| config.branding.logo = Some(logo)).await;
    let paste = id(31).encode();
    assert_eq!(h.post("/api/v1/pastes", create_body(&id(31))).await.status, StatusCode::CREATED);

    let retrieve = post(
        &format!("/api/v1/pastes/{paste}/retrieve"),
        retrieve_body(REVEAL_TOKEN).to_string().into_bytes(),
        None,
    );
    let logo_url = branding_url(PNG, "png");
    for (request, template, status) in [
        (retrieve, "/api/v1/pastes/{id}/retrieve", StatusCode::OK),
        (get(&format!("/de/p/{paste}")), "/de/p/{id}", StatusCode::OK),
        (get(&format!("/de/manage/{paste}")), "/de/manage/{id}", StatusCode::OK),
        (get(&format!("/en/p/{paste}")), "/en/p/{id}", StatusCode::FOUND),
        (get("/health/ready"), "/health/ready", StatusCode::OK),
        (get(&logo_url), logo_url.as_str(), StatusCode::OK),
    ] {
        let logged = logged(&h, &capture, request).await;
        assert_eq!(logged.answer.status, status, "for {template}");
        assert_eq!(logged.path(), template);
        assert!(
            !logged.text.contains(&paste),
            "the paste id reached the log\n\ncaptured:\n{}",
            logged.text
        );
    }
}

#[tokio::test]
async fn unmatched_paths_never_reach_the_log() {
    let (capture, _guard) = Capture::local();
    let h = harness_with("logging-unmatched", |_| {}).await;
    write_assets(&h);

    // Served assets are named: their path is a file of the asset source.
    for asset in ASSETS {
        let logged = logged(&h, &capture, get(asset)).await;
        assert_eq!(logged.answer.status, StatusCode::OK, "for {asset}");
        assert_eq!(logged.path(), asset);
    }

    // Everything the fallback refuses is one fixed label.
    const MARKER: &str = "UNMATCHEDxMARKERq7";
    for path in [
        format!("/{MARKER}.js"),
        format!("/assets/{MARKER}.css"),
        format!("/de/{MARKER}.png"),
        format!("/x/{MARKER}.json/y"),
        // A branding file's shape on an instance without that file.
        "/branding/0123456789abcdef.webp".to_owned(),
        format!("/{MARKER}.webp"),
        // A shell exists, but is never served through the fallback.
        "/de/index.html".to_owned(),
    ] {
        let logged = logged(&h, &capture, get(&path)).await;
        assert_eq!(logged.answer.status, StatusCode::NOT_FOUND, "for {path}");
        assert_eq!(logged.path(), UNMATCHED, "for {path}");
        for needle in [MARKER, "0123456789abcdef", "index.html"] {
            assert!(
                !logged.text.contains(needle),
                "{needle:?} from {path} reached the log\n\ncaptured:\n{}",
                logged.text
            );
        }
    }
}

#[tokio::test]
async fn rejections_log_the_route_template() {
    let (capture, _guard) = Capture::local();
    let paste = id(32).encode();
    let describe = format!("/api/v1/pastes/{paste}/describe");
    let describe_request = || post(&describe, describe_body(DESCRIBE_TOKEN).to_string().into_bytes(), None);

    // `429` from the limiter.
    let limited = harness_with("logging-rejections-429", |config| {
        config.rate_limits.descriptor.requests_per_minute = 1;
        config.rate_limits.descriptor.burst = 1;
    })
    .await;
    assert_eq!(limited.send(describe_request()).await.status, StatusCode::NOT_FOUND);
    let too_many = logged(&limited, &capture, describe_request()).await;
    assert_eq!(too_many.answer.status, StatusCode::TOO_MANY_REQUESTS);

    // `413` from the route's proof limit and from the global body limit.
    let h = harness("logging-rejections").await;
    let mut oversized = Vec::new();
    for size in [8 * 1024, 4 * 1024 * 1024] {
        let logged = logged(&h, &capture, post(&describe, vec![b' '; size], None)).await;
        assert_eq!(logged.answer.status, StatusCode::PAYLOAD_TOO_LARGE, "for {size} bytes");
        oversized.push(logged);
    }

    // `401` on create without a token in `token` mode.
    let unauthorized = logged(
        &h,
        &capture,
        post("/api/v1/pastes", create_body(&id(32)).to_string().into_bytes(), None),
    )
    .await;
    assert_eq!(unauthorized.answer.status, StatusCode::UNAUTHORIZED);

    let templates = [too_many.path(), oversized[0].path(), oversized[1].path(), unauthorized.path()];
    assert_eq!(
        templates,
        [
            "/api/v1/pastes/{id}/describe",
            "/api/v1/pastes/{id}/describe",
            "/api/v1/pastes/{id}/describe",
            "/api/v1/pastes",
        ]
    );
    for logged in [&too_many, &oversized[0], &oversized[1], &unauthorized] {
        assert!(
            !logged.text.contains(&paste),
            "the paste id reached the log\n\ncaptured:\n{}",
            logged.text
        );
    }
}

/// A small seeded generator, so a failure is reproducible without a dependency.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// Every route template the router registers on an instance with the default
/// configuration (no branding file).
fn route_templates() -> BTreeSet<String> {
    let mut templates: BTreeSet<String> = [
        "/api/v1/pastes",
        "/api/v1/pastes/{id}",
        "/api/v1/pastes/{id}/describe",
        "/api/v1/pastes/{id}/retrieve",
        "/api/v1/reports",
        "/health/live",
        "/health/ready",
    ]
    .map(str::to_owned)
    .into();
    let pages = ["/", "/p/{id}", "/manage/{id}", "/report", "/imprint", "/privacy", "/terms"];
    for language in KNOWN_LANGUAGES {
        let prefix = language_prefix(capsulebin_server::config::DEFAULT_LANGUAGE, language);
        templates.insert(format!("/{language}"));
        for page in pages {
            templates.insert(format!("{prefix}{page}"));
            templates.insert(format!("/{language}{page}"));
        }
    }
    templates
}

#[tokio::test]
async fn random_paths_log_only_templates_assets_or_unmatched() {
    const REQUESTS: usize = 600;
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._~%-";
    const LITERALS: &[&str] = &[
        "api",
        "v1",
        "pastes",
        "describe",
        "retrieve",
        "reports",
        "health",
        "live",
        "ready",
        "p",
        "manage",
        "report",
        "imprint",
        "privacy",
        "terms",
        "assets",
        "branding",
        "en",
        "de",
        "favicon.svg",
        "theme-init.js",
        "index-Dx7ndYyu.js",
    ];
    const EXTENSIONS: &[&str] = &["", ".js", ".css", ".png", ".webp", ".svg", ".ico", ".json", ".html", ".map"];

    let (capture, _guard) = Capture::local();
    let h = harness_with("logging-random", |_| {}).await;
    write_assets(&h);
    let templates = route_templates();
    let is_asset = |path: &str| ASSETS.contains(&path) && h.dir.join(path.trim_start_matches('/')).is_file();

    let mut rng = XorShift(0x5EED_CAB5_u64);
    let mut sent = 0;
    let mut outcomes = BTreeSet::new();
    while sent < REQUESTS {
        let mut generated = Vec::new();
        let mut path = String::new();
        for _ in 0..1 + rng.below(5) {
            let segment = match rng.below(10) {
                0..=3 => rng.pick(LITERALS).to_owned(),
                4 => id(rng.below(250) as u8).encode(),
                _ => {
                    let stem: String = (0..1 + rng.below(12)).map(|_| CHARSET[rng.below(CHARSET.len())] as char).collect();
                    let segment = format!("{stem}{}", rng.pick(EXTENSIONS));
                    // A random `p` or `de` is a route literal like any other.
                    if !LITERALS.contains(&segment.as_str()) {
                        generated.push(segment.clone());
                    }
                    segment
                }
            };
            path.push('/');
            path.push_str(&segment);
        }
        if rng.below(10) == 0 {
            path.push('/');
        }
        // A `%` without two hex digits makes some paths invalid URIs.
        let Ok(uri) = axum::http::Uri::try_from(path.as_str()) else {
            continue;
        };
        let request = match rng.below(4) {
            0 => post(&path, b"{}".to_vec(), None),
            _ => Request::builder().uri(uri).body(Body::empty()).unwrap(),
        };
        let logged = logged(&h, &capture, request).await;
        sent += 1;

        for logged_path in &logged.paths {
            assert!(
                templates.contains(logged_path) || is_asset(logged_path) || logged_path == UNMATCHED,
                "{path} logged {logged_path:?}, neither a template, a served asset nor {UNMATCHED}"
            );
            outcomes.insert(if templates.contains(logged_path) {
                "template"
            } else if logged_path == UNMATCHED {
                "unmatched"
            } else {
                "asset"
            });
            for segment in &generated {
                assert!(
                    !logged_path.split('/').any(|part| part == segment),
                    "the generated segment {segment:?} of {path} reached the log as {logged_path:?}"
                );
            }
        }
        for segment in generated.iter().filter(|segment| segment.len() >= 8) {
            assert!(
                !logged.text.contains(segment.as_str()),
                "the generated segment {segment:?} of {path} reached the log\n\ncaptured:\n{}",
                logged.text
            );
        }
    }
    // The generator must reach every kind of answer, or the property is vacuous.
    assert_eq!(outcomes.len(), 3, "only {outcomes:?} were reached");
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

    // --- reports dropped at the open report limit: one warning, no details ---
    let capped = harness_with("logging-capped", |config| config.abuse.max_open_reports_per_paste = 1).await;
    let capped_report = |paste_id: String| {
        serde_json::json!({
            "paste_id": paste_id,
            "reason": "spam",
            "description": "REPORT-DESCRIPTION-MARKER",
            "contact": "REPORT-CONTACT-MARKER@example.org",
        })
    };
    let limit_warnings = || capture.contents().matches("open report limit").count();
    capped.post("/api/v1/pastes", create_body(&id(4))).await;
    capped.post("/api/v1/reports", capped_report(id(9).encode())).await;
    capped.post("/api/v1/reports", capped_report(id(4).encode())).await;
    assert_eq!(limit_warnings(), 0, "a report about an unknown id or a stored one logged a warning");
    for _ in 0..5 {
        capped.post("/api/v1/reports", capped_report(id(4).encode())).await;
    }
    assert_eq!(
        limit_warnings(),
        1,
        "reports dropped within a second must log exactly one warning\n\ncaptured:\n{}",
        capture.contents()
    );

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

    let forbidden: [(&str, String); 12] = [
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
        ("capped paste id", id(4).encode()),
        ("unknown reported paste id", id(9).encode()),
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
        "the route template is missing, so the log cannot show which endpoint \
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

// --- Edge cases of the logged path ---

/// AC9: the query string is never part of the template, even when it carries
/// the id, and default-language pages log their unprefixed template.
#[tokio::test]
async fn query_strings_never_reach_the_template() {
    let (capture, _guard) = Capture::local();
    let h = harness_with("logging-acceptance-query", |_| {}).await;
    let paste = id(41).encode();
    for (path, template) in [
        (format!("/p/{paste}?id={paste}"), "/p/{id}"),
        (format!("/manage/{paste}?x={paste}"), "/manage/{id}"),
        (format!("/de/p/{paste}?id={paste}"), "/de/p/{id}"),
        (format!("/health/ready?{paste}"), "/health/ready"),
    ] {
        let logged = logged(&h, &capture, get(&path)).await;
        assert_eq!(logged.path(), template, "for {path}");
        assert!(
            !logged.text.contains(&paste),
            "the id from {path} reached the log\n\ncaptured:\n{}",
            logged.text
        );
    }
}

/// AC11: a create refused for a wrong bearer token logs the route template too,
/// and neither the token nor anything of it.
#[tokio::test]
async fn wrong_creation_token_logs_the_template() {
    let (capture, _guard) = Capture::local();
    let h = harness("logging-acceptance-wrong-token").await;
    let logged = logged(
        &h,
        &capture,
        post(
            "/api/v1/pastes",
            create_body(&id(42)).to_string().into_bytes(),
            Some("WRONGxTOKENxMARKER"),
        ),
    )
    .await;
    assert!(
        matches!(logged.answer.status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
        "unexpected status {}",
        logged.answer.status
    );
    assert_eq!(logged.path(), "/api/v1/pastes");
    assert!(!logged.text.contains("WRONGxTOKENxMARKER"), "captured:\n{}", logged.text);
}

/// AC10 with the embedded asset source: a fallback `404` is `{unmatched}` and
/// no part of the request reaches the log.
#[tokio::test]
async fn embedded_fallback_misses_are_unmatched() {
    let (capture, _guard) = Capture::local();
    let h = harness_with_embedded_assets("logging-acceptance-embedded").await;
    const MARKER: &str = "EMBEDDEDxMARKERk3";
    for path in [
        format!("/{MARKER}.js"),
        format!("/assets/{MARKER}.css"),
        format!("/de/{MARKER}.png"),
        format!("/x/{MARKER}.json/y"),
        format!("/en/{MARKER}"),
    ] {
        let logged = logged(&h, &capture, get(&path)).await;
        assert_eq!(logged.answer.status, StatusCode::NOT_FOUND, "for {path}");
        assert_eq!(logged.path(), UNMATCHED, "for {path}");
        assert!(!logged.text.contains(MARKER), "captured:\n{}", logged.text);
    }
}

/// AC10 with the embedded asset source: a file the build put into `dist` is
/// logged by its path once the fallback served it. Without a web build in the
/// binary (`dist` empty, as in the `Rust` CI job) the same paths are misses
/// and log `{unmatched}`.
#[tokio::test]
async fn embedded_served_assets_are_named() {
    let (capture, _guard) = Capture::local();
    let h = harness_with_embedded_assets("logging-acceptance-embedded-served").await;
    let built = h.get("/health/ready").await.json()["assets"] == "ok";
    for asset in ["/favicon.svg", "/theme-init.js", "/language-init.js"] {
        let logged = logged(&h, &capture, get(asset)).await;
        if built {
            assert_eq!(logged.answer.status, StatusCode::OK, "for {asset}");
            assert_eq!(logged.path(), asset);
        } else {
            assert_eq!(logged.answer.status, StatusCode::NOT_FOUND, "for {asset}");
            assert_eq!(logged.path(), UNMATCHED, "for {asset}");
        }
    }
}
