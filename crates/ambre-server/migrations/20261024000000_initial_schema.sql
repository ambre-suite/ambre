-- Ambre relay: the initial schema.
--
-- Ambre has not shipped, so this file creates the final state directly. The
-- first change after the release adds a new file beside it, never an edit.
-- The relay stores ciphertext and routing metadata only: no column holds a
-- plaintext document, title or key.

-- An account. A provider login creates it before its keys exist, so the keys
-- stay null until the key setup writes both public keys together (`SRV-163`).
CREATE TABLE users (
    id                 UUID NOT NULL DEFAULT gen_random_uuid(),
    email              TEXT NOT NULL,
    display_name       TEXT NOT NULL,
    master_key_enc     BYTEA,
    master_key_salt    BYTEA,
    signing_pubkey     BYTEA,
    exchange_pubkey    BYTEA,
    org_master_key_enc BYTEA,
    oidc_provider      TEXT,
    oidc_subject       TEXT,
    email_verified     BOOLEAN NOT NULL DEFAULT false,
    CONSTRAINT users_pkey PRIMARY KEY (id),
    CONSTRAINT users_keys_paired CHECK ((signing_pubkey IS NULL) = (exchange_pubkey IS NULL))
);

-- One account per address, whatever its case.
CREATE UNIQUE INDEX users_email_lower_unique ON users (lower(email));
CREATE UNIQUE INDEX users_oidc_unique ON users (oidc_provider, oidc_subject);

CREATE TABLE organizations (
    id               UUID NOT NULL DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL,
    org_pubkey       BYTEA NOT NULL,
    shamir_threshold SMALLINT NOT NULL DEFAULT 3,
    shamir_total     SMALLINT NOT NULL DEFAULT 5,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT organizations_pkey PRIMARY KEY (id)
);

CREATE TABLE organization_members (
    org_id           UUID NOT NULL,
    user_id          UUID NOT NULL,
    role             TEXT NOT NULL,
    shamir_share_enc BYTEA,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT organization_members_pkey PRIMARY KEY (org_id, user_id),
    CONSTRAINT organization_members_org_id_fkey FOREIGN KEY (org_id)
        REFERENCES organizations (id) ON DELETE CASCADE,
    CONSTRAINT organization_members_user_id_fkey FOREIGN KEY (user_id)
        REFERENCES users (id) ON DELETE CASCADE,
    CONSTRAINT organization_members_role_check CHECK (role IN ('owner', 'admin', 'member'))
);

CREATE TABLE org_recovery_log (
    id             UUID NOT NULL DEFAULT gen_random_uuid(),
    org_id         UUID NOT NULL,
    target_user_id UUID NOT NULL,
    recovered_by   UUID NOT NULL,
    reason         TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT org_recovery_log_pkey PRIMARY KEY (id),
    CONSTRAINT org_recovery_log_org_id_fkey FOREIGN KEY (org_id)
        REFERENCES organizations (id),
    CONSTRAINT org_recovery_log_target_user_id_fkey FOREIGN KEY (target_user_id)
        REFERENCES users (id),
    CONSTRAINT org_recovery_log_recovered_by_fkey FOREIGN KEY (recovered_by)
        REFERENCES users (id)
);

-- A document. Its name is sealed. `rotation_owed` marks a removal posted
-- without a rotation, and the relay refuses every frame until one lands
-- (`SRV-122`, `SRV-123`).
CREATE TABLE documents (
    id            UUID NOT NULL DEFAULT gen_random_uuid(),
    owner_id      UUID NOT NULL,
    doc_type      TEXT NOT NULL,
    name_enc      BYTEA,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    rotation_owed BOOLEAN NOT NULL DEFAULT false,
    CONSTRAINT documents_pkey PRIMARY KEY (id),
    CONSTRAINT documents_owner_id_fkey FOREIGN KEY (owner_id) REFERENCES users (id),
    CONSTRAINT documents_doc_type_check
        CHECK (doc_type IN ('prose', 'trame', 'toile', 'murmure'))
);

