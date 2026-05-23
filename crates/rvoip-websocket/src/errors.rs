//! `UctpWsError` mirrors the `UctpQuicError` / `UctpWtError` shape.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum UctpWsError {
    #[error(transparent)]
    Uctp(#[from] rvoip_uctp::errors::UctpError),

    #[error(transparent)]
    Substrate(#[from] rvoip_uctp::errors::SubstrateError),

    #[error("websocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    #[error("webrtc error: {0}")]
    WebRtc(String),

    #[error("adapter not started")]
    NotStarted,

    #[error("adapter shutdown")]
    Shutdown,

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, UctpWsError>;
