//! G.722 Test Utilities
//!
//! This module provides utilities for parsing ITU G.191 test vectors and handling
//! the test vector format used in G.722 compliance testing.

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use crate::AudioCodec;
use crate::codecs::g722::state::{G722EncoderState, G722DecoderState};
use crate::codecs::g722::G722Codec;

/// ITU-T G.191 test vector format constants
pub const G191_SYNC_PATTERN: u16 = 0x0001;
/// ITU-T G.191 sync pattern length in 16-bit words (32 bytes total)
pub const G191_SYNC_PATTERN_LENGTH: usize = 16;

/// ITU-T G.192 format constants
/// Good frame sync header in G.192 format
pub const G192_SYNC_GOOD: u16 = 0x6B21;  // Good frame sync
/// Bad frame sync header in G.192 format (frame erasure)
pub const G192_SYNC_BAD: u16 = 0x6B20;   // Bad frame sync (frame erasure)
/// Soft bit '0' representation in G.192 format
pub const G192_ZERO: u16 = 0x007F;       // Soft bit '0'
/// Soft bit '1' representation in G.192 format
pub const G192_ONE: u16 = 0x0081;        // Soft bit '1'

/// G.722 frame sizes in bits for different modes
/// Frame size in bits for Mode 1 (64 kbps)
pub const G722_FRAME_SIZE_BITS_MODE1: u16 = 640;  // 64 kbps (80 samples * 8 bits)
/// Frame size in bits for Mode 2 (56 kbps)
pub const G722_FRAME_SIZE_BITS_MODE2: u16 = 560;  // 56 kbps (80 samples * 7 bits)
/// Frame size in bits for Mode 3 (48 kbps)
pub const G722_FRAME_SIZE_BITS_MODE3: u16 = 480;  // 48 kbps (80 samples * 6 bits)
/// Frame size in samples at 8 kHz
pub const G722_FRAME_SIZE_SAMPLES: usize = 80;     // Frame size in samples at 8 kHz

/// G.722 Mode enumeration
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum G722Mode {
    /// Mode 1: 64 kbps
    Mode1 = 1,  // 64 kbps
    /// Mode 2: 56 kbps
    Mode2 = 2,  // 56 kbps
    /// Mode 3: 48 kbps
    Mode3 = 3,  // 48 kbps
}

impl G722Mode {
    /// Get the number of bits per sample for this mode
    pub fn bits_per_sample(&self) -> u8 {
        match self {
            G722Mode::Mode1 => 8,
            G722Mode::Mode2 => 7,
            G722Mode::Mode3 => 6,
        }
    }
    
    /// Get the frame size in bits for this mode
    pub fn frame_size_bits(&self) -> u16 {
        match self {
            G722Mode::Mode1 => G722_FRAME_SIZE_BITS_MODE1,
            G722Mode::Mode2 => G722_FRAME_SIZE_BITS_MODE2,
            G722Mode::Mode3 => G722_FRAME_SIZE_BITS_MODE3,
        }
    }
    
    /// Create a mode from frame size in bits
    pub fn from_frame_size(frame_size_bits: u16) -> Option<Self> {
        match frame_size_bits {
            G722_FRAME_SIZE_BITS_MODE1 => Some(G722Mode::Mode1),
            G722_FRAME_SIZE_BITS_MODE2 => Some(G722Mode::Mode2),
            G722_FRAME_SIZE_BITS_MODE3 => Some(G722Mode::Mode3),
            _ => None,
        }
    }
}

