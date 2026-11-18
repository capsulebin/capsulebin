#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Assertion that the database contains no plaintext.
//!
//! Correctness gate for the zero-knowledge architecture: after creating a paste
//! whose original content is known (the golden vectors' "Hello, CapsuleBin!"),
//! no database column may contain that plaintext. The test reads every column
//! of every row in the `pastes` table directly via sqlx.

use base64::Engine as _;
use sqlx::Row as _;

use capsulebin_server::config::{DatabaseConfig, Synchronous};
use capsulebin_server::domain::NewPaste;
use capsulebin_server::domain::{PasteId, Verifier};
use capsulebin_server::repository::PasteRepository as _;
use capsulebin_server::repository::sqlite::SqlitePasteRepository;

const GOLDEN_PLAINTEXT: &str = "Hello, CapsuleBin!";
const GOLDEN_CIPHERTEXT_B64: &str = "cTaHqj7zr2z_BgON7IeOBaucMCoMVSWupXDXypHRAcuIRQ5yVYr12-PZEe0io67W2bauwsrhBdLdOECCIM9A12uLZ0_PeM-d1ylFQuSaze81JvLGCGCsP5YLBbQgQIMtFh9frLLPXAq5Qo7V_GZggScmbRQLhQbHHDQK_ALB8Dd-sehMQx_U_uh6dC5c6zMkzC77nPlZz-MaNd8WvgDi43c8i4SMNjQYKS-Xuf7hAasEgRqbeIuI8TFla04AQpaZGI44Ffo1AsE-C5XkVg";

#[tokio::test]
async fn db_no_plaintext() {
    let dir = std::env::temp_dir().join("capsulebin-db-no-plaintext");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");

    let db_path = dir.join("capsulebin.db");

    let pool = capsulebin_server::db::connect(&DatabaseConfig {
        path: db_path.clone(),
        busy_timeout_ms: 5_000,
        synchronous: Synchronous::Full,
    })
    .await
    .expect("connect");

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

    // Read every text column from every row and concatenate them into one
    // searchable string.
    let rows = sqlx::query("SELECT * FROM pastes").fetch_all(&pool).await.expect("query");

    let mut all_text = String::new();
    for row in &rows {
        for (i, col) in row.columns().iter().enumerate() {
            // Try to read as string; fall back to hex dump of the raw value.
            if let Ok(s) = row.try_get::<String, _>(i) {
                all_text.push_str(&s);
                all_text.push('\n');
            } else if let Ok(blob) = row.try_get::<Vec<u8>, _>(i) {
                // Convert blob to hex for comparison.
                for b in &blob {
                    use std::fmt::Write;
                    write!(all_text, "{b:02X}").unwrap();
                }
                all_text.push('\n');
            } else if let Ok(n) = row.try_get::<i64, _>(i) {
                use std::fmt::Write;
                write!(all_text, "{n}").unwrap();
                all_text.push('\n');
            }
            let _ = col;
        }
    }

    pool.close().await;

    assert!(!all_text.contains(GOLDEN_PLAINTEXT), "plaintext found in database");

    let hex = GOLDEN_PLAINTEXT.as_bytes().iter().map(|b| format!("{b:02X}")).collect::<String>();
    assert!(!all_text.to_uppercase().contains(&hex), "hex-encoded plaintext found in database");

    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(GOLDEN_PLAINTEXT.as_bytes());
    assert!(!all_text.contains(&b64), "base64url-encoded plaintext found in database");
}
