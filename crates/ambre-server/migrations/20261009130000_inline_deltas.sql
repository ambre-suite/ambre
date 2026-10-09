-- `SRV-148` to `SRV-155`: the hot tail of deltas lives in the index.
--
-- A delta under the inline size keeps its sealed frame in its own row, so a
-- replay reads a page of rows in one query. A larger delta keeps an object of
-- its own, which `s3_key` names. Past the inline cap, the oldest inline deltas
-- move into one batch object, which `batch_key` names. Each row has exactly one
-- of the three homes.
ALTER TABLE updates ALTER COLUMN s3_key DROP NOT NULL;
ALTER TABLE updates ADD COLUMN wire BYTEA;
ALTER TABLE updates ADD COLUMN batch_key TEXT;
ALTER TABLE updates ADD CONSTRAINT updates_one_home CHECK (
    (wire IS NOT NULL)::int + (batch_key IS NOT NULL)::int + (s3_key IS NOT NULL)::int = 1
);

-- The inline tail of a document, counted on every write.
CREATE INDEX idx_updates_inline ON updates (document_id, seq) WHERE wire IS NOT NULL;
-- Whether a batch object is still named by a row, asked after a trim.
CREATE INDEX idx_updates_batch ON updates (batch_key) WHERE batch_key IS NOT NULL;
