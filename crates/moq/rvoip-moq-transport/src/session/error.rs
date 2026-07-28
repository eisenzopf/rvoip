// SPDX-FileCopyrightText: 2024-2026 Cloudflare Inc., Luke Curley, Mike English and contributors
// SPDX-FileCopyrightText: 2023-2024 Luke Curley and contributors
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::{coding, serve};

use super::RequestCapacityError;

/// Draft-19 Session Termination error codes used by transport/application
/// boundaries that must close before a full [`SessionError`] exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SessionTerminationCode {
    InternalError = 0x1,
    Unauthorized = 0x2,
    ProtocolViolation = 0x3,
    KeyValueFormattingError = 0x6,
}

impl SessionTerminationCode {
    pub const fn as_u32(self) -> u32 {
        self as u32
    }
}

#[derive(thiserror::Error, Debug, Clone)]
pub enum SessionError {
    #[error("webtransport error: {0}")]
    WebTransport(#[from] web_transport::Error),

    #[error("encode error: {0}")]
    Encode(#[from] coding::EncodeError),

    #[error("decode error: {0}")]
    Decode(coding::DecodeError),

    /// Draft-19 KEY_VALUE_FORMATTING_ERROR (0x6).
    #[error("key-value formatting error: {0}")]
    KeyValueFormatting(String),

    /// TODO SLG - eventually remove or morph into error for incorrect control message for publisher/subscriber
    /// The role negiotiated in the handshake was violated. For example, a publisher sent a SUBSCRIBE, or a subscriber sent an OBJECT.
    #[error("role violation")]
    RoleViolation,

    /// Some VarInt was too large and we were too lazy to handle it
    #[error("varint bounds exceeded")]
    BoundsExceeded(#[from] coding::BoundsExceeded),

    /// A duplicate ID was used
    #[error("duplicate")]
    Duplicate,

    #[error("internal error")]
    Internal,

    #[error("serve error: {0}")]
    Serve(#[from] serve::ServeError),

    #[error("wrong size")]
    WrongSize,

    /// Draft-19 INVALID_PATH (0x8): PATH was used on the wrong substrate or
    /// identifies an unsupported resource.
    #[error("invalid connection path: {0}")]
    InvalidPath(String),

    /// Draft-19 MALFORMED_PATH (0x9): PATH is not a valid path-abempty and
    /// optional query component.
    #[error("malformed connection path: {0}")]
    MalformedPath(String),

    /// Draft-19 INVALID_AUTHORITY (0x19): AUTHORITY was used on the wrong
    /// substrate or does not identify the accepting server.
    #[error("invalid connection authority: {0}")]
    InvalidAuthority(String),

    /// Draft-19 MALFORMED_AUTHORITY (0x1A): AUTHORITY is syntactically invalid.
    #[error("malformed connection authority: {0}")]
    MalformedAuthority(String),

    /// Draft-16 §3.4 INVALID_REQUEST_ID (0x4): peer used an invalid request ID.
    #[error("invalid request ID")]
    InvalidRequestId,

    /// Draft-16 §3.4 TOO_MANY_REQUESTS (0x7): request ID meets or exceeds the maximum.
    #[error("too many requests")]
    TooManyRequests,

    #[error("request capacity exhausted: {0}")]
    RequestCapacity(#[from] RequestCapacityError),

    /// Draft-19 TOO_MANY_REQUEST_UPDATES (0x1B): the peer exceeded the
    /// per-request-stream limit advertised in MAX_REQUEST_UPDATES.
    #[error("too many request updates")]
    TooManyRequestUpdates,

    /// Draft-16 §3.4 PROTOCOL_VIOLATION (0x3): peer violated a MUST rule.
    #[error("protocol violation: {0}")]
    ProtocolViolation(String),
}

// Session Termination Error Codes from draft-ietf-moq-transport-19.
impl SessionError {
    /// An integer code that is sent over the wire.
    /// Returns Session Termination Error Codes per draft-14.
    pub fn code(&self) -> u64 {
        match self {
            // PROTOCOL_VIOLATION (0x3) - The role negotiated in the handshake was violated
            Self::RoleViolation => 0x3,
            // INTERNAL_ERROR (0x1) - Generic internal errors
            Self::WebTransport(_) => 0x1,
            Self::Encode(_) => 0x1,
            Self::BoundsExceeded(_) => 0x1,
            Self::Internal => 0x1,
            // PROTOCOL_VIOLATION (0x3) - Malformed messages
            Self::Decode(_) => 0x3,
            Self::KeyValueFormatting(_) => 0x6,
            Self::WrongSize => 0x3,
            // Draft-19 setup-option failures.
            Self::InvalidPath(_) => 0x8,
            Self::MalformedPath(_) => 0x9,
            Self::InvalidAuthority(_) => 0x19,
            Self::MalformedAuthority(_) => 0x1A,
            // DUPLICATE_TRACK_ALIAS (0x5)
            Self::Duplicate => 0x5,
            // INVALID_REQUEST_ID (0x4)
            Self::InvalidRequestId => 0x4,
            // TOO_MANY_REQUESTS (0x7)
            Self::TooManyRequests => 0x7,
            Self::RequestCapacity(_) => 0x7,
            // TOO_MANY_REQUEST_UPDATES (0x1B)
            Self::TooManyRequestUpdates => 0x1B,
            // PROTOCOL_VIOLATION (0x3)
            Self::ProtocolViolation(_) => 0x3,
            // Delegate to ServeError for per-request error codes
            Self::Serve(err) => err.code(),
        }
    }

    /// Helper for unimplemented protocol features
    /// Logs a warning and returns a NotImplemented error instead of panicking
    pub fn unimplemented(feature: &str) -> Self {
        Self::Serve(serve::ServeError::not_implemented_ctx(feature))
    }

    /// Returns true if this error represents a graceful connection close.
    ///
    /// For WebTransport, a graceful close is a `CLOSE_WEBTRANSPORT_SESSION` capsule
    /// with code 0. For raw QUIC, it's `APPLICATION_CLOSE` with code 0 (NO_ERROR).
    /// Both are normal session termination, not error conditions.
    ///
    /// This method checks for:
    /// - WebTransport `Closed(0, _)` — web-transport-quinn v0.11+ typically converts
    ///   HTTP/3-encoded `ApplicationClosed` codes into `WebTransportError::Closed(code, reason)`
    ///   during `SessionError` conversion when decoding via `error_from_http3` succeeds
    /// - Raw QUIC `ApplicationClosed` with code 0
    /// - The local side closing the connection (`LocallyClosed`)
    ///
    /// ## Implementation Notes
    ///
    /// We pattern match on `web_transport_quinn::SessionError` variants. In v0.11+,
    /// WebTransport graceful closes arrive as `WebTransportError::Closed(0, _)` because
    /// the crate decodes HTTP/3 error codes at the `SessionError` level. For raw QUIC
    /// connections, the close code is checked directly on `ConnectionError::ApplicationClosed`.
    ///
    /// **Coupling note**: This implementation is coupled to `web-transport-quinn` and
    /// `quinn`. When transitioning to a different WebTransport backend (e.g., tokio-quiche),
    /// ensure the replacement provides equivalent error introspection, or update this
    /// method to handle the new error types.
    pub fn is_graceful_close(&self) -> bool {
        match self {
            Self::WebTransport(wt_err) => match wt_err {
                web_transport::Error::Session(session_err) => {
                    is_session_error_graceful(session_err)
                }
                web_transport::Error::Read(read_err) => {
                    if let web_transport::quinn::ReadError::SessionError(session_err) = read_err {
                        return is_session_error_graceful(session_err);
                    }
                    false
                }
                web_transport::Error::Write(write_err) => {
                    if let web_transport::quinn::WriteError::SessionError(session_err) = write_err {
                        return is_session_error_graceful(session_err);
                    }
                    false
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// True when a peer cancelled only the current request/data stream.
    /// Stream resets are request scoped and must not tear down the session.
    pub fn is_request_stream_cancelled(&self) -> bool {
        matches!(
            self,
            Self::WebTransport(web_transport::Error::Read(
                web_transport::quinn::ReadError::Reset(_)
            )) | Self::WebTransport(web_transport::Error::Write(
                web_transport::quinn::WriteError::Stopped(_)
            ))
        )
    }
}

impl From<coding::DecodeError> for SessionError {
    fn from(error: coding::DecodeError) -> Self {
        match error {
            coding::DecodeError::AuthorizationTokenFormatting(reason) => {
                Self::KeyValueFormatting(reason)
            }
            error => Self::Decode(error),
        }
    }
}

impl From<SessionError> for serve::ServeError {
    fn from(err: SessionError) -> Self {
        match err {
            SessionError::Serve(err) => err,
            _ => serve::ServeError::internal_ctx(format!("session error: {}", err)),
        }
    }
}

/// Helper to check if a `web_transport_quinn::SessionError` represents a graceful close.
///
/// This handles:
/// - WebTransport connections: `WebTransportError::Closed(0, _)` — web-transport-quinn v0.11+
///   typically decodes HTTP/3-encoded close codes at this layer (when `SessionError` conversion
///   applies), so graceful closes usually arrive here rather than as a raw
///   `ConnectionError::ApplicationClosed`.
/// - Raw QUIC connections: `ConnectionError::ApplicationClosed` with code 0
/// - Local close: `ConnectionError::LocallyClosed`
fn is_session_error_graceful(err: &web_transport::quinn::SessionError) -> bool {
    use web_transport::quinn::{SessionError, WebTransportError};

    match err {
        SessionError::ConnectionError(conn_err) => is_connection_error_graceful(conn_err),
        // WebTransport graceful close: peer sent close with code 0
        SessionError::WebTransportError(WebTransportError::Closed(0, _)) => true,
        // Other WebTransport errors (UnknownSession, read/write errors, non-zero close codes)
        SessionError::WebTransportError(_) => false,
        // SendDatagramError doesn't represent connection close
        SessionError::SendDatagramError(_) => false,
    }
}

/// Helper to check if a `quinn::ConnectionError` represents a graceful close.
///
/// Note: In web-transport-quinn v0.11+, WebTransport `ApplicationClosed` with an HTTP/3-encoded
/// close code is usually converted to `WebTransportError::Closed` during `SessionError` conversion
/// when decoding succeeds. This function primarily handles raw QUIC (moqt:// ALPN) connections
/// or non-decodable cases where the close code is not HTTP/3 encoded.
fn is_connection_error_graceful(err: &web_transport::quinn::quinn::ConnectionError) -> bool {
    use web_transport::quinn::quinn::ConnectionError;

    match err {
        ConnectionError::ApplicationClosed(close) => {
            let code = close.error_code.into_inner();

            // Check for raw QUIC code 0 (direct MoQ-over-QUIC)
            if code == 0 {
                return true;
            }

            // Check for WebTransport code 0 (HTTP/3 encoded)
            // This is a fallback — in v0.11+, WebTransport closes are typically caught
            // by is_session_error_graceful's WebTransportError::Closed branch.
            if let Some(wt_code) = web_transport::quinn::proto::error_from_http3(code) {
                return wt_code == 0;
            }

            false
        }
        // LocallyClosed means we closed the connection ourselves
        ConnectionError::LocallyClosed => true,
        // Other errors are not graceful closes
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn too_many_request_updates_uses_draft_19_code() {
        assert_eq!(SessionError::TooManyRequestUpdates.code(), 0x1B);
    }

    #[test]
    fn path_and_authority_errors_use_draft_19_codes() {
        assert_eq!(SessionError::InvalidPath(String::new()).code(), 0x8);
        assert_eq!(SessionError::MalformedPath(String::new()).code(), 0x9);
        assert_eq!(SessionError::InvalidAuthority(String::new()).code(), 0x19);
        assert_eq!(SessionError::MalformedAuthority(String::new()).code(), 0x1A);
    }

    #[test]
    fn request_stream_reset_is_scoped_cancellation() {
        let error = SessionError::WebTransport(web_transport::Error::Read(
            web_transport::quinn::ReadError::Reset(1),
        ));
        assert!(error.is_request_stream_cancelled());
        assert!(
            !SessionError::ProtocolViolation("bad frame".to_string()).is_request_stream_cancelled()
        );
    }
}
