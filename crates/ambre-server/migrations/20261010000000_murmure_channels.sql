-- Murmure channels as sets of documents (`ambre-spec-server.md section 3`).
--
-- The initial schema declared a channel with its own members, its own wrapped
-- key and a sealed name, and a read cursor per channel. No route and no
-- handler ever wrote them. The design replaces all three: a channel grants
-- access through its documents (`SRV-103`), its members are the member rows of
-- its `meta` document, and its name lives in that document. The tables are
-- empty, so they are dropped rather than migrated.
DROP TABLE IF EXISTS channel_read_cursors;
DROP TABLE IF EXISTS channel_members;
DROP TABLE IF EXISTS channels;

-- A channel: its space and its public flag, the self-join's inputs. A direct
-- message is a channel with no space. Nothing else: `ZK-015` bounds what the
-- relay learns of a channel.
CREATE TABLE channels (
    id        UUID PRIMARY KEY,
    space_id  UUID REFERENCES documents(id) ON DELETE CASCADE,
    is_public BOOLEAN NOT NULL DEFAULT false
);

-- A channel document: the channel, the document and its kind. Keyed by the
-- document, so a document belongs to one channel at most.
CREATE TABLE channel_documents (
    document_id UUID PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
    channel_id  UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('meta', 'segment', 'amendments', 'thread'))
);

-- One `meta` document per channel: the access check resolves every document
-- of the channel to it.
CREATE UNIQUE INDEX channel_documents_one_meta
    ON channel_documents (channel_id) WHERE kind = 'meta';

CREATE INDEX channel_documents_by_channel ON channel_documents (channel_id);
