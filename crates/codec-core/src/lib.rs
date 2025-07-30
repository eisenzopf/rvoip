//! # Codec-Core: High-Performance Audio Codec Library
//!
//! A comprehensive, production-ready implementation of audio codecs for VoIP applications.
//! This library provides ITU-T compliant codecs with SIMD optimizations, zero-copy APIs,
//! and extensive testing including real audio validation.
//!
//! ## 🎯 Production Ready
//!
//! - **ITU-T Compliant**: All codecs pass official compliance tests
//! - **Real Audio Tested**: Validated with actual speech samples  
//! - **High Quality**: >37 dB SNR with real speech validation
//! - **Performance Optimized**: SIMD acceleration and lookup tables
//! - **Zero Allocation**: Efficient batch processing APIs
//!
//! ## Features
//!
//! - **G.711 (PCMU/PCMA)**: ITU-T compliant μ-law and A-law with SIMD optimizations
//! - **Real Audio Testing**: Validated with actual speech samples via WAV roundtrip tests
//! - **Comprehensive Test Suite**: ITU-T compliance tests and quality validation
//!
//! ## Performance
//!
//! - **SIMD Optimized**: x86_64 SSE2 and AArch64 NEON support
//! - **Lookup Tables**: Pre-computed tables for O(1) operations
//! - **Zero-Copy APIs**: Minimal memory allocation during processing
//! - **Parallel Processing**: Multi-threaded encoding/decoding where beneficial
//!
//! ## Usage
//!
//! ### Quick Start
//!
//! ```rust
//! use codec_core::codecs::g711::G711Codec;
//! use codec_core::types::{AudioCodec, CodecConfig, CodecType, SampleRate};
//!
//! // Create a G.711 μ-law codec
//! let config = CodecConfig::new(CodecType::G711Pcmu)
//!     .with_sample_rate(SampleRate::Rate8000)
//!     .with_channels(1);
//! let mut codec = G711Codec::new_pcmu(config)?;
//!
//! // Encode audio samples (20ms at 8kHz = 160 samples)
//! let samples = vec![0i16; 160];
//! let encoded = codec.encode(&samples)?;
//!
//! // Decode back to samples
//! let decoded = codec.decode(&encoded)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Testing & Validation
//!
//! The library includes comprehensive testing including real audio validation:
//!
//! ```bash
//! # Run all codec tests including WAV roundtrip tests
//! cargo test
//!
//! # Run only G.711 WAV roundtrip tests (downloads real speech audio)
//! cargo test wav_roundtrip_test -- --nocapture
//! ```
//!
//! The WAV roundtrip tests automatically download real speech samples and validate:
//! - Signal-to-Noise Ratio (SNR) measurement
//! - Round-trip audio quality preservation  
//! - Proper encoding/decoding with real audio data
//! - Output WAV files for manual quality assessment
//!
//! ## Error Handling
//!
//! All codec operations return `Result` types with detailed error information:
//!
//! ```rust
//! use codec_core::codecs::g711::G711Codec;
//! use codec_core::types::{CodecConfig, CodecType, SampleRate};
//! use codec_core::error::CodecError;
//!
//! // Handle configuration errors
//! let config = CodecConfig::new(CodecType::G711Pcmu)
//!     .with_sample_rate(SampleRate::Rate48000) // Invalid for G.711
//!     .with_channels(1);
//!
//! match G711Codec::new_pcmu(config) {
//!     Ok(codec) => println!("Codec created successfully"),
//!     Err(CodecError::InvalidSampleRate { rate, supported }) => {
//!         println!("Invalid sample rate {}, supported: {:?}", rate, supported);
//!     }
//!     Err(e) => println!("Other error: {}", e),
//! }
//! ```
//!
//! ## Performance Tips
//!
//! - Use batch processing functions for better performance
//! - Pre-allocate output buffers when possible
//! - Enable SIMD optimizations (automatic on x86_64/AArch64)
//! - Use appropriate frame sizes (160 samples for G.711)
//!
//! ### Direct G.711 Functions
//!
//! ```rust
//! use codec_core::codecs::g711::{alaw_compress, alaw_expand, ulaw_compress, ulaw_expand};
//!
//! // Single sample processing
//! let sample = 1024i16;
//! let alaw_encoded = alaw_compress(sample);
//! let alaw_decoded = alaw_expand(alaw_encoded);
//!
//! let ulaw_encoded = ulaw_compress(sample);
//! let ulaw_decoded = ulaw_expand(ulaw_encoded);
//! ```
//!
//! ### Frame-Based Processing
//!
//! ```rust
//! use codec_core::codecs::g711::{G711Codec, G711Variant};
//!
//! let mut codec = G711Codec::new(G711Variant::MuLaw);
//!
//! // Process 160 samples (20ms at 8kHz)
//! let input_frame = vec![1000i16; 160]; // Some test samples
//! let encoded = codec.compress(&input_frame).unwrap();
//!
//! // Decode back to samples (same count for G.711)
//! let decoded = codec.expand(&encoded).unwrap();
//! assert_eq!(input_frame.len(), decoded.len());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Supported Codecs
//!
//! | Codec | Sample Rate | Channels | Bitrate | Frame Size | Status |
//! |-------|-------------|----------|---------|------------|--------|
//! | **G.711 μ-law (PCMU)** | 8 kHz | 1 | 64 kbps | 160 samples | ✅ Production |
//! | **G.711 A-law (PCMA)** | 8 kHz | 1 | 64 kbps | 160 samples | ✅ Production |
//!
//! ## Quality Metrics
//!
//! Based on real audio testing with the included WAV roundtrip tests:
//!
//! - **G.711**: 37+ dB SNR (excellent quality, industry standard)
//!
//! ## Feature Flags
//!
//! ### Core Codecs (enabled by default)
//! - `g711`: G.711 μ-law/A-law codecs
//! ### Optimizations
//! - `simd`: SIMD optimizations (auto-detected)
//! - `lut`: Lookup table optimizations (enabled by default)

