//! Codec negotiation functionality (moved from rtp-core)
//!
//! This module handles codec negotiation between media endpoints,
//! including capability matching and format selection.

use std::collections::HashMap;
use tracing::{debug, warn};

use crate::api::{error::MediaError, types::{MediaCodec, MediaDirection}};
use super::registry::{PayloadTypeInfo, get_global_registry};

/// Codec negotiation result
#[derive(Debug, Clone, PartialEq)]
pub struct NegotiationResult {
    /// Selected codec
    pub codec: MediaCodec,
    /// Negotiated parameters
    pub parameters: HashMap<String, String>,
    /// Agreed format parameters
    pub format_params: HashMap<String, String>,
}

/// Codec capability for negotiation
#[derive(Debug, Clone, PartialEq)]
pub struct CodecCapability {
    /// Codec information
    pub codec: MediaCodec,
    /// Direction capability
    pub direction: MediaDirection,
    /// Maximum bitrate (bps)
    pub max_bitrate: Option<u32>,
    /// Minimum bitrate (bps)
    pub min_bitrate: Option<u32>,
    /// Supported format parameters
    pub format_params: HashMap<String, Vec<String>>,
    /// Quality factor (0.0-1.0, higher is better)
    pub quality_factor: f32,
}

impl CodecCapability {
    /// Create a new codec capability
    pub fn new(codec: MediaCodec, direction: MediaDirection) -> Self {
        Self {
            codec,
            direction,
            max_bitrate: None,
            min_bitrate: None,
            format_params: HashMap::new(),
            quality_factor: 0.5, // Default medium quality
        }
    }
    
    /// Set bitrate range
    pub fn with_bitrate_range(mut self, min: u32, max: u32) -> Self {
        self.min_bitrate = Some(min);
        self.max_bitrate = Some(max);
        self
    }
    
    /// Set quality factor
    pub fn with_quality_factor(mut self, factor: f32) -> Self {
        self.quality_factor = factor.clamp(0.0, 1.0);
        self
    }
    
    /// Add format parameter
    pub fn with_format_param(mut self, key: String, values: Vec<String>) -> Self {
        self.format_params.insert(key, values);
        self
    }
    
    /// Check if this capability is compatible with another
    pub fn is_compatible_with(&self, other: &CodecCapability) -> bool {
        // Must be same codec
        if self.codec.name != other.codec.name {
            return false;
        }
        
        // Check direction compatibility
        match (self.direction, other.direction) {
            (MediaDirection::SendOnly, MediaDirection::ReceiveOnly) => true,
            (MediaDirection::ReceiveOnly, MediaDirection::SendOnly) => true,
            (MediaDirection::SendReceive, _) => true,
            (_, MediaDirection::SendReceive) => true,
            (MediaDirection::Inactive, _) => false,
            (_, MediaDirection::Inactive) => false,
            _ => false,
        }
    }
}

/// Codec negotiator for media session setup
pub struct CodecNegotiator {
    /// Local codec capabilities
    local_capabilities: Vec<CodecCapability>,
    /// Remote codec capabilities  
    remote_capabilities: Vec<CodecCapability>,
    /// Negotiation preferences
    preferences: NegotiationPreferences,
}

/// Negotiation preferences
#[derive(Debug, Clone)]
pub struct NegotiationPreferences {
    /// Prefer audio codecs over video
    pub prefer_audio: bool,
    /// Maximum number of codecs to negotiate
    pub max_codecs: usize,
    /// Prefer lower bandwidth codecs
    pub prefer_low_bandwidth: bool,
    /// Minimum acceptable quality factor
    pub min_quality_factor: f32,
    /// Preferred codec names (in order of preference)
    pub preferred_codecs: Vec<String>,
}

impl Default for NegotiationPreferences {
    fn default() -> Self {
        Self {
            prefer_audio: true,
            max_codecs: 5,
            prefer_low_bandwidth: false,
            min_quality_factor: 0.3,
            preferred_codecs: vec![
                "OPUS".to_string(),
                "G722".to_string(),
                "PCMU".to_string(),
                "PCMA".to_string(),
                "H264".to_string(),
                "VP8".to_string(),
            ],
        }
    }
}

