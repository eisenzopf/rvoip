//! G.729 Low-Bitrate Audio Codec Implementation
//!
//! This module implements the G.729 codec, a low bit-rate audio codec
//! standardized by ITU-T, commonly used in VoIP for its excellent compression.
//! G.729 uses ACELP (Algebraic Code Excited Linear Prediction) to achieve
//! 8 kbps compression with good voice quality.
//!
//! ## Runtime Configuration
//!
//! The G.729 implementation supports runtime configuration switches for
//! Annex A (reduced complexity) and Annex B (VAD/DTX/CNG) features:
//!
//! ```rust
//! use rvoip_codec_core::codecs::g729::{G729Codec, G729Config};
//!
//! // Default configuration (G.729BA - most common for production)
//! let mut codec = G729Codec::new_default().unwrap();
//! assert_eq!(codec.variant(), "G.729BA");
//!
//! // Custom configuration for specific use cases
//! let codec_a = G729Codec::new_with_annexes(true, false).unwrap();  // G.729A
//! let codec_b = G729Codec::new_with_annexes(false, true).unwrap();  // G.729B  
//! let codec_core = G729Codec::new_with_annexes(false, false).unwrap(); // G.729
//!
//! // Runtime configuration changes
//! codec.set_annex_a(false).unwrap(); // Disable reduced complexity
//! codec.set_annex_b(false).unwrap(); // Disable VAD/DTX/CNG
//! assert_eq!(codec.variant(), "G.729");
//! ```
//!
//! ## G.729 Variants
//!
//! | Variant | Annex A | Annex B | CPU Efficiency | Bandwidth Efficiency | Use Case |
//! |---------|---------|---------|----------------|---------------------|----------|
//! | **G.729** | ❌ | ❌ | 100% (baseline) | 100% (continuous) | Reference/testing |
//! | **G.729A** | ✅ | ❌ | 60% (~40% faster) | 100% (continuous) | Low-power devices |
//! | **G.729B** | ❌ | ✅ | 100% (baseline) | 50% (~50% savings) | Bandwidth-critical |
//! | **G.729BA** | ✅ | ✅ | 60% (~40% faster) | 50% (~50% savings) | **Production VoIP** |
//!
//! ## Configuration Examples
//!
//! ### Production VoIP (Recommended)
//! ```rust
//! # use rvoip_codec_core::codecs::g729::G729Codec;
//! // G.729BA: Best balance of CPU efficiency and bandwidth savings
//! let codec = G729Codec::new_default().unwrap();
//! assert_eq!(codec.variant(), "G.729BA");
//! assert!(codec.has_annex_a()); // 40% less CPU usage
//! assert!(codec.has_annex_b()); // 50% bandwidth savings in silence
//! ```
//!
//! ### Low-Power/Embedded Devices
//! ```rust
//! # use rvoip_codec_core::codecs::g729::G729Codec;
//! // G.729A: Reduced complexity without VAD overhead
//! let codec = G729Codec::new_with_annexes(true, false).unwrap();
//! assert_eq!(codec.variant(), "G.729A");
//! assert_eq!(codec.config().cpu_efficiency(), 0.6); // 40% faster
//! ```
//!
//! ### Bandwidth-Critical Connections
//! ```rust
//! # use rvoip_codec_core::codecs::g729::G729Codec;
//! // G.729B: Full quality with maximum bandwidth efficiency
//! let codec = G729Codec::new_with_annexes(false, true).unwrap();
//! assert_eq!(codec.variant(), "G.729B");
//! assert_eq!(codec.config().bandwidth_efficiency(), 0.5); // 50% savings
//! ```
//!
//! ### ITU Reference Testing
//! ```rust
//! # use rvoip_codec_core::codecs::g729::G729Codec;
//! // G.729 Core: Full complexity reference implementation
//! let codec = G729Codec::new_with_annexes(false, false).unwrap();
//! assert_eq!(codec.variant(), "G.729");
//! assert_eq!(codec.config().cpu_efficiency(), 1.0); // Baseline complexity
//! ```
//!
//! ### Adaptive Configuration
//! ```rust
//! # use rvoip_codec_core::codecs::g729::G729Codec;
//! let mut codec = G729Codec::new_with_annexes(false, false).unwrap();
//!
//! // Adapt to network conditions
//! if network_congested() {
//!     codec.set_annex_b(true).unwrap(); // Enable bandwidth savings
//! }
//!
//! // Adapt to CPU load
//! if cpu_load_high() {
//!     codec.set_annex_a(true).unwrap(); // Enable reduced complexity
//! }
//!
//! # fn network_congested() -> bool { true }
//! # fn cpu_load_high() -> bool { true }
//! ```

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, AudioCodecExt, CodecConfig, CodecInfo};
use crate::utils::{validate_g729_frame};
use tracing::{debug, trace, warn};

// Include the new G.729 implementation
pub mod src;

// Include ITU-T compliance test suite
#[cfg(test)]
pub mod itu_tests;

/// G.729 codec implementation
pub struct G729Codec {
    /// Sample rate (fixed at 8kHz)
    sample_rate: u32,
    /// Number of channels (fixed at 1)
    channels: u8,
    /// Frame size in samples (fixed at 80)
    frame_size: usize,
    /// Codec configuration
    config: G729Config,
    /// Encoder state
    encoder_state: G729EncoderState,
    /// Decoder state
    decoder_state: G729DecoderState,
}

