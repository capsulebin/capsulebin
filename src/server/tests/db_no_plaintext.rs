#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Assertion that the database contains no plaintext.
//!
//! Correctness gate for the zero-knowledge architecture: after creating a paste
//! whose original content is known (the golden vectors' "Hello, CapsuleBin!"),
//! no database column may contain that plaintext. The test reads every column
//! of every row in the `pastes` table directly via sqlx.
//!
//! The same holds for creation tokens kept in the database: only their verifier
//! is stored, never the token, in any encoding.

mod support;

use base64::Engine as _;
use sqlx::Row as _;

use capsulebin_server::config::{DatabaseConfig, Synchronous};
use capsulebin_server::domain::{Label, NewPaste};
use capsulebin_server::domain::{PasteId, Verifier};
use capsulebin_server::repository::sqlite::SqlitePasteRepository;
use capsulebin_server::repository::sqlite::creation_tokens::SqliteCreationTokenRepository;
use capsulebin_server::repository::{CreationTokenRepository as _, PasteRepository as _};

const GOLDEN_PLAINTEXT: &str = "Hello, CapsuleBin!";
const GOLDEN_CIPHERTEXT_B64: &str = "cTaHqj7zr2z_BgON7IeOBaucMCoMVSWupXDXypHRAcuIRQ5yVYr12-PZEe0io67W2bauwsrhBdLdOECCIM9A12uLZ0_PeM-d1ylFQuSaze81JvLGCGCsP5YLBbQgQIMtFh9frLLPXAq5Qo7V_GZggScmbRQLhQbHHDQK_ALB8Dd-sehMQx_U_uh6dC5c6zMkzC77nPlZz-MaNd8WvgDi43c8i4SMNjQYKS-Xuf7hAasEgRqbeIuI8TFla04AQpaZGI44Ffo1AsE-C5XkVg";

/// Creation tokens shaped like the ones `capsulebin token` prints (43
/// base64url characters), each with a label.
const CREATION_TOKENS: [(&str, &str); 2] = [
    ("Zr3Lq0d8Ck2mT7wYfP1xVb9sN4hJ6uEaG5oR2iWcQ8k", "alice"),
    ("bV7nM2xK9pL4qR8sT1wY6zA3cF5gH0jD2eU9iO7oP4a", "team laptop"),
];

async fn open(name: &str) -> sqlx::SqlitePool {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    capsulebin_server::db::connect(&DatabaseConfig {
        path: dir.join("capsulebin.db"),
        busy_timeout_ms: 5_000,
        synchronous: Synchronous::Full,
    })
    .await
    .expect("connect")
}

/// Every column of every row of `table` as one searchable string: text as is,
/// blobs as uppercase hex, integers in decimal.
async fn dump_table(pool: &sqlx::SqlitePool, table: &str) -> String {
    use std::fmt::Write as _;

    let rows = sqlx::query(sqlx::AssertSqlSafe(format!("SELECT * FROM {table}")))
        .fetch_all(pool)
        .await
        .expect("query");

    let mut all_text = String::new();
    for row in &rows {
        for i in 0..row.columns().len() {
            if let Ok(s) = row.try_get::<String, _>(i) {
                all_text.push_str(&s);
            } else if let Ok(blob) = row.try_get::<Vec<u8>, _>(i) {
                for b in &blob {
                    write!(all_text, "{b:02X}").unwrap();
                }
            } else if let Ok(n) = row.try_get::<i64, _>(i) {
                write!(all_text, "{n}").unwrap();
            }
            all_text.push('\n');
        }
    }
    all_text
}

/// Every table in the database, so a table added later is searched too.
async fn dump_database(pool: &sqlx::SqlitePool) -> String {
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .fetch_all(pool)
        .await
        .expect("list tables");
    let mut all_text = String::new();
    for table in tables {
        all_text.push_str(&dump_table(pool, &format!("\"{table}\"")).await);
    }
    all_text
}

#[tokio::test]
async fn db_no_plaintext() {
    let pool = open("capsulebin-db-no-plaintext").await;

    let repo = SqlitePasteRepository::new(pool.clone());

    let id = PasteId::from_bytes([0x01u8; 16]);
    let ciphertext = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(GOLDEN_CIPHERTEXT_B64.as_bytes())
        .expect("decode ciphertext");

    let new_paste = NewPaste {
        id,
        format_version: 1,
        public_header: vec![0xAA, 0xBB],
        ciphertext,
        reveal_verifier: Verifier::for_reveal_token(b"reveal"),
        delete_verifier: Verifier::for_delete_secret(b"delete"),
        describe_verifier: Verifier::for_describe_token(b"describe"),
        created_at: 1_000,
        expires_at: Some(2_000),
        burn_after_retrieval: false,
    };

    repo.create(&new_paste).await.expect("create");

    let all_text = dump_database(&pool).await;

    pool.close().await;

    assert_absent(&all_text, GOLDEN_PLAINTEXT.as_bytes(), "plaintext");
}

/// Database creation tokens store only `Verifier::for_creation_token(token)`.
/// No column of any table holds a token as text, hex or base64, also after each
/// token authorized a create.
/// The tokens are stored through the repository, with a label each.
#[tokio::test]
async fn database_creation_tokens_store_only_their_verifier() {
    let h = support::harness("db-no-plaintext-tokens").await;
    let pool = h.pool.clone();
    let tokens = SqliteCreationTokenRepository::new(pool.clone());

    for (byte, (token, label)) in (1..).zip(CREATION_TOKENS) {
        let label = Label::parse(label).expect("valid label");
        let created = tokens
            .create(&Verifier::for_creation_token(token.as_bytes()), &label, 1_000)
            .await
            .expect("create token");

        let stored: Vec<u8> = sqlx::query_scalar("SELECT verifier FROM creation_tokens WHERE id = ?1")
            .bind(&created.id)
            .fetch_one(&pool)
            .await
            .expect("stored verifier");
        assert_eq!(stored, Verifier::for_creation_token(token.as_bytes()).as_bytes().to_vec());

        let answer = h
            .post_with_token("/api/v1/pastes", support::create_body(&support::id(byte)), Some(token))
            .await;
        assert_eq!(answer.status, axum::http::StatusCode::CREATED, "{}", answer.text());
        let last_used: Option<i64> = sqlx::query_scalar("SELECT last_used_on FROM creation_tokens WHERE id = ?1")
            .bind(&created.id)
            .fetch_one(&pool)
            .await
            .expect("stored day of last use");
        assert!(last_used.is_some(), "the create was not authorized by the database token");
    }

    let all_text = dump_database(&pool).await;
    pool.close().await;

    // The labels are stored, so the dump is known to cover the token rows.
    for (token, label) in CREATION_TOKENS {
        assert!(all_text.contains(label), "label {label:?} missing from the dump");
        assert_absent(&all_text, token.as_bytes(), "creation token");
    }
}

/// `secret` appears in `all_text` neither as text, nor as hex, nor as base64
/// (standard or base64url, with or without padding).
fn assert_absent(all_text: &str, secret: &[u8], what: &str) {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};

    if let Ok(text) = std::str::from_utf8(secret) {
        assert!(!all_text.contains(text), "{what} found in database");
    }
    let hex = secret.iter().map(|b| format!("{b:02X}")).collect::<String>();
    assert!(!all_text.to_uppercase().contains(&hex), "hex-encoded {what} found in database");
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        let encoded = engine.encode(secret);
        assert!(!all_text.contains(&encoded), "base64-encoded {what} found in database");
    }
}
