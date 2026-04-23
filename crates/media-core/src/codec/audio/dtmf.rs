//! RFC 4733 telephone-event (DTMF) codec.
//!
//! Encodes and decodes the 4-byte RFC 4733 §2.3 event payload that rides on
//! RTP PT 101. This is the standard mechanism for carrying DTMF tones out-
//! of-band from the audio stream; every cloud carrier and every modern PBX
//! expects it.
//!
//! ## Wire format (RFC 4733 §2.3)
//!
//! ```text
//!  0                   1                   2                   3
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |     event     |E|R| volume    |          duration             |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```
//!
//! - `event` (8 bits) — DTMF event code 0-15 (digits 0-9, `*`, `#`, A-D).
//! - `E` (1 bit) — End-of-event marker. Set on the final retransmission.
//! - `R` (1 bit) — Reserved, MUST be zero (RFC 4733 §2.3).
//! - `volume` (6 bits) — Power level, expressed as negative dBm0
//!   (0 = 0 dBm0, 63 = -63 dBm0). Typical carrier value: 10.
//! - `duration` (16 bits) — Event duration in timestamp units (samples at
//!   the codec clock-rate, i.e. 1 ms at 8 kHz → 8 samples).
//!
//! Packets are sent at the normal RTP packetization interval (typically
//! 20 ms). Every packet in a tone burst carries the *same* RTP timestamp
//! (the start of the event); the `duration` field grows until the tone
//! ends, then an end packet is sent three times with `E=1`.
//!
//! This module implements the pure encode/decode. Transmitter/receiver
//! wiring into `rtp-core` and the session-core `send_dtmf` / `on_dtmf`
//! event bus is tracked as a follow-up.

use serde::{Deserialize, Serialize};

/// RFC 4733 DTMF event codes.
///
/// Events 0–15 are the IANA-registered subset used for DTMF. Higher event
/// numbers (16+) encode other tones (dial, ringback, busy, etc.) and are
/// out of scope for this codec — those are DTMF *proper* rather than
/// signalling tones, and carriers rarely care about them on the RTP path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DtmfEvent(pub u8);

impl DtmfEvent {
    /// Map a digit character to its RFC 4733 event code.
    ///
    /// Accepts `'0'`-`'9'`, `'*'`, `'#'`, and `'A'`-`'D'` (case-insensitive).
    /// Returns `None` for any other character.
    pub fn from_digit(c: char) -> Option<Self> {
        let code = match c {
            '0'..='9' => c as u8 - b'0',
            '*' => 10,
            '#' => 11,
            'A' | 'a' => 12,
            'B' | 'b' => 13,
            'C' | 'c' => 14,
            'D' | 'd' => 15,
            _ => return None,
        };
        Some(DtmfEvent(code))
    }

    /// The human-readable digit character for this event code. Returns `?`
    /// for out-of-DTMF-range codes (16-255), which never come out of
    /// `from_digit` and should only arise if a malformed packet is decoded.
    pub fn to_digit(self) -> char {
        match self.0 {
            0..=9 => (b'0' + self.0) as char,
            10 => '*',
            11 => '#',
            12 => 'A',
            13 => 'B',
            14 => 'C',
            15 => 'D',
            _ => '?',
        }
    }
}

/// Decoded RFC 4733 telephone-event packet body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelephoneEvent {
    /// Event code. For DTMF, one of 0-15; see [`DtmfEvent::to_digit`].
    pub event: u8,
    /// End-of-event marker. Set on the final retransmission of the tone
    /// boundary per RFC 4733 §2.5.1.3 (typically re-transmitted 3× for
    /// packet-loss resilience).
    pub end_of_event: bool,
    /// Volume as a positive integer ("-N dBm0") in 0..=63. Higher = quieter.
    pub volume: u8,
    /// Event duration in timestamp units (samples at the codec clock).
    pub duration: u16,
}

impl TelephoneEvent {
    pub fn new_digit(digit: char, duration_samples: u16, volume: u8, end: bool) -> Option<Self> {
        let event = DtmfEvent::from_digit(digit)?.0;
        Some(Self {
            event,
            end_of_event: end,
            volume: volume.min(63),
            duration: duration_samples,
        })
    }

    /// Encode to the 4-byte RFC 4733 §2.3 wire payload.
    pub fn encode(&self) -> [u8; 4] {
        let e_bit = if self.end_of_event { 0b1000_0000u8 } else { 0 };
        let r_bit = 0u8; // MUST be zero per RFC 4733 §2.3.
        let vol = self.volume & 0b0011_1111;
        let byte1 = e_bit | r_bit | vol;

        let duration_be = self.duration.to_be_bytes();
        [self.event, byte1, duration_be[0], duration_be[1]]
    }

