use std::time::Duration;

use rvoip_session_core::{BridgeError, SessionError, SessionId};
use thiserror::Error;

/// Crate-local result type.
pub type Result<T> = std::result::Result<T, B2buaError>;

/// Errors surfaced by the B2BUA orchestration layer.
#[derive(Debug, Error)]
pub enum B2buaError {
    /// Error returned by `session-core`.
    #[error("session-core error: {0}")]
    Session(#[from] SessionError),

    /// Error returned by the media bridge primitive.
    #[error("bridge error: {0}")]
    Bridge(#[from] BridgeError),

    /// The coordinator shut down before another incoming call arrived.
    #[error("incoming call stream closed")]
    IncomingClosed,

    /// A per-leg event stream closed while the call was still being handled.
    #[error("event stream closed for session {0}")]
    EventStreamClosed(SessionId),

    /// The outbound leg did not answer within the configured timeout.
    #[error("outbound leg {session_id} did not answer within {timeout:?}")]
    OutboundAnswerTimeout {
        /// Outbound session id.
        session_id: SessionId,
        /// Timeout that expired.
        timeout: Duration,
    },

    /// The outbound leg failed before the inbound call could be accepted.
    #[error("outbound leg {session_id} failed before answer: {status_code} {reason}")]
    OutboundFailed {
        /// Outbound session id.
        session_id: SessionId,
        /// SIP status code or synthesized status.
        status_code: u16,
        /// Human-readable reason.
        reason: String,
    },

    /// A leg ended before the B2BUA had a complete bridge.
    #[error("{leg} leg {session_id} ended before bridge: {reason}")]
    LegEndedBeforeBridge {
        /// Human-readable leg role.
        leg: &'static str,
        /// Session id.
        session_id: SessionId,
        /// Human-readable reason.
        reason: String,
    },

    /// A leg did not reach `CallState::Active` in time.
    #[error("session {session_id} did not become active within {timeout:?}")]
    ActiveStateTimeout {
        /// Session id.
        session_id: SessionId,
        /// Timeout that expired.
        timeout: Duration,
    },

    /// The router failed before producing a route decision.
    #[error("route failed: {0}")]
    Route(String),
}
