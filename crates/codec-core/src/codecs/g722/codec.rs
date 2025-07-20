//! G.722 Codec Implementation
//!
//! This module provides the main G.722 codec interface.

use crate::codecs::g722::{qmf, adpcm, state::*};
use crate::codecs::g722::reference::*;
use crate::types::{AudioCodec, CodecConfig, CodecInfo, CodecType};
use crate::error::{CodecError, Result};

/// G.722 frame size in samples (16 kHz input produces 80 sample pairs = 160 samples per 10ms frame)
pub const G722_FRAME_SIZE: usize = 160;

/// G.722 encoded frame size in bytes (80 bytes for 160 input samples)
pub const G722_ENCODED_FRAME_SIZE: usize = 80;

/// G.722 Codec with exact ITU-T reference implementation
/// 
/// 
/// # Example
/// ```
/// use rvoip_codec_core::codecs::g722::G722Codec;
/// 
/// let mut codec = G722Codec::new_with_mode(1).unwrap(); // Mode 1 (64 kbit/s)
/// 
/// // Encode a frame of 160 samples
/// let input_frame = vec![0i16; 160];
/// let encoded = codec.encode_frame(&input_frame).unwrap();
/// 
/// // Decode back to samples
/// let decoded = codec.decode_frame(&encoded).unwrap();
/// ```
#[derive(Debug, Clone)]
pub struct G722Codec {
    /// G.722 mode (1, 2, or 3)
    mode: u8,
    
    /// Encoder state (public for ITU-T compliance testing)
    pub encoder_state: G722EncoderState,
    
    /// Decoder state (public for ITU-T compliance testing)
    pub decoder_state: G722DecoderState,
}

impl G722Codec {
    /// Create a new G.722 codec from configuration
    /// 
    /// # Arguments
    /// * `config` - Codec configuration
    /// 
    /// # Returns
    /// * Result containing the codec or an error
    pub fn new(config: CodecConfig) -> Result<Self> {
        // Extract mode from G.722 configuration
        let mode = match config.codec_type {
            CodecType::G722 => {
                // Use quality parameter to determine mode, default to mode 1
                config.parameters.g722.quality + 1
            }
            _ => return Err(CodecError::unsupported_codec(format!("{:?}", config.codec_type))),
        };
        
        Self::new_with_mode(mode)
        }
        
    /// Create a new G.722 codec with specific mode
    /// 
    /// # Arguments
    /// * `mode` - G.722 mode (1=64kbit/s, 2=56kbit/s, 3=48kbit/s)
    /// 
    /// # Returns
    /// * Result containing the codec or an error
    pub fn new_with_mode(mode: u8) -> Result<Self> {
        if !(1..=3).contains(&mode) {
            return Err(CodecError::invalid_config("Invalid G.722 mode. Must be 1, 2, or 3."));
        }
        
        Ok(Self {
            mode,
            encoder_state: G722EncoderState::new(),
            decoder_state: G722DecoderState::new(),
        })
    }
    
    /// Reset the codec to initial state (ITU-T reset behavior with rs=1)
    pub fn reset(&mut self) {
        // ITU-T reference reset behavior (rs=1)
        self.encoder_state.reset();
        self.decoder_state.reset();
        
        // Additional ITU-T reset sequence
        self.reset_itu_state();
    }
    
    /// ITU-T reference reset sequence
    fn reset_itu_state(&mut self) {
        // Reset low-band state with ITU-T defaults
        self.encoder_state.state_mut().low_band_mut().reset_low_band();
        self.encoder_state.state_mut().high_band_mut().reset_high_band();
        self.decoder_state.state_mut().low_band_mut().reset_low_band();
        self.decoder_state.state_mut().high_band_mut().reset_high_band();
        
        // Reset QMF delay lines
        self.encoder_state.state_mut().qmf_tx_delay = [0; 24];
        self.encoder_state.state_mut().qmf_rx_delay = [0; 24];
        self.decoder_state.state_mut().qmf_tx_delay = [0; 24];
        self.decoder_state.state_mut().qmf_rx_delay = [0; 24];
    }
    
