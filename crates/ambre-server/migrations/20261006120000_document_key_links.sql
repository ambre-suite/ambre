-- `SYNC-116` to `SYNC-118`: the links a document's rotations leave.
--
-- Each link seals the key a rotation replaced under the key it installed. The
-- relay stores the bytes and the two identifiers they name, and opens nothing.
-- A link lives as long as its document (`SYNC-117`), and arrives in the same
-- transaction as the wrapped copies (`SYNC-118`). `seq` keeps the order the
-- rotations landed in.
CREATE TABLE document_key_links (
    document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    seq BIGSERIAL NOT NULL,
    key_id BYTEA NOT NULL,
    previous_key_id BYTEA NOT NULL,
    link BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (document_id, key_id)
);

CREATE INDEX document_key_links_by_seq ON document_key_links (document_id, seq);
