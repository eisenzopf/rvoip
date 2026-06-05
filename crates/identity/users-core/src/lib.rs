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

pub mod api;
pub mod api_keys;
pub mod auth;
pub mod auth_security_store;
pub mod config;
pub mod enterprise_identity_store;
pub mod error;
pub mod jwt;
pub mod sip_digest_credentials;
pub mod types;
pub mod user_store;
pub mod validation;

pub use api_keys::{ApiKey, ApiKeyStore};
pub use auth::{AuthenticationResult, AuthenticationService, TokenIssueContext, TokenPair};
pub use auth_security_store::AuthSecurityStore;
pub use config::UsersConfig;
pub use enterprise_identity_store::EnterpriseIdentityStore;
pub use error::{Error, Result};
pub use jwt::{JwtIssuer, UserClaims};
pub use sip_digest_credentials::{
    CreateSipDigestCredentialRequest, SipDigestAlgorithmFamily, SipDigestCredential,
};
pub use types::{
    CreateUserRequest, ExternalIdentity, PasskeyCredential, UpdateUserRequest,
    UpsertExternalIdentityRequest, UpsertPasskeyCredentialRequest, User, UserFilter,
};
#[cfg(feature = "postgres")]
pub use user_store::PostgresUserStore;
pub use user_store::{SqliteUserStore, UserStore};

#[cfg(feature = "auth-core")]
pub mod auth_core_bridge;

#[cfg(feature = "auth-core")]
pub use auth_core_bridge::UsersCoreAuthProvider;

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

    // Set the shared auth-security store for refresh/access revocation,
    // password/last-login updates, and SIP Digest HA1 storage.
    auth_service.set_auth_security_store(user_store_arc.clone());
    auth_service.set_enterprise_identity_store(user_store_arc);

    Ok(auth_service)
}
