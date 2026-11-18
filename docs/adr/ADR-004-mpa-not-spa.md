# ADR-004 — Multi-page application instead of an SPA, Lit for stateful views

**Status:** Accepted

## Context

The web client is TypeScript; Vite serves only as a dev server and bundler. Its structure decides
the server routes and the asset layout, and it has to fit the CSP (ADR-005): no inline styles, no
`eval`, Trusted Types for every HTML sink.

The four views have very different shapes. Create, manage and report are mostly forms wired up
once. The recipient view moves through a small state machine (loading → password prompt → KDF
progress → decrypting → items / burned / unavailable), switches between items, toggles Markdown
preview and source, and manages object URLs for downloads.

## Decision

**The client is a multi-page application:** four HTML documents, each with its own entry point.

| Route | Document | Purpose |
| --- | --- | --- |
| `GET /` | `index.html` | Create |
| `GET /p/{id}` | `p.html` | Open |
| `GET /manage/{id}` | `manage.html` | Delete capability |
| `GET /report` | `report.html` | Report |

Vite builds this through `rollupOptions.input` with four entries. Shared code (`shell.ts`,
`theme.ts`, styles) automatically ends up in a common chunk.

**The server does not inspect the path id.** `GET /p/{id}` returns byte-identical HTML for every
syntactically valid id. This is a security property, not convenience: a link scanner cannot infer
from status code, length or content whether the capsule exists, and it does not burn a one-time
paste. Retrieval is an explicit `POST` carrying the capabilities derived from the fragment
(ADR-001). An integration test (`view_shell_is_identical_for_every_id`) and the acceptance script
both assert the byte equality.

**[Lit](https://lit.dev/) (`lit` 3) renders views with non-trivial state** — today the recipient
view (`pages/view.ts`, `pages/render-items.ts`). Within these constraints:

- **Still an MPA.** Lit renders inside one page; there is no client-side router and no shared state
  across documents.
- **Light DOM only.** Components return `this` from `createRenderRoot()`. No shadow roots, so the
  global design tokens and stylesheets (ADR-005) apply unchanged and Lit's `adoptedStyleSheets` /
  injected-`<style>` path is never used. Components define no `static styles`.
- **CSP stays as is.** No `'unsafe-inline'`, no `'unsafe-eval'`; Lit's template engine needs
  neither.
- **Trusted Types.** Lit creates its own `lit-html` policy for its internal `innerHTML` writes.
  `unsafeHTML` is allowed only for output that already went through the project's own pipeline —
  DOMPurify-sanitized Markdown (`render-markdown.ts`) and highlight.js output (`render-code.ts`).
  Decrypted text is otherwise interpolated as a plain value and therefore escaped.
- **No further framework layer.** No Lit plugins for routing, context or state management; anything
  beyond `lit` and its bundled directives needs a new ADR.

Pages without meaningful state may stay plain DOM code.

## Rejected

- **An SPA with a history router.** Would require server-side rewriting of every path onto one
  document plus a routing layer in the client. For four views without shared state that is pure
  overhead — and each route here carries a different secret in its fragment, so they specifically
  do *not* share state.
- **One document with hash routing.** The fragment already carries the link secret. Using it for
  routing as well would put a collision at the most sensitive point of the application.
- **Server-side rendering of the view.** Impossible: the server does not have the key.
- **Hand-written DOM updates for the stateful views.** Keeps the dependency list shorter, but every
  phase transition needs its own partial re-render; declarative templates re-render from state and
  keep the phase machine readable.
- **React/Preact/Vue/Svelte.** Larger runtime or a compiler step, virtual DOM or JSX — all more than
  a handful of templates need. Lit is ~6 KB, uses standard tagged templates and custom elements,
  and needs no build transform beyond what Vite already does.
- **Shadow DOM components.** Would cut the components off from the global design tokens and pull
  in Lit's stylesheet injection, which conflicts with the no-inline-styles CSP on older browsers.

## Consequences

- Switching between views is a real page load. With four views and no shared state that costs
  nothing.
- A new route needs an entry in three places: `vite.config.ts`, the server route, and `Shell` in
  `api/assets.rs`. The unit test `every_route_maps_to_one_shell` pins the mapping down.
- Each document loads its own entry bundle, which keeps the shells small — the create page loads no
  Markdown renderer, the view page no editor.
- `lit` is a runtime dependency of `@capsulebin/web` and part of the dependency review (`npm audit`)
  like DOMPurify and marked.
- `unsafeHTML` call sites are security-relevant and are reviewed like `innerHTML` sinks: each one
  must take sanitized or highlighter-escaped input only.
