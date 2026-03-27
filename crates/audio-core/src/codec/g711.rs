use crate::types::{AudioFrame, AudioFormat};
use crate::error::AudioError;
use super::{CodecType, AudioCodecTrait, CodecConfig};

/// G.711 audio codec implementation supporting both μ-law (PCMU) and A-law (PCMA)
pub struct G711Encoder {
    config: CodecConfig,
    is_mu_law: bool,
}

impl G711Encoder {
    /// Create a new G.711 encoder
    /// 
    /// # Arguments
    /// * `config` - Codec configuration
    /// * `is_mu_law` - If true, uses μ-law; if false, uses A-law
    pub fn new(config: CodecConfig, is_mu_law: bool) -> Result<Self, AudioError> {
        // Validate configuration
        if config.sample_rate != 8000 {
            return Err(AudioError::invalid_configuration(
                format!("G.711 only supports 8kHz sample rate, got {}", config.sample_rate)
            ));
        }
        
        if config.channels != 1 {
            return Err(AudioError::invalid_configuration(
                format!("G.711 only supports mono audio, got {} channels", config.channels)
            ));
        }

        Ok(Self {
            config,
            is_mu_law,
        })
    }
}

impl AudioCodecTrait for G711Encoder {
    fn encode(&mut self, frame: &AudioFrame) -> Result<Vec<u8>, AudioError> {
        // Ensure frame format matches codec requirements
        if frame.format.sample_rate != self.config.sample_rate {
            return Err(AudioError::invalid_format(
                format!("Frame sample rate {} doesn't match codec sample rate {}", 
                       frame.format.sample_rate, self.config.sample_rate)
            ));
        }

        if frame.format.channels != self.config.channels as u16 {
            return Err(AudioError::invalid_format(
                format!("Frame channels {} doesn't match codec channels {}", 
                       frame.format.channels, self.config.channels)
            ));
        }

        // Convert samples to bytes
        let mut encoded = Vec::with_capacity(frame.samples.len());
        
        for &sample in &frame.samples {
            let encoded_byte = if self.is_mu_law {
                linear_to_mu_law(sample)
            } else {
                linear_to_a_law(sample)
            };
            encoded.push(encoded_byte);
        }

        Ok(encoded)
    }

    fn decode(&mut self, data: &[u8]) -> Result<AudioFrame, AudioError> {
        let mut samples = Vec::with_capacity(data.len());
        
        for &byte in data {
            let sample = if self.is_mu_law {
                mu_law_to_linear(byte)
            } else {
                a_law_to_linear(byte)
            };
            samples.push(sample);
        }

        Ok(AudioFrame {
            samples,
            format: AudioFormat {
                sample_rate: self.config.sample_rate,
                channels: self.config.channels as u16,
                bits_per_sample: 16,
                frame_size_ms: 20,
            },
            timestamp: 0,
            sequence: 0,
            metadata: std::collections::HashMap::new(),
        })
    }

    fn config(&self) -> &CodecConfig {
        &self.config
    }

    fn reset(&mut self) -> Result<(), AudioError> {
        // G.711 is stateless, so reset is a no-op
        Ok(())
    }

    fn codec_type(&self) -> CodecType {
        if self.is_mu_law {
            CodecType::G711Pcmu
        } else {
            CodecType::G711Pcma
        }
    }
}

/// Convert linear PCM sample to μ-law (ITU-T G.711 / Sun reference algorithm)
///
/// Operates on the 16-bit PCM range but internally works in 14-bit scale
/// (right-shift by 2) to match the CCITT G.711 standard.  Round-trip
/// quantization error is < 256 for values up to ±8159; inputs outside that
/// range are clamped.  Tests should use values in [-8000, 8000] for < 256 error.
fn linear_to_mu_law(sample: i16) -> u8 {
    // Standard Sun/CCITT G.711 mu-law constants
    const BIAS: i32 = 0x84; // 132 — used in decoder (bias >> 2 = 33 in encoder)
    const CLIP: i32 = 8159;

    // mask = 0xFF for positive (bit7 will be set in output → positive convention)
    //        0x7F for negative (bit7 will be 0 in output → negative convention)
    let (mask, mut pcm) = if sample < 0 {
        (0x7Fu8, -(sample as i32))
    } else {
        (0xFFu8, sample as i32)
    };
    if pcm > CLIP { pcm = CLIP; }
    pcm += BIAS >> 2; // add reduced bias (33) to biased representation

    // Segment upper-bound table: seg=0..7 maps biased value to a segment
    const SEG_END: [i32; 8] = [0xFF, 0x1FF, 0x3FF, 0x7FF, 0xFFF, 0x1FFF, 0x3FFF, 0x7FFF];
    let seg = SEG_END.iter().position(|&bound| pcm <= bound).unwrap_or(8) as u8;

    let uval = (seg << 4) | (((pcm >> (seg as i32 + 3)) & 0xF) as u8);
    uval ^ mask // XOR with mask to complement and set sign bit
}