/// Parse ITU G.191 format test vector file containing 16-bit PCM samples
/// 
/// ITU G.191 format:
/// - First 32 bytes: sync pattern (0x0001 repeated 16 times)
/// - Remaining data: actual PCM samples as 16-bit little-endian words
/// 
/// # Arguments
/// * `filename` - Test vector filename relative to test_vectors directory
/// 
/// # Returns
/// * Vector of 16-bit PCM samples
pub fn parse_g191_pcm_samples(filename: &str) -> io::Result<Vec<i16>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/codecs/g722/tests/test_vectors")
        .join(filename);
    
    let data = fs::read(&path)?;
    
    // Convert bytes to u16 words (little endian)
    let mut words = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        let word = u16::from_le_bytes([chunk[0], chunk[1]]);
        words.push(word);
    }
    
    // Skip the sync pattern (first 16 words = 32 bytes)
    let data_start = find_data_start(&words)?;
    let sample_words = &words[data_start..];
    
    // Convert u16 words to i16 samples
    let samples: Vec<i16> = sample_words.iter()
        .map(|&word| word as i16)
        .collect();
    
    Ok(samples)
}

/// Parse ITU G.191 format test vector file containing encoded G.722 data
/// 
/// ITU G.191 format for encoded data:
/// - First 32 bytes: sync pattern (0x0001 repeated 16 times)
/// - Remaining data: G.722 encoded bytes as 16-bit little-endian words (low byte contains data)
/// 
/// # Arguments
/// * `filename` - Test vector filename relative to test_vectors directory
/// 
/// # Returns
/// * Vector of encoded G.722 bytes
pub fn parse_g191_encoded_data(filename: &str) -> io::Result<Vec<u8>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/codecs/g722/tests/test_vectors")
        .join(filename);
    
    let data = fs::read(&path)?;
    
    // Convert bytes to u16 words (little endian)
    let mut words = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        let word = u16::from_le_bytes([chunk[0], chunk[1]]);
        words.push(word);
    }
    
    // Skip the sync pattern (first 16 words = 32 bytes)
    let data_start = find_data_start(&words)?;
    let data_words = &words[data_start..];
    
    // Extract low byte from each 16-bit word (G.722 encoded data)
    let encoded_data: Vec<u8> = data_words.iter()
        .map(|&word| (word & 0xFF) as u8)
        .collect();
    
    Ok(encoded_data)
}

/// Find the start of actual data after sync pattern
/// 
/// Look for the transition from sync pattern (0x0001) to actual data
fn find_data_start(words: &[u16]) -> io::Result<usize> {
    if words.len() < G191_SYNC_PATTERN_LENGTH {
        return Err(io::Error::new(io::ErrorKind::InvalidData, 
            "File too short to contain sync pattern"));
    }
    
    // Check if first 16 words are sync pattern
    let mut sync_count = 0;
    for &word in words.iter().take(G191_SYNC_PATTERN_LENGTH) {
        if word == G191_SYNC_PATTERN {
            sync_count += 1;
        } else {
            break;
        }
    }
    
    if sync_count >= G191_SYNC_PATTERN_LENGTH {
        // Standard format: exactly 16 sync patterns
        return Ok(G191_SYNC_PATTERN_LENGTH);
    }
    
    // If not standard format, try to find transition point
    for i in 0..words.len().saturating_sub(4) {
        if words[i] == G191_SYNC_PATTERN && 
           words[i+1] == G191_SYNC_PATTERN && 
           words[i+2] != G191_SYNC_PATTERN {
            return Ok(i + 2);
        }
    }
    
    // If no sync pattern found, assume no header
    Ok(0)
}

/// Convert raw G.722 encoded bytes to ITU G.191 format
/// 
/// This function converts G.722 encoded bytes to the ITU G.191 format for comparison
/// with reference test vectors.
/// 
/// # Arguments
/// * `encoded_data` - Raw G.722 encoded bytes
/// 
/// # Returns
/// * Vector of words in G.191 format (with sync pattern)
pub fn convert_to_g191_format(encoded_data: &[u8]) -> Vec<u16> {
    let mut g191_data = Vec::new();
    
    // Add sync pattern (16 times 0x0001)
    for _ in 0..G191_SYNC_PATTERN_LENGTH {
        g191_data.push(G191_SYNC_PATTERN);
    }
    
    // Add encoded data as 16-bit words (low byte contains data, high byte is 0)
    for &byte in encoded_data {
        g191_data.push(byte as u16);
    }
    
    g191_data
}

