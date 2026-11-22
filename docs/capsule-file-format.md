# Capsule File Format v1

This is the binding specification for the plaintext container that sits inside
a capsule's ciphertext. It is written so that a decoder can be built from this
document alone, in any language, without reading the rationale behind it.

The rationale for these choices — why a bespoke binary container instead of
CBOR, protobuf or base64-in-JSON — is recorded in
[ADR-008](adr/ADR-008-capsule-file-format.md). This document only
states the format itself.

The reference implementation is `@capsulebin/core` (`src/core/src/capsule.ts`).
Conformance is checked against [`test-vectors/capsule-v1.json`](test-vectors/capsule-v1.json).

## Versioning

Three independent version numbers exist, and they must not be conflated:

| Name | Where it lives | Changes when |
|---|---|---|
| Envelope format (`envelope_format`) | The public header, see [crypto-profile.md](crypto-profile.md) | The relationship between the ciphertext and its container changes |
| Framing version | Byte 8–9 of this format, below | The binary layout described in this document changes |
| Manifest schema version (`schema_version`) | Inside the manifest JSON | A mandatory manifest field is added, removed or changes meaning |

This document describes framing version `1` and manifest schema version `1`.

## Byte layout

All multi-byte integers are big-endian.

```
Byte  0..8     Magic            8 bytes, ASCII "CAPSULE\0" (0x43415053554c4500)
Byte  8..10    Framing version  u16, value 1
Byte 10..14    Manifest length  u32, byte length of the manifest that follows
Byte 14..14+n  Manifest         exactly n bytes of UTF-8 JSON, n = manifest length
Byte 14+n..    Items            item bytes, concatenated in manifest order, no delimiters
then           Padding          zero or more 0x00 bytes
```

`n` is the manifest length read from bytes 10–14. There is no delimiter between
items: the boundary of item *k* is the sum of the `size` fields of items
`0..k`. Everything after the last item is padding.

### Padding

The ciphertext length is visible to the server and to anyone who obtains it.
Unpadded, it equals the plaintext length plus 16 bytes, which identifies known
files by size. Encoders therefore pad the encoded capsule with `0x00` bytes:

```
L       = length of the capsule without padding (header + manifest + items)
padded  = max(L, min(PADME(L), MAX_CAPSULE_BYTES))

PADME(L), for L ≥ 2:                    (Nikitin et al., PURBs, 2019)
    E = floor(log2(L))
    S = floor(log2(E)) + 1
    B = E - S
    if B ≤ 0: return L
    return L rounded up to a multiple of 2^B
PADME(L) = L for L < 2
```

PADMÉ leaves only O(log log L) bits of the length visible at an overhead of at
most about 12 % (3 % at megabyte sizes). Decoders accept padding of any length
up to the format limit — including none — as long as every padding byte is
`0x00`; a non-zero byte after the last item is an error, so nothing can hide
there. A decoder refuses input longer than `MAX_CAPSULE_BYTES` before looking at
it, so it never scans more than that. This refuses nothing a valid encoder
produces: `L` is at most `MAX_CAPSULE_BYTES` and the padding is capped there.

## Manifest

The manifest is a single UTF-8 JSON object with this shape:

```json
{
  "schema_version": 1,
  "title": "optional string, or null",
  "items": [
    {
      "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
      "name": "notes.md",
      "size": 1234,
      "media_type": "text/markdown",
      "presentation": "markdown",
      "language": "optional string, omitted if not applicable"
    }
  ]
}
```

Field rules:

- **`schema_version`** — mandatory, integer, must equal `1`. An unrecognized
  value is an error, not a fallback.
- **`title`** — a string or `null`. Encoders always write the key (`null` when
  untitled); a decoder treats an absent key as `null` and rejects any other
  type.
- **`items`** — mandatory, a non-empty array, at most `16` entries (`MAX_ITEMS`).
- **`items[].id`** — mandatory string. A ULID (26 characters, Crockford
  base32), unique within the manifest. Not validated as a ULID by the decoder,
  only checked for uniqueness — a client that produces something else is not
  the decoder's problem to police.
- **`items[].name`** — mandatory string, at most `255` UTF-8 bytes. Display
  and download name; never a filesystem path, never validated for path
  traversal by the decoder because it is never used as one. A client that
  writes items to files derives the file name from it and must treat it as
  untrusted: keep only the last path component, replace control characters
  and characters common file systems refuse, never create a name starting
  with a dot (a sender could otherwise place `.bash_profile` or `.envrc`),
  and stay within the file system's name length (the reference CLI keeps
  255 bytes including a ` (n)` suffix for taken names).
- **`items[].size`** — mandatory non-negative integer, and must be a "safe
  integer" (exactly representable, matters for JSON's `f64` numbers in
  JavaScript). Must equal the number of raw bytes this item contributes.
- **`items[].media_type`** — mandatory string. A MIME type, used for download
  headers by the client; not validated against a fixed list.
- **`items[].presentation`** — mandatory string, one of `"plain"`, `"code"`,
  `"markdown"`, `"download"`. Any other value is a decode error.
- **`items[].language`** — optional string. Present only for `"code"` items as
  a syntax-highlighting hint. Absence is not an error for any presentation.

Unknown *optional* fields at either the manifest or item level are ignored by
the decoder — this is what makes an additive schema change possible without a
version bump. A missing *mandatory* field, or an item whose `size` does not
match the bytes actually present, is always an error.

## Limits

| Limit | Value | Enforced |
|---|---|---|
| Items per capsule (`MAX_ITEMS`) | 16 | On encode and decode |
| Item name length | 255 UTF-8 bytes | On encode and decode |
| Manifest size (`MAX_MANIFEST_BYTES`) | 64 KiB | On encode and decode, before the JSON is parsed |
| Total capsule size (`MAX_CAPSULE_BYTES`) | 10 MiB | On encode, and on decode before anything else; the server enforces its own configured `pastes.max_plaintext_size` independently (`capsulebin.example.toml`) |

The manifest length is validated against the limit *and* against the number of
bytes actually remaining in the buffer before any allocation happens, so a
manifest-length field claiming more data than exists is rejected rather than
causing an over-read.

## Decoder error conditions

Every one of these must be rejected, not warned about or silently corrected:

- Input longer than `MAX_CAPSULE_BYTES` (checked first, before the header).
- Input shorter than the 14-byte header.
- Wrong magic bytes.
- An unrecognized framing version.
- A manifest length exceeding `MAX_MANIFEST_BYTES`, or exceeding the bytes
  remaining in the input.
- Manifest bytes that are not valid UTF-8.
- Manifest bytes that are not valid JSON, or do not parse to an object.
- An unrecognized `schema_version`.
- A missing mandatory field, at the manifest or item level.
- `items` empty, or containing more than `MAX_ITEMS` entries.
- Any item whose `presentation` is not one of the four allowed values.
- Any item name exceeding 255 UTF-8 bytes.
- Duplicate `id` values across items.
- The sum of declared item sizes exceeding the number of bytes remaining after
  the manifest.
- Any non-zero byte after the last item (padding must be `0x00`).

## Item order

The manifest order is the order the creator arranged the items in, and viewers
show them in that order. The format attaches no meaning to it beyond that.
