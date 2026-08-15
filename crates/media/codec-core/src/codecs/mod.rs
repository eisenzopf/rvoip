//! # Audio Codec Implementations
//!
//! This module contains G.711 audio codec implementation for `VoIP` applications.
//!
//! ## Available Codecs
//!
//! ### G.711 (PCMU/PCMA) - [`g711`]
//! - **Standard**: ITU-T G.711
//! - **Sample Rate**: 8 kHz
//! - **Bitrate**: 64 kbps
//! - **Quality**: ~37 dB SNR
//! - **Use Case**: Standard telephony
//! - **Variants**: μ-law (PCMU), A-law (PCMA)
//!
//! ## Testing
//!
//! G.711 is validated with real speech samples through WAV roundtrip tests:
//! - Downloads reference audio samples
//! - Round-trip encoding and decoding validation
//! - Signal-to-Noise Ratio (SNR) measurement
//!
//! ## Usage Examples
//!
//! ### Using the Codec Factory
//! ```rust
//! # #[cfg(feature = "g711")]
//! # {
//! use codec_core::codecs::CodecFactory;
//! use codec_core::types::{CodecConfig, CodecType, SampleRate};
//!
//! // Create any codec through the factory
//! let config = CodecConfig::new(CodecType::G711Pcmu)
//!     .with_sample_rate(SampleRate::Rate8000);
//! let mut codec = CodecFactory::create(config)?;
//!
//! // Use unified interface
//! let samples = vec![0i16; 160];
//! let encoded = codec.encode(&samples)?;
//! let decoded = codec.decode(&encoded)?;
//! # }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ### Direct Codec Access
//! ```rust
//! # #[cfg(feature = "g711")]
//! # {
//! use codec_core::codecs::g711::{G711Codec, G711Variant};
//!
//! // Direct instantiation
//! let mut g711_ulaw = G711Codec::new(G711Variant::MuLaw);
//! let mut g711_alaw = G711Codec::new(G711Variant::ALaw);
//! # }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! ## Testing & Validation
//!
//! All codecs include comprehensive test suites:
//! - ITU-T compliance validation
//! - Real audio roundtrip tests
//! - Performance benchmarks
//! - Quality measurements (SNR)
//!
//! ```bash
//! # Test all codecs
//! cargo test
//!
//! # Test with real audio (downloads speech samples)
//! cargo test wav_roundtrip_test -- --nocapture
//! ```

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, CodecConfig, CodecInfo, CodecType};
use std::collections::HashMap;

// Codec implementations
#[cfg(feature = "g711")]
pub mod g711;

#[cfg(feature = "g729")]
pub mod g729;

#[cfg(feature = "opus")]
pub mod opus;

#[cfg(any(feature = "amr-nb", feature = "amr-wb"))]
pub mod amr;

/// Codec factory for creating codec instances
pub struct CodecFactory;

