# ADR-011 — The client-generated id is the idempotency key

**Status:** Accepted

## Context

Automation — the CLI and scripts built on it — needs safe retries. A client that sends `POST /api/v1/pastes` and loses the
connection before the response arrives cannot tell whether the paste was stored. A plain retry with
the same sealed paste would fail with `409 id_conflict` whenever the first attempt had landed;
sealing a new paste instead would leave an orphaned copy of the content on the server until
retention removes it.

The usual answer is an `Idempotency-Key` header plus a server-side table that maps keys to stored
responses.

## Decision

No separate key and no table: **the paste id is the idempotency key.** It is already 128 bits of
client-generated randomness, unique per sealed paste, and the server already rejects a second insert
under it.

When an insert hits an existing id, the server loads that row and answers `201` with the original
`CreateResponse` if — and only if — the request is a replay of the one that created it
(`Paste::is_replayed_by`):

- reveal, delete and describe verifiers, public header and ciphertext are identical, compared in
  constant time with every comparison evaluated;
- `format_version` and `burn_after_retrieval` are identical, and the requested lifetime resolves to
  the original one (`expires_at - created_at`);
- the paste is still retrievable now — not expired, consumed, disabled or deleted.

Every other collision stays `409 id_conflict`.

## Rejected

- **`Idempotency-Key` header + response table.** A second key with its own lifetime, storage and
  purge rules, for a guarantee the id already provides. It would also need the same "is this the
  same request?" comparison to be safe against key reuse.
- **Treating any collision as success.** Would let anyone who learns an id (it is in every share
  link) get a `201` for content they did not create, and would hide genuine client bugs.
- **Re-applying a replay's options.** A retry must not extend the expiry or revive a deleted or burned
  paste; "same request, same result" is the only semantics that is safe to repeat blindly.

## Consequences

- Clients may retry a create with the *same* `SealedPaste` on network errors, timeouts, `429` and
  `5xx`. `capsulebin put` does. Re-sealing on retry is wrong: it produces a new id and a duplicate.
- A replay costs one extra `SELECT` on the conflict path only; the normal path is unchanged.
- A replay reveals nothing new: the caller already holds every verifier and the ciphertext, and a
  mismatched request receives a plain `409`.
- Rate limits apply to replays like to any other create request.
