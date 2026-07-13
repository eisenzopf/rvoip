//! Authentication service

use crate::config::PasswordConfig;
use crate::{
    ApiKey, ApiKeyStore, AuthSecurityStore, CreateUserRequest, EnterpriseIdentityStore, Error,
    JwtIssuer, Result, SqliteUserStore, User, UserStore,
};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use password_hash::{rand_core::OsRng, SaltString};
use sqlx_sqlite::SqlitePool;
use std::sync::Arc;

/// Authentication service
#[allow(dead_code)] // reserved / not yet read
pub struct AuthenticationService {
    user_store: Arc<dyn UserStore>,
    jwt_issuer: JwtIssuer,
    api_key_store: Arc<dyn ApiKeyStore>,
    #[allow(dead_code)] // reserved / not yet read
    password_config: PasswordConfig,
    argon2: Argon2<'static>,
    auth_security_store: Option<Arc<dyn AuthSecurityStore>>,
    enterprise_identity_store: Option<Arc<dyn EnterpriseIdentityStore>>,
}

/// Result of authentication
pub struct AuthenticationResult {
    pub user: User,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: std::time::Duration,
}

impl std::fmt::Debug for AuthenticationResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthenticationResult")
            .field("user", &self.user)
            .field("access_token_present", &!self.access_token.is_empty())
            .field("access_token_len", &self.access_token.len())
            .field("refresh_token_present", &!self.refresh_token.is_empty())
            .field("refresh_token_len", &self.refresh_token.len())
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// Token pair for refresh
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: std::time::Duration,
}

impl std::fmt::Debug for TokenPair {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenPair")
            .field("access_token_present", &!self.access_token.is_empty())
            .field("access_token_len", &self.access_token.len())
            .field("refresh_token_present", &!self.refresh_token.is_empty())
            .field("refresh_token_len", &self.refresh_token.len())
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

/// Context describing why users-core issued tokens for a user.
#[derive(Clone)]
pub struct TokenIssueContext {
    pub source: String,
    pub provider_id: Option<String>,
    pub external_subject: Option<String>,
}

impl std::fmt::Debug for TokenIssueContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenIssueContext")
            .field("source_present", &!self.source.is_empty())
            .field("source_len", &self.source.len())
            .field("provider_present", &self.provider_id.is_some())
            .field("external_subject_present", &self.external_subject.is_some())
            .finish()
    }
}

impl TokenIssueContext {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            provider_id: None,
            external_subject: None,
        }
    }

    pub fn external_identity(
        source: impl Into<String>,
        provider_id: impl Into<String>,
        external_subject: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            provider_id: Some(provider_id.into()),
            external_subject: Some(external_subject.into()),
        }
    }
}

impl AuthenticationService {
    pub fn new(
        user_store: Arc<dyn UserStore>,
        jwt_issuer: JwtIssuer,
        api_key_store: Arc<dyn ApiKeyStore>,
        password_config: PasswordConfig,
    ) -> Result<Self> {
        let argon2 = Argon2::new(
            argon2::Algorithm::Argon2id,
            argon2::Version::V0x13,
            argon2::Params::new(
                password_config.argon2_memory_cost,
                password_config.argon2_time_cost,
                password_config.argon2_parallelism,
                None,
            )
            .map_err(|e| Error::Config(format!("Invalid Argon2 params: {}", e)))?,
        );

        Ok(Self {
            user_store,
            jwt_issuer,
            api_key_store,
            password_config,
            argon2,
            auth_security_store: None,
            enterprise_identity_store: None,
        })
    }

    /// Get a reference to the user store
    pub fn user_store(&self) -> &Arc<dyn UserStore> {
        &self.user_store
    }

    /// Get a reference to the API key store
    pub fn api_key_store(&self) -> &Arc<dyn ApiKeyStore> {
        &self.api_key_store
    }

    /// Get a reference to the JWT issuer
    pub fn jwt_issuer(&self) -> &JwtIssuer {
        &self.jwt_issuer
    }

