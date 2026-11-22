# AGENTS.md — CapsuleBin

## Overview

CapsuleBin is a minimal, accountless paste service for text, code, Markdown, and small files.
Encryption and decryption happen exclusively in the client; the server stores opaque ciphertext
and lifecycle metadata and **never sees capsule plaintext, keys, or link secrets.**

> **Non-negotiable invariant:** no API endpoint accepts or returns the plaintext of a capsule.
> The server never sees the link secret, the password, or the AES key. The only plaintext it
> receives is an abuse report's description and optional contact (personal data, ADR-013).

Version 1.0.0, not yet released (protocol v1 fixed by ADR-003, see `CHANGELOG.md`). A multi-item
encrypted capsule works end-to-end: create, describe, retrieve, delete, burn-after-retrieval,
password protection, Markdown/code rendering, file items, retention/pruning, rate limiting,
idempotent create, the CLI (`put`/`get`/`delete`), abuse reports and the operator moderation
commands are all implemented. The web interface is in English and German: the default language
without a path prefix, the other under its prefix (`/de/`). Operators can set the instance's name,
tagline, logo, favicon and theme colours (`[branding]`); every page says "Powered by CapsuleBin",
with "CapsuleBin" linked to the source code.

## Repository layout

```
capsulebin/
├── src/
│   ├── server/         Rust crate `capsulebin-server` — Axum server, config, DB, HTTP layer
│   │   ├── src/        Rust source
│   │   ├── web/        @capsulebin/web — browser client (Vite/Lit), embedded into server binary
│   │   └── build.rs    Creates src/server/web/dist if missing; rust-embed embeds it at compile time
│   ├── cli/            @capsulebin/cli — standalone command-line client (esbuild, Node.js)
│   └── core/           @capsulebin/core — platform-neutral capsule format, crypto, API client
│                         (no DOM, no build step; shared by web client and CLI)
├── migrations/         SQLite migrations (0001–0004), embedded into server binary
├── docs/
│   ├── adr/            17 Architecture Decision Records (see below)
│   ├── capsule-file-format.md   Language-neutral binary container specification
│   ├── crypto-profile.md        Binding crypto specification (primitives, derivation, verifiers)
│   ├── threat-model.md          Assets, adversaries, controls + the tests that back them
│   ├── operations.md            Deployment, reverse proxy, backups, upgrades, abuse runbook
│   ├── privacy-notice.md        What an instance processes; template for the operator's notice
│   ├── i18n.md                  Interface languages: paths, default, catalogues, adding a language
│   ├── branding.md              Operator branding: name, tagline, logo, favicon, colours, attribution link
│   └── test-vectors/             Golden vectors for cross-implementation conformance
├── scripts/            smoke.sh — acceptance test for a release binary; release-notes.sh;
│                         reproducible-web.sh — builds the web client twice and compares;
│                         support-windows.mjs — Node.js/Debian end-of-support check
├── .github/            CI and release workflows (GitHub Actions), Dependabot configuration
├── Dockerfile          Container image: web build → release binary → distroless, non-root, /data
├── Cargo.toml          Workspace root (members: src/server)
├── package.json        npm workspace root (workspaces: src/core, src/cli, src/server/web)
└── capsulebin.example.toml   Annotated configuration reference
```

### Why `src/server/web/` is nested inside `src/server/`

The web client has no independent release and no consumer other than the server, which
embeds its `dist/` output into the binary at compile time via `rust-embed`. The CLI is the
only standalone client. (See [ADR-010](docs/adr/ADR-010-repository-layout.md).)

## Technology stack

| Layer | Technology |
|-------|-----------|
| Server | Rust (edition 2024, MSRV 1.99), Axum 0.8, SQLx 0.9 (SQLite), Tokio |
| Web client | TypeScript 7.0, Vite 8, Lit 3, highlight.js, marked, DOMPurify |
| CLI | TypeScript, esbuild, Node.js 24 (version in `.node-version`) |
| Core library | TypeScript (no DOM, no build step), Web Crypto API |
| Database | SQLite via SQLx, WAL mode, `synchronous = full`, `secure_delete = on`, `journal_size_limit` 64 MiB |
| CSS | Design tokens in `src/server/web/src/styles/tokens.css`, CSP without inline styles |
| Formatting | Rust: `rustfmt`; TypeScript/CSS/HTML/JSON: Prettier (4-space indent, single quotes, 100-col) |

## Key design invariants

These are hard constraints — never relax them without an ADR:

1. **Zero server-side capsule plaintext.** No endpoint accepts or returns the plaintext of a
   capsule. The server stores opaque ciphertext. This is verified by `db_no_plaintext.rs`
   (a mandatory CI gate).
2. **The link secret lives in the URL fragment** (`#`), which browsers never transmit in
   HTTP requests.
3. **Capability verifiers, not secrets.** The server stores only SHA-256 hashes of
   describe/reveal/delete/creation tokens. Comparisons use constant-time (`subtle` crate), never
   `==`.
4. **Independent delete secret.** Not derived from the link secret — otherwise every
   recipient could delete the capsule.
5. **Indistinguishable responses.** Unknown, expired, disabled, deleted, and wrong-capability
   all answer identically. A caller cannot learn whether a paste ID exists.
6. **Logs must never contain plaintext, capabilities, tokens, or full paste IDs.**
   Verified by `logging.rs` (mandatory CI gate).
7. **`unsafe_code = "forbid"`** across the entire workspace.
8. **`unwrap_used`, `expect_used`, and `panic` are clippy warnings** — a panic inside a
    handler must not bypass the "no plaintext leaves the server" invariant.

## Crypto architecture

See [`docs/crypto-profile.md`](docs/crypto-profile.md) for the binding specification.
Key points:

- **Content encryption:** AES-256-GCM. Key derived via HKDF-SHA-256 from the link secret.
- **Three HKDF derivations** from the link secret with different `info` labels:
  `capsule/v1/content` (content key) and `capsule/v1/reveal` (reveal token), both salted with
  `hkdf_salt`; `capsule/v1/describe` (describe token for the descriptor), salted with the paste id.
- **Delete secret** is independently random, hashed with a domain prefix:
  `SHA-256("capsule/v1/delete" || secret)`.
- **Password protection:** password NFC-normalized, then PBKDF2-HMAC-SHA-256 (default 600k
  iterations, hard cap 5M).
  The core header parser refuses a non-integer count, fewer than 100k iterations
  (`MIN_PBKDF2_ITERATIONS`) or a salt other than canonical base64url of 16 bytes, before a
  password is asked for: the link's issuer chooses the header and receives the reveal token, so a
  cheap KDF would let it guess the password offline. `openPaste` reports such a header as
  `invalid_capsule`. The floor lives in the parser, not in `derivePasswordKey`.
  A non-empty whitespace-only password makes `Paste.seal()` throw; `''`/`null` mean no password.
  Password key is mixed into HKDF info — wrong password produces wrong reveal token,
  server rejects before decryption is attempted. No password verifier in the public header
  (would be an offline brute-force oracle).
- **Public header** (JSON, unencrypted, stored by server): `envelope_format`, `nonce`,
  `hkdf_salt`, optional `password` object. Canonical JSON serialization (fixed field order,
  no whitespace) because it's used as AES-GCM AAD.

## Capsule file format

Binary container inside the ciphertext. See [`docs/capsule-file-format.md`](docs/capsule-file-format.md).

```
Byte 0..8    Magic "CAPSULE\0"
Byte 8..10   Framing version (u16, BE, value 1)
Byte 10..14  Manifest length (u32, BE)
Byte 14..    Manifest (UTF-8 JSON) + items (concatenated bytes, no delimiters)
```

- Max 16 items, max 10 MiB per encoded capsule (`MAX_CAPSULE_BYTES`: header + manifest + items;
  items together at most `MAX_ITEM_BYTES`), max 64 KiB manifest.
- Padded with `0x00` to the PADMÉ length (capped at 10 MiB); decoders reject non-zero padding and
  refuse input above `MAX_CAPSULE_BYTES` before scanning it.
