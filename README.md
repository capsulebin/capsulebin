# CapsuleBin

**Paste it. Seal it. Share it.** · [capsulebin.eu](https://capsulebin.eu)

A minimal, accountless paste service for text, code, Markdown and small files. Encryption and
decryption happen exclusively in the client; the server stores opaque ciphertext and lifecycle
metadata.

> **Non-negotiable invariant:** no API endpoint accepts or returns the plaintext of a capsule.
> The server never sees the link secret, the password or the AES key. The only plaintext it
> receives is what a reporter writes into an abuse report — a description and an optional
> contact, handled as personal data.

## Status

**1.0** — the first stable release. The protocol (envelope, capsule file format, crypto profile and
HTTP API `v1`) is frozen: capsules created with 1.0 stay readable by every later 1.x release, and
breaking changes need a new version in the path or format
([ADR-003](docs/adr/ADR-003-protocol-versioning.md), [crypto profile](docs/crypto-profile.md),
[capsule file format](docs/capsule-file-format.md)).
What changed between releases is in the [changelog](CHANGELOG.md).

What 1.0 contains:

- **Capsules** with one or more text, code, Markdown or file items, encrypted in the browser or the
  command line client; the server only ever stores and returns ciphertext.
- **Lifecycle:** expiry, burn after first retrieval (enforced server-side, race-free), password
  protection, deletion through an independent management link, idempotent create for safe retries.
- **Rendering:** Markdown and syntax-highlighted code, sanitized and under a strict CSP.
- **Command line client** (`put` / `get` / `delete`) sharing the browser's crypto implementation.
- **Operations:** a single binary with embedded assets and migrations, SQLite, retention policy and
  scheduled purge, rate limiting, backup and restore, health endpoints.
- **Abuse handling:** a report form and an operator CLI for the report queue and moderation, without
  the operator ever decrypting a capsule.

## Layout

| Path | Contents |
| --- | --- |
| `src/server` | Axum server, configuration, repository, HTTP layer |
| `src/server/web` | `@capsulebin/web` — the browser client; Vite is only a dev server and bundler; embedded into the server binary |
| `src/cli` | `@capsulebin/cli` — the command line client |
| `src/core` | `@capsulebin/core` — capsule format, crypto, API client; no DOM, no build step |
| `migrations` | SQLite schema, embedded into the binary |
| `docs/adr` | Architecture decisions and their rationale |
| `docs` | Language-neutral capsule format and crypto profile specs, plus `test-vectors` |

`docs/test-vectors` holds the fixtures that keep implementations of the capsule format in step,
including negative vectors for every rejection rule. The format is specified in
[`docs/capsule-file-format.md`](docs/capsule-file-format.md) and
[`docs/adr/ADR-008-capsule-file-format.md`](docs/adr/ADR-008-capsule-file-format.md); `@capsulebin/core`
is its reference implementation ([ADR-009](docs/adr/ADR-009-typescript-cli-and-core-package.md)).

## Development

Requirements: Rust (version from `rust-toolchain.toml`) and Node.js 22.

```sh
# The server embeds src/server/web/dist at compile time, so build the frontend first.
# npm install (run once, at the repo root) resolves the src/* workspace.
npm install
npm run build -w @capsulebin/web

# capsulebin.dev.toml is committed and open by default - localhost only.
cargo run -p capsulebin-server -- --config capsulebin.dev.toml serve
```

The server listens on `127.0.0.1:8081`.

**A configuration is mandatory.** `serve` without one aborts, because `server.public_url` has no
sensible default and `creation.mode` defaults to `token` with no tokens configured — an instance
that is reachable before it is configured must not be an open write target. The equivalent without
a file:

```sh
CAPSULEBIN__SERVER__PUBLIC_URL=http://localhost:8080 \
CAPSULEBIN__CREATION__MODE=public \
cargo run -p capsulebin-server -- serve
```

For frontend work with hot reload:

```sh
cargo run -p capsulebin-server -- --config capsulebin.dev.toml serve   # terminal 1, :8081
npm run dev -w @capsulebin/web                                         # terminal 2, :5173
```

Vite proxies `/api` and `/health` to the server. Browsers treat `localhost` as a secure context, so
Web Crypto is available there; production requires HTTPS.

`capsulebin.dev.toml` reads assets from `src/server/web/dist` rather than the binary, so an edited
file takes effect without a rebuild. Without `server.asset_dir`, every build — release builds
included — serves the copy embedded in the binary; leave it unset in production.

### Command line client

`@capsulebin/cli` encrypts and decrypts locally with the same `@capsulebin/core` the web client
runs, so a capsule created on one side opens on the other:

```sh
npm run build -w @capsulebin/cli
export CAPSULEBIN_SERVER=http://127.0.0.1:8081

# One capsule from several files (kind inferred: Markdown, code by extension, text, or file)
node src/cli/dist/cli.js put notes.md main.rs logo.png --expires 7d
# From stdin, burn after the first retrieval, password from the environment
echo "db password: hunter2" | CAPSULEBIN_PASSWORD=... node src/cli/dist/cli.js put --burn --password

node src/cli/dist/cli.js get 'http://127.0.0.1:8081/p/…#v1.…'            # one item → stdout
node src/cli/dist/cli.js get 'http://127.0.0.1:8081/p/…#v1.…' -o out/    # all items → out/
node src/cli/dist/cli.js delete 'http://127.0.0.1:8081/manage/…#v1.…'
```

- **Secrets are never arguments.** Passwords come from `--password-file`, `CAPSULEBIN_PASSWORD`
  or a hidden prompt; creation tokens from `--token-file` or `CAPSULEBIN_TOKEN`.
- **`--json`** prints a single JSON object on stdout — the links on `put`, item metadata and
  written paths on `get`, `{"error": {"reason", "message"}}` on failure.
- **Exit codes** are stable: `0` ok, `2` usage, `3` not available (unknown, expired, deleted,
  consumed, wrong link or password — indistinguishable by design), `4` a burn capsule someone
  else retrieved concurrently, `5` undecryptable, `6` rejected by the server (token, size,
  expiry, rate limit), `7` unreachable or server error, `8` needs a password or burn
  confirmation. `capsulebin --help` lists all options.
- **Retries are safe.** `put` seals once and re-sends the identical request on network errors,
  `429` and `5xx`; the server answers a replay like the original
  ([ADR-011](docs/adr/ADR-011-idempotent-create.md)) instead of storing a second copy.
- **Burn capsules are not consumed by accident.** `get` asks first, and refuses
  non-interactively unless `--yes` is given. Once retrieved, output never fails over an existing
  file — a free name is chosen unless `--force`.

### Subcommands

| Command | Purpose |
| --- | --- |
| `serve` | Start the server |
| `check` | Verify configuration, database and migrations without opening a port |
| `healthcheck` | Probe the running server's `/health/ready` (`--live`: `/health/live`); exit 0 if healthy. For container health checks |
| `migrate` | Apply pending migrations and exit |
| `prune` | Apply the retention policy (capsules, closed reports, moderation records) once and exit |
| `token` | Generate a creation token and the hash to configure for it |
| `backup --output FILE` | Consistent, compacted snapshot of the live database (owner-only file) |
| `restore --input FILE` | Verify a backup and put it in place of the database (server stopped; `--force`, `--dry-run`) |
| `reports list\|show\|review\|close` | Work through the abuse report queue |
| `moderation disable\|delete\|restore\|status` | Block, remove or restore a capsule by id — without decrypting it |

`token` exists because the configured hash is domain-separated: `sha256sum` of the token alone will
not match it.

### Backup and restore

```sh
capsulebin backup --output /backups/capsulebin-$(date +%F).db   # while the server runs
capsulebin restore --input /backups/capsulebin-2026-11-17.db --dry-run   # verify only
capsulebin restore --input /backups/capsulebin-2026-11-17.db --force     # server stopped
```

`backup` uses SQLite's `VACUUM INTO`: a consistent snapshot of the live database without its
write-ahead log, compacted so that pages freed by deleted or burned capsules are not copied. The
file is created with owner-only permissions and verified (integrity and schema) after writing; an
existing file is never overwritten.

`restore` verifies the backup before touching anything, uses only its main file — never a `-wal`
next to it — and refuses to replace an existing database unless `--force` is given. Even then the
old database is moved to `<file>.pre-restore-<timestamp>` together with its `-wal`/`-shm` (which may
hold committed data), never deleted. Opening the restored file applies the migrations an older
backup lacks.

A backup contains ciphertext, verifiers and abuse reports with contact details: store it as
carefully as the database. Deleted capsules can survive in older backups — burn and delete only
promise that the live instance no longer serves them. Restore only backups you made yourself.

### Abuse handling

Anyone can report a capsule on `/report` by its id; the page strips the key from a pasted link before
sending, and the server refuses text that still contains one. Every well-formed report gets the
same answer, but only reports about a capsule that currently exists reach the queue — the form is
neither an oracle for ids nor a way to flood the operator
([ADR-013](docs/adr/ADR-013-abuse-reports-and-moderation.md)).

There is no admin web interface. The operator works on the database through the binary, which can
run next to a live server:

```sh
capsulebin reports list                      # open reports, oldest first (--status all, --json)
capsulebin reports show <report-id>
capsulebin moderation disable <paste-id> --reason phishing --note "ticket 42"   # reversible
capsulebin moderation delete <paste-id> --reason illegal_content                # final
capsulebin moderation restore <paste-id>
capsulebin moderation status <paste-id>      # lifecycle and audit trail
capsulebin reports close <report-id> --resolution action_taken
```

A moderated capsule answers exactly like an unknown one — there is no "removed" notice. Closed
reports and moderation records are deleted after `abuse.report_retention` (default 90 days); open
ones stay until they are closed. Report text is personal data and is never logged.

## Configuration

`capsulebin.example.toml` documents every setting with its default. Two properties are worth
knowing before editing one:

- **Unknown keys are a startup error.** A typo such as `burn_after_retreival` would otherwise leave
  a security setting silently inactive.
- **Every setting has an environment equivalent**, named `CAPSULEBIN__<TABLE>__<KEY>` — for example
  `CAPSULEBIN__DATABASE__PATH`. That is the intended way to configure a container. A table given
  partially keeps the defaults for its other fields.

Run `capsulebin --config … check` to validate a file before deploying it.

### Retention

The `[purge]` table decides when a capsule is removed. Three rules apply independently, and each one
deletes the whole row — the metadata goes with the capsule, not just the payload:

| Rule | Measured from |
| --- | --- |
| expired | `expires_at`, whenever one was requested |
| never retrieved | creation, after `never_retrieved_after` (default 30 days) |
| inactive | the last authorized retrieval, after `inactive_after` (default 90 days) |

A capsule created without a fixed expiry is **not** exempt from the last two: "no expiry" means the
creator named no deadline, not that the instance keeps it forever.

`serve` runs the same rules once at startup and then once every 24 hours; `prune` applies them once,
on demand, which doubles as the way to force a purge right after changing the thresholds. There is
deliberately no interval setting — what an operator configures is *what* is removed, not how often
the question is asked. Set `purge.enabled = false` to turn the scheduled job off and rely on `prune`
alone, for example when it is driven by an external scheduler instead.

The timestamp behind the inactivity rule is the only retrieval metadata the server keeps — there is
no retrieval counter — and it is kept as sparingly as the rule allows: it advances at most once per
`retrieval_timestamp_granularity` (default one hour), it is never returned by any endpoint, and it
is never correlated with a client address. Inside that window a retrieval is a pure read and never
waits for SQLite's single writer. It records that an authorized handout happened — a rejected capability, an unknown id or a
plain page load leave it untouched, and it says nothing about whether the recipient could actually
decrypt what they received.

A burn-after-retrieval capsule is consumed by the first successful retrieval: the ciphertext is
handed out and the row is marked consumed in the same database transaction, so two requests racing
for the same capsule cannot both receive it. Afterwards the capsule answers like any unavailable one
(`404 not_available`); only a request that loses the race inside the burn transaction gets
`410 already_retrieved`.

### Rate limiting

The `[rate_limits]` table holds independent token buckets per endpoint class (`create`, `descriptor`,
`retrieve`, `delete`, `report`), keyed by client IP. These are off by default, because a limit keyed
by client IP is only correct once `server.trusted_proxies` matches the deployment. A class takes `requests_per_minute` or, for rare requests such as reports,
`requests_per_hour`, plus a `burst` of at least 1. A rejected request gets `429` with `Retry-After`
(the seconds until the next token) and `code = "rate_limited"`, before any paste is looked up — a
limit can neither burn a paste nor reveal whether an id exists. (The per-paste password limit below
is the one exception to "before any lookup", and only for callers who already hold the link.)

Four limits target abuse rather than load:

- `invalid_capability_per_paste` — **on by default** (10 per minute, burst 10): wrong passwords for
  one capsule, from any address. It is the server-side protection of a capsule's password once its
  link has leaked — every guess needs a retrieve — and it works without `trusted_proxies`. Only a
  retrieve that proves the link (the describe token matches) and then fails the reveal token
  counts, and only such callers ever see its `429`. Someone who knows just the id can neither
  block the capsule nor tell from the limit that it exists.

- `create.bytes_per_day` — a per-client byte quota charged with the request's `Content-Length`, so a
  handful of maximum-size uploads cannot hide behind a requests-per-minute limit.
- `create.max_concurrent` — uploads in flight per client.
- `invalid_capability` — every describe, retrieve or delete that ends in `404` costs a token; once
  the bucket is empty, further ones from that client are refused. Guessing ids or
  capabilities slows to the refill rate while legitimate use, which almost never fails, is
  unaffected.

Buckets key on the TCP peer address unless it appears in `server.trusted_proxies`, in which case the
right-most untrusted address in `X-Forwarded-For` is used instead. Behind a reverse proxy, set
`trusted_proxies` to the proxy's own address before turning a limit on — otherwise every client
behind it shares one bucket. Counters are in memory and reset on restart; this is a single-instance
mechanism, not one that survives horizontal scaling.

## HTTP API

Versioned in the path. The capsule and cryptography versions are independent of it.

| Method | Path | Purpose |
| --- | --- | --- |
| `POST` | `/api/v1/pastes` | Create; bearer token required in `token` mode |
| `POST` | `/api/v1/pastes/{id}/describe` | Prove the describe capability (or the delete secret), receive public header and lifecycle — never the ciphertext |
| `POST` | `/api/v1/pastes/{id}/retrieve` | Prove the describe and reveal capabilities, receive the ciphertext |
| `DELETE` | `/api/v1/pastes/{id}` | Prove the delete capability |
| `POST` | `/api/v1/reports` | Report a capsule by id; same `202` whether or not it exists |
| `GET` | `/api/v1/reports` | Report policy: enabled, contact, reason codes, limits |
| `GET` | `/health/live` | The process is alive |
| `GET` | `/health/ready` | Configuration, database and schema are usable |

Errors are `application/problem+json` with a stable `code` field and a request id that is also
echoed in `X-Request-Id`. Clients branch on `code`, never on `title`.

An identical re-submission of a create request (same id, verifiers, header, ciphertext and
options) is answered with the original `201`, so clients can retry safely
([ADR-011](docs/adr/ADR-011-idempotent-create.md)); any other reuse of an id is `409`.

Unknown, expired, disabled, deleted, consumed and wrong-capability all answer **identically**. Every
endpoint that takes a paste id requires a capability derived from the link, so a caller who knows
only an id cannot learn whether it exists, nor when it expires, whether it burns or needs a
password.

The ciphertext is padded (PADMÉ, at most about 12 %), so its size reveals little more than the
order of magnitude of the content.

## Tests

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
npm run format:check
npm run check -w @capsulebin/core && npm test -w @capsulebin/core
npm run check -w @capsulebin/web && npm test -w @capsulebin/web
npm run check -w @capsulebin/cli && npm test -w @capsulebin/cli
```

The TypeScript side (`src/core`, `src/cli`, `src/server/web`) is formatted with Prettier, configured
in `.prettierrc.json` at the repo root — 4-space indentation, single quotes, a 100-column width. Run
`npm run format` to apply it; `npm run format:check` (a mandatory CI gate) only verifies it.

`scripts/smoke.sh` is the acceptance check for a release binary, and the only test that exercises
the shipped artefact rather than the source. It starts the binary, drives a full lifecycle over a
real socket, and asserts the security headers on every response including the 404. With
`CAPSULEBIN_CLI` set (as in CI) it also drives the bundled CLI through `put`, `get` and `delete`
against the binary:

```sh
npm run build -w @capsulebin/web && npm run build -w @capsulebin/cli
cargo build --release --locked -p capsulebin-server
CAPSULEBIN_CLI=src/cli/dist/cli.js ./scripts/smoke.sh ./target/release/capsulebin
```

Several test suites are worth knowing about because they encode rules rather than behaviour:

- `src/core/tests/golden-vectors.test.ts` reads `docs/test-vectors/capsule-v1.json`.
  `@capsulebin/core` is the only reference implementation of the capsule format in this repository
  ([ADR-009](docs/adr/ADR-009-typescript-cli-and-core-package.md)), so this test checks it against
  the language-neutral fixture — the vectors remain the contract a future third-party
  implementation is checked against. Mandatory CI gate.
- `src/server/tests/db_no_plaintext.rs` creates a paste with known plaintext and reads
  every column of every row back out of SQLite, asserting the plaintext appears nowhere — as text,
  as hex or as base64. Also a mandatory CI gate.
- `src/server/tests/logging.rs` captures real log output across a request cycle and
  fails if a payload, a capability secret, a token or a full paste id appears in it. Logs outlive
  pastes, so this is a security test.
- `src/core/tests/properties.test.ts` and `src/server/tests/properties.rs` are property-based
  tests (fast-check, proptest) for every parser that reads untrusted input — the capsule decoder,
  the public header, base64url, links, paste ids, verifiers, units, report validation and the
  access-log redaction. Arbitrary input may only fail in the documented way, accepted values
  round-trip, and every value has exactly one accepted spelling.
- `src/server/tests/concurrency.rs` races requests through the whole HTTP stack: 32 concurrent
  retrievals of a burn paste yield exactly one handout, normal retrievals are all counted, a delete
  racing retrievals leaves a clean tombstone. `smoke.sh` repeats the burn race with 20 real
  connections against the release binary.
- `src/server/web/tests/contrast.test.ts` computes the WCAG contrast ratio of every design token
  pair, so the accessibility claim is verified rather than asserted.

## Security

The [threat model](docs/threat-model.md) lists what is protected, against whom, by which control
and test, and what is not. [SECURITY.md](SECURITY.md) describes how to report an issue. Running an
instance is covered in the [operations handbook](docs/operations.md).

The trust model and its limits are in
[`docs/adr/ADR-001-trust-model.md`](docs/adr/ADR-001-trust-model.md). Three limitations stated there
are worth repeating here:

- **The web client trusts the JavaScript served to it.** A compromised server could ship modified
  code and exfiltrate keys. Embedded assets and a strict CSP reduce this; the
  command line client (`@capsulebin/cli`, TypeScript per
  [ADR-009](docs/adr/ADR-009-typescript-cli-and-core-package.md)) reduces it further, since running
  it does not depend on trusting code served over the network for that session; nothing removes it.
- **There is no secure-erase guarantee.** Deleting removes logical availability, and SQLite's
  `secure_delete` overwrites the freed ciphertext in the database file with zeros. Write-ahead log
  frames until the next checkpoint, the filesystem, SSD wear levelling and backups are beyond its
  reach, so physical erasure of the bytes is not promised.
- **Only restore databases you created.** The server opens its SQLite database and write-ahead log
  as trusted input. Restoring a database or `-wal` file from an untrusted source is unsupported —
  the bundled SQLite is currently affected by
  [RUSTSEC-2026-0459](https://rustsec.org/advisories/RUSTSEC-2026-0459), which only such a file can
  trigger (see the exception in `deny.toml`).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Commits carry a Developer Certificate of Origin sign-off
(`git commit -s`); there is no CLA.

## Licence

[AGPL-3.0-only](LICENSE).
