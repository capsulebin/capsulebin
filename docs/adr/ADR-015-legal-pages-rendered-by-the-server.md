# ADR-015 — Legal pages: operator text rendered by the server

**Status:** Accepted

## Context

An instance that is open to the public in the EU usually has to publish an imprint and a privacy
notice, and an operator wants terms of use. Not every operator hosts such pages elsewhere; a
self-contained instance should be able to serve them itself. Hosting and operating can be separate
parties: the imprint then names the operator first and the hosting provider second, and the privacy
notice names the hosting provider as processor.

Most of a privacy notice for CapsuleBin is fixed by the software: what the server never sees, what
it stores, how long, and that it sets no cookies. An operator who has to write that themselves will
get it wrong or out of date, for example when a retention period in the configuration changes.

Until now every byte the server put into a page came from the binary: the HTML shells, scripts and
styles are embedded at build time, and the server inserts nothing (ADR-004). That is part of the
answer to "can the server change what runs in my browser?" (ADR-001): only by shipping a different
binary.

## Decision

**`[legal]` configures an imprint, a privacy notice and terms of use**, served by CapsuleBin at
`/imprint`, `/privacy` and `/terms` and linked from the footer of every page:

- The imprint is the operator's Markdown (`[legal.operator]`, as `text` or `file`), followed by a
  "Hosting" block from `[legal.hosting]`. With `url` instead, the footer links to a page elsewhere.
- The privacy notice and the terms have **built-in texts**, in every interface language (see the
  amendment below). The privacy notice is filled in from the running configuration - retention
  rules, report retention, whether limits use the client address, the hosting provider - so it
  stays true when the configuration changes. `extra` adds what the software cannot know (the
  reverse proxy's logs, backups); `text`, `file` or `url` replace a text entirely.
- Built-in texts need an operator: a privacy notice without a controller is worse than none.
  Without `[legal.operator]` there are no legal pages and no footer links.

**The server renders the texts, at startup.** Files are read and Markdown is rendered once
(`legal.rs`, `pulldown-cmark`); a request copies the finished HTML into the `legal.html` shell at
HTML-comment markers, and the footer links into every shell. The rendered Markdown contains **no
raw HTML** (it is shown as text) and **no images** (they become links), and links only to `http(s)`,
`mailto`, `tel`, paths on this host and fragments. `url` values are limited to absolute `http(s)`
URLs and single-slash paths. Startup fails on a missing file, an empty or oversized text, or a
combination that cannot be meant.

**What stays true:** code comes only from the binary. The operator's text is data in a page, not a
script, stylesheet or shell; the CSP (`script-src 'self'`, Trusted Types) would refuse an inline
script even if one got through. The shells still do not depend on the path id, and capsule content
is never rendered by the server.

**Amendment (ADR-016, legal texts per language):** every legal page exists in every interface
language, at `/imprint`, `/privacy`, `/terms` for the default language and under `/{lang}/` for
every other ([i18n.md](../i18n.md#legal-pages) has the rules in detail):

- The built-in privacy notice and terms exist in English and German, with the same facts, headings
  and links, and are always served in the page language. Their links carry the page language's
  prefix.
- `[legal.*]` is the text of `i18n.default_language`. `[legal.<lang>.operator|hosting|privacy|terms]`
  gives a section for another known language, with the same keys and rules. For the pages of a
  language, each section is its own `[legal.<lang>.*]` table as a whole if there is one, otherwise
  the `[legal.*]` one - a fallback rather than a startup error, so an operator need not write every
  text twice; an empty `[legal.de.privacy]` therefore means the German built-in notice.
- `[legal.<default language>]`, a table that is not a known language, and a translated `operator`
  or `hosting` without its `[legal.*]` counterpart are startup errors: the latter keeps the imprint
  and the named processors the same in every language.
- Every block of a page (operator text, hosting block, the operator's `extra`, a replacement text)
  carries the language it was configured in and is marked with `lang` where that differs from the
  page; built-in texts are never marked.
- Title, header and `<h1>` are the footer labels from the interface catalogue ("Privacy", "Terms",
  "Datenschutz", …), which the server reads from the page's own footer marker in the shell. The
  server holds no page titles, and the `<h1>` is part of the shell, outside the rendered article.

## Rejected

- **Links only (`imprint_url`, `privacy_url`, `terms_url`).** The first version. Forces every
  operator to host the pages elsewhere, and leaves the privacy notice - most of which the software
  determines - to each operator.
- **Rendering in the browser with the existing Markdown pipeline.** No new dependency, but the
  imprint would need JavaScript, and the texts would have to be fetched from an API endpoint on
  every page view.
- **Plain text instead of Markdown.** Enough for an imprint, not for a privacy notice with sections
  and lists.
- **Full terms with liability clauses as a default.** Liability clauses depend on the jurisdiction
  and are easy to get wrong; a wrong default is worse than none. The built-in terms are short rules
  of use; an operator who needs more sets their own.

## Consequences

- The server now inserts content into a page. It is the operator's, rendered with a restricted
  Markdown subset, and covered by tests (raw HTML, unsafe links, images, escaping) and by the
  browser tests under the CSP guard.
- `pulldown-cmark` is a new server dependency, used for nothing else.
- The built-in texts are part of the product: a change to what the server stores or logs must
  update them in every language (`legal/defaults/{en,de}.rs`) and
  [privacy-notice.md](../privacy-notice.md). They are a careful
  default, not legal advice; the operator stays responsible for what the instance publishes.
- A changed text takes effect on restart, like every other setting.
