-- The move of an inline delta to its batch object is a heap-only update.
--
-- The move clears `wire` and sets `batch_key`. Postgres writes an update as a
-- heap-only tuple only when no index reads a column it changes, a predicate
-- column included, and when the new version fits on the page of the old one.
-- `idx_updates_inline` read `wire` in its predicate, and `idx_updates_batch`
-- read `batch_key`. So each move wrote a new entry in every index of the
-- table, and cost as much as the insert.
--
-- Both indexes go. The inline tail count reads the primary key range of the
-- document past its newest batch row. The trim asks whether a batch is still
-- named within the range of its own document.
--
-- The fillfactor keeps half of each new page free. Inside the move
-- transaction, the old and the new version of every moved row both hold room
-- on the page. When the page holds the rows of one document alone, a move
-- takes three quarters of them at once, and its new versions are smaller than
-- the old ones only for a frame of about 100 bytes or more. The setting
-- applies to the pages written from now on.
DROP INDEX idx_updates_inline;
DROP INDEX idx_updates_batch;
ALTER TABLE updates SET (fillfactor = 50);