    /// Encode a frame of samples (ITU-T frame-based processing)
    /// 
    /// # Arguments
    /// * `samples` - Input samples (must be exactly 160 samples for 10ms frame)
    /// 
    /// # Returns
    /// * Encoded frame bytes (80 bytes)
    pub fn encode_frame(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        if samples.len() != G722_FRAME_SIZE {
            return Err(CodecError::InvalidFrameSize {
                expected: G722_FRAME_SIZE,
                actual: samples.len(),
            });
        }
        
        // Validate input samples are in valid range
        for (i, &sample) in samples.iter().enumerate() {
            if sample < -32768 || sample > 32767 {
                return Err(CodecError::EncodingFailed {
                    reason: format!("Input sample {} out of range: {} (must be -32768 to 32767)", i, sample),
                });
            }
        }
        
        let mut encoded = Vec::with_capacity(G722_ENCODED_FRAME_SIZE);
        
        // ITU-T reference: process sample pairs
        for chunk in samples.chunks_exact(2) {
            let sample0 = chunk[0];
            let sample1 = chunk[1];
            
            // QMF analysis - split into low and high bands
            let (xl, xh) = qmf::qmf_analysis(sample0, sample1, self.encoder_state.state_mut());
            
            // ADPCM encode both bands
            let low_bits = adpcm::low_band_encode(xl, self.encoder_state.state_mut().low_band_mut(), self.mode);
            let high_bits = adpcm::high_band_encode(xh, self.encoder_state.state_mut().high_band_mut());
            
            // Pack bits according to ITU-T specification
            let encoded_byte = match self.mode {
                1 => (low_bits & 0x3F) | ((high_bits & 0x03) << 6),  // 6+2 bits
                2 => (low_bits & 0x1F) | ((high_bits & 0x03) << 5),  // 5+2 bits + 1 aux bit
                3 => (low_bits & 0x0F) | ((high_bits & 0x03) << 4),  // 4+2 bits + 2 aux bits
                _ => return Err(CodecError::invalid_config("Invalid G.722 mode")),
            };
            
            encoded.push(encoded_byte);
        }
        
        Ok(encoded)
    }
    
    /// Decode a frame of bytes (ITU-T frame-based processing)
    /// 
    /// # Arguments
    /// * `data` - Encoded frame bytes (must be exactly 80 bytes)
    /// 
    /// # Returns
    /// * Decoded samples (160 samples)
    pub fn decode_frame(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        if data.len() != G722_ENCODED_FRAME_SIZE {
            return Err(CodecError::InvalidFrameSize {
                expected: G722_ENCODED_FRAME_SIZE,
                actual: data.len(),
            });
        }
        
        let mut decoded = Vec::with_capacity(G722_FRAME_SIZE);
        
        // ITU-T reference: process each encoded byte
        for &byte in data {
            // Unpack bits according to ITU-T specification
            let (low_bits, high_bits) = match self.mode {
                1 => (byte & 0x3F, (byte >> 6) & 0x03),           // 6+2 bits
                2 => (byte & 0x1F, (byte >> 5) & 0x03),           // 5+2 bits
                3 => (byte & 0x0F, (byte >> 4) & 0x03),           // 4+2 bits
                _ => return Err(CodecError::invalid_config("Invalid G.722 mode")),
            };
            
            // ADPCM decode both bands
            let xl = adpcm::low_band_decode(low_bits, self.mode, self.decoder_state.state_mut().low_band_mut());
            let xh = adpcm::high_band_decode(high_bits, self.decoder_state.state_mut().high_band_mut());
            
            // QMF synthesis - combine bands back to time domain
            let (sample0, sample1) = qmf::qmf_synthesis(xl, xh, self.decoder_state.state_mut());
            
            decoded.push(sample0);
            decoded.push(sample1);
        }
        
        // Validate output samples are in valid range
        for (i, &sample) in decoded.iter().enumerate() {
            if sample < -32768 || sample > 32767 {
                return Err(CodecError::DecodingFailed {
                    reason: format!("Output sample {} out of range: {} (must be -32768 to 32767)", i, sample),
                });
            }
        }
        
        Ok(decoded)
    }
    
