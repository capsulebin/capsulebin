# Privacy notice: what an instance processes

CapsuleBin has a built-in privacy notice in English and German (`/privacy`, and `/de/privacy` or
`/en/privacy` for the language that is not the default) that an instance serves once
`[legal.operator]` is configured (see [Legal pages](operations.md#legal-pages)). Both languages
state the same facts under the same sections; the table names the English headings. This page
lists what that notice states and where it comes from, so that an operator can check it against
their deployment, add what it cannot know, or replace it with their own.

It is not legal advice. The built-in text is a careful default for an operator in the EU under the
GDPR; who has to publish what, and on which legal basis, depends on the operator and the
jurisdiction, and the operator stays responsible for what their instance publishes.

## What the built-in notice covers

| Section | Content | Taken from |
| --- | --- | --- |
| Controller | The operator named in the imprint; a hosting provider as processor (Art. 28 GDPR) | `[legal.operator]`, `[legal.hosting]` |
| What this service cannot see | Content, title, file names, key and password never reach the server | Fixed by the software |
| Capsules | The stored data, the legal basis, when a capsule is deleted, and that afterwards only its identifier is kept so it is never used for another capsule | `pastes.maximum_expiry`, `pastes.allow_permanent`, `purge.*` |
| Creation tokens | That the operator can keep issued tokens in the database: per token only a hash, a label, the creation and revocation time and the day of last use; capsules are not linked to the token that created them; a revoked token's entry is kept so it stays revoked; legal basis Art. 6(1)(f) GDPR | Fixed by the software (stated whether or not the instance uses database tokens) |
| IP addresses and server logs | Whether the client address is processed in memory for rate limits; what the log contains | `rate_limits.*` |
| Reports | What a report stores, the legal basis, how long it is kept | `abuse.enabled`, `abuse.report_retention` |
| Cookies and third parties | No cookies; the theme and language choice in local storage, and a creation token once one is entered or an invitation link opened; no third-party requests | Fixed by the software |
| Your rights | Data subject rights; how to find a capsule by its id without sending the key | Fixed by the software |

## What the operator has to add

The software cannot know what runs around it. Typical additions, as Markdown in
`[legal.privacy] extra` or `extra_file` (inserted before "Your rights", German "Deine Rechte"):

- **The reverse proxy's access log.** Usually the largest item: it records client IP addresses and
  full request paths, including capsule ids. State what is logged and for how long.
- **Backups.** A backup (`capsulebin backup`) contains capsules, reports and creation token
  records, including ones deleted after it was taken. State how long backups are kept, since a
  burned or deleted capsule can survive in one until then.
- **Monitoring, hosting, a data protection officer, the competent supervisory authority**, where
  they apply.

An operator who prefers their own text replaces the notice with `[legal.privacy] text`, `file` or
`url`; the table below is the basis for it.

These settings are the default language's. The other language's pages use `[legal.privacy]` as
well (its `extra` or own text shown there marked with the default language) unless
`[legal.<lang>.privacy]` is set, e.g. `[legal.de.privacy] extra` with the German additions; that
table replaces `[legal.privacy]` as a whole for those pages, so an empty one gives the plain
built-in notice.

## The facts in detail

What the server never receives:

- **The content of a capsule**: text, code, Markdown and files are encrypted in the browser or CLI
  before upload. The same holds for file names, media types, the number of items and the title.
- **The key**: the link secret lives in the URL fragment (`#…`), which browsers do not send.
- **The password** of a password-protected capsule, and anything derived from it that would allow
  decryption.

What it processes:

| Data | Purpose | Kept |
| --- | --- | --- |
| **Encrypted capsule** (ciphertext, public header with the encryption parameters, size) | Delivering the capsule to whoever holds the link | Until the first purge run (daily) after it expires, until it is deleted by its creator or by moderation, is retrieved once (burn after reading) or is removed by the retention rules; the ciphertext is overwritten on deletion (`secure_delete`) |
| **Capsule metadata**: id, hashes of the access secrets, creation and expiry time, time of the last retrieval (rounded to `purge.retrieval_timestamp_granularity`), burn flag and time of consumption, size, moderation status with its reason code and time | Lifecycle, access control, retention | With the capsule; a deleted or burned capsule leaves this row without ciphertext until a retention rule removes it |
| **Identifier of a removed capsule** (the id alone) | Refusing to use it for another capsule, so an old link can never open different content | Permanently, also across a restore that can read the replaced database; nothing else about the capsule is kept |
| **Client IP address** | Rate limiting, if a limit keyed by client is configured (`rate_limits.*`) | In memory only, never written to disk or logs; gone at the latest on restart |
| **Abuse reports**: capsule id, reason, description, optional contact | Handling the report (notice and action) | Open reports until closed; closed ones for `abuse.report_retention` (default 90 days) |
| **Moderation records**: capsule id, action, reason, operator note | Accountability for blocking and deleting | `abuse.report_retention` (default 90 days) |
| **Creation token hashes** in the configuration (`creation.token_hashes`) | Deciding who may create capsules | As long as the operator configures them |
| **Creation tokens in the database**: hash of the token, the operator's label, creation and revocation time, the day (not the time) of the last create it authorized | Deciding who may create capsules; telling a token in use from an abandoned one | Active tokens until revoked; a revoked token's entry permanently, so that it stays revoked, also across a restore. No capsule is linked to a token |

The server's own log contains the request method, the path with ids replaced by placeholders, the
status and the latency - no IP addresses, capsule ids, tokens, bodies or report text.

In the browser there are no cookies; the only stored values are the colour theme and the interface
language, in `localStorage` (`capsulebin.theme`, `capsulebin.language`), and on an instance in
`token` mode the creation token (`capsulebin.creation_token`): stored only after the user opens an
invitation link or enters a token with "Remember in this browser" checked, sent to the server only
as the bearer of a create request, removed by "Forget" on the create page or when the server
refuses it. Pages load nothing from third parties, and images referenced in Markdown are shown as
links. Every page sends `Referrer-Policy: no-referrer`.
