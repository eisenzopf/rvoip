//! Codec Mapping Utilities
//!
//! This module provides bidirectional mapping between codec names and RTP payload types,
//! supporting both static (RFC 3551) and dynamic payload type assignments.

use std::collections::HashMap;
use tracing::{debug, warn};
use crate::types::payload_types::{static_types, dynamic_range};

/// Represents different Opus configurations that can use different payload types
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpusConfig {
    /// Sample rate (8000, 16000, 48000)
    pub sample_rate: u32,
    /// Number of channels (1 or 2)
    pub channels: u8,
    /// Maximum bitrate (optional, for differentiation)
    pub max_bitrate: Option<u32>,
    /// Whether FEC is enabled (optional, for differentiation)
    pub fec_enabled: Option<bool>,
}

impl OpusConfig {
    /// Create a new Opus configuration
    pub fn new(sample_rate: u32, channels: u8) -> Self {
        Self {
            sample_rate,
            channels,
            max_bitrate: None,
            fec_enabled: None,
        }
    }
    
    /// Create Opus configuration with all parameters
    pub fn with_params(sample_rate: u32, channels: u8, max_bitrate: Option<u32>, fec_enabled: Option<bool>) -> Self {
        Self {
            sample_rate,
            channels,
            max_bitrate,
            fec_enabled,
        }
    }
    
    /// Get a string representation for codec identification
    pub fn to_codec_string(&self) -> String {
        let mut codec_str = format!("opus@{}Hz", self.sample_rate);
        if self.channels > 1 {
            codec_str.push_str(&format!("@{}ch", self.channels));
        }
        if let Some(bitrate) = self.max_bitrate {
            codec_str.push_str(&format!("@{}bps", bitrate));
        }
        if let Some(fec) = self.fec_enabled {
            if fec {
                codec_str.push_str("@fec");
            }
        }
        codec_str
    }
    
    /// Parse codec string back to OpusConfig
    pub fn from_codec_string(codec_str: &str) -> Option<Self> {
        if !codec_str.starts_with("opus@") {
            return None;
        }
        
        let mut config = Self::new(48000, 1); // Default values
        
        for part in codec_str.split('@').skip(1) {
            if part.ends_with("Hz") {
                if let Ok(rate) = part.trim_end_matches("Hz").parse::<u32>() {
                    config.sample_rate = rate;
                }
            } else if part.ends_with("ch") {
                if let Ok(channels) = part.trim_end_matches("ch").parse::<u8>() {
                    config.channels = channels;
                }
            } else if part.ends_with("bps") {
                if let Ok(bitrate) = part.trim_end_matches("bps").parse::<u32>() {
                    config.max_bitrate = Some(bitrate);
                }
            } else if part == "fec" {
                config.fec_enabled = Some(true);
            }
        }
        
        Some(config)
    }
    
    /// Get standard clock rate for SDP (always 48000 for Opus per RFC 7587)
    pub fn get_sdp_clock_rate(&self) -> u32 {
        48000
    }
    
