//! SCTP-over-DTLS for WebRTC Data Channels
//!
//! This module provides SCTP transport on top of a
//! [`DtlsConnection`](crate::dtls::DtlsConnection) for WebRTC Data
//! Channels (RFC 8831 / RFC 8832).
//!
//! ## Adapter (production)
//!
//! The [`adapter`] sub-module wraps the `webrtc-sctp` crate, which
//! provides a full RFC 4960 implementation with reliability, flow
//! control, and congestion control. **New code should use
//! [`adapter::SctpAssociationAdapter`].**
//!
//! ## Legacy stub
//!
//! The [`association`] sub-module contains a minimal, hand-rolled SCTP
//! implementation (~20% RFC coverage, no retransmission or congestion
//! control). It is **deprecated** and retained only for reference.

pub mod adapter;
#[deprecated(
    since = "0.1.26",
    note = "Use `adapter::SctpAssociationAdapter` backed by webrtc-sctp instead"
)]
pub mod association;
pub mod channel;
mod chunks;

pub use adapter::{DtlsConnBridge, SctpAssociationAdapter, SctpStreamAdapter};
pub use channel::{DataChannel, DataChannelConfig, DataChannelEvent, DataChannelState};

// Re-export the legacy association under a deprecated name so existing
// call-sites get a compiler warning.
#[allow(deprecated)]
pub use association::SctpAssociation;

/// Generate an SDP `m=application` line for WebRTC data channels.
///
/// # Arguments
/// * `port` - The UDP port number to advertise.
pub fn sdp_media_line(port: u16) -> String {
    format!("m=application {} UDP/DTLS/SCTP webrtc-datachannel", port)
}

/// Generate SDP attribute lines for SCTP data channels.
///
/// # Arguments
/// * `sctp_port` - The SCTP port to advertise in `a=sctp-port`.
/// * `max_message_size` - The maximum message size in bytes.
pub fn sdp_attributes(sctp_port: u16, max_message_size: u64) -> Vec<String> {
    vec![
        format!("a=sctp-port:{}", sctp_port),
        format!("a=max-message-size:{}", max_message_size),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdp_media_line() {
        let line = sdp_media_line(9);
        assert_eq!(line, "m=application 9 UDP/DTLS/SCTP webrtc-datachannel");
    }

    #[test]
    fn test_sdp_attributes() {
        let attrs = sdp_attributes(5000, 262144);
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0], "a=sctp-port:5000");
        assert_eq!(attrs[1], "a=max-message-size:262144");
    }
}
