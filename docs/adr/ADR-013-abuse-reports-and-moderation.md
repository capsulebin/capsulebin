# ADR-013 — Abuse reports and moderation without decryption

**Status:** Accepted

## Context

Zero knowledge rules out inspecting content, but not acting on a concrete capsule. Before an instance
is run publicly, someone must be able to report a capsule and the operator must be able to block or
remove it — without decrypting anything, and without weakening the properties the rest of the
system relies on:

- Unknown, expired, disabled, deleted and wrong-capability answer identically (ADR-001).
- A report form is public. If it confirmed whether an id exists, it would be an oracle; if it stored
  everything, anyone could fill the operator's queue with reports about random ids.
- Reports carry free text from the public — descriptions, contact addresses. That is personal data.

A common feature of moderated services is a "tombstone notice" that tells a recipient a capsule
was removed by moderation.

## Decision

**Reporting.** `POST /api/v1/reports` takes `paste_id`, a `reason` code, a `description` and an
optional `contact`.

- Every well-formed report gets the same `202 {"status":"received"}`, whether or not the paste
  exists. Only reports about a paste that currently exists — active and not expired or consumed, or
  disabled — are stored. Existence check and insert are one SQL statement.
- Open reports (status `open` or `reviewing`) are capped: a report is stored only while its paste
  has fewer than `abuse.max_open_reports_per_paste` (default 10) and the instance fewer than
  `abuse.max_open_reports` (default 1000) open reports. Existence check, both counts and insert
  are one statement; SQLite serializes writers, so the ceilings hold under concurrency. A report
  above a ceiling is dropped and gets the same `202`. Closing a report frees its place. Both
  ceilings are integers of at least 1: `0` would drop every report while the form still says
  "received"; `abuse.enabled = false` is the way to stop reports. The ceilings are not published
  in `GET /api/v1/reports`.
- The operator sees a full queue: the server logs `report not stored: open report limit reached`
  at most once a minute (a flood must not flood the log), without paste id, text or contact; and
  `capsulebin reports list` starts with the number of open reports and both ceilings, and says when
  new reports are not stored. `--json` stays a plain array for scripts.
- A report never carries a key: the report page strips the fragment from a pasted link in the id
  field, and it refuses a description or contact containing `#v1.` before anything is sent — no
  request, a message, focus on the first field concerned, the text left for the reporter to edit.
  The server refuses such text too, for clients other than the report page. Both use the same
  rule (`#` followed by the fragment prefix, case-sensitive; `containsKeyFragment` in the core,
  `check_text` on the server), so the page never sends a text the server would refuse for a key.
  A bare key without `#` is not detected on either side. The page refuses rather than cutting the
  key out: removing text from free text is guesswork, and the reporter would not see what was sent.
- Control characters are refused (in the description all but line breaks and tabs, in the contact
  all), so a report shown in the operator's terminal cannot carry escape sequences.
- Reports have their own rate-limit class (`rate_limits.report`, which may be set per hour).
- `GET /api/v1/reports` describes the policy (enabled, contact, reason codes, limits) for the page.
- `abuse.enabled = false` turns report submission off (`POST` answers `404 reports_disabled` before
  the body is read; `GET` still describes the policy with `enabled: false`); moderation keeps
  working.
- The request body is limited to 32 KiB: a report holds at most 4000 bytes of description and 254
  of contact, below 26 KB even with every character escaped as `\uXXXX`, so no valid report is
  refused.

**Moderation.** There is no admin web interface. The operator works with the binary:

- `capsulebin reports list|show|review|close` — the queue, with the current status of each paste.
- `capsulebin moderation disable|delete|restore|status <paste-id>` — disable keeps the payload and
  is reversible; delete drops it and is final; restore applies only to disabled pastes. Every action
  is recorded in `moderation_actions` with reason and optional note, in the same transaction.

**No tombstone notice.** A disabled or deleted capsule answers exactly like an unknown one, for every
caller, including one holding the link.

**Retention.** `abuse.report_retention` (default 90 days) removes closed reports and moderation
records older than that, in the daily job and in `prune`. Open and reviewing reports stay until an
operator closes them. Report content is never logged.

## Rejected

- **A tombstone notice for moderated capsules.** It would tell anyone holding a link — or guessing
  one — that a capsule existed and was acted upon, and it breaks the identical answers for a convenience.
- **Rejecting reports about unknown ids.** Makes the endpoint an existence oracle.
- **Storing every report.** Lets anyone flood the queue with reports about random ids — and, with
  one known id, grow the queue and the database without bound.
- **Coalescing reports above the ceiling into a counter.** The first reports per capsule keep their
  own text and contact as independent evidence; a counter of further ones adds no evidence and
  would need a schema change. Dropping them needs none.
- **Telling the reporter that a ceiling was reached.** The answer would differ by whether the id
  exists, an oracle again.
- **An admin web UI.** A second authenticated surface on a server that otherwise has no accounts.
- **Evidence uploads, content hashes, automatic classification.** Incompatible with the trust model
  or trivially bypassed by a different client.

## Consequences

- An operator learns about abuse only through reports or other channels; there is no proactive
  detection, by design.
- A reporter receives no feedback about what happened. `abuse.contact` offers an out-of-band channel
  for follow-up.
- Once a capsule or the instance has reached its ceiling, further reports — genuine ones included —
  are lost until an operator closes open ones. Anyone who knows an id can fill that capsule's
  ceiling with junk; the rate limit and the operator's queue work are the answer, not more storage.
- The existence check is a tiny timing difference between a stored and an unstored report (one row
  written, or none and one more read that only decides whether to log the ceiling warning). The
  rate limit on the endpoint and the 128-bit id space keep this from being a practical oracle.
- Deleting a capsule through moderation is as final as the creator's own delete; backups may still
  contain the ciphertext (ADR-002).
