CREATE TABLE IF NOT EXISTS rvoip_vcons (
    uuid UUID PRIMARY KEY,
    handle_url TEXT UNIQUE,
    tenant_id TEXT,
    conversation_id TEXT,
    session_id TEXT,
    vcon JSONB,
    vcon_bytes BYTEA,
    vcon_jws BYTEA,
    content_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (vcon IS NOT NULL OR vcon_bytes IS NOT NULL OR vcon_jws IS NOT NULL)
);

ALTER TABLE rvoip_vcons
    ADD COLUMN IF NOT EXISTS conversation_id TEXT;

ALTER TABLE rvoip_vcons
    ADD COLUMN IF NOT EXISTS vcon_bytes BYTEA;

ALTER TABLE rvoip_vcons
    DROP CONSTRAINT IF EXISTS rvoip_vcons_check;

ALTER TABLE rvoip_vcons
    DROP CONSTRAINT IF EXISTS rvoip_vcons_payload_check;

ALTER TABLE rvoip_vcons
    ADD CONSTRAINT rvoip_vcons_payload_check
    CHECK (vcon IS NOT NULL OR vcon_bytes IS NOT NULL OR vcon_jws IS NOT NULL);

CREATE INDEX IF NOT EXISTS rvoip_vcons_session_idx
    ON rvoip_vcons (session_id);

CREATE INDEX IF NOT EXISTS rvoip_vcons_conversation_idx
    ON rvoip_vcons (conversation_id);