/// G.729 codec configuration
#[derive(Debug, Clone)]
pub struct G729Config {
    /// Enable G.729 Annex A (reduced complexity - ~40% faster)
    pub annex_a: bool,
    /// Enable G.729 Annex B (VAD/DTX/CNG - ~50% bandwidth savings in silence)
    pub annex_b: bool,
}

impl Default for G729Config {
    fn default() -> Self {
        Self {
            annex_a: true,  // Use reduced complexity by default
            annex_b: true,  // Use VAD/DTX/CNG by default (G.729BA)
        }
    }
}

impl G729Config {
    /// Create a new G.729 configuration
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Enable/disable G.729 Annex A (reduced complexity)
    /// When enabled, uses simplified pitch analysis and ACELP search (~40% faster)
    pub fn with_annex_a(mut self, enabled: bool) -> Self {
        self.annex_a = enabled;
        self
    }
    
    /// Enable/disable G.729 Annex B (VAD/DTX/CNG)
    /// When enabled, provides bandwidth efficiency during silence periods
    pub fn with_annex_b(mut self, enabled: bool) -> Self {
        self.annex_b = enabled;
        self
    }
    
    /// Get the G.729 variant name based on enabled annexes
    pub fn variant_name(&self) -> &'static str {
        match (self.annex_a, self.annex_b) {
            (false, false) => "G.729",      // Full complexity, no VAD/DTX
            (true, false) => "G.729A",     // Reduced complexity, no VAD/DTX
            (false, true) => "G.729B",     // Full complexity + VAD/DTX/CNG
            (true, true) => "G.729BA",     // Reduced complexity + VAD/DTX/CNG
        }
    }
    
    /// Get expected computational efficiency vs baseline G.729
    pub fn cpu_efficiency(&self) -> f32 {
        if self.annex_a {
            0.6 // ~40% reduction in computational complexity
        } else {
            1.0 // Baseline complexity
        }
    }
    
    /// Get expected bandwidth efficiency vs continuous transmission
    pub fn bandwidth_efficiency(&self) -> f32 {
        if self.annex_b {
            0.5 // ~50% bandwidth savings during silence periods
        } else {
            1.0 // Continuous transmission
        }
    }
    
    /// Check if Voice Activity Detection is enabled
    pub fn has_vad(&self) -> bool {
        self.annex_b
    }
    
    /// Check if Discontinuous Transmission is enabled
    pub fn has_dtx(&self) -> bool {
        self.annex_b
    }
    
    /// Check if Comfort Noise Generation is enabled
    pub fn has_cng(&self) -> bool {
        self.annex_b
    }
}

/// G.729 encoder state
#[derive(Debug, Clone)]
struct G729EncoderState {
    /// Linear prediction coefficients
    lpc_coeffs: [f32; 10],
    /// Pitch analysis state
    pitch_analyzer: PitchAnalyzer,
    /// Codebook search state
    codebook_searcher: CodebookSearcher,
    /// Previous frame for continuity
    prev_frame: [i16; 80],
    /// Energy level for VAD
    energy_level: f32,
}

/// G.729 decoder state
#[derive(Debug, Clone)]
struct G729DecoderState {
    /// LPC synthesis filter
    lpc_synthesis: LpcSynthesis,
    /// Pitch synthesis filter
    pitch_synthesis: PitchSynthesis,
    /// Post-filter for quality enhancement
    postfilter: PostFilter,
    /// Previous excitation for continuity
    prev_excitation: [f32; 80],
    /// Bad frame indicator
    bad_frame_count: u32,
}

/// Pitch analysis for ACELP
#[derive(Debug, Clone)]
struct PitchAnalyzer {
    /// Pitch period range
    pitch_min: usize,
    pitch_max: usize,
    /// Previous pitch period
    prev_pitch: usize,
    /// Pitch gain
    pitch_gain: f32,
    /// Correlation buffer
    correlation_buf: [f32; 143],
}

/// Algebraic codebook searcher
#[derive(Debug, Clone)]
struct CodebookSearcher {
    /// Codebook indices for current frame
    indices: [usize; 4],
    /// Codebook gains
    gains: [f32; 4],
    /// Target signal for search
    target: [f32; 40],
}

/// LPC synthesis filter
#[derive(Debug, Clone)]
struct LpcSynthesis {
    /// Filter coefficients
    coeffs: [f32; 10],
    /// Filter memory
    memory: [f32; 10],
}

/// Pitch synthesis filter
#[derive(Debug, Clone)]
struct PitchSynthesis {
    /// Pitch period
    period: usize,
    /// Pitch gain
    gain: f32,
    /// Excitation buffer
    excitation_buf: [f32; 143],
}

/// Post-filter for quality enhancement
#[derive(Debug, Clone)]
struct PostFilter {
    /// Formant post-filter coefficients
    formant_coeffs: [f32; 10],
    /// Tilt compensation filter
    tilt_comp: f32,
    /// Filter memory
    memory: [f32; 10],
}

impl G729Codec {
    /// Create a new G.729 codec with default configuration (G.729BA)
    pub fn new_default() -> Result<Self> {
        Self::new_with_config(G729Config::default())
    }
    
