//! Error types for signing and verification operations.

use thiserror::Error;

/// Errors that can occur while signing an outbound PASSporT.
#[derive(Debug, Error)]
pub enum SignerError {
    #[error("key material unavailable: {0}")]
    KeyUnavailable(String),

    #[error("JWS signing failed: {0}")]
    SigningFailed(String),

    #[error("invalid claims: {0}")]
    InvalidClaims(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors that can occur while verifying an inbound `Identity` header.
///
/// The dialog layer converts these into the canonical 4xx response
/// codes per RFC 8224 §6.2.2 when `VerificationPolicy::RequireValid` /
/// `StrictReject` is configured.
#[derive(Debug, Error)]
pub enum VerifierError {
    /// 437 Unsupported Credential
    #[error("certificate fetch failed: {0}")]
    CertFetch(String),

    /// 437 Unsupported Credential
    #[error("certificate chain validation failed: {0}")]
    CertChain(String),

    /// 438 Invalid Identity Header
    #[error("JWS signature invalid")]
    BadSignature,

    /// 438 Invalid Identity Header
    #[error("PASSporT claim {field} does not match SIP {sip_field}")]
    ClaimMismatch {
        field: &'static str,
        sip_field: &'static str,
    },

    /// 403 Stale Date — per RFC 8224 §6.2.2 (iat outside freshness window)
    #[error("PASSporT iat outside freshness window (skew {skew_secs}s)")]
    StaleDate { skew_secs: i64 },

    /// 436 Bad Identity Info — cert URL malformed or unsupported scheme
    #[error("malformed info= URL: {0}")]
    BadInfo(String),

    #[error("PASSporT JWT parse failed: {0}")]
    BadToken(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