impl CodecNegotiator {
    /// Create a new codec negotiator
    pub fn new() -> Self {
        Self {
            local_capabilities: Vec::new(),
            remote_capabilities: Vec::new(),
            preferences: NegotiationPreferences::default(),
        }
    }
    
    /// Set negotiation preferences
    pub fn with_preferences(mut self, preferences: NegotiationPreferences) -> Self {
        self.preferences = preferences;
        self
    }
    
    /// Add local codec capability
    pub fn add_local_capability(&mut self, capability: CodecCapability) {
        self.local_capabilities.push(capability);
    }
    
    /// Add remote codec capability
    pub fn add_remote_capability(&mut self, capability: CodecCapability) {
        self.remote_capabilities.push(capability);
    }
    
    /// Set local capabilities from a list
    pub fn set_local_capabilities(&mut self, capabilities: Vec<CodecCapability>) {
        self.local_capabilities = capabilities;
    }
    
    /// Set remote capabilities from a list
    pub fn set_remote_capabilities(&mut self, capabilities: Vec<CodecCapability>) {
        self.remote_capabilities = capabilities;
    }
    
    /// Negotiate codecs between local and remote capabilities
    pub fn negotiate(&self) -> Result<Vec<NegotiationResult>, MediaError> {
        let mut results = Vec::new();
        let mut used_payload_types = std::collections::HashSet::new();
        
        // Find all compatible codec pairs
        let mut compatible_pairs = Vec::new();
        
        for local_cap in &self.local_capabilities {
            for remote_cap in &self.remote_capabilities {
                if local_cap.is_compatible_with(remote_cap) {
                    compatible_pairs.push((local_cap, remote_cap));
                }
            }
        }
        
        if compatible_pairs.is_empty() {
            return Err(MediaError::ConfigError("No compatible codecs found".to_string()));
        }
        
        // Sort pairs by preference
        compatible_pairs.sort_by(|a, b| {
            self.calculate_preference_score(a.0, a.1)
                .partial_cmp(&self.calculate_preference_score(b.0, b.1))
                .unwrap_or(std::cmp::Ordering::Equal)
                .reverse() // Higher scores first
        });
        
        // Select the best matches
        for (local_cap, remote_cap) in compatible_pairs.into_iter().take(self.preferences.max_codecs) {
            // Ensure payload type uniqueness
            let payload_type = local_cap.codec.payload_type;
            if used_payload_types.contains(&payload_type) {
                continue;
            }
            
            let result = self.negotiate_codec_pair(local_cap, remote_cap)?;
            used_payload_types.insert(payload_type);
            results.push(result);
        }
        
        if results.is_empty() {
            return Err(MediaError::ConfigError("No successful codec negotiations".to_string()));
        }
        
        debug!("Successfully negotiated {} codecs", results.len());
        Ok(results)
    }
    
    /// Calculate preference score for a codec pair
    fn calculate_preference_score(&self, local: &CodecCapability, remote: &CodecCapability) -> f32 {
        let mut score = 0.0;
        
        // Base quality score
        score += (local.quality_factor + remote.quality_factor) / 2.0;
        
        // Preference for specific codecs
        if let Some(pos) = self.preferences.preferred_codecs.iter().position(|name| name == &local.codec.name) {
            score += 1.0 - (pos as f32 / self.preferences.preferred_codecs.len() as f32);
        }
        
        // Audio preference
        if self.preferences.prefer_audio && local.codec.is_audio() {
            score += 0.5;
        }
        
        // Bandwidth preference
        if self.preferences.prefer_low_bandwidth {
            if let (Some(local_max), Some(remote_max)) = (local.max_bitrate, remote.max_bitrate) {
                let max_bitrate = local_max.min(remote_max);
                // Lower bitrate gets higher score (inverted)
                score += 1.0 - (max_bitrate as f32 / 1_000_000.0).min(1.0);
            }
        }
        
        // Quality threshold
        let min_quality = (local.quality_factor.min(remote.quality_factor));
        if min_quality < self.preferences.min_quality_factor {
            score *= min_quality / self.preferences.min_quality_factor;
        }
        
        score
    }
    