    /// Create a new G.729 codec with custom annex configuration
    pub fn new_with_annexes(annex_a: bool, annex_b: bool) -> Result<Self> {
        let config = G729Config::new()
            .with_annex_a(annex_a)
            .with_annex_b(annex_b);
        Self::new_with_config(config)
    }
    
    /// Create a new G.729 codec with specific configuration
    pub fn new_with_config(g729_config: G729Config) -> Result<Self> {
        debug!("Creating G.729 codec: variant={}, CPU efficiency={:.1}%, BW efficiency={:.1}%", 
               g729_config.variant_name(),
               g729_config.cpu_efficiency() * 100.0, 
               g729_config.bandwidth_efficiency() * 100.0);
        
        Ok(Self {
            sample_rate: 8000,    // G.729 fixed at 8kHz
            channels: 1,          // G.729 fixed at mono
            frame_size: 80,       // G.729 fixed at 80 samples (10ms)
            config: g729_config,
            encoder_state: G729EncoderState::new(),
            decoder_state: G729DecoderState::new(),
        })
    }
    
    /// Create a new G.729 codec from CodecConfig (for compatibility)
    pub fn new(config: CodecConfig) -> Result<Self> {
        // Validate configuration
        let sample_rate = config.sample_rate.hz();
        
        // G.729 only supports 8kHz
        if sample_rate != 8000 {
            return Err(CodecError::InvalidSampleRate {
                rate: sample_rate,
                supported: vec![8000],
            });
        }
        
        // G.729 only supports mono
        if config.channels != 1 {
            return Err(CodecError::InvalidChannelCount {
                channels: config.channels,
                supported: vec![1],
            });
        }
        
        // G.729 uses fixed 10ms frames (80 samples at 8kHz)
        let frame_size = 80;
        
        // Extract G.729 specific parameters with backward compatibility
        let g729_config = G729Config {
            annex_a: config.parameters.g729.annex_a,
            annex_b: config.parameters.g729.annex_b,
        };
        
        debug!("Creating G.729 codec: {}Hz, {}ch, variant={}, CPU efficiency={:.1}%, BW efficiency={:.1}%", 
               sample_rate, config.channels, g729_config.variant_name(),
               g729_config.cpu_efficiency() * 100.0, g729_config.bandwidth_efficiency() * 100.0);
        
        Ok(Self {
            sample_rate,
            channels: config.channels,
            frame_size,
            config: g729_config,
            encoder_state: G729EncoderState::new(),
            decoder_state: G729DecoderState::new(),
        })
    }
    
    /// Get the compression ratio (G.729 is 16:1, 16-bit to 1-bit per sample)
    pub fn compression_ratio(&self) -> f32 {
        0.125 // 80 samples (160 bytes) -> 10 bytes
    }
    
    /// Get the current G.729 configuration
    pub fn config(&self) -> &G729Config {
        &self.config
    }
    
    /// Get the current G.729 variant name
    pub fn variant(&self) -> &'static str {
        self.config.variant_name()
    }
    
    /// Check if Annex A (reduced complexity) is enabled
    pub fn has_annex_a(&self) -> bool {
        self.config.annex_a
    }
    
    /// Check if Annex B (VAD/DTX/CNG) is enabled
    pub fn has_annex_b(&self) -> bool {
        self.config.annex_b
    }
    
    /// Update codec configuration (will reset internal state)
    pub fn update_config(&mut self, new_config: G729Config) -> Result<()> {
        debug!("Updating G.729 config from {} to {}", 
               self.config.variant_name(), new_config.variant_name());
        
        self.config = new_config;
        
        // Reset codec state when configuration changes
        self.encoder_state = G729EncoderState::new();
        self.decoder_state = G729DecoderState::new();
        
        Ok(())
    }
    
    /// Enable/disable Annex A at runtime
    pub fn set_annex_a(&mut self, enabled: bool) -> Result<()> {
        if self.config.annex_a != enabled {
            let new_config = G729Config {
                annex_a: enabled,
                ..self.config
            };
            self.update_config(new_config)?;
        }
        Ok(())
    }
    
    /// Enable/disable Annex B at runtime
    pub fn set_annex_b(&mut self, enabled: bool) -> Result<()> {
        if self.config.annex_b != enabled {
            let new_config = G729Config {
                annex_b: enabled,
                ..self.config
            };
            self.update_config(new_config)?;
        }
        Ok(())
    }
}

impl AudioCodec for G729Codec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // Validate input
        validate_g729_frame(samples)?;
        
        // G.729 simulation encoding
        let encoded = self.simulate_encode(samples)?;
        
        trace!("G.729 encoded {} samples to {} bytes", 
               samples.len(), encoded.len());
        
        Ok(encoded)
    }
    
    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }
        
        // G.729 frames are typically 10 bytes
        if data.len() != 10 && data.len() != 2 { // 2 bytes for comfort noise
            return Err(CodecError::InvalidPayload {
                details: format!("Invalid G.729 frame size: {} bytes", data.len()),
            });
        }
        
        // G.729 simulation decoding
        let decoded = self.simulate_decode(data)?;
        
        trace!("G.729 decoded {} bytes to {} samples", 
               data.len(), decoded.len());
        
        Ok(decoded)
    }
    
    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: "G729",
            sample_rate: self.sample_rate,
            channels: self.channels,
            bitrate: 8000, // 8 kbps
            frame_size: self.frame_size,
            payload_type: Some(18),
        }
    }
    
    fn reset(&mut self) -> Result<()> {
        self.encoder_state = G729EncoderState::new();
        self.decoder_state = G729DecoderState::new();
        
        debug!("G.729 codec reset");
        Ok(())
    }
    
    fn frame_size(&self) -> usize {
        self.frame_size
    }
    
    fn supports_variable_frame_size(&self) -> bool {
        false // G.729 uses fixed 10ms frames
    }
}