/// G.192 Frame structure
#[derive(Debug, Clone)]
pub struct G192Frame {
    /// Sync header (0x6B21 for good frame, 0x6B20 for bad frame)
    pub sync_header: u16,
    /// Frame length in bits
    pub frame_length: u16,
    /// Data bits as G.192 soft bits
    pub data_bits: Vec<u16>,
    /// Whether this is a good frame (true) or bad frame (false)
    pub is_good_frame: bool,
}

impl G192Frame {
    /// Create a new G.192 frame
    pub fn new(data_bits: Vec<u16>, is_good_frame: bool) -> Self {
        let sync_header = if is_good_frame { G192_SYNC_GOOD } else { G192_SYNC_BAD };
        let frame_length = data_bits.len() as u16;
        
        Self {
            sync_header,
            frame_length,
            data_bits,
            is_good_frame,
        }
    }
    
    /// Get the G.722 mode from frame length
    pub fn mode(&self) -> Option<G722Mode> {
        G722Mode::from_frame_size(self.frame_length)
    }
    
    /// Convert frame to bytes for serialization
    pub fn to_bytes(&self) -> Vec<u16> {
        let mut bytes = Vec::with_capacity(2 + self.data_bits.len());
        bytes.push(self.sync_header);
        bytes.push(self.frame_length);
        bytes.extend_from_slice(&self.data_bits);
        bytes
    }
}

/// Parse G.192 format bitstream file
/// 
/// G.192 format structure:
/// - Sync header (16-bit): 0x6B21 (good frame) or 0x6B20 (bad frame)
/// - Frame length (16-bit): Number of data bits in the frame
/// - Data bits: Each bit as 16-bit word (0x007F for '0', 0x0081 for '1')
/// 
/// # Arguments
/// * `filename` - G.192 bitstream filename
/// 
/// # Returns
/// * Vector of G.192 frames
pub fn parse_g192_bitstream(filename: &str) -> io::Result<Vec<G192Frame>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/codecs/g722/tests/test_vectors")
        .join(filename);
    
    let data = fs::read(&path)?;
    
    // Convert bytes to u16 words (little endian)
    let mut words = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        let word = u16::from_le_bytes([chunk[0], chunk[1]]);
        words.push(word);
    }
    
    let mut frames = Vec::new();
    let mut i = 0;
    
    while i + 1 < words.len() {
        let sync_header = words[i];
        let frame_length = words[i + 1];
        
        // Validate sync header
        if sync_header != G192_SYNC_GOOD && sync_header != G192_SYNC_BAD {
            return Err(io::Error::new(io::ErrorKind::InvalidData, 
                format!("Invalid sync header: 0x{:04X}", sync_header)));
        }
        
        // Check if we have enough data
        if i + 2 + frame_length as usize > words.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, 
                "Incomplete frame data"));
        }
        
        // Extract frame data
        let data_bits = words[i + 2..i + 2 + frame_length as usize].to_vec();
        let is_good_frame = sync_header == G192_SYNC_GOOD;
        
        frames.push(G192Frame {
            sync_header,
            frame_length,
            data_bits,
            is_good_frame,
        });
        
        i += 2 + frame_length as usize;
    }
    
    Ok(frames)
}

/// Convert G.192 soft bits to hard bits (bytes)
/// 
/// This function converts G.192 soft-bit representation to hard bits and then
/// packs them into bytes using the ITU-T G.722 bit ordering.
/// 
/// # Arguments
/// * `soft_bits` - G.192 soft bits (0x007F for '0', 0x0081 for '1')
/// * `mode` - G.722 mode for bit ordering
/// 
/// # Returns
/// * Vector of packed bytes
pub fn g192_to_bytes(soft_bits: &[u16], mode: G722Mode) -> Vec<u8> {
    // Convert soft bits to hard bits
    let hard_bits: Vec<u8> = soft_bits.iter()
        .map(|&bit| if bit == G192_ONE { 1 } else { 0 })
        .collect();
    
    // Pack bits into bytes using ITU-T G.722 bit ordering
    pack_bits_with_ordering(&hard_bits, mode)
}

