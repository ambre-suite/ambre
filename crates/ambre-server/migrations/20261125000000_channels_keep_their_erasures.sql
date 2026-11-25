-- An owner of a channel erases a thread name no document of the channel
-- answers (`SRV-347`, `SYNC-322`).
--
-- Any member sets the absent `thread` field of a root, and the relay cannot
-- read it. A field that names no document of the channel fails each removal,
-- since the census refuses a listing short of a thread it names. The owner
-- signs the channel and the thread, and the relay keeps the signature, so
-- each client checks the erasure itself. The relay learns the thread id and
-- the owner's key alone (`ZK-015`).
CREATE TABLE channel_erasures (
    channel_id    UUID NOT NULL,
    thread_id     UUID NOT NULL,
    signer_pubkey BYTEA NOT NULL,
    signature     BYTEA NOT NULL,
    CONSTRAINT channel_erasures_pkey PRIMARY KEY (channel_id, thread_id),
    CONSTRAINT channel_erasures_channel_id_fkey FOREIGN KEY (channel_id)
        REFERENCES channels (id) ON DELETE CASCADE
);
