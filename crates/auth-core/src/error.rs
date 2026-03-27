//! Error types for authentication operations

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid token: {0}")]
    InvalidToken(String),
    
    #[error("Token expired")]
    TokenExpired,
    
    #[error("Insufficient permissions: {0}")]
    InsufficientPermissions(String),
    
    #[error("Authentication provider error: {0}")]
    ProviderError(String),
    
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    #[error("Network error: {0}")]
    NetworkError(String),
    
    #[error("Cache error: {0}")]
    CacheError(String),
    
    #[error("Internal error: {0}")]
    InternalError(String),
}

pub type Result<T> = std::result::Result<T, AuthError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_token_displays_message() {
        let err = AuthError::InvalidToken("bad jwt".to_string());
        assert_eq!(err.to_string(), "Invalid token: bad jwt");
    }

    #[test]
    fn token_expired_displays_message() {
        let err = AuthError::TokenExpired;
        assert_eq!(err.to_string(), "Token expired");
    }

    #[test]
    fn insufficient_permissions_displays_message() {
        let err = AuthError::InsufficientPermissions("admin required".to_string());
        assert_eq!(err.to_string(), "Insufficient permissions: admin required");
    }

    #[test]
    fn provider_error_displays_message() {
        let err = AuthError::ProviderError("oauth2 server down".to_string());
        assert_eq!(err.to_string(), "Authentication provider error: oauth2 server down");
    }

    #[test]
    fn config_error_displays_message() {
        let err = AuthError::ConfigError("missing client_id".to_string());
        assert_eq!(err.to_string(), "Configuration error: missing client_id");
    }

    #[test]
    fn network_error_displays_message() {
        let err = AuthError::NetworkError("connection refused".to_string());
        assert_eq!(err.to_string(), "Network error: connection refused");
    }

    #[test]
    fn cache_error_displays_message() {
        let err = AuthError::CacheError("eviction failed".to_string());
        assert_eq!(err.to_string(), "Cache error: eviction failed");
    }

    #[test]
    fn internal_error_displays_message() {
        let err = AuthError::InternalError("unexpected state".to_string());
        assert_eq!(err.to_string(), "Internal error: unexpected state");
    }

    #[test]
    fn auth_error_is_std_error() {
        let err: Box<dyn std::error::Error> =
            Box::new(AuthError::InvalidToken("test".to_string()));
        assert!(err.to_string().contains("Invalid token"));
    }

    #[test]
    fn auth_error_is_debug() {
        let err = AuthError::TokenExpired;
        let debug = format!("{:?}", err);
        assert!(debug.contains("TokenExpired"));
    }

    #[test]
    fn result_type_ok_variant() {
        let result: Result<u32> = Ok(42);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn result_type_err_variant() {
        let result: Result<u32> = Err(AuthError::TokenExpired);
        assert!(result.is_err());
    }

    #[test]
    fn error_with_empty_message() {
        let err = AuthError::InvalidToken(String::new());
        assert_eq!(err.to_string(), "Invalid token: ");
    }

    #[test]
    fn error_with_special_characters() {
        let err = AuthError::ProviderError("error: <xml>&\"quotes\"</xml>".to_string());
        assert!(err.to_string().contains("<xml>"));
    }
}