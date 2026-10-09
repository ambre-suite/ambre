-- `SRV-165`: a delete that has begun completes.
--
-- The delete is marked here before anything goes, and the second sweep of its
-- objects is scheduled here (`SRV-057`). The ephemeral store held both before,
-- so a reset during the late-write window lost them. A row outlives the
-- document it names, so it carries no foreign key. The flusher's pass finishes
-- every delete still marked, and drops the row once the second sweep is done.
CREATE TABLE document_deletions (
    document_id UUID PRIMARY KEY,
    -- Unix seconds: when the late-write window has passed and the second
    -- sweep may run.
    sweep_due BIGINT NOT NULL
);
