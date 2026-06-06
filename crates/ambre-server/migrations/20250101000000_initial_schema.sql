-- Ambre Blind Relay — initial schema

-- Users
CREATE TABLE IF NOT EXISTS users (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email           TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    master_key_enc  BYTEA NOT NULL,
    master_key_salt BYTEA NOT NULL,
    signing_pubkey  BYTEA NOT NULL,
    exchange_pubkey BYTEA NOT NULL,
    recovery_hint   TEXT,
    org_id          UUID,
    org_master_key_enc BYTEA,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Organizations
CREATE TABLE IF NOT EXISTS organizations (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name             TEXT NOT NULL,
    org_pubkey       BYTEA NOT NULL,
    shamir_threshold SMALLINT NOT NULL DEFAULT 3,
    shamir_total     SMALLINT NOT NULL DEFAULT 5,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS organization_members (
    org_id           UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id          UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role             TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    shamir_share_enc BYTEA,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (org_id, user_id)
);

CREATE TABLE IF NOT EXISTS org_recovery_log (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id         UUID NOT NULL REFERENCES organizations(id),
    target_user_id UUID NOT NULL REFERENCES users(id),
    recovered_by   UUID NOT NULL REFERENCES users(id),
    reason         TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Documents
CREATE TABLE IF NOT EXISTS documents (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id   UUID NOT NULL REFERENCES users(id),
    doc_type   TEXT NOT NULL CHECK (doc_type IN ('prose', 'trame', 'toile', 'murmure')),
    name_enc   BYTEA,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ACL documents
CREATE TABLE IF NOT EXISTS document_members (
    document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    user_id     UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role        TEXT NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    doc_key_enc BYTEA NOT NULL,
    invited_by  UUID REFERENCES users(id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (document_id, user_id)
);

-- Snapshots
CREATE TABLE IF NOT EXISTS snapshots (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id          UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    author_pubkey        BYTEA NOT NULL,
    parent_hash          BYTEA,
    parent_snapshot_proof BYTEA,
    update_clocks        JSONB NOT NULL DEFAULT '{}',
    s3_key               TEXT NOT NULL,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Updates (deltas)
CREATE TABLE IF NOT EXISTS updates (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id   UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    author_pubkey BYTEA NOT NULL,
    snapshot_id   UUID NOT NULL REFERENCES snapshots(id),
    clock         BIGINT NOT NULL,
    seq           BIGINT NOT NULL,
    s3_key        TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (document_id, author_pubkey, clock)
);
CREATE INDEX IF NOT EXISTS idx_updates_delta_sync ON updates (document_id, seq);

-- Media
CREATE TABLE IF NOT EXISTS media (
    id          TEXT PRIMARY KEY,
    document_id UUID NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    uploaded_by UUID NOT NULL REFERENCES users(id),
    mime_type   TEXT NOT NULL,
    size_bytes  BIGINT NOT NULL,
    s3_key      TEXT NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Channels
CREATE TABLE IF NOT EXISTS channels (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id   UUID NOT NULL REFERENCES users(id),
    name_enc   BYTEA,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS channel_members (
    channel_id      UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role            TEXT NOT NULL CHECK (role IN ('owner', 'admin', 'member')),
    channel_key_enc BYTEA NOT NULL,
    joined_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (channel_id, user_id)
);

-- Read cursors
CREATE TABLE IF NOT EXISTS channel_read_cursors (
    user_id    UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    channel_id UUID NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    last_seq   BIGINT NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, channel_id)
);

-- Meetings
CREATE TABLE IF NOT EXISTS meetings (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    creator_id UUID NOT NULL REFERENCES users(id),
    name       TEXT,
    status     TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'ended')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at   TIMESTAMPTZ
);