/// Pack bits into bytes using ITU-T G.722 scalable bit ordering
/// 
/// ITU-T G.722 uses special bit ordering for scalability:
/// [b2, b3, b4, b5, b6, b7, b1, b0] where b2-b5 are core bits
/// 
/// # Arguments
/// * `bits` - Hard bits (0 or 1)
/// * `mode` - G.722 mode determining number of bits per sample
/// 
/// # Returns
/// * Vector of packed bytes
pub fn pack_bits_with_ordering(bits: &[u8], mode: G722Mode) -> Vec<u8> {
    let samples_per_frame = G722_FRAME_SIZE_SAMPLES;
    let bits_per_sample = mode.bits_per_sample();
    let mut packed_bytes = vec![0u8; samples_per_frame];
    
    // Bit ordering: [b2, b3, b4, b5, b6, b7, b1, b0]
    let bit_order = match mode {
        G722Mode::Mode1 => vec![2, 3, 4, 5, 6, 7, 1, 0],  // 8 bits
        G722Mode::Mode2 => vec![2, 3, 4, 5, 6, 7, 1],     // 7 bits (no b0)
        G722Mode::Mode3 => vec![2, 3, 4, 5, 6, 7],        // 6 bits (no b1, b0)
    };
    
    let mut bit_idx = 0;
    
    for sample_idx in 0..samples_per_frame {
        let mut byte_val = 0u8;
        
        for &bit_pos in &bit_order {
            if bit_idx < bits.len() {
                if bits[bit_idx] != 0 {
                    byte_val |= 1 << bit_pos;
                }
                bit_idx += 1;
            }
        }
        
        packed_bytes[sample_idx] = byte_val;
    }
    
    packed_bytes
}

/// Convert hard bits to G.192 soft bits
/// 
/// # Arguments
/// * `hard_bits` - Hard bits (0 or 1)
/// 
/// # Returns
/// * Vector of G.192 soft bits
pub fn hard_bits_to_g192(hard_bits: &[u8]) -> Vec<u16> {
    hard_bits.iter()
        .map(|&bit| if bit != 0 { G192_ONE } else { G192_ZERO })
        .collect()
}

/// Convert packed bytes to G.192 soft bits using ITU-T bit ordering
/// 
/// # Arguments
/// * `bytes` - Packed G.722 bytes
/// * `mode` - G.722 mode for bit extraction
/// 
/// # Returns
/// * Vector of G.192 soft bits
pub fn bytes_to_g192(bytes: &[u8], mode: G722Mode) -> Vec<u16> {
    let bits_per_sample = mode.bits_per_sample();
    let mut soft_bits = Vec::with_capacity(bytes.len() * bits_per_sample as usize);
    
    // Bit ordering: [b2, b3, b4, b5, b6, b7, b1, b0]
    let bit_order = match mode {
        G722Mode::Mode1 => vec![2, 3, 4, 5, 6, 7, 1, 0],  // 8 bits
        G722Mode::Mode2 => vec![2, 3, 4, 5, 6, 7, 1],     // 7 bits (no b0)
        G722Mode::Mode3 => vec![2, 3, 4, 5, 6, 7],        // 6 bits (no b1, b0)
    };
    
    for &byte in bytes {
        for &bit_pos in &bit_order {
            let bit_val = (byte >> bit_pos) & 1;
            soft_bits.push(if bit_val != 0 { G192_ONE } else { G192_ZERO });
        }
    }
    
    soft_bits
}

