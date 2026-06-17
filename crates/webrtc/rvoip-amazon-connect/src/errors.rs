//! Error type for the Amazon Connect adapter.

use thiserror::Error;

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, ConnectError>;

/// Failures across the control plane (StartWebRTCContact), the Chime signaling
/// client, and the media bridge.
#[derive(Debug, Error)]
pub enum ConnectError {
    /// The AWS `StartWebRTCContact` control-plane call failed.
    #[error("StartWebRTCContact failed: {0}")]
    Control(String),

    /// The control-plane response was missing a field we require to join the
    /// Chime meeting (signaling URL, join token, etc.).
    #[error("incomplete ConnectionData from StartWebRTCContact: missing {0}")]
    MissingConnectionData(&'static str),

    /// Chime signaling websocket / protocol failure.
    #[error("Chime signaling error: {0}")]
    Signaling(String),

    /// The Chime media server returned an error frame.
    #[error("Chime server error: status={status:?} {description}")]
    ServerFrame {
        status: Option<u32>,
        description: String,
    },

    /// Underlying WebRTC peer-connection / media error (from rvoip-webrtc).
    #[error("WebRTC error: {0}")]
    WebRtc(String),

    /// No route exists for the given connection id.
    #[error("unknown connection: {0}")]
    UnknownConnection(String),

    /// A timeout waiting on a signaling step or media establishment.
    #[error("timeout waiting for {0}")]
    Timeout(&'static str),

    /// SIP-header → attribute translation produced an invalid attribute set.
    #[error("attribute mapping error: {0}")]
    Mapping(String),
}

impl From<rvoip_webrtc::WebRtcError> for ConnectError {
    fn from(e: rvoip_webrtc::WebRtcError) -> Self {
        ConnectError::WebRtc(e.to_string())
    }
}

impl From<ConnectError> for rvoip_core::error::RvoipError {
    fn from(e: ConnectError) -> Self {
        rvoip_core::error::RvoipError::Adapter(e.to_string())
    }
}
