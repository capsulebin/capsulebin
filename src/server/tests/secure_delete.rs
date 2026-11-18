#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Burned and deleted ciphertext must be gone from the database *file*, not
//! only from the rows.
//!
//! Without `secure_delete`, SQLite unlinks freed content but leaves the bytes in
//! free pages and freeblocks until they happen to be reused - readable by anyone
//! who copies the file. This test searches the raw files after a checkpoint.

use capsulebin_server::config::{DatabaseConfig, Synchronous};
use capsulebin_server::domain::NewPaste;
use capsulebin_server::domain::{PasteId, Verifier};
use capsulebin_server::repository::PasteRepository as _;
use capsulebin_server::repository::sqlite::SqlitePasteRepository;

const MARKER: &[u8] = b"BURNED-CIPHERTEXT-MUST-NOT-SURVIVE";

/// Large enough to need overflow pages, which go to the freelist when the
/// value is cleared, and a small one that lives inside the row's own page.
fn ciphertext(len: usize) -> Vec<u8> {
    MARKER.iter().copied().cycle().take(len).collect()
}

fn paste(byte: u8, ciphertext: Vec<u8>, burn: bool) -> NewPaste {
    NewPaste {
        id: PasteId::from_bytes([byte; 16]),
        format_version: 1,
        public_header: vec![0xAA, 0xBB],
        ciphertext,
        reveal_verifier: Verifier::for_reveal_token(b"reveal"),
        delete_verifier: Verifier::for_delete_secret(b"delete"),
        describe_verifier: Verifier::for_describe_token(b"describe"),
        created_at: 1_000,
        expires_at: None,
        burn_after_retrieval: burn,
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

#[tokio::test]
async fn burned_and_deleted_ciphertext_is_overwritten_in_the_file() {
    let dir = std::env::temp_dir().join("capsulebin-secure-delete");
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

    // Something that stays, so the freed pages are not simply cut off the end.
    repo.create(&paste(9, b"kept".to_vec(), false)).await.expect("create kept");

    let burned = paste(1, ciphertext(64 * 1024), true);
    let deleted = paste(2, ciphertext(300), false);
    repo.create(&burned).await.expect("create burned");
    repo.create(&deleted).await.expect("create deleted");
    repo.create(&paste(8, b"kept too".to_vec(), false)).await.expect("create kept");

    // Make sure the ciphertext really reached the main file before it is
    // removed - otherwise the test would pass for the wrong reason.
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&pool).await.unwrap();
    assert!(
        contains(&std::fs::read(&db_path).unwrap(), MARKER),
        "precondition: the ciphertext is in the database file"
    );

    assert!(repo.retrieve_and_burn(&burned.id, 1_100).await.expect("burn").is_some());
    assert!(repo.delete(&deleted.id).await.expect("delete"));

    // Move everything into the main file and truncate the write-ahead log.
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(&pool).await.unwrap();

    let file = std::fs::read(&db_path).unwrap();
    assert!(
        !contains(&file, MARKER),
        "burned or deleted ciphertext survives in the database file"
    );
    let wal = std::fs::read(dir.join("capsulebin.db-wal")).unwrap_or_default();
    assert!(
        !contains(&wal, MARKER),
        "burned or deleted ciphertext survives in the write-ahead log"
    );
}
