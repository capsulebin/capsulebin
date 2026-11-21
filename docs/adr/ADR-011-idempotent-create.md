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

An id is never freed. Delete, burn and moderation keep a tombstone row; a row the retention rules
delete leaves its id in the table `retired_ids` (migration `0004`, filled by an `AFTER DELETE`
trigger on `pastes`, never emptied), and `restore` retires the ids of the replaced database that the
backup lacks. Create inserts only if the id is not in `retired_ids`, in the same statement as the
insert; a retired id is the same `409 id_conflict` as an occupied one, byte for byte. A replay of a
retired id cannot succeed: its row, and with it the original request, is gone.

## Rejected

- **`Idempotency-Key` header + response table.** A second key with its own lifetime, storage and
  purge rules, for a guarantee the id already provides. It would also need the same "is this the
  same request?" comparison to be safe against key reuse.
- **Treating any collision as success.** Would let anyone who learns an id (it is in every share
  link) get a `201` for content they did not create, and would hide genuine client bugs.
- **Re-applying a replay's options.** A retry must not extend the expiry or revive a deleted or burned
  paste; "same request, same result" is the only semantics that is safe to repeat blindly.
- **Freeing the id of a purged row.** Because the client chooses the id, anyone holding a full link
  could, after the purge, create a new capsule under the same id and link secret: the old link would
  open different content without any client noticing, and open reports and the moderation history,
  which name a paste by its id, would attach to the new capsule. Row generations would not help, as
  the new row is a legitimate new generation.
- **Checking `retired_ids` in a separate `SELECT`.** A purge committing between the check and the
  insert would retire the id after it was checked.
- **A distinct code for a retired id.** It would tell "was purged" from "is live".

## Consequences

- Clients may retry a create with the *same* `SealedPaste` on network errors, timeouts, `429` and
  `5xx`. `capsulebin put` does. Re-sealing on retry is wrong: it produces a new id and a duplicate.
- A replay costs one extra `SELECT` on the conflict path only; the normal path is unchanged.
- A replay reveals nothing new: the caller already holds every verifier and the ciphertext, and a
  mismatched request receives a plain `409`.
- Rate limits apply to replays like to any other create request.
- Anyone allowed to create (every client in `public` mode, token holders in `token` mode) can test
  whether one specific id is taken or was ever used: creating it answers `409`. Live rows,
  tombstones and retired ids all answer alike, so the `409` does not say which. Without permission
  to create, ids stay indistinguishable from unknown ones. Ids are 128-bit random, so this only
  confirms ids the caller already knows.
- `retired_ids` holds only the id, with no timestamp, verifier, size or status, and has no retention
  rule or setting: anything more would be metadata kept forever, and a switch would reopen the gap.
  It grows by one entry (about 40 bytes) per purged capsule; lookups stay a primary-key probe.
- Rows purged before migration `0004` left no record; their ids are free.
