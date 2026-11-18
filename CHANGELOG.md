# Changelog

All notable changes to CapsuleBin are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). The server binary, `@capsulebin/core`,
`@capsulebin/cli` and `@capsulebin/web` share one version number.

Protocol versions are separate from release versions: the envelope, the capsule file format, the
crypto profile and the HTTP API are versioned on their own
([ADR-003](docs/adr/ADR-003-protocol-versioning.md)). A release states which protocol versions it
speaks.

## [Unreleased]

## [1.0.0]

First stable release. Speaks envelope format 1, capsule file format 1, crypto profile 1 and HTTP API
`v1` ([ADR-003](docs/adr/ADR-003-protocol-versioning.md)); the golden vectors in
`docs/test-vectors/capsule-v1.json` are the conformance contract.

### Capsules and cryptography

- End-to-end encrypted capsules with up to 16 text, code, Markdown or file items and up to 10 MiB of
  plaintext, in a binary container ([capsule file format](docs/capsule-file-format.md)).
- AES-256-GCM with keys derived by HKDF-SHA-256 from a link secret that lives only in the URL
  fragment ([crypto profile](docs/crypto-profile.md)).
- Capability verifiers instead of secrets: the server stores SHA-256 hashes of the describe, reveal
  and delete capabilities and compares them in constant time.
- An independent delete secret, so recipients of a link cannot delete the capsule.
- Optional password protection (NFC-normalized, PBKDF2-HMAC-SHA-256) mixed into the key derivation,
  with no offline-checkable verifier in the public header.
- PADMÉ padding, so the ciphertext size reveals little more than the order of magnitude.

### Server

- Single binary with embedded web assets and SQLite migrations; SQLite in WAL mode with
  `synchronous = full`, `secure_delete` (freed ciphertext is zeroed in the file) and a bounded
  write-ahead log.
- Retrievals are reads: the only retrieval metadata is a coarse timestamp for the inactivity rule.
- HTTP API under `/api/v1`: create, describe, retrieve, delete, abuse reports, health endpoints;
  errors as `application/problem+json` with a stable `code`.
- Unknown, expired, deleted, moderated and wrong-capability requests answer identically.
- Burn after first retrieval, enforced in one database transaction.
- Idempotent create: an identical re-submission returns the original `201`
  ([ADR-011](docs/adr/ADR-011-idempotent-create.md)).
- Creation modes `token`, `public` and `disabled` (read-only), defaulting to `token`.
- Retention rules (expired, never retrieved, inactive) with a scheduled daily purge and `prune`.
- Per-client rate limits, a daily byte quota and concurrent-upload limit, with trusted-proxy-aware
  client addresses; a per-paste limit on password attempts (on by default) that only callers
  holding the link can trigger.
- Strict security headers and CSP on every response; redacted access logs.
- Abuse reports and operator moderation without decryption
  ([ADR-013](docs/adr/ADR-013-abuse-reports-and-moderation.md)).
- Subcommands `serve`, `check`, `migrate`, `prune`, `token`, `backup`, `restore`, `reports` and
  `moderation`.
- Configuration by TOML file and `CAPSULEBIN__<TABLE>__<KEY>` environment variables; unknown keys
  are a startup error.

### Web client

- Creation and view pages with item tabs, a sticky tab bar and an overflow menu, plus a capsule title.
- Markdown (sanitized) and code (highlight.js) rendering; copy, download and source view per item.
- Management page for deleting a capsule and checking its status; report page that strips the key
  from a pasted link.
- Light and dark themes from design tokens, WCAG contrast verified by tests.

### Command line client

- `capsulebin put`, `get` and `delete`, sharing `@capsulebin/core` with the web client.
- Secrets only from files, environment variables or a prompt — never from arguments.
- Stable exit codes, `--json` output, safe retries and confirmation before consuming a burn capsule.

[Unreleased]: https://github.com/capsulebin/capsulebin/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/capsulebin/capsulebin/releases/tag/v1.0.0
