CREATE TABLE IF NOT EXISTS rvoip_vcons (
    uuid UUID PRIMARY KEY,
    handle_url TEXT UNIQUE,
    tenant_id TEXT,
    session_id TEXT,
    vcon JSONB,
    vcon_jws BYTEA,
    content_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (vcon IS NOT NULL OR vcon_jws IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS rvoip_vcons_session_idx
    ON rvoip_vcons (session_id);
