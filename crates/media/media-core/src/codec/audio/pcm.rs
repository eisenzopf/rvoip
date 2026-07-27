//! Raw signed 16-bit little-endian PCM.
//!
//! This codec exists for internal media-graph routing to transports that
//! exchange headerless PCM. It is not an RTP codec and must not be advertised
//! in SDP.

use super::common::{AudioCodec, CodecInfo};
use crate::error::{CodecError, Result};
use crate::types::AudioFrame;

/// Headerless, signed 16-bit little-endian PCM at 16 kHz, mono.
pub struct PcmS16LeCodec {
    sample_rate: u32,
    channels: u8,
}

impl PcmS16LeCodec {
    /// Construct the internal PCM codec.
    ///
    /// The media graph currently reserves this codec specifically for
    /// 16 kHz mono voice audio.
    pub fn new(sample_rate: u32, channels: u8) -> Result<Self> {
        if sample_rate != 16_000 || channels != 1 {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "pcm_s16le requires 16000 Hz mono, got {sample_rate} Hz/{channels} channels"
                ),
            }
            .into());
        }
        Ok(Self {
            sample_rate,
            channels,
        })
    }
}

impl Default for PcmS16LeCodec {
    fn default() -> Self {
        Self {
            sample_rate: 16_000,
            channels: 1,
        }
    }
}

impl AudioCodec for PcmS16LeCodec {
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        if audio_frame.sample_rate != self.sample_rate || audio_frame.channels != self.channels {
            return Err(CodecError::InvalidParameters {
                details: format!(
                    "pcm_s16le expected {} Hz/{} channel, got {} Hz/{} channels",
                    self.sample_rate, self.channels, audio_frame.sample_rate, audio_frame.channels
                ),
            }
            .into());
        }
        if audio_frame.samples.is_empty() {
            return Err(CodecError::EncodingFailed {
                reason: "pcm_s16le cannot encode an empty frame".into(),
            }
            .into());
        }

        let mut encoded = Vec::with_capacity(audio_frame.samples.len() * 2);
        for sample in &audio_frame.samples {
            encoded.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(encoded)
    }

    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        if encoded_data.is_empty() {
            return Err(CodecError::DecodingFailed {
                reason: "pcm_s16le cannot decode an empty frame".into(),
            }
            .into());
        }
        if encoded_data.len() % 2 != 0 {
            return Err(CodecError::InvalidFrameSize {
                expected: encoded_data.len() + 1,
                actual: encoded_data.len(),
            }
            .into());
        }

        let samples = encoded_data
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
            .collect();
        Ok(AudioFrame::new(samples, self.sample_rate, self.channels, 0))
    }

    fn get_info(&self) -> CodecInfo {
        CodecInfo {
            name: "pcm_s16le".into(),
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: self.sample_rate * u32::from(self.channels) * 16,
        }
    }

    fn reset(&mut self) {
        // Raw PCM is stateless.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_little_endian_samples() {
        let samples = vec![i16::MIN, -1, 0, 1, i16::MAX];
        let frame = AudioFrame::new(samples.clone(), 16_000, 1, 0);
        let mut codec = PcmS16LeCodec::default();

        let encoded = codec.encode(&frame).unwrap();
        assert_eq!(
            encoded,
            vec![0x00, 0x80, 0xff, 0xff, 0x00, 0x00, 0x01, 0x00, 0xff, 0x7f]
        );
        assert_eq!(codec.decode(&encoded).unwrap().samples, samples);
    }

    #[test]
    fn twenty_milliseconds_is_640_bytes() {
        let frame = AudioFrame::new(vec![0; 320], 16_000, 1, 0);
        let encoded = PcmS16LeCodec::default().encode(&frame).unwrap();
        assert_eq!(encoded.len(), 640);
    }

    #[test]
    fn rejects_odd_length_and_noncanonical_format() {
        let mut codec = PcmS16LeCodec::default();
        assert!(codec.decode(&[0]).is_err());
        assert!(codec
            .encode(&AudioFrame::new(vec![0; 160], 8_000, 1, 0))
            .is_err());
        assert!(PcmS16LeCodec::new(16_000, 2).is_err());
    }
}
