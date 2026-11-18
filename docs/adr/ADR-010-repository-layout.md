# ADR-010 — Repository layout: `src/{server,cli,core}`

**Status:** Accepted

## Context

The repository holds one Rust crate and three TypeScript packages, next to non-source directories
(`migrations/`, `docs/`, `scripts/`). Grouping the source by ecosystem (`crates/` for Rust,
`packages/` for TypeScript) would say nothing about what each part is, and would put compiled code
and reference material at the same level without a boundary.

`@capsulebin/web` is not a standalone consumer of the HTTP API the way `@capsulebin/cli` is. It has
no independent release, is never published, and its only consumer is `capsulebin-server`, which
embeds its build output into the binary at compile time via `rust-embed`
(`src/server/src/api/assets.rs`). Only when `server.asset_dir` is set, which is meant for
development, does the server read the files from disk instead.
The CLI, by contrast, is a standalone client that talks to any CapsuleBin server over the network.

## Decision

**All source lives in `src/`, grouped by role, with the web client nested inside the server crate
it is embedded into:**

```
src/
├── server/     Rust crate `capsulebin-server`
│   └── web/    @capsulebin/web — embedded into the server binary at compile time
├── cli/        @capsulebin/cli — standalone command line client
└── core/       @capsulebin/core — shared capsule format, crypto, API client
```

`migrations/`, `docs/` and `scripts/` live at the repository root; this ADR only concerns the
directories that hold compiled source.

## Rejected

- **Grouping by ecosystem (`crates/`, `packages/`).** Neither name signals "source code", and the
  web client would look like a peer of the CLI although it only ever ships inside the server.
- **`src/server/backend/` and `src/server/web/` as siblings under a `server/` container.**
  Adds a directory level and a filler name (`backend`) without benefit: the Rust crate
  manifest already lives at the level that names it a server, and nothing else needs to
  live alongside `backend/` inside `server/`.
- **`src/web/`, `src/cli/`, `src/core/`, `src/server/` as four flat siblings.** Treats the
  web client as a peer of the CLI, which the actual coupling (embedded into the server
  binary, no independent artifact, no non-dev consumer other than the server) does not
  support.
- **No `src/` at all, roles directly at the repository root.** Shortest paths and avoids
  the doubled `src/server/src/` for the Rust crate, but leaves no boundary between source and
  non-source top-level directories.

## Consequences

- The Rust crate's paths are short: `build.rs` and `#[folder = ...]` in `assets.rs` refer to
  `web/dist`, and `sqlx::migrate!("../../migrations")` in `db.rs` reaches the root.
- The npm workspace list is explicit — `["src/core", "src/cli", "src/server/web"]` — because
  `src/server` is a directory `npm` would otherwise try to treat as a workspace member.
- Scripts and CI refer to the JS workspaces by package name (`-w @capsulebin/core`,
  `-w @capsulebin/web`) rather than by path, so a layout change does not ripple into them.
- `.gitignore` needs an explicit `/src/server/web/{node_modules,dist}` entry alongside the
  general `/src/*/node_modules`, since `web/` is nested one level deeper than `cli/` and
  `core/`.
