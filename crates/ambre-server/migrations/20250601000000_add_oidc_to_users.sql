-- Add OIDC provider/subject columns and make key columns nullable
-- for users created via OIDC before key setup.

ALTER TABLE users ADD COLUMN IF NOT EXISTS oidc_provider TEXT;
ALTER TABLE users ADD COLUMN IF NOT EXISTS oidc_subject TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS users_oidc_unique ON users (oidc_provider, oidc_subject);

ALTER TABLE users ALTER COLUMN master_key_enc DROP NOT NULL;
ALTER TABLE users ALTER COLUMN master_key_salt DROP NOT NULL;
