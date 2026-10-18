-- The amendments document of a locked segment (`ambre-spec-sequencer.md
-- section 3`). The relay arbitrates which document takes the late edits of a
-- segment, so it holds the link. It learns that a segment was amended, never
-- what changed.
ALTER TABLE channel_documents
    -- The segment an amendments document amends. Only that kind carries one.
    ADD COLUMN amends UUID REFERENCES channel_documents (document_id) ON DELETE CASCADE;

ALTER TABLE channel_documents
    ADD CONSTRAINT channel_documents_amends_of_amendments
    CHECK ((kind = 'amendments') = (amends IS NOT NULL));

-- At most one amendments document per segment (`SRV-202`).
CREATE UNIQUE INDEX channel_documents_one_amendments
    ON channel_documents (amends) WHERE amends IS NOT NULL;
