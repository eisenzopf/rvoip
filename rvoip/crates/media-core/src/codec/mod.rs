//! Codec Framework for Media Processing
//!
//! This module provides a minimal codec framework for basic SIP functionality.
//! It focuses on G.711 codec support (PCMU/PCMA) for basic voice over IP.

pub mod audio;
pub mod transcoding;  // Add transcoding module

// Re-export audio codec types
pub use audio::common::*;

// Re-export G.711 codecs from our relay module
pub use crate::relay::{G711PcmuCodec, G711PcmaCodec};

// Re-export transcoding types
pub use transcoding::{Transcoder, TranscodingPath, TranscodingStats};

/// Basic codec trait for RTP payload processing
pub trait Codec: Send + Sync {
    /// Get the RTP payload type for this codec
    fn payload_type(&self) -> u8;
    
    /// Get the codec name
    fn name(&self) -> &'static str;
    
    /// Process RTP payload data (passthrough for basic relay)
    fn process_payload(&self, payload: &[u8]) -> crate::Result<Vec<u8>>;
}

/// Simple codec registry for G.711 codecs
pub struct CodecRegistry {
    codecs: std::collections::HashMap<u8, Box<dyn Codec>>,
}

impl CodecRegistry {
    /// Create a new codec registry with G.711 codecs
    pub fn new() -> Self {
        let mut codecs: std::collections::HashMap<u8, Box<dyn Codec>> = std::collections::HashMap::new();
        
        // Register G.711 codecs
        codecs.insert(0, Box::new(G711PcmuCodec::new()));  // PCMU
        codecs.insert(8, Box::new(G711PcmaCodec::new()));  // PCMA
        
        Self { codecs }
    }
    
    /// Get a codec by payload type
    pub fn get_codec(&self, payload_type: u8) -> Option<&dyn Codec> {
        self.codecs.get(&payload_type).map(|c| c.as_ref())
    }
    
    /// Check if a payload type is supported
    pub fn supports_payload_type(&self, payload_type: u8) -> bool {
        self.codecs.contains_key(&payload_type)
    }
    
    /// Get all supported payload types
    pub fn supported_payload_types(&self) -> Vec<u8> {
        self.codecs.keys().copied().collect()
    }
}

impl Default for CodecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Implement Codec trait for our G.711 codecs
impl Codec for G711PcmuCodec {
    fn payload_type(&self) -> u8 {
        G711PcmuCodec::payload_type(self)
    }
    
    fn name(&self) -> &'static str {
        G711PcmuCodec::name(self)
    }
    
    fn process_payload(&self, payload: &[u8]) -> crate::Result<Vec<u8>> {
        G711PcmuCodec::process_packet(self, payload).map(|bytes| bytes.to_vec())
    }
}

impl Codec for G711PcmaCodec {
    fn payload_type(&self) -> u8 {
        G711PcmaCodec::payload_type(self)
    }
    
    fn name(&self) -> &'static str {
        G711PcmaCodec::name(self)
    }
    
    fn process_payload(&self, payload: &[u8]) -> crate::Result<Vec<u8>> {
        G711PcmaCodec::process_packet(self, payload).map(|bytes| bytes.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_codec_registry() {
        let registry = CodecRegistry::new();
        
        // Test PCMU codec
        assert!(registry.supports_payload_type(0));
        let pcmu = registry.get_codec(0).unwrap();
        assert_eq!(pcmu.payload_type(), 0);
        assert_eq!(pcmu.name(), "PCMU");
        
        // Test PCMA codec
        assert!(registry.supports_payload_type(8));
        let pcma = registry.get_codec(8).unwrap();
        assert_eq!(pcma.payload_type(), 8);
        assert_eq!(pcma.name(), "PCMA");
        
        // Test unsupported codec
        assert!(!registry.supports_payload_type(96));
        assert!(registry.get_codec(96).is_none());
    }
    
    #[test]
    fn test_supported_payload_types() {
        let registry = CodecRegistry::new();
        let mut types = registry.supported_payload_types();
        types.sort();
        assert_eq!(types, vec![0, 8]);
    }
} 