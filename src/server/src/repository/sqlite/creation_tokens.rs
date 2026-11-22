//! SQLite implementation of [`CreationTokenRepository`].
//!
//! No statement here selects a row by its verifier; see the trait for why.

use sqlx::{Row as _, SqlitePool};

use super::verifier_from_blob;
use crate::domain::{ActiveCreationToken, CreationToken, Label, Verifier};
use crate::repository::{CreationTokenRepository, RepositoryError, RevokeOutcome};

#[derive(Debug, Clone)]
pub struct SqliteCreationTokenRepository {
    pool: SqlitePool,
}

impl SqliteCreationTokenRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn db(error: sqlx::Error) -> RepositoryError {
    RepositoryError::Database(error)
}

/// A stored label this code would not have written is corruption, like a
/// verifier of the wrong length. The error names the rule, never the text.
fn label_from_text(text: &str) -> Result<Label, RepositoryError> {
    Label::parse(text).map_err(|source| db(sqlx::Error::Decode(format!("stored label is invalid: {source}").into())))
}

fn row_to_token(row: &sqlx::sqlite::SqliteRow) -> Result<CreationToken, RepositoryError> {
    let label: String = row.try_get("label").map_err(db)?;
    Ok(CreationToken {
        id: row.try_get("id").map_err(db)?,
        label: label_from_text(&label)?,
        created_at: row.try_get("created_at").map_err(db)?,
        last_used_on: row.try_get("last_used_on").map_err(db)?,
        revoked_at: row.try_get("revoked_at").map_err(db)?,
    })
}

impl CreationTokenRepository for SqliteCreationTokenRepository {
    async fn create(&self, verifier: &Verifier, label: &Label, now: i64) -> Result<CreationToken, RepositoryError> {
        let id = ulid::Ulid::generate().to_string();
        sqlx::query("INSERT INTO creation_tokens (id, verifier, label, created_at) VALUES (?1, ?2, ?3, ?4)")
            .bind(&id)
            .bind(verifier.as_bytes().as_slice())
            .bind(label.as_str())
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(db)?;

        Ok(CreationToken {
            id,
            label: label.clone(),
            created_at: now,
            last_used_on: None,
            revoked_at: None,
        })
    }

    async fn list(&self) -> Result<Vec<CreationToken>, RepositoryError> {
        let rows = sqlx::query("SELECT id, label, created_at, last_used_on, revoked_at FROM creation_tokens ORDER BY created_at, id")
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.iter().map(row_to_token).collect()
    }

