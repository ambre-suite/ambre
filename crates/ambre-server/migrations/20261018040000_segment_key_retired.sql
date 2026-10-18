-- A segment sealed under a channel key that a rotation replaced
-- (`ambre-spec-sequencer.md section 3`). The relay locks such a segment once
-- it is closing (`SRV-147`), so a later edit goes to an amendment under the
-- new key. It learns that a rotation happened, which it serves anyway.
ALTER TABLE channel_documents
    ADD COLUMN key_retired BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE channel_documents
    ADD CONSTRAINT channel_documents_key_retired_of_segments
    CHECK (kind = 'segment' OR NOT key_retired);
