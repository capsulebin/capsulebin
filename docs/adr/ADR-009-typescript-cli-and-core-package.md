# ADR-009 — A single TypeScript implementation, shared by the web client and the CLI

**Status:** Accepted

## Context

Two clients need to produce and read the same capsule format: the browser (TypeScript, Web
Crypto) and the command line client. Two independently written implementations of the same
encoding, key derivation and HTTP client are not a meaningful safeguard against drift — they are
the same logic typed out twice, in different languages, with the ordinary maintenance cost of a
duplicate.

The capsule format, the key derivation and the API client need no `window`, `document`, `File` or
any other browser-only global — only `crypto.subtle`, `crypto.getRandomValues` and `fetch`, all of
which are native globals in Node 24 as well. There is no technical reason the CLI needs a different
language.

## Decision

**One reference implementation, `@capsulebin/core`, shared by both clients:**

```
src/core         @capsulebin/core — capsule format, crypto, API client. No DOM, no Node built-ins.
src/server/web   @capsulebin/web  — the browser client. Depends on @capsulebin/core.
src/cli          @capsulebin/cli  — the command line client. Depends on @capsulebin/core.
```

`@capsulebin/core` takes and returns `Uint8Array`, never a browser `File` or a Node `Buffer`
directly, so the same code runs unmodified under Vite (web) and esbuild (CLI). Its own
`tsconfig.json` omits the `DOM` lib on purpose — that absence is the platform-neutral boundary; a
change that only compiles because a DOM global leaked in is a change that would break the CLI at
runtime, not just in principle.

**No Rust protocol crate.** `capsulebin-server` needs only two things that a shared protocol
crate could provide — `PasteId` and `Verifier` (capability checking) — and neither depends on
the capsule format or crypto primitives. They live in `capsulebin-server`'s own `domain` module,
reachable as `crate::domain::{PasteId, Verifier}` inside the library crate and
`capsulebin_server::domain::*` from the binary and integration tests. The capsule format and crypto
profile are specified in `docs/capsule-file-format.md` and `docs/crypto-profile.md`, not in an
executable Rust crate — `docs/test-vectors/` is the shared fixture, and `@capsulebin/core` is the
reference implementation that must match the specification (ADR-003).

**The CLI ships as one bundled JavaScript file** (`dist/cli.js`, built by esbuild, with
`@capsulebin/core` inlined) that runs wherever Node 24 runs; there is no per-platform compiled
binary.

## Rejected

- **A Rust CLI sharing a Rust protocol crate with the server.** This is the duplicate-implementation
  problem described above: two implementations of the same format would exist — the actual web
  client, and a Rust CLI re-deriving the same format from ADR-008's specification — and every format
  change would need to land correctly in both a Rust crate and a TypeScript module rather than once.
- **Compiling `@capsulebin/core` to WebAssembly.** Instantiating WebAssembly requires
  `'wasm-unsafe-eval'` in `script-src`, which the CSP does not grant (ADR-005); the browser's native,
  hardware-accelerated `crypto.subtle` would be given up for a software AES-GCM and PBKDF2 running
  inside the sandbox; and a readable JavaScript bundle is auditable in a way an opaque WebAssembly
  blob is not (ADR-001's admission that the web client trusts the code served to it is a weak point
  that should not be made weaker).
- **Compiling `@capsulebin/core` to a native Node addon.** Unnecessary: Web Crypto and `fetch` are
  already native in Node 24, so there is nothing to gain by leaving JavaScript, and it would tie the
  CLI to a compiled artifact per platform for no benefit.
- **Publishing the CLI as a compiled single binary per platform.** One JavaScript bundle is
  simpler to build, ship and keep current, and this project has no other reason to maintain
  per-platform release artifacts.
- **A Rust protocol crate "for when a second Rust component needs it."** The server stores
  `public_header` and `ciphertext` as opaque `Vec<u8>` (ADR-001) and never parses either, so key
  derivation, header parsing and the format constants would have no Rust caller. Dead code that
  compiles is still a maintenance and audit cost.
- **Leaving the capsule format specified only inside ADR-008's prose**, with no separate
  implementation-facing document. ADR-008 explains *why* the format looks the way it does; a
  third-party implementer needs the precise byte layout and validation rules without wading through
  rationale, which is what `docs/capsule-file-format.md` is for.

## Consequences

- There is exactly one reference implementation of the format, `@capsulebin/core`, used by both
  shipped clients. Confidence that a third, independent implementation (in another language) can
  still interoperate rests entirely on `docs/capsule-file-format.md`, `docs/crypto-profile.md` and
  `docs/test-vectors/` being precise and complete.
- The CLI (`put`, `get`, `delete`, with password, burn and expiry options) and the web client
  produce and read capsules through the same code, so a capsule created with one opens with the
  other by construction.
- A future contributor adding a capsule-format feature touches `@capsulebin/core` once; both
  `src/server/web` and `src/cli` pick it up through the workspace dependency without a
  synchronization step.
- The Rust workspace is just `capsulebin-server`. `PasteId` and `Verifier` are internal to its
  `domain` module rather than behind a crate boundary — which matches what they actually are:
  server-internal types with no other consumer. A shared Rust crate is warranted only if a second
  Rust component appears that needs them, not preemptively.
