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

    #[error("not implemented: {0}")]
    NotImplemented(&'static str),
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