impl CodecFactory {
    /// Create a codec instance from configuration
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration is invalid, the codec feature
    /// is disabled, or codec construction fails.
    // Preserve the public by-value constructor in a build where every codec
    // branch is compiled out; feature-enabled branches transfer ownership to
    // their concrete codec constructors.
    #[cfg_attr(
        not(any(feature = "g711", feature = "g729", feature = "opus")),
        allow(clippy::needless_pass_by_value)
    )]
    pub fn create(config: CodecConfig) -> Result<Box<dyn AudioCodec>> {
        // Validate configuration first
        config.validate()?;

        match config.codec_type {
            #[cfg(feature = "g711")]
            CodecType::G711Pcmu => {
                let codec = g711::G711Codec::new_pcmu(config)?;
                Ok(Box::new(codec))
            }

            #[cfg(feature = "g711")]
            CodecType::G711Pcma => {
                let codec = g711::G711Codec::new_pcma(config)?;
                Ok(Box::new(codec))
            }

            #[cfg(feature = "g729")]
            CodecType::G729 | CodecType::G729A | CodecType::G729BA => {
                let codec = g729::G729Codec::new(config)?;
                Ok(Box::new(codec))
            }

            #[cfg(feature = "opus")]
            CodecType::Opus => {
                let codec = opus::OpusCodec::new(config)?;
                Ok(Box::new(codec))
            }

            #[cfg(feature = "amr-nb")]
            CodecType::AmrNb => {
                let codec = amr::AmrCodec::new(&config)?;
                Ok(Box::new(codec))
            }

            #[cfg(feature = "amr-wb")]
            CodecType::AmrWb => {
                let codec = amr::AmrCodec::new(&config)?;
                Ok(Box::new(codec))
            }

            codec_type => Err(CodecError::feature_not_enabled(format!(
                "Codec {codec_type} not enabled in build features"
            ))),
        }
    }

    /// Create a codec by name
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is unknown, its feature is disabled, or
    /// the configuration is invalid.
    pub fn create_by_name(name: &str, config: CodecConfig) -> Result<Box<dyn AudioCodec>> {
        let codec_type = match normalize_codec_name(name).as_str() {
            "PCMU" => CodecType::G711Pcmu,
            "PCMA" => CodecType::G711Pcma,
            "G729" => CodecType::G729,
            "G729A" => CodecType::G729A,
            "G729AB" | "G729BA" => CodecType::G729BA,
            "OPUS" => CodecType::Opus,
            "AMR" => CodecType::AmrNb,
            "AMR-WB" | "AMRWB" => CodecType::AmrWb,
            _ => return Err(CodecError::unsupported_codec(name)),
        };

        let config = CodecConfig {
            codec_type,
            ..config
        };

        Self::create(config)
    }

    /// Create a codec by RTP payload type
    ///
    /// # Errors
    ///
    /// Returns an error when the payload type is unknown, its codec feature is
    /// disabled, or the configuration is invalid.
    pub fn create_by_payload_type(
        payload_type: u8,
        config: CodecConfig,
    ) -> Result<Box<dyn AudioCodec>> {
        let codec_type = match payload_type {
            0 => CodecType::G711Pcmu,
            8 => CodecType::G711Pcma,
            18 => CodecType::G729,

            _ => return Err(CodecError::unsupported_codec(format!("PT{payload_type}"))),
        };

        let config = CodecConfig {
            codec_type,
            ..config
        };

        Self::create(config)
    }

    /// Get all supported codec names
    #[must_use]
    pub fn supported_codecs() -> Vec<&'static str> {
        vec![
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
            "OPUS",
            #[cfg(feature = "amr-nb")]
            "AMR",
            #[cfg(feature = "amr-wb")]
            "AMR-WB",
        ]
    }

    /// Check if a codec is supported
    #[must_use]
    pub fn is_supported(name: &str) -> bool {
        let normalized = normalize_codec_name(name);
        match normalized.as_str() {
            #[cfg(feature = "g711")]
            "PCMU" | "PCMA" => true,
            #[cfg(feature = "g729")]
            "G729" | "G729A" | "G729AB" | "G729BA" => true,
            #[cfg(feature = "opus")]
            "OPUS" => true,
            #[cfg(feature = "amr-nb")]
            "AMR" => true,
            #[cfg(feature = "amr-wb")]
            "AMR-WB" | "AMRWB" => true,
            _ => false,
        }
    }
}

fn normalize_codec_name(name: &str) -> String {
    name.to_ascii_uppercase().replace('.', "")
}

/// Codec registry for managing multiple codec instances
pub struct CodecRegistry {
    codecs: HashMap<String, Box<dyn AudioCodec>>,
}

impl CodecRegistry {
    /// Create a new empty registry
    #[must_use]
    pub fn new() -> Self {
        Self {
            codecs: HashMap::new(),
        }
    }

    /// Register a codec with a name
    pub fn register(&mut self, name: String, codec: Box<dyn AudioCodec>) {
        self.codecs.insert(name, codec);
    }

    /// Get a codec by name
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&dyn AudioCodec> {
        self.codecs.get(name).map(std::convert::AsRef::as_ref)
    }

    /// Get a mutable codec by name
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Box<dyn AudioCodec>> {
        self.codecs.get_mut(name)
    }

    /// Remove a codec by name
    pub fn remove(&mut self, name: &str) -> Option<Box<dyn AudioCodec>> {
        self.codecs.remove(name)
    }

    /// List all registered codec names
    #[must_use]
    pub fn list_codecs(&self) -> Vec<&String> {
        self.codecs.keys().collect()
    }

