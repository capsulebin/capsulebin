# ADR-019 — Creation tokens in the database

**Status:** Accepted

## Context

Invitation links ([ADR-018](ADR-018-creation-token-in-the-browser.md)) make one creation token per
person the normal case. Until this decision every token was a `creation.token_hashes` entry: each
new token meant editing the configuration and restarting the server, and taking one person's access
away meant the same. A `token`-mode instance without any hash refused to start, so tokens could not
be added later either.

Constraints:

- **The server stores verifiers, never secrets, and compares them in constant time** (AGENTS.md
  invariant 3). A database lookup `WHERE verifier = ?` compares in the index, not in constant
  time.
- **Creation is decided on the request head** before a body byte is read (ADR-018, ADR-011).
- **The web client already handles a withdrawn token**: on `401 unauthorized` it forgets the token
  and asks again (ADR-018). A `401` therefore means "this token is gone", never "try again later".
- **Operator commands may write the live database while `serve` runs** (SQLite WAL, one writer at a
  time), as reports and moderation do (ADR-013).
- **Ids are never freed, and a restore never frees one** (ADR-011). A restore must not undo an
  operator's decision either.
- **No accounts and no operator authentication on the web** (ADR-018).

## Decision

**Creation tokens are kept in the database and managed with `capsulebin token create|list|revoke`
while the server runs; every change applies from the next create request. Configured
`creation.token_hashes` keep working next to them.**

- **What is stored.** Migration `0005_creation_tokens.sql` adds `creation_tokens` with `id` (ULID,
  primary key), `verifier` (the 32 bytes of `Verifier::for_creation_token`, the same
  domain-separated hash as a configured entry, `UNIQUE`), `label` (`''` = none), `created_at`,
  `last_used_on` and `revoked_at` (Unix seconds, the last two `NULL` until set). The token itself is
  never stored; a dump of the table grants no right to create.
- **One candidate set, compared in constant time.** In `token` mode `authorize_creation` reads every
  active database verifier on each create and compares the bearer against the configured verifiers
  followed by the database ones with `capability::matching_index`, which visits every entry with
  `subtle` and returns the position of the first match without an early return. The repository has
  no lookup by verifier. A missing, unknown and revoked token all take this path and answer the
  same `401 unauthorized`.
- **Read per create, no cache.** A token issued or revoked by another process applies on the next
  request without any invalidation logic.
- **A failed read is `500 internal_error`, never `401`.** A `401` would make the web client forget
  a token that may be valid. The configured hashes are not checked on their own as a fallback: the
  create would fail writing anyway. The decision still comes before the body is read.