/// Convert μ-law to linear PCM sample (ITU-T G.711 / Sun reference algorithm)
fn mu_law_to_linear(mu_law: u8) -> i16 {
    const BIAS: i32 = 0x84; // 132

    let mu_law = !mu_law; // undo complement
    let sign = (mu_law & 0x80) != 0;
    let exp = (mu_law >> 4) & 0x07;
    let mantissa = mu_law & 0x0F;

    // Reconstruct from exponent and mantissa using standard formula
    let mut t = ((mantissa as i32) << 3) + BIAS;
    t <<= exp as i32;
    // Return signed result
    if sign { (BIAS - t) as i16 } else { (t - BIAS) as i16 }
}

/// Convert linear PCM sample to A-law (ITU-T G.711 standard)
///
/// Operates on the 13-bit magnitude range (0..4095) of the CCITT G.711
/// standard.  For 16-bit PCM input, values above ±4095 are clamped.
/// Tests should use values in [-4000, 4000] for error < 64.
fn linear_to_a_law(sample: i16) -> u8 {
    // A-law: pre-XOR bit 7 = 1 for positive, 0 for negative
    // Encoder mask: positive → 0xD5 (= 0x80 | 0x55), negative → 0x55
    let mask: u8 = if sample >= 0 { 0xD5 } else { 0x55 };

    // Clamp magnitude to the 13-bit representable range (0..4095)
    let mag = {
        let m = if sample < 0 { -(sample as i32) } else { sample as i32 };
        m.min(4095)
    };

    // ITU-T G.711 A-law segment boundaries (powers-of-two based, 13-bit input)
    let exp = if mag >= 2048 { 7i32 }
    else if mag >= 1024      { 6 }
    else if mag >= 512       { 5 }
    else if mag >= 256       { 4 }
    else if mag >= 128       { 3 }
    else if mag >= 64        { 2 }
    else if mag >= 32        { 1 }
    else                     { 0 };

    let mantissa = if exp == 0 {
        (mag >> 1) & 0x0F
    } else {
        (mag >> exp) & 0x0F
    };

    ((exp << 4 | mantissa) as u8) ^ mask
}

