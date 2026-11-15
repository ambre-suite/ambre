-- The delta index is keyed by its document and its relay sequence number.
--
-- A random key on each row wrote a random leaf page of its index on every
-- insert, and no query read it. After each checkpoint, that page went whole
-- into the write-ahead log. The pair (document_id, seq) already names a row:
-- the sequencer gives each accepted delta of a document its own number. It
-- grows within a document, so an insert writes the rightmost leaf of that
-- document's range.
--
-- The primary key takes the columns of `idx_updates_delta_sync`, so that index
-- goes: it repeated the key. A sequence number taken twice for one document
-- now fails the insert, where it used to store two rows under one number.
ALTER TABLE updates DROP CONSTRAINT updates_pkey;
ALTER TABLE updates DROP COLUMN id;
DROP INDEX idx_updates_delta_sync;
ALTER TABLE updates ADD CONSTRAINT updates_pkey PRIMARY KEY (document_id, seq);
