# ADR-003 — Independent version numbers, golden vectors as the conformance contract

**Status:** Accepted

## Context

A capsule written today has to remain decryptable by a client shipped a year from now — the server
never sees plaintext, so nothing about a stored capsule can be fixed after the fact. At the same
time, the format has to be able to change: a new optional manifest field, a new KDF, a bumped
PBKDF2 default.

Three things change for different, unrelated reasons and must not be conflated into one version
number: the HTTP API, the envelope (cipher and key derivation), and the plaintext container's
framing and manifest schema (ADR-008). Coupling them would mean an unrelated change — say, an added
optional manifest field — forces every existing link to be treated as legacy.

The format is also meant to be implementable by a third party in another language, not only by
`@capsulebin/core`. That only works if the format is specified precisely enough to build a decoder
from the specification alone, and if there is a machine-checkable way to confirm an implementation
actually matches it.

## Decision

**Independent version numbers.** The API version lives in the path (`/api/v1/`). The envelope
format lives in the public header and in `pastes.format_version`. Inside the encrypted container,
the framing version and the manifest schema version are versioned separately again (ADR-008).

**No silent algorithm changes.** Any change to derivation, cipher or serialization bumps the
relevant version and ships its own test vectors. The PBKDF2 iteration count is *carried in the
format* rather than hard-coded, so that older capsules stay readable after the default changes.

**Golden vectors are the conformance contract.** `docs/test-vectors/capsule-v1.json` is checked by
`@capsulebin/core`'s test suite (`src/core/tests/golden-vectors.test.ts`), a required CI gate. A
future third-party implementation is checked against the same vectors, not against
`@capsulebin/core`'s source.

**The public header is AAD.** The canonically serialized public header goes into AES-GCM as
additional authenticated data, so encoders serialize it deterministically — fixed field order, no
implementation-dependent formatting — and decoders authenticate the stored bytes as received (see
`docs/crypto-profile.md`). The container inside the
ciphertext is under no such requirement: it is covered by the tag as a whole, so it needs
unambiguous parsing rather than canonical encoding.

## Rejected

- **Documenting the format in prose only.** Adequate until a second implementation — in-repo or
  third-party — appears, then it diverges silently. `docs/capsule-file-format.md` and
  `docs/crypto-profile.md` exist so that a decoder can be built from them alone.
- **Omitting the algorithm identifiers and assuming them implicitly.** Prevents any later migration
  without breaking existing links.

## Consequences

- The test vectors are an API contract. A vector may change when the relevant version increases —
  never in order to turn a red test green.
- The container decoder is strict where it has to be and lenient where it can afford to be: a wrong
  magic value, an unknown framing version, an unknown manifest schema version, a missing mandatory
  field, too many items, an oversized name and any size that does not add up are errors, not
  warnings. Unknown *optional* manifest fields are ignored, which is what makes a later additive
  change possible. The rules are enumerated in ADR-008; each is covered by a negative test in
  `@capsulebin/core`, and the ones an independent decoder most easily gets wrong also by a negative
  vector.
- Confidence that an independent third-party implementation can interoperate rests entirely on
  `docs/capsule-file-format.md`, `docs/crypto-profile.md` and `docs/test-vectors/` being precise and
  complete — there is no second in-repo implementation to catch drift by disagreement (ADR-009).