- Reference implementation: `src/core/src/capsule.ts`.
- Golden vectors: `docs/test-vectors/capsule-v1.json` (checked by `src/core/tests/golden-vectors.test.ts`).

## HTTP API

Versioned in the path: `/api/v1/pastes`.

| Method | Path | Purpose |
|--------|------|---------|
| POST   | `/api/v1/pastes`              | Create (bearer token required in `token` mode) |
| POST   | `/api/v1/pastes/{id}/describe` | Describe token (or delete secret) → public header and lifecycle, never the ciphertext |
| POST   | `/api/v1/pastes/{id}/retrieve` | Prove describe + reveal capability, receive ciphertext |
| DELETE | `/api/v1/pastes/{id}`         | Prove delete capability |
| POST   | `/api/v1/reports`             | Abuse report by paste id (same `202` whether or not it exists) |
| GET    | `/api/v1/reports`             | Report policy for the report page |
| GET    | `/health/live`                | Process is alive |
| GET    | `/health/ready`               | Config, DB, and schema are usable |
| GET    | `/branding/<hash>.<ext>`      | A configured logo or favicon, or the stylesheet generated from `[branding.colors]`, immutable caching; any other `/branding/` path is the generic `404` |

Errors are `application/problem+json` with a stable `code` field and `X-Request-Id`.
Clients branch on `code`, never on `title`.

Request bodies: create takes the whole request and decides the creation capability from the head
(mode, `Authorization`) before reading the body; `POST /api/v1/reports` checks `abuse.enabled`
before reading its body — an extractor argument (`Json<T>`) would be read before the handler runs.
Fixed per-route limits sit inside the global `max_body_bytes` layer: 4 KiB for describe, retrieve
and delete (`PROOF_BODY_LIMIT` in `api/pastes.rs`), 32 KiB for reports (`MAX_REPORT_BODY_BYTES` in
`api/reports.rs`); create keeps `max_body_bytes`. Every `413` is `payload_too_large`
(`body_limit_problem` outside all limit layers), declared or streamed. No server-side body timeout:
that is the reverse proxy's (`docs/operations.md`).

Every page exists once per known language (ADR-016, [`docs/i18n.md`](docs/i18n.md)): `/`,
`/p/{id}`, `/manage/{id}`, `/report`, `/imprint`, `/privacy`, `/terms` in the default language
(`i18n.default_language`), and the same paths under `/{lang}/` for every other language. Paths
under the default language's prefix answer `302` (`no-store`) to the path without it; a prefix that
is not a known language is the generic `404`. The server does not read `Accept-Language` and sends
no `Vary` for it: the browser language is applied by `language-init.js`. Share and management links
are created without a prefix. API, `/health/*`, `/branding/*` and static assets (incl. the built-in
`/favicon.svg`) have no prefix, and no `*.html` is served directly.

## Configuration

See `capsulebin.example.toml` for all settings with defaults.

Critical rules:
- **Unknown keys are a startup error** — a typo must not silently disable a security setting.
- **Every setting has an env equivalent:** `CAPSULEBIN__<TABLE>__<KEY>`.
- **`server.public_url` is required, no default** — it appears in `problem+json` type URIs
  and share links, so it must come from configuration, not a request header. It must not carry a
  query or fragment, also an empty one (`…/team?`, `…#`): paths are appended to it as text
  (`has_query_or_fragment` in `config/model.rs`, `hasQueryOrFragment` in core `links.ts`; CLI `put`
  refuses such a `--server` too). Its path must not end in a known language code (`…/team/de`,
  also after dot segments are resolved: `…/team/de/.`): links below it would be ambiguous with the
  language prefix. CLI `put` refuses such a `--server` by the same rule (`trailing_language` in
  `config/model.rs`, `trailingLanguage` in core `links.ts`). Both rules are kept equal on both
  sides by the shared vectors `src/core/tests/fixtures/base-language.json`.
- **`creation.mode` defaults to `token`** — an instance reachable before configuration
  must not be an open write target.
- **`[legal]` texts are operator input placed into pages** (ADR-015) — Markdown rendered at
  startup without raw HTML or images and with links limited to http(s), mailto, tel and local
  paths; `url` values limited to absolute http(s) URLs and single-slash paths. The built-in
  privacy notice and terms (`legal/defaults/{en,de}.rs`) describe what the server stores and
  logs: change them together with any such behaviour, in every language. `[legal.*]` is the
  default language's text; `[legal.<lang>.*]` (same keys) gives another language's section, which
  replaces its `[legal.*]` counterpart as a whole; without one that language falls back to
  `[legal.*]`, marked with `lang` (`docs/i18n.md`). `[legal]` has no `deny_unknown_fields`
  (it uses `flatten`); `LegalConfig::validate` refuses unknown or default-language tables instead.
  Legal page titles come from the catalogue's footer labels, never from Rust.
- **`[i18n] default_language` is the only language setting** (default `en`) — it chooses the
  language whose pages have no path prefix; an unknown code or any other key in `[i18n]` is a
  startup error. Every language in `KNOWN_LANGUAGES` is always offered. The known languages exist
  three times (server `config/model.rs`, core `languages.ts`, catalogue files in
  `src/server/web/src/i18n/`), kept equal by `tests/languages.rs` and `i18n-catalogues.test.ts`.
- **`[branding]` values are operator input placed into pages** (ADR-017, `docs/branding.md`) —
  `name` (default `CapsuleBin`, ≤ 64 characters, no control characters), `hide_name`, `tagline`,
  `logo`, `logo_dark`, `favicon`, `source_url`, `[branding.colors.light]`/`[branding.colors.dark]`,
  and `[branding.<lang>]` with only `tagline` for a
  known language other than the default one (fallback: `[branding.<lang>] tagline` →
  `branding.tagline` → built-in `create.claim`; empty removes the create page's slogan; ≤ 80
  characters, no control characters; a table for an unknown or the default language is a startup
  error — `BrandingConfig` uses `flatten`, so `validate` refuses those tables). Only raster files
  (logos PNG/WebP, favicon PNG/ICO, checked by extension and signature, ≤ 256 KiB), never SVG;
  read once at startup and served at `/branding/<16 hex of SHA-256>.<ext>`.
  `hide_name`/`logo_dark` without `logo` and a `source_url` that is not an absolute http(s) URL
  without user info are startup errors. The name appears only in the web pages (not API, CLI,
  logs). The footer attribution "Powered by CapsuleBin" (AGPL §13) cannot be switched off and
  keeps the project's name; only "CapsuleBin" links to `source_url`, in a new tab
  (`target="_blank" rel="noopener noreferrer"`).
  `[branding.colors.<theme>]` takes the 18 colour tokens of `tokens.css` as snake_case keys (`x_y` →
  `--cb-x-y`; not `--cb-shadow-card`), only `#rgb`/`#rrggbb`, normalised to lowercase; unset tokens
  keep the built-in value per theme; an unknown key or table, or another value, is a startup error
  naming the full key (`Colors` has its own `Deserialize`). With any colour the server generates a
  stylesheet, serves it at `/branding/<16 hex>.css` and links it right before `</head>` of every
  page (light values in `@media not all and (prefers-color-scheme: dark)`, then the `data-theme`
  blocks, so system preference and toggle keep working). A pair below the built-in theme's contrast
  thresholds (`web/tests/fixtures/contrast-pairs.json`) is a warning: logged by `serve`, printed to
  stderr by `check`; never an error. The Rust palette and pairs in `branding/colors.rs` are kept
  equal to `tokens.css` and the fixture by tests.

### Retention (purge)

Three independent rules, each deletes the whole row (metadata + payload):
- **expired:** measured from `expires_at`
- **never retrieved:** creation + `never_retrieved_after` (default 30d)
- **inactive:** last authorized retrieval + `inactive_after` (default 90d)

