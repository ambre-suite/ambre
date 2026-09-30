-- `SRV-163`: an account that has not registered its keys stores none.
--
-- A provider login creates the account before its keys exist, and the columns
-- were NOT NULL, so the insert wrote 32 zero bytes into each. The key lookup and
-- the user search then served those zeros as a real identity, and a document key
-- wrapped to a zero exchange key is readable by anyone. The columns become
-- nullable and hold NULL until the key setup writes both.
ALTER TABLE users ALTER COLUMN signing_pubkey DROP NOT NULL;
ALTER TABLE users ALTER COLUMN exchange_pubkey DROP NOT NULL;

-- Backfill: the zero placeholder only ever came from the provider insert, so
-- any account still holding it has no keys. A real public key is never zero.
UPDATE users
SET signing_pubkey = NULL, exchange_pubkey = NULL
WHERE signing_pubkey = decode(repeat('00', 32), 'hex')
  AND exchange_pubkey = decode(repeat('00', 32), 'hex');

-- Both keys come from one seed and are written together, so an account holds
-- both or neither.
ALTER TABLE users ADD CONSTRAINT users_keys_paired
    CHECK ((signing_pubkey IS NULL) = (exchange_pubkey IS NULL));
