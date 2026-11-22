-- Creation tokens kept in the database, issued and revoked while the server runs.
--
--   * `verifier` holds the domain-separated SHA-256 of the token
--     (`Verifier::for_creation_token`), never the token. A dump of this table
--     grants no right to create. The server never selects a row by this value:
--     it loads every active verifier and compares in constant time.
--   * `id` is a ULID. It is not secret; it names a token for `token revoke` and
--     reveals only the creation time, which is stored anyway.
--   * `label` is the operator's note, `''` when none was given.
--   * `last_used_on` is the Unix seconds of 00:00 UTC of the day the token last
--     authorized a create, or NULL. Day granularity on purpose: it tells a used
--     token from an abandoned one and nothing more. No capsule is ever linked to
--     the token that created it.
--   * `revoked_at` is set once and never cleared. Revoked rows are kept forever
--     (no retention rule): a restore needs them so that a revocation is never
--     undone, and they hold only the hash, the label and dates.

CREATE TABLE creation_tokens (
    id            TEXT PRIMARY KEY NOT NULL,
    verifier      BLOB UNIQUE NOT NULL,
    label         TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    last_used_on  INTEGER,
    revoked_at    INTEGER
);
