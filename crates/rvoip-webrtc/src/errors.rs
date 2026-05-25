use thiserror::Error;

pub type Result<T> = std::result::Result<T, WebRtcError>;

#[derive(Debug, Error)]
pub enum WebRtcError {
    #[error("webrtc-rs: {0}")]
    Webrtc(String),

    #[error("adapter: {0}")]
    Adapter(String),

    #[error("sdp: {0}")]
    Sdp(String),

    #[error("signaling: {0}")]
    Signaling(String),

    #[error("timeout waiting for {0}")]
    Timeout(&'static str),

    #[error("connection not found")]
    ConnectionNotFound,

    #[error("incompatible capabilities")]
    IncompatibleCapabilities,

    #[error("wrong peer role: expected {expected}, got {actual}")]
    WrongRole {
        expected: &'static str,
        actual: &'static str,
    },

    #[error("subscribe_events already taken; only one subscriber is supported")]
    AlreadySubscribed,

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("unauthorized: {0}")]
    Unauthorized(String),

    #[error("precondition failed: {0}")]
    PreconditionFailed(String),

    #[error("invalid state: {0}")]
    InvalidState(&'static str),

    #[error("DTLS fingerprint not in pinned list")]
    FingerprintNotPinned,
}

impl From<webrtc::error::Error> for WebRtcError {
    fn from(e: webrtc::error::Error) -> Self {
        Self::Webrtc(format!("{e}"))
    }
}

impl From<rvoip_core::error::RvoipError> for WebRtcError {
    fn from(e: rvoip_core::error::RvoipError) -> Self {
        Self::Adapter(format!("{e}"))
    }
}
