# ADR-012 — No hand-maintained OpenAPI document

**Status:** Accepted

## Context

An API this small is often accompanied by an OpenAPI document. Here nothing would generate code
from one, nothing would validate server responses against it, and it would be a third hand-written
copy of shapes already defined in `src/server/src/api/pastes/wire.rs` (the source) and
`src/core/src/api.ts` (its client mirror).

## Decision

CapsuleBin does not ship a hand-maintained OpenAPI document.

- The interoperability contract for independent clients is the capsule file format, the crypto
  profile and the golden vectors (`docs/capsule-file-format.md`, `docs/crypto-profile.md`,
  `docs/test-vectors/`). They are what a second implementation has to get right, and they are
  enforced by tests.
- The HTTP surface is four paste endpoints and the two report endpoints plus health checks,
  documented in the README with its error model (`problem+json`, stable `code`, identical answers
  for every unavailable case, replay semantics from ADR-011).

## Rejected

- **A hand-written `openapi.yaml`.** Without a consumer or a check it drifts silently.
- **A hand-written document plus a contract test.** Workable, but it maintains a third copy of the
  wire types for no current consumer.

## Consequences

- If a real consumer appears (third-party clients, SDK generation, provisioning tooling), the
  document is **generated from the Rust wire types** (e.g. with `utoipa`) and served
  or exported by the binary, so it cannot diverge from the server. That is a new decision with its
  own ADR.
