-- A snapshot another member confirmed keeps the confirmation that member
-- signed (`SRV-284`, audit `N31`).
--
-- The confirmer signs the document, the snapshot, the horizon and the
-- author. The relay serves the signature with the snapshot, and a cold open
-- trusts the snapshot whole only on it. Every field the signature covers is
-- one the relay already holds, so the relay learns nothing new.
--
-- The two columns are set together, or not at all. A row confirmed before
-- this migration has neither, and an open checks it as its author's alone.
ALTER TABLE snapshots
    ADD COLUMN confirmer_pubkey BYTEA,
    ADD COLUMN confirm_signature BYTEA,
    ADD CONSTRAINT snapshots_confirmation_check
        CHECK ((confirmer_pubkey IS NULL) = (confirm_signature IS NULL));
