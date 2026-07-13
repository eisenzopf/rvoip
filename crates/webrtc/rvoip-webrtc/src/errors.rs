use std::fmt;

pub type Result<T> = std::result::Result<T, WebRtcError>;

pub enum WebRtcError {
    Webrtc(String),
    Adapter(String),
    Sdp(String),
    Signaling(String),
    Timeout(&'static str),
    ConnectionNotFound,
    IncompatibleCapabilities,
    WrongRole {
        expected: &'static str,
        actual: &'static str,
    },

    AlreadySubscribed,
    NotImplemented(&'static str),
    InvalidArgument(String),
    Unauthorized(String),
    Forbidden(String),
    PreconditionFailed(String),
    InvalidState(&'static str),

    /// Fixed, credential-free failure returned by secure inbound signaling
    /// for every missing, rejected, expired, or raced admission outcome.
    InboundAdmissionRejected,
    FingerprintNotPinned,
}

impl WebRtcError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::Webrtc(_) => "webrtc",
            Self::Adapter(_) => "adapter",
            Self::Sdp(_) => "sdp",
            Self::Signaling(_) => "signaling",
            Self::Timeout(_) => "timeout",
            Self::ConnectionNotFound => "connection-not-found",
            Self::IncompatibleCapabilities => "incompatible-capabilities",
            Self::WrongRole { .. } => "wrong-role",
            Self::AlreadySubscribed => "already-subscribed",
            Self::NotImplemented(_) => "not-implemented",
            Self::InvalidArgument(_) => "invalid-argument",
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::PreconditionFailed(_) => "precondition-failed",
            Self::InvalidState(_) => "invalid-state",
            Self::InboundAdmissionRejected => "inbound-admission",
            Self::FingerprintNotPinned => "fingerprint-not-pinned",
        }
    }
}

impl fmt::Display for WebRtcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "WebRTC operation failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for WebRtcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebRtcError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for WebRtcError {}

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
