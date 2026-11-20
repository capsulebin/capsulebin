# Operator branding

An operator can present a CapsuleBin instance under their own identity without a rebuild. The
`[branding]` table sets the name shown in place of "CapsuleBin" in the web pages, a logo that
replaces the built-in symbol in the header (optionally a second one for the dark theme) and a
favicon. The server reads the image files once at startup, serves them same-origin under
content-derived URLs and fills everything into the shells, as it does for the legal pages
(ADR-015). Every page of every instance, branded or not, carries a fixed "Powered by CapsuleBin"
footer link to the source code (AGPL-3.0 §13); only its target is configurable. An instance
without `[branding]` looks as before, plus that link and a built-in favicon. See
[ADR-017](adr/ADR-017-operator-branding.md). Theme colours are not part of this yet (see
[Limits](#limits-and-open-points)).

## Rules

### Configuration

`[branding]` (`deny_unknown_fields`: any other key is a startup error). Every key also as
`CAPSULEBIN__BRANDING__<KEY>`.

| Key | Default | Rule |
| --- | --- | --- |
| `name` | `"CapsuleBin"` | Trimmed; must not be empty or only whitespace, at most 64 characters (Unicode scalar values, after trimming), no control character. One name for every language. |
| `hide_name` | `false` | Show the logo without the name next to it. Requires `logo`. |
| `logo` | none | Path to a PNG or WebP file (`.png`, `.webp`). Replaces the built-in symbol in the header. |
| `logo_dark` | none | Path to a PNG or WebP file, shown instead of `logo` in the dark theme. Requires `logo`. |
| `favicon` | none | Path to a PNG or ICO file (`.png`, `.ico`). Without it, the built-in `/favicon.svg`. |
| `source_url` | `"https://github.com/capsulebin/capsulebin"` | Target of the attribution link: an absolute `http`/`https` URL with a host, without user name or password, without whitespace or control characters. |

Startup errors (also reported by `capsulebin check`), each naming the key:

- the `name` rules above; `hide_name = true` without `logo` ("the header would be empty");
  `logo_dark` without `logo`; a `source_url` that breaks its rule;
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
| Texts naming the service | `create.noscript`, `view.noscript`, `report.back` say `CapsuleBin` | the name |
| `<link rel="icon">` | `href="/favicon.svg"` | `href` = the favicon's `/branding/` URL |
| Footer, last link | `<a href="https://github.com/capsulebin/capsulebin">Powered by CapsuleBin</a>` (de: `Betrieben mit CapsuleBin`) | same text, `href` = `source_url` |

- **The attribution always says "CapsuleBin"** — it names the software, not the instance — and
  no key removes it.
- **Operator values are escaped and never interpreted.** `name`, `source_url` and the favicon URL
  are HTML-escaped (`&`, `<`, `>`, `"`, `'`). A name that contains a server token
  (`%%capsulebin:prefix%%`) or a marker (`<!--capsulebin:legal-link:…-->`) appears literally.
- **Logo size:** CSS height `1.5rem`, automatic width, at most `12rem` wide.
- **Dark logo:** exactly one of the two images is visible. The switch is CSS only and mirrors the
  three theme blocks of ADR-005: without `data-theme` on `<html>` the system preference decides
  (`prefers-color-scheme: dark` → dark logo), `data-theme="light"`/`"dark"` (the manual toggle)
  overrides it in both directions. Without `logo_dark` the one logo shows in both themes.
- The name appears only in the web pages: not in the API (`problem+json`), the CLI, the logs or
  the binary name.

## Structure

| Where | What |
| --- | --- |
| `src/server/src/config/model.rs` | `BrandingConfig` (`Config.branding`, defaults, `deny_unknown_fields`), `BrandingConfig::validate` (name, `hide_name`/`logo_dark` without `logo`, `is_source_url`), the constants for defaults and the 64-character limit |
| `src/server/src/config.rs` | `Config::validate` calls `branding.validate()` and `branding::resolve()`; `Resolved.branding` |
| `src/server/src/branding.rs` | `Branding` (name trimmed but not escaped, `hide_name`, `logo`/`logo_dark`/`favicon` as `Option<BrandingFile>`, `source_url`), `BrandingFile` (`url`, `content_type`, `bytes`), `resolve()`: reads and checks the files (formats, signatures, 256 KiB), computes the URLs; `Branding::files()` (deduplicated by URL) |
| `src/server/src/lib.rs` | `AppState.branding: Arc<Branding>`; `router()` merges `api::branding::routes` before the asset routes |
| `src/server/src/api/branding.rs` | One `GET` route per file in `Branding::files()`, with its MIME type and the immutable `Cache-Control` |
| `src/server/src/api/assets.rs` | `NAME_TOKEN`, `FAVICON_TOKEN`, `SOURCE_TOKEN`, `LOGO_REGION`, `WORDMARK_REGION`, `BUILT_IN_FAVICON`; `render()` → `with_branding_regions()` (logo and wordmark regions), `fill_tokens()` (all tokens in one pass) |
| `src/server/web/{index,p,manage,report,legal}.html` | `{{name}}` in `<title>` and the header wordmark, `<link rel="icon" href="{{favicon}}" />` after the `color-scheme` meta and before the head scripts, the logo and wordmark marker regions in the header, `<a href="{{source}}">{{shell.poweredBy}}</a>` as the last footer link |
| `src/server/web/scripts/shells.ts` | `{{name}}`/`{{favicon}}`/`{{source}}` → `NAME_TOKEN`/`FAVICON_TOKEN`/`SOURCE_TOKEN`; `{name}` in a catalogue text → `NAME_TOKEN` after escaping (`NAME_PLACEHOLDER`); `DEFAULT_BRANDING`; `fillServerTokens()` fills the three tokens with the defaults for the dev server |
| `src/server/web/src/i18n/{en,de}.json` | `shell.poweredBy`; `{name}` in `create.noscript`, `view.noscript`, `report.back` |
| `src/server/web/src/styles/base.css` | `.cb-logo-img` and the light/dark switch |
| `src/server/web/public/favicon.svg` | The built-in favicon |
| `capsulebin.example.toml` | The commented `[branding]` section |

### What the build writes and what the server fills in

Besides `%%capsulebin:prefix%%` and `%%capsulebin:default%%` ([i18n.md](i18n.md)) the web build
leaves three tokens and two marker regions to the server:

- `%%capsulebin:name%%` — the escaped `name`: in every `<title>`, in the header wordmark, and
  wherever a catalogue text has `{name}` (the generator escapes the text first, then replaces
  `{name}`, so the token survives escaping).
- `%%capsulebin:favicon%%` — the `href` of `<link rel="icon">`: `/favicon.svg` or the configured
  favicon's URL.
- `%%capsulebin:source%%` — the `href` of the attribution link: the escaped `source_url`.
- `<!--capsulebin:logo-->` … built-in symbol SVG … `<!--/capsulebin:logo-->`: replaced as a whole
  by the logo image(s) when `logo` is set; otherwise only the two markers are removed and the
  symbol stays.
- `<!--capsulebin:wordmark-->` `<span class="cb-logo-name">%%capsulebin:name%%</span>`
  `<!--/capsulebin:wordmark-->`: removed as a whole with `hide_name`; otherwise only the markers go.

The regions span several lines (Prettier formats the templates); the server replaces whatever lies
between the markers. A start marker without its end is left as it is. No `<!--capsulebin:` marker
of a complete region reaches a served page.

**Order in `render()`:** (1) logo and wordmark regions — the images' `alt` is written as the name
token; (2) every token (prefix, default, name, favicon, source) in **one pass**, so a value that
was put in is never searched for tokens again, and text that only looks like a token start stays;
(3) legal footer links; then, in `Document::build`, legal title and content. Every operator value
is escaped and therefore holds no `<`, so it cannot form a marker for the later steps either.

The Vite dev server fills the three tokens with `DEFAULT_BRANDING` (name `CapsuleBin`,
`/favicon.svg`, the upstream repository) and leaves the marker regions in place; they are comments
and render nothing.

### Tests

| Test | Covers |
| --- | --- |
| `src/server/src/config/tests.rs` | Defaults, env equivalents, unknown key, every `validate` and file error (temporary files) |
| `src/server/src/branding.rs`, `src/server/src/api/assets.rs` (unit) | Signatures, hash length; `render()` with default, escaped and token-like values, logo, dark logo, incomplete regions; every template carries tokens and regions once |
| `src/server/tests/branding.rs` | `/branding/` serving (both asset modes, read once at startup, same file twice, generic `404`), and every page in `en`/`de` from its own test shells via `server.asset_dir`: default, escaped name, token-like name, logo, `hide_name`, all keys set, favicon alone |
| `src/server/tests/branding_acceptance.rs` | The real binary: every unknown key, refused value and every unusable or unreadable file stops `check` and `serve` with a non-zero exit code and a message naming the key (no file content); every key set from the environment reaches the started binary |
| `src/server/web/tests/shells.test.ts` | No literal `CapsuleBin` in the templates, the attribution as the last footer link and the only one without the prefix token, `shell.poweredBy` ends in `CapsuleBin` |
| `src/server/web/tests/dev-server.acceptance.test.ts` | The dev server leaves no token unfilled |
| `src/server/web/e2e/branding.e2e.ts` | Default instance (favicon, symbol, attribution in `en`/`de`); a branded instance started per worker through `e2e/instance.ts` with the fixtures in `e2e/fixtures/` (name with special characters, PNG logo, WebP dark logo, PNG favicon, custom `source_url`); logo visibility for both system preferences and both toggle directions, with and without a dark logo |
| `scripts/smoke.sh` | Every page links `/favicon.svg` and the upstream repository (the one allowed outgoing link); `/favicon.svg` is `image/svg+xml` with the security headers |

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
- **Default instance: as before, plus the attribution and the built-in favicon** — which also ends
  the `404` for the browser's favicon request. (20261120-branding-identity)
- **Tokens and marker regions in the shells, filled by the server** — the reproducible web build
  holds nothing instance-specific; the same mechanism as the language prefix and the legal links.
  The templates use `{{name}}`/`{{favicon}}`/`{{source}}`, which the generator turns into tokens,
  so the shell test's rule "every visible text from the catalogue" still holds and no template
  contains a literal `CapsuleBin`. (20261120-branding-identity)
- **All tokens filled in one pass, after the regions and before the legal content** — a single
  pass means no inserted value is scanned again, so an operator value that looks like a token stays
  literal; the legal links and legal text (also operator input) come after and are not scanned for
  tokens either. Filling the tokens one `replace` after another would let a name like
  `%%capsulebin:prefix%%` be expanded. (20261120-branding-identity)
- **Without a logo the region markers are removed, the symbol and wordmark stay** — no server
  marker reaches a served page. (20261120-branding-identity)
- **`alt=""` next to the visible name, `alt` = name with `hide_name`** — the name is read once
  either way; with `logo_dark` both images carry the same `alt`. (20261120-branding-identity)
- **Logo at `1.5rem` height, auto width, max `12rem`** — keeps wordmark logos readable without
  pushing the header tools off a narrow screen. (20261120-branding-identity)
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
