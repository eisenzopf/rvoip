//! Core types for authentication

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents an authenticated user context
#[derive(Clone, Serialize, Deserialize)]
pub struct UserContext {
    /// Unique user identifier
    pub user_id: String,

    /// Username or email
    pub username: String,

    /// User's roles
    pub roles: Vec<String>,

    /// Additional claims from the token
    pub claims: HashMap<String, serde_json::Value>,

    /// Token expiration time (Unix timestamp)
    pub expires_at: Option<i64>,

    /// OAuth2 scopes
    pub scopes: Vec<String>,
}

impl std::fmt::Debug for UserContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserContext")
            .field("user_present", &!self.user_id.is_empty())
            .field("username_present", &!self.username.is_empty())
            .field("role_count", &self.roles.len())
            .field("claim_count", &self.claims.len())
            .field("expires_at_present", &self.expires_at.is_some())
            .field("scope_count", &self.scopes.len())
            .finish()
    }
}

/// Token types supported by the system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TokenType {
    Bearer,
    JWT,
    Opaque,
}

/// Authentication method
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthMethod {
    OAuth2,
    JWT,
    ApiKey,
    SIPDigest,
}
