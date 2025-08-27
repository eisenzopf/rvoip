//! # Auth-Core - Authentication and Authorization for RVoIP
//! 
//! This crate provides OAuth2 and token-based authentication services
//! for the RVoIP ecosystem, supporting multiple authentication flows
//! and token validation strategies.

pub mod error;
pub mod types;

pub use error::{AuthError, Result};