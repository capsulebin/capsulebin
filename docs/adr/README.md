# Architecture decisions

Each file records one decision, its context and its consequences. The value sits in the
*Rejected* and *Consequences* sections: they capture what is no longer possible, so that a later
change is recognised as a change rather than happening by accident.

| ADR | Title | Status |
| --- | --- | --- |
| [001](ADR-001-trust-model.md) | Trust model: zero knowledge for stored content | Accepted |
| [002](ADR-002-sqlite.md) | Embedded SQLite as the only mandatory persistence | Accepted |
| [003](ADR-003-protocol-versioning.md) | Independent version numbers, golden vectors as the conformance contract | Accepted |
| [004](ADR-004-mpa-not-spa.md) | Multi-page application instead of an SPA, Lit for stateful views | Accepted |
| [005](ADR-005-theme-and-csp.md) | Design tokens under a CSP without inline styles | Accepted |
| [006](ADR-006-highlighter.md) | highlight.js instead of Shiki for syntax highlighting | Accepted |
| [007](ADR-007-error-handling.md) | Typed errors in libraries, `anyhow` at the binary boundary | Accepted |
| [008](ADR-008-capsule-file-format.md) | A dedicated binary container instead of a general-purpose serialization | Accepted |
| [009](ADR-009-typescript-cli-and-core-package.md) | A single TypeScript implementation, shared by the web client and the CLI | Accepted |
| [010](ADR-010-repository-layout.md) | Repository layout: `src/{server,cli,core}` | Accepted |
| [011](ADR-011-idempotent-create.md) | The client-generated id is the idempotency key | Accepted |
| [012](ADR-012-no-openapi-document.md) | No hand-maintained OpenAPI document | Accepted |
| [013](ADR-013-abuse-reports-and-moderation.md) | Abuse reports and moderation without decryption | Accepted |
| [014](ADR-014-agpl-licence-and-contributions.md) | AGPL licence, DCO for contributions | Accepted |
| [015](ADR-015-legal-pages-rendered-by-the-server.md) | Legal pages: operator text rendered by the server | Accepted |
