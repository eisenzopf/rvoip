//! Authentication module for session-core-v3
//!
//! Re-exports SIP Digest authentication from auth-core.
//! This follows SIP industry best practices where authentication is a shared module.

// Re-export client-side digest authentication from auth-core
pub use rvoip_auth_core::{
    DigestClient as DigestAuth,
    DigestChallenge,
    DigestAlgorithm,
};
