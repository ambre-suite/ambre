-- An escrowed master key belongs to one organization membership (`SRV-305`,
-- audit `P56`).
--
-- The escrow column sat on the user. An owner of any organization could add
-- a user and ask the recovery route for that user, and read the copy escrowed
-- for another organization. The copy now sits on the membership it was made
-- for, and a recovery reads it there alone.
--
-- Nothing wrote the old column, so no copy moves.
ALTER TABLE organization_members ADD COLUMN master_key_enc BYTEA;
ALTER TABLE users DROP COLUMN org_master_key_enc;
