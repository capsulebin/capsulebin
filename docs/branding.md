# Operator branding

An operator can present a CapsuleBin instance under their own identity without a rebuild. The
`[branding]` table sets the name shown in place of "CapsuleBin" in the web pages, the slogan next
to the logo on the create page (per language if needed, or none), a logo that replaces the
built-in symbol in the header (optionally a second one for the dark theme) and a favicon. The
server reads the image files once at startup, serves them same-origin under content-derived URLs
and fills everything into the shells, as it does for the legal pages (ADR-015). Every page of
every instance, branded or not, carries a fixed "Powered by CapsuleBin" attribution in the footer,
in which "CapsuleBin" links to the source code (AGPL-3.0 §13) in a new tab; only the link target
is configurable. An instance without `[branding]` looks as before, plus that attribution and a
built-in favicon. See [ADR-017](adr/ADR-017-operator-branding.md). Theme colours are not part of
this yet (see [Limits](#limits-and-open-points)).

## Rules

### Configuration

`[branding]` takes exactly the keys below plus one optional subtable `[branding.<lang>]` per known
language other than `i18n.default_language`, which takes exactly `tagline`. Any other key in
`[branding]` or in a subtable, a subtable for an unknown language and a subtable for the default
language are startup errors. Every key also as `CAPSULEBIN__BRANDING__<KEY>`, a language's tagline
as `CAPSULEBIN__BRANDING__<LANG>__TAGLINE` (e.g. `CAPSULEBIN__BRANDING__DE__TAGLINE`).

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

### Serving the files

Each configured file is served at `/branding/<first 16 hex digits of its SHA-256>.<ext>` (`ext`
from the format: `png`, `webp`, `ico`), from memory, in both asset modes (embedded and
`server.asset_dir`):

- body byte-identical to the file; `Content-Type` `image/png`, `image/webp` or `image/x-icon`;
  `Cache-Control: public, max-age=31536000, immutable`; the security headers of every response;
- one route per distinct file: the same file configured under two keys is one URL;
- every other path under `/branding/`, and every `/branding/` path on an instance without branding
  files, has no route and gets the generic `404` of an unknown path, byte for byte.

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
| Footer, last item | `<span>Powered by <a href="https://github.com/capsulebin/capsulebin" target="_blank" rel="noopener noreferrer">CapsuleBin</a></span>` (de: `Betrieben mit …`) | same text, `href` = `source_url` |

- **The attribution always says "CapsuleBin"** — it names the software, not the instance — and
  no key removes it. Only the word "CapsuleBin" is the link; "Powered by" / "Betrieben mit" is
  text outside it. The link opens in a new tab (`target="_blank"`, `rel="noopener noreferrer"`), so
  the current page — possibly an opened capsule or a draft — stays in place.
- **The other pages keep their page title next to the logo** (`.cb-claim` without markers); the
  tagline replaces only the create page's slogan.
- **Operator values are escaped and never interpreted.** `name`, `tagline`, `source_url` and the
  favicon URL are HTML-escaped (`&`, `<`, `>`, `"`, `'`). A name or tagline that contains a server
  token (`%%capsulebin:prefix%%`) or a marker (`<!--capsulebin:legal-link:…-->`) appears literally.
- **Logo size:** CSS height `1.5rem`, automatic width, at most `12rem` wide; a wider logo is
  scaled down inside that box (`object-fit: contain`, left-aligned), not squashed.
- **Dark logo:** exactly one of the two images is visible. The switch is CSS only and mirrors the
  three theme blocks of ADR-005: without `data-theme` on `<html>` the system preference decides
  (`prefers-color-scheme: dark` → dark logo), `data-theme="light"`/`"dark"` (the manual toggle)
  overrides it in both directions. Without `logo_dark` the one logo shows in both themes.
- The name appears only in the web pages: not in the API (`problem+json`), the CLI, the logs or
  the binary name.

## Structure

| Where | What |
| --- | --- |
| `src/server/src/config/model.rs` | `BrandingConfig` (`Config.branding`, defaults; `languages` collects `[branding.<lang>]` through `flatten` and `branding_languages`, which refuses every non-table key while parsing), `BrandingTexts` (`[branding.<lang>]`, `deny_unknown_fields`, only `tagline`), `BrandingConfig::validate(default_language)` (name and taglines through `check_brand_text` — trimmed, length limit, no control characters, returns the trimmed text; only an empty name is refused —, `hide_name`/`logo_dark` without `logo`, `is_source_url`, unknown and default-language subtables), the constants for defaults and the 64- and 80-character limits |
| `src/server/src/config.rs` | `Config::validate` calls `branding.validate(default_language)` and `branding::resolve()`; `Resolved.branding` |
| `src/server/src/branding.rs` | `Branding` (name trimmed but not escaped, `hide_name`, `taglines` per known language, `logo`/`logo_dark`/`favicon` as `Option<BrandingFile>`, `source_url`), `Tagline` (`BuiltIn`, `Hidden`, `Text` — trimmed, not escaped), `Branding::tagline(language)` (an unlisted language: `BuiltIn`), `BrandingFile` (`url`, `content_type`, `bytes`), `resolve()`: resolves the taglines with their fallback, reads and checks the files (formats, signatures, 256 KiB), computes the URLs; `Branding::files()` (deduplicated by URL) |
| `src/server/src/lib.rs` | `AppState.branding: Arc<Branding>`; `router()` merges `api::branding::routes` before the asset routes |
| `src/server/src/api/branding.rs` | One `GET` route per file in `Branding::files()`, with its MIME type and the immutable `Cache-Control` |
| `src/server/src/api/assets.rs` | `NAME_TOKEN`, `FAVICON_TOKEN`, `SOURCE_TOKEN`, the server-internal `TAGLINE_TOKEN`, `LOGO_REGION`, `WORDMARK_REGION`, `TAGLINE_REGION`, `BUILT_IN_FAVICON`; `render()` → `with_branding_regions()` (logo, wordmark and tagline regions, the latter per page language; decides the tagline variant once and also returns the escaped value for `TAGLINE_TOKEN`), `fill_tokens()` (all tokens in one pass) |
| `src/server/web/{index,p,manage,report,legal}.html` | `{{name}}` in `<title>` and the header wordmark, `<link rel="icon" href="{{favicon}}" />` after the `color-scheme` meta and before the head scripts, the logo and wordmark marker regions in the header, `<span>{{shell.poweredBy}}</span>` as the last footer item; `index.html` also the tagline region around its `.cb-claim` |
| `src/server/web/scripts/shells.ts` | `{{name}}`/`{{favicon}}` → `NAME_TOKEN`/`FAVICON_TOKEN`; after escaping a catalogue text, `{name}` → `NAME_TOKEN` (`NAME_PLACEHOLDER`) and `{project}` → `PROJECT_LINK` (`PROJECT_PLACEHOLDER`), the project's name linked to `SOURCE_TOKEN` in a new tab; `DEFAULT_BRANDING`; `fillServerTokens()` fills name, favicon and source token with the defaults for the dev server |
| `src/server/web/src/i18n/{en,de}.json` | `shell.poweredBy` (`Powered by {project}`, `Betrieben mit {project}`); `create.claim` (the built-in slogan); `{name}` in `create.noscript`, `view.noscript`, `report.back` |
| `src/server/web/src/styles/base.css` | `.cb-logo-img` (size, `object-fit`) and the light/dark switch |
| `src/server/web/public/favicon.svg` | The built-in favicon |
| `capsulebin.example.toml` | The commented `[branding]` section |

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

**Order in `render()`:** (1) logo, wordmark and tagline regions — the images' `alt` is written as
the name token, the tagline as the tagline token; (2) every token (prefix, default, name, tagline,
favicon, source) in **one pass**, so a value that was put in is never searched for tokens again,
and text that only looks like a token start stays; (3) legal footer links; then, in
`Document::build`, legal title and content. Every operator value is escaped and therefore holds no
`<`, so it cannot form a marker for the later steps either.

The Vite dev server fills name, favicon and source token with `DEFAULT_BRANDING` (name
`CapsuleBin`, `/favicon.svg`, the upstream repository) and leaves the marker regions in place
(tagline region included, with the built-in slogan inside); they are comments and render nothing.

### Tests

| Test | Covers |
| --- | --- |
| `src/server/src/config/tests.rs` | Defaults, env equivalents (incl. `CAPSULEBIN__BRANDING__DE__TAGLINE`), unknown keys and subtables, tagline limits, every `validate` and file error (temporary files) |
| `src/server/src/branding.rs`, `src/server/src/api/assets.rs` (unit) | Signatures, hash length, tagline fallback and trimming; `render()` with default, escaped and token-like values, logo, dark logo, tagline per page language, incomplete regions; every template carries tokens and regions once, the tagline region only in `index.html` |
| `src/server/tests/branding.rs` | `/branding/` serving (both asset modes, read once at startup, same file twice, generic `404`), and every page in `en`/`de` from its own test shells via `server.asset_dir`: default, escaped name, token-like name, logo, `hide_name`, all keys set, favicon alone; tagline for every language, overridden by `[branding.de]`, only `[branding.de]`, empty, escaped and literal; the attribution with only the project's name linked |
| `src/server/tests/branding_acceptance.rs` | The real binary: every unknown key, refused value and every unusable or unreadable file stops `check` and `serve` with a non-zero exit code and a message naming the key (no file content); every key set from the environment reaches the started binary |
| `src/server/web/tests/shells.test.ts` | No literal `CapsuleBin` in the templates, the attribution as the last footer item with only the project's name linked (the only link without the prefix token), `{project}` turned into the link after escaping and refused inside a comment, the tagline markers around the create page's slogan and nowhere else |
| `src/server/web/tests/dev-server.acceptance.test.ts` | The dev server leaves no token unfilled and the tagline region in place |
| `src/server/web/e2e/branding.e2e.ts` | Default instance (favicon, symbol, attribution in `en`/`de`); a branded instance started per worker through `e2e/instance.ts` with the fixtures in `e2e/fixtures/` (name and taglines with special characters, `[branding.de]` tagline, PNG logo, WebP dark logo, PNG favicon, custom `source_url`); an instance with an empty tagline; the attribution opening a new page and leaving the current one; logo visibility for both system preferences and both toggle directions, with and without a dark logo |
| `scripts/smoke.sh` | Every page links `/favicon.svg` and the upstream repository (the one allowed outgoing link, only "CapsuleBin", new tab); `/favicon.svg` is `image/svg+xml` with the security headers |

The Rust page tests use hand-written test shells because the CI job `Rust` has no web build; the
real shells are covered by the shell tests and the browser tests.

## Decisions

- **Configuration, not operator CSS or templates** — named keys keep class names and markup
  internal, values checkable and upgrades safe; an operator stylesheet would turn every class name
  into an unannounced API. See [ADR-017](adr/ADR-017-operator-branding.md).
  (20261120-branding-identity)
- **No SVG from the operator: logos PNG/WebP, favicon PNG/ICO, checked by extension and
  signature** — an SVG is an active document; serving one would need a per-response sandbox CSP or
  a change to the security-header layer in `router()`, and SVG sanitising. The cost is lossless
  scaling; a PNG at twice the display height stays sharp. The built-in `/favicon.svg` stays: it
  comes from the build, not from the operator. (20261120-branding-identity)
- **Files read once at startup, served from memory under `/branding/<16 hex of SHA-256>.<ext>`
  with immutable caching** — the shells are `no-store`, so a replaced file shows up at once after a
  restart under its new URL, without a stale cache; the URL reveals nothing about the operator's
  paths. 256 KiB per file is generous for a header logo and small in memory. Paths resolve like
  `database.path`. (20261120-branding-identity)
- **Every other `/branding/` path falls through to the asset fallback** rather than having its own
  `404` handler — the answer is then byte-identical to any unknown path. (20261120-branding-identity)
- **File errors are startup errors in `Config::validate`** — so `capsulebin check` reports a
  missing or wrong file before `serve` does, like the legal texts. (20261120-branding-identity)
- **One name for every language** — a per-language name was rejected in the exploration; the
  name is an identity, not interface text. (20261120-branding-identity)
- **Name limits: trimmed non-empty, at most 64 characters, no control characters** — enough for a
  `<title>` and a wordmark, short enough to keep the header layout intact. (20261120-branding-identity)
- **The tagline per language: `tagline` for all, `[branding.<lang>] tagline` overrides, without
  either the built-in translated slogan** — unlike the name, the slogan is interface text, so an
  operator with a translated slogan needs one per language; one value for every language keeps the
  common case short. Same pattern as `[legal.<lang>]`: a subtable for the default language is
  refused because `tagline` already is its text, so the configuration has one place for it.
  (20261120-branding-identity, user decision)
- **An empty tagline removes the slogan element** rather than leaving an empty span or falling back
  to the built-in one — the operator's way to have no slogan, per language too.
  (20261120-branding-identity, user decision)
- **Tagline at most 80 characters, no control characters** — one line next to the logo.
  (20261120-branding-identity)
- **Only the create page's slogan is replaceable; the other pages keep their page title in that
  place** — there the `.cb-claim` says which page you are on. (20261120-branding-identity)
- **The tagline through a region in the shell and a server-internal token** — the region lets the
  server replace or drop the whole span; writing the text as `%%capsulebin:tagline%%` and filling it
  in the one token pass gives it the same "literal, never re-scanned" guarantee as the name.
  Replacing the region with the text directly would put operator text into the shell before the
  token pass, which would then expand a token-like tagline. (20261120-branding-identity)
- **`hide_name` and `logo_dark` without `logo` are refused, not ignored** — hiding the name
  without a logo would leave the header empty, and a dark variant of no logo is a configuration
  mistake; in line with "unknown keys are a startup error". (20261120-branding-identity)
- **`source_url` an absolute http(s) URL with a host, no user info, no whitespace or control
  characters** — it points to a source repository, so a local path makes no sense; the value is
  used as written, and the URL parser would silently drop whitespace and control characters that
  then still reach the page. (20261120-branding-identity)
- **The attribution cannot be switched off and always says "CapsuleBin"** — it is the AGPL §13
  hint to the source of the running software on every page, legal pages included; `source_url`
  lets the operator of a modified version point to their source. The name in it is the project's
  (`docs/brand/README.md`). (20261120-branding-identity)
- **Only "CapsuleBin" is the link, opened in a new tab** — requested by the user after review: the
  linked word is the software's name, and following it leaves the current page in place
  (`noopener noreferrer`: the new tab gets neither a handle on the page nor a referrer). Built as
  the catalogue placeholder `{project}`, replaced by the generator after escaping, so the
  translated text around it stays catalogue text and the link markup is the generator's, never the
  catalogue's. (20261120-branding-identity)
- **Default instance: as before, plus the attribution and the built-in favicon** — which also ends
  the `404` for the browser's favicon request. (20261120-branding-identity)
- **Tokens and marker regions in the shells, filled by the server** — the reproducible web build
  holds nothing instance-specific; the same mechanism as the language prefix and the legal links.
  The templates use `{{name}}`/`{{favicon}}` and the catalogue `{project}`, which the generator
  turns into tokens, so the shell test's rule "every visible text from the catalogue" still holds
  and no template contains a literal `CapsuleBin`. (20261120-branding-identity)
- **All tokens filled in one pass, after the regions and before the legal content** — a single
  pass means no inserted value is scanned again, so an operator value that looks like a token stays
  literal; the legal links and legal text (also operator input) come after and are not scanned for
  tokens either. Filling the tokens one `replace` after another would let a name like
  `%%capsulebin:prefix%%` be expanded. (20261120-branding-identity)
- **Without a logo the region markers are removed, the symbol and wordmark stay** — no server
  marker reaches a served page. (20261120-branding-identity)
- **`alt=""` next to the visible name, `alt` = name with `hide_name`** — the name is read once
  either way; with `logo_dark` both images carry the same `alt`. (20261120-branding-identity)
- **Logo at `1.5rem` height, auto width, max `12rem`, `object-fit: contain`** — keeps wordmark
  logos readable without pushing the header tools off a narrow screen; without `object-fit` a logo
  wider than the cap would be squashed horizontally. (20261120-branding-identity)
- **Dark logo switched by CSS on `data-theme`, not `<picture media>`** — `<picture>` follows only
  the system preference and would ignore the manual toggle; the CSS mirrors the three theme blocks
  of ADR-005. (20261120-branding-identity)
- **No change to the privacy notice or the built-in legal texts** — branding adds no third-party
  request, no cookie and no storage; the attribution is an ordinary outgoing link under
  `Referrer-Policy: no-referrer`. (20261120-branding-identity)

## Limits and open points

- Theme colours (`[branding.light]`/`[branding.dark]`, a generated override stylesheet, a contrast
  warning) are a planned follow-up and will extend ADR-017.
- No SVG logos or favicons, no custom fonts, no operator CSS, no layout changes.
- The name is not used in the CLI, the API's `problem+json`, the logs or the binary name.
- Changing a file needs a restart; there is no reload.
- The access log redacts the `branding` segment like any unknown segment: a PNG or ICO request is
  logged as `/{id}/<hash>.png`, a WebP request as `/{id}/{id}` (`webp` is not among the asset
  extensions of `access_log.rs`).
- The CSP and the security-header layer were not changed; images load under `img-src 'self'`.

## History

- 2026-11-20 20261120-branding-identity: `[branding]` with name, logo, dark logo, favicon and
  `source_url`; files served under `/branding/`; built-in `/favicon.svg`; "Powered by CapsuleBin"
  footer link on every page.
- 2026-11-20 20261120-branding-identity: `tagline` and `[branding.<lang>] tagline` for the create
  page's slogan (empty removes it); only "CapsuleBin" in the attribution is the link, opened in a
  new tab; wide logos scaled, not squashed.
