// Integration tests are separate crates without `#[cfg(test)]`, so
// `allow-unwrap-in-tests` from clippy.toml does not apply. A failing setup should
// abort loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, dead_code)]

//! Shared setup for the HTTP integration tests.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use capsulebin_server::api::assets::AssetSource;
use capsulebin_server::config::{Config, CreationConfig, CreationMode, DatabaseConfig, PastesConfig, PurgeConfig, ServerConfig};
use capsulebin_server::domain::clock::FixedClock;
use capsulebin_server::domain::{PasteId, Verifier};
use capsulebin_server::{AppState, router};
use tower::ServiceExt as _;

/// Wall clock instant every test starts from. A fixed value keeps expiry
/// arithmetic readable in assertions.
pub const START: i64 = 1_700_000_000;

/// The creation token the test harness uses.
pub const TOKEN: &str = "test-creation-token";

pub struct Harness {
    pub app: axum::Router,
    pub clock: FixedClock,
    pub dir: PathBuf,
    /// A second handle on the same database. The retrieval bookkeeping is
    /// deliberately absent from every response, so the only way to assert that a
    /// request did or did not touch it is to look at the stored row.
    pub pool: sqlx::SqlitePool,
}

pub fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("capsulebin-api-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(dir: &Path, mode: CreationMode, max_plaintext_size: capsulebin_server::config::units::ByteSize) -> Config {
    Config {
        server: ServerConfig {
            bind: "127.0.0.1:0".to_owned(),
            public_url: Some("https://paste.example".to_owned()),
            asset_dir: Some(dir.to_path_buf()),
            trusted_proxies: Vec::new(),
        },
        database: DatabaseConfig {
            path: dir.join("capsulebin.db"),
            ..DatabaseConfig::default()
        },
        pastes: PastesConfig {
            max_plaintext_size,
            ..PastesConfig::default()
        },
        purge: PurgeConfig::default(),
        creation: CreationConfig {
            mode,
            token_hashes: match mode {
                CreationMode::Token => {
                    vec![format!("sha256:{}", Verifier::for_creation_token(TOKEN.as_bytes()).encode())]
                }
                _ => Vec::new(),
            },
        },
        rate_limits: Default::default(),
        abuse: Default::default(),
        legal: Default::default(),
    }
}

/// Builds a server with a real SQLite file and a controllable clock.
pub async fn harness(name: &str) -> Harness {
    harness_with_mode(name, CreationMode::Token).await
}

/// A server whose plaintext budget is not artificially small, for tests that need
/// to exercise payload sizes above the small default the other harnesses use.
pub async fn harness_with_plaintext_limit(name: &str, max_plaintext_size: capsulebin_server::config::units::ByteSize) -> Harness {
    let dir = tempdir(name);
    write_shells(&dir);

    let config = config(&dir, CreationMode::Token, max_plaintext_size);
    let resolved = config.validate().expect("test configuration must be valid");
    let pool = capsulebin_server::db::connect(&resolved.config.database).await.expect("connect");

    let clock = FixedClock::at(START);
    let state = AppState::new(
        &resolved,
        pool.clone(),
        AssetSource::Directory(dir.clone()),
        Arc::new(clock.clone()),
    );

    Harness {
        app: router(state),
        clock,
        dir,
        pool,
    }
}

/// A server on a database restored from an SQL dump of an earlier release, with
/// the clock at `now`. The dump is loaded before the server connects, so the
/// connect applies this build's migrations on top of it, as an upgrade does.
pub async fn harness_from_dump(name: &str, dump: &'static str, now: i64) -> Harness {
    let dir = tempdir(name);
    write_shells(&dir);

    let config = config(&dir, CreationMode::Token, capsulebin_server::config::units::ByteSize(64 * 1024));
    let resolved = config.validate().expect("test configuration must be valid");

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&resolved.config.database.path)
        .create_if_missing(true);
    let seed = sqlx::SqlitePool::connect_with(options).await.expect("open the database to seed");
    sqlx::raw_sql(dump).execute(&seed).await.expect("load the dump");
    seed.close().await;

    let pool = capsulebin_server::db::connect(&resolved.config.database)
        .await
        .expect("the migrations must apply to the dumped database");

    let clock = FixedClock::at(now);
    let state = AppState::new(
        &resolved,
        pool.clone(),
        AssetSource::Directory(dir.clone()),
        Arc::new(clock.clone()),
    );

    Harness {
        app: router(state),
        clock,
        dir,
        pool,
    }
}