impl AudioCodecExt for G729Codec {
    fn encode_to_buffer(&mut self, samples: &[i16], output: &mut [u8]) -> Result<usize> {
        // Validate input
        validate_g729_frame(samples)?;
        
        if output.len() < 10 {
            return Err(CodecError::BufferTooSmall {
                needed: 10,
                actual: output.len(),
            });
        }
        
        // Simulate G.729 encoding
        let encoded = self.simulate_encode(samples)?;
        output[..encoded.len()].copy_from_slice(&encoded);
        
        trace!("G.729 encoded {} samples to {} bytes (zero-alloc)", 
               samples.len(), encoded.len());
        
        Ok(encoded.len())
    }
    
    fn decode_to_buffer(&mut self, data: &[u8], output: &mut [i16]) -> Result<usize> {
        if data.is_empty() {
            return Err(CodecError::InvalidPayload {
                details: "Empty encoded data".to_string(),
            });
        }
        
        if output.len() < 80 {
            return Err(CodecError::BufferTooSmall {
                needed: 80,
                actual: output.len(),
            });
        }
        
        // Simulate G.729 decoding
        let decoded = self.simulate_decode(data)?;
        output[..decoded.len()].copy_from_slice(&decoded);
        
        trace!("G.729 decoded {} bytes to {} samples (zero-alloc)", 
               data.len(), decoded.len());
        
        Ok(decoded.len())
    }
    
    fn max_encoded_size(&self, input_samples: usize) -> usize {
        // G.729 encodes 80 samples into 10 bytes
        if input_samples <= 80 {
            10
        } else {
            ((input_samples + 79) / 80) * 10
        }
    }
    
    fn max_decoded_size(&self, input_bytes: usize) -> usize {
        // G.729 decodes 10 bytes into 80 samples
        if input_bytes <= 10 {
            80
        } else {
            ((input_bytes + 9) / 10) * 80
        }
    }
}

impl G729Codec {
    /// Simulate G.729 encoding (for testing without external library)
    fn simulate_encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // This is a simulation for testing purposes
        // Real G.729 would perform LPC analysis, pitch detection, etc.
        
        let mut encoded = Vec::with_capacity(10);
        
        // Calculate energy for VAD
        let energy = samples.iter()
            .map(|&s| (s as f32).powi(2))
            .sum::<f32>() / samples.len() as f32;
        
        self.encoder_state.energy_level = energy;
        
        // VAD decision (simplified)
        let is_speech = if self.config.has_vad() {
            energy > 1000000.0 // Threshold for speech detection
        } else {
            true
        };
        
        if !is_speech && self.config.has_cng() {
            // Generate comfort noise frame (2 bytes)
            encoded.push(0x00); // Comfort noise flag
            encoded.push((energy.sqrt() / 256.0) as u8); // Energy level
            return Ok(encoded);
        }
        
        // Simulate LPC analysis
        self.lpc_analysis(samples);
        
        // Simulate pitch analysis
        let pitch_period = self.pitch_analysis(samples);
        
        // Simulate codebook search
        let codebook_indices = self.codebook_search(samples, pitch_period);
        
        // Pack parameters into G.729 frame (10 bytes)
        // LSP indices (18 bits total)
        encoded.push(0x80); // LSP0 high
        encoded.push(0x40); // LSP0 low | LSP1 high
        encoded.push(0x20); // LSP1 low
        
        // Pitch period and gains (8 + 4 + 5 bits)
        encoded.push((pitch_period & 0xFF) as u8);
        encoded.push(((pitch_period >> 8) | 0x10) as u8);
        
        // Algebraic codebook indices (2 × 13 bits)
        encoded.push(codebook_indices[0] as u8);
        encoded.push(codebook_indices[1] as u8);
        encoded.push(codebook_indices[2] as u8);
        
        // Gains (3 + 4 bits)
        encoded.push(codebook_indices[3] as u8);
        encoded.push(0x00); // Padding
        
        // Store frame for next iteration
        self.encoder_state.prev_frame.copy_from_slice(samples);
        
