# ADR-001 — Trust model: zero knowledge for stored content

**Status:** Accepted

## Context

CapsuleBin shares content through a link, without user accounts. The operator must be technically
unable to read shared content — not during normal operation, not from a backup, and not after a
database breach.

The paste id is not a secret in the way the rest of the link is. It travels in the URL path, so
server access logs, reverse proxies, browser history, link scanners and the report form all see it.
Knowing an id must therefore reveal nothing and allow nothing.

## Decision

Plaintext exists only in the client. The server is a content-blind storage, lifecycle and
moderation service.

Concretely:

- Encryption with AES-256-GCM in the browser or the CLI, before every upload.
- The link secret (32 random bytes) lives in the URL fragment, which is not transmitted with HTTP
  requests.
- Three values are derived from the link secret via HKDF with **domain separation**: the content
  key (`capsule/v1/content`), the reveal token (`capsule/v1/reveal`) and the describe token
  (`capsule/v1/describe`). The server learns the tokens but cannot obtain the content key from
  them. The exact derivations are in [`docs/crypto-profile.md`](../crypto-profile.md).
- For capabilities the server stores only `SHA-256` verifiers, never the secrets themselves, and
  compares them in constant time.
- The delete secret is **independently** random and is not derived from the share link — otherwise
  every recipient could delete the capsule.
- No API endpoint accepts or returns the plaintext of a capsule. File names, media types and the
  number of items live in the encrypted part, not in the public header. The only plaintext the
  server receives is what a reporter writes into an abuse report — a description and an optional
  contact — which is personal data, not capsule content
  ([ADR-013](ADR-013-abuse-reports-and-moderation.md)).

**Every paste endpoint that takes an id requires a capability derived from a link.** Without a
matching one it answers exactly like an unknown id, and the failure counts against the per-client
`invalid_capability` limit.

- **Describe** (`POST /api/v1/pastes/{id}/describe`) returns the public header (which shows
  whether a password is needed) and the lifecycle (expiry, burn flag, ciphertext size) for the
  describe token or the delete secret. The describe token is salted with the paste id, because `hkdf_salt` is inside the
  public header the descriptor returns; the password is not mixed in, because the descriptor is
  what tells the client that a password is needed.
- **Retrieve** needs both the describe token and the reveal token. The describe token proves the
  caller holds the link; the reveal token additionally proves the password, which is mixed into its
  derivation (NFC-normalized, PBKDF2).
- **Delete** needs the delete secret.

**Wrong passwords are limited per paste, and only link holders can reach that limit.** A retrieve
with a wrong describe token is a caller without the link: `404`, charged to the per-client limit
only. A right describe token with a wrong reveal token is a wrong password: `404`, and charged to
`invalid_capability_per_paste` as well (on by default). The per-paste limit is checked and charged
atomically by the retrieve handler *after* the describe token, so a caller with only the id can
neither use it up nor see its `429`. The handler keys it on the paste id it parsed from the
decoded path and looked the paste up by, so every spelling of that id in the request URI
(`%41…` for `A…`, in upper- or lower-case hex) shares one budget. The limiter is held in the
application state and shared with the rate-limiting middleware, so the retrieve handler always has
it; whether the limit is on is a matter of configuration only. Describe and delete never touch it:
their tokens are 256 bits with no password to guess.

## Rejected

- **Server-side encryption with a server-managed key.** More convenient (previews, full-text
  search and server-side rendering would all become possible), but the operator could read along.
  That is precisely the property the product sells.
- **A single secret for retrieval and decryption.** The server needs proof of authorisation on
  retrieval; if that value were the key, the server would know the key.
- **Deriving the delete secret from the link secret via HKDF, with its own info label** — the same
  domain-separation technique used for the content key and the tokens. HKDF domain separation
  only protects against a party who observes one derived *output* and must not be able to compute
  another (the server, which learns the tokens) — it gives no protection against a party who holds
  the *input* itself. The recipient holds the link secret directly, so any info label would be
  exactly as reproducible by the recipient as by the creator. Only independent randomness keeps
  delete out of a reader's reach.
- **Gating retrieval on the paste id alone.** If id-knowledge alone were enough to retrieve
  ciphertext, a burn-after-retrieval capsule could be consumed by anyone who merely saw the id
  somewhere, before the intended recipient ever opens the link; ciphertext could be archived by an
  id-only observer independently of ever holding the key, undoing the guarantee that key exposure
  and ciphertext exposure happen together; and for a password-protected capsule, an attacker could
  fetch the ciphertext once and brute-force the password entirely offline.
- **A public descriptor.** Honest if documented, but it would leave burn flag, expiry and password
  presence readable by anyone who saw an id in a log.
- **Storing a password verifier in the public header for a local, offline correctness check.**
  The convenience is real: without it, a mistyped password is only discovered after a network
  round trip. But whatever lets the legitimate client check a guess without touching the network
  lets anyone holding the descriptor do the same, entirely offline and unbounded by any rate limit.
  A password guess is instead only ever checked by deriving a reveal token with it and attempting
  retrieval: wrong password, wrong link secret and an expired or already-burned paste all fail
  identically.
- **Counting every failed request per paste id.** One request a second with a made-up token would
  keep the bucket empty, and the real recipient would get `429` for as long as that went on —
  blocking a capsule would need nothing but its id.
- **Checking the per-paste limit before the lookup, but charging only password attempts.** A caller
  with only the id could no longer drain it, but would see the `429` while a link holder guesses —
  an existence oracle for that id.
- **Taking the per-paste key from the raw request URI in the middleware** and handing the limit to
  the handler as an optional request extension. The middleware parsed the percent-encoded path, the
  handler looked the paste up by the decoded one: a percent-encoded spelling of a real id found the
  capsule with no limit attached, so a leaked link allowed unlimited password guesses. One id,
  parsed once, by the code that looks the paste up, closes that gap.
- **Preventive content moderation.** Impossible under zero knowledge. Instead there is
  notice-and-action on paste ids: a reported capsule can be disabled or deleted without decrypting
  it ([ADR-013](ADR-013-abuse-reports-and-moderation.md)).

## Consequences

- **No password reset, no recovery.** If the fragment is lost, the content is lost. That is not a
  gap, it is the property.
- Whoever can slow a capsule down through the per-paste limit holds its link and could retrieve or
  burn it anyway.
- **A limit of the model that must be documented openly:** the web client trusts the JavaScript
  served to it. A compromised server could ship modified JavaScript and exfiltrate keys.
  Mitigations reduce this (embedded assets, a strict CSP, no third-party resources, and a command
  line client that does not run code served by the server) but do not eliminate it. Marketing claims must
  not gloss over this.
- **No secure-erase guarantee.** Burn, delete and moderation delete remove logical availability.
  SQLite's `secure_delete` overwrites the freed ciphertext in the database file, but write-ahead log
  frames until the next checkpoint, the filesystem, SSD wear levelling and backups are outside that,
  so physical erasure is not promised.
- Every change that introduces a new data path needs at least one test proving that no plaintext
  travels along it. `db_no_plaintext.rs` asserts that no stored column contains the plaintext (as
  text, hex or base64), and the acceptance script searches the database files for a marker written
  through the command line client.
