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
  with no offline-checkable verifier in the public header. Clients refuse a header with fewer than
  100 000 iterations or a password salt other than 16 bytes before asking for the password, so the
  issuer of a link cannot choose a KDF cheap enough to guess the password offline.
- The web client and the CLI follow no redirect on an API request.
- A password of only whitespace is refused instead of silently creating an unprotected capsule.
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
  holding the link can trigger. The limiter's memory is capped (100 000 client and 100 000
  per-capsule buckets) with bounded work per request; a flood of addresses evicts the least recently
  used client, never a password budget.
- Strict security headers and CSP on every response; the access log records the matched route's
  template (`/de/p/{id}`), the path of a served static file, or `{unmatched}`, never the request
  path.
- Abuse reports and operator moderation without decryption
  ([ADR-013](docs/adr/ADR-013-abuse-reports-and-moderation.md)). Open reports are capped per
  capsule and per instance (`abuse.max_open_reports_per_paste`, default 10;
  `abuse.max_open_reports`, default 1000); a report above a ceiling is dropped with the same answer,
  and `capsulebin reports list` and a rate-limited log warning show the operator a full queue.
- Request bodies are read only when they may be used: create checks the creation token, and the
  report endpoint whether reports are enabled, before reading the body. Describe, retrieve and
  delete accept at most 4 KiB, reports 32 KiB.
- Subcommands `serve`, `check`, `healthcheck`, `migrate`, `prune`, `token`, `backup`, `restore`,
  `reports` and `moderation`.
- A container image: distroless without OpenSSL, non-root, configured by environment variables,
  with the database on a `/data` volume and a health check that needs no shell or `curl`. Every
  change and a weekly run scan it for known vulnerabilities in the base image.
- Configuration by TOML file and `CAPSULEBIN__<TABLE>__<KEY>` environment variables; unknown keys
  are a startup error.
- Legal pages (`[legal]`, [ADR-015](docs/adr/ADR-015-legal-pages-rendered-by-the-server.md)): an
  imprint from the operator's Markdown with an optional hosting provider block, and a privacy
  notice and terms of use with built-in English and German texts filled in from the configuration;
  each replaceable by own text, a file or a link, and linked from every page's footer. Every legal
  page is served in each interface language; `[legal.<lang>.*]` gives a section per language, and
  without one the default language's text is shown, marked with its `lang`.
- Operator branding (`[branding]`, [ADR-017](docs/adr/ADR-017-operator-branding.md)): the instance
  name, a slogan (per language with `[branding.<lang>]`), a header logo with an optional dark-theme
  variant and a favicon, without a rebuild. Logos and favicon are PNG, WebP or ICO files checked at
  startup and served same-origin under hashed, immutable URLs; SVG from the operator is refused.
  `[branding.colors.light]` and `[branding.colors.dark]` set the theme's colour tokens per theme
  (`#rgb` or `#rrggbb`, unset tokens keep the built-in value); the server serves them as a hashed
  stylesheet, and a pair below the built-in contrast thresholds is a warning in the log and in
  `capsulebin check`.

### Web client

- Creation and view pages with item tabs, a sticky tab bar and an overflow menu, plus a capsule title.
- Markdown (sanitized) and code (highlight.js) rendering; copy, download and source view per item.
- A wrong password keeps the password prompt open. Its message does not claim more than the server
  says - a wrong password and a capsule that went away look alike - and too many attempts show
  how long to wait.
- Management page for deleting a capsule and checking its status; report page that strips the key
  from a pasted link.
- An English and a German interface: the default language (`[i18n] default_language`) without a
  path prefix, the other under its prefix (`/de/`), chosen by a language switch or the browser
  language ([ADR-016](docs/adr/ADR-016-language-paths.md)); share and management links carry no
  prefix.
- Light and dark themes from design tokens, WCAG contrast verified by tests.
- A "Powered by CapsuleBin" link to the source code in every page's footer, on every instance, with
  a configurable target for modified versions (AGPL-3.0 §13); a built-in favicon.
- Reproducible assets: building the same source twice gives byte-identical files, checked on every
  change, so the client a server embeds can be compared with a rebuild of the release tag.

### Command line client

- `capsulebin put`, `get` and `delete`, sharing `@capsulebin/core` with the web client; runs on
  Node.js 24 or newer.
- Secrets only from files, environment variables or a prompt — never from arguments.
- Links can stay off the command line: `get -` and `delete -` read the link from stdin,
  `--link-file` from a file, and `put --delete-link-file` writes the management link to a new
  owner-only file instead of stderr.
- `CAPSULEBIN_PASSWORD` is used by `get` only for links to the server named with `--server` (or
  `CAPSULEBIN_SERVER`).
- Plain `http://` only to `localhost`, `127.0.0.0/8` and `[::1]`, elsewhere only with `--allow-http`
  or `CAPSULEBIN_ALLOW_HTTP=1`.
- Stable exit codes, `--json` output, safe retries and confirmation before consuming a burn capsule.
- `get` writes capsule content only to files, never to stdout: a single item to the current
  directory, several items to a new directory `capsule-<date>-<id prefix>` (`--here`: the current
  directory, `-o DIR`: `DIR`).
- Decrypted files are owner-only (`0600`, new directories `0700`) from the first byte and created
  exclusively: an existing file, directory or symlink at the name is never overwritten or followed,
  the item gets a free name (`name (1).ext`) instead; a failed write removes the files
  this run created and the directory it created for several items.
- Control characters from a capsule or a server do not reach the terminal: diagnostics on stderr
  show them as escapes, and `--json` writes DEL and C1 as `\u00NN`.
- The hidden password prompt and the burn confirmation work on a real terminal and restore it;
  Ctrl-C and EOF end with exit code `8` before the request the prompt guards (the retrieve, or
  the upload for `put`).

[Unreleased]: https://github.com/capsulebin/capsulebin/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/capsulebin/capsulebin/releases/tag/v1.0.0