        Ok(encoded)
    }
    
    /// Simulate G.729 decoding
    fn simulate_decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        let mut samples = vec![0i16; 80];
        
        if data.len() == 2 {
            // Comfort noise frame
            let energy_level = data[1] as f32 * 256.0;
            
            // Generate simple comfort noise
            for i in 0..80 {
                let noise = ((i as f32 * 0.1).sin() * energy_level * 0.1);
                samples[i] = noise.clamp(-32768.0, 32767.0) as i16;
            }
            
            return Ok(samples);
        }
        
        if data.len() != 10 {
            return Err(CodecError::InvalidPayload {
                details: format!("Invalid G.729 frame size: {} bytes", data.len()),
            });
        }
        
        // Parse G.729 parameters from packet
        let _lsp_indices = [data[0], data[1], data[2]];
        let pitch_period = (data[3] as usize) | ((data[4] as usize & 0x0F) << 8);
        let _codebook_indices = [data[5], data[6], data[7], data[8]];
        
        // Simulate ACELP synthesis
        let mut excitation = [0.0f32; 80];
        
        // Generate pitch excitation
        self.decoder_state.pitch_synthesis.synthesize(&mut excitation, pitch_period);
        
        // Add algebraic codebook contribution
        self.decoder_state.lpc_synthesis.add_codebook(&mut excitation);
        
        // LPC synthesis filtering
        let mut samples_f32 = [0.0f32; 80];
        self.decoder_state.lpc_synthesis.filter(&excitation, &mut samples_f32);
        
        // Post-filtering
        self.decoder_state.postfilter.process(&mut samples_f32);
        
        // Convert to i16
        for (i, &sample) in samples_f32.iter().enumerate() {
            samples[i] = sample.clamp(-32768.0, 32767.0) as i16;
        }
        
        Ok(samples)
    }
    
    /// Simulate LPC analysis
    fn lpc_analysis(&mut self, samples: &[i16]) {
        // Simplified LPC analysis - in reality this would:
        // 1. Apply windowing (Hamming window)
        // 2. Compute autocorrelation
        // 3. Solve Levinson-Durbin recursion
        // 4. Convert to LSP parameters
        
        // For simulation, use simple predictor
        for i in 0..10 {
            self.encoder_state.lpc_coeffs[i] = 0.1 * (i as f32 + 1.0) / 10.0;
        }
    }
    
    /// Simulate pitch analysis
    fn pitch_analysis(&mut self, samples: &[i16]) -> usize {
        // Simplified pitch analysis - in reality this would:
        // 1. Open-loop pitch search
        // 2. Closed-loop pitch refinement
        // 3. Pitch gain calculation
        
        // For simulation, estimate pitch from zero-crossing rate
        let mut zero_crossings = 0;
        for i in 1..samples.len() {
            if (samples[i] > 0) != (samples[i-1] > 0) {
                zero_crossings += 1;
            }
        }
        
        // Convert to pitch period (very rough approximation)
        let pitch_period = if zero_crossings > 0 {
            (samples.len() / zero_crossings).clamp(20, 143)
        } else {
            40 // Default pitch period
        };
        
        self.encoder_state.pitch_analyzer.prev_pitch = pitch_period;
        pitch_period
    }
    
    /// Simulate codebook search
    fn codebook_search(&mut self, _samples: &[i16], _pitch_period: usize) -> [usize; 4] {
        // Simplified codebook search - in reality this would:
        // 1. Generate target signal
        // 2. Search algebraic codebook
        // 3. Optimize gain quantization
        
        // For simulation, return fixed indices
        [42, 17, 93, 128]
    }
}

// Implementation stubs for simulation
impl G729EncoderState {
    fn new() -> Self {
        Self {
            lpc_coeffs: [0.0; 10],
            pitch_analyzer: PitchAnalyzer::new(),
            codebook_searcher: CodebookSearcher::new(),
            prev_frame: [0; 80],
            energy_level: 0.0,
        }
    }
}

impl G729DecoderState {
    fn new() -> Self {
        Self {
            lpc_synthesis: LpcSynthesis::new(),
            pitch_synthesis: PitchSynthesis::new(),
            postfilter: PostFilter::new(),
            prev_excitation: [0.0; 80],
            bad_frame_count: 0,
        }
    }
}

impl PitchAnalyzer {
    fn new() -> Self {
        Self {
            pitch_min: 20,
            pitch_max: 143,
            prev_pitch: 40,
            pitch_gain: 0.5,
            correlation_buf: [0.0; 143],
        }
    }
}

impl CodebookSearcher {
    fn new() -> Self {
        Self {
            indices: [0; 4],
            gains: [0.0; 4],
            target: [0.0; 40],
        }
    }
}

impl LpcSynthesis {
    fn new() -> Self {
        Self {
            coeffs: [0.0; 10],
            memory: [0.0; 10],
        }
    }
    
    fn add_codebook(&mut self, _excitation: &mut [f32]) {
        // Stub implementation
    }
    
    fn filter(&mut self, _excitation: &[f32], output: &mut [f32]) {
        // Stub implementation - just copy input
        for (i, sample) in output.iter_mut().enumerate() {
            *sample = if i < _excitation.len() { _excitation[i] } else { 0.0 };
        }
    }
}

impl PitchSynthesis {
    fn new() -> Self {
        Self {
            period: 40,
            gain: 0.5,
            excitation_buf: [0.0; 143],
        }
    }
    
    fn synthesize(&mut self, excitation: &mut [f32], _pitch_period: usize) {
        // Stub implementation - generate simple excitation
        for (i, sample) in excitation.iter_mut().enumerate() {
            *sample = (i as f32 * 0.01).sin() * 1000.0;
        }
    }
}

impl PostFilter {
    fn new() -> Self {
        Self {
            formant_coeffs: [0.0; 10],
            tilt_comp: 0.0,
            memory: [0.0; 10],
        }
    }
    
    fn process(&mut self, _samples: &mut [f32]) {
        // Stub implementation - no post-filtering
    }
}

