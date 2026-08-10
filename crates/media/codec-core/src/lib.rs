//! # `Codec-Core`: Audio Codec Library for `VoIP`
//!
//! A simple implementation of G.711 audio codec for `VoIP` applications.
//! This library provides `ITU-T` compliant G.711 μ-law and A-law encoding/decoding
//! with lookup table optimizations.
//!
//! ## Features
//!
//! - **ITU-T G.711 Compliant**: Passes official compliance tests
//! - **Real Audio Tested**: Validated with actual speech samples
//! - **Good Quality**: ~37 dB SNR with real speech
//! - **Lookup Table Optimized**: Fast O(1) encoding/decoding
//!
//! ## Implementation
//!
//! - **Lookup Tables**: Pre-computed tables for O(1) operations
//! - **Simple APIs**: Straightforward encoding/decoding functions
//!
//! ## Usage
//!
//! ### Quick Start
//!
//! ```rust
//! # #[cfg(feature = "g711")]
//! # {
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
//! # }
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
//! # #[cfg(feature = "g711")]
//! # {
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
//! # }
//! ```
//!
//! ## Performance Tips
//!

//! - Use appropriate frame sizes (160 samples for G.711 at 8kHz/20ms)
//!
//! ### Direct G.711 Functions
//!
//! ```rust
//! # #[cfg(feature = "g711")]
//! # {
//! use codec_core::codecs::g711::{alaw_compress, alaw_expand, ulaw_compress, ulaw_expand};
//!
//! // Single sample processing
//! let sample = 1024i16;
//! let alaw_encoded = alaw_compress(sample);
//! let alaw_decoded = alaw_expand(alaw_encoded);
//!
//! let ulaw_encoded = ulaw_compress(sample);
//! let ulaw_decoded = ulaw_expand(ulaw_encoded);
//! # }
//! ```
//!
//! ### Frame-Based Processing
//!
//! ```rust
//! # #[cfg(feature = "g711")]
//! # {
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
//! # }
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
//!
//! ### Optional Codecs
//! - `g729`: G.729A/G.729AB
//! - `opus`: Opus, backed by libopus
//! - `amr-nb` / `amr-wb` / `amr`: AMR narrowband and wideband. **Types and mode
//!   negotiation only at present — the encoder and decoder are not implemented.**
//!   See `docs/AMR_IMPLEMENTATION_PLAN.md`.

#![deny(missing_docs)]
#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::module_name_repetitions)]

pub mod codecs;
pub mod error;

/// ITU-T / 3GPP fixed-point basic operators (the ETSI "basicop" library).
///
/// Shared by every fixed-point speech codec here: G.729 and AMR both specify
/// their arithmetic in terms of these exact saturating operations, so a single
/// implementation is the only way both can be bit-exact against the same
/// definitions. Originally written for the G.729A port and promoted out of it
/// when AMR needed the same foundation.
///
/// Crate-internal: an implementation detail shared between codecs, not a public
/// API surface this crate wants to commit to.
#[cfg(any(feature = "g729", feature = "amr-nb", feature = "amr-wb"))]
pub(crate) mod fixed_point;
pub mod types;
pub mod utils;

// Re-export commonly used types and traits
pub use codecs::{CodecFactory, CodecRegistry};
pub use error::{CodecError, Result};
pub use types::{
    AudioCodec, AudioFrame, CodecCapability, CodecConfig, CodecInfo, CodecType, CodedFrame,
    FrameKind, SampleRate, VariableRateCodec,
};

/// Version information for the codec library
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Supported codec types
pub const SUPPORTED_CODECS: &[&str] = &[
    #[cfg(feature = "g711")]
    "PCMU",
    #[cfg(feature = "g711")]
    "PCMA",
    #[cfg(feature = "g729")]
    "G729",
    #[cfg(feature = "g729")]
    "G729A",
    #[cfg(feature = "g729")]
    "G729BA",
    #[cfg(feature = "opus")]
    "opus",
    #[cfg(feature = "amr-nb")]
    "AMR",
    #[cfg(feature = "amr-wb")]
    "AMR-WB",
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

    // Initialize lookup tables
    #[cfg(feature = "g711")]
    codecs::g711::init_tables();

    tracing::info!("Codec-Core v{} initialized", VERSION);
    tracing::info!("Supported codecs: {:?}", SUPPORTED_CODECS);

    Ok(())
}

/// Get library information
#[must_use]
pub fn info() -> LibraryInfo {
    LibraryInfo {
        version: VERSION,
        supported_codecs: SUPPORTED_CODECS.to_vec(),
    }
}

/// Library information structure
#[derive(Debug, Clone)]
pub struct LibraryInfo {
    /// Library version
    pub version: &'static str,
    /// List of supported codec names
    pub supported_codecs: Vec<&'static str>,
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

        #[cfg(any(feature = "g711", feature = "g729", feature = "opus"))]
        assert!(!info.supported_codecs.is_empty());

        #[cfg(not(any(feature = "g711", feature = "g729", feature = "opus")))]
        assert!(info.supported_codecs.is_empty());
    }

    #[test]
    fn test_supported_codecs() {
        #[cfg(any(feature = "g711", feature = "g729", feature = "opus"))]
        assert!(!SUPPORTED_CODECS.is_empty());

        #[cfg(not(any(feature = "g711", feature = "g729", feature = "opus")))]
        assert!(SUPPORTED_CODECS.is_empty());

        #[cfg(feature = "g711")]
        {
            assert!(SUPPORTED_CODECS.contains(&"PCMU"));
            assert!(SUPPORTED_CODECS.contains(&"PCMA"));
        }

        #[cfg(feature = "g729")]
        {
            assert!(SUPPORTED_CODECS.contains(&"G729"));
            assert!(SUPPORTED_CODECS.contains(&"G729A"));
            assert!(SUPPORTED_CODECS.contains(&"G729BA"));
        }

        #[cfg(feature = "opus")]
        assert!(SUPPORTED_CODECS.contains(&"opus"));
    }
}
