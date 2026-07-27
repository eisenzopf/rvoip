//! Error types for the Vapi adapter.

use thiserror::Error;

/// A diagnostic-safe Vapi adapter failure.
///
/// Variants intentionally do not retain API keys, WebSocket URLs, response
/// bodies, transcripts, or assistant definitions.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[non_exhaustive]
pub enum VapiError {
    #[error("invalid Vapi configuration: {0}")]
    InvalidConfiguration(&'static str),
    #[error("invalid Vapi call options: {0}")]
    InvalidCallOptions(&'static str),
    #[error("Vapi HTTP request failed")]
    Http,
    #[error("Vapi HTTP request timed out")]
    HttpTimeout,
    #[error("Vapi returned HTTP status {0}")]
    HttpStatus(u16),
    #[error("Vapi returned an invalid call response: {0}")]
    InvalidResponse(&'static str),
    #[error("Vapi WebSocket setup failed")]
    WebSocketSetup,
    #[error("Vapi WebSocket I/O failed")]
    WebSocketIo,
    #[error("Vapi WebSocket operation timed out")]
    WebSocketTimeout,
    #[error("Vapi media queue overflowed")]
    MediaQueueOverflow,
    #[error("Vapi control queue is unavailable")]
    ControlQueueUnavailable,
    #[error("Vapi control message exceeds the configured size limit")]
    ControlMessageTooLarge,
    #[error("Vapi connection is not active")]
    NotActive,
}

pub type Result<T> = std::result::Result<T, VapiError>;

impl From<VapiError> for rvoip_core::RvoipError {
    fn from(error: VapiError) -> Self {
        rvoip_core::RvoipError::Adapter(error.to_string())
    }
}
