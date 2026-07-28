//! Error type for the Amazon Connect adapter.

use std::fmt;

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, ConnectError>;

/// Stable, value-free classification for metrics, retry, and reconciliation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConnectErrorClass {
    /// Permanent or locally classified Connect control-plane failure.
    ControlPermanent,
    /// Retryable or potentially ambiguous Connect control-plane failure.
    ControlTransient,
    /// A required StartWebRTCContact response field was absent or invalid.
    InvalidResponse,
    /// Chime WebSocket or signaling-protocol failure.
    Signaling,
    /// Typed Chime remote error frame.
    RemoteSignaling,
    /// WebRTC/ICE/DTLS/media failure.
    WebRtc,
    /// No adapter route owns the supplied connection.
    UnknownConnection,
    /// A bounded operation reached its deadline.
    Timeout,
    /// Setup was explicitly cancelled.
    Cancelled,
    /// SIP-header/contact-attribute mapping failed.
    Mapping,
}

/// Failures across the control plane (StartWebRTCContact), the Chime signaling
/// client, and the media bridge.
///
/// Legacy variants and payloads remain source-compatible. `Debug` and
/// `Display` deliberately expose only fixed classes and safe static field names;
/// arbitrary SDK, signaling, media, target, and token details remain private to
/// explicit variant matching by the caller.
pub enum ConnectError {
    /// The AWS `StartWebRTCContact` control-plane call failed.
    Control(String),

    /// Retryable Connect control-plane failure (throttling, timeout, or
    /// temporary service unavailability).
    TransientControl(String),

    /// The control-plane response was missing or invalid in a field required
    /// to join and own the Chime meeting.
    MissingConnectionData(&'static str),

    /// Chime signaling websocket / protocol failure.
    Signaling(String),

    /// The Chime media server returned an error frame.
    ServerFrame {
        status: Option<u32>,
        description: String,
    },

    /// Underlying WebRTC peer-connection / media error (from rvoip-webrtc).
    WebRtc(String),

    /// No route exists for the given connection id.
    UnknownConnection(String),

    /// A timeout waiting on a signaling step or media establishment.
    Timeout(&'static str),

    /// The SIP or Connect peer ended while setup was still in progress.
    Cancelled,

    /// SIP-header → attribute translation produced an invalid attribute set.
    Mapping(String),
}

impl ConnectError {
    /// Stable value-free classification suitable for branching and metrics.
    pub const fn classification(&self) -> ConnectErrorClass {
        match self {
            Self::Control(_) => ConnectErrorClass::ControlPermanent,
            Self::TransientControl(_) => ConnectErrorClass::ControlTransient,
            Self::MissingConnectionData(_) => ConnectErrorClass::InvalidResponse,
            Self::Signaling(_) => ConnectErrorClass::Signaling,
            Self::ServerFrame { .. } => ConnectErrorClass::RemoteSignaling,
            Self::WebRtc(_) => ConnectErrorClass::WebRtc,
            Self::UnknownConnection(_) => ConnectErrorClass::UnknownConnection,
            Self::Timeout(_) => ConnectErrorClass::Timeout,
            Self::Cancelled => ConnectErrorClass::Cancelled,
            Self::Mapping(_) => ConnectErrorClass::Mapping,
        }
    }

    /// Whether the adapter may retry this failure without changing the exact
    /// request or stable client token.
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self.classification(),
            ConnectErrorClass::ControlTransient | ConnectErrorClass::Timeout
        )
    }
}

impl fmt::Debug for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut diagnostic = formatter.debug_struct("ConnectError");
        diagnostic.field("class", &self.classification());
        match self {
            Self::ServerFrame { status, .. } => {
                diagnostic.field("status", status);
            }
            _ => {}
        }
        diagnostic.finish()
    }
}

impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Control(_) => formatter.write_str("Amazon Connect control request failed"),
            Self::TransientControl(_) => {
                formatter.write_str("transient Amazon Connect control failure")
            }
            Self::MissingConnectionData(_) => {
                formatter.write_str("invalid required Amazon Connect response field")
            }
            Self::Signaling(_) => formatter.write_str("Chime signaling failed"),
            Self::ServerFrame { status, .. } => {
                write!(
                    formatter,
                    "Chime server returned an error (status={status:?})"
                )
            }
            Self::WebRtc(_) => formatter.write_str("WebRTC media setup failed"),
            Self::UnknownConnection(_) => formatter.write_str("unknown Amazon Connect connection"),
            Self::Timeout(_) => formatter.write_str("Amazon Connect operation timed out"),
            Self::Cancelled => formatter.write_str("screen-pop setup cancelled"),
            Self::Mapping(_) => formatter.write_str("contact attribute mapping failed"),
        }
    }
}

impl std::error::Error for ConnectError {}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_never_render_retained_arbitrary_details() {
        let canary = "secret-sdk-token-and-target";
        let errors = [
            ConnectError::Control(canary.into()),
            ConnectError::TransientControl(canary.into()),
            ConnectError::Signaling(canary.into()),
            ConnectError::ServerFrame {
                status: Some(500),
                description: canary.into(),
            },
            ConnectError::WebRtc(canary.into()),
            ConnectError::UnknownConnection(canary.into()),
            ConnectError::Mapping(canary.into()),
            ConnectError::MissingConnectionData(canary),
            ConnectError::Timeout(canary),
        ];
        for error in errors {
            assert!(!format!("{error}").contains(canary));
            assert!(!format!("{error:?}").contains(canary));
        }
    }

    #[test]
    fn retry_classification_is_typed() {
        assert!(ConnectError::TransientControl("secret".into()).is_retryable());
        assert!(!ConnectError::Control("secret".into()).is_retryable());
        assert_eq!(
            ConnectError::MissingConnectionData("contact_id").classification(),
            ConnectErrorClass::InvalidResponse
        );
    }
}
