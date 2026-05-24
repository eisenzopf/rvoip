//! # Auth-Core - Authentication and Authorization for RVoIP
//!
//! This crate provides OAuth2 and token-based authentication services
//! for the RVoIP ecosystem, supporting multiple authentication flows
//! and token validation strategies.
//!
//! Also includes SIP Digest authentication support per RFC 2617 and RFC 3261.

pub mod bearer;
pub mod error;
pub mod jwks;
pub mod jwt;
pub mod sip_digest;
pub mod types;

pub use bearer::{bearer_stub, BearerAuthError, BearerValidator};
pub use jwks::{JwksJwtValidator, DEFAULT_JWKS_CACHE_TTL};
pub use jwt::JwtValidator;
pub use error::{AuthError, Result};
pub use sip_digest::{
    DigestAlgorithm, DigestAuthenticator, DigestChallenge, DigestClient, DigestComputed,
    DigestResponse,
};
