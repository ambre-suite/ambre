-- The state of each segment of a channel (`ambre-spec-server.md section 3`):
-- `active`, `closing` or `final`. The relay arbitrates a close, so it holds
-- the state that decides, and `ZK-015` lets it learn it. Every other kind of
-- channel document carries none.
ALTER TABLE channel_documents
    ADD COLUMN state TEXT CHECK (state IN ('active', 'closing', 'final'));

-- Every segment registered so far is the first of its channel, so active.
UPDATE channel_documents SET state = 'active' WHERE kind = 'segment';

ALTER TABLE channel_documents
    ADD CONSTRAINT channel_documents_state_of_segments
    CHECK ((kind = 'segment') = (state IS NOT NULL));

-- At most one active and one closing segment per channel (`MURMURE-010`).
CREATE UNIQUE INDEX channel_documents_one_active
    ON channel_documents (channel_id) WHERE state = 'active';
CREATE UNIQUE INDEX channel_documents_one_closing
    ON channel_documents (channel_id) WHERE state = 'closing';
