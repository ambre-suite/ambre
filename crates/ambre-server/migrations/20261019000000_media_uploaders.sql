-- `SRV-138`: a media row records every member who uploaded its blob.
--
-- An address is content-derived, so two members who attach the same file to one
-- document confirm one object. The `uploaded_by` column names the first of them
-- alone, and an erase checked against it would let the first member remove a
-- file the second still holds. One claim per uploader keeps the object until the
-- last of them erases it (`SRV-140`, `SRV-141`).
CREATE TABLE IF NOT EXISTS media_uploaders (
    document_id UUID NOT NULL,
    id          TEXT NOT NULL,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    PRIMARY KEY (document_id, id, user_id),
    FOREIGN KEY (document_id, id) REFERENCES media(document_id, id) ON DELETE CASCADE
);

INSERT INTO media_uploaders (document_id, id, user_id)
SELECT document_id, id, uploaded_by FROM media
ON CONFLICT DO NOTHING;