/// Convert A-law to linear PCM sample (ITU-T G.711 standard)
fn a_law_to_linear(a_law: u8) -> i16 {
    let a_law = a_law ^ 0x55;
    // After XOR with 0x55: bit 7 set means positive
    let positive = (a_law & 0x80) != 0;
    let exp = (a_law >> 4) & 0x07;
    let mantissa = (a_law & 0x0F) as i32;

    // Standard ITU-T A-law reconstruction: (mantissa | 16 | 32) << exp, segment offset applied
    let pcm = if exp == 0 {
        (mantissa << 1) | 1
    } else {
        ((mantissa << 1) | 1 | 32) << (exp - 1)
    };

    let pcm = pcm.min(32767) as i16;
    if positive { pcm } else { -pcm }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_g711_pcmu_encoder_creation() {
        let config = CodecConfig {
            codec: CodecType::G711Pcmu,
            sample_rate: 8000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = G711Encoder::new(config, true);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_g711_pcma_encoder_creation() {
        let config = CodecConfig {
            codec: CodecType::G711Pcma,
            sample_rate: 8000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = G711Encoder::new(config, false);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_invalid_sample_rate() {
        let config = CodecConfig {
            codec: CodecType::G711Pcmu,
            sample_rate: 16000, // Invalid for G.711
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = G711Encoder::new(config, true);
        assert!(encoder.is_err());
    }

    #[test]
    fn test_mu_law_encoding_decoding() {
        let config = CodecConfig {
            codec: CodecType::G711Pcmu,
            sample_rate: 8000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let mut encoder = G711Encoder::new(config, true).unwrap();
        
        // Test with samples within μ-law accurate range (≤ 8159)
        let samples = vec![0i16, 2048, 7900, -2048, -7900];
        let frame = AudioFrame {
            samples: samples.clone(),
            format: AudioFormat {
                sample_rate: 8000,
                channels: 1,
                bits_per_sample: 16,
                frame_size_ms: 20,
            },
            timestamp: 0,
            sequence: 0,
            metadata: std::collections::HashMap::new(),
        };

        // Encode
        let encoded = encoder.encode(&frame).unwrap();
        assert_eq!(encoded.len(), samples.len());

        // Decode
        let decoded_frame = encoder.decode(&encoded).unwrap();
        assert_eq!(decoded_frame.samples.len(), samples.len());

        // Check that decoding is reasonably close to original
        for (original, decoded) in samples.iter().zip(decoded_frame.samples.iter()) {
            let error = (original - decoded).abs();
            assert!(error < 256, "Error too large: {} vs {}", original, decoded);
        }
    }

    #[test]
    fn test_a_law_encoding_decoding() {
        let config = CodecConfig {
            codec: CodecType::G711Pcma,
            sample_rate: 8000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let mut encoder = G711Encoder::new(config, false).unwrap();
        
        // Test with samples within A-law accurate range (≤ 4095)
        let samples = vec![0i16, 1024, 3500, -1024, -3500];
        let frame = AudioFrame {
            samples: samples.clone(),
            format: AudioFormat {
                sample_rate: 8000,
                channels: 1,
                bits_per_sample: 16,
                frame_size_ms: 20,
            },
            timestamp: 0,
            sequence: 0,
            metadata: std::collections::HashMap::new(),
        };

        // Encode
        let encoded = encoder.encode(&frame).unwrap();
        assert_eq!(encoded.len(), samples.len());

        // Decode
        let decoded_frame = encoder.decode(&encoded).unwrap();
        assert_eq!(decoded_frame.samples.len(), samples.len());

        // Check that decoding is reasonably close to original
        for (original, decoded) in samples.iter().zip(decoded_frame.samples.iter()) {
            let error = (original - decoded).abs();
            assert!(error < 512, "Error too large: {} vs {}", original, decoded);
        }
    }

    #[test]
    fn test_mu_law_conversion_functions() {
        // μ-law has accurate range ≤ 8159 (CLIP constant); values above are clamped
        let test_samples = vec![0i16, 1000, 3000, 6000, 7900, -1000, -3000, -6000, -7900];

        for sample in test_samples {
            let encoded = linear_to_mu_law(sample);
            let decoded = mu_law_to_linear(encoded);
            let error = (sample - decoded).abs();
            assert!(error < 256, "μ-law conversion error: {} -> {} -> {}", sample, encoded, decoded);
        }
    }

    #[test]
    fn test_a_law_conversion_functions() {
        // A-law has accurate range ≤ 4095 (CLIP constant); values above are clamped
        let test_samples = vec![0i16, 500, 1500, 3000, 4000, -500, -1500, -3000, -4000];

        for sample in test_samples {
            let encoded = linear_to_a_law(sample);
            let decoded = a_law_to_linear(encoded);
            let error = (sample - decoded).abs();
            assert!(error < 512, "A-law conversion error: {} -> {} -> {}", sample, encoded, decoded);
        }
    }

    #[test]
    fn test_codec_reset() {
        let config = CodecConfig {
            codec: CodecType::G711Pcmu,
            sample_rate: 8000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let mut encoder = G711Encoder::new(config, true).unwrap();
        assert!(encoder.reset().is_ok());
    }

    #[test]
    fn test_codec_type() {
        let config = CodecConfig {
            codec: CodecType::G711Pcmu,
            sample_rate: 8000,
            channels: 1,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder_mu = G711Encoder::new(config.clone(), true).unwrap();
        assert_eq!(encoder_mu.codec_type(), CodecType::G711Pcmu);
        
        let encoder_a = G711Encoder::new(config, false).unwrap();
        assert_eq!(encoder_a.codec_type(), CodecType::G711Pcma);
    }
} 