Runs at startup and every 24h. `prune` subcommand applies once on demand.
`purge.enabled = false` disables the scheduled job.

**Ids are never freed** (`migrations/0004_retired_ids.sql`, ADR-011). Delete, burn and moderation
keep a tombstone row; every `DELETE FROM pastes` puts the id into `retired_ids` (only column `id`,
`WITHOUT ROWID`) via the `AFTER DELETE` trigger — never move this into the purge code, and never
delete from `retired_ids` (no retention, no setting, no command). `SqlitePasteRepository::create`
checks `retired_ids` inside the `INSERT … SELECT … WHERE NOT EXISTS` statement (a separate `SELECT`
would race a purge); no row inserted → `IdConflict` → the same `409 id_conflict` as an occupied id.
`restore` and `--dry-run` carry the replaced database's ids (`pastes` and, if present,
`retired_ids`) missing from the backup into the staged copy's `retired_ids` with `ATTACH` (path as a
bound read-only `file:` URI, never SQL text) and `INSERT … SELECT`, never through process memory.
First a read probe (whether `old` has `retired_ids`, and a `count(*)` over the same ids) writes
nothing into the copy; then the ids go into the scratch table `restore_carried_ids` of the staged
copy (not `TEMP`: in memory, or a temporary file outside the stage), created and dropped in the
same transaction. Only a failure of `ATTACH` or of the probe is a warning
(`RestoreOutcome::warnings`) and leaves the copy unchanged; every later failure, also a read of
`old` that fails only in the insert, is an error that stops the restore before the cutover.

### Abuse report ceilings

`abuse.max_open_reports_per_paste` (default 10) and `abuse.max_open_reports` (default 1000), integers
≥ 1 (`0` is refused; `abuse.enabled = false` stops reports). Open = `open` or `reviewing`. A report
is stored only below both ceilings; existence check, both counts and insert are one statement
(`submit_report` in `repository/sqlite/abuse.rs`, returns `SubmitOutcome`). Every outcome answers
the same `202`. Both callers take the ceilings from `AbuseConfig::report_ceilings()`. A dropped report
logs one warning per minute at most, without id, text or contact; the throttle is per `AppState`
(`CeilingWarning` in `api/reports.rs`), not a process-wide static, so instances do not share it;
`reports list` (text) starts with `open reports: N of M (at most K per capsule)`, `--json` unchanged.

### Database files, backup and restore

- **Private from the first byte.** `db::connect` creates a missing database file itself
  (`db::create_private_file`: `create_new`, `0600` on Unix) and then opens it with
  `create_if_missing(false)` (`:memory:` excepted); SQLite gives `-wal`/`-shm` the main file's mode.
  Never let SQLite or `VACUUM INTO` create a file in a path others can reach, and never fix modes
  with a later `chmod` (it cannot revoke a descriptor opened in between) or by changing the umask.
- **Existing files are only reported.** `db::permission_warnings` (database, existing `-wal`/`-shm`
  with any group/other bit; directory with a group/other write bit): `serve` logs, `check` prints
  `warning: …` and stays `ok:`/exit 0. Nothing is `chmod`ed.
- **No fixed scratch names.** The write probe in `DatabaseConfig::validate` is
  `.capsulebin-write-probe-<16 hex>`, created exclusively and removed; backup and restore stages are
  `0700` directories `.<output>.backup-<16 hex>` (next to the output) and
  `.<file>.restore-<16 hex>` (next to the database), all named by `db::random_name` (`getrandom`),
  stages removed by a drop guard. The shared file helpers live in `db.rs`: `db::parent_or_current`
  (behind `DatabaseConfig::directory()`, the one source for the database directory, and the backup
  stage's directory) and `db::sidecars` (`-wal`/`-shm` names) — do not re-implement them.
- **Backup order** (`backup::create`): `occupied(output)` check → stage → `VACUUM INTO` the stage
  → `verify_file` → `occupied(output)` re-check → `rename` the snapshot to the output. `rename`
  follows no symlink but replaces an entry created after the re-check; that window is accepted
  like restore's (planting there needs write access to the output directory, which could replace
  the backup anyway) — a hard link would make backups fail on filesystems without hard links.
- **Restore order** (`backup::restore`): collision check (target, or with `--force`
  `<file>.pre-restore-<now>` and its sidecars, via `symlink_metadata`) → stage → copy only the main
  file → `verify_file` on the copy (never on the input: RUSTSEC-2026-0459) → `db::connect` +
  `db::verify` + `wal_checkpoint(TRUNCATE)` → collision re-check → move database, `-wal`, `-shm`
  aside → rename the copy into place. The moves use `rename`, which replaces a destination created
  after the re-check; that window is accepted (stopped server; planting there needs write access to
  the database directory) — publishing by hard link would make restore need hard links on the
  database volume. Every move is recorded and undone in reverse on a failure;
  if undoing fails, `BackupError::RollbackFailed` names what to move by hand. `restore_dry_run`
  stops after the checkpoint. Failure points are a `#[cfg(test)]` step hook, no production flag.
- **Restore leftovers.** The names come from `db::pre_restore_prefix` and
  `db::restore_stage_prefix` only. A missing database next to any `<file>.pre-restore-*` entry or a
  `.<file>.restore-*` directory is `DbError::RestoreLeftovers` for every command that opens it, not
  a new empty database; next to an existing database they are ignored.

### Rate limiting

Per-endpoint-class, per-client-IP token buckets (`requests_per_minute` or `requests_per_hour`,
`burst` ≥ 1; `0` disables), off by default. Plus `create.bytes_per_day` (Content-Length quota),
`create.max_concurrent`, `invalid_capability` (404s from describe/retrieve/delete per client) and
`invalid_capability_per_paste` (wrong passwords per paste id from any client — **on by default**,
10/min burst 10; protects passwords of leaked links; only retrieves with a correct describe token
count, checked in the retrieve handler so callers with only the id can neither drain it nor see its
`429`). The others apply before any lookup; `429` is
`problem+json` with a computed `Retry-After`.
Uses `X-Forwarded-For` when the peer is in `server.trusted_proxies`. Requires the client IP layer to
run before the limiter and the server to serve with connect info (see `router()`/`serve.rs`).
In-memory only, resets on restart. Two tables, client buckets and paste buckets (password
attempts), each capped at 100 000 entries (`CLIENT_CEILING`/`PASTE_CEILING`, fixed, no setting);
work per admission is bounded (a new key lets a sweep cursor drop up to `SWEEP_STEP` = 8 refilled
buckets, never a full scan under the lock). At the ceiling the client table evicts its least
recently used bucket; the paste table never evicts a budget. With room (a free slot or one the
bounded sweep reclaims, decided by `Table::has_room` under the same lock) a wrong password on an
untracked paste gets a bucket and a correct one passes without creating one. Without room every
attempt on an untracked paste, correct or wrong, takes a token from one overflow budget
(`Table::overflow`, shared by all untracked pastes, spec of `invalid_capability_per_paste`, off with
it, no setting); empty → `429` for both, `Retry-After` = one overflow token's refill time (6 s at the
default). Answering correct and wrong alike is the point: a pass for one and a `429` for the other
would be a password oracle; the accepted price is that first retrieves share that budget while the
table is held full. Tracked pastes use only their own bucket. A saturated table warns at most once a
minute (table and ceiling, no address or id), also when a correct attempt meets it. The clock is
injectable for tests (`bounded_for_tests`); `src/test_support.rs` (`#[cfg(test)]`) builds an
`AppState` over a temporary database for handler-level tests inside the crate.

## Build and test

### First-time setup
```sh
npm install          # resolves src/{core,cli,server/web} workspaces
# python3 must be on PATH for the CLI's pseudo-terminal tests
npm run build -w @capsulebin/web   # build frontend (else no pages are served)
```

### Development server
```sh
# Terminal 1: server on :8081
cargo run -p capsulebin-server -- --config capsulebin.dev.toml serve

# Terminal 2 (optional): Vite dev server on :5173, proxies /api to :8081
npm run dev -w @capsulebin/web
```

