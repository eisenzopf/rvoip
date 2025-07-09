//! Codec Mapping Utilities
//!
//! This module provides bidirectional mapping between codec names and RTP payload types,
//! supporting both static (RFC 3551) and dynamic payload type assignments.

use std::collections::HashMap;
use tracing::{debug, warn};

/// Bidirectional mapper between codec names and RTP payload types
#[derive(Debug, Clone)]
pub struct CodecMapper {
    /// Mapping from codec name to payload type
    name_to_payload: HashMap<String, u8>,
    /// Mapping from payload type to codec name
    payload_to_name: HashMap<u8, String>,
    /// Clock rates for each codec
    codec_clock_rates: HashMap<String, u32>,
}

impl CodecMapper {
    /// Create a new codec mapper with standard RFC 3551 mappings
    pub fn new() -> Self {
        let mut mapper = Self {
            name_to_payload: HashMap::new(),
            payload_to_name: HashMap::new(),
            codec_clock_rates: HashMap::new(),
        };
        
        // Register standard static payload types (RFC 3551)
        mapper.register_static_codec("PCMU", 0, 8000);
        mapper.register_static_codec("PCMA", 8, 8000);
        mapper.register_static_codec("G729", 18, 8000);
        
        // Register common dynamic payload types
        mapper.register_dynamic_codec("opus".to_string(), 111, 48000);
        mapper.register_dynamic_codec("Opus".to_string(), 111, 48000); // Case variant
        
        debug!("Initialized CodecMapper with {} codecs", mapper.name_to_payload.len());
        mapper
    }
    
    /// Register a static codec (RFC 3551 defined)
    fn register_static_codec(&mut self, name: &str, payload_type: u8, clock_rate: u32) {
        let name_string = name.to_string();
        self.name_to_payload.insert(name_string.clone(), payload_type);
        self.payload_to_name.insert(payload_type, name_string.clone());
        self.codec_clock_rates.insert(name_string, clock_rate);
        
        debug!("Registered static codec: {} -> PT:{} @ {}Hz", name, payload_type, clock_rate);
    }
    
    /// Register a dynamic codec with custom payload type
    pub fn register_dynamic_codec(&mut self, name: String, payload_type: u8, clock_rate: u32) {
        // Check for conflicts with static payload types
        if payload_type < 96 && self.payload_to_name.contains_key(&payload_type) {
            warn!("Attempting to register dynamic codec '{}' with static payload type {}", name, payload_type);
            return;
        }
        
        // Remove any existing registration for this name or payload type
        if let Some(old_payload) = self.name_to_payload.get(&name) {
            self.payload_to_name.remove(old_payload);
        }
        if let Some(old_name) = self.payload_to_name.get(&payload_type) {
            self.name_to_payload.remove(old_name);
            self.codec_clock_rates.remove(old_name);
        }
        
        // Register the new mapping
        self.name_to_payload.insert(name.clone(), payload_type);
        self.payload_to_name.insert(payload_type, name.clone());
        self.codec_clock_rates.insert(name.clone(), clock_rate);
        
        debug!("Registered dynamic codec: {} -> PT:{} @ {}Hz", name, payload_type, clock_rate);
    }
    
    /// Get payload type for a codec name
    pub fn codec_to_payload(&self, codec_name: &str) -> Option<u8> {
        let result = self.name_to_payload.get(codec_name).copied();
        
        if result.is_none() {
            // Try case-insensitive lookup
            for (name, payload) in &self.name_to_payload {
                if name.eq_ignore_ascii_case(codec_name) {
                    debug!("Found codec '{}' via case-insensitive match to '{}'", codec_name, name);
                    return Some(*payload);
                }
            }
            debug!("No payload type found for codec: '{}'", codec_name);
        }
        
        result
    }
    
    /// Get codec name for a payload type
    pub fn payload_to_codec(&self, payload_type: u8) -> Option<String> {
        let result = self.payload_to_name.get(&payload_type).cloned();
        
        if result.is_none() {
            debug!("No codec name found for payload type: {}", payload_type);
        }
        
        result
    }
    
    /// Get the RTP clock rate for a codec
    pub fn get_clock_rate(&self, codec_name: &str) -> u32 {
        // First try exact match
        if let Some(&clock_rate) = self.codec_clock_rates.get(codec_name) {
            return clock_rate;
        }
        
        // Try case-insensitive match
        for (name, &clock_rate) in &self.codec_clock_rates {
            if name.eq_ignore_ascii_case(codec_name) {
                debug!("Found clock rate for '{}' via case-insensitive match to '{}'", codec_name, name);
                return clock_rate;
            }
        }
        
        // Fallback based on common codec knowledge
        let fallback_rate = match codec_name.to_lowercase().as_str() {
            "pcmu" | "pcma" | "g711" => 8000,
            "g729" => 8000,
            "opus" => 48000,
            "ilbc" => 8000,
            "speex" => 8000,
            _ => 8000, // Default to 8kHz for telephony
        };
        
        warn!("No clock rate registered for codec '{}', using fallback: {}Hz", codec_name, fallback_rate);
        fallback_rate
    }
    