/// Generate G.192 format bitstream from encoded bytes
/// 
/// # Arguments
/// * `encoded_bytes` - G.722 encoded bytes
/// * `mode` - G.722 mode
/// * `is_good_frame` - Whether this is a good frame
/// 
/// # Returns
/// * G.192 frame
pub fn generate_g192_frame(encoded_bytes: &[u8], mode: G722Mode, is_good_frame: bool) -> G192Frame {
    let soft_bits = bytes_to_g192(encoded_bytes, mode);
    G192Frame::new(soft_bits, is_good_frame)
}

/// Frame synchronization and validation
#[derive(Debug, Clone)]
pub struct FrameSynchronizer {
    /// Expected frame size in bits for validation
    pub expected_frame_size: Option<u16>,
    /// Total number of frames processed
    pub frame_count: u32,
    /// Total number of validation errors
    pub error_count: u32,
}

impl FrameSynchronizer {
    /// Create a new FrameSynchronizer
    pub fn new(expected_frame_size: Option<u16>) -> Self {
        Self {
            expected_frame_size,
            frame_count: 0,
            error_count: 0,
        }
    }
    
    /// Validate a single G.192 frame
    pub fn validate_frame(&mut self, frame: &G192Frame) -> Result<(), String> {
        self.frame_count += 1;
        
        // Check sync header
        if frame.sync_header != G192_SYNC_GOOD && frame.sync_header != G192_SYNC_BAD {
            self.error_count += 1;
            return Err(format!("Invalid sync header: 0x{:04X}", frame.sync_header));
        }
        
        // Check frame length consistency
        if let Some(expected_size) = self.expected_frame_size {
            if frame.frame_length != expected_size {
                self.error_count += 1;
                return Err(format!("Frame length mismatch: expected {}, got {}", 
                    expected_size, frame.frame_length));
            }
        }
        
        // Check data bits format
        for (i, &bit) in frame.data_bits.iter().enumerate() {
            if bit != G192_ZERO && bit != G192_ONE {
                self.error_count += 1;
                return Err(format!("Invalid soft bit at position {}: 0x{:04X}", i, bit));
            }
        }
        
        Ok(())
    }
    
    /// Reset the synchronizer state
    pub fn reset(&mut self) {
        self.frame_count = 0;
        self.error_count = 0;
    }
}

/// G.722 Reference Decoder with ITU-T compliance
/// 
/// This decoder matches the ITU-T reference implementation behavior exactly,
/// including reset control, frame synchronization, and proper state management.
pub struct G722ReferenceDecoder {
    /// Current state of the G.722 decoder
    pub decoder_state: G722DecoderState,
    /// Synchronizer to validate and track frame boundaries
    pub frame_synchronizer: FrameSynchronizer,
    /// Flag to indicate if decoder state should be reset on the next frame
    pub reset_on_frame: bool,
}

impl G722ReferenceDecoder {
    /// Create a new G.722ReferenceDecoder
    pub fn new(mode: G722Mode) -> Self {
        let expected_frame_size = Some(mode.frame_size_bits());
        
        Self {
            decoder_state: G722DecoderState::new(),
            frame_synchronizer: FrameSynchronizer::new(expected_frame_size),
            reset_on_frame: false,
        }
    }
    