    /// Set the database pool for refresh token management
    pub fn set_pool(&mut self, pool: SqlitePool) {
        self.auth_security_store = Some(Arc::new(SqliteUserStore::from_pool(pool)));
    }

    /// Set the auth-service security store.
    ///
    /// This provider backs refresh-token revocation, access-token revocation,
    /// password hash updates, last-login updates, and SIP Digest HA1 storage.
    pub fn set_auth_security_store(&mut self, store: Arc<dyn AuthSecurityStore>) {
        self.auth_security_store = Some(store);
    }

    /// Get the configured auth-service security store, if any.
    pub fn auth_security_store(&self) -> Option<&Arc<dyn AuthSecurityStore>> {
        self.auth_security_store.as_ref()
    }

    /// Set enterprise identity extension storage.
    ///
    /// This provider backs external identity links and passkey credentials for
    /// optional SCIM, SAML, OIDC-linking, and WebAuthn extension crates.
    pub fn set_enterprise_identity_store(&mut self, store: Arc<dyn EnterpriseIdentityStore>) {
        self.enterprise_identity_store = Some(store);
    }

    /// Get the configured enterprise identity store, if any.
    pub fn enterprise_identity_store(&self) -> Option<&Arc<dyn EnterpriseIdentityStore>> {
        self.enterprise_identity_store.as_ref()
    }

    /// Create a new user with password hashing and validation
    pub async fn create_user(&self, mut request: CreateUserRequest) -> Result<User> {
        use crate::validation::{
            sanitize_display_name, validate_email, validate_roles, validate_username,
            PasswordValidator,
        };

        // Validate username
        validate_username(&request.username)
            .map_err(|e| Error::Validation(format!("Invalid username: {}", e)))?;

        // Validate email if provided
        if let Some(ref email) = request.email {
            validate_email(email)
                .map_err(|e| Error::Validation(format!("Invalid email: {}", e)))?;
        }

        // Validate roles
        validate_roles(&request.roles)
            .map_err(|e| Error::Validation(format!("Invalid roles: {}", e)))?;

        // Sanitize display name if provided
        if let Some(ref display_name) = request.display_name {
            request.display_name = Some(sanitize_display_name(display_name));
        }

        // Validate password with our policy
        let password_validator = PasswordValidator::with_default_policy();
        password_validator
            .validate(&request.password, &request.username)
            .map_err(|e| Error::InvalidPassword(e.user_message()))?;

        // Hash the password
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = self
            .argon2
            .hash_password(request.password.as_bytes(), &salt)
            .map_err(|e| Error::Internal(anyhow::anyhow!("Failed to hash password: {}", e)))?
            .to_string();

        // Replace plain password with hash
        request.password = password_hash;

        // Create user
        self.user_store.create_user(request).await
    }