    /// Negotiate a specific codec pair
    fn negotiate_codec_pair(&self, local: &CodecCapability, remote: &CodecCapability) -> Result<NegotiationResult, MediaError> {
        // Create negotiated codec
        let mut codec = local.codec.clone();
        
        // Negotiate clock rate (use minimum of both)
        if local.codec.clock_rate != remote.codec.clock_rate {
            codec.clock_rate = local.codec.clock_rate.min(remote.codec.clock_rate);
            debug!("Negotiated clock rate: {} Hz", codec.clock_rate);
        }
        
        // Negotiate channels (use minimum for audio)
        if let (Some(local_ch), Some(remote_ch)) = (local.codec.channels, remote.codec.channels) {
            codec.channels = Some(local_ch.min(remote_ch));
        }
        
        // Negotiate parameters
        let mut parameters = HashMap::new();
        
        // Merge parameters from both sides
        for (key, value) in &local.codec.parameters {
            parameters.insert(key.clone(), value.clone());
        }
        
        for (key, value) in &remote.codec.parameters {
            // Remote parameters can override local ones
            parameters.insert(key.clone(), value.clone());
        }
        
        // Negotiate format parameters
        let mut format_params = HashMap::new();
        
        for (key, local_values) in &local.format_params {
            if let Some(remote_values) = remote.format_params.get(key) {
                // Find intersection of supported values
                let common_values: Vec<String> = local_values
                    .iter()
                    .filter(|v| remote_values.contains(v))
                    .cloned()
                    .collect();
                
                if !common_values.is_empty() {
                    // Use the first common value
                    format_params.insert(key.clone(), common_values[0].clone());
                }
            }
        }
        
        Ok(NegotiationResult {
            codec,
            parameters,
            format_params,
        })
    }
    
    /// Create default capabilities for common codecs
    pub fn create_default_audio_capabilities() -> Vec<CodecCapability> {
        let registry = get_global_registry();
        let mut capabilities = Vec::new();
        
        // Add OPUS capability (high quality)
        if let Some(opus_info) = registry.get_payload_info(111) {
            let codec = MediaCodec::new(opus_info.codec_name.clone(), 111, opus_info.clock_rate)
                .with_channels(2);
            capabilities.push(
                CodecCapability::new(codec, MediaDirection::SendReceive)
                    .with_quality_factor(0.9)
                    .with_bitrate_range(32000, 128000)
            );
        }
        
        // Add G.722 capability (good quality)
        if let Some(g722_info) = registry.get_payload_info(9) {
            let codec = MediaCodec::new(g722_info.codec_name.clone(), 9, g722_info.clock_rate)
                .with_channels(1);
            capabilities.push(
                CodecCapability::new(codec, MediaDirection::SendReceive)
                    .with_quality_factor(0.7)
                    .with_bitrate_range(48000, 64000)
            );
        }
        
        // Add PCMU capability (baseline)
        if let Some(pcmu_info) = registry.get_payload_info(0) {
            let codec = MediaCodec::new(pcmu_info.codec_name.clone(), 0, pcmu_info.clock_rate)
                .with_channels(1);
            capabilities.push(
                CodecCapability::new(codec, MediaDirection::SendReceive)
                    .with_quality_factor(0.5)
                    .with_bitrate_range(64000, 64000)
            );
        }
        
        // Add PCMA capability (baseline)
        if let Some(pcma_info) = registry.get_payload_info(8) {
            let codec = MediaCodec::new(pcma_info.codec_name.clone(), 8, pcma_info.clock_rate)
                .with_channels(1);
            capabilities.push(
                CodecCapability::new(codec, MediaDirection::SendReceive)
                    .with_quality_factor(0.5)
                    .with_bitrate_range(64000, 64000)
            );
        }
        
        capabilities
    }
    
