//! Codec implementations and factory

use crate::error::{CodecError, Result};
use crate::types::{AudioCodec, CodecConfig, CodecInfo, CodecType};
use std::collections::HashMap;

// Codec implementations
#[cfg(feature = "g711")]
pub mod g711;

#[cfg(feature = "g722")]
pub mod g722;

#[cfg(any(feature = "g729", feature = "g729-sim"))]
pub mod g729;

#[cfg(any(feature = "opus", feature = "opus-sim"))]
pub mod opus;

/// Codec factory for creating codec instances
pub struct CodecFactory;

impl CodecFactory {
    /// Create a codec instance from configuration
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
            
            #[cfg(feature = "g722")]
            CodecType::G722 => {
                let codec = g722::G722Codec::new(config)?;
                Ok(Box::new(codec))
            }
            
            #[cfg(any(feature = "g729", feature = "g729-sim"))]
            CodecType::G729 => {
                let codec = g729::G729Codec::new(config)?;
                Ok(Box::new(codec))
            }
            
            #[cfg(any(feature = "opus", feature = "opus-sim"))]
            CodecType::Opus => {
                let codec = opus::OpusCodec::new(config)?;
                Ok(Box::new(codec))
            }
            
            codec_type => Err(CodecError::feature_not_enabled(format!(
                "Codec {} not enabled in build features",
                codec_type.name()
            ))),
        }
    }
    
    /// Create a codec by name
    pub fn create_by_name(name: &str, config: CodecConfig) -> Result<Box<dyn AudioCodec>> {
        let codec_type = match name.to_uppercase().as_str() {
            "PCMU" => CodecType::G711Pcmu,
            "PCMA" => CodecType::G711Pcma,
            "G722" => CodecType::G722,
            "G729" => CodecType::G729,
            "OPUS" => CodecType::Opus,
            _ => return Err(CodecError::unsupported_codec(name)),
        };
        
        let config = CodecConfig {
            codec_type,
            ..config
        };
        
        Self::create(config)
    }
    
    /// Create a codec by RTP payload type
    pub fn create_by_payload_type(payload_type: u8, config: CodecConfig) -> Result<Box<dyn AudioCodec>> {
        let codec_type = match payload_type {
            0 => CodecType::G711Pcmu,
            8 => CodecType::G711Pcma,
            9 => CodecType::G722,
            18 => CodecType::G729,
            _ => return Err(CodecError::unsupported_codec(format!("PT{}", payload_type))),
        };
        
        let config = CodecConfig {
            codec_type,
            ..config
        };
        
        Self::create(config)
    }
    
    /// Get all supported codec names
    pub fn supported_codecs() -> Vec<&'static str> {
        vec![
            #[cfg(feature = "g711")]
            "PCMU",
            #[cfg(feature = "g711")]
            "PCMA",
            #[cfg(feature = "g722")]
            "G722",
            #[cfg(any(feature = "g729", feature = "g729-sim"))]
            "G729",
            #[cfg(any(feature = "opus", feature = "opus-sim"))]
            "OPUS",
        ]
    }
    
    /// Check if a codec is supported
    pub fn is_supported(name: &str) -> bool {
        Self::supported_codecs().contains(&name.to_uppercase().as_str())
    }
}

/// Codec registry for managing multiple codec instances
pub struct CodecRegistry {
    codecs: HashMap<String, Box<dyn AudioCodec>>,
}

impl CodecRegistry {
    /// Create a new empty registry
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
    pub fn get(&self, name: &str) -> Option<&dyn AudioCodec> {
        self.codecs.get(name).map(|codec| codec.as_ref())
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
    pub fn list_codecs(&self) -> Vec<&String> {
        self.codecs.keys().collect()
    }
    
    /// Get the count of registered codecs
    pub fn len(&self) -> usize {
        self.codecs.len()
    }
    
    /// Check if the registry is empty
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

impl CodecCapabilities {
    /// Get capabilities for all supported codecs
    pub fn get_all() -> Self {
        let mut codec_types = Vec::new();
        let mut codec_info = HashMap::new();
        
        #[cfg(feature = "g711")]
        {
            codec_types.push(CodecType::G711Pcmu);
            codec_types.push(CodecType::G711Pcma);
            
            codec_info.insert(CodecType::G711Pcmu, CodecInfo {
                name: "PCMU",
                sample_rate: 8000,
                channels: 1,
                bitrate: 64000,
                frame_size: 160,
                payload_type: Some(0),
            });
            
            codec_info.insert(CodecType::G711Pcma, CodecInfo {
                name: "PCMA",
                sample_rate: 8000,
                channels: 1,
                bitrate: 64000,
                frame_size: 160,
                payload_type: Some(8),
            });
        }
        
        #[cfg(feature = "g722")]
        {
            codec_types.push(CodecType::G722);
            codec_info.insert(CodecType::G722, CodecInfo {
                name: "G722",
                sample_rate: 16000,
                channels: 1,
                bitrate: 64000,
                frame_size: 320,
                payload_type: Some(9),
            });
        }
        
        #[cfg(any(feature = "g729", feature = "g729-sim"))]
        {
            codec_types.push(CodecType::G729);
            codec_info.insert(CodecType::G729, CodecInfo {
                name: "G729",
                sample_rate: 8000,
                channels: 1,
                bitrate: 8000,
                frame_size: 80,
                payload_type: Some(18),
            });
        }
        
        #[cfg(any(feature = "opus", feature = "opus-sim"))]
        {
            codec_types.push(CodecType::Opus);
            codec_info.insert(CodecType::Opus, CodecInfo {
                name: "opus",
                sample_rate: 48000,
                channels: 1,
                bitrate: 64000,
                frame_size: 960,
                payload_type: None,
            });
        }
        
        Self {
            codec_types,
            codec_info,
        }
    }
    
    /// Check if a codec type is supported
    pub fn is_supported(&self, codec_type: CodecType) -> bool {
        self.codec_types.contains(&codec_type)
    }
    
    /// Get information for a specific codec type
    pub fn get_info(&self, codec_type: CodecType) -> Option<&CodecInfo> {
        self.codec_info.get(&codec_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SampleRate;

    #[test]
    fn test_codec_factory_supported_codecs() {
        let supported = CodecFactory::supported_codecs();
        assert!(!supported.is_empty());
        
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
        assert!(!caps.codec_types.is_empty());
        assert!(!caps.codec_info.is_empty());
        
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