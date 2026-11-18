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
- A report never carries a key: the report page strips the fragment from a pasted link before
  sending, and the server refuses text containing `#v1.`.
- Text with control characters is refused, so a report shown in the operator's terminal cannot carry
  escape sequences.
- Reports have their own rate-limit class (`rate_limits.report`, which may be set per hour).
- `GET /api/v1/reports` describes the policy (enabled, contact, reason codes, limits) for the page.
- `abuse.enabled = false` turns the endpoint off (`404 reports_disabled`); moderation keeps working.

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
- **Storing every report.** Lets anyone flood the queue with reports about random ids.
- **An admin web UI.** A second authenticated surface on a server that otherwise has no accounts.
- **Evidence uploads, content hashes, automatic classification.** Incompatible with the trust model
  or trivially bypassed by a different client.

## Consequences

- An operator learns about abuse only through reports or other channels; there is no proactive
  detection, by design.
- A reporter receives no feedback about what happened. `abuse.contact` offers an out-of-band channel
  for follow-up.
- The existence check is a tiny timing difference between a stored and an unstored report (one row
  written or not). The rate limit on the endpoint and the 128-bit id space keep this from being a
  practical oracle.
- Deleting a capsule through moderation is as final as the creator's own delete; backups may still
  contain the ciphertext (ADR-002).
