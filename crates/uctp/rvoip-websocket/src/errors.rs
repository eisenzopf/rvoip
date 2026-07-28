//! `UctpWsError` mirrors the `UctpQuicError` / `UctpWtError` shape.

pub enum UctpWsError {
    Uctp(rvoip_uctp::errors::UctpError),

    Substrate(rvoip_uctp::errors::SubstrateError),

    WebSocket(tokio_tungstenite::tungstenite::Error),

    WebRtc(String),

    NotStarted,

    Shutdown,

    Io(std::io::Error),
}

impl UctpWsError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::Uctp(_) => "uctp",
            Self::Substrate(_) => "substrate",
            Self::WebSocket(_) => "websocket",
            Self::WebRtc(_) => "webrtc",
            Self::NotStarted => "not-started",
            Self::Shutdown => "shutdown",
            Self::Io(_) => "io",
        }
    }
}

impl std::fmt::Display for UctpWsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "UCTP WebSocket error (class={})",
            self.diagnostic_class()
        )
    }
}

impl std::fmt::Debug for UctpWsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UctpWsError")
            .field("class", &self.diagnostic_class())
            .finish()
    }
}

impl std::error::Error for UctpWsError {}

impl From<rvoip_uctp::errors::UctpError> for UctpWsError {
    fn from(error: rvoip_uctp::errors::UctpError) -> Self {
        Self::Uctp(error)
    }
}

impl From<rvoip_uctp::errors::SubstrateError> for UctpWsError {
    fn from(error: rvoip_uctp::errors::SubstrateError) -> Self {
        Self::Substrate(error)
    }
}

impl From<tokio_tungstenite::tungstenite::Error> for UctpWsError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::WebSocket(error)
    }
}

impl From<std::io::Error> for UctpWsError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, UctpWsError>;

#[cfg(feature = "media-webrtc")]
impl From<rvoip_webrtc::WebRtcError> for UctpWsError {
    fn from(e: rvoip_webrtc::WebRtcError) -> Self {
        Self::WebRtc(format!("{e}"))
    }
}
