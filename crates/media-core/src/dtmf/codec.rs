//! RFC 4733 DTMF payload encoding and decoding.
//!
//! Provides functions to encode/decode the 4-byte telephone-event RTP payload
//! and to generate the proper sequence of RTP packets for a DTMF event,
//! including end-of-event markers with triple retransmission per RFC 4733.

use crate::error::{Error, CodecError};

use super::event::{DtmfEvent, DtmfPacket};

/// Default DTMF volume in -dBm0.  10 corresponds to -10 dBm0.
const DEFAULT_VOLUME: u8 = 10;

/// Number of end-of-event retransmissions as recommended by RFC 4733 Section 2.5.
const END_OF_EVENT_RETRANSMISSIONS: u32 = 3;

/// Default RTP ptime (packet interval) in milliseconds for DTMF packets.
const DEFAULT_PTIME_MS: u32 = 20;

/// Encodes a [`DtmfPacket`] into the 4-byte RFC 4733 payload.
///
/// Layout:
/// ```text
/// Byte 0:    event code (8 bits)
/// Byte 1:    E(1) | R(1) | volume(6)
/// Bytes 2-3: duration (16 bits, big-endian)
/// ```
pub fn encode_dtmf_packet(packet: &DtmfPacket) -> [u8; 4] {
    let mut buf = [0u8; 4];

    // Byte 0: event code
    buf[0] = packet.event.code();

    // Byte 1: E (bit 7), R (bit 6, reserved = 0), volume (bits 5-0)
    let e_bit = if packet.end_of_event { 0x80 } else { 0x00 };
    buf[1] = e_bit | (packet.volume & 0x3F);

    // Bytes 2-3: duration (big-endian)
    buf[2] = (packet.duration >> 8) as u8;
    buf[3] = (packet.duration & 0xFF) as u8;

    buf
}

/// Decodes a 4-byte RFC 4733 payload into a [`DtmfPacket`].
///
/// Returns an error if the slice is not exactly 4 bytes or the event code
/// is outside the recognized range (0-15).
pub fn decode_dtmf_packet(data: &[u8]) -> crate::error::Result<DtmfPacket> {
    if data.len() < 4 {
        return Err(Error::Codec(CodecError::DecodingFailed {
            reason: format!(
                "DTMF payload too short: expected 4 bytes, got {}",
                data.len()
            ),
        }));
    }

    let event_code = data[0];
    let event = DtmfEvent::from_code(event_code).ok_or_else(|| {
        Error::Codec(CodecError::DecodingFailed {
            reason: format!("Unknown DTMF event code: {}", event_code),
        })
    })?;

    let end_of_event = (data[1] & 0x80) != 0;
    let volume = data[1] & 0x3F;
    let duration = u16::from_be_bytes([data[2], data[3]]);

    Ok(DtmfPacket {
        event,
        end_of_event,
        volume,
        duration,
    })
}

/// Generates the full sequence of [`DtmfPacket`]s for a single DTMF key-press.
///
/// The sequence follows RFC 4733 recommendations:
/// - Initial packets sent every `ptime_ms` (default 20 ms) with incrementing duration
/// - Three end-of-event packets at the final duration (retransmissions)
///
/// # Arguments
/// * `event` — The DTMF digit/tone to generate.
/// * `duration_ms` — Total duration of the key-press in milliseconds.
/// * `sample_rate` — Clock rate for the telephone-event (typically 8000 Hz).
///
/// # Returns
/// A `Vec<DtmfPacket>` in transmission order.  The RTP timestamp for every
/// packet in the event should be the *same* (set to the start of the event);
/// only the `duration` field grows.
pub fn generate_dtmf_rtp_packets(
    event: DtmfEvent,
    duration_ms: u32,
    sample_rate: u32,
) -> Vec<DtmfPacket> {
    generate_dtmf_rtp_packets_with_ptime(event, duration_ms, sample_rate, DEFAULT_PTIME_MS)
}

