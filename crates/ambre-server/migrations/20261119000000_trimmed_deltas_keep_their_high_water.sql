-- The trim keeps the highest clock and number of each author it deletes
-- (`SRV-288`, audit `W68`).
--
-- A reset of the ephemeral store loses the per-author clocks and the delta
-- counter. A subscribe reseeds them from the durable rows. Once the
-- compaction pass has trimmed every delta of an author, no `updates` row
-- holds that author's clock any more. The counter then restarts at 1, and
-- the next pass deletes the new delta under the old horizon.
--
-- One row per author of a document, raised and never lowered. The relay
-- already held both numbers in the rows it trimmed, so it learns nothing new.
CREATE TABLE trimmed_high_water (
    document_id   UUID NOT NULL,
    author_pubkey BYTEA NOT NULL,
    max_clock     BIGINT NOT NULL,
    max_seq       BIGINT NOT NULL,
    CONSTRAINT trimmed_high_water_pkey PRIMARY KEY (document_id, author_pubkey),
    CONSTRAINT trimmed_high_water_document_id_fkey FOREIGN KEY (document_id)
        REFERENCES documents (id) ON DELETE CASCADE
);
