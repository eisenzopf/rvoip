use crate::types::{AudioFrame, AudioFormat};
use crate::error::AudioError;
use super::{CodecType, AudioCodecTrait, CodecConfig};

/// Opus audio codec implementation
/// 
/// This is a mock implementation for demonstration purposes.
/// In a real implementation, this would use the `opus` crate or similar.
pub struct OpusEncoder {
    config: CodecConfig,
    encoder_state: OpusEncoderState,
    decoder_state: OpusDecoderState,
}

/// Opus encoder state
struct OpusEncoderState {
    /// Frame size in samples
    frame_size: usize,
    /// Current bitrate
    bitrate: u32,
    /// Complexity setting (0-10)
    complexity: u8,
    /// VBR mode enabled
    vbr_enabled: bool,
    /// Application type (VoIP, Audio, etc.)
    application: OpusApplication,
    /// Internal buffer for frame accumulation
    buffer: Vec<f32>,
    /// Prediction filter state
    prediction_state: Vec<f32>,
}

/// Opus decoder state
struct OpusDecoderState {
    /// Frame size in samples
    frame_size: usize,
    /// Number of channels
    channels: u32,
    /// Packet loss concealment state
    plc_state: Vec<f32>,
    /// Post-filter state
    postfilter_state: Vec<f32>,
}

/// Opus application type
#[derive(Debug, Clone, Copy)]
enum OpusApplication {
    /// VoIP application - optimized for speech
    VoIP,
    /// Audio application - optimized for general audio
    Audio,
    /// Restricted low-delay mode
    RestrictedLowDelay,
}

impl OpusEncoder {
    /// Create a new Opus encoder
    pub fn new(config: CodecConfig) -> Result<Self, AudioError> {
        // Validate configuration
        let valid_sample_rates = [8000, 12000, 16000, 24000, 48000];
        if !valid_sample_rates.contains(&config.sample_rate) {
            return Err(AudioError::invalid_configuration(
                format!("Opus doesn't support sample rate {}, supported rates: {:?}", 
                       config.sample_rate, valid_sample_rates)
            ));
        }
        
        if config.channels == 0 || config.channels > 2 {
            return Err(AudioError::invalid_configuration(
                format!("Opus supports 1-2 channels, got {}", config.channels)
            ));
        }

        // Calculate frame size based on sample rate
        let frame_size = match config.sample_rate {
            8000 => 160,   // 20ms at 8kHz
            12000 => 240,  // 20ms at 12kHz
            16000 => 320,  // 20ms at 16kHz
            24000 => 480,  // 20ms at 24kHz
            48000 => 960,  // 20ms at 48kHz
            _ => return Err(AudioError::invalid_configuration(
                format!("Unsupported sample rate: {}", config.sample_rate)
            )),
        };

        let encoder_state = OpusEncoderState {
            frame_size,
            bitrate: config.bitrate,
            complexity: 5, // Default complexity
            vbr_enabled: true,
            application: OpusApplication::VoIP,
            buffer: Vec::new(),
            prediction_state: vec![0.0; 16],
        };

        let decoder_state = OpusDecoderState {
            frame_size,
            channels: config.channels,
            plc_state: vec![0.0; frame_size],
            postfilter_state: vec![0.0; 8],
        };

        Ok(Self {
            config,
            encoder_state,
            decoder_state,
        })
    }

    /// Set encoder complexity (0-10)
    pub fn set_complexity(&mut self, complexity: u8) -> Result<(), AudioError> {
        if complexity > 10 {
            return Err(AudioError::invalid_configuration(
                format!("Opus complexity must be 0-10, got {}", complexity)
            ));
        }
        self.encoder_state.complexity = complexity;
        Ok(())
    }

    /// Enable/disable variable bitrate mode
    pub fn set_vbr(&mut self, enabled: bool) {
        self.encoder_state.vbr_enabled = enabled;
    }

    /// Set application type
    pub fn set_application(&mut self, application: OpusApplication) {
        self.encoder_state.application = application;
    }

    /// Set target bitrate
    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<(), AudioError> {
        if bitrate < 6000 || bitrate > 512000 {
            return Err(AudioError::invalid_configuration(
                format!("Opus bitrate must be 6k-512k bps, got {}", bitrate)
            ));
        }
        self.encoder_state.bitrate = bitrate;
        self.config.bitrate = bitrate;
        Ok(())
    }
}

impl AudioCodecTrait for OpusEncoder {
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

        // Accumulate samples in buffer (convert i16 to f32 for processing)
        let f32_samples: Vec<f32> = frame.samples.iter().map(|&s| s as f32 / 32767.0).collect();
        self.encoder_state.buffer.extend_from_slice(&f32_samples);

