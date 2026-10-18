-- The fold of an amendments document into its segment (`ambre-spec-sequencer.md
-- section 3`). The relay arbitrates which snapshot folds the amendments and
-- when the document may go, so it holds the fold. It learns that a fold
-- happened and how far it reached, never what it changed.
ALTER TABLE channel_documents
    -- The snapshot of the segment that folds this document in. A dropped
    -- snapshot clears it, and the fold is owed again.
    ADD COLUMN folded_by UUID REFERENCES snapshots (id) ON DELETE SET NULL,
    -- The last delta number of this document the fold holds.
    ADD COLUMN folded_seq BIGINT,
    -- When the fold was committed, by the relay's clock.
    ADD COLUMN folded_at TIMESTAMPTZ;

ALTER TABLE channel_documents
    ADD CONSTRAINT channel_documents_fold_of_amendments
    CHECK (kind = 'amendments' OR (folded_by IS NULL AND folded_seq IS NULL AND folded_at IS NULL));
