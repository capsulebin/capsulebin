-- Initial schema.
--
-- Notes that matter for anyone reading a column name and guessing:
--
--   * All timestamps are UTC Unix seconds, stored as INTEGER. Not ISO strings:
--     comparison and index range scans on expiry have to be cheap.
--   * `reveal_verifier` and `delete_verifier` hold SHA-256 of the respective
--     secret, never the secret. A dump of this table grants no rights.
--   * `describe_verifier` is SHA-256 of the describe token, which the client
--     derives from the link secret and the paste id (crypto profile v1).
--     Reading a paste's descriptor (public header and lifecycle) requires it,
--     so that knowing a paste id alone does not reveal whether the paste
--     exists, when it expires, or whether it is burn-after-retrieval or
--     password-protected. Every row carries one: the schema cannot hold a
--     paste without the describe capability.
--   * `ciphertext` is nullable so that a deleted paste can keep a minimal
--     tombstone (id, status, timestamps) without its payload. That is what makes
--     "already retrieved" distinguishable from "never existed" for burn pastes,
--     while everything else stays generically unavailable.
--   * `public_header` is opaque to the server. It is authenticated as additional
--     data by the client's AEAD, so the server must store it byte-exactly.
--   * `last_retrieved_at` marks the most recent *authorized* ciphertext handout,
--     never a mere page view. It says nothing about whether the client managed to
--     decrypt anything. It is deliberately coarse — the writer rounds it down to a
--     configured granularity, so a stream of retrievals costs one write per window
--     instead of one per request. It is never returned by the public API and never
--     correlated with a client address; its only purpose is to let the purge job
--     tell a still-used capsule from an abandoned one.
--   * There is no retrieval counter. Keeping one exact would turn every
--     retrieval into a write transaction with a disk flush, all of them queued
--     on SQLite's single writer, and it would be retrieval metadata the
--     documentation says the server does not keep: the only one is the coarse
--     `last_retrieved_at` the inactivity rule needs.

CREATE TABLE pastes (
    id                    TEXT PRIMARY KEY,
    format_version        INTEGER NOT NULL,
    public_header         BLOB NOT NULL,
    ciphertext            BLOB,
    reveal_verifier       BLOB NOT NULL,
    delete_verifier       BLOB NOT NULL,
    created_at            INTEGER NOT NULL,
    last_retrieved_at     INTEGER,
    expires_at            INTEGER,
    burn_after_retrieval  INTEGER NOT NULL DEFAULT 0
                          CHECK (burn_after_retrieval IN (0, 1)),
    consumed_at           INTEGER,
    payload_size          INTEGER NOT NULL,
    availability_status   TEXT NOT NULL DEFAULT 'active'
                          CHECK (availability_status IN ('active','disabled','deleted')),
    moderation_reason     TEXT,
    moderated_at          INTEGER,
    describe_verifier     BLOB NOT NULL
);

CREATE TABLE abuse_reports (
    id                    TEXT PRIMARY KEY,
    paste_id              TEXT NOT NULL,
    reason_code           TEXT NOT NULL,
    description           TEXT NOT NULL,
    reporter_contact      TEXT,
    created_at            INTEGER NOT NULL,
    status                TEXT NOT NULL DEFAULT 'open'
                          CHECK (status IN ('open','reviewing','closed')),
    handled_at            INTEGER,
    resolution            TEXT
);

CREATE TABLE moderation_actions (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    paste_id              TEXT NOT NULL,
    action                TEXT NOT NULL,
    reason_code           TEXT,
    note                  TEXT,
    created_at            INTEGER NOT NULL
);

-- One index per retention rule the purge job applies. Each rule is a range scan
-- over a single column, so a shared composite index would serve none of them.
CREATE INDEX idx_pastes_expires_at ON pastes(expires_at);
CREATE INDEX idx_pastes_created_at ON pastes(created_at);
CREATE INDEX idx_pastes_last_retrieved_at ON pastes(last_retrieved_at);

-- Drives the moderation queue: open reports, oldest first.
CREATE INDEX idx_abuse_reports_status ON abuse_reports(status, created_at);

-- Never free a paste id.
--
-- The client chooses the id (ADR-011). Delete, burn and moderation keep a
-- tombstone row, so their id stays taken; the retention rules, however, delete
-- the whole row. Without this table the id would then be free, and anyone
-- holding the full link could create a new capsule under the same id with the
-- same link secret: the old link would open different content and no client
-- could notice. Open reports and the moderation history, which name a paste by
-- its id, would likewise attach to that later capsule (REL-005).
--
-- So every deleted row leaves its id here, and `create` refuses a retired id
-- exactly like an occupied one (`409 id_conflict`). A trigger rather than the
-- purge code does it, so no current or future `DELETE FROM pastes` can forget.
--
-- Only the id is kept: no timestamp, verifier, size or status. It is the
-- minimum needed to refuse reuse; anything more would be lifecycle metadata
-- kept forever. Nothing ever deletes from this table - no retention rule, no
-- setting, no command.

CREATE TABLE retired_ids (
    id TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;

CREATE TRIGGER retire_deleted_paste_id
AFTER DELETE ON pastes
BEGIN
    INSERT OR IGNORE INTO retired_ids (id) VALUES (OLD.id);
END;

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

-- Every create in `token` mode reads the active tokens in this order. Revoked
-- rows are kept forever, so without this index a long revocation history would
-- cost each create attempt a scan of the whole table. It serves the scan of
-- the active rows only, never a lookup by verifier.
CREATE INDEX idx_creation_tokens_active ON creation_tokens (created_at, id)
    WHERE revoked_at IS NULL;