        let mut encoded_packets = Vec::new();

        // Process complete frames
        while self.encoder_state.buffer.len() >= self.encoder_state.frame_size * self.config.channels as usize {
            let frame_samples = self.encoder_state.buffer.drain(..self.encoder_state.frame_size * self.config.channels as usize).collect::<Vec<_>>();
            
            // Mock Opus encoding - in reality this would use libopus
            let encoded_frame = self.encode_frame(&frame_samples)?;
            encoded_packets.extend(encoded_frame);
        }

        Ok(encoded_packets)
    }

    fn decode(&mut self, data: &[u8]) -> Result<AudioFrame, AudioError> {
        if data.is_empty() {
            return Err(AudioError::invalid_format("Empty Opus packet".to_string()));
        }

        // Mock Opus decoding - in reality this would use libopus
        let f32_samples = self.decode_packet(data)?;
        
        // Convert f32 samples back to i16
        let samples: Vec<i16> = f32_samples.iter().map(|&s| (s * 32767.0) as i16).collect();

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
        self.encoder_state.buffer.clear();
        self.encoder_state.prediction_state.fill(0.0);
        self.decoder_state.plc_state.fill(0.0);
        self.decoder_state.postfilter_state.fill(0.0);
        Ok(())
    }

    fn codec_type(&self) -> CodecType {
        CodecType::Opus
    }
}

impl OpusEncoder {
    /// Encode a single frame (mock implementation)
    fn encode_frame(&mut self, samples: &[f32]) -> Result<Vec<u8>, AudioError> {
        // This is a simplified mock encoding
        // Real Opus encoding would involve complex psychoacoustic modeling
        
        // Simple compression: quantize samples and run-length encode
        let mut encoded = Vec::new();
        
        // Add header with frame info
        encoded.push(0x80); // Opus packet header (mock)
        encoded.push((self.config.channels as u8) << 4 | (self.encoder_state.complexity & 0x0F));
        
        // Quantize samples to 8-bit
        let quantized: Vec<u8> = samples.iter()
            .map(|&sample| {
                let clamped = sample.clamp(-1.0, 1.0);
                ((clamped + 1.0) * 127.5) as u8
            })
            .collect();
        
        // Simple run-length encoding
        let mut i = 0;
        while i < quantized.len() {
            let current = quantized[i];
            let mut count = 1;
            
            // Count consecutive identical values
            while i + count < quantized.len() && quantized[i + count] == current && count < 255 {
                count += 1;
            }
            
            if count > 3 {
                // Use run-length encoding
                encoded.push(0xFF); // RLE marker
                encoded.push(count as u8);
                encoded.push(current);
            } else {
                // Store values directly
                for j in 0..count {
                    encoded.push(quantized[i + j]);
                }
            }
            
            i += count;
        }
        
        Ok(encoded)
    }

    /// Decode a packet (mock implementation)
    fn decode_packet(&mut self, data: &[u8]) -> Result<Vec<f32>, AudioError> {
        if data.len() < 2 {
            return Err(AudioError::invalid_format("Opus packet too short".to_string()));
        }

        // Parse mock header
        let _header = data[0];
        let _config = data[1];
        
        // Decode the payload
        let mut decoded_samples = Vec::new();
        let mut i = 2;
        
        while i < data.len() {
            if data[i] == 0xFF && i + 2 < data.len() {
                // Run-length encoded
                let count = data[i + 1] as usize;
                let value = data[i + 2];
                
                for _ in 0..count {
                    let sample = (value as f32 / 127.5) - 1.0;
                    decoded_samples.push(sample);
                }
                
                i += 3;
            } else {
                // Direct value
                let sample = (data[i] as f32 / 127.5) - 1.0;
                decoded_samples.push(sample);
                i += 1;
            }
        }
        
        // Apply post-processing (mock)
        self.apply_postfilter(&mut decoded_samples);
        
        Ok(decoded_samples)
    }

    /// Apply post-filter processing (mock)
    fn apply_postfilter(&mut self, samples: &mut [f32]) {
        // Simple high-pass filter to reduce low-frequency noise
        let alpha = 0.95;
        
        for i in 1..samples.len() {
            let filtered = samples[i] - alpha * samples[i - 1];
            samples[i] = filtered;
        }
    }
}

/// Opus-specific error types
#[derive(Debug, Clone)]
pub enum OpusError {
    /// Invalid packet format
    InvalidPacket,
    /// Unsupported configuration
    UnsupportedConfig,
    /// Internal encoder error
    EncoderError,
    /// Internal decoder error
    DecoderError,
}

