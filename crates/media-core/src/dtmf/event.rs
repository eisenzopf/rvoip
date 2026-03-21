//! RFC 4733 DTMF event types.
//!
//! Defines the telephone-event payload types used for transmitting DTMF
//! digits over RTP per RFC 4733 (superseding RFC 2833).

use std::fmt;

/// DTMF event codes as defined in RFC 4733 Section 3.
///
/// The event codes 0-15 correspond to the standard DTMF digit set
/// (0-9, *, #, A-D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DtmfEvent {
    /// DTMF digit 0
    Digit0 = 0,
    /// DTMF digit 1
    Digit1 = 1,
    /// DTMF digit 2
    Digit2 = 2,
    /// DTMF digit 3
    Digit3 = 3,
    /// DTMF digit 4
    Digit4 = 4,
    /// DTMF digit 5
    Digit5 = 5,
    /// DTMF digit 6
    Digit6 = 6,
    /// DTMF digit 7
    Digit7 = 7,
    /// DTMF digit 8
    Digit8 = 8,
    /// DTMF digit 9
    Digit9 = 9,
    /// DTMF star (*)
    Star = 10,
    /// DTMF pound (#)
    Pound = 11,
    /// DTMF letter A
    A = 12,
    /// DTMF letter B
    B = 13,
    /// DTMF letter C
    C = 14,
    /// DTMF letter D
    D = 15,
}

impl DtmfEvent {
    /// Creates a DtmfEvent from a raw event code.
    ///
    /// Returns `None` if the code is outside the valid range (0-15).
    pub fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Digit0),
            1 => Some(Self::Digit1),
            2 => Some(Self::Digit2),
            3 => Some(Self::Digit3),
            4 => Some(Self::Digit4),
            5 => Some(Self::Digit5),
            6 => Some(Self::Digit6),
            7 => Some(Self::Digit7),
            8 => Some(Self::Digit8),
            9 => Some(Self::Digit9),
            10 => Some(Self::Star),
            11 => Some(Self::Pound),
            12 => Some(Self::A),
            13 => Some(Self::B),
            14 => Some(Self::C),
            15 => Some(Self::D),
            _ => None,
        }
    }

    /// Creates a DtmfEvent from a character representation.
    ///
    /// Accepts '0'-'9', '*', '#', 'a'-'d', 'A'-'D'.
    /// Returns `None` for unrecognized characters.
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            '0' => Some(Self::Digit0),
            '1' => Some(Self::Digit1),
            '2' => Some(Self::Digit2),
            '3' => Some(Self::Digit3),
            '4' => Some(Self::Digit4),
            '5' => Some(Self::Digit5),
            '6' => Some(Self::Digit6),
            '7' => Some(Self::Digit7),
            '8' => Some(Self::Digit8),
            '9' => Some(Self::Digit9),
            '*' => Some(Self::Star),
            '#' => Some(Self::Pound),
            'a' | 'A' => Some(Self::A),
            'b' | 'B' => Some(Self::B),
            'c' | 'C' => Some(Self::C),
            'd' | 'D' => Some(Self::D),
            _ => None,
        }
    }

    /// Returns the raw event code (0-15).
    pub fn code(self) -> u8 {
        self as u8
    }

    /// Returns the character representation of the event.
    pub fn to_char(self) -> char {
        match self {
            Self::Digit0 => '0',
            Self::Digit1 => '1',
            Self::Digit2 => '2',
            Self::Digit3 => '3',
            Self::Digit4 => '4',
            Self::Digit5 => '5',
            Self::Digit6 => '6',
            Self::Digit7 => '7',
            Self::Digit8 => '8',
            Self::Digit9 => '9',
            Self::Star => '*',
            Self::Pound => '#',
            Self::A => 'A',
            Self::B => 'B',
            Self::C => 'C',
            Self::D => 'D',
        }
    }
}

impl fmt::Display for DtmfEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_char())
    }
}

/// A single RFC 4733 DTMF RTP payload packet.
///
/// The 4-byte payload format is:
/// ```text
///  0                   1                   2                   3
///  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// |     event     |E|R| volume    |          duration             |
/// +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DtmfPacket {
    /// The DTMF event (digit, *, #, A-D).
    pub event: DtmfEvent,
    /// End-of-event flag. When `true`, this is the final packet for the event.
    pub end_of_event: bool,
    /// Power level of the tone in -dBm0 (0-63). Lower means louder.
    /// Typical value: 10 (-10 dBm0).
    pub volume: u8,
    /// Duration of the event so far, in RTP timestamp units.
    /// For 8 kHz, each unit is 125 microseconds.
    pub duration: u16,
}

impl DtmfPacket {
    /// Creates a new DtmfPacket.
    ///
    /// Volume is clamped to the valid range 0-63.
    pub fn new(event: DtmfEvent, end_of_event: bool, volume: u8, duration: u16) -> Self {
        Self {
            event,
            end_of_event,
            volume: volume.min(63),
            duration,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dtmf_event_from_code() {
        assert_eq!(DtmfEvent::from_code(0), Some(DtmfEvent::Digit0));
        assert_eq!(DtmfEvent::from_code(9), Some(DtmfEvent::Digit9));
        assert_eq!(DtmfEvent::from_code(10), Some(DtmfEvent::Star));
        assert_eq!(DtmfEvent::from_code(11), Some(DtmfEvent::Pound));
        assert_eq!(DtmfEvent::from_code(15), Some(DtmfEvent::D));
        assert_eq!(DtmfEvent::from_code(16), None);
        assert_eq!(DtmfEvent::from_code(255), None);
    }

    #[test]
    fn test_dtmf_event_from_char() {
        assert_eq!(DtmfEvent::from_char('0'), Some(DtmfEvent::Digit0));
        assert_eq!(DtmfEvent::from_char('*'), Some(DtmfEvent::Star));
        assert_eq!(DtmfEvent::from_char('#'), Some(DtmfEvent::Pound));
        assert_eq!(DtmfEvent::from_char('A'), Some(DtmfEvent::A));
        assert_eq!(DtmfEvent::from_char('a'), Some(DtmfEvent::A));
        assert_eq!(DtmfEvent::from_char('x'), None);
    }

    #[test]
    fn test_dtmf_event_to_char() {
        assert_eq!(DtmfEvent::Digit5.to_char(), '5');
        assert_eq!(DtmfEvent::Star.to_char(), '*');
        assert_eq!(DtmfEvent::Pound.to_char(), '#');
        assert_eq!(DtmfEvent::A.to_char(), 'A');
    }

    #[test]
    fn test_dtmf_event_roundtrip() {
        for code in 0..=15u8 {
            let event = DtmfEvent::from_code(code);
            assert!(event.is_some());
            let event = event.unwrap_or(DtmfEvent::Digit0);
            assert_eq!(event.code(), code);
        }
    }

    #[test]
    fn test_dtmf_packet_volume_clamp() {
        let pkt = DtmfPacket::new(DtmfEvent::Digit1, false, 100, 160);
        assert_eq!(pkt.volume, 63);

        let pkt = DtmfPacket::new(DtmfEvent::Digit1, false, 10, 160);
        assert_eq!(pkt.volume, 10);
    }

    #[test]
    fn test_dtmf_event_display() {
        assert_eq!(format!("{}", DtmfEvent::Digit0), "0");
        assert_eq!(format!("{}", DtmfEvent::Star), "*");
        assert_eq!(format!("{}", DtmfEvent::Pound), "#");
    }
}