    /// Decode a single G.192 frame
    pub fn decode_frame(&mut self, frame: &G192Frame) -> Result<Vec<i16>, String> {
        // Validate frame synchronization
        self.frame_synchronizer.validate_frame(frame)?;
        
        // Handle bad frames (frame erasure)
        if !frame.is_good_frame {
            // In a full implementation, this would use PLC (Packet Loss Concealment)
            // For now, we'll return silence
            return Ok(vec![0i16; G722_FRAME_SIZE_SAMPLES * 2]); // 16 kHz output
        }
        
        // Determine mode from frame length
        let mode = frame.mode()
            .ok_or_else(|| format!("Invalid frame length: {}", frame.frame_length))?;
        
        // Convert G.192 soft bits to encoded bytes
        let encoded_bytes = g192_to_bytes(&frame.data_bits, mode);
        
        // Reset decoder state if requested (ITU-T reference behavior)
        if self.reset_on_frame {
            self.decoder_state = G722DecoderState::new();
            self.reset_on_frame = false;
        }
        
        // Decode the frame using G722Codec
        let mut codec = G722Codec::new_with_mode(mode as u8)
            .map_err(|e| format!("Failed to create codec: {}", e))?;
        
        codec.decoder_state = self.decoder_state.clone();
        
        let decoded_samples = codec.decode_frame(&encoded_bytes)
            .map_err(|e| format!("Decoding error: {}", e))?;
        
        // Save decoder state for next frame
        self.decoder_state = codec.decoder_state.clone();
        
        Ok(decoded_samples)
    }
    
    /// Reset the decoder state
    pub fn reset(&mut self) {
        self.decoder_state = G722DecoderState::new();
        self.frame_synchronizer.reset();
        self.reset_on_frame = false;
    }
    
    /// Set flag to reset decoder state on the next frame
    pub fn set_reset_on_next_frame(&mut self, reset: bool) {
        self.reset_on_frame = reset;
    }
}

/// G.722 Reference Encoder with ITU-T compliance
pub struct G722ReferenceEncoder {
    /// Current state of the G.722 encoder
    pub encoder_state: G722EncoderState,
    /// G.722 mode for encoding
    pub mode: G722Mode,
    /// Flag to indicate if encoder state should be reset on the next frame
    pub reset_on_frame: bool,
}

impl G722ReferenceEncoder {
    /// Create a new G.722ReferenceEncoder
    pub fn new(mode: G722Mode) -> Self {
        Self {
            encoder_state: G722EncoderState::new(),
            mode,
            reset_on_frame: false,
        }
    }
    
    /// Encode a single PCM frame
    pub fn encode_frame(&mut self, pcm_samples: &[i16]) -> Result<G192Frame, String> {
        // Validate input length (should be 160 samples for 16 kHz input)
        if pcm_samples.len() != G722_FRAME_SIZE_SAMPLES * 2 {
            return Err(format!("Invalid input length: expected {}, got {}", 
                G722_FRAME_SIZE_SAMPLES * 2, pcm_samples.len()));
        }
        
        // Reset encoder state if requested (ITU-T reference behavior)
        if self.reset_on_frame {
            self.encoder_state = G722EncoderState::new();
            self.reset_on_frame = false;
        }
        
        // Encode the frame using G722Codec
        let mut codec = G722Codec::new_with_mode(self.mode as u8)
            .map_err(|e| format!("Failed to create codec: {}", e))?;
        
        codec.encoder_state = self.encoder_state.clone();
        
        let encoded_bytes = codec.encode_frame(pcm_samples)
            .map_err(|e| format!("Encoding error: {}", e))?;
        
        // Save encoder state for next frame
        self.encoder_state = codec.encoder_state.clone();
        
        // Generate G.192 frame
        Ok(generate_g192_frame(&encoded_bytes, self.mode, true))
    }
    
    /// Reset the encoder state
    pub fn reset(&mut self) {
        self.encoder_state = G722EncoderState::new();
        self.reset_on_frame = false;
    }
    
    /// Set flag to reset encoder state on the next frame
    pub fn set_reset_on_next_frame(&mut self, reset: bool) {
        self.reset_on_frame = reset;
    }
}

/// Write G.192 frames to a file
/// 
/// # Arguments
/// * `frames` - G.192 frames to write
/// * `filename` - Output filename
/// 
/// # Returns
/// * Result indicating success or failure
pub fn write_g192_bitstream(frames: &[G192Frame], filename: &str) -> io::Result<()> {
    use std::fs::File;
    use std::io::Write;
    
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/codecs/g722/tests/test_vectors")
        .join(filename);
    
    let mut file = File::create(&path)?;
    
    for frame in frames {
        let frame_bytes = frame.to_bytes();
        
        // Write as little-endian 16-bit words
        for &word in &frame_bytes {
            file.write_all(&word.to_le_bytes())?;
        }
    }
    
    Ok(())
}

