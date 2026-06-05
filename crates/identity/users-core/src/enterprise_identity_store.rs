//! Enterprise identity extension storage.
//!
//! This keeps SCIM, SAML, and WebAuthn protocol dependencies out of users-core
//! while still giving optional extension crates a durable first-party store for
//! external identity links and passkey credentials.

use crate::{
    Error, ExternalIdentity, PasskeyCredential, Result, SqliteUserStore,
    UpsertExternalIdentityRequest, UpsertPasskeyCredentialRequest,
};
use async_trait::async_trait;
use chrono::Utc;
use sqlx_core::{query::query, row::Row};
use sqlx_sqlite::SqliteRow;

#[cfg(feature = "postgres")]
use crate::PostgresUserStore;
#[cfg(feature = "postgres")]
use sqlx_postgres::PgRow;

/// Storage contract for enterprise identity extension records.
#[async_trait]
pub trait EnterpriseIdentityStore: Send + Sync {
    /// Create or update an external IdP/provisioning identity link.
    async fn upsert_external_identity(
        &self,
        request: UpsertExternalIdentityRequest,
    ) -> Result<ExternalIdentity>;

    /// Look up an external identity by provider and subject.
    async fn get_external_identity(
        &self,
        provider_id: &str,
        external_subject: &str,
    ) -> Result<Option<ExternalIdentity>>;

    /// List external identities linked to a users-core user.
    async fn list_external_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<ExternalIdentity>>;

    /// Delete an external identity link.
    async fn delete_external_identity(
        &self,
        provider_id: &str,
        external_subject: &str,
    ) -> Result<()>;

    /// Create or replace a WebAuthn/passkey credential.
    async fn upsert_passkey_credential(
        &self,
        request: UpsertPasskeyCredentialRequest,
    ) -> Result<PasskeyCredential>;

    /// Look up a WebAuthn/passkey credential by credential id.
    async fn get_passkey_credential(
        &self,
        credential_id: &str,
    ) -> Result<Option<PasskeyCredential>>;

    /// List WebAuthn/passkey credentials for a user.
    async fn list_passkey_credentials_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<PasskeyCredential>>;

    /// Update passkey sign-count and last-used state after successful auth.
    async fn update_passkey_usage(&self, credential_id: &str, sign_count: u64) -> Result<()>;

    /// Delete a WebAuthn/passkey credential.
    async fn delete_passkey_credential(&self, credential_id: &str) -> Result<()>;
}

