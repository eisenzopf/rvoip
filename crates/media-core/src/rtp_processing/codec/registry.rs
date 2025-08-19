//! Payload Type Registry (moved from rtp-core)
//!
//! This module provides a centralized, RFC 3551-compliant payload type registry
//! to replace hardcoded payload type logic across the codebase.

use std::collections::HashMap;
use std::sync::OnceLock;
use crate::api::types::MediaFrameType;

/// Payload type information
#[derive(Debug, Clone, PartialEq)]
pub struct PayloadTypeInfo {
    /// Payload type number
    pub payload_type: u8,
    /// Media type (Audio, Video, Data)
    pub media_type: MediaFrameType,
    /// Codec name (e.g., "PCMU", "H264", "VP8")
    pub codec_name: String,
    /// Clock rate in Hz
    pub clock_rate: u32,
    /// Number of channels (for audio)
    pub channels: Option<u8>,
    /// Whether this is a static or dynamic payload type
    pub is_dynamic: bool,
    /// RFC reference
    pub rfc_reference: Option<String>,
}

/// Global payload type registry
static PAYLOAD_REGISTRY: OnceLock<PayloadTypeRegistry> = OnceLock::new();

/// Payload type registry implementation
#[derive(Debug)]
pub struct PayloadTypeRegistry {
    /// Static payload types (0-95)
    static_types: HashMap<u8, PayloadTypeInfo>,
    /// Dynamic payload type mappings (96-127)
    dynamic_types: HashMap<u8, PayloadTypeInfo>,
}

impl PayloadTypeRegistry {
    /// Create a new registry with RFC 3551 static payload types
    pub fn new() -> Self {
        let mut registry = Self {
            static_types: HashMap::new(),
            dynamic_types: HashMap::new(),
        };
        
        registry.populate_rfc3551_static_types();
        registry
    }
    
