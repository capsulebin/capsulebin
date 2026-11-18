-- The describe capability (crypto profile v1).
--
-- `describe_verifier` is SHA-256 of the describe token, which the client
-- derives from the link secret and the paste id. Reading a paste's descriptor
-- (public header and lifecycle) requires it, so that knowing a paste id alone
-- does not reveal whether the paste exists, when it expires, or whether it is
-- burn-after-retrieval or password-protected.
--
-- Nullable only because SQLite cannot add a NOT NULL column without a default
-- to a table that may hold rows. New pastes always carry one; a row without
-- one has no describe capability and answers like an unknown id.

ALTER TABLE pastes ADD COLUMN describe_verifier BLOB;