-- A member holds the document key, wrapped to its own exchange key.
CREATE TABLE document_members (
    document_id UUID NOT NULL,
    user_id     UUID NOT NULL,
    role        TEXT NOT NULL,
    doc_key_enc BYTEA NOT NULL,
    invited_by  UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT document_members_pkey PRIMARY KEY (document_id, user_id),
    CONSTRAINT document_members_document_id_fkey FOREIGN KEY (document_id)
        REFERENCES documents (id) ON DELETE CASCADE,
    CONSTRAINT document_members_user_id_fkey FOREIGN KEY (user_id)
        REFERENCES users (id) ON DELETE CASCADE,
    CONSTRAINT document_members_invited_by_fkey FOREIGN KEY (invited_by)
        REFERENCES users (id),
    CONSTRAINT document_members_role_check CHECK (role IN ('owner', 'editor', 'viewer'))
);

-- A sealed snapshot in the object store. The relay serves only one a member
-- has confirmed, or one its author holds for itself.
CREATE TABLE snapshots (
    id            UUID NOT NULL DEFAULT gen_random_uuid(),
    document_id   UUID NOT NULL,
    author_pubkey BYTEA NOT NULL,
    s3_key        TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    covered_seq   BIGINT NOT NULL DEFAULT 0,
    status        TEXT NOT NULL DEFAULT 'pending',
    CONSTRAINT snapshots_pkey PRIMARY KEY (id),
    CONSTRAINT snapshots_document_id_fkey FOREIGN KEY (document_id)
        REFERENCES documents (id) ON DELETE CASCADE,
    CONSTRAINT snapshots_status_check CHECK (status IN ('pending', 'self', 'confirmed'))
);

CREATE INDEX idx_snapshots_served ON snapshots (document_id, created_at DESC)
    WHERE status <> 'pending';

-- A sealed delta. It lives in exactly one place: inline, in a batch object, or
-- in an object of its own. `snapshot_id` carries no foreign key: a new
-- document's first deltas name a base snapshot that is never uploaded.
CREATE TABLE updates (
    id            UUID NOT NULL DEFAULT gen_random_uuid(),
    document_id   UUID NOT NULL,
    author_pubkey BYTEA NOT NULL,
    snapshot_id   UUID NOT NULL,
    clock         BIGINT NOT NULL,
    seq           BIGINT NOT NULL,
    s3_key        TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    wire          BYTEA,
    batch_key     TEXT,
    CONSTRAINT updates_pkey PRIMARY KEY (id),
    CONSTRAINT updates_document_id_author_pubkey_clock_key
        UNIQUE (document_id, author_pubkey, clock),
    CONSTRAINT updates_document_id_fkey FOREIGN KEY (document_id)
        REFERENCES documents (id) ON DELETE CASCADE,
    CONSTRAINT updates_one_home CHECK (
        (wire IS NOT NULL)::integer + (batch_key IS NOT NULL)::integer
            + (s3_key IS NOT NULL)::integer = 1
    )
);

CREATE INDEX idx_updates_delta_sync ON updates (document_id, seq);
CREATE INDEX idx_updates_inline ON updates (document_id, seq) WHERE wire IS NOT NULL;
CREATE INDEX idx_updates_batch ON updates (batch_key) WHERE batch_key IS NOT NULL;

CREATE TABLE meetings (
    id         UUID NOT NULL DEFAULT gen_random_uuid(),
    creator_id UUID NOT NULL,
    name       TEXT,
    status     TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at   TIMESTAMPTZ,
    CONSTRAINT meetings_pkey PRIMARY KEY (id),
    CONSTRAINT meetings_creator_id_fkey FOREIGN KEY (creator_id) REFERENCES users (id),
    CONSTRAINT meetings_status_check CHECK (status IN ('active', 'ended'))
);

-- A sealed media object. The key is the document and the address together,
-- so each document that holds one object has a row of its own (`SRV-058`).
CREATE TABLE media (
    id          TEXT NOT NULL,
    document_id UUID NOT NULL,
    uploaded_by UUID NOT NULL,
    size_bytes  BIGINT NOT NULL,
    s3_key      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT media_pkey PRIMARY KEY (document_id, id),
    CONSTRAINT media_document_id_fkey FOREIGN KEY (document_id)
        REFERENCES documents (id) ON DELETE CASCADE,
    CONSTRAINT media_uploaded_by_fkey FOREIGN KEY (uploaded_by) REFERENCES users (id)
);

