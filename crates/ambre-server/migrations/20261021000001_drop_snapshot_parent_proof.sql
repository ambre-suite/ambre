-- No client builds a snapshot's parent proof, and the relay now refuses one
-- (`SRV-228`): the column only ever held NULL and nothing read it. Once the
-- proof chain exists it travels inside the signed snapshot, which the relay
-- does not read, so the column goes.
ALTER TABLE snapshots DROP COLUMN IF EXISTS parent_snapshot_proof;
