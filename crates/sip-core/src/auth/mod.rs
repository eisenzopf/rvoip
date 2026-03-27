//! # SIP Digest Authentication
//!
//! This module implements RFC 2617/7616 Digest Authentication computation
//! for SIP, providing functions to compute digest responses and build
//! Authorization headers from WWW-Authenticate challenges.

pub mod digest;

pub use digest::{
    compute_digest_response,
    build_authorization_header,
    DigestAuthContext,
};
