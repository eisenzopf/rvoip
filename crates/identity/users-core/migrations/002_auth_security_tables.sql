-- Auth/security tables added after the initial users-core schema.
--
-- This migration is idempotent so existing databases that were previously
-- initialized by the one-shot schema loader can be safely upgraded.

CREATE TABLE IF NOT EXISTS revoked_access_tokens (
    jti TEXT PRIMARY KEY,
    user_id TEXT,
    expires_at TIMESTAMP NOT NULL,
    revoked_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sip_digest_credentials (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    sip_username TEXT NOT NULL,
    realm TEXT NOT NULL,
    algorithm TEXT NOT NULL,
    ha1 TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(sip_username, realm, algorithm),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_revoked_access_tokens_expires_at
    ON revoked_access_tokens(expires_at);

CREATE INDEX IF NOT EXISTS idx_sip_digest_credentials_user_id
    ON sip_digest_credentials(user_id);

CREATE INDEX IF NOT EXISTS idx_sip_digest_credentials_lookup
    ON sip_digest_credentials(sip_username, realm, algorithm);