    /// Get the count of registered codecs
    #[must_use]
    pub fn len(&self) -> usize {
        self.codecs.len()
    }

    /// Check if the registry is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.codecs.is_empty()
    }

    /// Clear all registered codecs
    pub fn clear(&mut self) {
        self.codecs.clear();
    }
}

impl Default for CodecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Codec capability information
#[derive(Debug, Clone)]
pub struct CodecCapabilities {
    /// Available codec types
    pub codec_types: Vec<CodecType>,
    /// Codec information
    pub codec_info: HashMap<CodecType, CodecInfo>,
}

/// Register the AMR variants enabled by feature flags.
///
/// Split out of [`CodecCapabilities::get_all`] to keep that function within the
/// workspace line limit as codecs accumulate.
// Both parameters go unused in a build with neither AMR feature enabled.
#[allow(unused_variables)]
fn add_amr_capabilities(
    codec_types: &mut Vec<CodecType>,
    codec_info: &mut HashMap<CodecType, CodecInfo>,
) {
    #[cfg(feature = "amr-nb")]
    {
        codec_types.push(CodecType::AmrNb);
        codec_info.insert(
            CodecType::AmrNb,
            CodecInfo {
                name: "AMR",
                sample_rate: 8000,
                channels: 1,
                bitrate: CodecType::AmrNb.default_bitrate(),
                frame_size: 160,
                payload_type: None,
            },
        );
    }

    #[cfg(feature = "amr-wb")]
    {
        codec_types.push(CodecType::AmrWb);
        codec_info.insert(
            CodecType::AmrWb,
            CodecInfo {
                name: "AMR-WB",
                sample_rate: 16000,
                channels: 1,
                bitrate: CodecType::AmrWb.default_bitrate(),
                frame_size: 320,
                payload_type: None,
            },
        );
    }
}

impl CodecCapabilities {
    /// Get capabilities for all supported codecs
    #[must_use]
    pub fn get_all() -> Self {
        // Both values are populated by feature-gated blocks. In a deliberately
        // codec-free build they remain empty and therefore need no mutation.
        #[allow(unused_mut)]
        let mut codec_types = Vec::new();
        #[allow(unused_mut)]
        let mut codec_info = HashMap::new();

        #[cfg(feature = "g711")]
        {
            codec_types.push(CodecType::G711Pcmu);
            codec_types.push(CodecType::G711Pcma);

            codec_info.insert(
                CodecType::G711Pcmu,
                CodecInfo {
                    name: "PCMU",
                    sample_rate: 8000,
                    channels: 1,
                    bitrate: 64000,
                    frame_size: 160,
                    payload_type: Some(0),
                },
            );

            codec_info.insert(
                CodecType::G711Pcma,
                CodecInfo {
                    name: "PCMA",
                    sample_rate: 8000,
                    channels: 1,
                    bitrate: 64000,
                    frame_size: 160,
                    payload_type: Some(8),
                },
            );
        }

        #[cfg(feature = "opus")]
        {
            codec_types.push(CodecType::Opus);
            codec_info.insert(
                CodecType::Opus,
                CodecInfo {
                    name: "opus",
                    sample_rate: 48000,
                    channels: 1,
                    bitrate: 64000,
                    frame_size: 960,
                    payload_type: None,
                },
            );
        }

        add_amr_capabilities(&mut codec_types, &mut codec_info);

        #[cfg(feature = "g729")]
        {
            codec_types.push(CodecType::G729);
            codec_types.push(CodecType::G729A);
            codec_types.push(CodecType::G729BA);

            codec_info.insert(
                CodecType::G729,
                CodecInfo {
                    name: "G729",
                    sample_rate: 8000,
                    channels: 1,
                    bitrate: 8000,
                    frame_size: 80,
                    payload_type: Some(18),
                },
            );
            codec_info.insert(
                CodecType::G729A,
                CodecInfo {
                    name: "G729A",
                    sample_rate: 8000,
                    channels: 1,
                    bitrate: 8000,
                    frame_size: 80,
                    payload_type: Some(18),
                },
            );
            codec_info.insert(
                CodecType::G729BA,
                CodecInfo {
                    name: "G729BA",
                    sample_rate: 8000,
                    channels: 1,
                    bitrate: 8000,
                    frame_size: 80,
                    payload_type: Some(18),
                },
            );
        }

        Self {
            codec_types,
            codec_info,
        }
    }