    /// Get actual encoding sample rate
    pub fn get_encoding_sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Bidirectional mapper between codec names and RTP payload types
#[derive(Debug, Clone)]
pub struct CodecMapper {
    /// Mapping from codec name to payload type
    name_to_payload: HashMap<String, u8>,
    /// Mapping from payload type to codec name
    payload_to_name: HashMap<u8, String>,
    /// Clock rates for each codec
    codec_clock_rates: HashMap<String, u32>,
    /// Opus configurations mapped by payload type
    opus_configs: HashMap<u8, OpusConfig>,
    /// Reverse mapping: OpusConfig to payload type
    opus_config_to_payload: HashMap<OpusConfig, u8>,
}

impl CodecMapper {
    /// Create a new codec mapper with standard RFC 3551 mappings
    pub fn new() -> Self {
        let mut mapper = Self {
            name_to_payload: HashMap::new(),
            payload_to_name: HashMap::new(),
            codec_clock_rates: HashMap::new(),
            opus_configs: HashMap::new(),
            opus_config_to_payload: HashMap::new(),
        };
        
        // Register standard static payload types (RFC 3551)
        mapper.register_static_codec("PCMU", static_types::PCMU, 8000);
        mapper.register_static_codec("PCMA", static_types::PCMA, 8000);
        // Note: G729 is commented out since we don't support it per user requirements
        // mapper.register_static_codec("G729", static_types::G729, 8000);
        
        // NOTE: Dynamic codecs like Opus should NOT be pre-registered with hardcoded payload types!
        // They must be registered with the actual negotiated payload type from SDP.
        // This was the source of the bug where SDP negotiated Opus(96) but system used Opus(111).
        
        debug!("Initialized CodecMapper with {} static codecs", mapper.name_to_payload.len());
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
        if !dynamic_range::is_dynamic(payload_type) && self.payload_to_name.contains_key(&payload_type) {
            warn!("Attempting to register dynamic codec '{}' with static payload type {} (should be in range {}-{})", 
                  name, payload_type, dynamic_range::DYNAMIC_START, dynamic_range::DYNAMIC_END);
            return;
        }
        
        // Remove any existing registration for this name or payload type
        if let Some(old_payload) = self.name_to_payload.get(&name) {
            self.payload_to_name.remove(old_payload);
            // Remove from Opus configs if it was an Opus codec
            if name.starts_with("opus") {
                self.opus_configs.remove(old_payload);
            }
        }
        if let Some(old_name) = self.payload_to_name.get(&payload_type) {
            self.name_to_payload.remove(old_name);
            self.codec_clock_rates.remove(old_name);
            // Remove from Opus configs if it was an Opus codec
            if old_name.starts_with("opus") {
                if let Some(config) = self.opus_configs.remove(&payload_type) {
                    self.opus_config_to_payload.remove(&config);
                }
            }
        }
        
        // Register the new mapping
        self.name_to_payload.insert(name.clone(), payload_type);
        self.payload_to_name.insert(payload_type, name.clone());
        self.codec_clock_rates.insert(name.clone(), clock_rate);
        
        debug!("Registered dynamic codec: {} -> PT:{} @ {}Hz", name, payload_type, clock_rate);
    }
    
    /// Register an Opus configuration with a specific payload type
    pub fn register_opus_config(&mut self, config: OpusConfig, payload_type: u8) {
        if !dynamic_range::is_dynamic(payload_type) {
            warn!("Attempting to register Opus config with non-dynamic payload type {}", payload_type);
            return;
        }
        
        let codec_name = config.to_codec_string();
        
        // Remove any existing registration for this payload type
        if let Some(old_config) = self.opus_configs.remove(&payload_type) {
            self.opus_config_to_payload.remove(&old_config);
            let old_name = old_config.to_codec_string();
            self.name_to_payload.remove(&old_name);
            self.codec_clock_rates.remove(&old_name);
        }
        
        // Remove any existing registration for this config
        if let Some(old_payload) = self.opus_config_to_payload.remove(&config) {
            self.opus_configs.remove(&old_payload);
            self.payload_to_name.remove(&old_payload);
        }
        
        // Register the new mapping
        self.opus_configs.insert(payload_type, config.clone());
        self.opus_config_to_payload.insert(config.clone(), payload_type);
        self.name_to_payload.insert(codec_name.clone(), payload_type);
        self.payload_to_name.insert(payload_type, codec_name.clone());
        self.codec_clock_rates.insert(codec_name.clone(), config.get_sdp_clock_rate());
        
        debug!("Registered Opus config: {} -> PT:{} @ {}Hz", 
               config.to_codec_string(), payload_type, config.get_sdp_clock_rate());
    }
    
    /// Get payload type for an Opus configuration
    pub fn get_opus_payload_type(&self, config: &OpusConfig) -> Option<u8> {
        self.opus_config_to_payload.get(config).copied()
    }
    
    /// Get Opus configuration for a payload type
    pub fn get_opus_config(&self, payload_type: u8) -> Option<&OpusConfig> {
        self.opus_configs.get(&payload_type)
    }
    