-- Every member who uploaded one media object, not only the first. The object
-- stays until the last of them erases it (`SRV-138`).
CREATE TABLE media_uploaders (
    document_id UUID NOT NULL,
    id          TEXT NOT NULL,
    user_id     UUID NOT NULL,
    CONSTRAINT media_uploaders_pkey PRIMARY KEY (document_id, id, user_id),
    CONSTRAINT media_uploaders_document_id_id_fkey FOREIGN KEY (document_id, id)
        REFERENCES media (document_id, id) ON DELETE CASCADE,
    CONSTRAINT media_uploaders_user_id_fkey FOREIGN KEY (user_id)
        REFERENCES users (id) ON DELETE CASCADE
);

-- The links a rotation leaves: each seals the key it replaced under the key it
-- installed. `seq` keeps the order the rotations landed in (`SYNC-116`).
CREATE TABLE document_key_links (
    document_id     UUID NOT NULL,
    seq             BIGSERIAL NOT NULL,
    key_id          BYTEA NOT NULL,
    previous_key_id BYTEA NOT NULL,
    link            BYTEA NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT document_key_links_pkey PRIMARY KEY (document_id, key_id),
    CONSTRAINT document_key_links_document_id_fkey FOREIGN KEY (document_id)
        REFERENCES documents (id) ON DELETE CASCADE
);

CREATE INDEX document_key_links_by_seq ON document_key_links (document_id, seq);

-- A delete that has begun, and when its second sweep may run, in Unix seconds.
-- The row outlives its document, so it carries no foreign key (`SRV-165`).
CREATE TABLE document_deletions (
    document_id UUID NOT NULL,
    sweep_due   BIGINT NOT NULL,
    CONSTRAINT document_deletions_pkey PRIMARY KEY (document_id)
);

-- The signed membership log of a document, one entry per position from zero
-- (`SRV-104`). An entry is never rewritten.
CREATE TABLE document_membership_log (
    document_id UUID NOT NULL,
    "position"  BIGINT NOT NULL,
    entry       BYTEA NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT document_membership_log_pkey PRIMARY KEY (document_id, "position"),
    CONSTRAINT document_membership_log_document_id_fkey FOREIGN KEY (document_id)
        REFERENCES documents (id) ON DELETE CASCADE
);

-- The documents that are spaces, so a member who joined no channel of a space
-- still lists it (`ZK-015`).
CREATE TABLE spaces (
    id UUID NOT NULL,
    CONSTRAINT spaces_pkey PRIMARY KEY (id),
    CONSTRAINT spaces_id_fkey FOREIGN KEY (id) REFERENCES documents (id) ON DELETE CASCADE
);

CREATE TABLE channels (
    id        UUID NOT NULL,
    space_id  UUID,
    is_public BOOLEAN NOT NULL DEFAULT false,
    CONSTRAINT channels_pkey PRIMARY KEY (id),
    CONSTRAINT channels_space_id_fkey FOREIGN KEY (space_id)
        REFERENCES documents (id) ON DELETE CASCADE
);

