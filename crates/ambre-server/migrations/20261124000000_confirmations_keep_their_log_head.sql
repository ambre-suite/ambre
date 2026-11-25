-- A confirmation keeps the log head its confirmer signed (`AUTHOR-038`).
--
-- A confirmer whose walk readmits a writer signs the head of that walk. An
-- open then admits a snapshot that folds the writer past the lifted cap
-- (`SYNC-317`). The relay already reads every log head in the public data of
-- an update, so it learns nothing new.
--
-- A head needs a confirmation, and a last entry needs a place. A row
-- confirmed before this migration has no head, and an open reads its
-- confirmation as version 2.
ALTER TABLE snapshots
    ADD COLUMN confirm_head_position BIGINT,
    ADD COLUMN confirm_head_digest BYTEA,
    ADD CONSTRAINT snapshots_confirm_head_check
        CHECK (confirm_head_position IS NULL OR confirm_signature IS NOT NULL),
    ADD CONSTRAINT snapshots_confirm_head_digest_check
        CHECK (confirm_head_digest IS NULL OR confirm_head_position IS NOT NULL);
