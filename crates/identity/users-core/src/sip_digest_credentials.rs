//! SIP Digest credential storage.
//!
//! SIP Digest needs HA1 material for `username:realm:password`. That material
//! is intentionally stored separately from users-core login password hashes;
//! Argon2 hashes cannot validate SIP Digest responses.

use crate::{AuthenticationService, Error, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest as Sha2Digest, Sha256, Sha512_256};
use sqlx_core::{query::query, row::Row};
use uuid::Uuid;

/// Hash family used for a stored SIP Digest HA1 value.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum SipDigestAlgorithmFamily {
    Md5,
    Sha256,
    Sha512256,
}

impl SipDigestAlgorithmFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha256 => "SHA-256",
            Self::Sha512256 => "SHA-512-256",
        }
    }

    fn hash(self, input: &[u8]) -> String {
        match self {
            Self::Md5 => hex::encode(md5::compute(input).0),
            Self::Sha256 => hex::encode(Sha256::digest(input)),
            Self::Sha512256 => hex::encode(Sha512_256::digest(input)),
        }
    }

    pub fn ha1(self, username: &str, realm: &str, password: &str) -> String {
        self.hash(format!("{username}:{realm}:{password}").as_bytes())
    }
}

impl std::fmt::Display for SipDigestAlgorithmFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SipDigestAlgorithmFamily {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "MD5" => Ok(Self::Md5),
            "SHA-256" => Ok(Self::Sha256),
            "SHA-512-256" => Ok(Self::Sha512256),
            other => Err(Error::Validation(format!(
                "unsupported SIP Digest algorithm family: {other}"
            ))),
        }
    }
}

#[cfg(feature = "auth-core")]
impl From<rvoip_auth_core::DigestAlgorithm> for SipDigestAlgorithmFamily {
    fn from(algorithm: rvoip_auth_core::DigestAlgorithm) -> Self {
        match algorithm {
            rvoip_auth_core::DigestAlgorithm::MD5 | rvoip_auth_core::DigestAlgorithm::MD5Sess => {
                Self::Md5
            }
            rvoip_auth_core::DigestAlgorithm::SHA256
            | rvoip_auth_core::DigestAlgorithm::SHA256Sess => Self::Sha256,
            rvoip_auth_core::DigestAlgorithm::SHA512256
            | rvoip_auth_core::DigestAlgorithm::SHA512256Sess => Self::Sha512256,
        }
    }
}

/// Stored SIP Digest credential.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipDigestCredential {
    pub id: String,
    pub user_id: String,
    pub sip_username: String,
    pub realm: String,
    pub algorithm: SipDigestAlgorithmFamily,
    #[serde(skip_serializing)]
    pub ha1: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Request to create or replace a SIP Digest credential.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateSipDigestCredentialRequest {
    pub user_id: String,
    pub sip_username: String,
    pub realm: String,
    pub algorithm: SipDigestAlgorithmFamily,
    #[serde(skip_serializing)]
    pub password: String,
}

impl AuthenticationService {
    /// Create or replace SIP Digest HA1 material for a user.
    pub async fn create_sip_digest_credential(
        &self,
        request: CreateSipDigestCredentialRequest,
    ) -> Result<SipDigestCredential> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            Error::Config("SIP Digest credential storage requires a database pool".to_string())
        })?;

        let user = self
            .user_store()
            .get_user(&request.user_id)
            .await?
            .ok_or_else(|| Error::UserNotFound(request.user_id.clone()))?;
        if !user.active {
            return Err(Error::InvalidCredentials);
        }

        let now = Utc::now();
        let ha1 = request
            .algorithm
            .ha1(&request.sip_username, &request.realm, &request.password);
        let credential = SipDigestCredential {
            id: Uuid::new_v4().to_string(),
            user_id: request.user_id,
            sip_username: request.sip_username,
            realm: request.realm,
            algorithm: request.algorithm,
            ha1,
            created_at: now,
            updated_at: now,
        };

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
        .execute(pool)
        .await?;

        self.lookup_sip_digest_credential(
            &credential.sip_username,
            &credential.realm,
            credential.algorithm,
        )
        .await?
        .ok_or_else(|| Error::Internal(anyhow::anyhow!("created SIP Digest credential missing")))
    }

    /// Rotate SIP Digest HA1 material.
    pub async fn rotate_sip_digest_credential(
        &self,
        user_id: impl Into<String>,
        sip_username: impl Into<String>,
        realm: impl Into<String>,
        algorithm: SipDigestAlgorithmFamily,
        password: impl Into<String>,
    ) -> Result<SipDigestCredential> {
        self.create_sip_digest_credential(CreateSipDigestCredentialRequest {
            user_id: user_id.into(),
            sip_username: sip_username.into(),
            realm: realm.into(),
            algorithm,
            password: password.into(),
        })
        .await
    }

    /// Delete SIP Digest material for a SIP username, realm, and algorithm.
    pub async fn delete_sip_digest_credential(
        &self,
        sip_username: &str,
        realm: &str,
        algorithm: SipDigestAlgorithmFamily,
    ) -> Result<()> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            Error::Config("SIP Digest credential storage requires a database pool".to_string())
        })?;
        query(
            "DELETE FROM sip_digest_credentials
             WHERE sip_username = ? AND realm = ? AND algorithm = ?",
        )
        .bind(sip_username)
        .bind(realm)
        .bind(algorithm.as_str())
        .execute(pool)
        .await?;
        Ok(())
    }

    /// Look up SIP Digest material for validation.
    pub async fn lookup_sip_digest_credential(
        &self,
        sip_username: &str,
        realm: &str,
        algorithm: SipDigestAlgorithmFamily,
    ) -> Result<Option<SipDigestCredential>> {
        let pool = self.pool.as_ref().ok_or_else(|| {
            Error::Config("SIP Digest credential storage requires a database pool".to_string())
        })?;
        let row = query(
            "SELECT id, user_id, sip_username, realm, algorithm, ha1, created_at, updated_at
             FROM sip_digest_credentials
             WHERE sip_username = ? AND realm = ? AND algorithm = ?",
        )
        .bind(sip_username)
        .bind(realm)
        .bind(algorithm.as_str())
        .fetch_optional(pool)
        .await?;

        row.map(row_to_credential).transpose()
    }
}

fn row_to_credential(row: sqlx_sqlite::SqliteRow) -> Result<SipDigestCredential> {
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
