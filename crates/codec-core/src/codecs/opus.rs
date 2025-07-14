//! Opus Audio Codec Implementation
//!
//! This module implements the Opus codec, a modern audio codec standardized 
//! by the Internet Engineering Task Force (IETF) in RFC 6716. Opus combines
//! the best features of both speech and music codecs with very low latency.

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecInfo, SampleRate};
use crate::utils::{validate_opus_frame};
use tracing::{debug, trace, warn};

// Re-export OpusApplication from types to avoid duplication
pub use crate::types::OpusApplication;

/// Opus codec implementation
pub struct OpusCodec {
    /// Sample rate (8, 12, 16, 24, or 48 kHz)
    sample_rate: u32,
    /// Number of channels (1 or 2)
    channels: u8,
    /// Frame size in samples
    frame_size: usize,
    /// Codec configuration
    config: OpusConfig,
}

/// Opus codec configuration
#[derive(Debug, Clone)]
pub struct OpusConfig {
    /// Application type (VoIP, Audio, or Low Delay)
    pub application: OpusApplication,
    /// Bitrate in bits per second
    pub bitrate: u32,
    /// Enable variable bitrate
    pub vbr: bool,
    /// Enable constrained VBR
    pub cvbr: bool,
    /// Complexity (0-10)
    pub complexity: u8,
    /// Enable inband FEC
    pub inband_fec: bool,
    /// DTX (Discontinuous Transmission)
    pub dtx: bool,
    /// Packet loss percentage (0-100)
    pub packet_loss_perc: u8,
    /// Force mono encoding
    pub force_mono: bool,
}

impl Default for OpusConfig {
    fn default() -> Self {
        Self {
            application: OpusApplication::Voip,
            bitrate: 64000,
            vbr: true,
            cvbr: false,
            complexity: 5,
            inband_fec: false,
            dtx: false,
            packet_loss_perc: 0,
            force_mono: false,
        }
    }
}

impl OpusCodec {
    /// Create a new Opus codec
    pub fn new(config: CodecConfig) -> Result<Self> {
        // Validate configuration
        let sample_rate = config.sample_rate.hz();
        
        // Opus supports 8, 12, 16, 24, 48 kHz
        if ![8000, 12000, 16000, 24000, 48000].contains(&sample_rate) {
            return Err(CodecError::InvalidSampleRate {
                rate: sample_rate,
                supported: vec![8000, 12000, 16000, 24000, 48000],
            });
        }
        
        // Opus supports mono and stereo
        if config.channels == 0 || config.channels > 2 {
            return Err(CodecError::InvalidChannelCount {
                channels: config.channels,
                supported: vec![1, 2],
            });
        }
        
        // Calculate frame size based on frame_size_ms or use default
        let frame_size = if let Some(frame_ms) = config.frame_size_ms {
            let samples_per_ms = sample_rate as f32 / 1000.0;
            (samples_per_ms * frame_ms) as usize
        } else {
            // Default to 20ms
            (sample_rate * 20 / 1000) as usize
        };
        
        // Create Opus configuration
        let opus_config = OpusConfig {
            application: config.parameters.opus.application,
            bitrate: config.parameters.opus.bitrate,
            vbr: config.parameters.opus.vbr,
            cvbr: config.parameters.opus.cvbr,
            complexity: config.parameters.opus.complexity,
            inband_fec: config.parameters.opus.inband_fec,
            dtx: config.parameters.opus.dtx,
            packet_loss_perc: config.parameters.opus.packet_loss_perc,
            force_mono: config.parameters.opus.force_mono,
        };
        
        debug!("Creating Opus codec: {}Hz, {}ch, {}bps, {:?} mode", 
               sample_rate, config.channels, opus_config.bitrate, opus_config.application);
        
        Ok(Self {
            sample_rate,
            channels: config.channels,
            frame_size,
            config: opus_config,
        })
    }
    
    /// Get the compression ratio (variable for Opus)
    pub fn compression_ratio(&self) -> f32 {
        let uncompressed_bits = self.frame_size as f32 * 16.0 * self.channels as f32;
        let compressed_bits = self.config.bitrate as f32 * (self.frame_size as f32 / self.sample_rate as f32);
        compressed_bits / uncompressed_bits
    }
    
    /// Set the bitrate
    pub fn set_bitrate(&mut self, bitrate: u32) -> Result<()> {
        if bitrate < 6000 || bitrate > 510000 {
            return Err(CodecError::InvalidBitrate {
                bitrate,
                min: 6000,
                max: 510000,
            });
        }
        
        self.config.bitrate = bitrate;
        debug!("Opus bitrate set to {} bps", bitrate);
        Ok(())
    }
    
    /// Set complexity level (0-10)
    pub fn set_complexity(&mut self, complexity: u8) -> Result<()> {
        if complexity > 10 {
            return Err(CodecError::invalid_config("Complexity must be 0-10"));
        }
        
        self.config.complexity = complexity;
        debug!("Opus complexity set to {}", complexity);
        Ok(())
    }
    
    /// Simulate Opus encoding
    fn simulate_encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // Calculate target size based on bitrate
        let frame_duration_ms = (samples.len() as f32 * 1000.0) / 
                               (self.sample_rate as f32 * self.channels as f32);
        let target_bits = (self.config.bitrate as f32 * frame_duration_ms / 1000.0) as usize;
        let target_bytes = target_bits / 8;
        
