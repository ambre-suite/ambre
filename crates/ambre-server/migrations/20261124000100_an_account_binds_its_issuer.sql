-- A provider account is keyed by the issuer and the subject it asserts
-- (`SRV-344`), the one pair OpenID Connect makes unique.
--
-- A subject is unique within one issuer alone. Keyed by the provider word and
-- the subject, two issuers that assert one subject would sign two people in
-- to one account: a relay that changes its issuer, or a `google` provider
-- discovered at another issuer (`SRV-322`).
--
-- Every `google` account so far was signed in through Google's own issuer,
-- the one issuer a deployment reached. A development account names no
-- issuer, and keeps its own index on its name.
ALTER TABLE users ADD COLUMN oidc_issuer TEXT;

UPDATE users SET oidc_issuer = 'https://accounts.google.com' WHERE oidc_provider = 'google';

ALTER TABLE users ADD CONSTRAINT users_provider_names_its_issuer
    CHECK (oidc_provider IS NULL OR oidc_provider = 'dev' OR oidc_issuer IS NOT NULL);

DROP INDEX users_oidc_unique;
CREATE UNIQUE INDEX users_oidc_unique ON users (oidc_issuer, oidc_subject);
CREATE UNIQUE INDEX users_dev_unique ON users (oidc_subject) WHERE oidc_provider = 'dev';
