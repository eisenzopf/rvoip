//! API key management

use crate::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// API key
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub user_id: String,
    pub name: String,
    #[serde(skip_serializing)]
    pub key_hash: String,
    pub permissions: Vec<String>,
    pub active: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApiKey")
            .field("id_present", &!self.id.is_empty())
            .field("user_present", &!self.user_id.is_empty())
            .field("name_len", &self.name.len())
            .field("key_hash_present", &!self.key_hash.is_empty())
            .field("key_hash_len", &self.key_hash.len())
            .field("permission_count", &self.permissions.len())
            .field("active", &self.active)
            .field("expires_at_present", &self.expires_at.is_some())
            .field("last_used_present", &self.last_used.is_some())
            .finish()
    }
}

/// API key storage trait
#[async_trait]
pub trait ApiKeyStore: Send + Sync {
    async fn create_api_key(&self, request: CreateApiKeyRequest) -> Result<(ApiKey, String)>;
    async fn validate_api_key(&self, key: &str) -> Result<Option<ApiKey>>;
    async fn set_api_key_active(&self, _id: &str, _active: bool) -> Result<()> {
        Err(crate::Error::Validation(
            "API key active-state updates are not supported by this store".to_string(),
        ))
    }
    async fn revoke_api_key(&self, id: &str) -> Result<()>;
    async fn list_api_keys(&self, user_id: &str) -> Result<Vec<ApiKey>>;
}

/// Request to create an API key
#[derive(Debug, Clone, Deserialize)]
pub struct CreateApiKeyRequest {
    pub user_id: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl CreateApiKeyRequest {
    /// Validate the request
    pub fn validate(&self) -> crate::Result<()> {
        use crate::validation::validate_api_key_name;

        // Validate name
        validate_api_key_name(&self.name)
            .map_err(|e| crate::Error::Validation(format!("Invalid API key name: {}", e)))?;

        // Validate permissions
        const ALLOWED_PERMISSIONS: &[&str] = &[
            "read",
            "write",
            "delete",
            "admin",
            "sip.register",
            "sip.call",
            "sip.message",
            "sip.presence",
            "*",
        ];
        for perm in &self.permissions {
            if !ALLOWED_PERMISSIONS.contains(&perm.as_str()) {
                return Err(crate::Error::Validation(format!(
                    "Invalid permission: {}",
                    perm
                )));
            }
        }

        // Validate expiry date if provided
        if let Some(expires_at) = self.expires_at {
            if expires_at <= Utc::now() {
                return Err(crate::Error::Validation(
                    "Expiry date must be in the future".to_string(),
                ));
            }
        }

        Ok(())
    }
}
