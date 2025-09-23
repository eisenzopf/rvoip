//! # Users-Core
//! 
//! User management and authentication service for RVoIP.
//! 
//! This crate provides:
//! - User storage and management in SQLite
//! - Password authentication with Argon2
//! - JWT token issuance for internal users
//! - API key management
//! - REST API for user operations
//! 
//! ## Architecture
//! 
//! Users-Core handles authentication (issuing tokens) while auth-core handles
//! validation of tokens from all sources (users-core, OAuth2 providers, etc.).

pub mod error;
pub mod types;
pub mod auth;
pub mod user_store;
pub mod api_keys;
pub mod jwt;
pub mod api;
pub mod config;

pub use error::{Error, Result};
pub use types::{User, CreateUserRequest, UpdateUserRequest, UserFilter};
pub use auth::{AuthenticationService, AuthenticationResult, TokenPair};
pub use user_store::{UserStore, SqliteUserStore};
pub use api_keys::{ApiKey, ApiKeyStore};
pub use jwt::{JwtIssuer, UserClaims};
pub use config::UsersConfig;

/// Initialize the users-core service
pub async fn init(config: UsersConfig) -> Result<AuthenticationService> {
    // Initialize database
    let user_store = SqliteUserStore::new(&config.database_url).await?;
    
    // Initialize JWT issuer
    let jwt_issuer = JwtIssuer::new(config.jwt)?;
    
    // Initialize API key store (same backing store)
    let user_store_arc = std::sync::Arc::new(user_store.clone());
    let api_key_store = user_store_arc.clone();
    
    // Create authentication service
    let mut auth_service = AuthenticationService::new(
        user_store_arc.clone(),
        jwt_issuer,
        api_key_store,
        config.password,
    )?;
    
    // Set the pool for refresh token management
    auth_service.set_pool(user_store.pool().clone());
    
    Ok(auth_service)
}