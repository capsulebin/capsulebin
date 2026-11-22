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
use capsulebin_server::domain::{Label, PasteId, Verifier};
use capsulebin_server::repository::CreationTokenRepository as _;
use capsulebin_server::repository::sqlite::creation_tokens::SqliteCreationTokenRepository;
use capsulebin_server::{AppState, router};
use sha2::{Digest as _, Sha256};
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
                    vec![configured_hash(TOKEN)]
                }
                _ => Vec::new(),
            },
        },
        rate_limits: Default::default(),
        abuse: Default::default(),
        legal: Default::default(),
        i18n: Default::default(),
        branding: Default::default(),
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
/// languages, and a token where the server puts the configured default.
const LANGUAGES_META: &str = r#"<meta name="capsulebin-languages" content="en de" data-default="%%capsulebin:default%%">"#;

/// [`LANGUAGES_META`] as a server with the default language `default` serves it.
pub fn languages_meta(default: &str) -> String {
    LANGUAGES_META.replace("%%capsulebin:default%%", default)
}

/// An internal link as the web build writes it into every shell, with a token
/// where the server puts the language prefix.
const REPORT_LINK: &str = r#"<a href="%%capsulebin:prefix%%/report">report</a>"#;

/// [`REPORT_LINK`] as served in a shell under `prefix` (`""` or `/de`).
pub fn report_link(prefix: &str) -> String {
    REPORT_LINK.replace("%%capsulebin:prefix%%", prefix)
}

/// Each possible `i18n.default_language` with the path prefix every known
/// language then has: none for the default language itself.
pub const LAYOUTS: [(&str, [(&str, &str); 2]); 2] = [("en", [("en", ""), ("de", "/de")]), ("de", [("en", "/en"), ("de", "")])];

