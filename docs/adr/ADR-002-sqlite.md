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
  file rather than left in free pages; `journal_size_limit` (64 MiB), so the write-ahead log is
  truncated after a checkpoint instead of keeping its peak size and old ciphertext frames.
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
  `capsulebin restore` uses only a backup's main file and moves an existing database aside rather
  than deleting it.
- Backups contain ciphertext and, where abuse reports exist, personal free-text data. They must be
  treated as sensitive operational data in their entirety.
- Deleted capsules can technically survive in disaster-recovery backups. That has to be documented
  openly by anyone running an instance for others; burn must not be advertised with delete promises
  it cannot keep.
