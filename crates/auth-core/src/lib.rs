//! # Auth-Core - Authentication and Authorization for RVoIP
//! 
//! This crate provides OAuth2 and token-based authentication services
//! for the RVoIP ecosystem, supporting multiple authentication flows
//! and token validation strategies.

pub mod error;
pub mod types;

pub use error::{AuthError, Result};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexported_auth_error_is_accessible() {
        let err = AuthError::TokenExpired;
        assert_eq!(err.to_string(), "Token expired");
    }

    #[test]
    fn reexported_result_type_works() {
        let ok: Result<i32> = Ok(1);
        assert!(ok.is_ok());
        let err: Result<i32> = Err(AuthError::TokenExpired);
        assert!(err.is_err());
    }

    #[test]
    fn types_module_is_accessible() {
        let ctx = types::UserContext {
            user_id: "test".to_string(),
            username: "tester".to_string(),
            roles: vec![],
            claims: std::collections::HashMap::new(),
            expires_at: None,
            scopes: vec![],
        };
        assert_eq!(ctx.user_id, "test");
    }

    #[test]
    fn token_type_accessible_via_types_module() {
        let _bearer = types::TokenType::Bearer;
        let _jwt = types::TokenType::JWT;
        let _opaque = types::TokenType::Opaque;
    }

    #[test]
    fn auth_method_accessible_via_types_module() {
        let _oauth = types::AuthMethod::OAuth2;
        let _jwt = types::AuthMethod::JWT;
        let _apikey = types::AuthMethod::ApiKey;
        let _sip = types::AuthMethod::SIPDigest;
    }
}