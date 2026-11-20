# Threat model

What CapsuleBin protects, against whom, how — and what it explicitly does not. The binding
decisions behind it are [ADR-001](adr/ADR-001-trust-model.md) (zero knowledge) and
[ADR-013](adr/ADR-013-abuse-reports-and-moderation.md) (abuse handling); the cryptography is
specified in [crypto-profile.md](crypto-profile.md).

## Assets

| Asset | Where it lives | Who may know it |
| --- | --- | --- |
| Plaintext (content, file names, media types, item count, title) | Only in the creating and the receiving client | Creator and recipients |
| Link secret | URL fragment of the share link (`#v1.…`) | Creator and recipients |
| Password | The creator's and recipients' heads | Creator and recipients |
| Content key | Derived in the client, never stored | Nobody beyond the clients |
| Reveal token | Derived in the client, sent on retrieval | Clients; the server sees it once per retrieval and stores only `SHA-256` |
| Delete secret | Fragment of the management link | Creator only |
| Creation token | Operator, configured as a domain-separated hash | Operator and the people it is issued to |
| Ciphertext, public header, verifiers | SQLite database, backups | Server and operator — useless without the link secret |
| Lifecycle metadata (size, timestamps, burn flag) | SQLite database | Server and operator |
| Abuse reports (description, optional contact) | SQLite database, backups | Operator — personal data |
| Client IP addresses | Rate-limiter memory only, never persisted | Server process |

## Trust boundary

The server is trusted to **store, enforce limits and serve the application** — not with content.
Everything left of the boundary sees plaintext; everything right of it must not.

```
 creator client ─┐                          ┌──────────── server ─────────────┐
                 │  ciphertext, header,     │ Axum: limits, capabilities,     │
                 ├─ verifiers ─────────────▶│ lifecycle, assets               │
 recipient ──────┤                          │ SQLite: ciphertext + metadata   │
                 │◀─ ciphertext on proof ───│ operator CLI: backup, moderation│
                 │   of the reveal token    └─────────────────────────────────┘
   plaintext, keys, link secret, password never cross this line
```

## Adversaries and controls

