//! G.722 High-Level Codec Implementation
//!
//! This module provides the high-level G.722 codec interface that implements
//! the AudioCodec trait for integration with the codec-core library.

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecInfo, SampleRate};
use crate::codecs::g722::{qmf, adpcm, state::*};
use tracing::{debug, trace};

/// G.722 codec implementation
pub struct G722Codec {
    /// Sample rate (fixed at 16kHz)
    sample_rate: u32,
    /// Number of channels (fixed at 1)
    channels: u8,
    /// Frame size in samples
    frame_size: usize,
    /// Encoder state
    encoder_state: G722EncoderState,
    /// Decoder state
    decoder_state: G722DecoderState,
    /// G.722 mode (1, 2, or 3)
    mode: u8,
}

impl G722Codec {
    /// Create a new G.722 codec
    pub fn new(config: CodecConfig) -> Result<Self> {
        // Validate configuration
        let sample_rate = config.sample_rate.hz();
        
        // G.722 only supports 16kHz
        if sample_rate != 16000 {
            return Err(CodecError::InvalidSampleRate {
                rate: sample_rate,
                supported: vec![16000],
            });
        }
        
        // G.722 only supports mono
        if config.channels != 1 {
            return Err(CodecError::InvalidChannelCount {
                channels: config.channels,
                supported: vec![1],
            });
        }
        
        // Calculate frame size based on frame_size_ms or use default
        let frame_size = if let Some(frame_ms) = config.frame_size_ms {
            (sample_rate as f32 * frame_ms / 1000.0) as usize
        } else {
            320 // Default 20ms frame at 16kHz
        };
        
        // Validate frame size (must be even for QMF processing)
        if frame_size % 2 != 0 || ![160, 320, 480, 640].contains(&frame_size) {
            return Err(CodecError::InvalidFrameSize {
                expected: 320,
                actual: frame_size,
            });
        }
        
        debug!("Creating G.722 codec: {}Hz, {}ch, {} samples/frame", 
               sample_rate, config.channels, frame_size);
        
        Ok(Self {
            sample_rate,
            channels: config.channels,
            frame_size,
            encoder_state: G722EncoderState::new(),
            decoder_state: G722DecoderState::new(),
            mode: 1, // Default to mode 1 (64 kbps)
        })
    }
    
    /// Set G.722 mode
    pub fn set_mode(&mut self, mode: u8) -> Result<()> {
        if ![1, 2, 3].contains(&mode) {
            return Err(CodecError::InvalidPayload {
                details: format!("Invalid G.722 mode: {}. Must be 1, 2, or 3", mode),
            });
        }
        self.mode = mode;
        Ok(())
    }
    
    /// Get G.722 mode
    pub fn mode(&self) -> u8 {
        self.mode
    }
    
    /// Get the compression ratio
    pub fn compression_ratio(&self) -> f32 {
        0.5 // G.722 is 2:1 compression (16-bit to 8-bit)
    }
    
    /// Encode a single sample pair
    fn encode_sample_pair(&mut self, samples: [i16; 2]) -> u8 {
        // QMF analysis - split into low and high bands
        let (xl, xh) = qmf::qmf_analysis(samples[0], samples[1], self.encoder_state.state_mut());
        
        // ADPCM encode both bands with proper mode support
        let low_bits = adpcm::low_band_encode(xl, self.encoder_state.state_mut().low_band_mut(), self.mode);
        let high_bits = adpcm::high_band_encode(xh, self.encoder_state.state_mut().high_band_mut());
        
        // Pack bits: 6 bits for low band + 2 bits for high band
        (low_bits & 0x3F) | ((high_bits & 0x03) << 6)
    }
    
    /// Decode a single byte to sample pair
    fn decode_byte(&mut self, byte: u8) -> [i16; 2] {
        // Unpack bits: 6 bits for low band + 2 bits for high band
        let low_bits = byte & 0x3F;
        let high_bits = (byte >> 6) & 0x03;
        
        // ADPCM decode both bands
        let xl = adpcm::low_band_decode(low_bits, self.mode, self.decoder_state.state_mut().low_band_mut());
        let xh = adpcm::high_band_decode(high_bits, self.decoder_state.state_mut().high_band_mut());
        
        // QMF synthesis - combine bands back to time domain
        let (sample1, sample2) = qmf::qmf_synthesis(xl, xh, self.decoder_state.state_mut());
        [sample1, sample2]
    }
}

