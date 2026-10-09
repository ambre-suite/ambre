-- `SRV-126` to `SRV-135`: a snapshot is pending until it is confirmed.
--
-- `pending`: uploaded, and confirmed by nobody. It is never served to an open
-- and never moves a trim. `self`: confirmed by its own author, served to an
-- open beside the reference snapshot. `confirmed`: confirmed by another
-- member, the only status that moves the base trim.
--
-- Every snapshot written before this migration was served and trusted, so
-- it keeps that standing as `confirmed`. A new one starts `pending`.
ALTER TABLE snapshots ADD COLUMN status TEXT NOT NULL DEFAULT 'confirmed'
    CHECK (status IN ('pending', 'self', 'confirmed'));
ALTER TABLE snapshots ALTER COLUMN status SET DEFAULT 'pending';

-- The latest served snapshot of a document, asked on every open.
CREATE INDEX idx_snapshots_served ON snapshots (document_id, created_at DESC)
    WHERE status <> 'pending';
