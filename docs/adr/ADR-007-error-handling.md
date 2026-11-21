# ADR-007 — Typed errors in libraries, `anyhow` at the binary boundary

**Status:** Accepted

## Context

Errors in this project cross two very different boundaries.

At the **HTTP boundary** every failure becomes a `problem+json` response with a stable machine
readable code and an HTTP status. The caller of a domain function therefore has to be able to tell
failures apart: a size limit violation is a 413, an unknown id and an expired capsule are both a
generic unavailability, an invalid capability must not confirm that the id exists at all. That
decision cannot be made from an error string.

At the **process boundary** — startup, migrations, the operational subcommands — nobody matches on
anything. What helps there is a readable chain: which config file, which path, which underlying
cause.

There is also a constraint specific to this project: **error messages are part of the security
boundary.** They must never contain plaintext, key material, capability values, full paste ids or
cryptographic detail, because they end up in logs and in responses.

## Decision

**Typed errors with `thiserror` in every layer whose caller has to react to the difference.** That
is the server's `domain`, `repository` and `api` modules. Each variant carries the data needed for
the mapping and nothing else; the HTTP layer owns a single, exhaustive `match` from error variant to
status and problem code.

**`anyhow` only in the binary** (`main.rs` and `commands/`), for startup and the operational
subcommands, where `Context` builds
a useful chain and no caller inspects the result.

**Error messages are reviewed as security-relevant text.** Fixed strings, no interpolated payload.
A wrong password and damaged ciphertext collapse into one generic failure, so that the two are
indistinguishable — otherwise the error itself becomes an oracle. Where a value genuinely helps
debugging, it is logged as a shortened or hashed form, never in full.

**`unwrap`, `expect` and `panic` are lint warnings in production code** (see `Cargo.toml`), because
a panic inside a handler bypasses the mapping entirely and can surface a message nobody reviewed.
Tests opt out; they should fail loudly.

## Rejected

- **`anyhow` everywhere.** Tempting because it is less code. But the HTTP layer would have to
  string-match to pick a status, and the no-leak rule would become unenforceable: `anyhow`'s natural
  idiom is `.context(format!(...))`, and a formatted context string is exactly where a payload or a
  full id accidentally ends up. The convenience is paid for at the boundary that matters most.
- **`thiserror` everywhere, including `main.rs`.** An enum with one variant per startup failure is
  boilerplate with no consumer, and it loses the cause chain that makes a misconfiguration
  diagnosable.
- **`Box<dyn Error>`.** Gives up both matching and structured context, which are the two things
  actually needed here.

## Consequences

- `thiserror` is a workspace dependency, used in `domain`, `repository`, `config`, `db`, `backup`,
  `healthcheck` and `api::problem`.
  An unused error variant is not caught by the existing audit gate, so pruning dead variants when a
  code path is removed is a review responsibility.
- The exhaustive `match` from error variant to problem code is deliberately not defaulted. Adding a
  variant should fail to compile until someone decides its status, code and whether it may reveal
  existence.
- `tests/logging.rs` asserts that no log line contains a capability value, a token or body bytes,
  across a full request cycle including the failing variants. That test is the
  enforceable form of the rule above; the prose here only explains why it exists.
