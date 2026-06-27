-- Add `covered_seq` to snapshots: the highest delta `seq` folded into the
-- snapshot (the author's last received seq at upload time). On open, a client
-- that downloads the snapshot requests only deltas AFTER this seq, instead of
-- replaying the whole history since genesis (cold-start gating).
--
-- DEFAULT 0 keeps pre-existing snapshots safe: a 0 means "covers nothing", so
-- the client falls back to a full replay (the prior behavior) for old rows.
ALTER TABLE snapshots ADD COLUMN IF NOT EXISTS covered_seq BIGINT NOT NULL DEFAULT 0;