    /// Populate RFC 3551 static payload types
    fn populate_rfc3551_static_types(&mut self) {
        // Audio payload types (RFC 3551)
        self.add_static_type(PayloadTypeInfo {
            payload_type: 0,
            media_type: MediaFrameType::Audio,
            codec_name: "PCMU".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 3,
            media_type: MediaFrameType::Audio,
            codec_name: "GSM".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 4,
            media_type: MediaFrameType::Audio,
            codec_name: "G723".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 5,
            media_type: MediaFrameType::Audio,
            codec_name: "DVI4".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 6,
            media_type: MediaFrameType::Audio,
            codec_name: "DVI4".to_string(),
            clock_rate: 16000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 7,
            media_type: MediaFrameType::Audio,
            codec_name: "LPC".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 8,
            media_type: MediaFrameType::Audio,
            codec_name: "PCMA".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 9,
            media_type: MediaFrameType::Audio,
            codec_name: "G722".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 10,
            media_type: MediaFrameType::Audio,
            codec_name: "L16".to_string(),
            clock_rate: 44100,
            channels: Some(2),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 11,
            media_type: MediaFrameType::Audio,
            codec_name: "L16".to_string(),
            clock_rate: 44100,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 12,
            media_type: MediaFrameType::Audio,
            codec_name: "QCELP".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 13,
            media_type: MediaFrameType::Audio,
            codec_name: "CN".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3389".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 14,
            media_type: MediaFrameType::Audio,
            codec_name: "MPA".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 15,
            media_type: MediaFrameType::Audio,
            codec_name: "G728".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 16,
            media_type: MediaFrameType::Audio,
            codec_name: "DVI4".to_string(),
            clock_rate: 11025,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 17,
            media_type: MediaFrameType::Audio,
            codec_name: "DVI4".to_string(),
            clock_rate: 22050,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 18,
            media_type: MediaFrameType::Audio,
            codec_name: "G729".to_string(),
            clock_rate: 8000,
            channels: Some(1),
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        // Video payload types (RFC 3551)
        self.add_static_type(PayloadTypeInfo {
            payload_type: 25,
            media_type: MediaFrameType::Video,
            codec_name: "CelB".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: false,
            rfc_reference: Some("RFC 2029".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 26,
            media_type: MediaFrameType::Video,
            codec_name: "JPEG".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: false,
            rfc_reference: Some("RFC 2435".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 28,
            media_type: MediaFrameType::Video,
            codec_name: "nv".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: false,
            rfc_reference: Some("RFC 3551".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 31,
            media_type: MediaFrameType::Video,
            codec_name: "H261".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: false,
            rfc_reference: Some("RFC 4587".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 32,
            media_type: MediaFrameType::Video,
            codec_name: "MPV".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: false,
            rfc_reference: Some("RFC 2250".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 33,
            media_type: MediaFrameType::Video,
            codec_name: "MP2T".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: false,
            rfc_reference: Some("RFC 2250".to_string()),
        });
        
        self.add_static_type(PayloadTypeInfo {
            payload_type: 34,
            media_type: MediaFrameType::Video,
            codec_name: "H263".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: false,
            rfc_reference: Some("RFC 4629".to_string()),
        });
        
        // Common dynamic payload types (these are just examples, actual mappings would be negotiated)
        self.add_dynamic_type(PayloadTypeInfo {
            payload_type: 96,
            media_type: MediaFrameType::Video,
            codec_name: "H264".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: true,
            rfc_reference: Some("RFC 6184".to_string()),
        });
        
        self.add_dynamic_type(PayloadTypeInfo {
            payload_type: 97,
            media_type: MediaFrameType::Video,
            codec_name: "VP8".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: true,
            rfc_reference: Some("RFC 7741".to_string()),
        });
        
        self.add_dynamic_type(PayloadTypeInfo {
            payload_type: 98,
            media_type: MediaFrameType::Video,
            codec_name: "VP9".to_string(),
            clock_rate: 90000,
            channels: None,
            is_dynamic: true,
            rfc_reference: Some("draft-ietf-payload-vp9".to_string()),
        });
        
        self.add_dynamic_type(PayloadTypeInfo {
            payload_type: 111,
            media_type: MediaFrameType::Audio,
            codec_name: "OPUS".to_string(),
            clock_rate: 48000,
            channels: Some(2),
            is_dynamic: true,
            rfc_reference: Some("RFC 7587".to_string()),
        });
    }
    
    /// Add a static payload type
    fn add_static_type(&mut self, info: PayloadTypeInfo) {
        self.static_types.insert(info.payload_type, info);
    }
    
    /// Add a dynamic payload type
    fn add_dynamic_type(&mut self, info: PayloadTypeInfo) {
        self.dynamic_types.insert(info.payload_type, info);
    }
    
    /// Register a dynamic payload type mapping
    pub fn register_dynamic_payload(&mut self, info: PayloadTypeInfo) -> Result<(), String> {
        if info.payload_type < 96 || info.payload_type > 127 {
            return Err(format!("Dynamic payload types must be in range 96-127, got {}", info.payload_type));
        }
        
        self.dynamic_types.insert(info.payload_type, info);
        Ok(())
    }
    
    /// Get payload type information
    pub fn get_payload_info(&self, payload_type: u8) -> Option<&PayloadTypeInfo> {
        // First check static types
        if let Some(info) = self.static_types.get(&payload_type) {
            return Some(info);
        }
        
        // Then check dynamic types
        self.dynamic_types.get(&payload_type)
    }
    
    /// Determine media frame type from payload type
    pub fn get_media_frame_type(&self, payload_type: u8) -> MediaFrameType {
        if let Some(info) = self.get_payload_info(payload_type) {
            info.media_type
        } else {
            // RFC 3551 fallback for unregistered payload types
            match payload_type {
                0..=23 => MediaFrameType::Audio,     // Audio range
                24..=34 => MediaFrameType::Video,    // Video range  
                35..=71 => MediaFrameType::Data,     // Unassigned - treat as data
                72..=76 => MediaFrameType::Data,     // Reserved for RTCP conflict avoidance
                77..=95 => MediaFrameType::Data,     // Unassigned - treat as data
                96..=127 => MediaFrameType::Data,    // Dynamic - default to data if not registered
                _ => MediaFrameType::Data,           // Invalid - treat as data
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
    
    /// Check if payload type is dynamic
    pub fn is_dynamic_payload_type(payload_type: u8) -> bool {
        payload_type >= 96 && payload_type <= 127
    }
    
    /// Check if payload type is static
    pub fn is_static_payload_type(payload_type: u8) -> bool {
        payload_type <= 95
    }
    
    /// List all registered payload types
    pub fn list_all_payload_types(&self) -> Vec<&PayloadTypeInfo> {
        let mut types: Vec<&PayloadTypeInfo> = self.static_types.values().collect();
        types.extend(self.dynamic_types.values());
        types.sort_by_key(|info| info.payload_type);
        types
    }
}

impl Default for PayloadTypeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the global payload type registry
pub fn get_global_registry() -> &'static PayloadTypeRegistry {
    PAYLOAD_REGISTRY.get_or_init(|| PayloadTypeRegistry::new())
}

/// Get media frame type for payload type (convenience function)
pub fn get_media_frame_type(payload_type: u8) -> MediaFrameType {
    get_global_registry().get_media_frame_type(payload_type)
}

/// Get codec name for payload type (convenience function)
pub fn get_codec_name(payload_type: u8) -> String {
    get_global_registry().get_codec_name(payload_type)
}

/// Get payload type information (convenience function)
pub fn get_payload_info(payload_type: u8) -> Option<&'static PayloadTypeInfo> {
    get_global_registry().get_payload_info(payload_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rfc3551_audio_types() {
        let registry = PayloadTypeRegistry::new();
        
        // Test PCMU
        let pcmu = registry.get_payload_info(0).unwrap();
        assert_eq!(pcmu.codec_name, "PCMU");
        assert_eq!(pcmu.media_type, MediaFrameType::Audio);
        assert_eq!(pcmu.clock_rate, 8000);
        
        // Test PCMA
        let pcma = registry.get_payload_info(8).unwrap();
        assert_eq!(pcma.codec_name, "PCMA");
        assert_eq!(pcma.media_type, MediaFrameType::Audio);
        assert_eq!(pcma.clock_rate, 8000);
    }

    #[test]
    fn test_rfc3551_video_types() {
        let registry = PayloadTypeRegistry::new();
        
        // Test H261
        let h261 = registry.get_payload_info(31).unwrap();
        assert_eq!(h261.codec_name, "H261");
        assert_eq!(h261.media_type, MediaFrameType::Video);
        assert_eq!(h261.clock_rate, 90000);
    }

    #[test]
    fn test_dynamic_types() {
        let registry = PayloadTypeRegistry::new();
        
        // Test H264 (common dynamic type)
        let h264 = registry.get_payload_info(96).unwrap();
        assert_eq!(h264.codec_name, "H264");
        assert_eq!(h264.media_type, MediaFrameType::Video);
        assert!(h264.is_dynamic);
    }

    #[test]
    fn test_media_frame_type_fallback() {
        let registry = PayloadTypeRegistry::new();
        
        // Test unregistered audio range
        assert_eq!(registry.get_media_frame_type(1), MediaFrameType::Audio);
        
        // Test unregistered video range
        assert_eq!(registry.get_media_frame_type(27), MediaFrameType::Video);
        
        // Test unassigned range
        assert_eq!(registry.get_media_frame_type(50), MediaFrameType::Data);
        
        // Test dynamic range (unregistered)
        assert_eq!(registry.get_media_frame_type(100), MediaFrameType::Data);
    }
}