//! Crate-wide error types.
//!
//! `UctpError` partitions errors by source (decode, state-machine,
//! capability, auth, transport). Adapter crates wrap `UctpError` /
//! `SubstrateError` with their own outer variant per design doc §3.2.1.

use std::fmt;

use rvoip_auth_core::BearerAuthError;

pub enum UctpError {
    Decode(serde_json::Error),

    UnknownEnvelopeType(String),

    MissingField(&'static str),

    IllegalTransition {
        state: &'static str,
        event: &'static str,
    },

    /// Typically code 488 (incompatible capabilities).
    CapabilityNegotiationFailed {
        code: u16,
    },

    Auth(BearerAuthError),

    StreamHandleExhausted,

    InvalidStreamBinding(&'static str),

    Timeout,

    Closed,

    Transport(SubstrateError),
}

pub enum SubstrateError {
    Quinn(quinn::ConnectionError),

    Write(quinn::WriteError),

    Read(quinn::ReadError),

    Tls(rustls::Error),

    /// Version mismatch, length too short, bad flags.
    InvalidDatagram(&'static str),

    EnvelopeParse(serde_json::Error),

    FrameTooLarge(usize),

    DispatchClosed,

    Closed,

    Io(std::io::Error),
}

impl UctpError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::Decode(_) => "decode",
            Self::UnknownEnvelopeType(_) => "unknown-envelope",
            Self::MissingField(_) => "missing-field",
            Self::IllegalTransition { .. } => "illegal-transition",
            Self::CapabilityNegotiationFailed { .. } => "capability-negotiation",
            Self::Auth(_) => "authentication",
            Self::StreamHandleExhausted => "stream-handle-exhausted",
            Self::InvalidStreamBinding(_) => "invalid-stream-binding",
            Self::Timeout => "timeout",
            Self::Closed => "closed",
            Self::Transport(_) => "transport",
        }
    }
}

impl fmt::Display for UctpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "UCTP operation failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for UctpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UctpError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for UctpError {}

impl SubstrateError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::Quinn(_) => "quinn-connection",
            Self::Write(_) => "quinn-write",
            Self::Read(_) => "quinn-read",
            Self::Tls(_) => "tls",
            Self::InvalidDatagram(_) => "invalid-datagram",
            Self::EnvelopeParse(_) => "envelope-parse",
            Self::FrameTooLarge(_) => "frame-too-large",
            Self::DispatchClosed => "dispatch-closed",
            Self::Closed => "closed",
            Self::Io(_) => "io",
        }
    }
}

impl fmt::Display for SubstrateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "substrate failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for SubstrateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SubstrateError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for SubstrateError {}

impl From<serde_json::Error> for UctpError {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(error)
    }
}
impl From<BearerAuthError> for UctpError {
    fn from(error: BearerAuthError) -> Self {
        Self::Auth(error)
    }
}
impl From<SubstrateError> for UctpError {
    fn from(error: SubstrateError) -> Self {
        Self::Transport(error)
    }
}
impl From<quinn::ConnectionError> for SubstrateError {
    fn from(error: quinn::ConnectionError) -> Self {
        Self::Quinn(error)
    }
}
impl From<quinn::WriteError> for SubstrateError {
    fn from(error: quinn::WriteError) -> Self {
        Self::Write(error)
    }
}
impl From<quinn::ReadError> for SubstrateError {
    fn from(error: quinn::ReadError) -> Self {
        Self::Read(error)
    }
}
impl From<rustls::Error> for SubstrateError {
    fn from(error: rustls::Error) -> Self {
        Self::Tls(error)
    }
}
impl From<serde_json::Error> for SubstrateError {
    fn from(error: serde_json::Error) -> Self {
        Self::EnvelopeParse(error)
    }
}
impl From<std::io::Error> for SubstrateError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Crate-local `Result` alias matching the rvoip-sip convention.
pub type Result<T> = std::result::Result<T, UctpError>;
