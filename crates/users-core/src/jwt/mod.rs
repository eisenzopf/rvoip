//! JWT token issuance

use serde::{Deserialize, Serialize};
use crate::{Result, User};

/// JWT issuer
pub struct JwtIssuer {
    // TODO: Implement
}

/// JWT claims for user tokens
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserClaims {
    // Standard claims
    pub iss: String,              // Issuer
    pub sub: String,              // Subject (user ID)
    pub aud: Vec<String>,         // Audience
    pub exp: u64,                 // Expiration
    pub iat: u64,                 // Issued at
    pub jti: String,              // JWT ID
    
    // Custom claims
    pub username: String,
    pub email: Option<String>,
    pub roles: Vec<String>,
    pub scope: String,
}

/// JWT configuration
#[derive(Debug, Clone, Deserialize)]
pub struct JwtConfig {
    pub issuer: String,
    pub audience: Vec<String>,
    pub access_ttl_seconds: u64,
    pub refresh_ttl_seconds: u64,
    pub algorithm: String,
}

impl JwtIssuer {
    pub fn new(_config: JwtConfig) -> Result<Self> {
        todo!("Implement JwtIssuer")
    }
    
    pub fn create_access_token(&self, _user: &User) -> Result<String> {
        todo!("Implement create_access_token")
    }
    
    pub fn create_refresh_token(&self, _user_id: &str) -> Result<String> {
        todo!("Implement create_refresh_token")
    }
}
