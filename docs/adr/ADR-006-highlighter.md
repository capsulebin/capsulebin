# ADR-006 — highlight.js instead of Shiki for syntax highlighting

**Status:** Accepted

## Context

Code is displayed as highlighted text and never executed. Highlighting is meant to be curated, with
the language chosen explicitly rather than guessed.

The obvious modern choice would be Shiki: the same grammar engine as VS Code, very accurate
results. Shiki, however, emits its colours as **inline styles** (`<span style="color:#...">`). Under
`style-src 'self'` without `'unsafe-inline'` the browser discards those — the code would appear
uncoloured, and silently so.

## Decision

**highlight.js with explicit language registration.** Class-based output (`.hljs-keyword`,
`.hljs-string`, …) coloured through our own CSS.

- **No bundled hljs theme.** `src/server/web/src/styles/code.css` maps the `.hljs-*` classes onto the design
  tokens from ADR-005. Code and interface therefore draw from one colour source and both themes stay
  consistent without a second stylesheet.
- **No `highlightAuto`.** Languages are registered individually; the allowlist is `plaintext`,
  `bash`, `json`, `yaml`, `ini` (also used for TOML), `rust`, `typescript`, `javascript`, `python`,
  `sql`, `html`, `css`, `diff` and `markdown`. That matches the requirement that language selection is explicit, and it bounds the
  bundle size — the complete highlight.js language set would be several times larger.
- **Highlighting operates on `textContent` only,** never on an HTML string, so source code can never
  become markup.
- Unknown or unlisted languages fall back to `plaintext`.

## Rejected

- **Shiki.** Emits inline styles and is therefore incompatible with the CSP. A nonce or hash for
  styles would be theoretically possible but would mean dynamically generated CSS per response,
  which is impractical with statically embedded assets and would weaken the boundary for the wrong
  reason.
- **Prism.** Also class-based and would be workable. highlight.js wins on the simpler explicit
  language registration; Prism resolves language dependencies through plugins, which cuts against
  the allowlist idea.
- **Server-side highlighting.** Impossible: the server never sees the code.
- **No highlighting at all.** Simplest from a CSP standpoint, but it removes one of the two core
  benefits of the product.

## Consequences

- Adding a language requires a deliberate change to the allowlist and grows the bundle. That is
  intended: the list should stay curated.
- `code.css` has to be updated when a token changes. The coupling is deliberate — the alternative
  would be a second, independent colour set.
- Should the library be replaced later, the CSP is the deciding criterion, not grammar accuracy.
  This ADR is where that can be looked up.
