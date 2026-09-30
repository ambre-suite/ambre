-- `SRV-058`: a media row is keyed by its document and its address together.
--
-- The address alone was the primary key, so the same file confirmed in a second
-- document found the first document's row and wrote none of its own. A download
-- link that reads the media index needs one row per document that holds the
-- object, so the key becomes the pair.
ALTER TABLE media DROP CONSTRAINT media_pkey;
ALTER TABLE media ADD PRIMARY KEY (document_id, id);
