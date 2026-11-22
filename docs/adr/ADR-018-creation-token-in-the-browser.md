# ADR-018 — The creation token in the browser

**Status:** Accepted

## Context

`creation.mode` defaults to `token`: an instance reachable before it is configured must not be an
open write target. Until this decision the web create page never sent a creation token, so on a
token instance capsules could only be created with the CLI, and the page let a person write a
whole draft only to be told at the end that the server refuses it (`401 unauthorized`). `public`
mode made the page work but let anyone upload.

Constraints:

- **The server never sees a link secret, and pages are fixed bytes** (ADR-001, ADR-004, ADR-016).
  The server may put data into a shell only if it is the same for every request; `/p/{id}` must
  stay identical for every id.
- **No accounts, sessions or cookies.** The service is accountless; the CSP and Trusted Types
  (ADR-005) leave no room for inline script.
- **Creation is decided on the request head** (mode, `Authorization`) before a body byte is read,
  and create is idempotent per paste id (ADR-011).
- **Security as an option, not a loss of convenience**: a token typed once should keep working.

## Decision

**The create page takes a creation token from a field or an invitation link, keeps it in the
browser, and sends it only as the bearer of the create request.**

- **The mode is in the page.** The server fills the shell token `%%capsulebin:creation%%`
  (`public`, `token` or `disabled`) into `<meta name="capsulebin-creation">` of the create shell
  only, in the same single pass as the other server tokens. The value is fixed per run, so the
  page is the same for every request; no other shell carries it. A missing or unknown
  value counts as `public` on the page — the server still decides.
- **The page follows the mode before anything is written.** `disabled`: no form, only a notice
  that the instance accepts no new capsules. `token` without a token: no editor, a notice that a
  token is needed (the operator can give one or an invitation link), the token field and a
  "Remember in this browser" checkbox. `token` with a token: the editor, with a line "Token stored
  in this browser" or "Token for this page only" and a "Forget" button. `public`: unchanged, no
  token UI.
- **A token is 1 to 1024 characters of visible ASCII (U+0021–U+007E) after trimming.** The server
  reads the bearer as an HTTP header value; anything else could never match, so it is refused on
  the page and never used.
- **The invitation link is `<public_url>/#token=<token>`.** Like a share link's secret, the token
  sits in the fragment, which browsers never send. The page (`/` or `/<lang>/`) reads it once at
  boot, without percent-decoding (generated tokens are base64url), and removes it from the
  address bar and the current history entry with `history.replaceState` before anything is
  rendered — in every mode. Only in `token` mode is it used: stored, with a confirmation ("This
  browser can now create capsules"); an unusable value shows that the link cannot be used. Any
  other fragment is left alone. `capsulebin token` prints the link after the token and hash when
  `server.public_url` is set (by `--config` or the environment) and passes the rules `serve`
  applies to it.
- **Remembered by default.** The checkbox is checked; a remembered token is kept in
  `localStorage` under `capsulebin.creation_token` (one origin per instance, so per instance).
  Unchecked, the token is held by this page only and storage is not touched. An invitation link
  always stores. When storage is unavailable (blocked site data), the token is held for the page
  only and the line says so. "Forget" removes it from storage and from the page.
- **Sent only on create.** The token goes into `PasteApi`'s `creationToken`, which the core client
  sends as `Authorization: Bearer` on the create request and on nothing else, and only in `token`
  mode. It is never part of a URL or a body.
- **A refused token never costs the draft.** On `401 unauthorized` in `token` mode a stored token
  is removed at once; the error view offers the token field and "Create", which sends the *same*
  sealed capsule again: no new encryption, the same paste id. The refused request stored nothing
  (the server decides on the head), and an identical re-submission is idempotent (ADR-011).
  "Back to editing" after any failed create restores the draft as it was submitted (items,
  active item, title, expiry, burn, password).

## Rejected

- **A cookie or server-side session.** It would add state to the server, a cookie to the privacy
  notice and a credential the browser attaches to every request; the token would reach endpoints
  that have no use for it. A bearer added by the page to one request is narrower.
- **A policy endpoint for the mode.** An extra request before the page can decide what to show,
  and a new API surface; the mode is fixed per run, so the page can carry it. Revealing it
  discloses nothing new: an unauthorized create already answers `401` or `403`.
- **An admin UI to issue, list or revoke tokens.** It needs operator authentication on the web,
  which CapsuleBin does not have; `capsulebin token` and the configured hashes stay the only
  management.
- **Discovering the mode from a `401` after submitting.** The person would write a draft that can
  never be sent.
- **Not remembering by default.** Every visit would need the token again; protection is the
  opt-out (unchecked box, "Forget"), convenience the default.
- **The token in the query string or path of the invitation link.** The address is sent with the
  request, so the token would reach the reverse proxy's access log like any request path.
- **Re-encrypting after a refused token.** A new id and key for the same content, and the draft
  would have to be rebuilt; resending the sealed capsule is cheaper and idempotent.
- **Amending ADR-001.** The trust model for capsule content is unchanged; the browser now holds an
  operator credential, which is a separate decision.

## Consequences

- The browser holds a credential. A script running on the instance's origin can read it; the
  strict CSP, Trusted Types and the absence of third-party resources (ADR-005) are what protect
  it. Whoever has the browser profile or the invitation link has the token, until the operator
  removes its hash.
- `localStorage` holds a third key next to the theme and the language; the built-in privacy notice
  (English and German) names it, when it is stored, when it is sent and how to remove it. This
  widens ADR-005's rule that only the theme and the language are stored; ADR-005 names the token
  among the stored values.
- The create shell carries a server token the other shells do not; the Vite dev server fills it
  with `public`, the mode of the development configuration.
- `capsulebin token` reads the configuration, but validates only `server.public_url`: it keeps
  working without any configuration. An explicit `--config` that cannot be loaded is an error and
  prints no token; a `public_url` that breaks a rule (and, without `--config`, an environment that
  cannot be loaded) is a note on stderr that never repeats the value, and the token and hash are
  still printed. The new lines come after the existing ones, so scripts that read the token from
  line 2 and the `sha256:` line keep working.
- The CLI does not accept an invitation link as a token source.
