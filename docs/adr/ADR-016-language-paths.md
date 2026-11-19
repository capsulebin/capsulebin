# ADR-016 — Interface languages in the path, the default language without a prefix

**Status:** Accepted

## Context

CapsuleBin's interface was English only. It gets a German interface, and further languages should
need little more than a catalogue. Three things constrain how:

- **Share and management links already exist and must keep working.** They are
  `<origin>/p/<id>#v1.…` and `<origin>/manage/<id>#v1.…`; the fragment holds the key and never
  reaches the server.
- **The shells are fixed bytes from the binary** (ADR-004, ADR-015). `GET /p/{id}` answers the same
  for every id, and the server inserts only data that is the same for every request.
- **The CSP and the reproducible web build** (ADR-005) leave no room for inline scripts or for
  build output that differs between runs.

## Decision

**Every page exists once per known language. The default language has no path prefix, every other
language lives under `/<lang>/`:** with the default `en`, `/p/{id}` is the English view page and
`/de/p/{id}` the German one; likewise `/`, `/manage/{id}`, `/report`, `/imprint`, `/privacy`,
`/terms`.

- **Every known language is always offered; the operator chooses only the default.**
  `[i18n] default_language` (default `en`) is the only key; an unknown code or any other key in
  `[i18n]` is a startup error.
- **The prefix of the default language is redirected away** (`302`, `Cache-Control: no-store`):
  `/en/p/{id}` → `/p/{id}`, `/en` → `/`, query kept. The path is copied, not inspected, so the
  redirect is the same for an existing and an unknown id; the fragment stays in the browser.
  `/<lang>` without a slash redirects to `/<lang>/`.
- **The server does not negotiate.** It never reads `Accept-Language` and sends no `Vary` for it;
  the path alone decides which shell is served. Share and management links keep their prefix-less
  form and open in the default language.
- **A prefix that is not a known language is an unknown path** (the generic `404`), not a redirect.
- **The browser decides the language.** A classic head script (`language-init.js`, before
  `theme-init.js`) moves the page to the stored choice (`localStorage` `capsulebin.language`), else
  to the first offered browser language, else to the default — keeping query and fragment. A path
  without a known language prefix counts as the default language. The offered languages and the
  default reach the script through a `<meta>` in every shell.
- **Shells are generated at build time** from one set of templates and one catalogue per language
  (`src/server/web/src/i18n/<lang>.json`) into `dist/<lang>/<shell>.html`. Catalogue values are
  plain text and HTML-escaped. The build does not know the default language: it writes the tokens
  `%%capsulebin:prefix%%` in front of internal links and `%%capsulebin:default%%` into the meta
  element, and the server fills them in from its configuration (no prefix for the default
  language, `/<lang>` otherwise) — data that is the same for every request.
- **One web build; each shell loads the catalogue module of its language**
  (`src/i18n/register-<lang>.ts`) before the page script. The page scripts hold no catalogue and
  are shared by all languages, so a page never loads another language's texts and more languages
  make no page larger. Numbers and dates follow the page language.
- **A language switch** next to the theme toggle stores the choice and loads the same address in
  the chosen language, replacing the history entry — except once a page must not be reloaded (a
  retrieval or deletion has started, or the create or report form holds any input), where it only
  stores the choice and says so **in the chosen language**. That notice is the one text every page
  holds in every language. The other way round, a page the switch is already loading in another
  language starts no retrieval or deletion.
- **`parseLink` accepts a known language segment** before `p`/`manage`, so a link copied from the
  address bar works in the CLI. To keep that unambiguous, no link is created on a base whose path
  ends in a known language code: the server refuses it as `server.public_url` (startup error) and
  the CLI's `put` as `--server`. Both apply one rule — the last non-empty segment of the path as a
  URL parser normalizes it, compared exactly — and are tested against the same vectors.
- **Footer link labels come from the catalogue** like every other shell text; the server only
  decides per `[legal]` whether each link appears and where it points.

## Rejected

- **Every language under a prefix, prefix-less paths redirected by `Accept-Language`.** Every
  share link would start with a redirect; the server would need an `Accept-Language` parser and
  `Vary: Accept-Language` on the redirect, and every cache in front of it would have to honour
  that. The head script decides with more knowledge (the stored choice) anyway. Without a prefix
  for the default language, URLs are shorter and share links land directly on a page.
- **One web build per language, each with its catalogue resolved at build time.** Keeps a page on
  its own language too, but needs a build script running Vite once per language, a search for the
  keys the scripts name, splitting language-free code into chunks of its own, a check of the
  output and a separate all-language catalogue module for the dev server and tests — more
  machinery than the property needs.
- **One shared bundle carrying every catalogue, selected by `<html lang>` at runtime.** Every page
  would ship every language and grow with each new one.
- **The operator chooses the list of offered languages.** Costs configuration, startup
  validation, a languages marker the server fills into every shell and legal footers per
  configured language, for little value. With every known language always offered, the list is
  known at build time; only the default is configuration.
- **English shells, catalogues loaded with `import()` behind a loading indicator.** Translates at
  runtime and shows a spinner on every page; the language paths make both unnecessary.
- **The server picks the language from a preference cookie, same URL for every language.** Needs a
  cookie and `Vary` on the pages. The choice is kept in `localStorage`, like the theme; there is no
  cookie.
- **Hiding the page until it is translated, or rendering every shell text with Lit.** Unnecessary
  once each language has its own shell.
- **The server fills the interface texts into the shells at markers.** Would soften ADR-004 and
  ADR-015 the most: the server would put far more than data into every page.
- **Redirecting a prefix that is not a known language.** A `404` keeps the set of page URLs small
  and unambiguous.

## Consequences

- The route table of ADR-004 exists once per language: prefix-less for the default language,
  prefixed for the others, plus the redirect from the default language's prefix.
- Which paths are pages and which are redirects depends on `default_language`. Changing it on an
  instance moves the previous default's pages under its prefix; prefix-less links then open in the
  new default language until the head script moves the browser to its own.
- A recipient whose browser prefers another offered language gets one client-side
  `location.replace` before the page is shown; without JavaScript the page stays in the language
  of its path.
- The set of known languages is in three places — `KNOWN_LANGUAGES` in the server
  (`config/model.rs`) and in core (`languages.ts`), and the catalogue files — because the three are
  built separately. Tests fail when they differ. A new language is a catalogue file, its
  `register-<lang>.ts`, an entry in `language-next-page.ts` and two list entries; every instance
  running that binary offers it.
- The web build generates `languages × shells` documents as inputs of one Vite build; the
  generator is deterministic, so the reproducible-build check still holds. The Vite dev server
  fills the tokens itself, with the default `en`.
- No cache or proxy in front of the server has to treat page responses specially: none varies by
  a request header.
- The CLI, the API's `problem+json` texts and the legal texts stay English for now; the pages map
  error codes to their own texts instead of showing the API's. Operators cannot replace the
  catalogues.
- A page that has started a retrieval or deletion, or holds a create draft or a started report,
  cannot change language in place; the choice applies from the next page.
- An instance cannot use a `public_url` whose path ends in `/en` or `/de` (or any later language
  code), also not through dot segments (`/team/de/.`); such a sub-path has to be renamed. Links
  issued under such a base before are read by the CLI as carrying a language segment (see the
  upgrade note in `docs/operations.md`).
