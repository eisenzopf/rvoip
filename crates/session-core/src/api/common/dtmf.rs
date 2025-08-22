//! DTMF handling utilities

use crate::api::types::SessionId;
use crate::api::control::SessionControl;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;
use std::sync::Arc;

/// DTMF tone definitions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DtmfTone {
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Star,
    Pound,
    A,
    B,
    C,
    D,
}

impl DtmfTone {
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
            'A' | 'a' => Some(Self::A),
            'B' | 'b' => Some(Self::B),
            'C' | 'c' => Some(Self::C),
            'D' | 'd' => Some(Self::D),
            _ => None,
        }
    }
}

/// Send a DTMF sequence
pub async fn send_dtmf_sequence(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    sequence: &str,
    inter_digit_delay_ms: u64,
) -> Result<()> {
    for c in sequence.chars() {
        if let Some(tone) = DtmfTone::from_char(c) {
            SessionControl::send_dtmf(coordinator, session_id, &tone.to_char().to_string()).await?;
            if inter_digit_delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(inter_digit_delay_ms)).await;
            }
        }
    }
    Ok(())
}