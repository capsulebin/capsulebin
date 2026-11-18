# ADR-005 — Design tokens under a CSP without inline styles

**Status:** Accepted

## Context

The Content Security Policy constrains the visual implementation harder than it looks at first
reading:

```
default-src 'none'; script-src 'self'; style-src 'self'; ...
require-trusted-types-for 'script'
```

`style-src 'self'` without `'unsafe-inline'` forbids **every** `style` attribute and every `<style>`
block. `font-src 'self'` together with the ban on third party resources rules out font CDNs.

## Decision

**All colours and measures as CSS custom properties** in `src/server/web/src/styles/tokens.css`. The accent is
teal; in the dark theme it is lightened, because `#0f766e` drops below 3:1 on a dark ground.

**Theme resolution written three times**, so that a manual toggle can override the system
preference in *both* directions:

```css
:root { /* light */ }
@media (prefers-color-scheme: dark) { :root { /* dark */ } }
:root[data-theme="light"] { /* light, higher specificity */ }
:root[data-theme="dark"]  { /* dark,  higher specificity */ }
```

**System fonts only.** A self-hosted face would be CSP-compliant but costs bytes in the binary and
introduces a flash of unstyled text. For an application whose content is largely monospace the gain
is small.

**Two border tokens.** Checking the contrast ratios showed that a single `--cb-border` cannot serve
both purposes: as a divider it should recede (1.27:1 against the background), but as the outline of
an input field WCAG 1.4.11 asks for 3:1. Hence `--cb-border-strong` (`#7d8e8c` / `#587471`)
exclusively for controls. A globally darker border would have made the interface needlessly heavy.

**No CSS-in-JS, no `el.style.*`.** State travels through classes and `data-*` attributes.

**An external classic script for theme initialisation.** The usual anti-flash snippet in `<head>`
is inline and therefore forbidden. A module script would run too late (`type="module"` is always
deferred). The solution is `src/server/web/public/theme-init.js`: external, classic, without `defer`/`async` —
it blocks the parser and is guaranteed to run before the first paint, and being self-hosted it is
covered by `script-src 'self'`.

## Rejected

- **A web font from a CDN.** Blocked by the CSP. The `<link>` would fail silently and the page
  would ship in a fallback face — worse than choosing system fonts from the start.
- **Theme via `prefers-color-scheme` only.** Cheaper, but takes the choice away from users whose
  system setting does not match the situation.
- **`[data-theme]` only, without the media query.** The first visit would have no theme and would
  always start light.
- **Using `localStorage` for anything beyond the theme choice.** Persistence is a side channel. Only
  `light|dark` is stored; for `auto` the entry is removed. A test asserts that after using the
  toggle exactly one key exists in `localStorage`.

## Consequences

- Contrast is verifiable and is verified: `src/server/web/tests/contrast.test.ts` computes every token pair
  against `--cb-bg`, `--cb-surface` and `--cb-surface-sunken`. The weakest pair is `--cb-warning` on
  `--cb-surface-sunken` at 4.58:1.
- The storage key is duplicated between `public/theme-init.js` and `src/theme.ts`. Importing it
  would turn the file into a module and destroy the very property it exists for.
  `src/server/web/tests/theme-init.test.ts` keeps both places in sync.
- A test checks the HTML shells for inline scripts, `style` attributes and external references, so a
  CSP violation surfaces during the test run rather than in the browser.
- When adding a component, use `--cb-border-strong` for controls and `--cb-border` for dividers.
  Confusing the two is an accessibility bug, not a cosmetic one.
