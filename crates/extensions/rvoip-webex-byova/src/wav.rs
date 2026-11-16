//! Bounded PCMU/8 kHz to mono PCM16 LE/8 kHz RIFF conversion.
//! WebSocket use is an explicit sandbox compatibility exception, not the guide profile.
use crate::ProtocolError;

/// Resource bound for whole replies: 60 seconds at 8 kHz, never truncate/split.
pub const MAX_WAV_SAMPLES: usize = 480_000;

pub fn pcmu_to_pcm16_wav(audio: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    if audio.is_empty() || audio.len() > MAX_WAV_SAMPLES {
        return Err(ProtocolError::Limit);
    }
    let len = (audio.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    for n in [1u16, 1] {
        wav.extend_from_slice(&n.to_le_bytes());
    }
    for n in [8000u32, 16000] {
        wav.extend_from_slice(&n.to_le_bytes());
    }
    for n in [2u16, 16] {
        wav.extend_from_slice(&n.to_le_bytes());
    }
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&len.to_le_bytes());
    for &byte in audio {
        wav.extend_from_slice(&codec_core::codecs::g711::ulaw_expand(byte).to_le_bytes());
    }
    Ok(wav)
}
