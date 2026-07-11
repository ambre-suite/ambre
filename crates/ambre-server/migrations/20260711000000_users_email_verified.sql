-- #3: distinguish a provider-verified email from a self-asserted one.
--
-- `/auth/setup` lets an unauthenticated caller claim ANY email (the passphrase
-- signup asserts its own address, unverified). Combined with the global
-- first-claimant-wins uniqueness (N5), this let anyone permanently lock a known
-- email out of OIDC login by pre-registering it -- an unauthenticated
-- denial-of-service, and an email-squat for share targeting.
--
-- Policy (settled 2026-07-11, supersedes the N5 "always 409" note for the
-- verified-vs-unverified case): a provider-verified email wins over a
-- self-asserted one. An OIDC login RECLAIMS an email currently held by an
-- UNVERIFIED account (that account's email is unbound to a unique sentinel);
-- a collision between two VERIFIED identities still surfaces 409. See
-- `db/users.rs::upsert_user_from_oidc`.
--
-- Backfill: existing OIDC and dev rows carry a provider-authenticated email, so
-- they are verified; existing `/auth/setup` rows (oidc_provider IS NULL) are
-- self-asserted, so they stay unverified and become reclaimable.
ALTER TABLE users ADD COLUMN email_verified BOOLEAN NOT NULL DEFAULT false;

UPDATE users SET email_verified = true WHERE oidc_provider IS NOT NULL;