    /// Decode from a 4-byte RFC 4733 wire payload. Returns `None` if the
    /// slice is shorter than 4 bytes (oversized payloads are tolerated — we
    /// read only the first four bytes, per RFC 4733's forward-compatibility
    /// clause).
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }
        let event = bytes[0];
        let byte1 = bytes[1];
        let end_of_event = (byte1 & 0b1000_0000) != 0;
        // `R` bit is ignored on receive; spec mandates zero but we don't
        // reject non-conforming peers.
        let volume = byte1 & 0b0011_1111;
        let duration = u16::from_be_bytes([bytes[2], bytes[3]]);
        Some(Self {
            event,
            end_of_event,
            volume,
            duration,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_roundtrip_covers_all_dtmf_codes() {
        for (ch, expected) in &[
            ('0', 0u8),
            ('1', 1),
            ('5', 5),
            ('9', 9),
            ('*', 10),
            ('#', 11),
            ('A', 12),
            ('B', 13),
            ('C', 14),
            ('D', 15),
        ] {
            let ev = DtmfEvent::from_digit(*ch).unwrap();
            assert_eq!(ev.0, *expected);
            assert_eq!(ev.to_digit(), *ch);
        }
    }

    #[test]
    fn lowercase_abcd_is_accepted() {
        for (lc, uc) in [('a', 'A'), ('b', 'B'), ('c', 'C'), ('d', 'D')] {
            assert_eq!(
                DtmfEvent::from_digit(lc).unwrap(),
                DtmfEvent::from_digit(uc).unwrap()
            );
        }
    }

    #[test]
    fn non_dtmf_characters_rejected() {
        for ch in ['x', 'y', ' ', '+', '-', 'E', 'e', 'F', 'f'] {
            assert!(DtmfEvent::from_digit(ch).is_none(), "unexpected accept of {:?}", ch);
        }
    }

    #[test]
    fn encode_matches_rfc_example_digit_5() {
        // Digit '5', not end-of-event, volume 10 (-10 dBm0), duration
        // 400 samples (50 ms at 8 kHz).
        let ev = TelephoneEvent::new_digit('5', 400, 10, false).unwrap();
        let bytes = ev.encode();
        assert_eq!(bytes[0], 5); // event code
        assert_eq!(bytes[1] & 0b1000_0000, 0); // E=0
        assert_eq!(bytes[1] & 0b0100_0000, 0); // R=0
        assert_eq!(bytes[1] & 0b0011_1111, 10); // volume=10
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 400);
    }

    #[test]
    fn encode_sets_e_bit_on_end() {
        let ev = TelephoneEvent::new_digit('3', 800, 10, true).unwrap();
        let bytes = ev.encode();
        assert_eq!(bytes[1] & 0b1000_0000, 0b1000_0000);
    }

    #[test]
    fn decode_rejects_short_payload() {
        assert!(TelephoneEvent::decode(&[]).is_none());
        assert!(TelephoneEvent::decode(&[1, 2, 3]).is_none());
    }

    #[test]
    fn decode_tolerates_oversized_payload() {
        // Spec allows extensions beyond 4 bytes — we read the first four
        // and ignore the rest.
        let bytes = [5u8, 0b0000_1010, 0x01, 0x90, 0xFF, 0xFF, 0xFF];
        let ev = TelephoneEvent::decode(&bytes).unwrap();
        assert_eq!(ev.event, 5);
        assert_eq!(ev.end_of_event, false);
        assert_eq!(ev.volume, 10);
        assert_eq!(ev.duration, 0x0190);
    }

    #[test]
    fn encode_decode_roundtrip_every_digit() {
        for digit in "0123456789*#ABCD".chars() {
            let ev = TelephoneEvent::new_digit(digit, 320, 12, true).unwrap();
            let bytes = ev.encode();
            let decoded = TelephoneEvent::decode(&bytes).unwrap();
            assert_eq!(decoded, ev, "roundtrip failed for digit {:?}", digit);
            assert_eq!(DtmfEvent(decoded.event).to_digit(), digit);
        }
    }

    #[test]
    fn decode_ignores_reserved_bit() {
        // R bit set — must not affect the decode; E and volume still read.
        let bytes = [9u8, 0b1100_0001, 0x00, 0x10];
        let ev = TelephoneEvent::decode(&bytes).unwrap();
        assert_eq!(ev.event, 9);
        assert!(ev.end_of_event);
        assert_eq!(ev.volume, 1);
        assert_eq!(ev.duration, 16);
    }

    #[test]
    fn volume_saturates_at_63() {
        let ev = TelephoneEvent::new_digit('1', 160, 200, false).unwrap();
        assert_eq!(ev.volume, 63);
    }
}
