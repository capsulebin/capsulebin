# Crypto Profile v1

This is the binding specification for CapsuleBin's client-side cryptography:
key derivation, encryption and the capability verifiers the server checks.
It describes *what* to compute; the trust model behind these choices — why
the server never sees capsule plaintext or keys — is recorded in
[ADR-001](adr/ADR-001-trust-model.md); why it runs as plain
TypeScript rather than compiled to WebAssembly is recorded in
[ADR-009](adr/ADR-009-typescript-cli-and-core-package.md).

The reference implementation is `@capsulebin/core`
(`src/core/src/crypto.ts` and `src/core/src/public_header.ts`), shared unmodified by the
web client and the CLI. Conformance is checked against
[`test-vectors/capsule-v1.json`](test-vectors/capsule-v1.json).

## Primitives

| Purpose | Algorithm |
|---|---|
| Content encryption | AES-256-GCM |
| Key derivation from the link secret | HKDF-SHA-256 |
| Key derivation from a password | PBKDF2-HMAC-SHA-256 |
| Capability verifiers | SHA-256 |

All of the above are available as native Web Crypto (`crypto.subtle`) in
browsers and in Node.js 24, which is what lets both `src/server/web` and
`src/cli` share one implementation with no platform adapter.

## Secrets

Two independent 32-byte secrets, both generated with a cryptographically
secure RNG (`crypto.getRandomValues`) and never sent to the server in raw
form:

- **Link secret** — lives in the URL fragment (`#`), which browsers never
  transmit in an HTTP request. Everything needed to decrypt the capsule is
  derived from this value.
- **Delete secret** — independently random, *not* derived from the link
  secret. This is what keeps a recipient who can read a capsule from also
  being able to delete it: knowing the link secret gives no information about
  the delete secret. HKDF domain separation would not achieve this — see
  [ADR-001](adr/ADR-001-trust-model.md) for why.

## Derivation chain

```
link-secret (32 random bytes)
    │
    ├─ HKDF-SHA-256(salt=hkdf_salt, info="capsule/v1/content" [+ password-key])
    │      └─► content-key (32 bytes) ──► AES-256-GCM key
    │
    ├─ HKDF-SHA-256(salt=hkdf_salt, info="capsule/v1/reveal" [+ password-key])
    │      └─► reveal-token (32 bytes)
    │             └─ SHA-256 ──► reveal-verifier   (sent to, and stored by, the server)
    │
    └─ HKDF-SHA-256(salt=paste-id (16 raw bytes), info="capsule/v1/describe")
           └─► describe-token (32 bytes)
                  └─ SHA-256 ──► describe-verifier (sent to, and stored by, the server)

delete-secret (32 random bytes, independent)
    └─ SHA-256("capsule/v1/delete" || delete-secret) ──► delete-verifier   (stored by the server)
```

- `hkdf_salt` is 16 random bytes, generated once per capsule and carried in
  the public header (see below) so the recipient can repeat the derivation.
- The two HKDF calls use the **same** `ikm` (the link secret) and the **same**
  `salt`, but different `info` labels. This domain separation is what makes
  the reveal token learn-able by the server (it is presented on every
  retrieval) without exposing the content key: HKDF's `info` parameter is
  one-way with respect to the derived output, so possessing `reveal-token`
  gives no way to compute `content-key`.
- The **describe token** is the capability to read the descriptor (public
  header and lifecycle metadata). It cannot use `hkdf_salt`, because that salt
  lives in the public header the descriptor returns; it uses the paste id's
  16 raw bytes (the base64url id, decoded) as salt instead, and its own `info`
  label. It never includes the password key: the descriptor is what tells a
  client that a password is needed. Without a matching describe token (or the
  delete secret, for the management link) the descriptor answers exactly like
  an unknown id, so a paste id alone reveals neither existence nor metadata.
- The delete verifier's domain separation works differently: it is a prefixed
  hash (`SHA-256(domain || secret)`) rather than an HKDF label, because the
  delete secret is not run through HKDF at all — there is nothing else to
  derive from it.

### Password-protected capsules

When a capsule is password-protected, a **password key** is derived first and
mixed into both HKDF calls above, so that a wrong password produces both a
wrong content key *and* a wrong reveal token — the server rejects the request
before the client ever attempts an AES-GCM decryption with the wrong key.

```
password (string)
    │
    ▼  Unicode NFC normalization, then UTF-8
    ▼  PBKDF2-HMAC-SHA-256(salt=password_salt, iterations=N)
password-key (32 bytes)
    │
    └─► mixed into HKDF info: info || 0x00 || password-key
        (both the content-key and reveal-token derivations above)
```

There is deliberately no password verifier in the public header. A wrong
password produces a wrong reveal token, and the server rejects it exactly
like a wrong link secret — discovered only via the rate-limited retrieve
request, the same generic error either way. A locally checkable verifier
would have to live in the public header, which the descriptor
endpoint hands out to anyone holding the link — turning it into an
offline brute-force oracle decoupled from the rate limit. See [ADR-001](adr/ADR-001-trust-model.md).

- The password is normalized to Unicode **NFC** before it is encoded as UTF-8.
  The same password can arrive as different code point sequences depending on
  the operating system and input method (`ä` as U+00E4, or as `a` followed by
  U+0308); without normalization those derive different keys and the capsule
  cannot be opened.
- `password_salt` is 16 random bytes, independent of `hkdf_salt`. In the
  public header it must be canonical unpadded base64url of exactly 16 bytes
  (22 characters); any other salt is rejected.