    /// Authenticate user with password (constant-time implementation)
    pub async fn authenticate_password(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AuthenticationResult> {
        let user = self.verify_password_user(username, password).await?;
        self.issue_tokens_for_user(&user.id, TokenIssueContext::new("password"))
            .await
    }

    /// Issue access and refresh tokens for an already-authenticated user.
    ///
    /// Extension crates use this after validating external login flows such as
    /// SAML assertions or WebAuthn/passkey ceremonies. This method does not
    /// authenticate the caller; it only centralizes users-core token issuance,
    /// refresh-token storage, active-user checks, and last-login updates.
    pub async fn issue_tokens_for_user(
        &self,
        user_id: &str,
        _context: TokenIssueContext,
    ) -> Result<AuthenticationResult> {
        let user = self
            .user_store
            .get_user(user_id)
            .await?
            .ok_or_else(|| Error::UserNotFound(user_id.to_string()))?;

        if !user.active {
            return Err(Error::InvalidCredentials);
        }

        let access_token = self.jwt_issuer.create_access_token(&user)?;
        let refresh_token = self.jwt_issuer.create_refresh_token(&user.id)?;
        self.store_refresh_token_if_configured(&refresh_token)
            .await?;
        self.update_last_login(&user.id).await?;

        Ok(AuthenticationResult {
            user,
            access_token,
            refresh_token,
            expires_in: std::time::Duration::from_secs(self.jwt_issuer.config.access_ttl_seconds),
        })
    }

    /// Verify username/password credentials without issuing tokens.
    pub async fn verify_password_only(&self, username: &str, password: &str) -> Result<User> {
        self.verify_password_user(username, password).await
    }

    async fn verify_password_user(&self, username: &str, password: &str) -> Result<User> {
        use std::time::Duration;

        // Always fetch user (or use dummy)
        let user_result = self.user_store.get_user_by_username(username).await;

        // Create a dummy hash if user doesn't exist
        // This is a valid Argon2 hash to ensure consistent timing
        const DUMMY_HASH: &str =
            "$argon2id$v=19$m=65536,t=3,p=4$c29tZXNhbHQ$RdescudvJCsgt3ub+b+dWRWJTmaaJObG";

        // Extract user and hash in constant time
        let (user_opt, hash_to_verify) = match user_result {
            Ok(Some(user)) => {
                let hash = user.password_hash.clone();
                (Some(user), hash)
            }
            Ok(None) => (None, DUMMY_HASH.to_string()),
            Err(_) => (None, DUMMY_HASH.to_string()),
        };

        // Parse hash - always succeeds with dummy
        let parsed_hash = PasswordHash::new(&hash_to_verify)
            .unwrap_or_else(|_| PasswordHash::new(DUMMY_HASH).unwrap());

        // Verify password - always runs regardless of user existence
        let password_valid = self
            .argon2
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok();

        // Add small random delay to further obscure timing (100-500 microseconds)
        let delay_us = {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            rng.gen_range(100..500)
        };
        tokio::time::sleep(Duration::from_micros(delay_us)).await;

        // Now check results after constant-time operations
        match (user_opt, password_valid) {
            (Some(user), true) => {
                // Check if account is active
                if !user.active {
                    return Err(Error::InvalidCredentials);
                }
                Ok(user)
            }
            _ => {
                // Failed login - could record for rate limiting here
                Err(Error::InvalidCredentials)
            }
        }
    }

    async fn store_refresh_token_if_configured(&self, refresh_token: &str) -> Result<()> {
        if let Some(store) = &self.auth_security_store {
            let claims = self.jwt_issuer.validate_refresh_token(refresh_token)?;
            let expires_at = chrono::DateTime::<chrono::Utc>::from_timestamp(claims.exp as i64, 0)
                .ok_or_else(|| {
                    Error::Validation("refresh token exp is outside supported range".to_string())
                })?;
            store
                .store_refresh_token(&claims.jti, &claims.sub, expires_at)
                .await?;
        }
        Ok(())
    }

    /// Authenticate with API key
    pub async fn authenticate_api_key(&self, api_key: &str) -> Result<AuthenticationResult> {
        let (user, _) = self.verify_api_key_only(api_key).await?;

        // Generate tokens (API keys get shorter-lived tokens)
        let access_token = self.jwt_issuer.create_access_token(&user)?;
        let refresh_token = self.jwt_issuer.create_refresh_token(&user.id)?;

        Ok(AuthenticationResult {
            user,
            access_token,
            refresh_token,
            expires_in: std::time::Duration::from_secs(300), // 5 minutes for API keys
        })
    }

    /// Verify an API key without issuing tokens.
    pub async fn verify_api_key_only(&self, api_key: &str) -> Result<(User, ApiKey)> {
        let key_info = self
            .api_key_store
            .validate_api_key(api_key)
            .await?
            .ok_or(Error::InvalidCredentials)?;

        let user = self
            .user_store
            .get_user(&key_info.user_id)
            .await?
            .ok_or_else(|| Error::UserNotFound(key_info.user_id.clone()))?;

        if !user.active {
            return Err(Error::InvalidCredentials);
        }

        Ok((user, key_info))
    }

    /// Refresh access token
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenPair> {
        // Validate refresh token
        let claims = self.jwt_issuer.validate_refresh_token(refresh_token)?;

        // Check if revoked (if security storage is available)
        if let Some(store) = &self.auth_security_store {
            store.check_refresh_token_revoked(&claims.jti).await?;
        }

        // Get user
        let user = self
            .user_store
            .get_user(&claims.sub)
            .await?
            .ok_or(Error::UserNotFound(claims.sub.clone()))?;

        // Check if account is still active
        if !user.active {
            return Err(Error::InvalidCredentials);
        }

        // Generate new access token
        let access_token = self.jwt_issuer.create_access_token(&user)?;

        Ok(TokenPair {
            access_token,
            refresh_token: refresh_token.to_string(), // Keep same refresh token
            expires_in: std::time::Duration::from_secs(self.jwt_issuer.config.access_ttl_seconds),
        })
    }

    /// Revoke tokens for a user
    pub async fn revoke_tokens(&self, user_id: &str) -> Result<()> {
        if let Some(store) = &self.auth_security_store {
            store.revoke_refresh_tokens_for_user(user_id).await?;
        }
        Ok(())
    }

    /// Revoke a single access token until its normal expiry.
    ///
    /// Access tokens are stateless and short-lived. This stores the token's
    /// `jti` in the reference SQLite service so validators that enable
    /// revocation checks can reject it immediately.
    pub async fn revoke_access_token(&self, access_token: &str) -> Result<()> {
        let claims = self.jwt_issuer.validate_access_token(access_token)?;
        let expires_at = chrono::DateTime::<chrono::Utc>::from_timestamp(claims.exp as i64, 0)
            .ok_or_else(|| {
                Error::Validation("access token exp is outside supported range".to_string())
            })?;
        self.revoke_access_token_jti(&claims.jti, Some(&claims.sub), expires_at)
            .await
    }

    /// Revoke an access-token JWT ID until expiry.
    pub async fn revoke_access_token_jti(
        &self,
        jti: &str,
        user_id: Option<&str>,
        expires_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        if let Some(store) = &self.auth_security_store {
            store
                .revoke_access_token_jti(jti, user_id, expires_at)
                .await?;
        }
        Ok(())
    }

    /// Check whether an access-token JWT ID is currently revoked.
    pub async fn is_access_token_revoked(&self, jti: &str) -> Result<bool> {
        let Some(store) = &self.auth_security_store else {
            return Ok(false);
        };
        store.is_access_token_revoked(jti).await
    }

    /// Change user password
    pub async fn change_password(
        &self,
        user_id: &str,
        old_password: &str,
        new_password: &str,
    ) -> Result<()> {
        use crate::validation::PasswordValidator;

        // Get user
        let user = self
            .user_store
            .get_user(user_id)
            .await?
            .ok_or(Error::UserNotFound(user_id.to_string()))?;

        // Verify old password
        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|_| Error::Internal(anyhow::anyhow!("Invalid password hash format")))?;

        self.argon2
            .verify_password(old_password.as_bytes(), &parsed_hash)
            .map_err(|_| Error::InvalidCredentials)?;

        // Validate new password with our policy
        let password_validator = PasswordValidator::with_default_policy();
        password_validator
            .validate(new_password, &user.username)
            .map_err(|e| Error::InvalidPassword(e.user_message()))?;

        // Hash new password
        let salt = SaltString::generate(&mut OsRng);
        let new_hash = self
            .argon2
            .hash_password(new_password.as_bytes(), &salt)
            .map_err(|e| Error::Internal(anyhow::anyhow!("Failed to hash password: {}", e)))?
            .to_string();

        // Update password in database
        if let Some(store) = &self.auth_security_store {
            store.update_password_hash(user_id, &new_hash).await?;
        }

        // Revoke all existing tokens
        self.revoke_tokens(user_id).await?;

        Ok(())
    }

    async fn update_last_login(&self, user_id: &str) -> Result<()> {
        if let Some(store) = &self.auth_security_store {
            store.update_last_login(user_id).await?;
        }
        Ok(())
    }
}

// Re-export JwtIssuer config
pub use crate::jwt::JwtConfig;
