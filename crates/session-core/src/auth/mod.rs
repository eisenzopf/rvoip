//! Authentication and authorization module for session-core
//!
//! Provides OAuth 2.0 Bearer token validation (RFC 8898) for SIP
//! and SIP Digest Authentication (RFC 2617 / RFC 7616) for REGISTER flows.

pub mod oauth;
pub mod types;
pub mod jwt;
pub mod digest;

pub use oauth::{OAuth2Validator, OAuth2Config, OAuth2Scopes};
pub use types::{TokenInfo, AuthError, AuthResult};
pub use jwt::{JwtValidator, validate_jwt_with_jwks};

// Re-export commonly used items
pub use oauth::RefreshConfig;