//! Payload format registry for media processing (moved from rtp-core)
//!
//! This module provides a centralized registry for payload format handlers
//! focused on media processing capabilities.

use std::collections::HashMap;
use std::sync::OnceLock;
use super::traits::{PayloadFormat, PayloadFormatFactory};
use super::{G711UPayloadFormat, G711APayloadFormat, G722PayloadFormat, OpusPayloadFormat, Vp8PayloadFormat, Vp9PayloadFormat};

/// Global payload format registry
static PAYLOAD_REGISTRY: OnceLock<PayloadFormatRegistry> = OnceLock::new();

/// Media type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Audio,
    Video,
    Data,
}

/// Payload type information for media processing
#[derive(Debug, Clone)]
pub struct PayloadTypeInfo {
    /// Payload type number
    pub payload_type: u8,
    /// Media type classification
    pub media_type: MediaType,
    /// Codec name
    pub codec_name: String,
    /// Default clock rate
    pub clock_rate: u32,
    /// Number of channels (for audio)
    pub channels: Option<u8>,
    /// Whether this is a static or dynamic payload type
    pub is_static: bool,
}

/// Factory for creating standard payload format handlers
#[derive(Clone)]
pub struct StandardPayloadFormatFactory;

impl PayloadFormatFactory for StandardPayloadFormatFactory {
    fn create_format(&self, payload_type: u8, clock_rate: u32) -> Option<Box<dyn PayloadFormat>> {
        match payload_type {
            0 => Some(Box::new(G711UPayloadFormat::new(clock_rate))),
            8 => Some(Box::new(G711APayloadFormat::new(clock_rate))),
            9 => Some(Box::new(G722PayloadFormat::new(clock_rate))),
            96..=127 => {
                // Dynamic payload types - we need to guess based on common usage
                match payload_type {
                    96 => Some(Box::new(OpusPayloadFormat::new(payload_type, 2))), // Assume stereo Opus
                    97 => Some(Box::new(Vp8PayloadFormat::new(payload_type))),
                    98 => Some(Box::new(Vp9PayloadFormat::new(payload_type))),
                    111 => Some(Box::new(OpusPayloadFormat::new(payload_type, 2))), // Common WebRTC Opus PT
                    _ => None, // Unknown dynamic type
                }
            }
            _ => None,
        }
    }
    
    fn can_handle(&self, payload_type: u8) -> bool {
        matches!(payload_type, 0 | 8 | 9 | 96..=127)
    }
}

/// Registry for payload format handlers and information
pub struct PayloadFormatRegistry {
    /// Registered payload type information
    payload_info: HashMap<u8, PayloadTypeInfo>,
    /// Registered format factories
    factories: HashMap<u8, Box<dyn PayloadFormatFactory>>,
}

impl PayloadFormatRegistry {
    /// Create a new registry with standard payload types
    pub fn new() -> Self {
        let mut registry = Self {
            payload_info: HashMap::new(),
            factories: HashMap::new(),
        };
        
        registry.populate_standard_types();
        registry.register_standard_factories();
        registry
    }
    
    /// Populate standard RFC 3551 payload types relevant to media processing
    fn populate_standard_types(&mut self) {
        // Audio payload types
        let audio_types = [
            (0, "PCMU", 8000, Some(1)),
            (3, "GSM", 8000, Some(1)),
            (4, "G723", 8000, Some(1)),
            (8, "PCMA", 8000, Some(1)),
            (9, "G722", 8000, Some(1)), // Note: RTP clock rate is 8kHz for G.722
            (10, "L16", 44100, Some(2)),
            (11, "L16", 44100, Some(1)),
            (15, "G728", 8000, Some(1)),
            (18, "G729", 8000, Some(1)),
        ];
        
        for (pt, name, clock_rate, channels) in audio_types {
            self.payload_info.insert(pt, PayloadTypeInfo {
                payload_type: pt,
                media_type: MediaType::Audio,
                codec_name: name.to_string(),
                clock_rate,
                channels,
                is_static: true,
            });
        }
        
        // Video payload types
        let video_types = [
            (26, "JPEG", 90000, None),
            (31, "H261", 90000, None),
            (32, "MPV", 90000, None),
            (34, "H263", 90000, None),
        ];
        
        for (pt, name, clock_rate, channels) in video_types {
            self.payload_info.insert(pt, PayloadTypeInfo {
                payload_type: pt,
                media_type: MediaType::Video,
                codec_name: name.to_string(),
                clock_rate,
                channels,
                is_static: true,
            });
        }
        
        // Common dynamic payload types
        let dynamic_types = [
            (96, "H264", MediaType::Video, 90000, None),
            (97, "VP8", MediaType::Video, 90000, None),
            (98, "VP9", MediaType::Video, 90000, None),
            (111, "OPUS", MediaType::Audio, 48000, Some(2)),
        ];
        
        for (pt, name, media_type, clock_rate, channels) in dynamic_types {
            self.payload_info.insert(pt, PayloadTypeInfo {
                payload_type: pt,
                media_type,
                codec_name: name.to_string(),
                clock_rate,
                channels,
                is_static: false,
            });
        }
    }
    
