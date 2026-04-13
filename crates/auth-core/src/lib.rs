//! # Auth-Core - Authentication and Authorization for RVoIP
//! 
//! This crate provides OAuth2 and token-based authentication services
//! for the RVoIP ecosystem, supporting multiple authentication flows
//! and token validation strategies.
//!
//! Also includes SIP Digest authentication support per RFC 2617 and RFC 3261.

pub mod error;
pub mod types;
pub mod sip_digest;

pub use error::{AuthError, Result};
pub use sip_digest::{
    DigestAuthenticator, DigestClient, DigestChallenge, DigestResponse, DigestAlgorithm
};