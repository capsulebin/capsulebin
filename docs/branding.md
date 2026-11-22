# Operator branding

An operator can present a CapsuleBin instance under their own identity without a rebuild. The
`[branding]` table sets the name shown in place of "CapsuleBin" in the web pages, the slogan next to
the logo on the create page (per language if needed, or none), a logo that replaces the built-in
symbol in the header (optionally a second one for the dark theme), a favicon, and the colours of the
light and the dark theme (`[branding.colors]`). The server reads the image files once at startup,
generates a small stylesheet from the colours, serves both same-origin under content-derived URLs
and fills everything into the shells, as it does for the legal pages (ADR-015). The system
preference and the theme toggle keep working with the operator's colours; a colour pair below the
contrast the built-in theme holds is a warning, not an error. Every page of every instance, branded
or not, carries a fixed "Powered by CapsuleBin" attribution in the footer, in which "CapsuleBin"
links to the source code (AGPL-3.0 §13) in a new tab; only the link target is configurable. An
instance without `[branding]` shows the built-in name, symbol, slogan and colours, plus that
attribution and the built-in favicon. See [ADR-017](adr/ADR-017-operator-branding.md).

## Rules

### Configuration

`[branding]` takes exactly the keys below, the table `[branding.colors]` (see
[Theme colours](#theme-colours)) and one optional subtable `[branding.<lang>]` per known language
other than `i18n.default_language`, which takes exactly `tagline`. Any other key in `[branding]` or
in a subtable, a subtable for an unknown language and a subtable for the default language are
startup errors. Every key also as `CAPSULEBIN__BRANDING__<KEY>`, a language's tagline as
`CAPSULEBIN__BRANDING__<LANG>__TAGLINE` (e.g. `CAPSULEBIN__BRANDING__DE__TAGLINE`), a colour as
`CAPSULEBIN__BRANDING__COLORS__<THEME>__<KEY>`.

| Key | Default | Rule |
| --- | --- | --- |
| `name` | `"CapsuleBin"` | Trimmed; must not be empty or only whitespace, at most 64 characters (Unicode scalar values, after trimming), no control character. One name for every language. |
| `hide_name` | `false` | Show the logo without the name next to it. Requires `logo`. |
| `tagline` | none: the built-in, translated slogan (`create.claim`) | The slogan next to the logo on the create page, for every language without its own. Trimmed; at most 80 characters (after trimming), no control character. Empty (also only whitespace) removes the slogan. |
| `[branding.<lang>] tagline` | none: falls back to `tagline` | Same rules; replaces `tagline` in the pages of `<lang>`. Empty removes the slogan in that language only. |
| `logo` | none | Path to a PNG or WebP file (`.png`, `.webp`). Replaces the built-in symbol in the header. |
| `logo_dark` | none | Path to a PNG or WebP file, shown instead of `logo` in the dark theme. Requires `logo`. |
| `favicon` | none | Path to a PNG or ICO file (`.png`, `.ico`). Without it, the built-in `/favicon.svg`. |
| `source_url` | `"https://github.com/capsulebin/capsulebin"` | Target of the attribution link: an absolute `http`/`https` URL with a host, without user name or password, without whitespace or control characters. |

**Tagline per language**, highest first: the language's `[branding.<lang>] tagline`, then
`branding.tagline`, then the built-in slogan of the catalogue. The default language's tagline is
`branding.tagline`, which is why a subtable for it is refused.

Startup errors (also reported by `capsulebin check`), each naming the key:

- an unknown key of `[branding]` (`branding.<key> is unknown`) or of a subtable
  (`branding.<lang>: …`); a subtable for an unknown language or for the default language;
- the `name` rules above; `hide_name = true` without `logo` ("the header would be empty");
  `logo_dark` without `logo`; a `source_url` that breaks its rule; a tagline longer than 80
  characters or with a control character (`branding.tagline`, `branding.<lang>.tagline`);
- for each configured file, naming key and path: an extension outside the allowed set (case
  insensitive; an `.svg` gets the same message: "only PNG or WebP files (.png, .webp) are accepted"
  or "only PNG or ICO files (.png, .ico)"), a file that cannot be read, a file larger than 256 KiB,
  or content that does not start with the signature of its extension — PNG
  `89 50 4E 47 0D 0A 1A 0A`, WebP `RIFF` + 4 bytes + `WEBP`, ICO `00 00 01 00`. Messages never
  contain anything read from the file.

The checks run in this order: the non-file values first (`BrandingConfig::validate`), then the
files (`branding::resolve`): extension, read (at most 256 KiB + 1 byte is read), size, signature.

Relative paths are resolved against the working directory, like `database.path`. Files are read
once at startup; a replaced file takes effect after a restart.

### Theme colours

`[branding.colors.light]` and `[branding.colors.dark]` set colour tokens of the built-in theme
(`src/server/web/src/styles/tokens.css`) for one theme each. Both tables take the same 18 keys, all
optional; the key `x_y` sets the token `--cb-x-y`:

`bg`, `surface`, `surface_sunken`, `border`, `border_strong`, `text`, `text_muted`, `accent`,
`accent_hover`, `accent_text`, `danger`, `warning`, `success`, `focus_ring`, `tint_accent`,
`tint_danger`, `tint_warning`, `tint_success`.

- **Fallback per token and theme:** a key left out keeps its built-in value in that theme. A value
  set for one theme never reaches the other (a `danger` set only in `light` leaves the dark theme's
  built-in `danger`). Without any colour (also with empty tables) nothing changes: no stylesheet,
  no link, byte-identical pages.
- **Syntax:** a TOML string `#rgb` or `#rrggbb`, hex digits in either case, nothing around it — no
  whitespace (values are not trimmed), no alpha, no `rgb()`, `hsl()` or colour names. Normalised
  to lowercase `#rrggbb` (`#ABC` → `#aabbcc`).
- **Not configurable:** `--cb-shadow-card` (not a plain colour) and every non-colour token
  (fonts, radii, spacing). The built-in `/favicon.svg` keeps its fixed colours; for a matching
  favicon set `branding.favicon`.
- **Env:** `CAPSULEBIN__BRANDING__COLORS__<THEME>__<KEY>`, e.g.
  `CAPSULEBIN__BRANDING__COLORS__DARK__ACCENT_HOVER=#5eead4`. Keys are case-sensitive in the file
  (`BG` is unknown); env keys are lowercased by the overlay.

Startup errors, raised while the configuration is parsed (so also by `capsulebin check`), each
naming the full key: a key of `[branding.colors]` other than `light`/`dark`
(`branding.colors.dim is unknown`, `branding.colors.shadow_card is unknown`), `[branding.colors]`
or a theme that is not a table, an unknown key in a theme table
(`branding.colors.light.shadow_card is unknown`, followed by the accepted keys), and a value that
is not a string in the syntax above (`branding.colors.light.bg must be a colour as #rgb or
#rrggbb`).

**Contrast warnings.** Per theme, the merged palette (configured value, else built-in) is checked
against the pairs and thresholds of the built-in theme (WCAG 2.2 contrast ratio):

| Foreground | On | At least |
| --- | --- | --- |
| `text`, `text_muted`, `accent`, `danger`, `warning`, `success` | `bg`, `surface`, `surface_sunken` | 4.5:1 |
| `accent_text` | `accent` | 4.5:1 |
| `accent` | `tint_accent` | 4.5:1 |
| `danger` | `tint_danger` | 4.5:1 |
| `warning` | `tint_warning` | 4.5:1 |
| `success` | `tint_success` | 4.5:1 |
| `text` | `tint_accent`, `tint_danger`, `tint_warning`, `tint_success` | 4.5:1 |
| `border_strong` | `bg`, `surface`, `surface_sunken` | 3:1 |
| `focus_ring` | `bg` | 3:1 |

Each failing pair is one warning naming theme, both keys and the ratio with two decimals, e.g.
`light theme: branding.colors.light.text on branding.colors.light.bg has a contrast of 1.03:1,
below 4.5:1`. A warning names both keys even if only one of them is configured (the other is the
built-in value). The built-in palette gives none. Warnings never stop anything: `serve` logs each
once at `WARN` at startup; `check` prints each as `warning: …` to stderr, still prints its `ok:`
line to stdout and exits 0. The other subcommands load the same configuration silently. Tokens
outside the table (`border`, `accent_hover`) are not checked. A tint set as dark as its status
colour is warned about, e.g. a light `tint_warning = "#b45309"` gives
`light theme: branding.colors.light.warning on branding.colors.light.tint_warning has a contrast
of 1.00:1, below 4.5:1`.

### Serving the files

Each configured file is served at `/branding/<first 16 hex digits of its SHA-256>.<ext>` (`ext`
from the format: `png`, `webp`, `ico`), from memory, in both asset modes (embedded and
`server.asset_dir`):

- body byte-identical to the file; `Content-Type` `image/png`, `image/webp` or `image/x-icon`;
  `Cache-Control: public, max-age=31536000, immutable`; the security headers of every response;
- one route per distinct file: the same file configured under two keys is one URL;
- every other path under `/branding/`, and every `/branding/` path on an instance without branding
  files, has no route and gets the generic `404` of an unknown path, byte for byte.

With at least one colour configured, the generated stylesheet is served the same way at
`/branding/<first 16 hex digits of its SHA-256>.css` as `text/css; charset=utf-8`. It contains
only the configured tokens, as `--cb-x-y: #rrggbb;`, in the order of `tokens.css`, in these blocks
(a theme without configured tokens has none):

1. light tokens in `@media not all and (prefers-color-scheme: dark) { :root { … } }`;
2. dark tokens in `@media (prefers-color-scheme: dark) { :root { … } }`;
3. light tokens in `:root[data-theme="light"] { … }`;
4. dark tokens in `:root[data-theme="dark"] { … }`.

It is linked after the bundled stylesheet, so each block wins over its built-in counterpart of
equal specificity; the two media queries are exact complements, so neither system preference sees
the other theme's values; the `data-theme` blocks (the manual toggle) are more specific and win
over the system preference in both directions. Without colours every `/branding/…css` path is the
generic `404`.

**Access log:** each configured branding file and the colour stylesheet is a route of its own,
registered under its URL, so the access log records that URL as the route's template, verbatim
(`/branding/<16 hex>.<ext>`: a content hash, useful when diagnosing caching). Every other path
below `/branding/` answers the generic `404` and is logged as `{unmatched}`, like any request
that matched no route; nothing from the request reaches the log.

`/favicon.svg` is part of the web build (`public/favicon.svg`, the CapsuleBin symbol in white on a
rounded `#0f766e` tile, fixed colours) and served like any static file as `image/svg+xml`. A
configured favicon replaces the link to it, not the file.

### What the pages show

Every page of the five shells (create, view, manage, report, and the three legal pages from
`legal.html`), in every language:

| Where | Without `[branding]` | With `[branding]` |
| --- | --- | --- |
| `<title>` | `CapsuleBin` (create), `<page> · CapsuleBin` | the name in place of `CapsuleBin`, also with `hide_name` |
| Header symbol | the built-in inline SVG | `logo`: `<img class="cb-logo-img" src="/branding/…">`, no SVG left. With `logo_dark` two images, classes `cb-logo-img cb-logo-img-light` and `cb-logo-img cb-logo-img-dark`, same `alt` |
| Header name | `<span class="cb-logo-name">CapsuleBin</span>` | the name; with `hide_name` the span is gone and the image's `alt` is the name (otherwise `alt=""`, the name next to it says it) |
| Slogan next to the logo (create page only) | `<span class="cb-claim">` with the catalogue's `create.claim` | the page language's tagline in the same span; an empty one removes the span |
| Texts naming the service | `create.noscript`, `view.noscript`, `report.back` say `CapsuleBin` | the name |
| `<link rel="icon">` | `href="/favicon.svg"` | `href` = the favicon's `/branding/` URL |
| Colour stylesheet | no link | with `[branding.colors]`: one `<link rel="stylesheet" href="/branding/<hash>.css">` immediately before `</head>`, after every other stylesheet link; the rest of the page unchanged |
| Footer, last item | `<span>Powered by <a href="https://github.com/capsulebin/capsulebin" target="_blank" rel="noopener noreferrer">CapsuleBin</a></span>` (de: `Betrieben mit …`) | same text, `href` = `source_url` |

- **The attribution always says "CapsuleBin"** — it names the software, not the instance — and
  no key removes it. Only the word "CapsuleBin" is the link; "Powered by" / "Betrieben mit" is
  text outside it. The link opens in a new tab (`target="_blank"`, `rel="noopener noreferrer"`), so
  the current page — possibly an opened capsule or a draft — stays in place.
- **The manage, report and legal pages keep their page title next to the logo** (`.cb-claim`
  without markers); the view page has none. The tagline replaces only the create page's slogan.
- **Operator values are escaped and never interpreted.** `name`, `tagline`, `source_url` and the
  favicon URL are HTML-escaped (`&`, `<`, `>`, `"`, `'`). A name or tagline that contains a server
  token (`%%capsulebin:prefix%%`) or a marker (`<!--capsulebin:legal-link:…-->`) appears literally.
- **Logo size:** CSS height `1.5rem`, automatic width, at most `12rem` wide; a wider logo is
  scaled down inside that box (`object-fit: contain`, left-aligned), not squashed.
- **Dark logo:** exactly one of the two images is visible. The switch is CSS only and mirrors the
  three theme blocks of ADR-005: without `data-theme` on `<html>` the system preference decides
  (`prefers-color-scheme: dark` → dark logo), `data-theme="light"`/`"dark"` (the manual toggle)
  overrides it in both directions. Without `logo_dark` the one logo shows in both themes.
- **The state hexagon stays.** The symbol on the progress, password, warning and "gone" pages
  ([web-interface.md](web-interface.md#state-hexagon)) is CapsuleBin's own and is shown also with
  `logo`: it signals a state, not the brand. It is drawn in the theme's colour tokens.
- The name appears only in the web pages: not in the API (`problem+json`), the CLI, the logs or
  the binary name.

## Structure

| Where | What |
| --- | --- |
| `src/server/src/config/model.rs` | `BrandingConfig` (`Config.branding`, defaults; `colors` is a named field, so `[branding.colors]` never reaches the language map; `languages` collects `[branding.<lang>]` through `flatten` and `branding_languages`, which refuses every non-table key while parsing), `BrandingTexts` (`[branding.<lang>]`, `deny_unknown_fields`, only `tagline`), `BrandingConfig::validate(default_language)` (name and taglines through `check_brand_text` — trimmed, length limit, no control characters, returns the trimmed text; only an empty name is refused —, `hide_name`/`logo_dark` without `logo`, `is_source_url`, unknown and default-language subtables), the constants for defaults and the 64- and 80-character limits |
| `src/server/src/config.rs` | `Config::validate` calls `branding.validate(default_language)` and `branding::resolve()`; `Resolved.branding` |
| `src/server/src/branding.rs` | `Branding` (name trimmed but not escaped, `hide_name`, `taglines` per known language, `logo`/`logo_dark`/`favicon` as `Option<BrandingFile>`, `source_url`), `Tagline` (`BuiltIn`, `Hidden`, `Text` — trimmed, not escaped), `Branding::tagline(language)` (an unlisted language: `BuiltIn`), `BrandingFile` (`url`, `content_type`, `bytes`), `resolve()`: resolves the taglines with their fallback, reads and checks the files (formats, signatures, 256 KiB), computes the URLs; `Branding.colors` and `Branding.stylesheet` (the generated stylesheet as a `BrandingFile`, `None` without colours); `Branding::files()` (files and stylesheet, deduplicated by URL) |
| `src/server/src/branding/colors.rs` | `Color` (parse `#rgb`/`#rrggbb`, WCAG luminance and contrast, `#rrggbb` display), `Theme`, `TOKENS` (the 18 keys with their built-in light and dark values, order of `tokens.css`), `CONTRAST_PAIRS`; `Colors` — configuration and runtime type with its own `Deserialize` (every error names the full key), `configured(theme)`, `stylesheet()`, `contrast_warnings()`; `ContrastWarning` (its `Display` is the warning text) |
| `src/server/src/commands/serve.rs`, `check.rs` | Contrast warnings: `tracing::warn!` at startup, `warning: …` on stderr |
| `src/server/src/lib.rs` | `AppState.branding: Arc<Branding>`; `router()` merges `api::branding::routes` before the asset routes |
| `src/server/src/api/branding.rs` | One `GET` route per file in `Branding::files()` (stylesheet included), with its MIME type and the immutable `Cache-Control` |
| `src/server/src/api/access_log.rs` | Logs a branding request by its route template, the file's literal `/branding/<16 hex>.<ext>` URL; any other `/branding/` path is `{unmatched}` |
| `src/server/web/tests/fixtures/contrast-pairs.json` | The contrast pairs and thresholds, by config key; read by `contrast.test.ts`, equal to `CONTRAST_PAIRS` by a Rust test |
| `src/server/src/api/assets.rs` | `with_stylesheet()` (`HEAD_END`), `NAME_TOKEN`, `FAVICON_TOKEN`, `SOURCE_TOKEN`, the server-internal `TAGLINE_TOKEN`, `LOGO_REGION`, `WORDMARK_REGION`, `TAGLINE_REGION`, `BUILT_IN_FAVICON`; `render()` → `with_stylesheet()` → `with_branding_regions()` (logo, wordmark and tagline regions, the latter per page language; decides the tagline variant once and also returns the escaped value for `TAGLINE_TOKEN`), `fill_tokens()` (all tokens in one pass) |
| `src/server/web/{index,p,manage,report,legal}.html` | `{{name}}` in `<title>` and the header wordmark, `<link rel="icon" href="{{favicon}}" />` after the `color-scheme` meta and before the head scripts, the logo and wordmark marker regions in the header, `<span>{{shell.poweredBy}}</span>` as the last footer item; `index.html` also the tagline region around its `.cb-claim` |
| `src/server/web/scripts/shells.ts` | `{{name}}`/`{{favicon}}` → `NAME_TOKEN`/`FAVICON_TOKEN`; after escaping a catalogue text, `{name}` → `NAME_TOKEN` (`NAME_PLACEHOLDER`) and `{project}` → `PROJECT_LINK` (`PROJECT_PLACEHOLDER`), the project's name linked to `SOURCE_TOKEN` in a new tab; `DEFAULT_BRANDING`; `fillServerTokens()` fills name, favicon and source token with the defaults for the dev server |
| `src/server/web/src/i18n/{en,de}.json` | `shell.poweredBy` (`Powered by {project}`, `Betrieben mit {project}`); `create.claim` (the built-in slogan); `{name}` in `create.noscript`, `view.noscript`, `report.back` |
| `src/server/web/src/styles/base.css` | `.cb-logo-img` (size, `object-fit`) and the light/dark switch |
| `src/server/web/public/favicon.svg` | The built-in favicon |
| `capsulebin.example.toml` | The commented `[branding]` section, `[branding.colors.light]`/`[branding.colors.dark]` with every key at its built-in value |

### What the build writes and what the server fills in

Besides `%%capsulebin:prefix%%` and `%%capsulebin:default%%` ([i18n.md](i18n.md)) the web build
leaves three tokens and three marker regions to the server:

- `%%capsulebin:name%%` — the escaped `name`: in every `<title>`, in the header wordmark, and
  wherever a catalogue text has `{name}` (the generator escapes the text first, then replaces
  `{name}`, so the token survives escaping).
- `%%capsulebin:favicon%%` — the `href` of `<link rel="icon">`: `/favicon.svg` or the configured
  favicon's URL.
- `%%capsulebin:source%%` — the `href` of the attribution link: the escaped `source_url`. No
  template has a placeholder for it: the catalogue text `shell.poweredBy` carries `{project}`,
  which the generator, after escaping the text, replaces with
  `<a href="%%capsulebin:source%%" target="_blank" rel="noopener noreferrer">CapsuleBin</a>`
  (`PROJECT_LINK`). A `{project}` in a catalogue text placed inside a marker comment fails the
  build.
- `<!--capsulebin:logo-->` … built-in symbol SVG … `<!--/capsulebin:logo-->`: replaced as a whole
  by the logo image(s) when `logo` is set; otherwise only the two markers are removed and the
  symbol stays.
- `<!--capsulebin:wordmark-->` `<span class="cb-logo-name">%%capsulebin:name%%</span>`
  `<!--/capsulebin:wordmark-->`: removed as a whole with `hide_name`; otherwise only the markers go.
- `<!--capsulebin:tagline-->` `<span class="cb-claim">…create.claim…</span>`
  `<!--/capsulebin:tagline-->`, in `index.html` only: with a configured tagline of the page's
  language replaced as a whole by `<span class="cb-claim">%%capsulebin:tagline%%</span>`, removed
  as a whole with an empty one; with the built-in slogan only the markers go.
  `%%capsulebin:tagline%%` is server-internal — the web build never writes it — and is filled with
  the escaped text in the same token pass as the name.

The regions span several lines (Prettier formats the templates); the server replaces whatever lies
between the markers. A start marker without its end is left as it is. No `<!--capsulebin:` marker
of a complete region reaches a served page.

The colour stylesheet's link needs neither token nor marker: no template changes, so the web build
stays the same for every instance.

**Order in `render()`:** (0) with colours, the stylesheet link before the first `</head>` of the raw
shell, before any operator value is in it (a shell without `</head>` stays as it is); (1) logo,
wordmark and tagline regions — the images' `alt` is written as the name token, the tagline as the
tagline token; (2) every token (prefix, default, name, tagline,
favicon, source) in **one pass**, so a value that was put in is never searched for tokens again,
and text that only looks like a token start stays; (3) legal footer links; then, in
`Document::build`, legal title and content. Every operator value is escaped and therefore holds no
`<`, so it cannot form a marker for the later steps either.

The Vite dev server fills name, favicon and source token with `DEFAULT_BRANDING` (name
`CapsuleBin`, `/favicon.svg`, the upstream repository) and leaves the marker regions in place
(tagline region included, with the built-in slogan inside); they are comments and render nothing.
It links no colour stylesheet and shows the built-in colours only.

### Tests

| Test | Covers |
| --- | --- |
| `src/server/src/config/tests.rs` | Defaults, env equivalents (incl. `CAPSULEBIN__BRANDING__DE__TAGLINE` and every colour key), unknown keys and subtables (colours included), refused colour values, `[branding.colors]` next to `[branding.de]`, tagline limits, every `validate` and file error (temporary files) |
| `src/server/src/branding/colors.rs` (unit) | Colour syntax and normalisation, WCAG ratio, warnings on the merged palette per theme (also a tint equal to its status colour), the exact stylesheet text; `TOKENS` equal to all four palette blocks of `tokens.css`, `CONTRAST_PAIRS` equal to the fixture, the example configuration's colour sections equal to `TOKENS` |
| `src/server/src/branding.rs`, `src/server/src/api/assets.rs` (unit) | Signatures, hash length, tagline fallback and trimming; `render()` with default, escaped and token-like values, logo, dark logo, tagline per page language, incomplete regions; every template carries tokens and regions once, the tagline region only in `index.html` |
| `src/server/tests/branding.rs` | `/branding/` serving (both asset modes, read once at startup, same file twice, generic `404`), and every page in `en`/`de` from its own test shells via `server.asset_dir`: default, escaped name, token-like name, logo, `hide_name`, all keys set, favicon alone; tagline for every language, overridden by `[branding.de]`, only `[branding.de]`, empty, escaped and literal; the attribution with only the project's name linked |
| `src/server/tests/branding_colors.rs` | The stylesheet served from memory in both asset modes; without colours every `/branding/…css` the generic `404`; every page in `en`/`de` (legal pages included) with exactly one link last in the head and otherwise unchanged, without colours no link |
| `src/server/tests/branding_acceptance.rs` | The real binary: every unknown key, refused value (colours from file and environment included) and every unusable or unreadable file stops `check` and `serve` with a non-zero exit code and a message naming the key (no file content); every key set from the environment reaches the started binary; contrast warnings on `check`'s stderr with `ok:` and exit 0, and in the `WARN` log of a started `serve` |
| `src/server/tests/logging.rs` | A configured branding file logged by its literal URL; a branding-shaped path on an instance without that file logged as `{unmatched}` |
| `src/server/web/tests/contrast.test.ts` | Every pair of `contrast-pairs.json` on all four palette blocks of `tokens.css` |
| `src/server/web/tests/shells.test.ts` | No literal `CapsuleBin` in the templates, the attribution as the last footer item with only the project's name linked (the only link without the prefix token), `{project}` turned into the link after escaping and refused inside a comment, the tagline markers around the create page's slogan and nowhere else |
| `src/server/web/tests/dev-server.acceptance.test.ts` | The dev server leaves no token unfilled and the tagline region in place |
| `src/server/web/e2e/branding.e2e.ts` | Default instance (favicon, symbol, attribution in `en`/`de`); a branded instance started per worker through `e2e/instance.ts` with the fixtures in `e2e/fixtures/` (name and taglines with special characters, `[branding.de]` tagline, PNG logo, WebP dark logo, PNG favicon, custom `source_url`); an instance with an empty tagline; the attribution opening a new page and leaving the current one; logo visibility for both system preferences and both toggle directions, with and without a dark logo; on the branded instance (colours from the environment: `accent` in both themes, `danger` light only) the computed `--cb-accent`/`--cb-danger` in every theme state, the dark `danger` staying built-in; the default instance requests no colour stylesheet |
| `scripts/smoke.sh` | Every page links `/favicon.svg` and the upstream repository (the one allowed outgoing link, only "CapsuleBin", new tab); `/favicon.svg` is `image/svg+xml` with the security headers |

The Rust page tests use hand-written test shells because the CI job `Rust` has no web build; the
real shells are covered by the shell tests and the browser tests.

## Decisions

- **Configuration, not operator CSS or templates** — named keys keep class names and markup
  internal, values checkable and upgrades safe; an operator stylesheet would turn every class name
  into an unannounced API. See [ADR-017](adr/ADR-017-operator-branding.md).
- **No SVG from the operator: logos PNG/WebP, favicon PNG/ICO, checked by extension and
  signature** — an SVG is an active document; serving one would need a per-response sandbox CSP or
  a change to the security-header layer in `router()`, and SVG sanitising. The cost is lossless
  scaling; a PNG at twice the display height stays sharp. The built-in `/favicon.svg` stays: it
  comes from the build, not from the operator.
- **Files read once at startup, served from memory under `/branding/<16 hex of SHA-256>.<ext>`
  with immutable caching** — the shells are `no-store`, so a replaced file shows up at once after a
  restart under its new URL, without a stale cache; the URL reveals nothing about the operator's
  paths. 256 KiB per file is generous for a header logo and small in memory. Paths resolve like
  `database.path`.
- **Every other `/branding/` path falls through to the asset fallback** rather than having its own
  `404` handler — the answer is then byte-identical to any unknown path.
- **File errors are startup errors in `Config::validate`** — so `capsulebin check` reports a
  missing or wrong file before `serve` does, like the legal texts.
- **One name for every language** — a per-language name was rejected; the
  name is an identity, not interface text.
- **Name limits: trimmed non-empty, at most 64 characters, no control characters** — enough for a
  `<title>` and a wordmark, short enough to keep the header layout intact.
- **The tagline per language: `tagline` for all, `[branding.<lang>] tagline` overrides, without
  either the built-in translated slogan** — unlike the name, the slogan is interface text, so an
  operator with a translated slogan needs one per language; one value for every language keeps the
  common case short. Same pattern as `[legal.<lang>]`: a subtable for the default language is
  refused because `tagline` already is its text, so the configuration has one place for it.
- **An empty tagline removes the slogan element** rather than leaving an empty span or falling back
  to the built-in one — the operator's way to have no slogan, per language too.
- **Tagline at most 80 characters, no control characters** — one line next to the logo.
- **Only the create page's slogan is replaceable; the manage, report and legal pages keep their page
  title in that place** (the view page has none) — there the `.cb-claim` says which page you are on.
- **The tagline through a region in the shell and a server-internal token** — the region lets the
  server replace or drop the whole span; writing the text as `%%capsulebin:tagline%%` and filling it
  in the one token pass gives it the same "literal, never re-scanned" guarantee as the name.
  Replacing the region with the text directly would put operator text into the shell before the
  token pass, which would then expand a token-like tagline.
- **`hide_name` and `logo_dark` without `logo` are refused, not ignored** — hiding the name
  without a logo would leave the header empty, and a dark variant of no logo is a configuration
  mistake; in line with "unknown keys are a startup error".
- **`source_url` an absolute http(s) URL with a host, no user info, no whitespace or control
  characters** — it points to a source repository, so a local path makes no sense; the value is
  used as written, and the URL parser would silently drop whitespace and control characters that
  then still reach the page.
- **The attribution cannot be switched off and always says "CapsuleBin"** — it is the AGPL §13
  hint to the source of the running software on every page, legal pages included; `source_url`
  lets the operator of a modified version point to their source. The name in it is the project's
  (`docs/brand/README.md`).
- **Only "CapsuleBin" is the link, opened in a new tab** — the
  linked word is the software's name, and following it leaves the current page in place
  (`noopener noreferrer`: the new tab gets neither a handle on the page nor a referrer). Built as
  the catalogue placeholder `{project}`, replaced by the generator after escaping, so the
  translated text around it stays catalogue text and the link markup is the generator's, never the
  catalogue's.
- **Default instance: the built-in identity, plus the attribution and the built-in `/favicon.svg`**
  — so the browser's favicon request is answered.
- **Tokens and marker regions in the shells, filled by the server** — the reproducible web build
  holds nothing instance-specific; the same mechanism as the language prefix and the legal links.
  The templates use `{{name}}`/`{{favicon}}` and the catalogue `{project}`, which the generator
  turns into tokens, so the shell test's rule "every visible text from the catalogue" still holds
  and no template contains a literal `CapsuleBin`.
- **All tokens filled in one pass, after the regions and before the legal content** — a single
  pass means no inserted value is scanned again, so an operator value that looks like a token stays
  literal; the legal links and legal text (also operator input) come after and are not scanned for
  tokens either. Filling the tokens one `replace` after another would let a name like
  `%%capsulebin:prefix%%` be expanded.
- **Without a logo the region markers are removed, the symbol and wordmark stay** — no server
  marker reaches a served page.
- **`alt=""` next to the visible name, `alt` = name with `hide_name`** — the name is read once
  either way; with `logo_dark` both images carry the same `alt`.
- **Logo at `1.5rem` height, auto width, max `12rem`, `object-fit: contain`** — keeps wordmark
  logos readable without pushing the header tools off a narrow screen; without `object-fit` a logo
  wider than the cap would be squashed horizontally.
- **Dark logo switched by CSS on `data-theme`, not `<picture media>`** — `<picture>` follows only
  the system preference and would ignore the manual toggle; the CSS mirrors the three theme blocks
  of ADR-005.
- **Branding is not mentioned in the privacy notice or the built-in legal texts** — it adds no
  third-party request, no cookie and no storage; the attribution is an ordinary outgoing link under
  `Referrer-Policy: no-referrer`.
- **Colours as named token keys, turned into a generated stylesheet** — not operator CSS (see
  above): the server writes the CSS itself from checked values, so the tokens stay the only
  interface and contrast can be computed.
- **A contrast failure is a warning, not a startup error** — an operator's corporate colours may
  miss a threshold by a little, and refusing to start would be out of proportion; the warning
  makes it visible in the log and in `check`.
- **Only `#rgb` / `#rrggbb`** — enough to compute contrast exactly, trivial to validate, and the
  value can never leave its CSS declaration. No alpha: the contrast of a translucent colour depends
  on what lies below it. Values are not trimmed; whitespace is refused like any wrong syntax.
- **`--cb-shadow-card` stays built-in** — not a plain colour and no part of any contrast pair; 18
  tokens per theme are configurable.
- **`[branding.colors.light]`/`[branding.colors.dark]`, not `[branding.light]`/`[branding.dark]`**
  — `BrandingConfig` collects `[branding.<lang>]` through `flatten`; `colors` as a named field never
  reaches that map, so colours and language tables cannot collide. Keys are the token names in
  snake_case because env keys are lowercased and `__` separates path segments.
- **`Colors` has its own `Deserialize` instead of `deny_unknown_fields`** — so every error names
  the full key (`branding.colors.light.shadow_card`), from the file or the environment.
- **The light block sits in `@media not all and (prefers-color-scheme: dark)`, not in a plain
  `:root`** — the generated sheet comes after the bundled one, so a plain `:root { light }` would
  override the bundle's dark media block for a dark system preference. With the exact complement of
  the dark query, a token set for one theme never leaks into the other; the `data-theme` blocks
  follow and win by specificity, so the toggle works both ways.
- **The link is inserted before `</head>`, not through a shell marker** — Vite appends its own
  stylesheet link at the end of the head, after anything in the template; with equal specificity
  the later sheet wins. Without template changes the web build and its reproducibility are
  untouched, and an instance without colours serves byte-identical pages. It is inserted first in
  `render()`, into the raw shell, before any operator value is put in, so the `</head>` it is placed
  before is always the shell's own.
- **The stylesheet is a `BrandingFile`** — the same content-hashed URL, in-memory serving,
  immutable caching and generic `404` as the images, through the same route; `style-src 'self'`
  allows it, so the CSP is unchanged.
- **Contrast pairs in one fixture, the built-in palette copied to Rust and pinned by tests** — the
  web contrast test and the server's warnings cannot drift apart. The pairs are a Rust constant
  that a test keeps equal to `src/server/web/tests/fixtures/contrast-pairs.json`, so production
  code does not depend on a file under `web/tests`; `TOKENS` is kept equal to `tokens.css` the same
  way.
- **Coloured text on the tints is checked too** — the pages put `accent`, `danger`, `warning`,
  `success` and `text` on the `tint_*` tokens (chips, the copy buttons' copied state, the warning
  states, the danger button's hover; see [web-interface.md](web-interface.md)), so a tint that
  leaves its text unreadable is worth a warning. The built-in palette passes (lowest: light
  `warning` on `tint_warning`, 4.57:1); an operator palette may now get a warning for these pairs,
  never an error.
- **Warnings only from `serve` and `check`** — the two commands where an operator looks at the
  configuration; `check` writes them to stderr so its stdout stays one `ok:` line.
- **Branding file URLs reach the access log as route templates** — the log records only the
  template of the matched route, and branding routes are registered under their configured,
  content-hashed URL, so they stay visible without any filename rule.

## Limits and open points

- No SVG logos or favicons, no custom fonts, no operator CSS, no layout changes.
- Colours: no `--cb-shadow-card` or other non-colour token, no syntax beyond `#rgb`/`#rrggbb`, no
  colours per language. The built-in `/favicon.svg` and the Vite dev server keep the built-in
  colours. Contrast is checked only for the pairs above (not `border`, not `accent_hover`); a
  failing pair never stops the start.
- The name and the colours are not used in the CLI, the API's `problem+json`, the logs or the
  binary name.
- Changing a file or a colour needs a restart; there is no reload.
- Branding needs no CSP or security-header change; images load under `img-src 'self' data:`, the
  colour stylesheet under `style-src 'self'`.