-- One document of a channel: its meta, a segment, the amendments of a
-- segment, or a thread. Each kind carries only the columns it uses.
CREATE TABLE channel_documents (
    document_id    UUID NOT NULL,
    channel_id     UUID NOT NULL,
    kind           TEXT NOT NULL,
    state          TEXT,
    closed_at      TIMESTAMPTZ,
    horizon        BIGINT,
    lease_id       UUID,
    lease_until    TIMESTAMPTZ,
    finalized_at   TIMESTAMPTZ,
    final_snapshot UUID,
    amends         UUID,
    folded_by      UUID,
    folded_seq     BIGINT,
    folded_at      TIMESTAMPTZ,
    key_retired    BOOLEAN NOT NULL DEFAULT false,
    thread_root    UUID,
    CONSTRAINT channel_documents_pkey PRIMARY KEY (document_id),
    CONSTRAINT channel_documents_document_id_fkey FOREIGN KEY (document_id)
        REFERENCES documents (id) ON DELETE CASCADE,
    CONSTRAINT channel_documents_channel_id_fkey FOREIGN KEY (channel_id)
        REFERENCES channels (id) ON DELETE CASCADE,
    CONSTRAINT channel_documents_amends_fkey FOREIGN KEY (amends)
        REFERENCES channel_documents (document_id) ON DELETE CASCADE,
    CONSTRAINT channel_documents_folded_by_fkey FOREIGN KEY (folded_by)
        REFERENCES snapshots (id) ON DELETE SET NULL,
    CONSTRAINT channel_documents_kind_check
        CHECK (kind IN ('meta', 'segment', 'amendments', 'thread')),
    CONSTRAINT channel_documents_state_check CHECK (state IN ('active', 'closing', 'final')),
    CONSTRAINT channel_documents_state_of_segments CHECK ((kind = 'segment') = (state IS NOT NULL)),
    CONSTRAINT channel_documents_amends_of_amendments
        CHECK ((kind = 'amendments') = (amends IS NOT NULL)),
    CONSTRAINT channel_documents_fold_of_amendments CHECK (
        kind = 'amendments' OR (folded_by IS NULL AND folded_seq IS NULL AND folded_at IS NULL)
    ),
    CONSTRAINT channel_documents_key_retired_of_segments CHECK (kind = 'segment' OR NOT key_retired),
    CONSTRAINT channel_documents_root_of_threads
        CHECK ((kind = 'thread') = (thread_root IS NOT NULL))
);

CREATE INDEX channel_documents_by_channel ON channel_documents (channel_id);
CREATE UNIQUE INDEX channel_documents_one_meta ON channel_documents (channel_id)
    WHERE kind = 'meta';
CREATE UNIQUE INDEX channel_documents_one_active ON channel_documents (channel_id)
    WHERE state = 'active';
CREATE UNIQUE INDEX channel_documents_one_closing ON channel_documents (channel_id)
    WHERE state = 'closing';
CREATE UNIQUE INDEX channel_documents_one_amendments ON channel_documents (amends)
    WHERE amends IS NOT NULL;
CREATE UNIQUE INDEX channel_documents_one_thread ON channel_documents (channel_id, thread_root)
    WHERE thread_root IS NOT NULL;

-- How far a member has read one document.
CREATE TABLE read_cursors (
    user_id     UUID NOT NULL,
    document_id UUID NOT NULL,
    seq         BIGINT NOT NULL,
    CONSTRAINT read_cursors_pkey PRIMARY KEY (user_id, document_id),
    CONSTRAINT read_cursors_user_id_fkey FOREIGN KEY (user_id)
        REFERENCES users (id) ON DELETE CASCADE,
    CONSTRAINT read_cursors_document_id_fkey FOREIGN KEY (document_id)
        REFERENCES documents (id) ON DELETE CASCADE,
    CONSTRAINT read_cursors_seq_check CHECK (seq >= 0)
);

-- A call of a channel, and the pseudonym of its room on the SFU, which the
-- relay draws at the first join (`ambre-spec-call-media.md` section 3). The
-- relay learns the channel, the start and the end, and nothing else
-- (`ZK-022`).
CREATE TABLE calls (
    id         UUID        NOT NULL,
    channel_id UUID        NOT NULL,
    room       BYTEA       NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at   TIMESTAMPTZ,
    CONSTRAINT calls_pkey PRIMARY KEY (id),
    CONSTRAINT calls_channel_id_fkey FOREIGN KEY (channel_id)
        REFERENCES channels (id) ON DELETE CASCADE,
    CONSTRAINT calls_room_check CHECK (octet_length(room) = 16)
);

-- The roster of a call: one row per participant session, under its public
-- key, kept while its socket is open. Two sessions of one key are two rows,
-- and a leave removes one of them.
CREATE TABLE call_participants (
    call_id        UUID        NOT NULL,
    signing_pubkey BYTEA       NOT NULL,
    joined_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT call_participants_call_id_fkey FOREIGN KEY (call_id)
        REFERENCES calls (id) ON DELETE CASCADE,
    CONSTRAINT call_participants_signing_pubkey_check
        CHECK (octet_length(signing_pubkey) = 32)
);

CREATE INDEX call_participants_call_id_idx ON call_participants (call_id);