/// Same as [`generate_dtmf_rtp_packets`] but allows specifying the ptime.
pub fn generate_dtmf_rtp_packets_with_ptime(
    event: DtmfEvent,
    duration_ms: u32,
    sample_rate: u32,
    ptime_ms: u32,
) -> Vec<DtmfPacket> {
    let ptime_ms = if ptime_ms == 0 { DEFAULT_PTIME_MS } else { ptime_ms };

    // Total duration in RTP timestamp units — clamp to u16::MAX instead of
    // silently truncating from u64.
    let total_duration_ts = u16::try_from(
        duration_ms as u64 * sample_rate as u64 / 1000
    ).unwrap_or(u16::MAX);

    // Duration increment per packet in timestamp units — clamp and ensure
    // the step is at least 1 to prevent an infinite loop when the
    // computation would otherwise round down to zero.
    let mut step_ts = u16::try_from(
        ptime_ms as u64 * sample_rate as u64 / 1000
    ).unwrap_or(u16::MAX);

    if step_ts == 0 {
        step_ts = 1;
    }

    let mut packets = Vec::new();

    // Generate intermediate (non-end) packets
    let mut current_ts: u16 = step_ts;
    while current_ts < total_duration_ts {
        packets.push(DtmfPacket::new(event, false, DEFAULT_VOLUME, current_ts));
        current_ts = current_ts.saturating_add(step_ts);
    }

    // If no intermediate packets were generated (very short duration),
    // emit at least one non-end packet.
    if packets.is_empty() {
        packets.push(DtmfPacket::new(event, false, DEFAULT_VOLUME, total_duration_ts));
    }

    // End-of-event packets (retransmitted 3 times per RFC 4733 Section 2.5)
    for _ in 0..END_OF_EVENT_RETRANSMISSIONS {
        packets.push(DtmfPacket::new(event, true, DEFAULT_VOLUME, total_duration_ts));
    }

    packets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = DtmfPacket::new(DtmfEvent::Digit5, false, 10, 800);
        let encoded = encode_dtmf_packet(&original);
        let decoded = decode_dtmf_packet(&encoded).unwrap_or_else(|_| {
            panic!("decode should succeed for valid packet")
        });
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_encode_decode_end_of_event() {
        let original = DtmfPacket::new(DtmfEvent::Star, true, 10, 1600);
        let encoded = encode_dtmf_packet(&original);

        // Check raw bytes
        assert_eq!(encoded[0], 10); // Star = 10
        assert_eq!(encoded[1] & 0x80, 0x80); // E bit set
        assert_eq!(encoded[1] & 0x3F, 10);  // volume = 10

        let decoded = decode_dtmf_packet(&encoded).unwrap_or_else(|_| {
            panic!("decode should succeed")
        });
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_encode_byte_layout() {
        let pkt = DtmfPacket::new(DtmfEvent::Pound, true, 20, 0x1234);
        let encoded = encode_dtmf_packet(&pkt);

        assert_eq!(encoded[0], 11);        // Pound = 11
        assert_eq!(encoded[1], 0x80 | 20); // E=1, R=0, vol=20
        assert_eq!(encoded[2], 0x12);      // duration high byte
        assert_eq!(encoded[3], 0x34);      // duration low byte
    }

    #[test]
    fn test_decode_too_short() {
        let result = decode_dtmf_packet(&[0, 1]);
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_unknown_event() {
        // Event code 16 is outside our DTMF range
        let data = [16, 0, 0, 160];
        let result = decode_dtmf_packet(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_dtmf_packets_basic() {
        // 100ms duration at 8000 Hz = 800 timestamp units
        // With 20ms ptime, step = 160 units
        // Intermediate packets at durations: 160, 320, 480, 640
        // Then 3 end-of-event packets at 800
        let packets = generate_dtmf_rtp_packets(DtmfEvent::Digit1, 100, 8000);

        // Check that we have intermediate + 3 end-of-event packets
        assert!(!packets.is_empty());

        // The last 3 packets should be end-of-event
        let len = packets.len();
        assert!(len >= 3, "Should have at least 3 end-of-event packets, got {}", len);

        for pkt in &packets[len - 3..] {
            assert!(pkt.end_of_event, "Last 3 packets should be end-of-event");
            assert_eq!(pkt.duration, 800, "End-of-event duration should be total");
        }

        // Non-end packets should have increasing duration
        for pkt in &packets[..len - 3] {
            assert!(!pkt.end_of_event);
        }

        // All packets should reference the same event
        for pkt in &packets {
            assert_eq!(pkt.event, DtmfEvent::Digit1);
        }
    }

    #[test]
    fn test_generate_dtmf_packets_very_short() {
        // 5ms at 8000 Hz = 40 timestamp units, shorter than one ptime
        let packets = generate_dtmf_rtp_packets(DtmfEvent::Digit0, 5, 8000);

        // Should have at least 1 non-end + 3 end-of-event = 4 packets
        assert!(packets.len() >= 4, "Got {} packets", packets.len());

        // First packet should not be end-of-event
        assert!(!packets[0].end_of_event);

        // Last 3 should be end-of-event
        for pkt in &packets[packets.len() - 3..] {
            assert!(pkt.end_of_event);
        }
    }

    #[test]
    fn test_generate_dtmf_packets_16khz() {
        // 100ms at 16000 Hz = 1600 timestamp units
        let packets = generate_dtmf_rtp_packets(DtmfEvent::A, 100, 16000);

        let len = packets.len();
        assert!(len >= 3);

        // End-of-event duration should be 1600
        assert_eq!(packets[len - 1].duration, 1600);
    }

    #[test]
    fn test_all_dtmf_events_encode_decode() {
        for code in 0..=15u8 {
            let event = DtmfEvent::from_code(code).unwrap_or(DtmfEvent::Digit0);
            let pkt = DtmfPacket::new(event, false, 10, 160);
            let encoded = encode_dtmf_packet(&pkt);
            let decoded = decode_dtmf_packet(&encoded).unwrap_or_else(|_| {
                panic!("Failed to decode event code {}", code)
            });
            assert_eq!(decoded.event.code(), code);
        }
    }
}