    async fn revoke(&self, id: &str, now: i64) -> Result<RevokeOutcome, RepositoryError> {
        let result = sqlx::query("UPDATE creation_tokens SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL")
            .bind(id)
            .bind(now)
            .execute(&self.pool)
            .await
            .map_err(db)?;
        if result.rows_affected() == 1 {
            return Ok(RevokeOutcome::Revoked);
        }

        // A revocation is final, so a row that exists now was revoked before.
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM creation_tokens WHERE id = ?1)")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(db)?;
        Ok(if exists {
            RevokeOutcome::AlreadyRevoked
        } else {
            RevokeOutcome::NotFound
        })
    }

    async fn active(&self) -> Result<Vec<ActiveCreationToken>, RepositoryError> {
        let rows = sqlx::query("SELECT id, verifier, last_used_on FROM creation_tokens WHERE revoked_at IS NULL ORDER BY created_at, id")
            .fetch_all(&self.pool)
            .await
            .map_err(db)?;
        rows.iter()
            .map(|row| {
                Ok(ActiveCreationToken {
                    id: row.try_get("id").map_err(db)?,
                    verifier: verifier_from_blob(row.try_get("verifier").map_err(db)?, "verifier")?,
                    last_used_on: row.try_get("last_used_on").map_err(db)?,
                })
            })
            .collect()
    }

    async fn touch_last_use(&self, id: &str, day: i64) -> Result<bool, RepositoryError> {
        let result =
            sqlx::query("UPDATE creation_tokens SET last_used_on = ?2 WHERE id = ?1 AND (last_used_on IS NULL OR last_used_on < ?2)")
                .bind(id)
                .bind(day)
                .execute(&self.pool)
                .await
                .map_err(db)?;
        Ok(result.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DatabaseConfig, Synchronous};
    use crate::domain::capability::matching_index;
    use crate::domain::utc_day_start;

    const NOW: i64 = 1_795_074_774;
    const DAY: i64 = 86_400;

    async fn repository(name: &str) -> (SqliteCreationTokenRepository, SqlitePool) {
        let dir = std::env::temp_dir().join(format!("capsulebin-tokens-{name}-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let pool = crate::db::connect(&DatabaseConfig {
            path: dir.join("capsulebin.db"),
            busy_timeout_ms: 5_000,
            synchronous: Synchronous::Full,
        })
        .await
        .expect("connect");
        (SqliteCreationTokenRepository::new(pool.clone()), pool)
    }

    fn label(text: &str) -> Label {
        Label::parse(text).unwrap()
    }

    #[tokio::test]
    async fn create_stores_an_active_token_with_its_verifier() {
        let (tokens, pool) = repository("create").await;
        let verifier = Verifier::for_creation_token(b"token-a");

        let created = tokens.create(&verifier, &label("alice"), NOW).await.unwrap();
        assert!(ulid::Ulid::from_string(&created.id).is_ok(), "{}", created.id);
        assert_eq!(created.label.as_str(), "alice");
        assert_eq!((created.created_at, created.last_used_on, created.revoked_at), (NOW, None, None));

        let stored: Vec<u8> = sqlx::query_scalar("SELECT verifier FROM creation_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, verifier.as_bytes().to_vec());

        assert_eq!(tokens.list().await.unwrap(), vec![created.clone()]);
        let active = tokens.active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, created.id);
        assert!(active[0].verifier.matches(&verifier));
    }

    #[tokio::test]
    async fn no_label_is_stored_as_empty_text() {
        let (tokens, pool) = repository("no-label").await;
        tokens
            .create(&Verifier::for_creation_token(b"t"), &Label::default(), NOW)
            .await
            .unwrap();
        let stored: String = sqlx::query_scalar("SELECT label FROM creation_tokens")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, "");
    }

    #[tokio::test]
    async fn the_same_verifier_cannot_be_stored_twice() {
        let (tokens, _) = repository("unique").await;
        let verifier = Verifier::for_creation_token(b"same");
        tokens.create(&verifier, &Label::default(), NOW).await.unwrap();
        assert!(tokens.create(&verifier, &Label::default(), NOW).await.is_err());
        assert_eq!(tokens.list().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn list_holds_active_and_revoked_tokens_in_creation_order() {
        let (tokens, _) = repository("list").await;
        let second = tokens
            .create(&Verifier::for_creation_token(b"b"), &label("b"), NOW + 1)
            .await
            .unwrap();
        let first = tokens.create(&Verifier::for_creation_token(b"a"), &label("a"), NOW).await.unwrap();
        assert_eq!(tokens.revoke(&first.id, NOW + 2).await.unwrap(), RevokeOutcome::Revoked);

        let listed = tokens.list().await.unwrap();
        let ids: Vec<&str> = listed.iter().map(|token| token.id.as_str()).collect();
        assert_eq!(ids, [first.id.as_str(), second.id.as_str()]);
        assert_eq!(listed[0].revoked_at, Some(NOW + 2));
        assert_eq!(listed[1].revoked_at, None);
    }

    #[tokio::test]
    async fn revoke_is_final_and_distinguishes_its_outcomes() {
        let (tokens, _) = repository("revoke").await;
        let verifier = Verifier::for_creation_token(b"r");
        let token = tokens.create(&verifier, &Label::default(), NOW).await.unwrap();

        assert_eq!(tokens.revoke(&token.id, NOW + 10).await.unwrap(), RevokeOutcome::Revoked);
        assert!(tokens.active().await.unwrap().is_empty());

        assert_eq!(tokens.revoke(&token.id, NOW + 20).await.unwrap(), RevokeOutcome::AlreadyRevoked);
        assert_eq!(tokens.list().await.unwrap()[0].revoked_at, Some(NOW + 10));

        let unknown = ulid::Ulid::generate().to_string();
        assert_eq!(tokens.revoke(&unknown, NOW).await.unwrap(), RevokeOutcome::NotFound);
        assert_eq!(tokens.revoke("not an id", NOW).await.unwrap(), RevokeOutcome::NotFound);
    }

    /// What authorization does with [`CreationTokenRepository::active`]: the
    /// matched index names the row whose use is recorded.
    #[tokio::test]
    async fn active_verifiers_pick_the_matched_row() {
        let (tokens, _) = repository("active").await;
        let mut created = Vec::new();
        for name in ["a", "b", "c"] {
            created.push(
                tokens
                    .create(&Verifier::for_creation_token(name.as_bytes()), &label(name), NOW)
                    .await
                    .unwrap(),
            );
        }
        tokens.revoke(&created[0].id, NOW).await.unwrap();

        let active = tokens.active().await.unwrap();
        let verifiers: Vec<Verifier> = active.iter().map(|token| token.verifier).collect();
        let index = matching_index(&Verifier::for_creation_token(b"c"), &verifiers).unwrap();
        assert_eq!(active[index].id, created[2].id);
        assert_eq!(matching_index(&Verifier::for_creation_token(b"a"), &verifiers), None);
    }

    #[tokio::test]
    async fn touch_writes_at_most_once_per_day() {
        let (tokens, _) = repository("touch").await;
        let token = tokens
            .create(&Verifier::for_creation_token(b"t"), &Label::default(), NOW)
            .await
            .unwrap();
        let today = utc_day_start(NOW);

        assert!(tokens.touch_last_use(&token.id, today).await.unwrap());
        assert_eq!(tokens.active().await.unwrap()[0].last_used_on, Some(today));

        assert!(!tokens.touch_last_use(&token.id, today).await.unwrap());
        // An older day never moves the value back.
        assert!(!tokens.touch_last_use(&token.id, today - DAY).await.unwrap());
        assert_eq!(tokens.active().await.unwrap()[0].last_used_on, Some(today));

        assert!(tokens.touch_last_use(&token.id, today + DAY).await.unwrap());
        assert_eq!(tokens.active().await.unwrap()[0].last_used_on, Some(today + DAY));

        assert!(!tokens.touch_last_use("unknown", today + 2 * DAY).await.unwrap());
    }
}
