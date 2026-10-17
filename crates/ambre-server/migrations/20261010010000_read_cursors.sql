-- A read cursor: a member, a document and a number (`ambre-spec-server.md
-- section 3`). The number is a sequence number of the document, the one the
-- activity answer compares with the document's last (`SRV-109`). Keyed by the
-- user and never by a device, so every device of a user reads the same
-- cursor. Nothing else: `ZK-015` bounds what the relay learns of a channel.
CREATE TABLE read_cursors (
    user_id     UUID   NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    document_id UUID   NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    seq         BIGINT NOT NULL CHECK (seq >= 0),
    PRIMARY KEY (user_id, document_id)
);
