-- A snapshot's `parent_hash` was never sent by a client nor read by the relay:
-- the column only ever held NULL. The request field and the insert are gone,
-- so the column goes too.
ALTER TABLE snapshots DROP COLUMN IF EXISTS parent_hash;
