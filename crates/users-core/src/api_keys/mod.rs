//! API key management

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use crate::Result;

/// API key
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: String,
    pub user_id: String,
    pub name: String,
    #[serde(skip_serializing)]
    pub key_hash: String,
    pub permissions: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// API key storage trait
#[async_trait]
pub trait ApiKeyStore: Send + Sync {
    async fn create_api_key(&self, request: CreateApiKeyRequest) -> Result<(ApiKey, String)>;
    async fn validate_api_key(&self, key: &str) -> Result<Option<ApiKey>>;
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
