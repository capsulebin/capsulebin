# ADR-008 — A dedicated binary container instead of a general-purpose serialization

**Status:** Accepted

## Context

Inside the ciphertext sits one plaintext container holding everything a capsule contains: an optional
title, per-item metadata (name, size, media type, how it should be presented) and the item bytes
themselves. Items are UTF-8 text in the common case and arbitrary files in the general one.

The container is created by the browser client and by the command line client, and read by both. It
never reaches the server in readable form, so nothing about it can be fixed server-side after the
fact: a capsule written today has to remain parseable by a client shipped a year from now.

A general-purpose binary serialization would have been the obvious choice. It was not taken.

## Decision

**A container format defined by this project, `capsule-file-v1`.** A fixed 14-byte frame, a UTF-8
JSON manifest, the raw item bytes back to back in manifest order, then zero padding — the exact
byte layout, manifest schema and validation rules are specified in
[`docs/capsule-file-format.md`](../capsule-file-format.md), not repeated here.

An item starts at the sum of the sizes of all items before it. There is no separator, and none is
needed, because the manifest states every size.

**No base64 anywhere in the plaintext container.** File bytes are stored as they are. Metadata is
JSON because it is small, human-readable in a debugger and available in both languages without a
dependency; payload bytes are binary because that is what they are.

**Three separately versioned things.** The envelope version (`FORMAT_VERSION`, announced in the
public header and stored in `pastes.format_version`), the framing version (`FRAMING_VERSION`, the
u16 above) and the manifest schema version (`MANIFEST_SCHEMA_VERSION`, repeated inside the manifest
as `schema_version`). They change for different reasons; coupling them would mean an added optional
field invalidates every link in circulation.

**Strict on structure, forward compatible on optional fields.** Unknown *optional* manifest fields
are ignored; a missing mandatory field, an unknown `schema_version` and every size or count
violation are errors. This is the opposite of the rule for configuration, where an unknown key is a
startup error — and deliberately so. A typo in a configuration file can be corrected by the operator
who made it; a stored capsule cannot be corrected at all, so tolerance has to sit on the side that
cannot be repaired.

**Item ids are ULIDs.** 26 characters of Crockford base32, unique within one manifest, stable enough
for a client to refer to an item. The timestamp inside
a ULID stays in the encrypted container and never reaches the server.

**Sizes are validated before they are trusted.** The manifest length is checked against a hard 64 KiB
limit *and* against the bytes actually present before anything is allocated; item sizes are checked
as safe integers (at most 16 of them, so their sum cannot overflow) and must fit into the remaining
payload.

**Padded to the PADMÉ length.** Encoders append `0x00` bytes up to
`min(PADMÉ(L), MAX_CAPSULE_BYTES)`; decoders accept any amount of zero padding after the last item
and reject a non-zero byte there. Without padding the ciphertext length would be the plaintext
length plus 16 bytes, which identifies known files by their size. PADMÉ leaves only the order of
magnitude visible, at an overhead of at most about 12 %.

**Magic bytes are identification, not protection.** They say "this is a capsule file", which is
useful when a decryption succeeds against the wrong key material or a future format appears.
Integrity is provided by AES-GCM over the entire byte sequence, and by nothing else here.

## Rejected

- **CBOR (or MessagePack, or protobuf).** The obvious candidate, rejected for three reasons:

  1. **A dependency on both sides.** Rust would gain a CBOR crate and the browser bundle a
     hand-written or imported decoder, for a structure that is one header, one small JSON object and
     a concatenation. The audit surface of a general-purpose decoder is much larger than the format
     needs, and it processes attacker-controlled bytes.
  2. **Item boundaries would come from the encoding rather than from stated lengths.** With explicit
     sizes in the manifest, "where does item 3 start" has one answer, computed before any byte is
     read. A nested encoding puts that answer inside the decoder.
  3. **Canonical form is a recurring hazard.** CBOR has more than one way to encode the same value,
     so two implementations can agree semantically and disagree byte-wise. That does not matter for
     the container — it is not authenticated data — but it is a trap that every additional
     implementation would have to be tested against.

- **JSON for everything, with items base64-encoded.** Simplest to write and the reason it loses is
  arithmetic: base64 inflates by a third, so a 10 MiB capsule of files becomes 13.4 MiB of
  ciphertext plus the cost of encoding and decoding it in a browser, entirely to avoid one length
  field.

- **A delimiter between items instead of lengths.** Arbitrary files contain every byte sequence,
  including whatever delimiter is chosen, so this needs escaping — which changes the payload bytes,
  costs a pass over the data, and puts a second parser in the path. Lengths avoid the problem rather
  than solving it.

- **A single version number for the whole format.** It would force every existing link to be treated
  as legacy for the sake of an added optional field, which is precisely the change that should be
  cheap.

- **Padding as a manifest field.** The field's own digits change the manifest length, which makes
  the target length awkward to reach exactly; zero bytes after the last item need no bookkeeping
  and keep "no unexplained bytes" as a decoder rule.

- **Fixed-size buckets (powers of two) instead of PADMÉ.** Up to 100 % overhead; PADMÉ leaks about
  as little for at most ~12 %.

- **Compression before encryption.** Not part of this decision and not adopted: compression ratios
  leak information about the plaintext, which contradicts the trust model in ADR-001. If it is ever
  wanted, it belongs in a new framing version with that trade-off stated explicitly.

## Consequences

- The format has to be specified precisely enough for a third party to implement, because there is
  no second, independent decoder that would catch a misreading. `docs/capsule-file-format.md` is that specification,
  `@capsulebin/core` is its reference implementation (ADR-009), ADR-003 governs versioning, and the
  golden vectors are the contract.
- Every parser rule above needs a negative test, not only a positive one. A decoder that accepts
  a manifest length larger than the payload passes every positive test there is.
- JSON numbers are f64 in JavaScript, so every size and count is validated with `Number.isSafeInteger` before use.
- The manifest is plaintext *inside* the ciphertext. File names, sizes and media types are therefore
  as protected as the content — but a client that displays a name is displaying attacker-controlled
  text, so name sanitisation stays a client concern.
- Ciphertexts are up to ~12 % larger than the plaintext container. A server whose
  `pastes.max_plaintext_size` is set far below the client limit may reject a capsule whose unpadded
  size would have fit.
- Adding a mandatory manifest field is a breaking change and requires a new schema version with its
  own vectors. Adding an optional one does not.