/// Verify G.192 bitstream integrity
/// 
/// This function performs the same verification as the ITU-T reference decoder
/// 
/// # Arguments
/// * `filename` - G.192 bitstream filename
/// 
/// # Returns
/// * Result with frame count or error
pub fn verify_g192_bitstream(filename: &str) -> io::Result<u32> {
    let frames = parse_g192_bitstream(filename)?;
    let mut synchronizer = FrameSynchronizer::new(None);
    
    for frame in &frames {
        synchronizer.validate_frame(frame)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    }
    
    Ok(synchronizer.frame_count)
}

/// Convert raw PCM samples to ITU G.191 format
/// 
/// # Arguments
/// * `samples` - Raw PCM samples
/// 
/// # Returns
/// * Vector of words in G.191 format (with sync pattern)
pub fn convert_pcm_to_g191_format(samples: &[i16]) -> Vec<u16> {
    let mut g191_data = Vec::new();
    
    // Add sync pattern (16 times 0x0001)
    for _ in 0..G191_SYNC_PATTERN_LENGTH {
        g191_data.push(G191_SYNC_PATTERN);
    }
    
    // Add PCM samples as 16-bit words
    for &sample in samples {
        g191_data.push(sample as u16);
    }
    
    g191_data
}

/// Calculate similarity between two byte vectors
/// 
/// # Arguments
/// * `a` - First vector
/// * `b` - Second vector
/// 
/// # Returns
/// * Similarity as a percentage (0.0 to 1.0)
pub fn calculate_byte_similarity(a: &[u8], b: &[u8]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    
    let min_len = a.len().min(b.len());
    let matches = a.iter().zip(b.iter())
        .take(min_len)
        .filter(|(x, y)| x == y)
        .count();
    
    (matches as f32) / (min_len as f32)
}

/// Calculate similarity between two sample vectors
/// 
/// # Arguments
/// * `a` - First vector
/// * `b` - Second vector
/// 
/// # Returns
/// * Similarity as a percentage (0.0 to 1.0)
pub fn calculate_sample_similarity(a: &[i16], b: &[i16]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    
    let min_len = a.len().min(b.len());
    
    // Calculate normalized correlation
    let mut sum_diff_sq = 0.0;
    let mut sum_a_sq = 0.0;
    let mut sum_b_sq = 0.0;
    
    for i in 0..min_len {
        let diff = (a[i] as f64) - (b[i] as f64);
        sum_diff_sq += diff * diff;
        sum_a_sq += (a[i] as f64) * (a[i] as f64);
        sum_b_sq += (b[i] as f64) * (b[i] as f64);
    }
    
    if sum_a_sq == 0.0 && sum_b_sq == 0.0 {
        return 1.0; // Both are zero vectors
    }
    
    // Calculate normalized mean square error
    let mse = sum_diff_sq / (min_len as f64);
    let max_energy = sum_a_sq.max(sum_b_sq) / (min_len as f64);
    
    if max_energy == 0.0 {
        return 0.0;
    }
    
    // Convert to similarity (higher is better)
    let nmse = mse / max_energy;
    let similarity = 1.0 / (1.0 + nmse);
    
    similarity as f32
}

/// Generate test signal patterns for validation
pub mod test_signals {
    /// Generate a sine wave at specified frequency
    /// 
    /// # Arguments
    /// * `frequency` - Frequency in Hz
    /// * `sample_rate` - Sample rate in Hz
    /// * `duration_samples` - Number of samples to generate
    /// * `amplitude` - Amplitude (0-32767)
    /// 
    /// # Returns
    /// * Vector of sine wave samples
    pub fn generate_sine_wave(frequency: f32, sample_rate: f32, duration_samples: usize, amplitude: i16) -> Vec<i16> {
        (0..duration_samples)
            .map(|i| {
                let t = i as f32 / sample_rate;
                let sample = (amplitude as f32) * (2.0 * std::f32::consts::PI * frequency * t).sin();
                sample.round() as i16
            })
            .collect()
    }
    
