//! Authentication service

use std::sync::Arc;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use password_hash::{rand_core::OsRng, SaltString};
use sqlx::{SqlitePool, Row};
use crate::{Result, Error, User, UserStore, ApiKeyStore, JwtIssuer, CreateUserRequest};
use crate::jwt::RefreshTokenClaims;
use crate::config::PasswordConfig;

/// Authentication service
pub struct AuthenticationService {
    user_store: Arc<dyn UserStore>,
    jwt_issuer: JwtIssuer,
    api_key_store: Arc<dyn ApiKeyStore>,
    password_config: PasswordConfig,
    argon2: Argon2<'static>,
    pool: Option<SqlitePool>,  // For refresh token management
}

/// Result of authentication
#[derive(Debug)]
pub struct AuthenticationResult {
    pub user: User,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: std::time::Duration,
}

/// Token pair for refresh
#[derive(Debug)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: std::time::Duration,
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
                None
            ).map_err(|e| Error::Config(format!("Invalid Argon2 params: {}", e)))?
        );
        
        Ok(Self {
            user_store,
            jwt_issuer,
            api_key_store,
            password_config,
            argon2,
            pool: None,
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
        self.pool = Some(pool);
    }
    
    /// Create a new user with password hashing
    pub async fn create_user(&self, mut request: CreateUserRequest) -> Result<User> {
        // Validate password
        self.validate_password_strength(&request.password)?;
        
        // Hash the password
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = self.argon2
            .hash_password(request.password.as_bytes(), &salt)
            .map_err(|e| Error::Internal(anyhow::anyhow!("Failed to hash password: {}", e)))?
            .to_string();
        
        // Replace plain password with hash
        request.password = password_hash;
        
        // Create user
        self.user_store.create_user(request).await
    }
    
    /// Authenticate user with password
    pub async fn authenticate_password(
        &self,
        username: &str,
        password: &str,
    ) -> Result<AuthenticationResult> {
        // Get user
        let user = self.user_store
            .get_user_by_username(username)
            .await?
            .ok_or(Error::InvalidCredentials)?;
        
        // Verify password
        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|_| Error::Internal(anyhow::anyhow!("Invalid password hash format")))?;
        
        self.argon2
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| Error::InvalidCredentials)?;
        
        // Check if account is active
        if !user.active {
            return Err(Error::InvalidCredentials);
        }
        
        // Generate tokens
        let access_token = self.jwt_issuer.create_access_token(&user)?;
        let refresh_token = self.jwt_issuer.create_refresh_token(&user.id)?;
        
        // Store refresh token JTI if pool is available
        if let Some(pool) = &self.pool {
            let claims = self.jwt_issuer.validate_refresh_token(&refresh_token)?;
            self.store_refresh_token(pool, &claims).await?;
        }
        
        // Update last login
        self.update_last_login(&user.id).await?;
        
        Ok(AuthenticationResult {
            user,
            access_token,
            refresh_token,
            expires_in: std::time::Duration::from_secs(self.jwt_issuer.config.access_ttl_seconds),
        })
    }
    
    /// Authenticate with API key
    pub async fn authenticate_api_key(
        &self,
        api_key: &str,
    ) -> Result<AuthenticationResult> {
        // Validate API key
        let key_info = self.api_key_store
            .validate_api_key(api_key)
            .await?
            .ok_or(Error::InvalidCredentials)?;
        
        // Get associated user
        let user = self.user_store
            .get_user(&key_info.user_id)
            .await?
            .ok_or(Error::UserNotFound(key_info.user_id.clone()))?;
        
        // Check if account is active
        if !user.active {
            return Err(Error::InvalidCredentials);
        }
        
        // Generate tokens (API keys get shorter-lived tokens)
        let access_token = self.jwt_issuer.create_access_token(&user)?;
        let refresh_token = self.jwt_issuer.create_refresh_token(&user.id)?;
        
        Ok(AuthenticationResult {
            user,
            access_token,
            refresh_token,
            expires_in: std::time::Duration::from_secs(300),  // 5 minutes for API keys
        })
    }
    
    /// Refresh access token
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenPair> {
        // Validate refresh token
        let claims = self.jwt_issuer.validate_refresh_token(refresh_token)?;
        
        // Check if revoked (if pool is available)
        if let Some(pool) = &self.pool {
            self.check_refresh_token_revoked(pool, &claims.jti).await?;
        }
        
        // Get user
        let user = self.user_store
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
            refresh_token: refresh_token.to_string(),  // Keep same refresh token
            expires_in: std::time::Duration::from_secs(self.jwt_issuer.config.access_ttl_seconds),
        })
    }
    
    /// Revoke tokens for a user
    pub async fn revoke_tokens(&self, user_id: &str) -> Result<()> {
        if let Some(pool) = &self.pool {
            sqlx::query("UPDATE refresh_tokens SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL")
                .bind(&chrono::Utc::now())
                .bind(user_id)
                .execute(pool)
                .await?;
        }
        Ok(())
    }
    
    /// Change user password
    pub async fn change_password(&self, user_id: &str, old_password: &str, new_password: &str) -> Result<()> {
        // Get user
        let user = self.user_store
            .get_user(user_id)
            .await?
            .ok_or(Error::UserNotFound(user_id.to_string()))?;
        
        // Verify old password
        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|_| Error::Internal(anyhow::anyhow!("Invalid password hash format")))?;
        
        self.argon2
            .verify_password(old_password.as_bytes(), &parsed_hash)
            .map_err(|_| Error::InvalidCredentials)?;
        
        // Validate new password
        self.validate_password_strength(new_password)?;
        
        // Hash new password
        let salt = SaltString::generate(&mut OsRng);
        let new_hash = self.argon2
            .hash_password(new_password.as_bytes(), &salt)
            .map_err(|e| Error::Internal(anyhow::anyhow!("Failed to hash password: {}", e)))?
            .to_string();
        
        // Update password in database
        if let Some(pool) = &self.pool {
            sqlx::query("UPDATE users SET password_hash = ?, updated_at = ? WHERE id = ?")
                .bind(&new_hash)
                .bind(&chrono::Utc::now())
                .bind(user_id)
                .execute(pool)
                .await?;
        }
        
        // Revoke all existing tokens
        self.revoke_tokens(user_id).await?;
        
        Ok(())
    }
    
    fn validate_password_strength(&self, password: &str) -> Result<()> {
        if password.len() < self.password_config.min_length {
            return Err(Error::InvalidPassword(
                format!("Password must be at least {} characters", self.password_config.min_length)
            ));
        }
        
        if self.password_config.require_uppercase && !password.chars().any(|c| c.is_uppercase()) {
            return Err(Error::InvalidPassword("Password must contain uppercase letter".to_string()));
        }
        
        if self.password_config.require_lowercase && !password.chars().any(|c| c.is_lowercase()) {
            return Err(Error::InvalidPassword("Password must contain lowercase letter".to_string()));
        }
        
        if self.password_config.require_numbers && !password.chars().any(|c| c.is_numeric()) {
            return Err(Error::InvalidPassword("Password must contain number".to_string()));
        }
        
        if self.password_config.require_special && !password.chars().any(|c| !c.is_alphanumeric()) {
            return Err(Error::InvalidPassword("Password must contain special character".to_string()));
        }
        
        Ok(())
    }
    
    async fn update_last_login(&self, user_id: &str) -> Result<()> {
        if let Some(pool) = &self.pool {
            sqlx::query("UPDATE users SET last_login = ? WHERE id = ?")
                .bind(&chrono::Utc::now())
                .bind(user_id)
                .execute(pool)
                .await?;
        }
        Ok(())
    }
    
    async fn store_refresh_token(&self, pool: &SqlitePool, claims: &RefreshTokenClaims) -> Result<()> {
        sqlx::query(
            "INSERT INTO refresh_tokens (jti, user_id, expires_at, created_at)
             VALUES (?, ?, ?, ?)"
        )
        .bind(&claims.jti)
        .bind(&claims.sub)
        .bind(&chrono::DateTime::<chrono::Utc>::from_timestamp(claims.exp as i64, 0))
        .bind(&chrono::Utc::now())
        .execute(pool)
        .await?;
        
        Ok(())
    }
    
    async fn check_refresh_token_revoked(&self, pool: &SqlitePool, jti: &str) -> Result<()> {
        let row = sqlx::query("SELECT revoked_at FROM refresh_tokens WHERE jti = ?")
            .bind(jti)
            .fetch_optional(pool)
            .await?;
        
        if let Some(row) = row {
            let revoked_at: Option<chrono::DateTime<chrono::Utc>> = row.get("revoked_at");
            if revoked_at.is_some() {
                return Err(Error::InvalidCredentials);
            }
        }
        
        Ok(())
    }
}

// Re-export JwtIssuer config
pub use crate::jwt::JwtConfig;
