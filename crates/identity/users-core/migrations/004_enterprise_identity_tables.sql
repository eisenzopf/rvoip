CREATE TABLE IF NOT EXISTS external_identities (
    provider_id TEXT NOT NULL,
    external_subject TEXT NOT NULL,
    user_id TEXT NOT NULL,
    email TEXT,
    username TEXT,
    display_name TEXT,
    groups_json TEXT NOT NULL DEFAULT '[]',
    active BOOLEAN NOT NULL DEFAULT TRUE,
    last_seen_at TIMESTAMP,
    created_at TIMESTAMP NOT NULL,
    updated_at TIMESTAMP NOT NULL,
    PRIMARY KEY (provider_id, external_subject),
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_external_identities_user_id
    ON external_identities(user_id);

CREATE INDEX IF NOT EXISTS idx_external_identities_provider_user
    ON external_identities(provider_id, user_id);

CREATE TABLE IF NOT EXISTS passkey_credentials (
    credential_id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    public_key TEXT NOT NULL,
    sign_count INTEGER NOT NULL DEFAULT 0,
    transports_json TEXT NOT NULL DEFAULT '[]',
    backup_eligible BOOLEAN NOT NULL DEFAULT FALSE,
    backup_state BOOLEAN NOT NULL DEFAULT FALSE,
    display_name TEXT,
    created_at TIMESTAMP NOT NULL,
    last_used_at TIMESTAMP,
    FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_passkey_credentials_user_id
    ON passkey_credentials(user_id);
