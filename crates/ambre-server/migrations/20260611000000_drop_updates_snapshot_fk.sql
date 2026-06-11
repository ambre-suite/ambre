-- Drop the foreign key from `updates.snapshot_id` to `snapshots(id)`.
--
-- Snapshots are end-to-end encrypted client blobs. The server stores and
-- replays them but never dereferences `snapshot_id` to enforce graph
-- integrity. A brand-new document's first deltas reference a base snapshot
-- the client invents locally and never uploads, so the FK is structurally
-- unsatisfiable and silently rejected every such insert. Keep the column as
-- an opaque client-side correlation tag, without the constraint.
ALTER TABLE updates DROP CONSTRAINT IF EXISTS updates_snapshot_id_fkey;