### CLI
```sh
npm run build -w @capsulebin/cli
node src/cli/dist/cli.js put notes.md --server http://127.0.0.1:8081
```

### Subcommands
| Command | Purpose |
|---------|---------|
| `serve`  | Start the server |
| `check`  | Validate configuration, database, migrations (no port opened) |
| `healthcheck` | Probe the running server's `/health/ready` (`--live`, `--address`); exit 0 if healthy (container `HEALTHCHECK`). Dispatched before `Config::validate`: reads only `server.bind` (nothing with `--address`), writes nothing |
| `migrate`| Apply pending migrations and exit |
| `prune`  | Apply retention policy once and exit |
| `token`  | Generate a creation token + the hash to configure (domain-separated) |
| `reports`| Abuse report queue: `list`, `show`, `review`, `close` |
| `moderation` | `disable` (reversible), `delete` (final), `restore`, `status` by paste id |
| `backup` | `VACUUM INTO` snapshot of the live DB into a private `0700` stage next to the output (compacted, 0600, verified), published by `rename` after a re-check that the output name is free (never follows an entry) |
| `restore`| Copy the backup's main file into a private stage next to the DB, verify + migrate + checkpoint it, retire the replaced DB's ids the backup lacks in the copy (read-only `ATTACH`; unreadable → warning), then cut over with rollback (server stopped; `--force` moves the old DB + `-wal`/`-shm` aside, `--dry-run` migrates the staged copy and counts the ids only) |

### Test commands (CI gates)
```sh
# Rust
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

# TypeScript
npm run format:check
npm run check -w @capsulebin/core && npm test -w @capsulebin/core
npm run check -w @capsulebin/web && npm test -w @capsulebin/web
npm run check -w @capsulebin/cli && npm test -w @capsulebin/cli   # needs python3 (PTY tests)
./scripts/reproducible-web.sh   # two web builds, byte-identical

# Acceptance (release binary + smoke test, incl. CLI put/get/delete)
npm run build -w @capsulebin/web && npm run build -w @capsulebin/cli
cargo build --release --locked -p capsulebin-server
CAPSULEBIN_CLI=src/cli/dist/cli.js ./scripts/smoke.sh ./target/release/capsulebin

# Browser tests against that binary (Chromium, Firefox, WebKit)
npm exec -w @capsulebin/web -- playwright install chromium firefox webkit   # once
npm run e2e -w @capsulebin/web
```

### Key test suites and what they encode

