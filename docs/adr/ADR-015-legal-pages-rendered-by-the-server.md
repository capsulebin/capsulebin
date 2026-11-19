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
- The privacy notice and the terms have **built-in texts**, in English like the rest of the
  interface; translations come with the localisation of the whole service. The privacy notice is
  filled in from the running configuration - retention
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
  update them (`legal/defaults.rs`) and [privacy-notice.md](../privacy-notice.md). They are a careful
  default, not legal advice; the operator stays responsible for what the instance publishes.
- A changed text takes effect on restart, like every other setting.
