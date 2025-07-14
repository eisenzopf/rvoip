//! # Codec-Core: High-Performance Audio Codec Library
//!
//! This library provides a unified, high-performance implementation of audio codecs
//! for VoIP applications. It consolidates and optimizes codec implementations to
//! provide consistent, efficient audio processing across the RVOIP ecosystem.
//!
//! ## Features
//!
//! - **G.711 (PCMU/PCMA)**: ITU-T compliant μ-law and A-law with SIMD optimizations
//! - **G.722**: Wideband audio codec with sub-band coding
//! - **G.729**: Low-bitrate codec with ACELP (simulation and real modes)
//! - **Opus**: Modern codec with flexible bitrate and sample rate support
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
//! ```rust
//! use rvoip_codec_core::{CodecFactory, CodecConfig, CodecType};
//!
//! // Create a G.711 μ-law codec
//! let config = CodecConfig::g711_pcmu();
//! let mut codec = CodecFactory::create(config)?;
//!
//! // Encode audio samples
//! let samples = vec![0i16; 160]; // 20ms at 8kHz
//! let encoded = codec.encode(&samples)?;
//!
//! // Decode back to samples
//! let decoded = codec.decode(&encoded)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Feature Flags
//!
//! - `g711`: G.711 μ-law/A-law codecs (enabled by default)
//! - `g722`: G.722 wideband codec (enabled by default)
//! - `g729`: Real G.729 codec (requires external library)
//! - `g729-sim`: G.729 simulation mode (enabled by default)
//! - `opus`: Real Opus codec (requires external library)
//! - `opus-sim`: Opus simulation mode (enabled by default)
//! - `simd`: SIMD optimizations (auto-detected)

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
    #[cfg(feature = "g722")]
    "G722",
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
        assert!(SUPPORTED_CODECS.contains(&"G722"));
        
        #[cfg(any(feature = "g729", feature = "g729-sim"))]
        assert!(SUPPORTED_CODECS.contains(&"G729"));
        
        #[cfg(any(feature = "opus", feature = "opus-sim"))]
        assert!(SUPPORTED_CODECS.contains(&"opus"));
    }
} 