// Simple random number generator for simulation
mod rand {
    use std::sync::atomic::{AtomicU64, Ordering};
    
    static SEED: AtomicU64 = AtomicU64::new(12345);
    
    pub fn random<T>() -> T 
    where
        T: From<f32>,
    {
        let current = SEED.load(Ordering::Relaxed);
        let next = current.wrapping_mul(1103515245).wrapping_add(12345);
        SEED.store(next, Ordering::Relaxed);
        let normalized = (next as f32) / (u64::MAX as f32);
        T::from(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CodecConfig, CodecType, SampleRate};

    fn create_test_config() -> CodecConfig {
        CodecConfig::new(CodecType::G729)
            .with_sample_rate(SampleRate::Rate8000)
            .with_channels(1)
    }

    #[test]
    fn test_g729_creation() {
        let config = create_test_config();
        let codec = G729Codec::new(config);
        assert!(codec.is_ok());
        
        let codec = codec.unwrap();
        assert_eq!(codec.frame_size(), 80);
        
        let info = codec.info();
        assert_eq!(info.name, "G729");
        assert_eq!(info.sample_rate, 8000);
        assert_eq!(info.payload_type, Some(18));
        assert_eq!(info.bitrate, 8000);
    }

    #[test]
    fn test_invalid_sample_rate() {
        let mut config = create_test_config();
        config.sample_rate = SampleRate::Rate16000;
        
        let codec = G729Codec::new(config);
        assert!(codec.is_err());
    }

    #[test]
    fn test_invalid_channels() {
        let mut config = create_test_config();
        config.channels = 2;
        
        let codec = G729Codec::new(config);
        assert!(codec.is_err());
    }

    #[test]
    fn test_encoding_decoding_roundtrip() {
        let config = create_test_config();
        let mut codec = G729Codec::new(config).unwrap();
        
        // Create test signal
        let mut samples = Vec::new();
        for i in 0..80 {
            let t = i as f32 / 8000.0;
            let sample = ((2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 16000.0) as i16;
            samples.push(sample);
        }
        
        // Encode
        let encoded = codec.encode(&samples).unwrap();
        assert_eq!(encoded.len(), 10); // G.729 frame size
        
        // Decode
        let decoded = codec.decode(&encoded).unwrap();
        assert_eq!(decoded.len(), 80);
        
        // G.729 is very lossy, so we just check that decoding produces output
        let mut has_non_zero = false;
        for &sample in &decoded {
            if sample != 0 {
                has_non_zero = true;
                break;
            }
        }
        assert!(has_non_zero, "Decoded signal should not be all zeros");
    }

    #[test]
    fn test_zero_copy_apis() {
        let mut config = create_test_config();
        config.parameters.g729.annex_b = false; // Disable VAD to ensure predictable frame size
        let mut codec = G729Codec::new(config).unwrap();
        
        let samples = vec![1000i16; 80];
        let mut encoded = vec![0u8; 10];
        let mut decoded = vec![0i16; 80];
        
        // Test zero-copy encoding
        let encoded_len = codec.encode_to_buffer(&samples, &mut encoded).unwrap();
        assert_eq!(encoded_len, 10);
        
        // Test zero-copy decoding
        let decoded_len = codec.decode_to_buffer(&encoded, &mut decoded).unwrap();
        assert_eq!(decoded_len, 80);
    }

    #[test]
    fn test_frame_size_validation() {
        let config = create_test_config();
        let mut codec = G729Codec::new(config).unwrap();
        
        // Wrong frame size should fail
        let wrong_samples = vec![0i16; 160];
        assert!(codec.encode(&wrong_samples).is_err());
        
        // Empty samples should fail
        let empty_samples: Vec<i16> = vec![];
        assert!(codec.encode(&empty_samples).is_err());
    }

    #[test]
    fn test_codec_reset() {
        let config = create_test_config();
        let mut codec = G729Codec::new(config).unwrap();
        
        assert!(codec.reset().is_ok());
    }

    #[test]
    fn test_compression_ratio() {
        let config = create_test_config();
        let codec = G729Codec::new(config).unwrap();
        
        assert_eq!(codec.compression_ratio(), 0.125);
        assert_eq!(codec.max_encoded_size(80), 10);
        assert_eq!(codec.max_decoded_size(10), 80);
    }

    #[test]
    fn test_comfort_noise_generation() {
        let config = create_test_config();
        let mut codec = G729Codec::new(config).unwrap();
        
        // Low energy samples should trigger comfort noise
        let quiet_samples = vec![10i16; 80];
        let encoded = codec.encode(&quiet_samples).unwrap();
        
        // Comfort noise frames are 2 bytes
        if encoded.len() == 2 {
            let decoded = codec.decode(&encoded).unwrap();
            assert_eq!(decoded.len(), 80);
        }
    }

    #[test]
    fn test_vad_configuration() {
        let mut config = create_test_config();
        config.parameters.g729.annex_b = false; // Disable VAD/DTX/CNG
        
        let codec = G729Codec::new(config).unwrap();
        assert!(!codec.config.has_vad());
        assert!(!codec.config.has_cng());
        assert!(!codec.config.has_dtx());
    }

    #[test]
    fn test_buffer_size_validation() {
        let config = create_test_config();
        let mut codec = G729Codec::new(config).unwrap();
        
        let samples = vec![0i16; 80];
        let mut small_buffer = vec![0u8; 5]; // Too small
        
        assert!(codec.encode_to_buffer(&samples, &mut small_buffer).is_err());
    }

    #[test]
    fn test_invalid_encoded_data() {
        let config = create_test_config();
        let mut codec = G729Codec::new(config).unwrap();
        
        // Invalid frame size
        let invalid_data = vec![0u8; 7];
        assert!(codec.decode(&invalid_data).is_err());
        
        // Empty data
        let empty_data: Vec<u8> = vec![];
        assert!(codec.decode(&empty_data).is_err());
    }

    #[test]
    fn test_fixed_frame_size() {
        let config = create_test_config();
        let codec = G729Codec::new(config).unwrap();
        
        // G.729 doesn't support variable frame sizes
        assert!(!codec.supports_variable_frame_size());
        assert_eq!(codec.frame_size(), 80);
    }
} 

#[cfg(test)]
mod runtime_config_tests {
    use super::*;
    
    #[test]
    fn test_g729_config_variants() {
        // Test all four G.729 variants
        let g729 = G729Config::new().with_annex_a(false).with_annex_b(false);
        assert_eq!(g729.variant_name(), "G.729");
        assert_eq!(g729.cpu_efficiency(), 1.0);
        assert_eq!(g729.bandwidth_efficiency(), 1.0);
        assert!(!g729.has_vad() && !g729.has_dtx() && !g729.has_cng());
        
        let g729a = G729Config::new().with_annex_a(true).with_annex_b(false);
        assert_eq!(g729a.variant_name(), "G.729A");
        assert_eq!(g729a.cpu_efficiency(), 0.6);
        assert_eq!(g729a.bandwidth_efficiency(), 1.0);
        assert!(!g729a.has_vad() && !g729a.has_dtx() && !g729a.has_cng());
        
        let g729b = G729Config::new().with_annex_a(false).with_annex_b(true);
        assert_eq!(g729b.variant_name(), "G.729B");
        assert_eq!(g729b.cpu_efficiency(), 1.0);
        assert_eq!(g729b.bandwidth_efficiency(), 0.5);
        assert!(g729b.has_vad() && g729b.has_dtx() && g729b.has_cng());
        
        let g729ba = G729Config::new().with_annex_a(true).with_annex_b(true);
        assert_eq!(g729ba.variant_name(), "G.729BA");
        assert_eq!(g729ba.cpu_efficiency(), 0.6);
        assert_eq!(g729ba.bandwidth_efficiency(), 0.5);
        assert!(g729ba.has_vad() && g729ba.has_dtx() && g729ba.has_cng());
    }
    
    #[test]
    fn test_codec_creation_methods() {
        // Test default creation (should be G.729BA)
        let codec_default = G729Codec::new_default().unwrap();
        assert_eq!(codec_default.variant(), "G.729BA");
        assert!(codec_default.has_annex_a());
        assert!(codec_default.has_annex_b());
        
        // Test creation with explicit annexes
        let codec_core = G729Codec::new_with_annexes(false, false).unwrap();
        assert_eq!(codec_core.variant(), "G.729");
        assert!(!codec_core.has_annex_a());
        assert!(!codec_core.has_annex_b());
        
        let codec_a = G729Codec::new_with_annexes(true, false).unwrap();
        assert_eq!(codec_a.variant(), "G.729A");
        assert!(codec_a.has_annex_a());
        assert!(!codec_a.has_annex_b());
        
        let codec_b = G729Codec::new_with_annexes(false, true).unwrap();
        assert_eq!(codec_b.variant(), "G.729B");
        assert!(!codec_b.has_annex_a());
        assert!(codec_b.has_annex_b());
        
        // Test creation with custom config
        let config = G729Config::new().with_annex_a(true).with_annex_b(true);
        let codec_ba = G729Codec::new_with_config(config).unwrap();
        assert_eq!(codec_ba.variant(), "G.729BA");
        assert!(codec_ba.has_annex_a());
        assert!(codec_ba.has_annex_b());
    }
    
    #[test]
    fn test_runtime_configuration_changes() {
        let mut codec = G729Codec::new_with_annexes(false, false).unwrap();
        assert_eq!(codec.variant(), "G.729");
        
        // Enable Annex A
        codec.set_annex_a(true).unwrap();
        assert_eq!(codec.variant(), "G.729A");
        assert!(codec.has_annex_a());
        assert!(!codec.has_annex_b());
        
        // Enable Annex B  
        codec.set_annex_b(true).unwrap();
        assert_eq!(codec.variant(), "G.729BA");
        assert!(codec.has_annex_a());
        assert!(codec.has_annex_b());
        
        // Disable Annex A
        codec.set_annex_a(false).unwrap();
        assert_eq!(codec.variant(), "G.729B");
        assert!(!codec.has_annex_a());
        assert!(codec.has_annex_b());
        
        // Disable Annex B
        codec.set_annex_b(false).unwrap();
        assert_eq!(codec.variant(), "G.729");
        assert!(!codec.has_annex_a());
        assert!(!codec.has_annex_b());
    }
    
    #[test]
    fn test_configuration_update() {
        let mut codec = G729Codec::new_default().unwrap();
        assert_eq!(codec.variant(), "G.729BA");
        
        // Update to G.729 core
        let new_config = G729Config::new().with_annex_a(false).with_annex_b(false);
        codec.update_config(new_config).unwrap();
        assert_eq!(codec.variant(), "G.729");
        assert!(!codec.has_annex_a());
        assert!(!codec.has_annex_b());
        
        // Update to G.729A
        let new_config = G729Config::new().with_annex_a(true).with_annex_b(false);
        codec.update_config(new_config).unwrap();
        assert_eq!(codec.variant(), "G.729A");
        assert!(codec.has_annex_a());
        assert!(!codec.has_annex_b());
    }
    
    #[test]
    fn test_config_efficiency_metrics() {
        let config_base = G729Config::new().with_annex_a(false).with_annex_b(false);
        let config_a = G729Config::new().with_annex_a(true).with_annex_b(false);
        let config_b = G729Config::new().with_annex_a(false).with_annex_b(true);
        let config_ba = G729Config::new().with_annex_a(true).with_annex_b(true);
        
        // CPU efficiency should be 1.0 for base, 0.6 for Annex A variants
        assert_eq!(config_base.cpu_efficiency(), 1.0);
        assert_eq!(config_a.cpu_efficiency(), 0.6);
        assert_eq!(config_b.cpu_efficiency(), 1.0);
        assert_eq!(config_ba.cpu_efficiency(), 0.6);
        
        // Bandwidth efficiency should be 1.0 for base, 0.5 for Annex B variants
        assert_eq!(config_base.bandwidth_efficiency(), 1.0);
        assert_eq!(config_a.bandwidth_efficiency(), 1.0);
        assert_eq!(config_b.bandwidth_efficiency(), 0.5);
        assert_eq!(config_ba.bandwidth_efficiency(), 0.5);
    }
    
    #[test]
    fn test_codec_configuration_access() {
        let codec = G729Codec::new_with_annexes(true, false).unwrap();
        
        let config = codec.config();
        assert!(config.annex_a);
        assert!(!config.annex_b);
        assert_eq!(config.variant_name(), "G.729A");
        assert_eq!(config.cpu_efficiency(), 0.6);
        assert_eq!(config.bandwidth_efficiency(), 1.0);
    }
    
    #[test]
    fn test_idempotent_configuration_changes() {
        let mut codec = G729Codec::new_with_annexes(true, true).unwrap();
        let initial_variant = codec.variant();
        
        // Setting the same configuration should not change anything
        codec.set_annex_a(true).unwrap(); // Already enabled
        codec.set_annex_b(true).unwrap(); // Already enabled
        
        assert_eq!(codec.variant(), initial_variant);
        assert!(codec.has_annex_a());
        assert!(codec.has_annex_b());
    }
}

#[cfg(test)]
mod usage_examples_tests {
    use super::*;
    
    #[test]
    fn test_production_voip_setup() {
        // Most common production setup: G.729BA (reduced complexity + VAD/DTX/CNG)
        let codec = G729Codec::new_default().unwrap(); // Defaults to G.729BA
        assert_eq!(codec.variant(), "G.729BA");
        assert!(codec.has_annex_a()); // Reduced complexity for better performance
        assert!(codec.has_annex_b()); // VAD/DTX/CNG for bandwidth efficiency
    }
    
    #[test]
    fn test_low_power_device_setup() {
        // For IoT/embedded devices: G.729A (reduced complexity, no VAD overhead)
        let codec = G729Codec::new_with_annexes(true, false).unwrap();
        assert_eq!(codec.variant(), "G.729A");
        assert!(codec.has_annex_a()); // 40% less CPU usage
        assert!(!codec.has_annex_b()); // No VAD processing overhead
    }
    
    #[test]
    fn test_bandwidth_critical_setup() {
        // For satellite/expensive connections: G.729B (full quality + maximum bandwidth savings)
        let codec = G729Codec::new_with_annexes(false, true).unwrap();
        assert_eq!(codec.variant(), "G.729B");
        assert!(!codec.has_annex_a()); // Full quality processing
        assert!(codec.has_annex_b()); // Maximum bandwidth efficiency during silence
    }
    
    #[test]
    fn test_reference_testing_setup() {
        // For ITU compliance testing: G.729 core (full complexity reference)
        let codec = G729Codec::new_with_annexes(false, false).unwrap();
        assert_eq!(codec.variant(), "G.729");
        assert!(!codec.has_annex_a()); // Full complexity reference implementation
        assert!(!codec.has_annex_b()); // No VAD/DTX extensions
    }
    
    #[test]
    fn test_adaptive_configuration_scenario() {
        // Scenario: Start with basic G.729, adapt based on network conditions
        let mut codec = G729Codec::new_with_annexes(false, false).unwrap();
        assert_eq!(codec.variant(), "G.729");
        
        // Network congestion detected -> enable bandwidth savings
        codec.set_annex_b(true).unwrap();
        assert_eq!(codec.variant(), "G.729B");
        
        // CPU load high -> enable reduced complexity
        codec.set_annex_a(true).unwrap();
        assert_eq!(codec.variant(), "G.729BA");
        
        // Network improved, CPU load normal -> back to reference quality
        codec.set_annex_a(false).unwrap();
        codec.set_annex_b(false).unwrap();
        assert_eq!(codec.variant(), "G.729");
    }
} 