    /// Get all registered codec names
    pub fn get_registered_codecs(&self) -> Vec<String> {
        self.name_to_payload.keys().cloned().collect()
    }
    
    /// Get all registered payload types
    pub fn get_registered_payload_types(&self) -> Vec<u8> {
        let mut payload_types: Vec<u8> = self.payload_to_name.keys().copied().collect();
        payload_types.sort();
        payload_types
    }
    
    /// Check if a codec is registered
    pub fn is_codec_registered(&self, codec_name: &str) -> bool {
        self.codec_to_payload(codec_name).is_some()
    }
    
    /// Check if a payload type is registered
    pub fn is_payload_type_registered(&self, payload_type: u8) -> bool {
        self.payload_to_name.contains_key(&payload_type)
    }
    
    /// Get codec information as a formatted string
    pub fn get_codec_info(&self, codec_name: &str) -> Option<String> {
        if let Some(payload_type) = self.codec_to_payload(codec_name) {
            let clock_rate = self.get_clock_rate(codec_name);
            Some(format!("{} (PT:{}, {}Hz)", codec_name, payload_type, clock_rate))
        } else {
            None
        }
    }
    
    /// Remove a dynamic codec registration
    pub fn unregister_codec(&mut self, codec_name: &str) -> bool {
        if let Some(payload_type) = self.name_to_payload.remove(codec_name) {
            self.payload_to_name.remove(&payload_type);
            self.codec_clock_rates.remove(codec_name);
            debug!("Unregistered codec: {}", codec_name);
            true
        } else {
            false
        }
    }
    
    /// Clear all dynamic codec registrations (keeps static ones)
    pub fn clear_dynamic_codecs(&mut self) {
        let static_payload_types = [0u8, 8, 18]; // PCMU, PCMA, G729
        
        // Collect codecs to remove (those not using static payload types)
        let codecs_to_remove: Vec<String> = self.name_to_payload
            .iter()
            .filter(|(_, &payload_type)| !static_payload_types.contains(&payload_type))
            .map(|(name, _)| name.clone())
            .collect();
        
        // Remove them
        for codec_name in codecs_to_remove {
            self.unregister_codec(&codec_name);
        }
        
        debug!("Cleared all dynamic codec registrations");
    }
}

impl Default for CodecMapper {
    fn default() -> Self {
        Self::new()
    }
}

/// Codec capability information
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecCapability {
    /// Codec name
    pub name: String,
    /// RTP payload type
    pub payload_type: u8,
    /// RTP clock rate
    pub clock_rate: u32,
    /// Whether this is a static (RFC 3551) or dynamic payload type
    pub is_static: bool,
}

impl CodecMapper {
    /// Get codec capability information
    pub fn get_codec_capability(&self, codec_name: &str) -> Option<CodecCapability> {
        let payload_type = self.codec_to_payload(codec_name)?;
        let clock_rate = self.get_clock_rate(codec_name);
        let is_static = payload_type < 96;
        
        Some(CodecCapability {
            name: codec_name.to_string(),
            payload_type,
            clock_rate,
            is_static,
        })
    }
    
