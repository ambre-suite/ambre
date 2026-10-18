-- The finalization of a segment (`ambre-spec-sequencer.md section 3`). The
-- relay arbitrates who writes the final snapshot, so it holds the close time,
-- the lock, the lease and the finalization, which `ZK-015` lets it learn.
ALTER TABLE channel_documents
    -- When the segment became `closing`, by the relay's clock.
    ADD COLUMN closed_at TIMESTAMPTZ,
    -- The last delta number the lock let in: the horizon of the final
    -- snapshot. Set once, when a lease first locks the segment for good.
    ADD COLUMN horizon BIGINT,
    -- The live lease, if any: its identifier and its end.
    ADD COLUMN lease_id UUID,
    ADD COLUMN lease_until TIMESTAMPTZ,
    -- The accepted finalization: its time and its snapshot.
    ADD COLUMN finalized_at TIMESTAMPTZ,
    ADD COLUMN final_snapshot UUID;

-- A segment closing before this migration starts its grace period now.
UPDATE channel_documents SET closed_at = now() WHERE state = 'closing';