    /// Get G.722 mode
    pub fn mode(&self) -> u8 {
        self.mode
    }
    
    /// Get encoder state (for ITU-T compliance testing)
    pub fn encoder_state(&self) -> &G722EncoderState {
        &self.encoder_state
    }
    
    /// Get mutable encoder state (for ITU-T compliance testing)
    pub fn encoder_state_mut(&mut self) -> &mut G722EncoderState {
        &mut self.encoder_state
    }
    
    /// Get decoder state (for ITU-T compliance testing)
    pub fn decoder_state(&self) -> &G722DecoderState {
        &self.decoder_state
    }
    
    /// Get mutable decoder state (for ITU-T compliance testing)
    pub fn decoder_state_mut(&mut self) -> &mut G722DecoderState {
        &mut self.decoder_state
    }
    
    /// Get the compression ratio
    pub fn compression_ratio(&self) -> f32 {
        0.5 // G.722 is 2:1 compression (16-bit to 8-bit)
    }
    
    /// Encode a single sample pair (for compatibility)
    pub fn encode_sample_pair(&mut self, samples: [i16; 2]) -> u8 {
        // QMF analysis - split into low and high bands
        let (xl, xh) = qmf::qmf_analysis(samples[0], samples[1], self.encoder_state.state_mut());
        
        // ADPCM encode both bands with proper mode support
        let low_bits = adpcm::low_band_encode(xl, self.encoder_state.state_mut().low_band_mut(), self.mode);
        let high_bits = adpcm::high_band_encode(xh, self.encoder_state.state_mut().high_band_mut());
        
        // Pack bits according to ITU-T specification (mode-dependent)
        match self.mode {
            1 => (low_bits & 0x3F) | ((high_bits & 0x03) << 6),  // 6+2 bits
            2 => (low_bits & 0x1F) | ((high_bits & 0x03) << 5),  // 5+2 bits + 1 aux bit
            3 => (low_bits & 0x0F) | ((high_bits & 0x03) << 4),  // 4+2 bits + 2 aux bits
            _ => 0, // Invalid mode - should not happen
        }
    }
    
    /// Decode a single byte to sample pair (for compatibility)
    pub fn decode_byte(&mut self, byte: u8) -> [i16; 2] {
        // Unpack bits according to ITU-T specification (mode-dependent)
        let (low_bits, high_bits) = match self.mode {
            1 => (byte & 0x3F, (byte >> 6) & 0x03),           // 6+2 bits
            2 => (byte & 0x1F, (byte >> 5) & 0x03),           // 5+2 bits
            3 => (byte & 0x0F, (byte >> 4) & 0x03),           // 4+2 bits
            _ => (0, 0), // Invalid mode - should not happen
        };
        
        // ADPCM decode both bands
        let xl = adpcm::low_band_decode(low_bits, self.mode, self.decoder_state.state_mut().low_band_mut());
        let xh = adpcm::high_band_decode(high_bits, self.decoder_state.state_mut().high_band_mut());
        
        // QMF synthesis - combine bands back to time domain
        let (sample1, sample2) = qmf::qmf_synthesis(xl, xh, self.decoder_state.state_mut());
        [sample1, sample2]
    }
}

impl Default for G722Codec {
    fn default() -> Self {
        Self::new_with_mode(1).unwrap()
    }
}

