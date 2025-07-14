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

/// Convert linear PCM sample to μ-law
fn linear_to_mu_law(sample: i16) -> u8 {
    const BIAS: i16 = 0x84;
    const CLIP: i16 = 32635;
    
    // Get sign and make sample positive
    let sign = if sample < 0 { 0x7F } else { 0xFF };
    let mut sample = if sample < 0 { 
        if sample == i16::MIN { i16::MAX } else { -sample }
    } else { 
        sample 
    };
    
    // Apply bias and clipping
    sample = sample.saturating_add(BIAS);
    if sample > CLIP {
        sample = CLIP;
    }
    
    // Find the exponent
    let exp = if sample < 256 {
        0
    } else if sample < 512 {
        1
    } else if sample < 1024 {
        2
    } else if sample < 2048 {
        3
    } else if sample < 4096 {
        4
    } else if sample < 8192 {
        5
    } else if sample < 16384 {
        6
    } else {
        7
    };
    
    // Calculate mantissa
    let mantissa = (sample >> (exp + 3)) & 0x0F;
    
    // Combine to form μ-law byte
    let mu_law = (exp << 4) | mantissa;
    
    // Apply complement and sign
    ((!mu_law) & sign) as u8
}

/// Convert μ-law to linear PCM sample
fn mu_law_to_linear(mu_law: u8) -> i16 {
    const BIAS: i16 = 0x84;
    
    // Get sign and invert the μ-law byte
    let sign = (mu_law & 0x80) == 0;
    let mu_law = !mu_law;
    
    // Extract exponent and mantissa
    let exp = (mu_law >> 4) & 0x07;
    let mantissa = mu_law & 0x0F;
    
    // Calculate linear value
    let linear = ((mantissa as i16) << 3) + BIAS;
    let linear = if exp == 0 {
        linear
    } else {
        (linear << exp) - BIAS
    };
    
    // Apply sign
    if sign {
        -linear
    } else {
        linear
    }
}

/// Convert linear PCM sample to A-law
fn linear_to_a_law(sample: i16) -> u8 {
    // Get sign and make sample positive
    let sign = if sample >= 0 { 0x80 } else { 0x00 };
    let mut sample = if sample >= 0 { 
        sample 
    } else { 
        if sample == i16::MIN { i16::MAX } else { -sample }
    };
    
    // Find the exponent
    let exp = if sample < 16 {
        0
    } else if sample < 32 {
        1
    } else if sample < 64 {
        2
    } else if sample < 128 {
        3
    } else if sample < 256 {
        4
    } else if sample < 512 {
        5
    } else if sample < 1024 {
        6
    } else if sample < 2048 {
        7
    } else {
        sample >>= 4; // Scale down for higher values
        if sample < 16 {
            4
        } else if sample < 32 {
            5
        } else if sample < 64 {
            6
        } else {
            7
        }
    };
    
    // Calculate mantissa
    let mantissa = if exp < 4 {
        (sample >> (exp + 1)) & 0x0F
    } else {
        (sample >> (exp - 3)) & 0x0F
    };
    
    // Combine sign, exponent, and mantissa
    let a_law = sign | (exp << 4) | mantissa;
    
    // Apply A-law XOR mask
    (a_law ^ 0x55) as u8
}

/// Convert A-law to linear PCM sample
fn a_law_to_linear(a_law: u8) -> i16 {
    // Remove A-law XOR mask
    let a_law = a_law ^ 0x55;
    
    // Extract sign, exponent, and mantissa
    let sign = (a_law & 0x80) != 0;
    let exp = (a_law >> 4) & 0x07;
    let mantissa = a_law & 0x0F;
    
    // Calculate linear value
    let linear = if exp < 4 {
        (mantissa << (exp + 1)) | (1 << exp)
    } else {
        ((mantissa << (exp - 3)) | (1 << (exp - 4))) << 4
    };
    
    // Apply sign
    if sign {
        linear as i16
    } else {
        -(linear as i16)
    }
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
        
        // Test with sine wave samples
        let samples = vec![0, 16384, 32767, -16384, -32767];
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
            assert!(error < 1000, "Error too large: {} vs {}", original, decoded);
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
        
        // Test with sine wave samples
        let samples = vec![0, 16384, 32767, -16384, -32767];
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
            assert!(error < 1000, "Error too large: {} vs {}", original, decoded);
        }
    }

    #[test]
    fn test_mu_law_conversion_functions() {
        let test_samples = vec![0, 8192, 16384, 24576, 32767, -8192, -16384, -24576, -32767];
        
        for sample in test_samples {
            let encoded = linear_to_mu_law(sample);
            let decoded = mu_law_to_linear(encoded);
            let error = (sample - decoded).abs();
            assert!(error < 1000, "μ-law conversion error: {} -> {} -> {}", sample, encoded, decoded);
        }
    }

    #[test]
    fn test_a_law_conversion_functions() {
        let test_samples = vec![0, 8192, 16384, 24576, 32767, -8192, -16384, -24576, -32767];
        
        for sample in test_samples {
            let encoded = linear_to_a_law(sample);
            let decoded = a_law_to_linear(encoded);
            let error = (sample - decoded).abs();
            assert!(error < 1000, "A-law conversion error: {} -> {} -> {}", sample, encoded, decoded);
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