impl AudioCodec for G722Codec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // Validate input
        if samples.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize {
                expected: self.frame_size,
                actual: samples.len(),
            });
        }
        
        let mut output = vec![0u8; samples.len() / 2];
        let encoded_len = self.encode_to_buffer(samples, &mut output)?;
        output.truncate(encoded_len);
        
        trace!("G.722 encoded {} samples to {} bytes", 
               samples.len(), output.len());
        
        Ok(output)
    }
    
    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }
        
        let mut output = vec![0i16; data.len() * 2];
        let decoded_len = self.decode_to_buffer(data, &mut output)?;
        output.truncate(decoded_len);
        
        trace!("G.722 decoded {} bytes to {} samples", 
               data.len(), output.len());
        
        Ok(output)
    }
    
    fn info(&self) -> CodecInfo {
        let bitrate = match self.mode {
            1 => 64000,  // 64 kbps
            2 => 56000,  // 56 kbps
            3 => 48000,  // 48 kbps
            _ => 64000,
        };
        
        CodecInfo {
            name: "G722",
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate,
            frame_size: self.frame_size,
            payload_type: Some(9),
        }
    }
    
    fn reset(&mut self) -> Result<()> {
        self.encoder_state.reset();
        self.decoder_state.reset();
        
        debug!("G.722 codec reset");
        Ok(())
    }
    
    fn frame_size(&self) -> usize {
        self.frame_size
    }
    
    fn supports_variable_frame_size(&self) -> bool {
        true
    }
}

impl AudioCodecExt for G722Codec {
    fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        // Validate input
        if samples.len() != self.frame_size {
            return Err(CodecError::InvalidFrameSize {
                expected: self.frame_size,
                actual: samples.len(),
            });
        }
        
        if samples.len() % 2 != 0 {
            return Err(CodecError::InvalidFrameSize {
                expected: self.frame_size,
                actual: samples.len(),
            });
        }
        
        let expected_output_size = samples.len() / 2;
        if output.len() < expected_output_size {
            return Err(CodecError::BufferTooSmall {
                needed: expected_output_size,
                actual: output.len(),
            });
        }
        
        let mut output_idx = 0;
        
        // Process samples in pairs (QMF requires even number)
        for chunk in samples.chunks_exact(2) {
            let encoded_byte = self.encode_sample_pair([chunk[0], chunk[1]]);
            output[output_idx] = encoded_byte;
            output_idx += 1;
        }
        
        trace!("G.722 encoded {} samples to {} bytes (zero-alloc)", 
               samples.len(), output_idx);
        