- **`last_used_on` at day granularity.** After a database token authorized a create, and before
  the body is read, the day (Unix seconds of 00:00 UTC, from the server's clock) is written if the
  stored one is missing or earlier — at most one write per token per UTC day, in one conditional
  `UPDATE`, so parallel creates write once. A configured hash and a refused token write nothing.
  A failed write is logged as a warning without the token, its id or label, and the create goes
  on: the token was valid.
- **Capsules are not linked to tokens.** Nothing records which token created which capsule, and
  there is no per-use log beyond `last_used_on`.
- **`token` mode without any token is valid.** `creation.token_hashes` may be empty in `token`
  mode; `serve` (log) and `check` (`warning:` on stderr, still `ok:`, exit 0) warn when neither a
  configured hash nor an active database token exists — every create is refused until one is
  issued — and when active database tokens exist while the mode is `public` or `disabled`, where
  they have no effect. The warnings carry only counts, never a token, hash or label. If the tokens
  cannot be read at startup, `serve` and `check` fail. `token_hashes` with another mode stays a
  startup error.
- **Operator commands.** `token create [--label <text>]`, `token list [--json]` and
  `token revoke <id>` load and validate the configuration like `reports` (so they need
  `server.public_url`) and open the database; they can run while the server is up.
  - `create` checks the label first (at most 64 characters, no control characters; an error names
    `--label` and the rule, never the text, and stores nothing), stores one active row and prints
    the token on line 2 (as plain `token` does), `id: <ULID>`, `label: …` if one was given, and the
    invitation link `<public_url>/#token=<token>` — no `sha256:` hash and no configuration entry,
    which a database token does not need. Outside `token` mode the token is still created, with a
    `note:` on stderr that it has no effect until the mode is `token`.
  - `list` starts with `active tokens: N (configured hashes: M)` and prints every token, active and
    revoked, oldest first: id, created (`YYYY-MM-DD HH:MM UTC`), last used (`YYYY-MM-DD` or
    `never`), revoked (`YYYY-MM-DD HH:MM UTC` or `-`) and the label. `--json` is a plain array of
    `{id, label, created_at, last_used_on, revoked_at}`. Neither shows anything of the verifier.
  - `revoke` takes the id `list` shows (a ULID, case-insensitive, stored upper-case), sets
    `revoked_at` and exits `0`; an already revoked token exits `0` saying so and keeps its
    `revoked_at`; an unknown id exits `1`. A revocation is final.
- **Plain `capsulebin token` is unchanged.** It stays a generator run before configuration: it
  stores nothing and prints the token, the `sha256:` entry and (with a valid `public_url`) the
  invitation link, as ADR-018 describes.
- **Revoked rows are kept forever.** No retention rule removes them; they hold only the hash, the
  label and dates, and a restore needs them.
- **A restore carries tokens over, matched by verifier.** In the same read-only `ATTACH`, probe and
  transaction that retire the replaced database's ids (ADR-011), the staged copy takes from the
  replaced database: every token whose verifier it lacks (with id, label, dates and revocation);
  the revocation of a token revoked there but active in the copy (with the replaced database's
  `revoked_at`); and the later `last_used_on` of the two. Tokens only in the backup stay as they
  are. A replaced database without the table (schema below 5) carries nothing and warns about
  nothing. A failed `ATTACH` or probe is the existing single warning, and then neither ids nor
  tokens are carried; any failure after the probe stops the restore before the cutover.
  `restore` and `restore --dry-run` print how many tokens, revocations and last-use days are (or
  would be) carried.

## Rejected

- **Reloading `creation.*` on `SIGHUP`.** Still a file edit per token, no labels, no usage
  information, and revocation by hand.
- **Only relaxing the startup check.** Adding a token would still need a restart.
- **Selecting the row by verifier (`WHERE verifier = ?`).** An index comparison, not constant
  time.
- **A cache keyed on `PRAGMA data_version`.** The value is per connection and easy to get wrong in
  a pool; one small read is cheap next to the create's own write.
- **Answering `401` when the tokens cannot be read**, or falling back to the configured hashes.
  The web client would forget a valid token.
- **Recording which token created a capsule, or every use.** It would tie every capsule to a
  person. The day of last use is enough to tell a used token from an abandoned one.
- **Printing the hash or a configuration entry on `token create`.** Both are meaningless for a
  database token and invite configuring it twice.
- **Revoking by label.** Labels need not be unique; the id is.
- **Un-revoking, deleting rows, or a retention rule for revoked tokens.** A restore could then
  bring a revoked token back.
- **An error for database tokens outside `token` mode.** Tokens may be prepared before the mode is
  switched; a warning is enough.
- **Amending ADR-018.** This adds persistence, operator commands and a restore rule; ADR-018 covers
  the browser side, which is unchanged.

## Consequences

- An operator issues and revokes per-person tokens without a restart; a browser holding a revoked
  token gets `401` on its next create, forgets it and asks again (ADR-018, no web client change).
- Every create in `token` mode reads the active tokens, and at most once per token per UTC day
  writes `last_used_on`.
- The database and its backups hold the token hashes and the operator's labels, which may name
  people. The built-in privacy notice (English and German) says what is stored per token, that
  revoked entries are kept and that capsules are not linked to tokens.
- `creation.token_hashes` remains for setups configured by file or environment and for scripts
  (`scripts/smoke.sh`, the browser tests and the container job use plain `token`). Changing it
  still needs a restart.
- A carried token whose id the copy already uses for another verifier fails on the primary key and
  stops the restore before the cutover; with ULIDs this is practically impossible and has no
  special handling.
- Still not included: issuing, listing or revoking tokens on the web or through any HTTP endpoint,
  token expiry, per-token quotas, rate limits or permissions.
