//! Authentication and authorization module for session-core
//! 
//! Provides OAuth 2.0 Bearer token validation (RFC 8898) for SIP

pub mod oauth;
pub mod types;

pub use oauth::{OAuth2Validator, OAuth2Config, OAuth2Scopes};
pub use types::{TokenInfo, AuthError, AuthResult};

// Re-export commonly used items
pub use oauth::RefreshConfig;