/// What the web build writes into the `<head>` of every shell: all known
/// languages and the default.
pub const LANGUAGES_META: &str = r#"<meta name="capsulebin-languages" content="en de" data-default="en">"#;

/// A legal section configured as Markdown text.
pub fn legal_text(markdown: &str) -> Option<capsulebin_server::config::LegalSection> {
    Some(capsulebin_server::config::LegalSection {
        text: Some(markdown.to_owned()),
        ..Default::default()
    })
}

/// The headers of an answer without the one that differs per request.
pub fn stable_headers(answer: &Answer) -> axum::http::HeaderMap {
    let mut headers = answer.headers.clone();
    headers.remove("x-request-id");
    headers
}

/// The asset directory doubles as the shell source, so the tests do not depend on
/// a frontend build having happened. Like the build, it has the shells of every
/// known language under `<language>/`, each with the language list in its head.
pub fn write_shells(dir: &Path) {
    for language in capsulebin_server::config::KNOWN_LANGUAGES {
        let shells = dir.join(language);
        std::fs::create_dir_all(&shells).unwrap();
        let head = format!(r#"<!doctype html><html lang="{language}"><head>{LANGUAGES_META}"#);
        for (file, marker) in [
            ("index.html", "create"),
            ("p.html", "view"),
            ("manage.html", "manage"),
            ("report.html", "report"),
        ] {
            std::fs::write(
                shells.join(file),
                format!("{head}<title>{marker}</title></head><footer><!--capsulebin:legal-links--></footer>"),
            )
            .unwrap();
        }
        std::fs::write(
            shells.join("legal.html"),
            format!(
                "{head}<title><!--capsulebin:legal-title--></title></head><header><!--capsulebin:legal-title--></header>\
                 <main><!--capsulebin:legal-content--></main><footer><!--capsulebin:legal-links--></footer>"
            ),
        )
        .unwrap();
    }
}

/// A server whose asset directory has no shells, for the degraded readiness path.
pub async fn harness_without_assets(name: &str) -> Harness {
    let harness = harness_with_mode(name, CreationMode::Token).await;
    for language in capsulebin_server::config::KNOWN_LANGUAGES {
        let _ = std::fs::remove_dir_all(harness.dir.join(language));
    }
    harness
}

pub async fn harness_with_mode(name: &str, mode: CreationMode) -> Harness {
    harness_with(name, |config| config.creation.mode = mode).await
}

/// A token-mode harness with one configuration change applied, for tests about a
/// specific setting.
pub async fn harness_with(name: &str, adjust: impl FnOnce(&mut Config)) -> Harness {
    let dir = tempdir(name);

    write_shells(&dir);

    // Small enough that the oversize test does not have to allocate 10 MiB.
    let mut config = config(&dir, CreationMode::Token, capsulebin_server::config::units::ByteSize(64 * 1024));
    adjust(&mut config);
    if config.creation.mode != CreationMode::Token {
        config.creation.token_hashes.clear();
    }
    let resolved = config.validate().expect("test configuration must be valid");
    let pool = capsulebin_server::db::connect(&resolved.config.database).await.expect("connect");

    let clock = FixedClock::at(START);
    let state = AppState::new(
        &resolved,
        pool.clone(),
        AssetSource::Directory(dir.clone()),
        Arc::new(clock.clone()),
    );

    Harness {
        app: router(state),
        clock,
        dir,
        pool,
    }
}

pub struct Answer {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Vec<u8>,
}

impl Answer {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|error| panic!("response is not JSON ({error}): {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn code(&self) -> String {
        self.json()["code"].as_str().unwrap_or_default().to_owned()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// A response header as text, if present.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

impl Harness {
    pub async fn send(&self, request: Request<Body>) -> Answer {
        let response = self.app.clone().oneshot(request).await.unwrap();

        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), 32 * 1024 * 1024).await.unwrap().to_vec();

        Answer { status, headers, body }
    }

    pub async fn get(&self, path: &str) -> Answer {
        self.get_with_headers(path, &[]).await
    }

    pub async fn get_with_headers(&self, path: &str, headers: &[(&str, &str)]) -> Answer {
        let mut builder = Request::builder().uri(path);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        self.send(builder.body(Body::empty()).unwrap()).await
    }

    /// The descriptor of `id` (base64url), with the harness's describe token.
    pub async fn describe(&self, id: &str) -> Answer {
        self.post_with_token(&format!("/api/v1/pastes/{id}/describe"), describe_body(DESCRIBE_TOKEN), None)
            .await
    }

    pub async fn post(&self, path: &str, body: serde_json::Value) -> Answer {
        self.post_with_token(path, body, Some(TOKEN)).await
    }

    pub async fn post_with_token(&self, path: &str, body: serde_json::Value, token: Option<&str>) -> Answer {
        self.post_with_headers(path, body, token, &[]).await
    }

    /// A POST with a `Content-Length`, as every real client sends it (the
    /// create byte quota depends on it), plus extra headers.
    pub async fn post_with_headers(&self, path: &str, body: serde_json::Value, token: Option<&str>, headers: &[(&str, &str)]) -> Answer {
        let body = body.to_string();
        let mut builder = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("content-length", body.len());

        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }

        self.send(builder.body(Body::from(body)).unwrap()).await
    }

    /// The retrieval bookkeeping as stored. Read straight from the table, because
    /// no endpoint returns it.
    pub async fn bookkeeping(&self, id: &PasteId) -> Bookkeeping {
        let (last_retrieved_at, consumed_at) =
            sqlx::query_as::<_, (Option<i64>, Option<i64>)>("SELECT last_retrieved_at, consumed_at FROM pastes WHERE id = ?1")
                .bind(id.encode())
                .fetch_one(&self.pool)
                .await
                .expect("the row must exist");

        Bookkeeping {
            last_retrieved_at,
            consumed_at,
        }
    }

    pub async fn delete(&self, path: &str, body: serde_json::Value) -> Answer {
        self.send(
            Request::builder()
                .method("DELETE")
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }
}

/// Server-side retrieval bookkeeping. Never part of a response; see
/// `descriptor_never_exposes_the_retrieval_bookkeeping`.
#[derive(Debug, PartialEq, Eq)]
pub struct Bookkeeping {
    pub last_retrieved_at: Option<i64>,
    pub consumed_at: Option<i64>,
}

// --- capsule fixtures ---

/// A distinct id per test, so a leftover database file cannot cause a collision.
pub fn id(byte: u8) -> PasteId {
    PasteId::from_bytes([byte; 16])
}

pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The reveal token a client would derive. Opaque to the server.
pub const REVEAL_TOKEN: &[u8] = b"reveal-token-bytes";
pub const DELETE_SECRET: &[u8] = b"delete-secret-bytes";
/// The describe token a client would derive from the link secret and the id.
pub const DESCRIBE_TOKEN: &[u8] = b"describe-token-bytes";

/// Ciphertext used by the tests. Contains bytes that are not valid UTF-8 and a
/// recognisable marker, so a test can assert it never appears in a log.
pub const MARKER: &[u8] = b"PLAINTEXT-MARKER-DO-NOT-LOG";

pub fn ciphertext() -> Vec<u8> {
    let mut bytes = vec![0x00, 0xFF, 0xFE];
    bytes.extend_from_slice(MARKER);
    bytes.extend_from_slice(&[0x80, 0xC0, 0x00]);
    bytes
}

/// A well-formed creation body.
pub fn create_body(id: &PasteId) -> serde_json::Value {
    serde_json::json!({
        "id": id.encode(),
        "format_version": 1,
        "public_header": b64(&[0xAA, 0xBB, 0xCC]),
        "ciphertext": b64(&ciphertext()),
        "reveal_verifier": Verifier::for_reveal_token(REVEAL_TOKEN).encode(),
        "delete_verifier": Verifier::for_delete_secret(DELETE_SECRET).encode(),
        "describe_verifier": Verifier::for_describe_token(DESCRIBE_TOKEN).encode(),
    })
}

pub fn describe_body(token: &[u8]) -> serde_json::Value {
    serde_json::json!({ "describe_token": b64(token) })
}

/// A retrieve from someone holding the link: the right describe token, and
/// `token` as the reveal token (a wrong one stands for a wrong password).
pub fn retrieve_body(token: &[u8]) -> serde_json::Value {
    retrieve_body_with(DESCRIBE_TOKEN, token)
}

pub fn retrieve_body_with(describe_token: &[u8], reveal_token: &[u8]) -> serde_json::Value {
    serde_json::json!({ "describe_token": b64(describe_token), "reveal_token": b64(reveal_token) })
}

pub fn delete_body(secret: &[u8]) -> serde_json::Value {
    serde_json::json!({ "delete_secret": b64(secret) })
}
