-- `SRV-104`: the signed membership log of a document.
--
-- Each entry records one invite, revoke or role change, signed by the member
-- who made it (`SYNC-132`). The relay checks the signature and stores the
-- bytes beside the member rows, in the transaction that changes them. It
-- computes no digest: a client walks the chain. `position` is the entry's
-- place from zero, and an entry lands at the next place alone, so two racing
-- writers never fork the log. An entry is never rewritten (`SYNC-107`), and
-- lives as long as its document.
CREATE TABLE document_membership_log (
    document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    position BIGINT NOT NULL,
    entry BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (document_id, position)
);
