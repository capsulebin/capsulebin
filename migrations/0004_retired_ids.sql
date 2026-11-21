-- Never free a paste id again.
--
-- The client chooses the id (ADR-011). Delete, burn and moderation keep a
-- tombstone row, so their id stays taken; the retention rules, however, delete
-- the whole row. Without this table the id would then be free, and anyone
-- holding the full link could create a new capsule under the same id with the
-- same link secret: the old link would open different content and no client
-- could notice. Open reports and the moderation history, which name a paste by
-- its id, would likewise attach to that later capsule (REL-005).
--
-- So every deleted row leaves its id here, and `create` refuses a retired id
-- exactly like an occupied one (`409 id_conflict`). A trigger rather than the
-- purge code does it, so no current or future `DELETE FROM pastes` can forget.
--
-- Only the id is kept: no timestamp, verifier, size or status. It is the
-- minimum needed to refuse reuse; anything more would be lifecycle metadata
-- kept forever. Nothing ever deletes from this table - no retention rule, no
-- setting, no command.

CREATE TABLE retired_ids (
    id TEXT PRIMARY KEY NOT NULL
) WITHOUT ROWID;

CREATE TRIGGER retire_deleted_paste_id
AFTER DELETE ON pastes
BEGIN
    INSERT OR IGNORE INTO retired_ids (id) VALUES (OLD.id);
END;