        Ok(output_idx)
    }
    
    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }
        
        let expected_output_size = data.len() * 2;
        if output.len() < expected_output_size {
            return Err(CodecError::BufferTooSmall {
                needed: expected_output_size,
                actual: output.len(),
            });
        }
        
        let mut output_idx = 0;
        
        // Decode each byte to two samples
        for &byte in data {
            let decoded_samples = self.decode_byte(byte);
            output[output_idx] = decoded_samples[0];
            output[output_idx + 1] = decoded_samples[1];
            output_idx += 2;
        }
        
        trace!("G.722 decoded {} bytes to {} samples (zero-alloc)", 
               data.len(), output_idx);
        
        Ok(output_idx)
    }
    
    fn max_encoded_size(&self, input_samples: usize) -> usize {
        // G.722 encodes 2 samples into 1 byte
        input_samples / 2
    }
    
    fn max_decoded_size(&self, input_bytes: usize) -> usize {
        // G.722 decodes 1 byte into 2 samples
        input_bytes * 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CodecConfig, CodecType, SampleRate};

    fn create_test_config() -> CodecConfig {
        CodecConfig::new(CodecType::G722)
            .with_sample_rate(SampleRate::Rate16000)
            .with_channels(1)
            .with_frame_size_ms(20.0)
    }

    #[test]
    fn test_g722_creation() {
        let config = create_test_config();
        let codec = G722Codec::new(config);
        assert!(codec.is_ok());
        
        let codec = codec.unwrap();
        assert_eq!(codec.sample_rate, 16000);
        assert_eq!(codec.channels, 1);
        assert_eq!(codec.frame_size, 320);
        assert_eq!(codec.mode, 1);
    }

    #[test]
    fn test_g722_mode() {
        let config = create_test_config();
        let mut codec = G722Codec::new(config).unwrap();
        
        assert_eq!(codec.mode(), 1);
        
        assert!(codec.set_mode(2).is_ok());
        assert_eq!(codec.mode(), 2);
        
        assert!(codec.set_mode(4).is_err()); // Invalid mode
    }

    #[test]
    fn test_invalid_sample_rate() {
        let mut config = create_test_config();
        config.sample_rate = SampleRate::Rate8000;
        
        let codec = G722Codec::new(config);
        assert!(codec.is_err());
    }

    #[test]
    fn test_invalid_channels() {
        let mut config = create_test_config();
        config.channels = 2;
        
        let codec = G722Codec::new(config);
        assert!(codec.is_err());
    }

    #[test]
    fn test_encoding_decoding_roundtrip() {
        let config = create_test_config();
        let mut codec = G722Codec::new(config).unwrap();
        
        // Create test signal
        let samples = vec![1000i16; 320];
        
        // Encode
        let encoded = codec.encode(&samples).unwrap();
        assert_eq!(encoded.len(), samples.len() / 2);
        
        // Decode
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), samples.len());
        
        // Check that decoding produces reasonable output
        // G.722 is lossy, so we expect some distortion
        for (original, decoded) in samples.iter().zip(decoded.iter()) {
            let error = (original - decoded).abs();
            assert!(error < 16000, "Error too large: {} vs {} (error: {})", original, decoded, error);
        }
    }

    #[test]
    fn test_zero_copy_apis() {
        let config = create_test_config();
        let mut codec = G722Codec::new(config).unwrap();
        
        let samples = vec![1000i16; 320];
        let mut encoded = vec![0u8; 160];
        let mut decoded = vec![0i16; 320];
        
        // Test zero-copy encoding
        let encoded_len = codec.encode_to_buffer(&samples, &mut encoded).unwrap();
        assert_eq!(encoded_len, 160);
        
        // Test zero-copy decoding
        let decoded_len = codec.decode_to_buffer(&encoded, &mut decoded).unwrap();
        assert_eq!(decoded_len, 320);
    }

    #[test]
    fn test_frame_size_validation() {
        let config = create_test_config();
        let mut codec = G722Codec::new(config).unwrap();
        
        // Wrong frame size should fail
        let wrong_samples = vec![0i16; 100];
        assert!(codec.encode(&wrong_samples).is_err());
        
        // Odd number of samples should fail in encode_to_buffer
        let odd_samples = vec![0i16; 321];
        let mut output = vec![0u8; 200];
        assert!(codec.encode_to_buffer(&odd_samples, &mut output).is_err());
    }

    #[test]
    fn test_codec_reset() {
        let config = create_test_config();
        let mut codec = G722Codec::new(config).unwrap();
        
        assert!(codec.reset().is_ok());
    }

    #[test]
    fn test_compression_ratio() {
        let config = create_test_config();
        let codec = G722Codec::new(config).unwrap();
        
        assert_eq!(codec.compression_ratio(), 0.5);
        assert_eq!(codec.max_encoded_size(320), 160);
        assert_eq!(codec.max_decoded_size(160), 320);
    }

    #[test]
    fn test_different_frame_sizes() {
        // Test 10ms frame
        let mut config = create_test_config();
        config.frame_size_ms = Some(10.0);
        let codec = G722Codec::new(config).unwrap();
        assert_eq!(codec.frame_size(), 160);
        
        // Test 30ms frame
        let mut config = create_test_config();
        config.frame_size_ms = Some(30.0);
        let codec = G722Codec::new(config).unwrap();
        assert_eq!(codec.frame_size(), 480);
    }

    #[test]
    fn test_buffer_size_validation() {
        let config = create_test_config();
        let mut codec = G722Codec::new(config).unwrap();
        
        let samples = vec![0i16; 320];
        let mut small_buffer = vec![0u8; 80]; // Too small
        
        assert!(codec.encode_to_buffer(&samples, &mut small_buffer).is_err());
    }

    #[test]
    fn test_empty_data_handling() {
        let config = create_test_config();
        let mut codec = G722Codec::new(config).unwrap();
        
        // Empty encoded data should fail
        let empty_data: Vec<u8> = vec![];
        assert!(codec.decode(&empty_data).is_err());
    }

    #[test]
    fn test_codec_info_details() {
        let config = create_test_config();
        let codec = G722Codec::new(config).unwrap();
        
        let info = codec.info();
        assert_eq!(info.name, "G722");
        assert_eq!(info.sample_rate, 16000);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bitrate, 64000);
        assert_eq!(info.payload_type, Some(9));
        
        assert!(codec.supports_variable_frame_size());
    }

    #[test]
    fn test_mode_bitrate() {
        let config = create_test_config();
        let mut codec = G722Codec::new(config).unwrap();
        
        // Test different modes affect bitrate
        codec.set_mode(1).unwrap();
        assert_eq!(codec.info().bitrate, 64000);
        
        codec.set_mode(2).unwrap();
        assert_eq!(codec.info().bitrate, 56000);
        
        codec.set_mode(3).unwrap();
        assert_eq!(codec.info().bitrate, 48000);
    }
} 