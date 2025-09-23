//! Authentication service

use std::sync::Arc;
use crate::{Result, User};

/// Authentication service
pub struct AuthenticationService {
    // TODO: Implement
}

/// Result of authentication
pub struct AuthenticationResult {
    pub user: User,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: std::time::Duration,
}

impl AuthenticationService {
    pub fn new(
        _user_store: Arc<dyn crate::UserStore>,
        _jwt_issuer: crate::JwtIssuer,
        _api_key_store: Arc<dyn crate::ApiKeyStore>,
        _config: crate::config::PasswordConfig,
    ) -> Result<Self> {
        todo!("Implement AuthenticationService")
    }
}