    /// Check if a codec type is supported
    #[must_use]
    pub fn is_supported(&self, codec_type: CodecType) -> bool {
        self.codec_types.contains(&codec_type)
    }

    /// Get information for a specific codec type
    #[must_use]
    pub fn get_info(&self, codec_type: CodecType) -> Option<&CodecInfo> {
        self.codec_info.get(&codec_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codec_factory_supported_codecs() {
        let supported = CodecFactory::supported_codecs();

        #[cfg(any(feature = "g711", feature = "g729", feature = "opus"))]
        assert!(!supported.is_empty());

        #[cfg(not(any(feature = "g711", feature = "g729", feature = "opus")))]
        assert!(supported.is_empty());

        #[cfg(feature = "g711")]
        {
            assert!(supported.contains(&"PCMU"));
            assert!(supported.contains(&"PCMA"));
        }
    }

    #[test]
    fn test_codec_factory_is_supported() {
        #[cfg(feature = "g711")]
        {
            assert!(CodecFactory::is_supported("PCMU"));
            assert!(CodecFactory::is_supported("pcmu"));
            assert!(CodecFactory::is_supported("PCMA"));
        }

        assert!(!CodecFactory::is_supported("UNSUPPORTED"));
        assert!(!CodecFactory::is_supported("G722"));

        #[cfg(feature = "opus")]
        for name in ["opus", "Opus", "OPUS"] {
            assert!(CodecFactory::is_supported(name));
        }
        #[cfg(not(feature = "opus"))]
        for name in ["opus", "Opus", "OPUS"] {
            assert!(!CodecFactory::is_supported(name));
        }
    }

    #[test]
    fn test_codec_registry() {
        let mut registry = CodecRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);

        #[cfg(feature = "g711")]
        {
            let config = CodecConfig::g711_pcmu();
            let codec = CodecFactory::create(config).unwrap();
            registry.register("test_pcmu".to_string(), codec);

            assert_eq!(registry.len(), 1);
            assert!(!registry.is_empty());
            assert!(registry.get("test_pcmu").is_some());
        }

        registry.clear();
        assert!(registry.is_empty());
    }

    #[test]
    fn test_codec_capabilities() {
        let caps = CodecCapabilities::get_all();

        #[cfg(any(feature = "g711", feature = "g729", feature = "opus"))]
        {
            assert!(!caps.codec_types.is_empty());
            assert!(!caps.codec_info.is_empty());
        }

        #[cfg(not(any(feature = "g711", feature = "g729", feature = "opus")))]
        {
            assert!(caps.codec_types.is_empty());
            assert!(caps.codec_info.is_empty());
        }

        #[cfg(feature = "g711")]
        {
            assert!(caps.is_supported(CodecType::G711Pcmu));
            assert!(caps.get_info(CodecType::G711Pcmu).is_some());
        }
    }

    #[test]
    #[cfg(feature = "g711")]
    fn test_codec_creation() {
        let config = CodecConfig::g711_pcmu();
        let codec = CodecFactory::create(config);
        assert!(codec.is_ok());

        let codec = codec.unwrap();
        let info = codec.info();
        assert_eq!(info.name, "PCMU");
        assert_eq!(info.sample_rate, 8000);
    }

    #[test]
    #[cfg(feature = "g711")]
    fn test_codec_creation_by_name() {
        let config = CodecConfig::new(CodecType::G711Pcmu);
        let codec = CodecFactory::create_by_name("PCMU", config.clone());
        assert!(codec.is_ok());

        let codec = CodecFactory::create_by_name("UNKNOWN", config);
        assert!(codec.is_err());
    }

    #[test]
    #[cfg(feature = "g711")]
    fn test_codec_creation_by_payload_type() {
        let config = CodecConfig::new(CodecType::G711Pcmu);
        let codec = CodecFactory::create_by_payload_type(0, config.clone());
        assert!(codec.is_ok());

        let codec = CodecFactory::create_by_payload_type(255, config);
        assert!(codec.is_err());
    }
}
