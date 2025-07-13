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
    // Use i16 sample directly
    let pcm = sample;
    
    // Convert to μ-law
    let mask = if pcm < 0 { 0x7F } else { 0xFF };
    let pcm = if pcm < 0 { (-pcm) as u16 } else { pcm as u16 };
    let pcm = pcm + 33;
    
    let exp = if pcm > 0x1FFF {
        7
    } else if pcm > 0x0FFF {
        6
    } else if pcm > 0x07FF {
        5
    } else if pcm > 0x03FF {
        4
    } else if pcm > 0x01FF {
        3
    } else if pcm > 0x00FF {
        2
    } else if pcm > 0x007F {
        1
    } else {
        0
    };
    
    let mantissa = (pcm >> (exp + 3)) & 0x0F;
    let mu_law = !((exp << 4) | mantissa);
    
    (mu_law & mask) as u8
}

/// Convert μ-law to linear PCM sample
fn mu_law_to_linear(mu_law: u8) -> i16 {
    let mu_law = !mu_law;
    let sign = (mu_law & 0x80) != 0;
    let exp = (mu_law >> 4) & 0x07;
    let mantissa = mu_law & 0x0F;
    
    let mut pcm = ((mantissa << 3) + 0x84) << exp;
    pcm -= 0x84;
    
    if sign {
        -(pcm as i16)
    } else {
        pcm as i16
    }
}

/// Convert linear PCM sample to A-law
fn linear_to_a_law(sample: i16) -> u8 {
    // Use i16 sample directly
    let mut pcm = sample;
    
    let mask = if pcm >= 0 { 0xD5 } else { 0x55 };
    if pcm < 0 {
        pcm = -pcm;
    }
    let pcm = pcm as u16;
    
    let exp = if pcm > 0x0FFF {
        7
    } else if pcm > 0x07FF {
        6
    } else if pcm > 0x03FF {
        5
    } else if pcm > 0x01FF {
        4
    } else if pcm > 0x00FF {
        3
    } else if pcm > 0x007F {
        2
    } else if pcm > 0x003F {
        1
    } else {
        0
    };
    
    let mantissa = if exp == 0 {
        (pcm >> 1) & 0x0F
    } else {
        (pcm >> exp) & 0x0F
    };
    
    (((exp << 4) | mantissa) ^ mask) as u8
}

/// Convert A-law to linear PCM sample
fn a_law_to_linear(a_law: u8) -> i16 {
    let a_law = a_law ^ 0x55;
    let sign = (a_law & 0x80) != 0;
    let exp = (a_law >> 4) & 0x07;
    let mantissa = a_law & 0x0F;
    
    let pcm = if exp == 0 {
        (mantissa << 1) + 1
    } else {
        ((mantissa << 1) + 33) << (exp - 1)
    };
    
    if sign {
        -(pcm as i16)
    } else {
        pcm as i16
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