    /// Register standard format factories
    fn register_standard_factories(&mut self) {
        let factory = Box::new(StandardPayloadFormatFactory) as Box<dyn PayloadFormatFactory>;
        
        // Register for payload types we can handle
        for pt in [0, 8, 9] {
            // Each factory needs to be separately boxed since Box<dyn Trait> isn't Clone
            self.factories.insert(pt, Box::new(StandardPayloadFormatFactory));
        }
    }
    
    /// Get payload type information
    pub fn get_payload_info(&self, payload_type: u8) -> Option<&PayloadTypeInfo> {
        self.payload_info.get(&payload_type)
    }
    
    /// Get media type for payload type
    pub fn get_media_type(&self, payload_type: u8) -> MediaType {
        if let Some(info) = self.get_payload_info(payload_type) {
            info.media_type
        } else {
            // RFC 3551 fallback
            match payload_type {
                0..=23 => MediaType::Audio,
                24..=34 => MediaType::Video,
                _ => MediaType::Data,
            }
        }
    }
    
    /// Get codec name for payload type
    pub fn get_codec_name(&self, payload_type: u8) -> String {
        if let Some(info) = self.get_payload_info(payload_type) {
            info.codec_name.clone()
        } else {
            format!("Unknown-{}", payload_type)
        }
    }
    
    /// Create a payload format handler
    pub fn create_format(&self, payload_type: u8, clock_rate: Option<u32>) -> Option<Box<dyn PayloadFormat>> {
        let clock_rate = clock_rate.or_else(|| {
            self.get_payload_info(payload_type).map(|info| info.clock_rate)
        })?;
        
        self.factories.get(&payload_type)?.create_format(payload_type, clock_rate)
    }
    
    /// Register a custom payload type
    pub fn register_payload_type(&mut self, info: PayloadTypeInfo) {
        self.payload_info.insert(info.payload_type, info);
    }
    
    /// Register a custom format factory
    pub fn register_factory(&mut self, payload_type: u8, factory: Box<dyn PayloadFormatFactory>) {
        self.factories.insert(payload_type, factory);
    }
    
    /// Check if payload type is dynamic (96-127)
    pub fn is_dynamic_payload_type(payload_type: u8) -> bool {
        payload_type >= 96 && payload_type <= 127
    }
    
    /// List all supported payload types
    pub fn supported_payload_types(&self) -> Vec<u8> {
        let mut types: Vec<u8> = self.factories.keys().copied().collect();
        types.sort();
        types
    }
}

/// Get the global payload format registry
pub fn get_global_registry() -> &'static PayloadFormatRegistry {
    PAYLOAD_REGISTRY.get_or_init(|| PayloadFormatRegistry::new())
}

/// Create a payload format handler (convenience function)
pub fn create_payload_format(payload_type: u8, clock_rate: Option<u32>) -> Option<Box<dyn PayloadFormat>> {
    get_global_registry().create_format(payload_type, clock_rate)
}

/// Get media type for payload type (convenience function)
pub fn get_media_type(payload_type: u8) -> MediaType {
    get_global_registry().get_media_type(payload_type)
}

/// Get codec name for payload type (convenience function)  
pub fn get_codec_name(payload_type: u8) -> String {
    get_global_registry().get_codec_name(payload_type)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_standard_audio_types() {
        let registry = PayloadFormatRegistry::new();
        
        // Test PCMU
        let pcmu_info = registry.get_payload_info(0).unwrap();
        assert_eq!(pcmu_info.codec_name, "PCMU");
        assert_eq!(pcmu_info.media_type, MediaType::Audio);
        assert_eq!(pcmu_info.clock_rate, 8000);
        
        // Test format creation
        let pcmu_format = registry.create_format(0, None).unwrap();
        assert_eq!(pcmu_format.payload_type(), 0);
        assert_eq!(pcmu_format.clock_rate(), 8000);
    }
    
    #[test]
    fn test_media_type_fallback() {
        let registry = PayloadFormatRegistry::new();
        
        // Test fallback for unregistered types
        assert_eq!(registry.get_media_type(1), MediaType::Audio); // Audio range
        assert_eq!(registry.get_media_type(27), MediaType::Video); // Video range
        assert_eq!(registry.get_media_type(50), MediaType::Data);  // Other
    }
    
    #[test]
    fn test_dynamic_payload_detection() {
        assert!(PayloadFormatRegistry::is_dynamic_payload_type(96));
        assert!(PayloadFormatRegistry::is_dynamic_payload_type(127));
        assert!(!PayloadFormatRegistry::is_dynamic_payload_type(0));
        assert!(!PayloadFormatRegistry::is_dynamic_payload_type(95));
    }
}