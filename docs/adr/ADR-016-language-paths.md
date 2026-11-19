# ADR-016 — Interface languages as path prefixes, shells generated per language

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

**Every page exists once per known language, under a path prefix:** `/en/`, `/de/p/{id}`,
`/de/manage/{id}`, `/de/report`, `/de/imprint` and so on. Every language the binary was built with
is offered, and `en` is the default; the operator does not choose.

- **A path without a prefix is redirected** (`302`, `Vary: Accept-Language`,
  `Cache-Control: no-store`) to the known language `Accept-Language` weighs highest, otherwise to
  `en`. The path is copied, not inspected, so the redirect of `/p/{id}` is the same for an
  existing and an unknown id; the fragment stays in the browser. Share and management links keep
  their prefix-less form.
- **A prefix that is not a known language is an unknown path** (the generic `404`), not a redirect.
- **The browser decides finally.** A classic head script (`language-init.js`, before
  `theme-init.js`) moves the page to the stored choice (`localStorage` `capsulebin.language`), else
  to the first offered browser language, else to the default — keeping query and fragment. The
  languages and the default reach it through a `<meta>` the shell generator writes into every
  shell at build time; the server inserts nothing for it.
- **Shells are generated at build time** from one set of templates and one catalogue per language
  (`src/server/web/src/i18n/<lang>.json`) into `dist/<lang>/<shell>.html`. Catalogue values are
  plain text and HTML-escaped. The scripts' texts are resolved at build time too: the web client
  is built once per language, each build carrying only the texts of its language that the scripts
  use, so a page never ships another language and more languages make no page larger. Code that
  does not depend on the language lives in chunks of its own, built identically for every
  language and shipped once under `/assets/`; a check on the build output enforces both. Numbers
  and dates follow the page language.
- **A language switch** next to the theme toggle stores the choice and loads the same address
  under the other prefix, replacing the history entry — except once a page must not be reloaded
  (a retrieval or deletion has started, or the create form holds a draft), where it only stores the
  choice.
- **`parseLink` accepts a known language segment** before `p`/`manage`, so a link copied from the
  address bar works in the CLI. To keep that unambiguous, `server.public_url` must not end in a
  known language code (startup error).
- **Footer link labels come from the catalogue** like every other shell text; the server only
  decides per `[legal]` whether each link appears and where it points.

## Rejected

- **English shells, catalogues loaded with `import()` behind a loading indicator.** Translates at
  runtime and shows a spinner on every page; the language paths make both unnecessary.
- **One shared bundle carrying every catalogue, selected by `<html lang>` at runtime.** Every page
  would ship every language and grow with each new one.
- **The server picks the language from a preference cookie, same URL for every language.** Needs a
  cookie and `Vary` on the pages. The choice is kept in `localStorage`, like the theme; there is no
  cookie.
- **Hiding the page until it is translated, or rendering every shell text with Lit.** Unnecessary
  once each language has its own shell.
- **The server fills the interface texts into the shells at markers.** Would soften ADR-004 and
  ADR-015 the most: the server would put far more than data into every page.
- **Redirecting a prefix that is not a known language.** A `404` keeps the set of page URLs small
  and unambiguous.
- **The operator chooses the offered languages and the default.** Costs configuration, startup
  validation, a languages marker the server fills into every shell and legal footers per
  configured language, for little value. With a fixed set the language list is known at build
  time.

## Consequences

- The route table of ADR-004 now exists once per language; every prefix-less page path is a
  redirect.
- The set of known languages is in three places — `KNOWN_LANGUAGES` in the server
  (`config/model.rs`) and in core (`languages.ts`), and the catalogue files — because the three are
  built separately. Tests fail when they differ. A new language is a catalogue plus two list
  entries; the web build picks up every catalogue file by itself, and every instance running that
  binary offers it. The default `en` is a constant in the server (`DEFAULT_LANGUAGE`) and in the web
  client (`default-language.ts`).
- The web build generates `languages × shells` documents and runs Vite once per language
  (`scripts/build.ts`); generator and builds are deterministic, so the reproducible-build check
  still holds. The Vite dev server and the unit tests load every catalogue, chosen by
  `<html lang>`.
- Caches and proxies in front of the server must keep `Vary: Accept-Language` on the redirect (it
  is also `no-store`).
- The CLI, the API's `problem+json` texts and the legal texts stay English for now; the pages map
  error codes to their own texts instead of showing the API's. Operators cannot replace the
  catalogues.
- A page that has started a retrieval or deletion, or holds a create draft, cannot change language
  in place; the choice applies from the next page.
- An instance cannot use a `public_url` whose path ends in `/en` or `/de` (or any later language
  code); such a sub-path has to be renamed.
