# ADR-017 — Operator branding through configuration

**Status:** Accepted

## Context

Anyone may run CapsuleBin as their own instance (ADR-014), and operators want to present it under
their own identity: their name instead of "CapsuleBin", their logo, their favicon. Several things
constrain how:

- **The web build is reproducible and the same for every instance** (ADR-005). Nothing
  instance-specific may end up in `dist/`, and an operator must not need to rebuild.
- **Code comes only from the binary** (ADR-001, ADR-015). The server may put data into a page —
  the language prefix, the legal texts — but no script, stylesheet or shell of the operator's.
- **The CSP is strict** (`img-src 'self' data:`, `style-src 'self'`, no inline styles), and the
  security-header layer in `router()` is security-critical. Images have to be same-origin files.
- **AGPL-3.0 §13**: users of a network service must be offered the source of the version they use.
  The licence covers the code, not the name "CapsuleBin" or the logo (`docs/brand/README.md`).

## Decision

**A `[branding]` table configures name, logo, dark logo, favicon and the source link**; the server
reads it at startup and fills it into the shells like the language prefix and the legal links.
[branding.md](../branding.md) has the rules in detail.

- **Named keys only**: `name`, `hide_name`, `logo`, `logo_dark`, `favicon`, `source_url`, every one
  validated at startup; an unknown key is a startup error. Without the table an instance looks as
  before, plus the attribution link and a built-in favicon (`/favicon.svg` from the web build).
- **One name for every language.** It replaces "CapsuleBin" in the `<title>`, the header and the
  interface texts that name the service, and nowhere else: not in the API, the CLI, the logs.
- **Raster images only**: logos PNG or WebP, favicon PNG or ICO, checked by extension and file
  signature, at most 256 KiB each. Read once at startup and served from memory at
  `/branding/<16 hex digits of SHA-256>.<ext>` with immutable caching; any other `/branding/`
  path is the generic `404`.
- **Operator values are data**: HTML-escaped and filled in with every other server token in a
  single pass, so a value is never read as a token or marker of the shell. The build writes
  `%%capsulebin:name%%`, `%%capsulebin:favicon%%`, `%%capsulebin:source%%` and two comment-marker
  regions (logo, wordmark) into the shells.
- **A dark logo follows the theme toggle**: CSS on `data-theme` mirroring the three theme blocks of
  ADR-005, not `<picture media>`.
- **Every page carries "Powered by CapsuleBin"** (de: "Betrieben mit CapsuleBin") as the last
  footer link, also on the legal pages. No key removes it, and its text keeps the project's name
  whatever the instance is called. Its target is `source_url`, by default the project repository; an
  operator of a modified version points it to their source. It must be an absolute `http(s)` URL.

## Rejected

- **An operator stylesheet or template overrides.** Every class name and every piece of markup
  would become an unannounced API that an upgrade can break, and nothing could be checked
  (contrast, layout). Named configuration keys keep the markup internal.
- **SVG logos and favicons from the operator.** An SVG is an active document; serving one safely
  needs a per-response sandbox CSP — a change to the security-header layer — and sanitising. The
  cost of refusing it is lossless scaling; a PNG at twice the display height is enough for a header.
- **A name per language.** The name is an identity, not interface text.
- **A switch for the attribution.** It is the instance's pointer to its source (§13) on every
  page; an operator can redirect it with `source_url`, not remove it.
- **Files served under their own name, or re-read per request.** A content-derived URL can be
  cached for good and still changes when the file does, and it reveals nothing about the
  operator's paths; the shells are `no-store`, so a restart shows a new file at once.

## Consequences

- The server now puts operator images and an operator name into every page. Both are data: images
  are same-origin raster files, the name is escaped text. The CSP and the header layer are unchanged.
- A changed file or name takes effect after a restart, like every other setting.
- `capsulebin check` reports a missing, oversized or wrongly formatted file.
- The shells contain no literal "CapsuleBin" any more; the Vite dev server fills the defaults.
- Every page has one outgoing link (the source link); the smoke test allows exactly that one.
- Theme colours are not part of this decision; a later extension of `[branding]` will amend it.
