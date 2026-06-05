-- Add API-key suspension without deleting or revoking key records.
--
-- Disabled keys validate as absent while remaining visible to administrators
-- through list APIs.

ALTER TABLE api_keys ADD COLUMN active BOOLEAN NOT NULL DEFAULT TRUE;

CREATE INDEX IF NOT EXISTS idx_api_keys_active
    ON api_keys(active);