| Test | What it verifies |
|------|-----------------|
| `src/server/tests/db_no_plaintext.rs` | Plaintext never appears in any DB column (text, hex, or base64). **Mandatory CI gate.** |
| `src/server/tests/secure_delete.rs` | Burned and deleted ciphertext is gone from the raw database file and WAL after a `wal_checkpoint(TRUNCATE)` (`secure_delete`); an ordinary checkpoint truncates nothing, so this is not a claim about WAL frames in normal operation. |
| `src/server/tests/retired_ids.rs` | An id freed by each retention rule cannot be created again (`409` byte-identical to a live collision, also for a replay of the original request; old link stays unknown); a report on a purged paste shows `gone`, moderation answers `NotFound`, a new report is not stored. |
| `src/server/tests/retired_ids_acceptance.rs` | Against the real binary: after its `prune` removed a reported capsule, `reports list` (text and `--json`) and `reports show` print the paste as `gone`, `moderation disable`/`delete` answer "no paste" and record nothing, re-creating the id is `409 id_conflict`, a new report is answered `202` and not stored. |
| `src/server/tests/logging.rs` | Log output never contains payload, capability secrets, tokens, full paste IDs, or report text; the access log's `path` is a route template (also on `429`/`413`/`401`), a served asset or `{unmatched}`, also for seeded random paths. **Mandatory CI gate.** |
| `src/server/tests/rate_limits.rs` | Limits apply before lookup (no burn), invalid-capability throttling, byte quota, X-Forwarded-For spoofing. |
| Handler tests inside the crate (`src/server/src/test_support.rs`: helpers only, no tests) | Requests through `router()` over an `AppState` the test may change (e.g. a small limiter ceiling), next to the code they cover. `api/rate_limit.rs`: `a_saturated_retrieve_answers_correct_and_wrong_passwords_alike` (a full paste table's overflow budget answers correct and wrong passwords with the same status, `Retry-After` and body; a burn paste is not consumed by a `429`), `correct_retrieves_with_room_spend_neither_slots_nor_overflow`. `api/pastes/handlers.rs`: `a_conflict_is_decided_before_the_payload_is_read`, `replays_of_unretrievable_pastes_read_no_payload`, `a_foreign_request_on_a_taken_id_reads_no_payload` (conflicting creates read no ciphertext, a possible replay at most once; counted by the `#[cfg(test)]` `payload_reads` counter of `SqlitePasteRepository`). |
| `src/core/tests/properties.test.ts`, `src/server/tests/properties.rs` | Property-based tests (fast-check / proptest) for every parser of untrusted input. |
| `src/server/tests/concurrency.rs` | Burn race, parallel retrievals, delete vs. retrieve through the HTTP stack. |
| `src/server/tests/database_files_acceptance.rs` | Against the real binary (Unix): a new database and its `-wal`/`-shm` are `0600` under umask `022`; permissive files and a writable directory are warned about by `check`/`serve` and left unchanged; the write probe follows no planted symlink; a missing database next to restore leftovers is refused, not recreated. |
| `src/server/src/backup/tests.rs` | Backup staged privately and never replacing or following an entry; restore verifies and migrates a private copy, keeps the old database byte-identical after a failure at every step (copy, verify, migrate, checkpoint, each move, install), refuses a taken aside name, ignores a sibling `-wal`, follows no planted scratch entry; retires the replaced database's ids missing from the backup (rows and retired ids, also from a schema-3 database, 100 000 ids in SQL, installed schema equal to a fresh one so no scratch table left), warns and still installs over an unreadable one (garbage, `-wal` without `-shm`, readable schema over unreadable tables), but stops (restore and dry run, old database and directory unchanged) when writing the ids into the copy fails (`SQLITE_FULL` from a test-only `max_page_count`); dry-run counts them and changes nothing. |
| `src/server/tests/healthcheck_acceptance.rs` | `healthcheck` succeeds with only `CAPSULEBIN__SERVER__BIND` (no `public_url`, missing database directory), with `--config` or with `--address` alone, and writes nothing into the database directory. |
| `src/server/tests/restore_acceptance.rs` | Against the real binary: `restore --dry-run` with a backup at an older schema prints its contents and the migrated schema version, exits 0 and changes nothing (input, target, sidecars, directory); an unusable backup fails it, again without a change; `restore --force` keeps the old database and both sidecars byte-identical under `<file>.pre-restore-<now>` (nothing touched without `--force`) and installs the backup's rows at the current schema; with a current database holding ids not in the backup (a live row and a retired id), `--dry-run` and `--force` print that 2 ids would be/were retired (no `warning:`), the dry run changes nothing, and afterwards both ids are in `retired_ids`. |
| `src/server/tests/upgrade.rs` | A database at schema 3 (migrations 0001–0003, `tests/fixtures/capsulebin-1.0.sql`, never regenerated) migrates and stays usable: capabilities, burn tombstones, password headers, reports, moderation; migration 0004 starts with no retired ids, tombstones still refuse re-creation, purged fixture ids are retired. Fails when an existing migration is edited (sqlx checksum). |
| `src/server/tests/abuse.rs` | Reports answer identically for existing/unknown ids and when dropped at a ceiling; moderated pastes look like unknown ones; a disabled endpoint never reads the body (`Expect: 100-continue` over a real socket, `post_expecting_continue`); the 32 KiB report body limit. |
| `src/server/tests/http.rs` | Security headers on every response; `/p/{id}` and `/<lang>/p/{id}` answer byte-identically for every id, whichever language is the default. |
| `src/server/tests/language_paths_acceptance.rs`, `legal_languages.rs` | Edge cases of the language paths; legal pages per language (`[legal.<lang>.*]`, fallback per section, `lang` marking, titles from the shell). |
| `src/server/tests/report_ceilings_acceptance.rs` | Against the real binary: `reports list` starts with the open reports and both ceilings (also with none open, and says when the global one is reached), `--json` stays a plain array; both ceilings read from the file and the environment; `0` or a negative value stops the binary naming the key. |
| `src/server/tests/api.rs` (body tests) | An unauthorized create never reads the body (`Expect: 100-continue` over a real socket, `post_expecting_continue` in `tests/support/mod.rs`); 4 KiB limit on describe/retrieve/delete and `max_body_bytes` on create, declared and streamed. |
| `src/core/tests/limits.test.ts`, `limits.acceptance.test.ts` | Core API client against a fake `fetch`: oversized bodies (every endpoint, error statuses, without/wrong/correct `Content-Length`) cancelled after at most the limit plus one chunk; a capsule at `MAX_CAPSULE_BYTES` still opens; total and idle deadlines with fake timers abort the signal. `limits.acceptance.test.ts`: the edges (an error body of exactly 64 KiB read, one byte more cut off; a create still waiting after 30 s; a describe or error body stalled for 30 s times out). Shared fakes (`endlessBody`, `stallingBody`, `observe`, `sealedPaste`) in `limits-support.ts`. |
| `src/core/tests/redirect.test.ts` | The API client follows no redirect, against two real HTTP servers. |
| `src/core/tests/golden-vectors.test.ts` | `@capsulebin/core` against `docs/test-vectors/capsule-v1.json`. **Mandatory CI gate.** |
| `src/server/web/tests/contrast.test.ts` | WCAG contrast ratio of every design token pair in `tests/fixtures/contrast-pairs.json` (shared with the server's colour warnings) on all four palette blocks. |
| `src/server/web/tests/xss.test.ts`, `trusted-html-usage.test.ts` | XSS vectors through the marked + DOMPurify pipeline; no bare `innerHTML` assignment in the sources (jsdom does not enforce Trusted Types). |
| `src/server/tests/languages.rs`, `src/server/web/tests/i18n-catalogues.test.ts` | Server list, core list and catalogue files name the same languages; catalogues have the same keys, every plural form, no empty value. |
| `src/server/web/tests/shells.test.ts` | Every visible text in a shell template comes from the catalogue (no literal `CapsuleBin`); generated shells keep the server markers, start every link but the attribution with the prefix token, end the footer with the attribution (only the project name linked to the source token) and carry the default token only in the languages meta; the tagline markers wrap the create page's slogan only. |
| `src/server/tests/branding.rs` | `/branding/` files served byte-identical with MIME type, immutable caching and security headers, every other `/branding/` path the generic `404`; name, tagline (per language, empty, escaped), logo, `hide_name`, dark logo, favicon and attribution on every page in `en`/`de` (own test shells), operator values escaped and never read as tokens or markers. |
| `src/server/tests/branding_colors.rs` | The colour stylesheet served from memory (both asset modes), without colours every `/branding/…css` the generic `404`; with colours exactly one link last in the head of every page in `en`/`de`, the page otherwise unchanged. |
| `src/server/tests/branding_acceptance.rs` | Against the real binary: every unknown key or refused `[branding]` value and unusable or unreadable file stops `check` and `serve` (key named, no file content); every key set from the environment reaches the started binary; contrast warnings on `check`'s stderr (still `ok:`, exit 0) and in `serve`'s log. |
| `src/server/web/tests/catalogue-modules.test.ts` | Only `register-<lang>.ts` imports a catalogue file; every built page loads its own catalogue module and no other; the switch notice in `language-next-page.ts` equals the catalogues and is the only text held in every language. |
| `scripts/smoke.sh` | Full lifecycle over a real socket against a release binary, asserts security headers on every response; with `CAPSULEBIN_CLI` also CLI put/get/delete (retrieved content compared from the written files), incl. the private link channels (no `#v1.` on stderr, link file mode `600`). |
| `src/cli/tests/cli.test.ts` | CLI exit codes, secret handling (never argv), burn confirmation, retry-with-replay, HTTP policy, refused redirects, response limits and timeouts (exit `7`, retry counts), `put`'s input limit (before reading, endless stdin, `/dev/zero`), `CAPSULEBIN_PASSWORD` bound to `--server`, `CAPSULEBIN_TOKEN` withheld from a `--server` other than `CAPSULEBIN_SERVER`, empty path options refused before any request (no fallback), the 16 384-byte first line of link/token/password sources (stdin left open, `\r\n`, `/dev/zero`), link channels (stdin, `--link-file`, `--delete-link-file`), the delete link kept after a failed `put` (file kept, else printed; `delete_url`/`delete_link_file` in the JSON error), file output (`0600`/`0700` under umask `000`, existing entries and symlinks never opened or followed but a free name taken, no `--force`/`--raw`, no leftover on a failed write, the `capsule-<date>-<id>` directory also for a single item, `--here`/`-o`, nothing on stdout), terminal output (escaped diagnostics, DEL/C1 in JSON). Fake origin `https://capsule.test`. |
| `src/cli/tests/escape.test.ts` | The escaping functions of `terminal.ts` over every code point and byte in U+0000–U+00FF plus seeded random input: no control character emitted, text without one unchanged. |
| `src/cli/tests/terminal.test.ts` | The bundled CLI on a real pseudo-terminal (python3 `pty`): hidden password prompt, burn confirmation, Ctrl-C/EOF → exit `8` with no guarded request, `stty` restored after every run. Fails (no skip) without `python3`, with a message naming it (`pty-requirement.test.ts`); skipped on Windows. |
| `src/server/web/e2e/*.e2e.ts` | Playwright against the release binary in Chromium, Firefox and WebKit: every user flow, the legal pages, hostile Markdown and a capsule at the size limit, a German flow without English text, the legal pages in German (headings, built-in texts, `lang` on fallback text), the language switch and the head script's language choice (`i18n.e2e.ts`), the built-in identity and a branded instance incl. taglines and theme colours in every system-preference/toggle state, the dark logo following the theme toggle and the attribution opening a new tab (`branding.e2e.ts`, extra instances via `e2e/instance.ts`), failing on any CSP/Trusted Types violation, page or console error, or executed XSS canary (`e2e/guard.ts`, which `guard.e2e.ts` checks itself). |

## Architecture Decision Records (docs/adr)

| # | Title | Key takeaway |
|---|-------|-------------|
| 001 | Trust model: zero knowledge | Server never sees capsule plaintext or keys; link secret in the URL fragment; every endpoint taking an id needs a link-derived capability (describe/reveal token, delete secret); wrong passwords limited per paste, only for link holders |
| 002 | Embedded SQLite | SQLite is the only mandatory persistence; WAL mode, `synchronous=full`, `secure_delete`; retrievals stay reads |
| 003 | Protocol versioning | Envelope, framing, and manifest schema are independently versioned; golden vectors are the conformance contract |
| 004 | MPA, Lit for stateful views | Multi-page application (separate HTML entry points per view), no client-side router; Lit inside a page (light DOM, `unsafeHTML` only for sanitized/highlighted output) |
| 005 | Theme and CSP | Design tokens on `:root`, no inline styles, strict CSP |
| 006 | Highlighter | highlight.js instead of Shiki (class-based output fits the CSP, explicit language allowlist) |
| 007 | Error handling | `thiserror` for typed library errors, `anyhow` at the binary boundary only |
| 008 | Capsule file format | Bespoke binary container (magic + version + manifest + items + PADMÉ zero padding), not CBOR/protobuf |
| 009 | TS CLI and core | Single TypeScript implementation shared by web client and CLI; no WASM |
| 010 | Repository layout | `src/{server,cli,core}` with `web/` nested inside `server/` |
| 011 | Idempotent create | The paste id is the idempotency key: an identical re-submission returns the original `201`, anything else stays `409`; a conflict is decided from metadata, the ciphertext read only for a possible replay; ids are never freed (`retired_ids`), so creation permission can test whether one id is taken or was ever used |
| 012 | No OpenAPI document | Contract = capsule format, crypto profile, golden vectors; if ever needed, generate from `wire.rs` |
| 013 | Abuse reports and moderation | Reports only stored for existing pastes, same answer either way; operator CLI; moderated = indistinguishable from unknown |
| 014 | AGPL licence, DCO | AGPL-3.0-only open source; contributions with a DCO sign-off (`git commit -s`), no CLA |
| 015 | Legal pages rendered by the server | Imprint, privacy notice, terms from `[legal]`, built-in texts (English, German) filled in from the configuration; per language `[legal.<lang>.*]` with fallback to `[legal.*]`; restricted Markdown rendered at startup; code still only from the binary |
| 016 | Language paths | Default language without a prefix, every other under `/{lang}/`; no `Accept-Language` negotiation; shells generated per language at build time, one catalogue module per language; browser choice `localStorage` > browser language > default; no cookie |
| 017 | Operator branding | `[branding]` keys instead of operator CSS; name, raster logo/dark logo/favicon (no SVG) read at startup, served at content-hashed `/branding/` URLs; one name for all languages, a tagline per language; theme colours as token values in a generated stylesheet, contrast failures only a warning; "Powered by CapsuleBin" on every page ("CapsuleBin" linked to the source, new tab), not switchable |

## Source code map

### Rust server (`src/server/`)

| File | Purpose |
|------|---------|
| `src/main.rs` | Binary entry point, CLI argument parsing (clap) |
| `src/lib.rs` | `AppState`, `router()` — layer order is security-critical (see comments) |
| `src/config.rs` | Config loading: TOML + env override, validation, `Resolved` |
| `src/config/model.rs` | Config struct definitions; `KNOWN_LANGUAGES`, `DEFAULT_LANGUAGE`, `I18nConfig`, `language_prefix()`; `BrandingConfig` (named field `colors`), `BrandingTexts` (`[branding.<lang>]`) and their validation |
| `src/branding.rs` | `[branding]` resolved at startup: `Branding`, `Tagline` (`BuiltIn`/`Hidden`/`Text`, `Branding::tagline(language)`), `BrandingFile`; reads and checks logo/dark logo/favicon (format, signature, 256 KiB), content-hashed URLs; the colour stylesheet as a `BrandingFile` (`branding/colors.rs`: `Colors` parsing, built-in palette `TOKENS`, `CONTRAST_PAIRS`, `stylesheet()`, `contrast_warnings()`) |
| `src/config/env.rs` | Environment variable mapping (`CAPSULEBIN__...`) |
| `src/config/units.rs` | Duration and byte-size parsing |
| `src/db.rs` | Database connection pool, migration runner; private creation of a missing database (`create_private_file`), restore-leftover guard and its name prefixes, `permission_warnings`; shared file helpers `random_name` (scratch names), `parent_or_current`, `sidecars` |
| `src/healthcheck.rs` | Client side of `capsulebin healthcheck`: one HTTP/1.1 request, status line only |
| `src/backup.rs` | Backup (`VACUUM INTO` into a private stage, published by `rename`), backup verification, staged restore with id carry-over (`carry_ids`) and rollback, `restore_dry_run` (tests in `backup/tests.rs`) |
| `src/domain.rs` | Domain module root; re-exports `PasteId` (`domain/id.rs`: 128 bits, 22-char base64url), `Verifier`, `Clock`/`SystemClock` (`domain/clock.rs`, also `FixedClock` for tests), `Availability`/`NewPaste`/`Paste`, `PurgePolicy`/`PurgeOutcome` |
| `src/domain/paste.rs` | `Paste` struct — the full row representation |
| `src/domain/capability.rs` | Capability verification (constant-time via `subtle`) |
| `src/domain/purge.rs` | Retention rule evaluation |
| `src/domain/abuse.rs` | Report validation, reason/resolution/action codes (ADR-013) |
| `src/repository.rs` | `PasteRepository` and `AbuseRepository` traits, `ReportCeilings`, `ReportEntry`, `SubmitOutcome`, `AbusePurgeOutcome` |
| `src/repository/sqlite.rs` | SQLite implementation (SQLx queries) |
| `src/repository/sqlite/abuse.rs` | `AbuseRepository`: reports (capped by the open report ceilings), moderation, audit trail, retention |
| `src/api.rs` | API module root |
| `src/api/pastes.rs` | Paste router; per-route 4 KiB `PROOF_BODY_LIMIT` for describe, retrieve and delete |
| `src/api/pastes/handlers.rs` | Route handlers: create, describe, retrieve, delete |
| `src/api/pastes/codec.rs` | Wire format encode/decode |
| `src/api/pastes/wire.rs` | Request/response types |
| `src/api/health.rs` | Health endpoints |
| `src/api/assets.rs` | Page routes per language (default without a prefix), redirect away from the default language's prefix, asset serving (embedded: pages prepared once at startup; from disk: read per request); first links the colour stylesheet before `</head>` (if configured), then fills the shell tokens `%%capsulebin:prefix%%`/`%%capsulebin:default%%`/`%%capsulebin:name%%`/`%%capsulebin:favicon%%`/`%%capsulebin:source%%` and the server-internal `%%capsulebin:tagline%%` in one pass after replacing the header regions `<!--capsulebin:logo-->`/`<!--capsulebin:wordmark-->`/`<!--capsulebin:tagline-->` from `[branding]` (tagline per page language), turns the footer markers `<!--capsulebin:legal-link:<page>:<label>-->` into links (label from the shell, target from `[legal]`) or drops them, puts legal pages into `legal.html` titled with the label of their own footer marker |
| `src/legal.rs` | `[legal]` resolution per interface language: imprint, privacy notice, terms; each section from `[legal.<lang>.*]` or else `[legal.*]`, `lang` on blocks not in the page language; footer link targets with the language's prefix (`legal/markdown.rs`: restricted Markdown, `legal/defaults.rs` + `defaults/{en,de}.rs`: built-in texts per language) |
| `src/api/branding.rs` | One route per configured branding file and the colour stylesheet, served from memory with its MIME type and immutable caching |
| `src/api/problem.rs` | `application/problem+json` error responses |
| `src/api/request_id.rs` | Per-request UUID, echoed in response body and `X-Request-Id` |
| `src/api/access_log.rs` | Access log: method and a `path` never taken from the request — the matched route template (`MatchedPath`, recorded by `record_path` right inside the trace layer and outside every rejecting layer), the path of a file `static_file` served (`ServedAsset` response extension), or `{unmatched}` |
| `src/api/rate_limit.rs` | Token-bucket rate limiting (per minute or per hour); bucket tables `Table<K>` with ceiling, LRU list and sweep cursor |
| `src/api/reports.rs` | Report endpoint and policy; 32 KiB body limit, `enabled` checked before the body, ceiling warning throttled by `CeilingWarning` (one per `AppState`) |
| `src/api/client_ip.rs` | Client IP extraction with trusted proxy support |
| `src/api/security_headers.rs` | Security headers on every response |
| `src/commands/` | Subcommand implementations (serve, check, healthcheck, migrate, prune, token, backup/restore, reports/moderation in `abuse.rs`) |
| `build.rs` | Creates `web/dist` if missing and makes Cargo recompile when it changes; `rust-embed` (with `debug-embed`: every profile) embeds it at compile time |

### TypeScript core (`src/core/`)

| File | Purpose |
|------|---------|
| `src/crypto.ts` | Key derivation, encryption/decryption (Web Crypto) |
| `src/capsule.ts` | Capsule file format encode/decode; `decode` refuses input above `MAX_CAPSULE_BYTES` first |
| `src/paste.ts` | High-level create/retrieve/delete operations |
| `src/api.ts` | HTTP API client (optional creation token; `ApiError` with `code`/`retryAfter`); follows no redirect (`ApiError` code `redirect_refused`, exported as `REDIRECT_REFUSED`, `Location` not passed on); `MalformedHeaderError` for a describe header the parser refuses; every body (errors too) read as a counted stream, cancelled above 64 KiB, a successful retrieve above base64url of `MAX_CIPHERTEXT_BYTES` (`MAX_CAPSULE_BYTES` + 16) + 1 KiB → code `response_too_large` (`RESPONSE_TOO_LARGE`); total deadline 30 s describe/delete, 10 min create/retrieve, 30 s idle per body → code `timeout` (`TIMEOUT`); both status `0`, fixed, not configurable |
| `src/open.ts` | Recipient flow `openPaste()`: descriptor → reveal → decrypt → decode (web + CLI); a ciphertext above `MAX_CIPHERTEXT_BYTES` is `invalid_capsule` before decrypting |
| `src/links.ts` | Share/management link and fragment parsing (accepts a language segment before `p`/`manage`); `trailingLanguage()`: the language code a base URL ends in; `hasQueryOrFragment()`: a base URL with a `?` or `#` |
| `src/index.ts` | Package entry, re-exports the public modules |
| `src/languages.ts` | `KNOWN_LANGUAGES` — interface language codes |
| `src/public_header.ts` | Protocol constants (incl. `MIN_PBKDF2_ITERATIONS`, `MAX_PBKDF2_ITERATIONS`, `PASSWORD_SALT_BYTES`), public header type, its parser (password parameter bounds) and canonical serialization |
| `src/helpers/bytes.ts` | Byte array utilities (base64url, hex, concatenation) |
| `src/helpers/ulid.ts` | ULID generation |
| `src/helpers/format.ts` | Size and date formatting (English; the web client uses `language.ts`) |

### Web client (`src/server/web/`)

| File | Purpose |
|------|---------|
| `src/pages/create.ts` | Creation form: item tabs, "+ Add item" menu, per-item editor, drag & drop |
| `src/pages/view.ts` | Capsule view: decrypt, sticky item tabs, copy/download/source per item |
| `src/pages/manage.ts` | Management page (delete, burn status) |
| `src/pages/open-capsule.ts` | Password prompt + decrypt flow |
| `src/pages/render-items.ts` | Item rendering dispatch |
| `src/pages/item-tabs.ts` | Shared item tab behaviour (keyboard, reveal) for create and view |
| `src/pages/shell.ts` | Page shell (theme, layout, language switch; `languageFromNextPage`/`isLeavingPage` keep the switch and a retrieval, deletion or draft from interfering) |
| `src/pages/errors.ts` | Catalogue error texts by HTTP status and problem `code` |
| `src/pages/report.ts` | Abuse report form (strips the fragment, only the id is sent) |
| `src/pages/legal.ts` | Legal pages: shell only, the server renders the text (ADR-015) |
| `src/render-markdown.ts` | Markdown rendering (marked + DOMPurify) |
| `src/render-code.ts` | Code rendering (highlight.js) |
| `src/theme.ts` | Theme switching |
| `src/language.ts` | Page language from `<html lang>`, `register()` and `t()`/`tn()` over the registered catalogue, `Intl` formatting, paths in a language (default from the meta's `data-default`), switch logic |
| `src/language-prefix.ts` | `languagePrefix(defaultLanguage, language)`: the one prefix rule (none for the default language, `/<lang>` otherwise), no imports and no DOM; shared by `language.ts`, the shell generator (and through `fillServerTokens()` the dev server), tests |
| `src/i18n/<lang>.json` | Catalogues (plain text, plural keys `<key>#<category>`) |
| `src/i18n/register-<lang>.ts` | Catalogue module of one language; its shells load it before the page script |
| `src/i18n/language-next-page.ts` | The switch's "applies from the next page" notice in every language (shown in the chosen language) |
| `src/default-language.ts` | Built-in default `en`: fallback without a usable meta, default language of the Vite dev server |
| `scripts/shells.ts` | Generates one shell per template and language (`.shells/`, built into `dist/<lang>/`), including the languages `<meta>` and the tokens for prefix, default language, name, favicon and source link (after escaping, `{name}` in a catalogue text becomes the name token, `{project}` the `CapsuleBin` link to the source token, opened in a new tab); `fillServerTokens()` fills them with the defaults for the dev server |
| `vite.config.ts` | One build over all generated shells; dev server serves the page routes with the default `en`, fills the tokens and answers `404` for every raw template path |
| `public/theme-init.js` | Head script: applies the stored theme and sets the `color-scheme` meta before the paint |
| `public/language-init.js` | Head script (after `theme-init.js`, which sets the `color-scheme` meta): moves the page to the stored, browser or default language before the paint |
| `src/trusted-html.ts` | Trusted HTML helper (CSP compliance) |
| `src/styles/tokens.css` | Design tokens |
| `src/styles/base.css` | Base styles |
| `src/styles/code.css` | Code block styles |
| `src/styles/prose.css` | Prose/typography styles |
| `public/favicon.svg` | Built-in favicon (the symbol), linked unless `branding.favicon` is set |
| `*.html` | Shell templates with `{{lang}}`/`{{languages}}`/`{{prefix}}`/`{{default}}`/`{{name}}`/`{{favicon}}`/`{{<key>}}` placeholders and the logo/wordmark marker regions in the header (`index.html` also the tagline region around its slogan); no literal `CapsuleBin`: `index.html` (create), `p.html` (view), `manage.html`, `report.html`, `legal.html` (imprint, privacy, terms) |

### CLI (`src/cli/`)

| File | Purpose |
|------|---------|
| `src/main.ts` | Executable entry point (bundled to `dist/cli.js`) |
| `src/cli.ts` | Argument parsing, help text, command dispatch; `run(argv, io)` returns the exit code; `put` refuses a server URL with a query or fragment or ending in a language code; `get --server` (else a usable `CAPSULEBIN_SERVER`) is the anchor for `CAPSULEBIN_PASSWORD`; `get --here` (refused together with `-o`); `checkOptions` refuses an empty value of every option in `PATH_OPTIONS` (`--password-file`, `--token-file`, `--delete-link-file`, `--link-file`, `-o`) with exit `2`; error output through `escapeDiagnostic`/`jsonLine`, plus the `deleteLink` of a failed `put` (second stderr line, `delete_url`/`delete_link_file` in the JSON error) |
| `src/commands/{put,get,delete}.ts` | The three commands; `put` sums regular file sizes and refuses above `MAX_CAPSULE_BYTES` before reading, then reads all inputs (files as streams, stdin) within one shared budget, exit `2` (`the input exceeds the capsule limit of 10485760 bytes`); `put` retries `timeout`, never `response_too_large`; `put --delete-link-file` (exclusive `0600` file written before the upload, removed only if that write fails); once `createWithRetry` is entered a failure carries the delete link (`withDeleteLink`: `{ file }` or `{ url }`), the file is never removed; `put` passes its server to `creationToken`; `get` never writes item content to stdout: every capsule, a single item included, goes to a new `0700` directory `capsule-<local date>-<id prefix>` (non-recursive `mkdir`, free name ` (n)` if taken), `--here` the current directory, `-o DIR` created `0700` with parents; each file opened `wx` with mode `0600` (no temp file, `link` or `rename`), on `EEXIST` the next free name `name (n).ext`, at most 1000 attempts (`no_free_name`); a failed write removes the files and the empty new directory of this run; `safeFileName` keeps the last path component and replaces C0, DEL and C1 |
| `src/terminal.ts` | `escapeDiagnostic` (every control character incl. `\n`, for stderr: applied centrally in `run()`'s error output incl. its "capsule may exist" delete link line, to `get`'s `created …`/`wrote …` lines, `put`'s `delete link written to …` line, both `secrets.ts` notices (environment password not used, `CAPSULEBIN_TOKEN` withheld) and `main.ts`'s last-resort message), `jsonLine` (every `--json` output; DEL/C1 as `\u00NN`). Backslashes are not doubled |
| `src/exit.ts` | Stable exit codes and the API-error → exit-code mapping (core's `REDIRECT_REFUSED`, `RESPONSE_TOO_LARGE`, `TIMEOUT` → `7` with their own reason, checked before the status); `promptStopped()`: Ctrl-C (`interrupted`) or EOF (`input_closed`) at a prompt → `8`; `CliError.deleteLink` (`DeleteLink`) and `withDeleteLink()` for a `put` that failed after its create request |
| `src/items.ts` | Presentation/language inference for input files |
| `src/secrets.ts` | Password and creation-token sources (file, env, prompt — never argv); `put` refuses a whitespace-only password; `CAPSULEBIN_PASSWORD` for `get` only when the link's base equals the anchor; `creationToken(io, tokenFile, server)`: `--token-file` for any server, `CAPSULEBIN_TOKEN` withheld (notice on stderr) when `CAPSULEBIN_SERVER` is set and differs from `server` after `normalizeBase` or is no URL; `readLink`: positional link, `-` (stdin) or `--link-file`; every file and the stdin link read only to the first line ending, at most `MAX_LINE_BYTES` (16 384, fixed; `\r` before `\n` not counted), a longer line a usage error naming the source, never the content; presence checked with `!== undefined` |
| `src/transport.ts` | HTTPS-or-loopback policy (`localhost`, `127.0.0.0/8`, `[::1]`; else `--allow-http`/`CAPSULEBIN_ALLOW_HTTP=1`), `normalizeBase()` for comparing server bases |
| `src/io.ts` | Process I/O behind an interface, replaced in tests; `Io.readStdin(limit)` and `readBounded(chunks, limit)` (limit required, no unbounded read) keep at most `limit + 1` bytes and leave the iteration once over the limit (content of `put` only); `readFirstLine(chunks, limit)` and `Io.readStdinLine(limit)` return the first line (`null` above the limit) and leave the iteration at the line ending or once the line is too long, so an open source does not block and at most the limit plus one chunk is read; `terminalPrompt` gives readline a real `Writable` (muted for hidden input), restores the terminal on answer, Ctrl-C and EOF |
| `tests/cli.test.ts` | End-to-end against the in-memory API in `src/core/tests/fake-server.ts` |
| `tests/terminal.test.ts`, `tests/pty.ts`, `tests/pty-driver.py` | The bundled CLI (built with `scripts.build` from `package.json`, into a temp dir) in a python3 `pty` pseudo-terminal against the fake API on `127.0.0.1:0`; input is sent only after the expected prompt text appeared; without python3 on PATH `requirePython()` in `pty.ts` throws a message naming it (checked by `tests/pty-requirement.test.ts`) |

## Required checks

Every command under [Test commands](#test-commands-ci-gates) must pass before a change is merged,
including the acceptance run against a release binary. Dependency advisories are checked with
`cargo deny check` and `npm audit --audit-level=high`; accepted exceptions are documented in
`deny.toml`.

`.github/workflows/ci.yml` runs all of this on every pull request and on `main` in eight jobs —
`Rust`, `TypeScript` (on the Node.js version in `.node-version`), `TypeScript (Node 26)` (the tests
on the newest LTS), `Acceptance (release binary)`, `Browser (Chromium, Firefox, WebKit)` (the
Playwright tests against the acceptance job's binary), `Container image` (builds the `Dockerfile`,
scans the image with Grype and waits until both an environment-configured container and one configured only by a read-only mounted file report healthy), `Support windows` and
`Dependency audit` — each a required check. Actions are pinned to commit SHAs; Dependabot proposes updates for them, Cargo, npm
and the base image digests.

`Support windows` (`scripts/support-windows.mjs`) checks that every place naming the Node.js
version agrees with `.node-version`, and warns before Node.js or the Debian release under the base
images reaches its end of support: Node.js with a warning at 180 days and a failure at 90, Debian
at 365 and 180. It, the container job and the audit job also run weekly, so an advisory or an
approaching end of support surfaces without a code change.

The `TypeScript` job builds the web client twice from scratch (`scripts/reproducible-web.sh`) and
fails unless both builds are byte-identical; the release binary embeds those assets, so a tag's
client can be rebuilt and compared. The image scan fails on a high or critical vulnerability with a
fixed version in the base image's Debian packages; the fix is a new base image digest (Dependabot
proposes one weekly, or bump it by hand). The release binaries are built from the locked
dependencies with `SOURCE_DATE_EPOCH` set, but nothing builds them twice and compares.

## Commits and pull requests

Commit messages, pull request titles and squash-merge subjects follow
[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/) — `fix(server): …`,
`feat(web): …`, `build(deps): …`, `ci: …`, `docs: …`; scopes `server`, `web`, `cli`, `core`, `deps`,
`ci`, `release` (see `CONTRIBUTING.md`). Every commit carries a DCO sign-off (`git commit -s`).

## Releasing

`.github/workflows/release.yml` runs on a tag `vX.Y.Z` and publishes binaries, the CLI bundle, the
multi-arch image on GHCR, a source SBOM, `SHA256SUMS` and keyless cosign signatures, with the
release notes taken from `CHANGELOG.md` (`scripts/release-notes.sh`). Steps for a release:

1. One pull request that sets the version in `Cargo.toml` (`[workspace.package]`), the three
   `package.json` files under `src/` and `package-lock.json`, and turns `## [Unreleased]` into
   `## [X.Y.Z] - YYYY-MM-DD` with a fresh empty `[Unreleased]` above it.
2. Optionally run the workflow by hand (`gh workflow run release.yml`): it builds and tests both
   architectures for the version in `Cargo.toml` without publishing anything.
3. After the merge, tag `main` with a signed tag and push it:
   `git tag -s vX.Y.Z -m "CapsuleBin X.Y.Z" && git push origin vX.Y.Z`.

The workflow refuses a tag that does not match the versions, or a changelog section without a date.

## Security boundaries (when editing code)

- **Never add an endpoint or code path that accepts or returns capsule plaintext or derives it
  server-side.**
- **Never log a payload, capability secret, token, or full paste ID** — see `access_log.rs`.
- **Never compare verifiers with `==`** — always use constant-time comparison (`subtle` crate).
- **Layer order in `router()` is security-critical** — security headers and request ID must
  be outermost (below only the access log) so they reach every response, including rejections;
  the access log's `record_path` must stay outside every rejecting layer, so a `429` or `413` logs
  the route template. Never log a path taken from the request: no filename or segment heuristic,
  only `MatchedPath`, a served asset or `{unmatched}`.
- **Indistinguishable error responses** — don't add different status codes or messages for
  "not found" vs "wrong capability" vs "expired" vs "moderated".
- **Page routes and the redirect from the default language's prefix never inspect the path id** —
  same bytes for every id under `/p/{id}` and `/{lang}/p/{id}`, same redirect, no retrieval
  (ADR-004, ADR-016).
- **Catalogue values are plain text** — the shell generator escapes them, pages insert them as
  text; never as HTML.
- **`[branding]` values are escaped and filled in one pass** — a name, tagline or URL that looks like a
  token or marker must stay literal (`fill_tokens` in `api/assets.rs` never rescans an inserted
  value; tokens filled one `replace` after another would expand it). Never accept SVG or other
  active documents from the operator (ADR-017).
- **Database files and operator scratch entries are created by CapsuleBin, exclusively and private**
  — never leave creation to SQLite or `VACUUM INTO` in a reachable path, never use a fixed scratch
  name, never verify or open a restore input in place (see [Database files, backup and
  restore](#database-files-backup-and-restore)).
- **Report text is personal data** — never log it; it has its own retention (`abuse.report_retention`).
- **The public header is canonical JSON** — field order and whitespace are fixed because it's
  AES-GCM AAD. Changing serialization breaks all existing capsules.
- **`unsafe_code = "forbid"`** is workspace-wide. Don't add it.
