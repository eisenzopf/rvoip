//! Authentication types and errors

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result type for authentication operations
pub type AuthResult<T> = Result<T, AuthError>;

/// Information extracted from a validated token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    /// User identity (maps to SIP user, e.g., "alice@example.com")
    pub subject: String,
    
    /// Granted permissions/scopes
    pub scopes: Vec<String>,
    
    /// Token expiration time
    pub expires_at: DateTime<Utc>,
    
    /// OAuth client that issued the token
    pub client_id: String,
    
    /// Authorization realm (optional)
    pub realm: Option<String>,
    
    /// Additional claims from the token
    pub extra_claims: std::collections::HashMap<String, serde_json::Value>,
}

impl TokenInfo {
    /// Check if token has expired
    pub fn is_expired(&self) -> bool {
        self.expires_at < Utc::now()
    }
    
    /// Check if token has a specific scope
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }
    
    /// Check if token has all required scopes
    pub fn has_all_scopes(&self, scopes: &[String]) -> bool {
        scopes.iter().all(|scope| self.has_scope(scope))
    }
    
    /// Check if token has any of the required scopes
    pub fn has_any_scope(&self, scopes: &[String]) -> bool {
        scopes.iter().any(|scope| self.has_scope(scope))
    }
}

/// Authentication errors
#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid token: {0}")]
    InvalidToken(String),
    
    #[error("Token expired")]
    TokenExpired,
    
    #[error("Insufficient scope: required {required:?}, had {actual:?}")]
    InsufficientScope {
        required: Vec<String>,
        actual: Vec<String>,
    },
    
    #[error("JWKS fetch failed: {0}")]
    JwksFetchError(String),
    
    #[error("Token introspection failed: {0}")]
    IntrospectionError(String),
    
    #[error("JWT validation error: {0}")]
    JwtValidationError(String),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    #[error("Network error: {0}")]
    NetworkError(String),
    
    #[error("Cache error: {0}")]
    CacheError(String),
}

/// Token introspection response (RFC 7662)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntrospectionResponse {
    pub active: bool,
    pub scope: Option<String>,
    pub client_id: Option<String>,
    pub username: Option<String>,
    pub token_type: Option<String>,
    pub exp: Option<i64>,
    pub iat: Option<i64>,
    pub nbf: Option<i64>,
    pub sub: Option<String>,
    pub aud: Option<serde_json::Value>,
    pub iss: Option<String>,
    pub jti: Option<String>,
}

impl IntrospectionResponse {
    /// Convert to TokenInfo
    pub fn to_token_info(&self) -> AuthResult<TokenInfo> {
        if !self.active {
            return Err(AuthError::InvalidToken("Token is not active".to_string()));
        }
        
        let subject = self.sub.as_ref()
            .or(self.username.as_ref())
            .ok_or_else(|| AuthError::InvalidToken("No subject in token".to_string()))?
            .clone();
        
        let scopes = self.scope.as_ref()
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_default();
        
        let expires_at = self.exp
            .map(|exp| DateTime::from_timestamp(exp, 0))
            .flatten()
            .ok_or_else(|| AuthError::InvalidToken("No expiration in token".to_string()))?;
        
        let client_id = self.client_id.clone()
            .unwrap_or_else(|| "unknown".to_string());
        
        Ok(TokenInfo {
            subject,
            scopes,
            expires_at,
            client_id,
            realm: None,
            extra_claims: std::collections::HashMap::new(),
        })
    }
}