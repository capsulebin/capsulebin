-- Drop the retrieval counter.
--
-- Nothing read it - not the API, not the purge, not moderation - yet keeping it
-- exact turned every retrieval into a write transaction with a disk flush, all
-- of them queued on SQLite's single writer. It was also retrieval metadata the
-- documentation says the server does not keep: the only one left is the coarse
-- `last_retrieved_at` the inactivity rule needs.

ALTER TABLE pastes DROP COLUMN retrieval_count;
