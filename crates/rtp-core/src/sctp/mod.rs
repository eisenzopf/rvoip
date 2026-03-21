//! Minimal SCTP-over-DTLS implementation for WebRTC Data Channels
//!
//! This module implements the subset of SCTP (RFC 4960) required for
//! WebRTC Data Channels (RFC 8831 / RFC 8832). It is designed to run
//! on top of a [`DtlsConnection`](crate::dtls::DtlsConnection) and
//! provides reliable, ordered message delivery.
//!
//! The implementation is intentionally minimal -- only the chunk types
//! needed for association setup, data transfer, acknowledgment, and
//! teardown are supported.

pub mod association;
pub mod channel;
mod chunks;

pub use association::SctpAssociation;
pub use channel::{DataChannel, DataChannelConfig, DataChannelEvent, DataChannelState};

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
