# ADR-017 — Operator branding through configuration

**Status:** Accepted

## Context

Anyone may run CapsuleBin as their own instance (ADR-014), and operators want to present it under
their own identity: their name instead of "CapsuleBin", their logo, their favicon, their colours.
Several things constrain how:

- **The web build is reproducible and the same for every instance** (ADR-005). Nothing
  instance-specific may end up in `dist/`, and an operator must not need to rebuild.
- **Code comes only from the binary** (ADR-001, ADR-015). The server may put data into a page —
  the language prefix, the legal texts — but no script, stylesheet or shell of the operator's.
- **The CSP is strict** (`img-src 'self' data:`, `style-src 'self'`, no inline styles), and the
  security-header layer in `router()` is security-critical. Images have to be same-origin files.
- **AGPL-3.0 §13**: users of a network service must be offered the source of the version they use.
  The licence covers the code, not the name "CapsuleBin" or the logo (`docs/brand/README.md`).

## Decision

**A `[branding]` table configures name, tagline, logo, dark logo, favicon, the source link and
the theme colours**; the server reads it at startup and fills it into the shells like the language
prefix and the legal links. [branding.md](../branding.md) has the rules in detail.

- **Named keys only**: `name`, `hide_name`, `tagline`, `logo`, `logo_dark`, `favicon`, `source_url`,
  `[branding.colors.light]`/`[branding.colors.dark]` with the colour tokens of the theme, and
  `[branding.<lang>]` with only `tagline` for a known language other than the default one; every one
  validated at startup, an unknown key or table is a startup error. Without the table an instance
  shows the built-in identity, plus the attribution link and a built-in favicon (`/favicon.svg` from
  the web build).
- **One name for every language.** It replaces "CapsuleBin" in the `<title>`, the header and the
  interface texts that name the service, and nowhere else: not in the API, the CLI, the logs.
- **A tagline per language.** The slogan next to the logo on the create page is interface text:
  `tagline` replaces the built-in translated slogan in every language, `[branding.<lang>] tagline`
  in one language (as `[legal.<lang>]` does for the legal texts); an empty value removes the
  slogan. At most 80 characters. The manage, report and legal pages keep their page title there.
- **Raster images only**: logos PNG or WebP, favicon PNG or ICO, checked by extension and file
  signature, at most 256 KiB each. Read once at startup and served from memory at
  `/branding/<16 hex digits of SHA-256>.<ext>` with immutable caching; any other `/branding/`
  path is the generic `404`.
- **Operator values are data**: HTML-escaped and filled in with every other server token in a
  single pass, so a value is never read as a token or marker of the shell. The build writes
  `%%capsulebin:name%%`, `%%capsulebin:favicon%%`, `%%capsulebin:source%%` and three comment-marker
  regions (logo, wordmark, and the tagline on the create page) into the shells; a configured
  tagline goes in through a server-internal token in the same pass.
- **A dark logo follows the theme toggle**: CSS on `data-theme` mirroring the three theme blocks of
  ADR-005, not `<picture media>`.
- **Theme colours as token values, written into a generated stylesheet.** Each theme table sets any
  of the 18 colour tokens of `tokens.css` (key `x_y` for `--cb-x-y`; `--cb-shadow-card` and
  non-colour tokens stay built-in); a token left out keeps its built-in value, per theme. Values are
  `#rgb` or `#rrggbb` only, so they can be checked exactly and never leave a CSS declaration. The
  server generates the stylesheet itself from the configured tokens, serves it like the images at
  `/branding/<16 hex>.css`, and links it last in the head of every page, so it overrides the
  bundled theme. Its light block sits in the exact complement of the dark media query and the
  `data-theme` blocks follow, so the system preference and the toggle keep working both ways. No
  colours: no stylesheet, no link, unchanged pages.
- **Contrast below the built-in theme's thresholds is a warning, not an error.** The merged palette
  of each theme is checked against the pairs the built-in theme is held to (WCAG 4.5:1 for text,
  3:1 for control outlines and the focus ring; one shared fixture with the web contrast test).
  `serve` logs each failing pair at startup, `check` prints it; neither refuses to start.
- **Every page carries "Powered by CapsuleBin"** (de: "Betrieben mit CapsuleBin") as the last
  footer item, also on the legal pages. No key removes it, and its text keeps the project's name
  whatever the instance is called. Only "CapsuleBin" is the link, and it opens in a new tab. Its
  target is `source_url`, by default the project repository; an operator of a modified version
  points it to their source. It must be an absolute `http(s)` URL.

## Rejected

- **An operator stylesheet or template overrides.** Every class name and every piece of markup
  would become an unannounced API that an upgrade can break, and nothing could be checked
  (contrast, layout). Named configuration keys keep the markup internal.
- **SVG logos and favicons from the operator.** An SVG is an active document; serving one safely
  needs a per-response sandbox CSP — a change to the security-header layer — and sanitising. The
  cost of refusing it is lossless scaling; a PNG at twice the display height is enough for a header.
- **A name per language.** The name is an identity, not interface text (unlike the tagline).
- **A switch for the attribution.** It is the instance's pointer to its source (§13) on every
  page; an operator can redirect it with `source_url`, not remove it.
- **Colours in any CSS syntax (`rgb()`, `hsl()`, names, alpha).** They would need a CSS parser to
  check, and the contrast of a translucent colour depends on what lies below it.
- **A contrast failure as a startup error.** An operator's corporate colours may miss a threshold
  narrowly; refusing to start would be out of proportion to a warning that makes it visible.
- **Colour tables directly as `[branding.light]`/`[branding.dark]`.** They would share the
  namespace of the per-language tables `[branding.<lang>]`; `[branding.colors]` keeps them apart.
- **A shell marker for the stylesheet link.** The web build appends its own stylesheet link at the
  end of the head, after anything in the template; the server inserts the link before `</head>`
  instead, without template changes.
- **Files served under their own name, or re-read per request.** A content-derived URL can be
  cached for good and still changes when the file does, and it reveals nothing about the
  operator's paths; the shells are `no-store`, so a restart shows a new file at once.

## Consequences

- The server puts operator images, an operator name and tagline, and operator colours into the
  pages. All are data: images are same-origin raster files, name and tagline are escaped text, the
  colours are `#rrggbb` values in a stylesheet the server writes itself (same-origin, allowed by
  `style-src 'self'`). The CSP and the header layer need no change.
- A changed file, name or colour takes effect after a restart, like every other setting.
- `capsulebin check` reports a missing, oversized or wrongly formatted file and a refused colour,
  and prints contrast warnings.
- The built-in palette exists twice (`tokens.css` and the server's copy for the contrast check),
  kept equal by a test; the contrast pairs exist once, in a fixture both tests read.
- The Vite dev server and the built-in `/favicon.svg` show the built-in colours only.
- The shells contain no literal "CapsuleBin"; the Vite dev server fills the defaults.
- Every page has one outgoing link (the source link); the smoke test allows exactly that one.