- Default iteration count: `600,000`. Chosen to stay under ~2 seconds on
  reference hardware; carried in the public header rather than hard-coded, so
  a future default can rise without breaking capsules created under the old
  one.
- Floor: `100,000` iterations (`MIN_PBKDF2_ITERATIONS`). The iteration count
  must be an integer; a public header claiming fewer, or a non-integer, is
  rejected. The header comes from whoever issued the link, and the reveal
  token derived from the password goes to that issuer's server: without a
  floor, a hostile issuer could pick a trivially cheap KDF and guess the
  password offline from the reveal token. The floor leaves the default free to
  change in either direction above it.
- Hard ceiling: `5,000,000` iterations. A public header claiming more than
  this is rejected before PBKDF2 runs, to bound the client-side cost of
  parsing an arbitrary (attacker-supplied) header.
- Both bounds and the salt rule are checked when the header is parsed, before
  a password is asked for or anything is derived from it — not in the PBKDF2
  primitive, which stays usable with any parameters. A client treats such a
  header as an invalid capsule, not as a network failure. The server stores
  the header as opaque bytes and does not check it. No CapsuleBin client has
  created a header outside these rules; the
  [test vectors](test-vectors/capsule-v1.json) include headers a parser must
  accept and refuse (`public_header`), and the password vectors use the floor.
- The `info` concatenation is `info_label || 0x00 || password_key` — a single
  `0x00` separator byte between the label and the key.

## AES-256-GCM parameters

- **Key**: 256 bits, the `content-key` derived above.
- **Nonce (IV)**: 12 random bytes, generated fresh per capsule, carried in the
  public header. Never reused with the same key — each capsule derives its own
  `content-key` from a fresh `hkdf_salt`, so nonce reuse across capsules is not
  a risk even though the derivation is deterministic given the same inputs.
- **Additional authenticated data (AAD)**: the public header's canonical JSON
  serialization (see below). GCM authenticates the header together with the
  ciphertext, so a bit-flip in a stored or transmitted header is detected on
  decryption rather than silently accepted.
- **Plaintext**: the encoded capsule file (see
  [capsule-file-format.md](capsule-file-format.md)).

## Capability verifiers

The server never learns a secret directly. It learns and stores only a
verifier, and checks a presented value against it in constant time
(`subtle`-crate / constant-time comparison, never `==`):

| Capability | Presented value | Stored verifier |
|---|---|---|
| Describe | `describe-token`, or the delete secret | `SHA-256(describe-token)`; the delete verifier below |
| Retrieve | `describe-token` and `reveal-token`, both | `SHA-256(describe-token)` and `SHA-256(reveal-token)` |
| Delete | `delete-secret` | `SHA-256("capsule/v1/delete" || delete-secret)` |
| Create (token mode, operator-configured) | creation token | `SHA-256("capsule/v1/creation" || token)` |

All verifiers are 32 bytes (SHA-256 output), encoded as base64url without
padding on the wire.

Retrieval is gated on the describe and reveal tokens rather than on the paste id alone: the id
travels in the URL path (server logs, proxies, browser history), while the link secret lives in
the fragment, which browsers never transmit. See [ADR-001](adr/ADR-001-trust-model.md) for the
concrete risks an id-only check would have (premature burn, ciphertext exfiltration independent of
key exposure, offline password brute-forcing).

## The public header

The public header is a small JSON object, sent to and stored by the server
**unencrypted** — it has to be, since a recipient needs `nonce`, `hkdf_salt`
and `password.*` before it can derive anything else. The server hands it out
only with the descriptor, i.e. to a holder of the describe token or the delete
secret. Nothing in it reveals capsule content, item count, or size.

```json
{
  "envelope_format": 1,
  "content_format": "capsule-file-v1",
  "cipher": "AES-256-GCM",
  "nonce": "base64url, 12 bytes",
  "hkdf_salt": "base64url, 16 bytes",
  "password": {
    "required": true,
    "kdf": "PBKDF2-HMAC-SHA-256",
    "salt": "base64url, 16 bytes",
    "iterations": 600000
  }
}
```

`password` is present only when the capsule requires one; its absence means
the capsule has no password.

**Canonical serialization**: since the header is authenticated as AES-GCM
AAD, its byte representation must be reproducible — the same logical header
must serialize to the exact same bytes on every implementation, every time.
The rule: `JSON.stringify` with no added whitespace, fields in the order shown
above (`password` last, omitted entirely rather than `null` when absent). Any
implementation must serialize in this fixed field order — an implementation
that reorders keys, changes whitespace, or emits `password: null` instead of
omitting the key would produce a header that fails AAD verification on every
other implementation, even though the logical content is identical. This
requirement applies to the header only; the capsule manifest is inside the
ciphertext and is authenticated as a whole, so it needs no canonical form
(see [capsule-file-format.md](capsule-file-format.md)).

A header is rejected if `envelope_format`, `content_format` or `cipher` do not
match the values above exactly.

## Version constants

| Constant | Value |
|---|---|
| `envelope_format` | `1` |
| `content_format` | `"capsule-file-v1"` |
| `cipher` | `"AES-256-GCM"` |
| Fragment prefix | `"v1."` (prepended to the base64url link secret in the share URL fragment) |

A change to any derivation, cipher choice or serialization rule bumps
`envelope_format` and ships new test vectors — never a silent change under the
same version number, since an existing link in circulation must remain
decryptable with the rules in force when it was created.