#[async_trait]
impl EnterpriseIdentityStore for SqliteUserStore {
    async fn upsert_external_identity(
        &self,
        request: UpsertExternalIdentityRequest,
    ) -> Result<ExternalIdentity> {
        let now = Utc::now();
        let groups_json = serde_json::to_string(&request.groups)
            .map_err(|err| Error::Validation(err.to_string()))?;
        query(
            "INSERT INTO external_identities
                (provider_id, external_subject, user_id, email, username, display_name,
                 groups_json, active, last_seen_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(provider_id, external_subject) DO UPDATE SET
                user_id = excluded.user_id,
                email = excluded.email,
                username = excluded.username,
                display_name = excluded.display_name,
                groups_json = excluded.groups_json,
                active = excluded.active,
                last_seen_at = excluded.last_seen_at,
                updated_at = excluded.updated_at",
        )
        .bind(&request.provider_id)
        .bind(&request.external_subject)
        .bind(&request.user_id)
        .bind(&request.email)
        .bind(&request.username)
        .bind(&request.display_name)
        .bind(&groups_json)
        .bind(request.active)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_external_identity(&request.provider_id, &request.external_subject)
            .await?
            .ok_or_else(|| Error::Internal(anyhow::anyhow!("external identity upsert failed")))
    }

    async fn get_external_identity(
        &self,
        provider_id: &str,
        external_subject: &str,
    ) -> Result<Option<ExternalIdentity>> {
        let row = query(
            "SELECT provider_id, external_subject, user_id, email, username, display_name,
                    groups_json, active, last_seen_at, created_at, updated_at
             FROM external_identities
             WHERE provider_id = ? AND external_subject = ?",
        )
        .bind(provider_id)
        .bind(external_subject)
        .fetch_optional(self.pool())
        .await?;

        Ok(row.map(sqlite_external_identity_from_row))
    }

    async fn list_external_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<ExternalIdentity>> {
        let rows = query(
            "SELECT provider_id, external_subject, user_id, email, username, display_name,
                    groups_json, active, last_seen_at, created_at, updated_at
             FROM external_identities
             WHERE user_id = ?
             ORDER BY provider_id, external_subject",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(sqlite_external_identity_from_row)
            .collect())
    }

    async fn delete_external_identity(
        &self,
        provider_id: &str,
        external_subject: &str,
    ) -> Result<()> {
        query("DELETE FROM external_identities WHERE provider_id = ? AND external_subject = ?")
            .bind(provider_id)
            .bind(external_subject)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn upsert_passkey_credential(
        &self,
        request: UpsertPasskeyCredentialRequest,
    ) -> Result<PasskeyCredential> {
        let now = Utc::now();
        let transports_json = serde_json::to_string(&request.transports)
            .map_err(|err| Error::Validation(err.to_string()))?;
        let sign_count = i64::try_from(request.sign_count)
            .map_err(|_| Error::Validation("passkey sign_count exceeds i64".to_string()))?;
        query(
            "INSERT INTO passkey_credentials
                (credential_id, user_id, public_key, sign_count, transports_json,
                 backup_eligible, backup_state, display_name, created_at, last_used_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)
             ON CONFLICT(credential_id) DO UPDATE SET
                user_id = excluded.user_id,
                public_key = excluded.public_key,
                sign_count = excluded.sign_count,
                transports_json = excluded.transports_json,
                backup_eligible = excluded.backup_eligible,
                backup_state = excluded.backup_state,
                display_name = excluded.display_name",
        )
        .bind(&request.credential_id)
        .bind(&request.user_id)
        .bind(&request.public_key)
        .bind(sign_count)
        .bind(&transports_json)
        .bind(request.backup_eligible)
        .bind(request.backup_state)
        .bind(&request.display_name)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_passkey_credential(&request.credential_id)
            .await?
            .ok_or_else(|| Error::Internal(anyhow::anyhow!("passkey credential upsert failed")))
    }

    async fn get_passkey_credential(
        &self,
        credential_id: &str,
    ) -> Result<Option<PasskeyCredential>> {
        let row = query(
            "SELECT credential_id, user_id, public_key, sign_count, transports_json,
                    backup_eligible, backup_state, display_name, created_at, last_used_at
             FROM passkey_credentials
             WHERE credential_id = ?",
        )
        .bind(credential_id)
        .fetch_optional(self.pool())
        .await?;

        Ok(row.map(sqlite_passkey_from_row))
    }

    async fn list_passkey_credentials_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<PasskeyCredential>> {
        let rows = query(
            "SELECT credential_id, user_id, public_key, sign_count, transports_json,
                    backup_eligible, backup_state, display_name, created_at, last_used_at
             FROM passkey_credentials
             WHERE user_id = ?
             ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows.into_iter().map(sqlite_passkey_from_row).collect())
    }

    async fn update_passkey_usage(&self, credential_id: &str, sign_count: u64) -> Result<()> {
        let sign_count = i64::try_from(sign_count)
            .map_err(|_| Error::Validation("passkey sign_count exceeds i64".to_string()))?;
        query("UPDATE passkey_credentials SET sign_count = ?, last_used_at = ? WHERE credential_id = ?")
            .bind(sign_count)
            .bind(Utc::now())
            .bind(credential_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn delete_passkey_credential(&self, credential_id: &str) -> Result<()> {
        query("DELETE FROM passkey_credentials WHERE credential_id = ?")
            .bind(credential_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}

fn sqlite_external_identity_from_row(row: SqliteRow) -> ExternalIdentity {
    let groups_json: String = row.get("groups_json");
    ExternalIdentity {
        provider_id: row.get("provider_id"),
        external_subject: row.get("external_subject"),
        user_id: row.get("user_id"),
        email: row.get("email"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        groups: serde_json::from_str(&groups_json).unwrap_or_default(),
        active: row.get("active"),
        last_seen_at: row.get("last_seen_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn sqlite_passkey_from_row(row: SqliteRow) -> PasskeyCredential {
    let transports_json: String = row.get("transports_json");
    let sign_count: i64 = row.get("sign_count");
    PasskeyCredential {
        credential_id: row.get("credential_id"),
        user_id: row.get("user_id"),
        public_key: row.get("public_key"),
        sign_count: u64::try_from(sign_count).unwrap_or_default(),
        transports: serde_json::from_str(&transports_json).unwrap_or_default(),
        backup_eligible: row.get("backup_eligible"),
        backup_state: row.get("backup_state"),
        display_name: row.get("display_name"),
        created_at: row.get("created_at"),
        last_used_at: row.get("last_used_at"),
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl EnterpriseIdentityStore for PostgresUserStore {
    async fn upsert_external_identity(
        &self,
        request: UpsertExternalIdentityRequest,
    ) -> Result<ExternalIdentity> {
        let now = Utc::now();
        let groups_json = serde_json::to_string(&request.groups)
            .map_err(|err| Error::Validation(err.to_string()))?;
        query(
            "INSERT INTO external_identities
                (provider_id, external_subject, user_id, email, username, display_name,
                 groups_json, active, last_seen_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
             ON CONFLICT(provider_id, external_subject) DO UPDATE SET
                user_id = excluded.user_id,
                email = excluded.email,
                username = excluded.username,
                display_name = excluded.display_name,
                groups_json = excluded.groups_json,
                active = excluded.active,
                last_seen_at = excluded.last_seen_at,
                updated_at = excluded.updated_at",
        )
        .bind(&request.provider_id)
        .bind(&request.external_subject)
        .bind(&request.user_id)
        .bind(&request.email)
        .bind(&request.username)
        .bind(&request.display_name)
        .bind(&groups_json)
        .bind(request.active)
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_external_identity(&request.provider_id, &request.external_subject)
            .await?
            .ok_or_else(|| Error::Internal(anyhow::anyhow!("external identity upsert failed")))
    }

    async fn get_external_identity(
        &self,
        provider_id: &str,
        external_subject: &str,
    ) -> Result<Option<ExternalIdentity>> {
        let row = query(
            "SELECT provider_id, external_subject, user_id, email, username, display_name,
                    groups_json, active, last_seen_at, created_at, updated_at
             FROM external_identities
             WHERE provider_id = $1 AND external_subject = $2",
        )
        .bind(provider_id)
        .bind(external_subject)
        .fetch_optional(self.pool())
        .await?;

        Ok(row.map(postgres_external_identity_from_row))
    }

    async fn list_external_identities_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<ExternalIdentity>> {
        let rows = query(
            "SELECT provider_id, external_subject, user_id, email, username, display_name,
                    groups_json, active, last_seen_at, created_at, updated_at
             FROM external_identities
             WHERE user_id = $1
             ORDER BY provider_id, external_subject",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(postgres_external_identity_from_row)
            .collect())
    }

    async fn delete_external_identity(
        &self,
        provider_id: &str,
        external_subject: &str,
    ) -> Result<()> {
        query("DELETE FROM external_identities WHERE provider_id = $1 AND external_subject = $2")
            .bind(provider_id)
            .bind(external_subject)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn upsert_passkey_credential(
        &self,
        request: UpsertPasskeyCredentialRequest,
    ) -> Result<PasskeyCredential> {
        let now = Utc::now();
        let transports_json = serde_json::to_string(&request.transports)
            .map_err(|err| Error::Validation(err.to_string()))?;
        let sign_count = i64::try_from(request.sign_count)
            .map_err(|_| Error::Validation("passkey sign_count exceeds i64".to_string()))?;
        query(
            "INSERT INTO passkey_credentials
                (credential_id, user_id, public_key, sign_count, transports_json,
                 backup_eligible, backup_state, display_name, created_at, last_used_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL)
             ON CONFLICT(credential_id) DO UPDATE SET
                user_id = excluded.user_id,
                public_key = excluded.public_key,
                sign_count = excluded.sign_count,
                transports_json = excluded.transports_json,
                backup_eligible = excluded.backup_eligible,
                backup_state = excluded.backup_state,
                display_name = excluded.display_name",
        )
        .bind(&request.credential_id)
        .bind(&request.user_id)
        .bind(&request.public_key)
        .bind(sign_count)
        .bind(&transports_json)
        .bind(request.backup_eligible)
        .bind(request.backup_state)
        .bind(&request.display_name)
        .bind(now)
        .execute(self.pool())
        .await?;

        self.get_passkey_credential(&request.credential_id)
            .await?
            .ok_or_else(|| Error::Internal(anyhow::anyhow!("passkey credential upsert failed")))
    }

    async fn get_passkey_credential(
        &self,
        credential_id: &str,
    ) -> Result<Option<PasskeyCredential>> {
        let row = query(
            "SELECT credential_id, user_id, public_key, sign_count, transports_json,
                    backup_eligible, backup_state, display_name, created_at, last_used_at
             FROM passkey_credentials
             WHERE credential_id = $1",
        )
        .bind(credential_id)
        .fetch_optional(self.pool())
        .await?;

        Ok(row.map(postgres_passkey_from_row))
    }

    async fn list_passkey_credentials_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<PasskeyCredential>> {
        let rows = query(
            "SELECT credential_id, user_id, public_key, sign_count, transports_json,
                    backup_eligible, backup_state, display_name, created_at, last_used_at
             FROM passkey_credentials
             WHERE user_id = $1
             ORDER BY created_at",
        )
        .bind(user_id)
        .fetch_all(self.pool())
        .await?;

        Ok(rows.into_iter().map(postgres_passkey_from_row).collect())
    }

    async fn update_passkey_usage(&self, credential_id: &str, sign_count: u64) -> Result<()> {
        let sign_count = i64::try_from(sign_count)
            .map_err(|_| Error::Validation("passkey sign_count exceeds i64".to_string()))?;
        query(
            "UPDATE passkey_credentials
             SET sign_count = $1, last_used_at = $2
             WHERE credential_id = $3",
        )
        .bind(sign_count)
        .bind(Utc::now())
        .bind(credential_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn delete_passkey_credential(&self, credential_id: &str) -> Result<()> {
        query("DELETE FROM passkey_credentials WHERE credential_id = $1")
            .bind(credential_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }
}

#[cfg(feature = "postgres")]
fn postgres_external_identity_from_row(row: PgRow) -> ExternalIdentity {
    let groups_json: String = row.get("groups_json");
    ExternalIdentity {
        provider_id: row.get("provider_id"),
        external_subject: row.get("external_subject"),
        user_id: row.get("user_id"),
        email: row.get("email"),
        username: row.get("username"),
        display_name: row.get("display_name"),
        groups: serde_json::from_str(&groups_json).unwrap_or_default(),
        active: row.get("active"),
        last_seen_at: row.get("last_seen_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[cfg(feature = "postgres")]
fn postgres_passkey_from_row(row: PgRow) -> PasskeyCredential {
    let transports_json: String = row.get("transports_json");
    let sign_count: i64 = row.get("sign_count");
    PasskeyCredential {
        credential_id: row.get("credential_id"),
        user_id: row.get("user_id"),
        public_key: row.get("public_key"),
        sign_count: u64::try_from(sign_count).unwrap_or_default(),
        transports: serde_json::from_str(&transports_json).unwrap_or_default(),
        backup_eligible: row.get("backup_eligible"),
        backup_state: row.get("backup_state"),
        display_name: row.get("display_name"),
        created_at: row.get("created_at"),
        last_used_at: row.get("last_used_at"),
    }
}