| Adversary / threat | Controls | Verified by | Residual risk |
| --- | --- | --- | --- |
| **Database or backup leak** | AES-256-GCM ciphertext; no keys, secrets or content metadata stored; verifiers only (`SHA-256`, domain-separated); backups compacted so freed pages of deleted pastes are not copied | `db_no_plaintext.rs` (CI gate), backup tests, `smoke.sh` greps DB and backup for secrets | Padded sizes (order of magnitude), timestamps and lifecycle flags are visible. Reports contain personal data. |
| **Curious or compelled operator** | Same as above; operator tools work on ids and metadata only, never decrypt | — (by construction) | Operator sees who creates when (via logs on the proxy, not in CapsuleBin), and sizes. |
| **Malicious or compromised server shipping modified JavaScript** | Embedded assets, strict CSP, no third-party resources; the CLI as a client that does not depend on served code; reproducible web assets, so a tag's client can be rebuilt and compared with what a server ships | CSP asserted on every response (`smoke.sh`); two web builds compared byte for byte (`scripts/reproducible-web.sh`, CI) | **Not prevented.** A compromised server can exfiltrate keys from web users. This is the model's documented limit (ADR-001). |
| **Network observer** | HTTPS (at the reverse proxy, with HSTS); the fragment is never sent; `Referrer-Policy: no-referrer`; ciphertext padded (PADMÉ) | Security headers in `smoke.sh`, padding tests and golden vectors | Server IP, timing and padded sizes are observable. |
| **Link preview bots and scanners** | A share link `GET /p/{id}` (and `/{lang}/p/{id}` in every other language) serves the same static shell for every id; a path under the default language's prefix answers a `302` to the path without it, copied without inspecting the id; the server never reads `Accept-Language`; nothing looks the id up, retrieval is an explicit `POST` with the reveal token | `view_shell_is_identical_for_every_id`, `the_prefix_of_the_default_language_is_redirected_away`, `smoke.sh` | Scanners that execute JavaScript and hold the full link (fragment included) can consume a burn capsule. |
| **Someone who learns a paste id** (logs, history, proxy, report form) | Descriptor, retrieval and delete each need a capability derived from the link (describe token, reveal token, delete secret); ids are 128-bit random; ids are redacted from the access log | `the_descriptor_needs_a_capability`, `logging.rs` (CI gate), property test on `redact_path` | None: without the link the id answers like an unknown one. |
| **Existence probing** (does id X exist?) | Every endpoint taking an id requires a capability; unknown, expired, consumed, disabled, deleted and wrong-capability answer identically; the check runs against a dummy verifier for unknown ids; reports are answered identically whether or not the id exists | `the_descriptor_needs_a_capability`, `wrong_token_and_unknown_id_are_indistinguishable`, `moderated_pastes_look_exactly_like_unknown_ones`, report tests, `smoke.sh` | Small timing differences (e.g. a stored vs. an unstored report); mitigated by rate limits and the id space. |
| **Brute force of ids, reveal tokens or delete secrets** | 128-/256-bit randomness; failed attempts are optionally throttled per client before any lookup | `tests/rate_limits.rs`, `smoke.sh` over a real socket | None practical at these key sizes. |
| **Online password guessing** (link leaked, password not) | Every guess needs a retrieve with a reveal token derived from it; retrieves of one paste that prove the link but fail the reveal token are limited from any address (`invalid_capability_per_paste`, on by default), checked and charged atomically so parallel guesses cannot exceed the burst; keyed on the id the retrieve handler looked the paste up by, so every spelling of the id in the URI (percent-encoded in upper- or lower-case hex, fully or partly) shares one budget | `tests/rate_limits.rs`, `parallel_password_guesses_cannot_exceed_the_per_paste_burst`, `password_attempts_share_one_budget_across_id_spellings`, `password_attempts_across_id_spellings_are_limited_under_concurrency` | Roughly 14 000 guesses a day per capsule at the default; weak passwords still fall eventually. |
| **Blocking a capsule by its id** (id leaked, link not) | The per-paste limit counts only retrieves that prove the link with the describe token, and is checked only after that proof; describe and delete never touch it; this holds for every spelling of the id | `a_caller_with_only_the_id_cannot_block_a_capsule`, `a_caller_with_only_the_id_cannot_drain_the_budget_in_any_spelling`, `password_attempts_on_one_paste_are_limited_by_default_from_any_address` | A link holder can keep the password limit drained - but could retrieve or burn the capsule anyway. |
| **Offline password guessing** | The link secret is a mandatory second factor; PBKDF2 (600 000 iterations, versioned); no password verifier in the public header; ciphertext only after a correct reveal token | Golden vectors, `public_header` tests | Someone who has the link **and** the ciphertext can guess offline. Weak passwords fall. |
| **XSS through Markdown or code** | Only Markdown renders HTML: marked → DOMPurify allowlist → link policy; highlight.js escapes; strict CSP; Trusted Types required; no inline scripts or styles | `xss.test.ts`, `trusted-html-usage.test.ts`, CSP in `smoke.sh`; browser tests (`src/server/web/e2e`) in Chromium, Firefox and WebKit with hostile Markdown, an XSS canary and every CSP/Trusted Types violation failing the test | Sanitizer or browser bugs. |
| **Script through the operator's legal texts** (`[legal]`, ADR-015) | The operator is trusted, but the text lands on every visitor's screen: rendered at startup without raw HTML or images, links limited to http(s), mailto, tel and local paths, `url` values to absolute http(s) URLs and single-slash paths; strict CSP on the pages | `legal/markdown.rs` tests (raw HTML, unsafe links, images, escaping), `legal/tests.rs` (URL allowlist), `legal.e2e.ts` under the CSP guard | A misleading but harmless text; the operator controls the instance anyway. |
| **Malicious capsule** (crafted by a creator to attack the recipient's client) | AES-GCM authenticates the whole container; strict, length-checked parser that never allocates from unchecked sizes; files are only downloaded, never previewed | Property tests on the decoder (arbitrary, corrupted, truncated input) | Downloaded files are as dangerous as any download. |
| **Burn races** (two recipients, or a scanner and a recipient) | `BEGIN IMMEDIATE` burn transaction with an affected-row check; rate-limited and unauthorized requests never reach it | Repository and HTTP-level race tests, 20-connection race in `smoke.sh` | "Burned" means first *authorized retrieval*, not "read by a human". |
| **I/O amplification by unauthorized requests** | Descriptor, retrieve and delete check capabilities against metadata only; the payload is read after authorization | `metadata_lookups_do_not_read_the_payload` | — |
| **Storage and resource abuse** | Token creation mode by default; body and plaintext limits; per-client request rates, daily byte quota, concurrent-upload cap; retention purge | `tests/rate_limits.rs`, `api.rs` size tests | Public-mode instances remain abusable within the limits. |
| **Illegal content** | Notice and action: report by id, operator disables or deletes without decrypting; audit trail | `tests/abuse.rs`, `smoke.sh` | No preventive moderation under zero knowledge. |
| **A key pasted into a report** (a reporter copies the whole share or management link into the description or contact) | The report page sends only the id from the id field (the fragment is cut off first) and refuses a description or contact containing `#v1.` before any request exists: no `POST`, a message in the page language, focus on the first field concerned, the text left for the reporter to correct. The detection rule is one core function (`containsKeyFragment` in `src/core/src/links.ts`), exactly the server's rule; the server refuses such text too, for other clients | `containsKeyFragment` tests (`src/core/tests/links.test.ts`), `report-page.test.ts` (no `POST`, message, focus, text kept), `the report form refuses a link with its key in the description or contact` (`capsule.e2e.ts`, requests counted in the browser), `a_report_containing_a_key_is_refused` and `accepted_reports_carry_no_key_and_no_control_characters` (server) | A bare key without `#` (`v1.…` or the base64url secret alone) is not detected, in the browser or on the server. A client other than the report page sends the text before the server refuses it. |
| **A password that protects nothing** (the creator types only whitespace) | The create page refuses a non-empty password made only of whitespace before anything is encrypted or sent, with a message below the password field; an empty field means no password, and spaces around real characters stay part of the password | `create-page.test.ts` (no request, message, focus, inputs kept; `' secret '` opens only with the spaces), `a password of only spaces is refused before anything is sent` (`capsule.e2e.ts`) | Only the create page checks this: the core `Paste` and the CLI still treat a whitespace-only password as none. |
| **Rate-limit evasion via spoofed `X-Forwarded-For`** | Header honoured only when the TCP peer is in `trusted_proxies`; right-most untrusted address is used | `tests/rate_limits.rs` | Misconfigured `trusted_proxies` (too broad) re-opens it. |
| **Secrets in logs** | Access log records method and redacted path only; no bodies, no `Authorization`; report text never logged; `Debug` impls redact. Configuration errors on stderr (and from there the service log) name `server.public_url` and the reason, never its value — a share or management link pasted there by mistake carries the paste id and the link secret; a file that is not valid TOML is reported by line, column and reason without quoting the line (the same link written without quotes). `server.public_url` with user information, whitespace or control characters is refused at startup, so credentials never reach share links or problem type URIs | `logging.rs` (CI gate); `public_url_errors_never_repeat_the_value`, `public_url_with_credentials_whitespace_or_controls_is_refused`, `toml_syntax_errors_do_not_quote_the_file` (`config/tests.rs`), `ac25_check_never_repeats_a_rejected_public_url`, `ac25_serve_does_not_start_with_a_rejected_public_url` (`check` and `serve` on the binary, value from the file and from the environment) | Logs of the reverse proxy are outside CapsuleBin's control — configure them not to log query strings or bodies. Only `server.public_url` is protected this way: errors about other settings (e.g. `i18n.default_language`, a `[legal]` `url`) still repeat the rejected value, so a secret pasted into the wrong key can reach the log. |
| **Crafted database files** (restore of an untrusted backup) | `restore` verifies integrity and schema and never uses a `-wal` next to the backup | Backup tests | The bundled SQLite treats the file as trusted input (RUSTSEC-2026-0459): restore only your own backups. |
| **Supply chain** | Lockfiles, `cargo deny`, `npm audit`, minimal dependencies, `unsafe_code = "forbid"`; base images pinned by digest; actions pinned to commit SHAs; releases with an SBOM, `SHA256SUMS` and keyless cosign signatures | `cargo deny check`, `npm audit`, Grype scan of the container image (fixable high/critical fail), each also weekly | Advisories without a fix (currently a dev-only `picomatch` ReDoS); vulnerabilities in the base image without a fixed Debian package are reported, not blocked. |

## Explicit non-goals

- Protection against a fully compromised server for **web** users (see above).
- Physical erasure. SQLite's `secure_delete` overwrites burned, deleted and purged ciphertext with
  zeros in the database file, and the write-ahead log is truncated after checkpoints
  (`secure_delete.rs`). Beyond that file nothing is promised: WAL frames until the next checkpoint,
  the filesystem, SSD wear levelling and older backups may still hold ciphertext.
- Anonymity: CapsuleBin does not hide who talks to the server.
- Preventive content scanning, proof of reading, or recovery of lost links.

## Keeping this current

A change that adds a data path, an endpoint, a log line or a stored field updates the table above
and adds the test that backs the new row.