    /// Create default capabilities for common video codecs
    pub fn create_default_video_capabilities() -> Vec<CodecCapability> {
        let registry = get_global_registry();
        let mut capabilities = Vec::new();
        
        // Add H.264 capability (high quality)
        if let Some(h264_info) = registry.get_payload_info(96) {
            let codec = MediaCodec::new(h264_info.codec_name.clone(), 96, h264_info.clock_rate)
                .with_parameter("profile-level-id".to_string(), "42e01e".to_string());
            capabilities.push(
                CodecCapability::new(codec, MediaDirection::SendReceive)
                    .with_quality_factor(0.9)
                    .with_bitrate_range(100000, 2000000)
            );
        }
        
        // Add VP8 capability (good quality)
        if let Some(vp8_info) = registry.get_payload_info(97) {
            let codec = MediaCodec::new(vp8_info.codec_name.clone(), 97, vp8_info.clock_rate);
            capabilities.push(
                CodecCapability::new(codec, MediaDirection::SendReceive)
                    .with_quality_factor(0.8)
                    .with_bitrate_range(100000, 1500000)
            );
        }
        
        capabilities
    }
}

impl Default for CodecNegotiator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_codec_capability_compatibility() {
        let send_cap = CodecCapability::new(
            MediaCodec::new("OPUS".to_string(), 111, 48000),
            MediaDirection::SendOnly
        );
        
        let recv_cap = CodecCapability::new(
            MediaCodec::new("OPUS".to_string(), 111, 48000),
            MediaDirection::ReceiveOnly
        );
        
        assert!(send_cap.is_compatible_with(&recv_cap));
        assert!(recv_cap.is_compatible_with(&send_cap));
    }
    
    #[test]
    fn test_codec_capability_incompatibility() {
        let opus_cap = CodecCapability::new(
            MediaCodec::new("OPUS".to_string(), 111, 48000),
            MediaDirection::SendOnly
        );
        
        let pcmu_cap = CodecCapability::new(
            MediaCodec::new("PCMU".to_string(), 0, 8000),
            MediaDirection::ReceiveOnly
        );
        
        assert!(!opus_cap.is_compatible_with(&pcmu_cap));
    }
    
    #[test]
    fn test_negotiation() {
        let mut negotiator = CodecNegotiator::new();
        
        // Add local capabilities
        negotiator.add_local_capability(
            CodecCapability::new(
                MediaCodec::new("OPUS".to_string(), 111, 48000),
                MediaDirection::SendReceive
            ).with_quality_factor(0.9)
        );
        
        negotiator.add_local_capability(
            CodecCapability::new(
                MediaCodec::new("PCMU".to_string(), 0, 8000),
                MediaDirection::SendReceive
            ).with_quality_factor(0.5)
        );
        
        // Add remote capabilities (in different order)
        negotiator.add_remote_capability(
            CodecCapability::new(
                MediaCodec::new("PCMU".to_string(), 0, 8000),
                MediaDirection::SendReceive
            ).with_quality_factor(0.5)
        );
        
        negotiator.add_remote_capability(
            CodecCapability::new(
                MediaCodec::new("OPUS".to_string(), 111, 48000),
                MediaDirection::SendReceive
            ).with_quality_factor(0.9)
        );
        
        let results = negotiator.negotiate().unwrap();
        assert!(!results.is_empty());
        
        // OPUS should be preferred due to higher quality
        assert_eq!(results[0].codec.name, "OPUS");
    }
    
    #[test]
    fn test_default_capabilities() {
        let audio_caps = CodecNegotiator::create_default_audio_capabilities();
        assert!(!audio_caps.is_empty());
        
        // Should include common audio codecs
        let codec_names: Vec<&str> = audio_caps.iter().map(|c| c.codec.name.as_str()).collect();
        assert!(codec_names.contains(&"PCMU"));
        assert!(codec_names.contains(&"PCMA"));
    }
}