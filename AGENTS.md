# AGENTS.md — CapsuleBin

## Overview

CapsuleBin is a minimal, accountless paste service for text, code, Markdown, and small files.
Encryption and decryption happen exclusively in the client; the server stores opaque ciphertext
and lifecycle metadata and **never sees capsule plaintext, keys, or link secrets.**

> **Non-negotiable invariant:** no API endpoint accepts or returns the plaintext of a capsule.
> The server never sees the link secret, the password, or the AES key. The only plaintext it
> receives is an abuse report's description and optional contact (personal data, ADR-013).

Version 1.0 (protocol v1 frozen, see ADR-003 and `CHANGELOG.md`). A multi-item encrypted capsule works end-to-end:
create, describe, retrieve, delete, burn-after-retrieval, password protection, Markdown/code
rendering, file items, retention/pruning, rate limiting, idempotent create, the CLI
(`put`/`get`/`delete`), abuse reports and the operator moderation commands are all implemented.
The web interface is in English and German, every page under a language prefix (`/en/`, `/de/`).

## Repository layout

```
capsulebin/
├── src/
│   ├── server/         Rust crate `capsulebin-server` — Axum server, config, DB, HTTP layer
│   │   ├── src/        Rust source
│   │   ├── web/        @capsulebin/web — browser client (Vite/Lit), embedded into server binary
│   │   └── build.rs    Embeds src/server/web/dist at compile time via rust-embed
│   ├── cli/            @capsulebin/cli — standalone command-line client (esbuild, Node.js)
│   └── core/           @capsulebin/core — platform-neutral capsule format, crypto, API client
│                         (no DOM, no build step; shared by web client and CLI)
├── migrations/         SQLite migrations (0001–0003), embedded into server binary
├── docs/
│   ├── adr/            16 Architecture Decision Records (see below)
│   ├── capsule-file-format.md   Language-neutral binary container specification
│   ├── crypto-profile.md        Binding crypto specification (primitives, derivation, verifiers)
│   ├── threat-model.md          Assets, adversaries, controls + the tests that back them
│   ├── operations.md            Deployment, reverse proxy, backups, upgrades, abuse runbook
│   ├── privacy-notice.md        What an instance processes; template for the operator's notice
│   ├── i18n.md                  Interface languages: prefixes, redirect, catalogues, adding a language
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
   reveal/delete/create tokens. Comparisons use constant-time (`subtle` crate), never `==`.
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

- Max 16 items, max 10 MiB plaintext per capsule, max 64 KiB manifest.
- Padded with `0x00` to the PADMÉ length; decoders reject non-zero padding.
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

Errors are `application/problem+json` with a stable `code` field and `X-Request-Id`.
Clients branch on `code`, never on `title`.

Pages live under a language prefix per offered language (ADR-016, [`docs/i18n.md`](docs/i18n.md)):
`/{lang}/`, `/{lang}/p/{id}`, `/{lang}/manage/{id}`, `/{lang}/report`, `/{lang}/imprint`,
`/{lang}/privacy`, `/{lang}/terms`. The same paths without a prefix answer `302` to the language
from `Accept-Language` (`Vary: Accept-Language`, `no-store`); a prefix that is not offered is the
generic `404`. Share and management links are created without a prefix. API, `/health/*` and static
assets have no prefix, and no `*.html` is served directly.

## Configuration

See `capsulebin.example.toml` for all settings with defaults.

Critical rules:
- **Unknown keys are a startup error** — a typo must not silently disable a security setting.
- **Every setting has an env equivalent:** `CAPSULEBIN__<TABLE>__<KEY>`.
- **`server.public_url` is required, no default** — it appears in `problem+json` type URIs
  and share links, so it must come from configuration, not a request header.
- **`creation.mode` defaults to `token`** — an instance reachable before configuration
  must not be an open write target.
- **`[legal]` texts are operator input placed into pages** (ADR-015) — Markdown rendered at
  startup without raw HTML or images and with links limited to http(s), mailto, tel and local
  paths; `url` values limited to absolute http(s) URLs and single-slash paths. The built-in
  privacy notice and terms (`legal/defaults.rs`) describe what the server stores and
  logs: change them together with any such behaviour.
- **`[i18n]` `languages` / `default_language`** (default `["en", "de"]` / `"en"`) — only codes
  from `KNOWN_LANGUAGES`, no duplicates, the default must be offered; anything else is a startup
  error. The known languages exist three times (server `config/model.rs`, core `languages.ts`,
  catalogue files in `src/server/web/src/i18n/`), kept equal by `tests/languages.rs` and
  `i18n-catalogues.test.ts`.

### Retention (purge)

Three independent rules, each deletes the whole row (metadata + payload):
- **expired:** measured from `expires_at`
- **never retrieved:** creation + `never_retrieved_after` (default 30d)
- **inactive:** last authorized retrieval + `inactive_after` (default 90d)

Runs at startup and every 24h. `prune` subcommand applies once on demand.
`purge.enabled = false` disables the scheduled job.

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
In-memory only, resets on restart; full buckets are swept above 10 000 entries.

## Build and test

### First-time setup
```sh
npm install          # resolves src/{core,cli,server/web} workspaces
npm run build -w @capsulebin/web   # build frontend (required before cargo build)
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
| `healthcheck` | Probe the running server's `/health/ready` (`--live`, `--address`); exit 0 if healthy (container `HEALTHCHECK`) |
| `migrate`| Apply pending migrations and exit |
| `prune`  | Apply retention policy once and exit |
| `token`  | Generate a creation token + the hash to configure (domain-separated) |
| `reports`| Abuse report queue: `list`, `show`, `review`, `close` |
| `moderation` | `disable` (reversible), `delete` (final), `restore`, `status` by paste id |
| `backup` | `VACUUM INTO` snapshot of the live DB (compacted, 0600, verified, never overwrites) |
| `restore`| Verify a backup and replace the DB (server stopped; `--force` moves the old DB aside, `--dry-run`) |

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
npm run check -w @capsulebin/cli && npm test -w @capsulebin/cli
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
| `src/server/tests/secure_delete.rs` | Burned and deleted ciphertext is gone from the raw database file and WAL after a checkpoint (`secure_delete`). |
| `src/server/tests/logging.rs` | Log output never contains payload, capability secrets, tokens, full paste IDs, or report text. **Mandatory CI gate.** |
| `src/server/tests/rate_limits.rs` | Limits apply before lookup (no burn), invalid-capability throttling, byte quota, X-Forwarded-For spoofing. |
| `src/core/tests/properties.test.ts`, `src/server/tests/properties.rs` | Property-based tests (fast-check / proptest) for every parser of untrusted input. |
| `src/server/tests/concurrency.rs` | Burn race, parallel retrievals, delete vs. retrieve through the HTTP stack. |
| `src/server/tests/upgrade.rs` | A database written by 1.0 (`tests/fixtures/capsulebin-1.0.sql`) migrates and stays usable: capabilities, burn tombstones, password headers, reports, moderation. Fails on an edited released migration. |
| `src/server/tests/abuse.rs` | Reports answer identically for existing/unknown ids; moderated pastes look like unknown ones. |
| `src/core/tests/golden-vectors.test.ts` | `@capsulebin/core` against `docs/test-vectors/capsule-v1.json`. **Mandatory CI gate.** |
| `src/server/web/tests/contrast.test.ts` | WCAG contrast ratio of every design token pair. |
| `src/server/tests/languages.rs`, `src/server/web/tests/i18n-catalogues.test.ts` | Server list, core list and catalogue files name the same languages; catalogues have the same keys, every plural form, no empty value. |
| `src/server/web/tests/shells.test.ts` | Every visible text in a shell template comes from the catalogue; generated shells keep the server markers and link only within their language. |
| `scripts/smoke.sh` | Full lifecycle over a real socket against a release binary, asserts security headers on every response; with `CAPSULEBIN_CLI` also CLI put/get/delete. |
| `src/cli/tests/cli.test.ts` | CLI exit codes, secret handling (never argv), burn confirmation, retry-with-replay. |
| `src/server/web/e2e/*.e2e.ts` | Playwright against the release binary in Chromium, Firefox and WebKit: every user flow, the legal pages, hostile Markdown and a capsule at the size limit, a German flow without English text, the language switch and redirects (`i18n.e2e.ts`), failing on any CSP/Trusted Types violation, page or console error, or executed XSS canary (`e2e/guard.ts`, which `guard.e2e.ts` checks itself). |

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
| 011 | Idempotent create | The paste id is the idempotency key: an identical re-submission returns the original `201`, anything else stays `409` |
| 012 | No OpenAPI document | Contract = capsule format, crypto profile, golden vectors; if ever needed, generate from `wire.rs` |
| 013 | Abuse reports and moderation | Reports only stored for existing pastes, same answer either way; operator CLI; moderated = indistinguishable from unknown |
| 014 | AGPL licence, DCO | AGPL-3.0-only open source; contributions with a DCO sign-off (`git commit -s`), no CLA |
| 015 | Legal pages rendered by the server | Imprint, privacy notice, terms from `[legal]`, built-in texts filled in from the configuration; restricted Markdown rendered at startup; code still only from the binary |
| 016 | Language paths | Every page under `/{lang}/`, prefix-less paths redirected by `Accept-Language`, shells generated per language at build time, browser choice `localStorage` > browser language > default; no cookie |

## Source code map

### Rust server (`src/server/`)

| File | Purpose |
|------|---------|
| `src/main.rs` | Binary entry point, CLI argument parsing (clap) |
| `src/lib.rs` | `AppState`, `router()` — layer order is security-critical (see comments) |
| `src/config.rs` | Config loading: TOML + env override, validation, `Resolved` |
| `src/config/model.rs` | Config struct definitions; `I18nConfig`, `KNOWN_LANGUAGES` |
| `src/config/env.rs` | Environment variable mapping (`CAPSULEBIN__...`) |
| `src/config/units.rs` | Duration and byte-size parsing |
| `src/db.rs` | Database connection pool, migration runner |
| `src/healthcheck.rs` | Client side of `capsulebin healthcheck`: one HTTP/1.1 request, status line only |
| `src/backup.rs` | Backup (`VACUUM INTO`), backup verification, restore |
| `src/domain.rs` | Domain types: `PasteId`, `Verifier`, `Capability`, `Clock` |
| `src/domain/paste.rs` | `Paste` struct — the full row representation |
| `src/domain/capability.rs` | Capability verification (constant-time via `subtle`) |
| `src/domain/purge.rs` | Retention rule evaluation |
| `src/domain/abuse.rs` | Report validation, reason/resolution/action codes (ADR-013) |
| `src/repository.rs` | `PasteRepository` trait |
| `src/repository/sqlite.rs` | SQLite implementation (SQLx queries) |
| `src/repository/sqlite/abuse.rs` | `AbuseRepository`: reports, moderation, audit trail, retention |
| `src/api.rs` | API module root |
| `src/api/pastes/handlers.rs` | Route handlers: create, describe, retrieve, delete |
| `src/api/pastes/codec.rs` | Wire format encode/decode |
| `src/api/pastes/wire.rs` | Request/response types |
| `src/api/health.rs` | Health endpoints |
| `src/api/assets.rs` | Page routes per language prefix, `Accept-Language` redirect of prefix-less paths, asset serving (embedded or from disk); puts the languages meta and the legal footer links into every shell and legal pages into `legal.html` |
| `src/legal.rs` | `[legal]` resolution per offered language: imprint, privacy notice, terms; footer links with the language prefix (`legal/markdown.rs`: restricted Markdown, `legal/defaults.rs`: built-in texts) |
| `src/api/problem.rs` | `application/problem+json` error responses |
| `src/api/request_id.rs` | Per-request UUID, echoed in response body and `X-Request-Id` |
| `src/api/access_log.rs` | Redacted access logging (method + redacted path only) |
| `src/api/rate_limit.rs` | Token-bucket rate limiting (per minute or per hour) |
| `src/api/reports.rs` | Report endpoint and policy |
| `src/api/client_ip.rs` | Client IP extraction with trusted proxy support |
| `src/api/security_headers.rs` | Security headers on every response |
| `src/commands/` | Subcommand implementations (serve, check, healthcheck, migrate, prune, token, backup/restore, reports/moderation in `abuse.rs`) |
| `build.rs` | Embeds `web/dist` and `migrations/` into the binary |

### TypeScript core (`src/core/`)

| File | Purpose |
|------|---------|
| `src/crypto.ts` | Key derivation, encryption/decryption (Web Crypto) |
| `src/capsule.ts` | Capsule file format encode/decode |
| `src/paste.ts` | High-level create/retrieve/delete operations |
| `src/api.ts` | HTTP API client (optional creation token; `ApiError` with `code`/`retryAfter`) |
| `src/open.ts` | Recipient flow `openPaste()`: descriptor → reveal → decrypt → decode (web + CLI) |
| `src/links.ts` | Share/management link and fragment parsing (accepts a language segment before `p`/`manage`) |
| `src/languages.ts` | `KNOWN_LANGUAGES` — interface language codes |
| `src/public_header.ts` | Protocol constants, public header type and its canonical serialization |
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
| `src/pages/shell.ts` | Page shell (theme, layout, language switch) |
| `src/pages/errors.ts` | Catalogue error texts by HTTP status and problem `code` |
| `src/pages/report.ts` | Abuse report form (strips the fragment, only the id is sent) |
| `src/pages/legal.ts` | Legal pages: shell only, the server renders the text (ADR-015) |
| `src/render-markdown.ts` | Markdown rendering (marked + DOMPurify) |
| `src/render-code.ts` | Code rendering (highlight.js) |
| `src/theme.ts` | Theme switching |
| `src/language.ts` | Page language from `<html lang>`, `t()`/`tn()`, `Intl` formatting, prefixed paths, switch logic |
| `src/i18n/<lang>.json` | Catalogues (plain text, plural keys `<key>#<category>`) |
| `scripts/shells.ts` | Generates one shell per template and language (`.shells/`, built into `dist/<lang>/`) |
| `public/language-init.js` | Head script: moves the page to the stored, browser or default language before the paint |
| `src/trusted-html.ts` | Trusted HTML helper (CSP compliance) |
| `src/styles/tokens.css` | Design tokens |
| `src/styles/base.css` | Base styles |
| `src/styles/code.css` | Code block styles |
| `src/styles/prose.css` | Prose/typography styles |
| `*.html` | Shell templates with `{{lang}}`/`{{<key>}}` placeholders: `index.html` (create), `p.html` (view), `manage.html`, `report.html`, `legal.html` (imprint, privacy, terms) |

### CLI (`src/cli/`)

| File | Purpose |
|------|---------|
| `src/main.ts` | Executable entry point (bundled to `dist/cli.js`) |
| `src/cli.ts` | Argument parsing, help text, command dispatch; `run(argv, io)` returns the exit code |
| `src/commands/{put,get,delete}.ts` | The three commands |
| `src/exit.ts` | Stable exit codes and the API-error → exit-code mapping |
| `src/items.ts` | Presentation/language inference for input files |
| `src/secrets.ts` | Password and creation-token sources (file, env, prompt — never argv) |
| `src/io.ts` | Process I/O behind an interface, replaced in tests |
| `tests/cli.test.ts` | End-to-end against the in-memory API in `src/core/tests/fake-server.ts` |

## Required checks

Every command under [Test commands](#test-commands-ci-gates) must pass before a change is merged,
including the acceptance run against a release binary. Dependency advisories are checked with
`cargo deny check` and `npm audit --audit-level=high`; accepted exceptions are documented in
`deny.toml`.

`.github/workflows/ci.yml` runs all of this on every pull request and on `main` in eight jobs —
`Rust`, `TypeScript` (on the Node.js version in `.node-version`), `TypeScript (Node 26)` (the tests
on the newest LTS), `Acceptance (release binary)`, `Browser (Chromium, Firefox, WebKit)` (the
Playwright tests against the acceptance job's binary), `Container image` (builds the `Dockerfile`,
scans the image with Grype and waits for the container to report healthy), `Support windows` and
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
  be outermost so they reach every response, including rejections.
- **Indistinguishable error responses** — don't add different status codes or messages for
  "not found" vs "wrong capability" vs "expired" vs "moderated".
- **Page routes and the prefix redirect never inspect the path id** — same bytes for every id under
  `/{lang}/p/{id}`, same redirect from `/p/{id}`, no retrieval (ADR-004, ADR-016).
- **Catalogue values are plain text** — the shell generator escapes them, pages insert them as
  text; never as HTML.
- **Report text is personal data** — never log it; it has its own retention (`abuse.report_retention`).
- **The public header is canonical JSON** — field order and whitespace are fixed because it's
  AES-GCM AAD. Changing serialization breaks all existing capsules.
- **`unsafe_code = "forbid"`** is workspace-wide. Don't add it.