    /// Register multiple Opus configurations from SDP negotiation
    pub fn register_opus_from_sdp(&mut self, sdp_entries: &[(OpusConfig, u8)]) {
        for (config, payload_type) in sdp_entries {
            self.register_opus_config(config.clone(), *payload_type);
        }
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
            
            // Special handling for generic "opus" lookup
            if codec_name.eq_ignore_ascii_case("opus") {
                // Look for any registered Opus configuration
                for (name, payload) in &self.name_to_payload {
                    if name.starts_with("opus@") {
                        debug!("Found generic opus match: {} -> PT:{}", name, payload);
                        return Some(*payload);
                    }
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
        
        // Special handling for generic "opus" lookup
        if codec_name.eq_ignore_ascii_case("opus") {
            // Look for any registered Opus configuration
            for (name, &clock_rate) in &self.codec_clock_rates {
                if name.starts_with("opus@") {
                    debug!("Found generic opus clock rate: {} -> {}Hz", name, clock_rate);
                    return clock_rate;
                }
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
    
    /// Get all registered Opus configurations
    pub fn get_registered_opus_configs(&self) -> Vec<(OpusConfig, u8)> {
        self.opus_configs.iter()
            .map(|(payload_type, config)| (config.clone(), *payload_type))
            .collect()
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
            
            // Add Opus-specific information if available
            if let Some(config) = self.opus_configs.get(&payload_type) {
                Some(format!("{} (PT:{}, {}Hz, {}ch, encoding@{}Hz)", 
                           codec_name, payload_type, clock_rate, 
                           config.channels, config.get_encoding_sample_rate()))
            } else {
                Some(format!("{} (PT:{}, {}Hz)", codec_name, payload_type, clock_rate))
            }
        } else {
            None
        }
    }
    
    /// Remove a dynamic codec registration
    pub fn unregister_codec(&mut self, codec_name: &str) -> bool {
        if let Some(payload_type) = self.name_to_payload.remove(codec_name) {
            self.payload_to_name.remove(&payload_type);
            self.codec_clock_rates.remove(codec_name);
            
            // Remove from Opus configs if it was an Opus codec
            if codec_name.starts_with("opus") {
                if let Some(config) = self.opus_configs.remove(&payload_type) {
                    self.opus_config_to_payload.remove(&config);
                }
            }
            
            debug!("Unregistered codec: {}", codec_name);
            true
        } else {
            false
        }
    }
    
    /// Clear all dynamic codec registrations (keeps static ones)
    pub fn clear_dynamic_codecs(&mut self) {
        let static_payload_types = [static_types::PCMU, static_types::PCMA]; // Only supported static types
        
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
        
        // Clear Opus-specific mappings
        self.opus_configs.clear();
        self.opus_config_to_payload.clear();
        
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
    /// Opus-specific configuration (if applicable)
    pub opus_config: Option<OpusConfig>,
}

impl CodecMapper {
    /// Get codec capability information
    pub fn get_codec_capability(&self, codec_name: &str) -> Option<CodecCapability> {
        let payload_type = self.codec_to_payload(codec_name)?;
        let clock_rate = self.get_clock_rate(codec_name);
        let is_static = !dynamic_range::is_dynamic(payload_type);
        let opus_config = self.opus_configs.get(&payload_type).cloned();
        
        Some(CodecCapability {
            name: codec_name.to_string(),
            payload_type,
            clock_rate,
            is_static,
            opus_config,
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
        assert_eq!(mapper.codec_to_payload("PCMU"), Some(static_types::PCMU));
        assert_eq!(mapper.payload_to_codec(static_types::PCMU), Some("PCMU".to_string()));
        assert_eq!(mapper.get_clock_rate("PCMU"), 8000);
        
        // Test PCMA
        assert_eq!(mapper.codec_to_payload("PCMA"), Some(static_types::PCMA));
        assert_eq!(mapper.payload_to_codec(static_types::PCMA), Some("PCMA".to_string()));
        assert_eq!(mapper.get_clock_rate("PCMA"), 8000);
        
        // Note: G729 test removed since we don't support it per user requirements
    }
    
    #[test]
    fn test_opus_config_creation() {
        // Test basic Opus configuration
        let config = OpusConfig::new(48000, 2);
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 2);
        assert_eq!(config.max_bitrate, None);
        assert_eq!(config.fec_enabled, None);
        
        // Test Opus configuration with all parameters
        let config = OpusConfig::with_params(16000, 1, Some(64000), Some(true));
        assert_eq!(config.sample_rate, 16000);
        assert_eq!(config.channels, 1);
        assert_eq!(config.max_bitrate, Some(64000));
        assert_eq!(config.fec_enabled, Some(true));
    }
    
    #[test]
    fn test_opus_config_string_conversion() {
        // Test basic configuration
        let config = OpusConfig::new(48000, 1);
        assert_eq!(config.to_codec_string(), "opus@48000Hz");
        
        // Test stereo configuration
        let config = OpusConfig::new(48000, 2);
        assert_eq!(config.to_codec_string(), "opus@48000Hz@2ch");
        
        // Test with bitrate
        let config = OpusConfig::with_params(16000, 1, Some(32000), None);
        assert_eq!(config.to_codec_string(), "opus@16000Hz@32000bps");
        
        // Test with FEC
        let config = OpusConfig::with_params(8000, 1, None, Some(true));
        assert_eq!(config.to_codec_string(), "opus@8000Hz@fec");
        
        // Test full configuration
        let config = OpusConfig::with_params(48000, 2, Some(128000), Some(true));
        assert_eq!(config.to_codec_string(), "opus@48000Hz@2ch@128000bps@fec");
    }
    
    #[test]
    fn test_opus_config_parsing() {
        // Test parsing basic configuration
        let config = OpusConfig::from_codec_string("opus@48000Hz").unwrap();
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 1);
        
        // Test parsing stereo
        let config = OpusConfig::from_codec_string("opus@48000Hz@2ch").unwrap();
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 2);
        
        // Test parsing with bitrate
        let config = OpusConfig::from_codec_string("opus@16000Hz@32000bps").unwrap();
        assert_eq!(config.sample_rate, 16000);
        assert_eq!(config.max_bitrate, Some(32000));
        
        // Test parsing with FEC
        let config = OpusConfig::from_codec_string("opus@8000Hz@fec").unwrap();
        assert_eq!(config.sample_rate, 8000);
        assert_eq!(config.fec_enabled, Some(true));
        
        // Test parsing full configuration
        let config = OpusConfig::from_codec_string("opus@48000Hz@2ch@128000bps@fec").unwrap();
        assert_eq!(config.sample_rate, 48000);
        assert_eq!(config.channels, 2);
        assert_eq!(config.max_bitrate, Some(128000));
        assert_eq!(config.fec_enabled, Some(true));
        
        // Test invalid string
        assert!(OpusConfig::from_codec_string("invalid").is_none());
    }
    
    #[test]
    fn test_opus_config_registration() {
        let mut mapper = CodecMapper::new();
        
        // Opus should NOT be pre-registered
        assert_eq!(mapper.codec_to_payload("opus"), None);
        assert_eq!(mapper.codec_to_payload("Opus"), None);
        
        // Register different Opus configurations
        let config1 = OpusConfig::new(48000, 2);
        let config2 = OpusConfig::new(16000, 1);
        let config3 = OpusConfig::with_params(8000, 1, Some(32000), Some(true));
        
        mapper.register_opus_config(config1.clone(), 96);
        mapper.register_opus_config(config2.clone(), 97);
        mapper.register_opus_config(config3.clone(), 98);
        
        // Test retrieval by payload type
        assert_eq!(mapper.get_opus_config(96), Some(&config1));
        assert_eq!(mapper.get_opus_config(97), Some(&config2));
        assert_eq!(mapper.get_opus_config(98), Some(&config3));
        
        // Test retrieval by configuration
        assert_eq!(mapper.get_opus_payload_type(&config1), Some(96));
        assert_eq!(mapper.get_opus_payload_type(&config2), Some(97));
        assert_eq!(mapper.get_opus_payload_type(&config3), Some(98));
        
        // Test codec string lookup
        assert_eq!(mapper.codec_to_payload("opus@48000Hz@2ch"), Some(96));
        assert_eq!(mapper.codec_to_payload("opus@16000Hz"), Some(97));
        assert_eq!(mapper.codec_to_payload("opus@8000Hz@32000bps@fec"), Some(98));
        
        // Test generic "opus" lookup (should find first registered)
        assert!(mapper.codec_to_payload("opus").is_some());
        assert!(mapper.codec_to_payload("Opus").is_some());
    }
    
    #[test]
    fn test_opus_config_replacement() {
        let mut mapper = CodecMapper::new();
        
        let config1 = OpusConfig::new(48000, 2);
        let config2 = OpusConfig::new(16000, 1);
        
        // Register first configuration
        mapper.register_opus_config(config1.clone(), 96);
        assert_eq!(mapper.get_opus_config(96), Some(&config1));
        
        // Register different configuration with same payload type
        mapper.register_opus_config(config2.clone(), 96);
        assert_eq!(mapper.get_opus_config(96), Some(&config2));
        
        // Original configuration should no longer be mapped
        assert_eq!(mapper.get_opus_payload_type(&config1), None);
        assert_eq!(mapper.get_opus_payload_type(&config2), Some(96));
    }
    
    #[test]
    fn test_opus_sdp_registration() {
        let mut mapper = CodecMapper::new();
        
        let opus_entries = vec![
            (OpusConfig::new(48000, 2), 96),
            (OpusConfig::new(16000, 1), 97),
            (OpusConfig::with_params(8000, 1, Some(32000), Some(true)), 98),
        ];
        
        mapper.register_opus_from_sdp(&opus_entries);
        
        // Verify all configurations were registered
        assert_eq!(mapper.get_opus_config(96), Some(&opus_entries[0].0));
        assert_eq!(mapper.get_opus_config(97), Some(&opus_entries[1].0));
        assert_eq!(mapper.get_opus_config(98), Some(&opus_entries[2].0));
        
        // Verify reverse mappings
        assert_eq!(mapper.get_opus_payload_type(&opus_entries[0].0), Some(96));
        assert_eq!(mapper.get_opus_payload_type(&opus_entries[1].0), Some(97));
        assert_eq!(mapper.get_opus_payload_type(&opus_entries[2].0), Some(98));
    }
    
    #[test]
    fn test_case_insensitive_lookup() {
        let mapper = CodecMapper::new();
        
        // Test case variations
        assert_eq!(mapper.codec_to_payload("pcmu"), Some(static_types::PCMU));
        assert_eq!(mapper.codec_to_payload("PCMU"), Some(static_types::PCMU));
        assert_eq!(mapper.codec_to_payload("PcMu"), Some(static_types::PCMU));
        
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
        assert_eq!(mapper.get_clock_rate("opus"), 48000); // Fallback for opus
    }
    
    #[test]
    fn test_dynamic_codec_registration() {
        let mut mapper = CodecMapper::new();
        
        // Register custom codec in dynamic range
        mapper.register_dynamic_codec("custom".to_string(), dynamic_range::DYNAMIC_START, 16000);
        
        assert_eq!(mapper.codec_to_payload("custom"), Some(dynamic_range::DYNAMIC_START));
        assert_eq!(mapper.payload_to_codec(dynamic_range::DYNAMIC_START), Some("custom".to_string()));
        assert_eq!(mapper.get_clock_rate("custom"), 16000);
        
        // Test overriding with another dynamic payload type
        let custom_pt_97 = 97; // Another dynamic payload type
        mapper.register_dynamic_codec("custom2".to_string(), custom_pt_97, 32000);
        
        assert_eq!(mapper.codec_to_payload("custom"), None); // Old registration removed
        assert_eq!(mapper.codec_to_payload("custom2"), Some(custom_pt_97));
        assert_eq!(mapper.get_clock_rate("custom2"), 32000);
    }
    
    #[test]
    fn test_codec_capability() {
        let mut mapper = CodecMapper::new();
        
        let pcmu_cap = mapper.get_codec_capability("PCMU").unwrap();
        assert_eq!(pcmu_cap.name, "PCMU");
        assert_eq!(pcmu_cap.payload_type, static_types::PCMU);
        assert_eq!(pcmu_cap.clock_rate, 8000);
        assert!(pcmu_cap.is_static);
        assert!(pcmu_cap.opus_config.is_none());
        
        // Register Opus configuration
        let opus_config = OpusConfig::new(48000, 2);
        mapper.register_opus_config(opus_config.clone(), 96);
        
        let opus_cap = mapper.get_codec_capability("opus@48000Hz@2ch").unwrap();
        assert_eq!(opus_cap.name, "opus@48000Hz@2ch");
        assert_eq!(opus_cap.payload_type, 96);
        assert_eq!(opus_cap.clock_rate, 48000);
        assert!(!opus_cap.is_static);
        assert_eq!(opus_cap.opus_config, Some(opus_config));
    }
    
    #[test]
    fn test_codec_info_string() {
        let mut mapper = CodecMapper::new();
        
        assert_eq!(mapper.get_codec_info("PCMU"), Some(format!("PCMU (PT:{}, 8000Hz)", static_types::PCMU)));
        
        // Register Opus configuration and test info string
        let opus_config = OpusConfig::new(48000, 2);
        mapper.register_opus_config(opus_config, 96);
        
        let info = mapper.get_codec_info("opus@48000Hz@2ch").unwrap();
        assert!(info.contains("opus@48000Hz@2ch"));
        assert!(info.contains("PT:96"));
        assert!(info.contains("48000Hz"));
        assert!(info.contains("2ch"));
        assert!(info.contains("encoding@48000Hz"));
        
        assert_eq!(mapper.get_codec_info("unknown"), None);
    }
    
    #[test]
    fn test_registration_queries() {
        let mut mapper = CodecMapper::new();
        
        // Test static codecs
        assert!(mapper.is_codec_registered("PCMU"));
        assert!(!mapper.is_codec_registered("opus")); // Not pre-registered
        assert!(!mapper.is_codec_registered("unknown"));
        
        assert!(mapper.is_payload_type_registered(static_types::PCMU));
        assert!(!mapper.is_payload_type_registered(96)); // Not pre-registered
        assert!(!mapper.is_payload_type_registered(99));
        
        // Test after Opus registration
        let opus_config = OpusConfig::new(48000, 2);
        mapper.register_opus_config(opus_config, 96);
        
        assert!(mapper.is_codec_registered("opus")); // Now registered (generic lookup)
        assert!(mapper.is_codec_registered("opus@48000Hz@2ch")); // Specific lookup
        assert!(mapper.is_payload_type_registered(96)); // Now registered
    }
    
    #[test]
    fn test_opus_config_retrieval() {
        let mut mapper = CodecMapper::new();
        
        // Register multiple Opus configurations
        let config1 = OpusConfig::new(48000, 2);
        let config2 = OpusConfig::new(16000, 1);
        let config3 = OpusConfig::with_params(8000, 1, Some(32000), Some(true));
        
        mapper.register_opus_config(config1.clone(), 96);
        mapper.register_opus_config(config2.clone(), 97);
        mapper.register_opus_config(config3.clone(), 98);
        
        // Test get_registered_opus_configs
        let opus_configs = mapper.get_registered_opus_configs();
        assert_eq!(opus_configs.len(), 3);
        assert!(opus_configs.contains(&(config1, 96)));
        assert!(opus_configs.contains(&(config2, 97)));
        assert!(opus_configs.contains(&(config3, 98)));
    }
    
    #[test]
    fn test_clear_dynamic_codecs() {
        let mut mapper = CodecMapper::new();
        
        // Add some dynamic codecs
        mapper.register_dynamic_codec("test1".to_string(), 96, 8000);
        mapper.register_dynamic_codec("test2".to_string(), 97, 16000);
        
        // Add Opus configurations
        let config1 = OpusConfig::new(48000, 2);
        let config2 = OpusConfig::new(16000, 1);
        mapper.register_opus_config(config1, 98);
        mapper.register_opus_config(config2, 99);
        
        assert!(mapper.is_codec_registered("test1"));
        assert!(mapper.is_codec_registered("test2"));
        assert!(mapper.is_codec_registered("opus")); // Generic lookup
        assert!(mapper.is_codec_registered("PCMU")); // Static should remain
        
        // Clear dynamic codecs
        mapper.clear_dynamic_codecs();
        
        assert!(!mapper.is_codec_registered("test1"));
        assert!(!mapper.is_codec_registered("test2"));
        assert!(!mapper.is_codec_registered("opus")); // Dynamic codec cleared
        assert!(mapper.is_codec_registered("PCMU")); // Static should remain
        
        // Verify Opus configs are cleared
        assert!(mapper.get_registered_opus_configs().is_empty());
    }
    
    #[test]
    fn test_opus_clock_rates() {
        let mut mapper = CodecMapper::new();
        
        // Register Opus configurations with different encoding rates
        let config1 = OpusConfig::new(48000, 2);
        let config2 = OpusConfig::new(16000, 1);
        let config3 = OpusConfig::new(8000, 1);
        
        mapper.register_opus_config(config1, 96);
        mapper.register_opus_config(config2, 97);
        mapper.register_opus_config(config3, 98);
        
        // All should report 48000 Hz as SDP clock rate (per RFC 7587)
        assert_eq!(mapper.get_clock_rate("opus@48000Hz@2ch"), 48000);
        assert_eq!(mapper.get_clock_rate("opus@16000Hz"), 48000);
        assert_eq!(mapper.get_clock_rate("opus@8000Hz"), 48000);
        
        // Generic opus lookup should also work
        assert_eq!(mapper.get_clock_rate("opus"), 48000);
    }
} 