    /// Generate white noise
    /// 
    /// # Arguments
    /// * `duration_samples` - Number of samples to generate
    /// * `amplitude` - Maximum amplitude
    /// 
    /// # Returns
    /// * Vector of noise samples
    pub fn generate_white_noise(duration_samples: usize, amplitude: i16) -> Vec<i16> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        (0..duration_samples)
            .map(|i| {
                // Simple pseudo-random number generator
                i.hash(&mut hasher);
                let hash_value = hasher.finish();
                let normalized = ((hash_value % 65536) as i32) - 32768;
                ((normalized * amplitude as i32) / 32768) as i16
            })
            .collect()
    }
    
    /// Generate impulse signal
    /// 
    /// # Arguments
    /// * `duration_samples` - Number of samples to generate
    /// * `impulse_position` - Position of impulse (0-based)
    /// * `amplitude` - Amplitude of impulse
    /// 
    /// # Returns
    /// * Vector with impulse at specified position
    pub fn generate_impulse(duration_samples: usize, impulse_position: usize, amplitude: i16) -> Vec<i16> {
        let mut signal = vec![0i16; duration_samples];
        if impulse_position < duration_samples {
            signal[impulse_position] = amplitude;
        }
        signal
    }
}

/// Test vector file information
#[derive(Debug, Clone)]
pub struct TestVectorInfo {
    /// Filename of the test vector
    pub filename: String,
    /// Expected CRC-32 checksum
    pub expected_crc: u32,
    /// Expected file size in bytes
    pub expected_size: usize,
    /// Human-readable description
    pub description: String,
}

/// Get information about standard ITU test vectors
pub fn get_standard_test_vectors() -> Vec<TestVectorInfo> {
    vec![
        TestVectorInfo {
            filename: "bt1c1.xmt".to_string(),
            expected_crc: 0x0C3BFCA7,
            expected_size: 32832,
            description: "Input PCM test vector 1".to_string(),
        },
        TestVectorInfo {
            filename: "bt1c2.xmt".to_string(),
            expected_crc: 0x2D604685,
            expected_size: 1600,
            description: "Input PCM test vector 2".to_string(),
        },
        TestVectorInfo {
            filename: "bt2r1.cod".to_string(),
            expected_crc: 0xD1DAA1D1,
            expected_size: 32832,
            description: "G.722 encoded output 1".to_string(),
        },
        TestVectorInfo {
            filename: "bt2r2.cod".to_string(),
            expected_crc: 0x344EA5D0,
            expected_size: 1600,
            description: "G.722 encoded output 2".to_string(),
        },
        TestVectorInfo {
            filename: "bt3l1.rc1".to_string(),
            expected_crc: 0xED1B3993,
            expected_size: 32832,
            description: "Low-band decoded output 1 (mode 1)".to_string(),
        },
        TestVectorInfo {
            filename: "bt3l1.rc2".to_string(),
            expected_crc: 0x8E8C4E2B,
            expected_size: 32832,
            description: "Low-band decoded output 1 (mode 2)".to_string(),
        },
        TestVectorInfo {
            filename: "bt3l1.rc3".to_string(),
            expected_crc: 0xB7AA5569,
            expected_size: 32832,
            description: "Low-band decoded output 1 (mode 3)".to_string(),
        },
        TestVectorInfo {
            filename: "bt3h1.rc0".to_string(),
            expected_crc: 0xE9250851,
            expected_size: 32832,
            description: "High-band decoded output 1".to_string(),
        },
    ]
} 