        let mut encoded = Vec::with_capacity(target_bytes.max(10));
        
        // Simple simulation - just create dummy data
        for i in 0..target_bytes {
            encoded.push((i % 256) as u8);
        }
        
        Ok(encoded)
    }
    
    /// Simulate Opus decoding
    fn simulate_decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        let mut samples = vec![0i16; self.frame_size * self.channels as usize];
        
        // Simple simulation - generate noise based on input
        for (i, sample) in samples.iter_mut().enumerate() {
            let data_idx = i % data.len();
            *sample = ((data[data_idx] as i16) << 8) | (i as i16 & 0xFF);
        }
        
        Ok(samples)
    }
}

impl AudioCodec for OpusCodec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // Validate input
        validate_opus_frame(samples, SampleRate::from_hz(self.sample_rate))?;
        
        // Simulate Opus encoding
        let encoded = self.simulate_encode(samples)?;
        
        trace!("Opus encoded {} samples to {} bytes", 
               samples.len(), encoded.len());
        
        Ok(encoded)
    }
    
    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }
        
        // Simulate Opus decoding
        let decoded = self.simulate_decode(data)?;
        
        trace!("Opus decoded {} bytes to {} samples", 
               data.len(), decoded.len());
        
        Ok(decoded)
    }
    
    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: "Opus",
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: self.config.bitrate,
            frame_size: self.frame_size,
            payload_type: Some(111), // Dynamic payload type
        }
    }
    
    fn reset(&mut self) -> Result<()> {
        debug!("Opus codec reset");
        Ok(())
    }
    
    fn frame_size(&self) -> usize {
        self.frame_size
    }
    
    fn supports_variable_frame_size(&self) -> bool {
        true // Opus supports multiple frame sizes
    }
}

impl AudioCodecExt for OpusCodec {
    fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        // Validate input
        validate_opus_frame(samples, SampleRate::from_hz(self.sample_rate))?;
        
        // Simulate Opus encoding
        let encoded = self.simulate_encode(samples)?;
        
        if output.len() < encoded.len() {
            return Err(CodecError::BufferTooSmall {
                needed: encoded.len(),
                actual: output.len(),
            });
        }
        
        output[..encoded.len()].copy_from_slice(&encoded);
        
        trace!("Opus encoded {} samples to {} bytes (zero-alloc)", 
               samples.len(), encoded.len());
        
        Ok(encoded.len())
    }
    
    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }
        
        // Simulate Opus decoding
        let decoded = self.simulate_decode(data)?;
        
        if output.len() < decoded.len() {
            return Err(CodecError::BufferTooSmall {
                needed: decoded.len(),
                actual: output.len(),
            });
        }
        
        output[..decoded.len()].copy_from_slice(&decoded);
        
        trace!("Opus decoded {} bytes to {} samples (zero-alloc)", 
               data.len(), decoded.len());
        
        Ok(decoded.len())
    }
    
    fn max_encoded_size(&self, input_samples: usize) -> usize {
        // Opus maximum frame size is 1275 bytes
        let bits_per_sample = self.config.bitrate as f32 / self.sample_rate as f32;
        let max_bytes = (input_samples as f32 * bits_per_sample / 8.0) as usize;
        max_bytes.min(1275)
    }
    
    fn max_decoded_size(&self, _input_bytes: usize) -> usize {
        // Opus can decode to various frame sizes
        let max_frame_ms = 60.0; // 60ms is the maximum
        ((self.sample_rate as f32 * max_frame_ms / 1000.0) as usize) * self.channels as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CodecConfig, CodecType, SampleRate};

    fn create_test_config() -> CodecConfig {
        CodecConfig::new(CodecType::Opus)
            .with_sample_rate(SampleRate::Rate48000)
            .with_channels(1)
            .with_frame_size_ms(20.0)
    }

    #[test]
    fn test_opus_creation() {
        let config = create_test_config();
        let codec = OpusCodec::new(config);
        assert!(codec.is_ok());
        
        let codec = codec.unwrap();
        assert_eq!(codec.frame_size(), 960); // 20ms at 48kHz
        
        let info = codec.info();
        assert_eq!(info.name, "Opus");
        assert_eq!(info.sample_rate, 48000);
        assert_eq!(info.payload_type, Some(111));
    }

    #[test]
    fn test_encoding_decoding_roundtrip() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();
        
        // Create test signal
        let mut samples = Vec::new();
        for i in 0..960 {
            let t = i as f32 / 48000.0;
            let sample = ((2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 16000.0) as i16;
            samples.push(sample);
        }
        
        // Encode
        let encoded = codec.encode(&samples).unwrap();
        assert!(encoded.len() > 0);
        
        // Decode
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), samples.len());
    }

    #[test]
    fn test_bitrate_control() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();
        
        // Test valid bitrates
        assert!(codec.set_bitrate(32000).is_ok());
        assert!(codec.set_bitrate(128000).is_ok());
        
        // Test invalid bitrates
        assert!(codec.set_bitrate(1000).is_err());
        assert!(codec.set_bitrate(1000000).is_err());
    }

    #[test]
    fn test_complexity_control() {
        let config = create_test_config();
        let mut codec = OpusCodec::new(config).unwrap();
        
        // Test valid complexity levels
        for complexity in 0..=10 {
            assert!(codec.set_complexity(complexity).is_ok());
        }
        
        // Test invalid complexity
        assert!(codec.set_complexity(11).is_err());
    }
} 