/// A token-mode harness with `default` as `i18n.default_language`.
pub async fn harness_with_default_language(name: &str, default: &str) -> Harness {
    harness_with(name, |config| config.i18n.default_language = default.to_owned()).await
}

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
/// known language under `<language>/`, each with the language list in its
/// head, an internal link and the footer markers with the labels of its
/// catalogue - and the tokens the server fills in where the build writes them.
/// `legal.html` has the title markers where the real one has them: in
/// `<title>`, the header and the `<h1>` above the content.
pub fn write_shells(dir: &Path) {
    for language in capsulebin_server::config::KNOWN_LANGUAGES {
        let shells = dir.join(language);
        std::fs::create_dir_all(&shells).unwrap();
        let head = format!(r#"<!doctype html><html lang="{language}"><head>{LANGUAGES_META}"#);
        let footer = format!("<nav>{REPORT_LINK}</nav><footer>{}</footer>", legal_link_markers(language));
        for (file, marker) in [
            ("index.html", "create"),
            ("p.html", "view"),
            ("manage.html", "manage"),
            ("report.html", "report"),
        ] {
            std::fs::write(shells.join(file), format!("{head}<title>{marker}</title></head>{footer}")).unwrap();
        }
        std::fs::write(
            shells.join("legal.html"),
            format!(
                "{head}<title><!--capsulebin:legal-title--> · CapsuleBin</title></head><header><!--capsulebin:legal-title--></header>\
                 <main><div class=\"cb-prose cb-legal\"><h1><!--capsulebin:legal-title--></h1><!--capsulebin:legal-content--></div></main>{footer}"
            ),
        )
        .unwrap();
    }
}

/// The footer markers as the web build writes them into the shells of
/// `language`: one per legal page, labelled from the catalogue.
pub fn legal_link_markers(language: &str) -> String {
    let catalogue = catalogue(language);
    ["imprint", "privacy", "terms"]
        .map(|page| {
            let label = catalogue[format!("shell.{page}")].as_str().unwrap();
            format!("<!--capsulebin:legal-link:{page}:{label}-->")
        })
        .concat()
}

/// The catalogue of `language`, as the web build reads it.
pub fn catalogue(language: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("web/src/i18n/{language}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The start of a PNG, a WebP and an ICO file: the server checks the
/// signature, not the image.
pub const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR logo";
pub const WEBP: &[u8] = b"RIFF\x1a\0\0\0WEBPVP8L dark logo";
pub const ICO: &[u8] = b"\0\0\x01\0\x01\0\x10\x10 favicon";

/// The URL a branding file is expected at, computed here rather than taken
/// from the server.
pub fn branding_url(bytes: &[u8], extension: &str) -> String {
    let hash: String = Sha256::digest(bytes)[..8].iter().map(|byte| format!("{byte:02x}")).collect();
    format!("/branding/{hash}.{extension}")
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

/// A server on the assets embedded into this build instead of an asset
/// directory: the shells of the web build, or none if it has not run.
pub async fn harness_with_embedded_assets(name: &str) -> Harness {
    harness_on(name, |_| {}, |_| AssetSource::Embedded).await
}

/// [`harness_with_embedded_assets`] with one configuration change applied.
pub async fn harness_with_embedded_assets_and(name: &str, adjust: impl FnOnce(&mut Config)) -> Harness {
    harness_on(name, adjust, |_| AssetSource::Embedded).await
}

/// A token-mode harness with one configuration change applied, for tests about a
/// specific setting.
pub async fn harness_with(name: &str, adjust: impl FnOnce(&mut Config)) -> Harness {
    harness_on(name, adjust, |dir| AssetSource::Directory(dir.to_path_buf())).await
}

async fn harness_on(name: &str, adjust: impl FnOnce(&mut Config), assets: impl FnOnce(&Path) -> AssetSource) -> Harness {
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
    let state = AppState::new(&resolved, pool.clone(), assets(&dir), Arc::new(clock.clone()));

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

/// The `creation.token_hashes` entry for `token`.
pub fn configured_hash(token: &str) -> String {
    format!("sha256:{}", Verifier::for_creation_token(token.as_bytes()).encode())
}

/// A configuration file for the real binary over the harness's database and
/// shells, with `public_url`, the creation `mode` and `token_hashes`.
pub fn creation_config_file(h: &Harness, public_url: &str, mode: &str, token_hashes: &[String]) -> PathBuf {
    let path = h.dir.join(format!("capsulebin-{mode}-{}.toml", token_hashes.len()));
    let hashes: Vec<String> = token_hashes.iter().map(|hash| format!("\"{hash}\"")).collect();
    std::fs::write(
        &path,
        format!(
            "[server]\npublic_url = \"{public_url}\"\nasset_dir = \"{}\"\n\
             [database]\npath = \"{}\"\n[creation]\nmode = \"{mode}\"\ntoken_hashes = [{}]\n",
            h.dir.display(),
            h.dir.join("capsulebin.db").display(),
            hashes.join(", ")
        ),
    )
    .unwrap();
    path
}

/// Creation tokens kept in the harness's database, written through the
/// repository on the same database file the server reads.
impl Harness {
    fn creation_tokens(&self) -> SqliteCreationTokenRepository {
        SqliteCreationTokenRepository::new(self.pool.clone())
    }

    /// Stores `token` as an active database token with `label`; returns its id.
    pub async fn issue_token(&self, token: &str, label: &str) -> String {
        let label = Label::parse(label).unwrap();
        self.creation_tokens()
            .create(&Verifier::for_creation_token(token.as_bytes()), &label, START)
            .await
            .unwrap()
            .id
    }

    pub async fn revoke_token(&self, id: &str) {
        self.creation_tokens().revoke(id, START).await.unwrap();
    }

    /// `last_used_on` of the token with `id`, read straight from the table.
    pub async fn token_last_used_on(&self, id: &str) -> Option<i64> {
        sqlx::query_scalar("SELECT last_used_on FROM creation_tokens WHERE id = ?1")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }
}

/// How a request body is framed: with a declared `Content-Length`, or
/// streamed without one, so that only reading it can find its size.
#[derive(Clone, Copy, Debug)]
pub enum Framing {
    Declared,
    Streamed,
}

impl Harness {
    /// `method path` with a JSON `body` framed as `framing`, in process. Without
    /// `Content-Length` the body limit layer cannot refuse the request up front
    /// and only reading the body runs into the limit.
    pub async fn send_framed(&self, method: &str, path: &str, body: Vec<u8>, framing: Framing, token: Option<&str>) -> Answer {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Framing::Declared = framing {
            builder = builder.header("content-length", body.len());
        }
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        self.send(builder.body(Body::from(body)).unwrap()).await
    }

    /// `POST path` with a JSON `body` over a real loopback socket, and whether
    /// the server ever polled the body.
    ///
    /// The request asks with `Expect: 100-continue`, and hyper sends `100
    /// Continue` only when the body is first polled - so the interim answer
    /// records the poll, and the body is sent only after it. A server that
    /// answers without it never asked for a single body byte.
    pub async fn post_expecting_continue(&self, path: &str, body: &[u8], framing: Framing, token: Option<&str>) -> (bool, Answer) {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = self.app.clone().into_make_service_with_connect_info::<std::net::SocketAddr>();
        let server = tokio::spawn(async move { axum::serve(listener, app).await });

        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        let mut head = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/json\r\nExpect: 100-continue\r\n"
        );
        head.push_str(&match framing {
            Framing::Declared => format!("Content-Length: {}\r\n", body.len()),
            Framing::Streamed => "Transfer-Encoding: chunked\r\n".to_owned(),
        });
        if let Some(token) = token {
            head.push_str(&format!("Authorization: Bearer {token}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).await.unwrap();

        let timeout = std::time::Duration::from_secs(10);
        let mut received = Vec::new();
        let head_end = loop {
            if let Some(end) = received.windows(4).position(|window| window == b"\r\n\r\n") {
                break end + 4;
            }
            let mut chunk = [0_u8; 4096];
            let read = tokio::time::timeout(timeout, stream.read(&mut chunk))
                .await
                .expect("the server must answer without the body or ask for it")
                .unwrap();
            assert!(read > 0, "connection closed before an answer");
            received.extend_from_slice(&chunk[..read]);
        };

        let polled = received.starts_with(b"HTTP/1.1 100 ");
        if polled {
            received.drain(..head_end);
            match framing {
                Framing::Declared => stream.write_all(body).await.unwrap(),
                Framing::Streamed => {
                    stream.write_all(format!("{:x}\r\n", body.len()).as_bytes()).await.unwrap();
                    stream.write_all(body).await.unwrap();
                    stream.write_all(b"\r\n0\r\n\r\n").await.unwrap();
                }
            }
        }
        tokio::time::timeout(timeout, stream.read_to_end(&mut received))
            .await
            .expect("the server must close the connection after the answer")
            .unwrap();
        server.abort();

        (polled, parse_answer(&received))
    }
}

/// An HTTP/1.1 answer with a `Content-Length` body, as read from a socket.
fn parse_answer(raw: &[u8]) -> Answer {
    let head_end = raw.windows(4).position(|window| window == b"\r\n\r\n").expect("an answer head") + 4;
    let head = std::str::from_utf8(&raw[..head_end]).unwrap();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse::<u16>().unwrap();

    let mut headers = axum::http::HeaderMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').unwrap();
        headers.append(
            axum::http::HeaderName::try_from(name.trim()).unwrap(),
            axum::http::HeaderValue::try_from(value.trim()).unwrap(),
        );
    }

    Answer {
        status: StatusCode::from_u16(status).unwrap(),
        headers,
        body: raw[head_end..].to_vec(),
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
