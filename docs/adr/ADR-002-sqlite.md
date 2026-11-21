# ADR-002 — Embedded SQLite as the only mandatory persistence

**Status:** Accepted

## Context

What gets stored are small, short-lived blobs (10 MiB per capsule by default) plus lifecycle
metadata. The product must be self-hostable: one binary, one file, no external service as a
mandatory dependency.

The critical requirement is not throughput but **atomicity**: "burn after first retrieval" has to
be an at-most-once handoff, even under concurrent retrieval.

## Decision

Embedded SQLite via `sqlx`, with migrations embedded in the binary. Ciphertext is stored as a blob
in the table.

Operational parameters set at startup:

- WAL mode (many readers, one writer). The data directory has to live on a local filesystem — WAL
  over NFS is not reliable.
- `busy_timeout`, `foreign_keys = ON`, explicit transactions.
- `synchronous = FULL` by default. At this data volume the cost is small, and losing data after an
  acknowledged write would be unrecoverable for one-time pastes.
- `secure_delete = ON`, so burned, deleted and purged ciphertext is overwritten with zeros in the
  file rather than left in free pages, once a checkpoint has copied the zeroed pages into it;
  `journal_size_limit` (64 MiB), so the write-ahead log does not keep its peak size after a burst
  of large uploads. That limit is what the WAL is truncated to when it is reset; an ordinary
  checkpoint truncates nothing, and WAL frames can hold old ciphertext until later writes reuse them
  or a `wal_checkpoint(TRUNCATE)` or the last connection closing truncates the WAL (threat model,
  physical erasure).
- Retrievals stay reads: the only retrieval metadata is a coarse `last_retrieved_at`, written at most
  once per granularity window, so the single writer is not a bottleneck for downloads.
- The burn transaction starts with `BEGIN IMMEDIATE`, so that the write lock is taken before the
  `SELECT`. Without it two readers could observe the same payload and both deliver it.

## Rejected

- **PostgreSQL or similar.** No benefit for the core use case, and it would stretch the
  self-hosting instructions from "one binary" to "one binary plus a database server".
- **Ciphertext on the filesystem, metadata in SQLite.** Two storage locations mean two delete paths
  and the possibility of orphans. At 10 MiB there is nothing to gain. A filesystem or S3 backend is
  explicitly not a goal.
- **Horizontal scaling.** With in-memory rate limits and a single SQLite writer it would not be
  consistent anyway. More capacity means more independent instances, each
  with its own database, rather than a cluster.

## Consequences

- One writer at a time. Sufficient for the expected load, but a deliberate limit.
- Backups go through `VACUUM INTO` (`capsulebin backup`), never by copying an active WAL database.
  The snapshot is written into a private directory next to the output and then renamed to the
  output name, because `VACUUM INTO` opens its target by name: a pre-created file could be swapped
  between creation and use, and a `chmod` afterwards could not revoke a descriptor opened in
  between. An existing entry at the output name is refused, checked again right before the
  rename; an entry created between that check and the rename would be replaced. That window is
  accepted as for `restore`: `rename` follows no symlink, and only someone who can write the
  output's directory could use it, who could replace the backup anyway. Publishing by hard link
  would close it, but would make backups fail on filesystems without hard links (FAT/exFAT, some
  SMB/CIFS mounts).
- `capsulebin restore` copies only a backup's main file into a private directory next to the
  database, verifies and migrates that copy, and only then moves an existing database (with its
  `-wal`/`-shm`) aside rather than deleting it and renames the copy into place; a failure undoes
  every move. So exactly the bytes that are installed are checked, a `-wal` next to the backup never
  reaches SQLite (the bundled SQLite treats database files as trusted input, RUSTSEC-2026-0459), and
  disk-full or migration errors happen before a live file moves. A missing database next to restore
  leftovers is a startup error, not a new empty database.
- Paste ids are never freed. Delete, burn and moderation keep a tombstone row without ciphertext;
  the retention rules delete the row, and an `AFTER DELETE` trigger on `pastes` puts its id into
  `retired_ids`, a `WITHOUT ROWID` table with the id as its only column that nothing deletes from
  ([ADR-011](ADR-011-idempotent-create.md)). The trigger rather than the purge code does it, so no
  current or future `DELETE FROM pastes` can forget. Before its cutover, `restore` attaches the
  replaced database read-only to the staged copy's connection and copies its ids (rows and retired
  ids) that the copy does not hold as a row into the copy's `retired_ids`, inside SQLite so millions
  of ids never pass through the process; the database path is bound as a parameter, never formatted
  into SQL. The replaced database is read in exactly one statement, into a scratch table of the
  staged copy that is created and dropped in the same transaction, so it never reaches the installed
  database. Not a `TEMP` table: the bundled SQLite keeps those in memory, and a temporary file would
  lie outside the private stage. If the replaced database cannot be attached or read, the restore
  still runs and warns: corruption is the typical reason to restore, and refusing would remove the
  way back. Only that one read can produce the warning; a failure counting or inserting the ids
  concerns the copy and stops the restore like any other step.
- CapsuleBin creates the database file itself, exclusively and owner-only (`0600`), instead of
  leaving that to SQLite: SQLite derives the modes of `-wal` and `-shm` from the main file, so this
  keeps all three private without changing the process-wide umask.
- Backups contain ciphertext and, where abuse reports exist, personal free-text data. They must be
  treated as sensitive operational data in their entirety.
- Deleted capsules can technically survive in disaster-recovery backups. That has to be documented
  openly by anyone running an instance for others; burn must not be advertised with delete promises
  it cannot keep.
- `retired_ids` grows by one entry (about 40 bytes) per purged capsule, forever, and with it backup
  size and `restore`'s carry-over.
