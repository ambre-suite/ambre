-- `ZK-018`: the account row holds what the relay may learn of a user, and no more.
--
-- No code wrote the recovery hint, and none read the organization column: the
-- membership of an organization lives in `organization_members`. The two
-- timestamps told the relay when an account was created and when its keys
-- last changed, which no route needs.
ALTER TABLE users DROP COLUMN recovery_hint;
ALTER TABLE users DROP COLUMN org_id;
ALTER TABLE users DROP COLUMN created_at;
ALTER TABLE users DROP COLUMN updated_at;
