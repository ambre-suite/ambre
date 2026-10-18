-- The thread document of a root message (`ambre-spec-sequencer.md section 3`).
-- The relay arbitrates which document takes the replies to a message, so it
-- holds the link. It learns the root's identifier, a random value, never what
-- the root or a reply says.
ALTER TABLE channel_documents
    -- The root message a thread document gathers the replies of. Only that
    -- kind carries one.
    ADD COLUMN thread_root UUID;

ALTER TABLE channel_documents
    ADD CONSTRAINT channel_documents_root_of_threads
    CHECK ((kind = 'thread') = (thread_root IS NOT NULL));

-- At most one thread document per root message of a channel (`SRV-214`).
CREATE UNIQUE INDEX channel_documents_one_thread
    ON channel_documents (channel_id, thread_root) WHERE thread_root IS NOT NULL;