    /// Get all registered codec capabilities
    pub fn get_all_capabilities(&self) -> Vec<CodecCapability> {
        self.name_to_payload
            .keys()
            .filter_map(|name| self.get_codec_capability(name))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_static_codec_mappings() {
        let mapper = CodecMapper::new();
        
        // Test PCMU
        assert_eq!(mapper.codec_to_payload("PCMU"), Some(0));
        assert_eq!(mapper.payload_to_codec(0), Some("PCMU".to_string()));
        assert_eq!(mapper.get_clock_rate("PCMU"), 8000);
        
        // Test PCMA
        assert_eq!(mapper.codec_to_payload("PCMA"), Some(8));
        assert_eq!(mapper.payload_to_codec(8), Some("PCMA".to_string()));
        assert_eq!(mapper.get_clock_rate("PCMA"), 8000);
        

        
        // Test G729
        assert_eq!(mapper.codec_to_payload("G729"), Some(18));
        assert_eq!(mapper.payload_to_codec(18), Some("G729".to_string()));
        assert_eq!(mapper.get_clock_rate("G729"), 8000);
    }
    
    #[test]
    fn test_dynamic_codec_mappings() {
        let mapper = CodecMapper::new();
        
        // Test Opus (both cases)
        assert_eq!(mapper.codec_to_payload("opus"), Some(111));
        assert_eq!(mapper.codec_to_payload("Opus"), Some(111));
        assert_eq!(mapper.payload_to_codec(111), Some("Opus".to_string())); // Latest registration wins
        assert_eq!(mapper.get_clock_rate("opus"), 48000);
    }
    
    #[test]
    fn test_case_insensitive_lookup() {
        let mapper = CodecMapper::new();
        
        // Test case variations
        assert_eq!(mapper.codec_to_payload("pcmu"), Some(0));
        assert_eq!(mapper.codec_to_payload("PCMU"), Some(0));
        assert_eq!(mapper.codec_to_payload("PcMu"), Some(0));
        
        assert_eq!(mapper.get_clock_rate("pcmu"), 8000);
        assert_eq!(mapper.get_clock_rate("PCMU"), 8000);
    }
    
    #[test]
    fn test_unknown_codec_handling() {
        let mapper = CodecMapper::new();
        
        // Unknown codec
        assert_eq!(mapper.codec_to_payload("unknown"), None);
        assert_eq!(mapper.payload_to_codec(99), None);
        
        // Fallback clock rate
        assert_eq!(mapper.get_clock_rate("unknown"), 8000);
    }
    
    #[test]
    fn test_dynamic_codec_registration() {
        let mut mapper = CodecMapper::new();
        
        // Register custom codec
        mapper.register_dynamic_codec("custom".to_string(), 96, 16000);
        
        assert_eq!(mapper.codec_to_payload("custom"), Some(96));
        assert_eq!(mapper.payload_to_codec(96), Some("custom".to_string()));
        assert_eq!(mapper.get_clock_rate("custom"), 16000);
        
        // Test overriding
        mapper.register_dynamic_codec("custom2".to_string(), 96, 32000);
        
        assert_eq!(mapper.codec_to_payload("custom"), None); // Old registration removed
        assert_eq!(mapper.codec_to_payload("custom2"), Some(96));
        assert_eq!(mapper.get_clock_rate("custom2"), 32000);
    }
    
    #[test]
    fn test_codec_capability() {
        let mapper = CodecMapper::new();
        
        let pcmu_cap = mapper.get_codec_capability("PCMU").unwrap();
        assert_eq!(pcmu_cap.name, "PCMU");
        assert_eq!(pcmu_cap.payload_type, 0);
        assert_eq!(pcmu_cap.clock_rate, 8000);
        assert!(pcmu_cap.is_static);
        
        let opus_cap = mapper.get_codec_capability("opus").unwrap();
        assert_eq!(opus_cap.name, "opus");
        assert_eq!(opus_cap.payload_type, 111);
        assert_eq!(opus_cap.clock_rate, 48000);
        assert!(!opus_cap.is_static);
    }
    
    #[test]
    fn test_codec_info_string() {
        let mapper = CodecMapper::new();
        
        assert_eq!(mapper.get_codec_info("PCMU"), Some("PCMU (PT:0, 8000Hz)".to_string()));
        assert_eq!(mapper.get_codec_info("opus"), Some("opus (PT:111, 48000Hz)".to_string()));
        assert_eq!(mapper.get_codec_info("unknown"), None);
    }
    
    #[test]
    fn test_registration_queries() {
        let mapper = CodecMapper::new();
        
        assert!(mapper.is_codec_registered("PCMU"));
        assert!(mapper.is_codec_registered("opus"));
        assert!(!mapper.is_codec_registered("unknown"));
        
        assert!(mapper.is_payload_type_registered(0));
        assert!(mapper.is_payload_type_registered(111));
        assert!(!mapper.is_payload_type_registered(99));
    }
    
    #[test]
    fn test_clear_dynamic_codecs() {
        let mut mapper = CodecMapper::new();
        
        // Add some dynamic codecs
        mapper.register_dynamic_codec("test1".to_string(), 96, 8000);
        mapper.register_dynamic_codec("test2".to_string(), 97, 16000);
        
        assert!(mapper.is_codec_registered("test1"));
        assert!(mapper.is_codec_registered("test2"));
        assert!(mapper.is_codec_registered("PCMU")); // Static should remain
        
        // Clear dynamic codecs
        mapper.clear_dynamic_codecs();
        
        assert!(!mapper.is_codec_registered("test1"));
        assert!(!mapper.is_codec_registered("test2"));
        assert!(mapper.is_codec_registered("PCMU")); // Static should remain
        assert!(!mapper.is_codec_registered("opus")); // This was cleared too since it's dynamic
    }
} 