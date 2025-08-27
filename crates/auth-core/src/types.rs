//! Core types for authentication

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents an authenticated user context
#[derive(Debug, Clone, Serialize, Deserialize)]
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