impl std::fmt::Display for OpusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpusError::InvalidPacket => write!(f, "Invalid Opus packet"),
            OpusError::UnsupportedConfig => write!(f, "Unsupported Opus configuration"),
            OpusError::EncoderError => write!(f, "Opus encoder error"),
            OpusError::DecoderError => write!(f, "Opus decoder error"),
        }
    }
}

impl std::error::Error for OpusError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opus_encoder_creation() {
        let config = CodecConfig {
            codec: CodecType::Opus,
            sample_rate: 48000,
            channels: 1,
            bitrate: 32000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = OpusEncoder::new(config);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_opus_stereo_creation() {
        let config = CodecConfig {
            codec: CodecType::Opus,
            sample_rate: 48000,
            channels: 2,
            bitrate: 64000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = OpusEncoder::new(config);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_invalid_sample_rate() {
        let config = CodecConfig {
            codec: CodecType::Opus,
            sample_rate: 11025, // Invalid for Opus
            channels: 1,
            bitrate: 32000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = OpusEncoder::new(config);
        assert!(encoder.is_err());
    }

    #[test]
    fn test_opus_encoding_decoding() {
        let config = CodecConfig {
            codec: CodecType::Opus,
            sample_rate: 16000,
            channels: 1,
            bitrate: 32000,
            params: std::collections::HashMap::new(),
        };
        
        let mut encoder = OpusEncoder::new(config).unwrap();
        
        // Create a full frame (320 samples for 16kHz)
        let samples = vec![0; 320];
        let frame = AudioFrame {
            samples,
            format: AudioFormat {
                sample_rate: 16000,
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
        assert!(!encoded.is_empty());
        
        // Decode
        let decoded_frame = encoder.decode(&encoded).unwrap();
        assert!(decoded_frame.samples.len() > 0);
    }

    #[test]
    fn test_opus_configuration() {
        let config = CodecConfig {
            codec: CodecType::Opus,
            sample_rate: 48000,
            channels: 1,
            bitrate: 32000,
            params: std::collections::HashMap::new(),
        };
        
        let mut encoder = OpusEncoder::new(config).unwrap();
        
        // Test complexity setting
        assert!(encoder.set_complexity(8).is_ok());
        assert!(encoder.set_complexity(11).is_err());
        
        // Test bitrate setting
        assert!(encoder.set_bitrate(24000).is_ok());
        assert!(encoder.set_bitrate(1000).is_err());
        
        // Test VBR setting
        encoder.set_vbr(false);
        encoder.set_vbr(true);
        
        // Test application setting
        encoder.set_application(OpusApplication::Audio);
        encoder.set_application(OpusApplication::VoIP);
    }

    #[test]
    fn test_opus_frame_accumulation() {
        let config = CodecConfig {
            codec: CodecType::Opus,
            sample_rate: 16000,
            channels: 1,
            bitrate: 32000,
            params: std::collections::HashMap::new(),
        };
        
        let mut encoder = OpusEncoder::new(config).unwrap();
        
        // Send partial frame (less than 320 samples)
        let partial_samples = vec![0; 100];
        let partial_frame = AudioFrame {
            samples: partial_samples,
            format: AudioFormat {
                sample_rate: 16000,
                channels: 1,
                bits_per_sample: 16,
                frame_size_ms: 20,
            },
            timestamp: 0,
            sequence: 0,
            metadata: std::collections::HashMap::new(),
        };
        
        // Should not produce output yet
        let encoded = encoder.encode(&partial_frame).unwrap();
        assert!(encoded.is_empty());
        
        // Send remaining samples to complete frame
        let remaining_samples = vec![0; 220];
        let remaining_frame = AudioFrame {
            samples: remaining_samples,
            format: AudioFormat {
                sample_rate: 16000,
                channels: 1,
                bits_per_sample: 16,
                frame_size_ms: 20,
            },
            timestamp: 0,
            sequence: 0,
            metadata: std::collections::HashMap::new(),
        };
        
        // Should now produce output
        let encoded = encoder.encode(&remaining_frame).unwrap();
        assert!(!encoded.is_empty());
    }

    #[test]
    fn test_codec_reset() {
        let config = CodecConfig {
            codec: CodecType::Opus,
            sample_rate: 48000,
            channels: 1,
            bitrate: 32000,
            params: std::collections::HashMap::new(),
        };
        
        let mut encoder = OpusEncoder::new(config).unwrap();
        assert!(encoder.reset().is_ok());
    }

    #[test]
    fn test_codec_type() {
        let config = CodecConfig {
            codec: CodecType::Opus,
            sample_rate: 48000,
            channels: 1,
            bitrate: 32000,
            params: std::collections::HashMap::new(),
        };
        
        let encoder = OpusEncoder::new(config).unwrap();
        assert_eq!(encoder.codec_type(), CodecType::Opus);
    }
} 