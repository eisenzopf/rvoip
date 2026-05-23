//! Crate-wide error types.
//!
//! `UctpError` partitions errors by source (decode, state-machine,
//! capability, auth, transport). Adapter crates wrap `UctpError` /
//! `SubstrateError` with their own outer variant per design doc §3.2.1.

use thiserror::Error;

use rvoip_auth_core::BearerAuthError;

#[derive(Debug, Error)]
pub enum UctpError {
    #[error("envelope decode failed: {0}")]
    Decode(#[from] serde_json::Error),

    #[error("unknown envelope type: {0}")]
    UnknownEnvelopeType(String),

    #[error("missing required field: {0}")]
    MissingField(&'static str),

    #[error("illegal state transition: state={state} event={event}")]
    IllegalTransition {
        state: &'static str,
        event: &'static str,
    },

    /// Typically code 488 (incompatible capabilities).
    #[error("capability negotiation failed: code={code}")]
    CapabilityNegotiationFailed { code: u16 },

    #[error("authentication failed: {0}")]
    Auth(#[from] BearerAuthError),

    #[error("stream-handle exhausted (u16 wrap)")]
    StreamHandleExhausted,

    #[error("operation timed out")]
    Timeout,

    #[error("coordinator closed")]
    Closed,

    #[error(transparent)]
    Transport(#[from] SubstrateError),
}

#[derive(Debug, Error)]
pub enum SubstrateError {
    #[error("quinn connection error: {0}")]
    Quinn(#[from] quinn::ConnectionError),

    #[error("quinn write error: {0}")]
    Write(#[from] quinn::WriteError),

    #[error("quinn read error: {0}")]
    Read(#[from] quinn::ReadError),

    #[error("rustls error: {0}")]
    Tls(#[from] rustls::Error),

    /// Version mismatch, length too short, bad flags.
    #[error("invalid datagram: {0}")]
    InvalidDatagram(&'static str),

    #[error("envelope parse failed: {0}")]
    EnvelopeParse(#[from] serde_json::Error),

    #[error("frame too large: {0} bytes (max 1 MiB)")]
    FrameTooLarge(usize),

    #[error("alpn dispatch closed")]
    DispatchClosed,

    #[error("substrate closed")]
    Closed,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Crate-local `Result` alias matching the rvoip-sip convention.
pub type Result<T> = std::result::Result<T, UctpError>;
