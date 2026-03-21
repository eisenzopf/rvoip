//! DTMF support per RFC 4733 (telephone-event).
//!
//! This module provides:
//!
//! - **Event types** — [`DtmfEvent`] representing DTMF digits 0-9, *, #, A-D.
//! - **Packet codec** — Encode/decode the 4-byte RFC 4733 RTP payload.
//! - **Packet generation** — Build the full sequence of RTP packets for a DTMF
//!   key-press, including end-of-event retransmissions.
//! - **SDP helpers** — Utilities for adding `telephone-event` to SDP offers
//!   and recognizing it in SDP answers.
//!
//! # Example
//!
//! ```rust
//! use rvoip_media_core::dtmf::{DtmfEvent, DtmfPacket, encode_dtmf_packet, decode_dtmf_packet};
//!
//! let pkt = DtmfPacket::new(DtmfEvent::Digit5, false, 10, 800);
//! let bytes = encode_dtmf_packet(&pkt);
//! let decoded = decode_dtmf_packet(&bytes).expect("valid payload");
//! assert_eq!(decoded.event, DtmfEvent::Digit5);
//! ```

pub mod event;
pub mod codec;

// Re-export public API
pub use event::{DtmfEvent, DtmfPacket};
pub use codec::{
    encode_dtmf_packet,
    decode_dtmf_packet,
    generate_dtmf_rtp_packets,
    generate_dtmf_rtp_packets_with_ptime,
};

/// Default RTP payload type for telephone-event (commonly negotiated as 101).
pub const TELEPHONE_EVENT_PT: u8 = 101;

/// Default clock rate for telephone-event (RFC 4733 Section 4).
pub const TELEPHONE_EVENT_CLOCK_RATE: u32 = 8000;

/// The SDP rtpmap encoding name for telephone-event.
pub const TELEPHONE_EVENT_ENCODING_NAME: &str = "telephone-event";

/// Returns an SDP `a=rtpmap` line for telephone-event.
///
/// # Arguments
/// * `payload_type` — The dynamic payload type (e.g. 101).
/// * `clock_rate` — The clock rate (typically 8000).
///
/// # Example output
/// `"101 telephone-event/8000"`
pub fn sdp_rtpmap(payload_type: u8, clock_rate: u32) -> String {
    format!("{} {}/{}", payload_type, TELEPHONE_EVENT_ENCODING_NAME, clock_rate)
}

/// Returns an SDP `a=fmtp` value for telephone-event.
///
/// The standard value `"0-16"` indicates support for DTMF events 0-15
/// plus the flash event (16).
///
/// # Arguments
/// * `payload_type` — The dynamic payload type (e.g. 101).
///
/// # Example output
/// `"101 0-16"`
pub fn sdp_fmtp(payload_type: u8) -> String {
    format!("{} 0-16", payload_type)
}

/// Checks whether an SDP rtpmap value describes a telephone-event codec.
///
/// Accepts values like `"101 telephone-event/8000"` or
/// `"telephone-event/8000"` (encoding name with or without PT prefix).
pub fn is_telephone_event_rtpmap(rtpmap: &str) -> bool {
    let lower = rtpmap.to_ascii_lowercase();
    lower.contains("telephone-event")
}

/// Extracts the payload type and clock rate from a telephone-event rtpmap value.
///
/// Returns `None` if the value does not describe a telephone-event codec.
///
/// Accepted formats:
/// - `"101 telephone-event/8000"`
/// - `"telephone-event/8000"` (returns `None` for PT in that case)
pub fn parse_telephone_event_rtpmap(rtpmap: &str) -> Option<(Option<u8>, u32)> {
    if !is_telephone_event_rtpmap(rtpmap) {
        return None;
    }

    // Try to split "PT encoding/clock"
    let trimmed = rtpmap.trim();
    let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();

    match parts.len() {
        2 => {
            // "101 telephone-event/8000"
            let pt = parts[0].parse::<u8>().ok();
            let clock = extract_clock_rate(parts[1]);
            Some((pt, clock))
        }
        1 => {
            // "telephone-event/8000"
            let clock = extract_clock_rate(parts[0]);
            Some((None, clock))
        }
        _ => None,
    }
}

/// Extracts clock rate from "telephone-event/8000" or similar.
fn extract_clock_rate(s: &str) -> u32 {
    if let Some(slash_pos) = s.rfind('/') {
        s[slash_pos + 1..].parse::<u32>().unwrap_or(TELEPHONE_EVENT_CLOCK_RATE)
    } else {
        TELEPHONE_EVENT_CLOCK_RATE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdp_rtpmap() {
        let rtpmap = sdp_rtpmap(101, 8000);
        assert_eq!(rtpmap, "101 telephone-event/8000");
    }

    #[test]
    fn test_sdp_fmtp() {
        let fmtp = sdp_fmtp(101);
        assert_eq!(fmtp, "101 0-16");
    }

    #[test]
    fn test_is_telephone_event_rtpmap() {
        assert!(is_telephone_event_rtpmap("101 telephone-event/8000"));
        assert!(is_telephone_event_rtpmap("96 telephone-event/16000"));
        assert!(is_telephone_event_rtpmap("telephone-event/8000"));
        assert!(is_telephone_event_rtpmap("101 TELEPHONE-EVENT/8000"));
        assert!(!is_telephone_event_rtpmap("0 PCMU/8000"));
        assert!(!is_telephone_event_rtpmap("opus/48000/2"));
    }

    #[test]
    fn test_parse_telephone_event_rtpmap() {
        let result = parse_telephone_event_rtpmap("101 telephone-event/8000");
        assert_eq!(result, Some((Some(101), 8000)));

        let result = parse_telephone_event_rtpmap("96 telephone-event/16000");
        assert_eq!(result, Some((Some(96), 16000)));

        let result = parse_telephone_event_rtpmap("telephone-event/8000");
        assert_eq!(result, Some((None, 8000)));

        let result = parse_telephone_event_rtpmap("0 PCMU/8000");
        assert_eq!(result, None);
    }

    #[test]
    fn test_default_constants() {
        assert_eq!(TELEPHONE_EVENT_PT, 101);
        assert_eq!(TELEPHONE_EVENT_CLOCK_RATE, 8000);
        assert_eq!(TELEPHONE_EVENT_ENCODING_NAME, "telephone-event");
    }
}
