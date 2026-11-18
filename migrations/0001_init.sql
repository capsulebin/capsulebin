-- Initial schema.
--
-- Notes that matter for anyone reading a column name and guessing:
--
--   * All timestamps are UTC Unix seconds, stored as INTEGER. Not ISO strings:
--     comparison and index range scans on expiry have to be cheap.
--   * `reveal_verifier` and `delete_verifier` hold SHA-256 of the respective
--     secret, never the secret. A dump of this table grants no rights.
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
    retrieval_count       INTEGER NOT NULL DEFAULT 0,
    availability_status   TEXT NOT NULL DEFAULT 'active'
                          CHECK (availability_status IN ('active','disabled','deleted')),
    moderation_reason     TEXT,
    moderated_at          INTEGER
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
