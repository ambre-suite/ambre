-- The spaces of a user (`ambre-spec-server.md section 3`). A member who joined
-- no channel of a space still lists it, so the relay holds which documents
-- are spaces. It learns that a document is a space, which the first channel of
-- the space already tells it, and nothing else: `ZK-015` bounds it.
CREATE TABLE spaces (
    id UUID PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE
);

-- Every space a channel already names is a space.
INSERT INTO spaces (id)
    SELECT DISTINCT space_id FROM channels WHERE space_id IS NOT NULL
    ON CONFLICT DO NOTHING;