#![deny(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::module_name_repetitions)]

pub mod codecs;
pub mod error;
pub mod types;
pub mod utils;

// Re-export commonly used types and traits
pub use codecs::{CodecFactory, CodecRegistry};
pub use error::{CodecError, Result};
pub use types::{
    AudioCodec, AudioFrame, CodecCapability, CodecConfig, CodecInfo, CodecType, SampleRate,
};

/// Version information for the codec library
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Supported codec types
pub const SUPPORTED_CODECS: &[&str] = &[
    #[cfg(feature = "g711")]
    "PCMU",
    #[cfg(feature = "g711")]
    "PCMA",

    #[cfg(any(feature = "g729", feature = "g729-sim"))]
    "G729",
    #[cfg(any(feature = "opus", feature = "opus-sim"))]
    "opus",
];

/// Initialize the codec library
///
/// This function should be called once at program startup to initialize
/// any global state or lookup tables. It's safe to call multiple times.
///
/// # Errors
///
/// Returns an error if initialization fails (e.g., SIMD detection fails)
pub fn init() -> Result<()> {
    // Initialize logging if not already done
    let _ = tracing_subscriber::fmt::try_init();

    // Initialize SIMD capabilities
    utils::simd::init_simd_support();

    // Initialize lookup tables
    #[cfg(feature = "g711")]
    codecs::g711::init_tables();

    tracing::info!("Codec-Core v{} initialized", VERSION);
    tracing::info!("Supported codecs: {:?}", SUPPORTED_CODECS);

    Ok(())
}

/// Get library information
pub fn info() -> LibraryInfo {
    LibraryInfo {
        version: VERSION,
        supported_codecs: SUPPORTED_CODECS.to_vec(),
        simd_support: utils::simd::get_simd_support(),
    }
}

/// Library information structure
#[derive(Debug, Clone)]
pub struct LibraryInfo {
    /// Library version
    pub version: &'static str,
    /// List of supported codec names
    pub supported_codecs: Vec<&'static str>,
    /// SIMD support information
    pub simd_support: utils::simd::SimdSupport,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init() {
        assert!(init().is_ok());
    }

    #[test]
    fn test_info() {
        let info = info();
        assert_eq!(info.version, VERSION);
        assert!(!info.supported_codecs.is_empty());
    }

    #[test]
    fn test_supported_codecs() {
        assert!(!SUPPORTED_CODECS.is_empty());
        
        #[cfg(feature = "g711")]
        {
            assert!(SUPPORTED_CODECS.contains(&"PCMU"));
            assert!(SUPPORTED_CODECS.contains(&"PCMA"));
        }
        
        #[cfg(feature = "g722")]
        // G.722 support removed - only G.711 variants now supported
        
        #[cfg(any(feature = "g729", feature = "g729-sim"))]
        assert!(SUPPORTED_CODECS.contains(&"G729"));
        
        #[cfg(any(feature = "opus", feature = "opus-sim"))]
        assert!(SUPPORTED_CODECS.contains(&"opus"));
    }
} 