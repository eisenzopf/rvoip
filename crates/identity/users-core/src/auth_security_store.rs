//! Auth-service security table storage.
//!
//! `AuthenticationService` uses this trait for state that is not part of the
//! core user/profile API: refresh-token revocation, access-token revocation,
//! last-login/password updates, and SIP Digest HA1 material. Protocol crates
//! still depend on auth-core traits; this is the optional users-core reference
//! database implementation.

use crate::{Error, Result, SipDigestAlgorithmFamily, SipDigestCredential, SqliteUserStore};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx_core::{query::query, row::Row};
use sqlx_sqlite::SqliteRow;

#[cfg(feature = "postgres")]
use crate::PostgresUserStore;
#[cfg(feature = "postgres")]
use sqlx_postgres::PgRow;

/// Storage contract for auth-service security tables.
#[async_trait]
pub trait AuthSecurityStore: Send + Sync {
    /// Store a refresh-token JTI until its normal expiry.
    async fn store_refresh_token(
        &self,
        jti: &str,
        user_id: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()>;

    /// Reject unless a refresh-token JTI is durably known and active.
    async fn check_refresh_token_revoked(&self, jti: &str) -> Result<()>;

    /// Revoke all active refresh tokens for a user.
    async fn revoke_refresh_tokens_for_user(&self, user_id: &str) -> Result<()>;

    /// Revoke an access-token JTI until its normal expiry.
    async fn revoke_access_token_jti(
        &self,
        jti: &str,
        user_id: Option<&str>,
        expires_at: DateTime<Utc>,
    ) -> Result<()>;

    /// Return whether an access-token JTI is currently revoked.
    async fn is_access_token_revoked(&self, jti: &str) -> Result<bool>;

    /// Update a user's password hash.
    async fn update_password_hash(&self, user_id: &str, password_hash: &str) -> Result<()>;

    /// Update a user's last-login timestamp.
    async fn update_last_login(&self, user_id: &str) -> Result<()>;

    /// Create or replace SIP Digest HA1 material.
    async fn upsert_sip_digest_credential(&self, credential: &SipDigestCredential) -> Result<()>;

    /// Delete SIP Digest HA1 material.
    async fn delete_sip_digest_credential(
        &self,
        sip_username: &str,
        realm: &str,
        algorithm: SipDigestAlgorithmFamily,
    ) -> Result<()>;

    /// Look up SIP Digest HA1 material.
    async fn lookup_sip_digest_credential(
        &self,
        sip_username: &str,
        realm: &str,
        algorithm: SipDigestAlgorithmFamily,
    ) -> Result<Option<SipDigestCredential>>;
}

#[async_trait]
impl AuthSecurityStore for SqliteUserStore {
    async fn store_refresh_token(
        &self,
        jti: &str,
        user_id: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        query(
            "INSERT INTO refresh_tokens (jti, user_id, expires_at, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(jti)
        .bind(user_id)
        .bind(expires_at)
        .bind(Utc::now())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn check_refresh_token_revoked(&self, jti: &str) -> Result<()> {
        let row = query("SELECT revoked_at FROM refresh_tokens WHERE jti = ?")
            .bind(jti)
            .fetch_optional(self.pool())
            .await?;

        let Some(row) = row else {
            return Err(Error::InvalidCredentials);
        };
        let revoked_at: Option<DateTime<Utc>> = row.get("revoked_at");
        if revoked_at.is_some() {
            return Err(Error::InvalidCredentials);
        }
        Ok(())
    }

    async fn revoke_refresh_tokens_for_user(&self, user_id: &str) -> Result<()> {
        query("UPDATE refresh_tokens SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
            .bind(Utc::now())
            .bind(user_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn revoke_access_token_jti(
        &self,
        jti: &str,
        user_id: Option<&str>,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        query(
            "INSERT INTO revoked_access_tokens (jti, user_id, expires_at, revoked_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(jti) DO UPDATE SET
                user_id = excluded.user_id,
                expires_at = excluded.expires_at,
                revoked_at = excluded.revoked_at",
        )
        .bind(jti)
        .bind(user_id)
        .bind(expires_at)
        .bind(Utc::now())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn is_access_token_revoked(&self, jti: &str) -> Result<bool> {
        let row = query("SELECT expires_at FROM revoked_access_tokens WHERE jti = ?")
            .bind(jti)
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else {
            return Ok(false);
        };

        let expires_at: DateTime<Utc> = row.get("expires_at");
        if expires_at > Utc::now() {
            return Ok(true);
        }

        query("DELETE FROM revoked_access_tokens WHERE jti = ?")
            .bind(jti)
            .execute(self.pool())
            .await?;
        Ok(false)
    }

    async fn update_password_hash(&self, user_id: &str, password_hash: &str) -> Result<()> {
        query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
            .bind(password_hash)
            .bind(Utc::now())
            .bind(user_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn update_last_login(&self, user_id: &str) -> Result<()> {
        query("UPDATE users SET last_login = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(user_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn upsert_sip_digest_credential(&self, credential: &SipDigestCredential) -> Result<()> {
        query(
            "INSERT INTO sip_digest_credentials
                (id, user_id, sip_username, realm, algorithm, ha1, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(sip_username, realm, algorithm)
             DO UPDATE SET
                user_id = excluded.user_id,
                ha1 = excluded.ha1,
                updated_at = excluded.updated_at",
        )
        .bind(&credential.id)
        .bind(&credential.user_id)
        .bind(&credential.sip_username)
        .bind(&credential.realm)
        .bind(credential.algorithm.as_str())
        .bind(&credential.ha1)
        .bind(credential.created_at)
        .bind(credential.updated_at)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn delete_sip_digest_credential(
        &self,
        sip_username: &str,
        realm: &str,
        algorithm: SipDigestAlgorithmFamily,
    ) -> Result<()> {
        query(
            "DELETE FROM sip_digest_credentials
             WHERE sip_username = ? AND realm = ? AND algorithm = ?",
        )
        .bind(sip_username)
        .bind(realm)
        .bind(algorithm.as_str())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn lookup_sip_digest_credential(
        &self,
        sip_username: &str,
        realm: &str,
        algorithm: SipDigestAlgorithmFamily,
    ) -> Result<Option<SipDigestCredential>> {
        let row = query(
            "SELECT id, user_id, sip_username, realm, algorithm, ha1, created_at, updated_at
             FROM sip_digest_credentials
             WHERE sip_username = ? AND realm = ? AND algorithm = ?",
        )
        .bind(sip_username)
        .bind(realm)
        .bind(algorithm.as_str())
        .fetch_optional(self.pool())
        .await?;
        row.map(sqlite_row_to_credential).transpose()
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl AuthSecurityStore for PostgresUserStore {
    async fn store_refresh_token(
        &self,
        jti: &str,
        user_id: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        query(
            "INSERT INTO refresh_tokens (jti, user_id, expires_at, created_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(jti)
        .bind(user_id)
        .bind(expires_at)
        .bind(Utc::now())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn check_refresh_token_revoked(&self, jti: &str) -> Result<()> {
        let row = query("SELECT revoked_at FROM refresh_tokens WHERE jti = $1")
            .bind(jti)
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else {
            return Err(Error::InvalidCredentials);
        };
        let revoked_at: Option<DateTime<Utc>> = row.get("revoked_at");
        if revoked_at.is_some() {
            return Err(Error::InvalidCredentials);
        }
        Ok(())
    }

    async fn revoke_refresh_tokens_for_user(&self, user_id: &str) -> Result<()> {
        query(
            "UPDATE refresh_tokens SET revoked_at = $1
             WHERE user_id = $2 AND revoked_at IS NULL",
        )
        .bind(Utc::now())
        .bind(user_id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn revoke_access_token_jti(
        &self,
        jti: &str,
        user_id: Option<&str>,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        query(
            "INSERT INTO revoked_access_tokens (jti, user_id, expires_at, revoked_at)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT(jti) DO UPDATE SET
                user_id = excluded.user_id,
                expires_at = excluded.expires_at,
                revoked_at = excluded.revoked_at",
        )
        .bind(jti)
        .bind(user_id)
        .bind(expires_at)
        .bind(Utc::now())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn is_access_token_revoked(&self, jti: &str) -> Result<bool> {
        let row = query("SELECT expires_at FROM revoked_access_tokens WHERE jti = $1")
            .bind(jti)
            .fetch_optional(self.pool())
            .await?;
        let Some(row) = row else {
            return Ok(false);
        };

        let expires_at: DateTime<Utc> = row.get("expires_at");
        if expires_at > Utc::now() {
            return Ok(true);
        }

        query("DELETE FROM revoked_access_tokens WHERE jti = $1")
            .bind(jti)
            .execute(self.pool())
            .await?;
        Ok(false)
    }

    async fn update_password_hash(&self, user_id: &str, password_hash: &str) -> Result<()> {
        query("UPDATE users SET password_hash = $1, updated_at = $2 WHERE id = $3")
            .bind(password_hash)
            .bind(Utc::now())
            .bind(user_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn update_last_login(&self, user_id: &str) -> Result<()> {
        query("UPDATE users SET last_login = $1 WHERE id = $2")
            .bind(Utc::now())
            .bind(user_id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    async fn upsert_sip_digest_credential(&self, credential: &SipDigestCredential) -> Result<()> {
        query(
            "INSERT INTO sip_digest_credentials
                (id, user_id, sip_username, realm, algorithm, ha1, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT(sip_username, realm, algorithm)
             DO UPDATE SET
                user_id = excluded.user_id,
                ha1 = excluded.ha1,
                updated_at = excluded.updated_at",
        )
        .bind(&credential.id)
        .bind(&credential.user_id)
        .bind(&credential.sip_username)
        .bind(&credential.realm)
        .bind(credential.algorithm.as_str())
        .bind(&credential.ha1)
        .bind(credential.created_at)
        .bind(credential.updated_at)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn delete_sip_digest_credential(
        &self,
        sip_username: &str,
        realm: &str,
        algorithm: SipDigestAlgorithmFamily,
    ) -> Result<()> {
        query(
            "DELETE FROM sip_digest_credentials
             WHERE sip_username = $1 AND realm = $2 AND algorithm = $3",
        )
        .bind(sip_username)
        .bind(realm)
        .bind(algorithm.as_str())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    async fn lookup_sip_digest_credential(
        &self,
        sip_username: &str,
        realm: &str,
        algorithm: SipDigestAlgorithmFamily,
    ) -> Result<Option<SipDigestCredential>> {
        let row = query(
            "SELECT id, user_id, sip_username, realm, algorithm, ha1, created_at, updated_at
             FROM sip_digest_credentials
             WHERE sip_username = $1 AND realm = $2 AND algorithm = $3",
        )
        .bind(sip_username)
        .bind(realm)
        .bind(algorithm.as_str())
        .fetch_optional(self.pool())
        .await?;
        row.map(postgres_row_to_credential).transpose()
    }
}

fn sqlite_row_to_credential(row: SqliteRow) -> Result<SipDigestCredential> {
    let algorithm: String = row.get("algorithm");
    Ok(SipDigestCredential {
        id: row.get("id"),
        user_id: row.get("user_id"),
        sip_username: row.get("sip_username"),
        realm: row.get("realm"),
        algorithm: algorithm.parse()?,
        ha1: row.get("ha1"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

#[cfg(feature = "postgres")]
fn postgres_row_to_credential(row: PgRow) -> Result<SipDigestCredential> {
    let algorithm: String = row.get("algorithm");
    Ok(SipDigestCredential {
        id: row.get("id"),
        user_id: row.get("user_id"),
        sip_username: row.get("sip_username"),
        realm: row.get("realm"),
        algorithm: algorithm.parse()?,
        ha1: row.get("ha1"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}