impl AudioCodec for G722Codec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        self.encode_frame(samples)
    }
    
    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        self.decode_frame(data)
    }
    
    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: "G.722",
            sample_rate: 16000,
            channels: 1,
            bitrate: match self.mode {
                1 => 64000,
                2 => 56000,
                3 => 48000,
                _ => 64000,
            },
            frame_size: G722_FRAME_SIZE,
            payload_type: Some(9),
        }
    }
    
    fn reset(&mut self) -> Result<()> {
        self.reset();
        Ok(())
    }
    
    fn frame_size(&self) -> usize {
        G722_FRAME_SIZE
    }
}

/// ITU-T G.722 encoder function
/// 
/// Encodes a frame of 160 input samples to 80 output bytes
/// 
/// # Arguments
/// * `input` - Input samples (160 samples)
/// * `output` - Output encoded bytes (80 bytes)
/// * `mode` - G.722 mode (1, 2, or 3)
/// * `state` - Encoder state
/// 
/// # Returns
/// * Number of bytes encoded
pub fn g722_encode_frame(
    input: &[i16],
    output: &mut [u8],
    mode: u8,
    state: &mut G722EncoderState,
) -> Result<usize> {
    if input.len() != G722_FRAME_SIZE {
            return Err(CodecError::InvalidFrameSize {
            expected: G722_FRAME_SIZE,
            actual: input.len(),
            });
        }
    if output.len() < G722_ENCODED_FRAME_SIZE {
            return Err(CodecError::BufferTooSmall {
            needed: G722_ENCODED_FRAME_SIZE,
                actual: output.len(),
            });
        }
        
    for (i, chunk) in input.chunks_exact(2).enumerate() {
        let sample0 = chunk[0];
        let sample1 = chunk[1];
        
        // QMF analysis
        let (xl, xh) = qmf::qmf_analysis(sample0, sample1, state.state_mut());
        
        // ADPCM encode
        let low_bits = adpcm::low_band_encode(xl, state.state_mut().low_band_mut(), mode);
        let high_bits = adpcm::high_band_encode(xh, state.state_mut().high_band_mut());
        
        // Pack bits
        output[i] = (low_bits & 0x3F) | ((high_bits & 0x03) << 6);
        }
        
    Ok(G722_ENCODED_FRAME_SIZE)
}

/// ITU-T G.722 decoder function
/// 
/// Decodes 80 input bytes to 160 output samples
/// 
/// # Arguments
/// * `input` - Input encoded bytes (80 bytes)
/// * `output` - Output decoded samples (160 samples)
/// * `mode` - G.722 mode (1, 2, or 3)
/// * `state` - Decoder state
/// 
/// # Returns
/// * Number of samples decoded
pub fn g722_decode_frame(
    input: &[u8],
    output: &mut [i16],
    mode: u8,
    state: &mut G722DecoderState,
) -> Result<usize> {
    if input.len() != G722_ENCODED_FRAME_SIZE {
        return Err(CodecError::InvalidFrameSize {
            expected: G722_ENCODED_FRAME_SIZE,
            actual: input.len(),
        });
    }
    if output.len() < G722_FRAME_SIZE {
            return Err(CodecError::BufferTooSmall {
            needed: G722_FRAME_SIZE,
                actual: output.len(),
            });
        }
        
    for (i, &byte) in input.iter().enumerate() {
        // Unpack bits
        let low_bits = byte & 0x3F;
        let high_bits = (byte >> 6) & 0x03;
        
        // ADPCM decode
        let xl = adpcm::low_band_decode(low_bits, mode, state.state_mut().low_band_mut());
        let xh = adpcm::high_band_decode(high_bits, state.state_mut().high_band_mut());
        
        // QMF synthesis
        let (sample0, sample1) = qmf::qmf_synthesis(xl, xh, state.state_mut());
        
        output[i * 2] = sample0;
        output[i * 2 + 1] = sample1;
    }
    
    Ok(G722_FRAME_SIZE)
}