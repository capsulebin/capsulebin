//! Handler tests inside the crate.
//!
//! The integration tests under `tests/` see only the public interface: they
//! cannot give the server a limiter with a small ceiling or look into the
//! repository. Tests here build an [`AppState`] over a fresh database file,
//! may replace any of its parts (e.g. `limiter` with
//! `LimiterState::bounded_for_tests`) and then send real requests through
//! [`router`].

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use tower::ServiceExt as _;

use crate::api::assets::AssetSource;
use crate::config::{Config, CreationMode, DatabaseConfig, ServerConfig};
use crate::domain::clock::FixedClock;
use crate::domain::{PasteId, Verifier};
use crate::{AppState, router};

/// Wall clock time every test starts from, in Unix seconds.
pub(crate) const START: i64 = 1_700_000_000;

/// The public URL of the test server, also the base of `problem+json` types.
pub(crate) const PUBLIC_URL: &str = "https://paste.example";

/// The capabilities a client would derive. Opaque to the server.
pub(crate) const DESCRIBE_TOKEN: &[u8] = b"describe-token-bytes";
pub(crate) const REVEAL_TOKEN: &[u8] = b"reveal-token-bytes";
pub(crate) const DELETE_SECRET: &[u8] = b"delete-secret-bytes";

/// An [`AppState`] in public creation mode over a new database file in a
/// directory of its own (`name` must be unique among the tests), with the
/// wall clock at [`START`]. `adjust` changes the configuration first.
pub(crate) async fn state(name: &str, adjust: impl FnOnce(&mut Config)) -> (AppState, FixedClock) {
    let dir = std::env::temp_dir().join(format!("capsulebin-handler-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let mut config = Config {
        server: ServerConfig {
            public_url: Some(PUBLIC_URL.to_owned()),
            ..ServerConfig::default()
        },
        database: DatabaseConfig {
            path: dir.join("capsulebin.db"),
            ..DatabaseConfig::default()
        },
        ..Config::default()
    };
    config.creation.mode = CreationMode::Public;
    adjust(&mut config);
    let resolved = config.validate().expect("test configuration must be valid");
    let pool = crate::db::connect(&resolved.config.database).await.expect("connect");

    let clock = FixedClock::at(START);
    let state = AppState::new(&resolved, pool, AssetSource::Directory(dir), Arc::new(clock.clone()));
    (state, clock)
}

/// The router over a prepared [`AppState`].
pub(crate) struct TestServer {
    app: Router,
}

impl TestServer {
    pub(crate) fn new(state: AppState) -> Self {
        Self { app: router(state) }
    }

    pub(crate) async fn send(&self, request: Request<Body>) -> Answer {
        let response = self.app.clone().oneshot(request).await.expect("the router is infallible");
        let status = response.status();
        let headers = response.headers().clone();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read the body")
            .to_vec();
        Answer { status, headers, body }
    }

    /// `POST path` with a JSON body and its `Content-Length`.
    pub(crate) async fn post(&self, path: &str, body: &serde_json::Value) -> Answer {
        let body = body.to_string();
        let request = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json")
            .header("content-length", body.len())
            .body(Body::from(body))
            .expect("a valid request");
        self.send(request).await
    }

    /// Creates a paste through the API.
    pub(crate) async fn create(&self, body: &serde_json::Value) -> Answer {
        self.post("/api/v1/pastes", body).await
    }

    /// A retrieve of `id` by a link holder: the right describe token and
    /// `reveal_token` (a wrong one stands for a wrong password).
    pub(crate) async fn retrieve(&self, id: &PasteId, reveal_token: &[u8]) -> Answer {
        let body = serde_json::json!({
            "describe_token": b64(DESCRIBE_TOKEN),
            "reveal_token": b64(reveal_token),
        });
        self.post(&format!("/api/v1/pastes/{}/retrieve", id.encode()), &body).await
    }
}

pub(crate) struct Answer {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

impl Answer {
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("a JSON body")
    }

    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|value| value.to_str().ok())
    }
}

pub(crate) fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A well-formed create request for `id` with `ciphertext` and the
/// capabilities above; adjust the JSON for options.
pub(crate) fn create_body(id: &PasteId, ciphertext: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "id": id.encode(),
        "format_version": 1,
        "public_header": b64(&[0xAA, 0xBB, 0xCC]),
        "ciphertext": b64(ciphertext),
        "reveal_verifier": Verifier::for_reveal_token(REVEAL_TOKEN).encode(),
        "delete_verifier": Verifier::for_delete_secret(DELETE_SECRET).encode(),
        "describe_verifier": Verifier::for_describe_token(DESCRIBE_TOKEN).encode(),
    })
}
