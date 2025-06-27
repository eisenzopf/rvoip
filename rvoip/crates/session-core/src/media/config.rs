//! Media Configuration Converter
//!
//! Handles conversion between SDP and media-core configuration, adapted from the proven
//! working implementation in src-old/media/config.rs. This converter bridges SIP signaling
//! and media processing configuration.

use super::types::*;
use super::MediaError;
use std::collections::HashMap;

/// Converts between SDP and media-core configuration
/// 
/// This will be adapted from the working MediaConfigConverter in 
/// src-old/media/config.rs to handle SDP parsing and generation.
#[derive(Debug, Clone)]
pub struct MediaConfigConverter {
    /// Supported codecs for offer/answer
    supported_codecs: Vec<CodecInfo>,
    
    /// Default port range for RTP
    port_range: (RtpPort, RtpPort),
    
    /// Codec preference order
    codec_preferences: Vec<String>,
}

impl MediaConfigConverter {
    /// Create a new converter with default configuration
    pub fn new() -> Self {
        Self {
            supported_codecs: vec![
                CodecInfo {
                    name: "PCMU".to_string(),
                    payload_type: 0,
                    sample_rate: 8000,
                    channels: 1,
                },
                CodecInfo {
                    name: "PCMA".to_string(),
                    payload_type: 8,
                    sample_rate: 8000,
                    channels: 1,
                },
                CodecInfo {
                    name: "G722".to_string(),
                    payload_type: 9,
                    sample_rate: 8000,
                    channels: 1,
                },
                CodecInfo {
                    name: "opus".to_string(),
                    payload_type: 96,
                    sample_rate: 48000,
                    channels: 2,
                },
            ],
            port_range: (10000, 20000),
            codec_preferences: vec![
                "opus".to_string(),
                "PCMU".to_string(),
                "PCMA".to_string(),
            ],
        }
    }
    
    /// Create converter with custom configuration
    pub fn with_config(capabilities: &MediaCapabilities) -> Self {
        Self {
            supported_codecs: capabilities.codecs.clone(),
            port_range: capabilities.port_range,
            codec_preferences: capabilities.codecs.iter()
                .map(|c| c.name.clone())
                .collect(),
        }
    }
    
    /// Create converter with codec preferences from MediaConfig
    pub fn with_media_config(media_config: &super::types::MediaConfig) -> Self {
        let mut converter = Self::new();
        if !media_config.preferred_codecs.is_empty() {
            converter.codec_preferences = media_config.preferred_codecs.clone();
        }
        // Apply port range if specified
        if let Some(port_range) = media_config.port_range {
            converter.port_range = port_range;
        }
        converter
    }
    
    /// Generate SDP offer from media capabilities
    /// 
    /// This will be expanded with logic from src-old/media/config.rs
    /// to generate proper SDP offers with codec negotiation.
    pub fn generate_sdp_offer(&self, local_ip: &str, local_port: RtpPort) -> super::MediaResult<String> {
        tracing::debug!("Generating SDP offer for {}:{}", local_ip, local_port);
        
        // Order codecs based on preferences
        let mut ordered_codecs = Vec::new();
        for pref_name in &self.codec_preferences {
            if let Some(codec) = self.supported_codecs.iter().find(|c| &c.name == pref_name) {
                ordered_codecs.push(codec.clone());
            }
        }
        // Add any remaining codecs not in preferences
        for codec in &self.supported_codecs {
            if !ordered_codecs.iter().any(|c| c.name == codec.name) {
                ordered_codecs.push(codec.clone());
            }
        }
        
        let mut sdp = String::new();
        
        // SDP version
        sdp.push_str("v=0\r\n");
        
        // Origin
        sdp.push_str(&format!("o=rvoip {} {} IN IP4 {}\r\n", 
                             chrono::Utc::now().timestamp(), 
                             chrono::Utc::now().timestamp(), 
                             local_ip));
        
        // Session name
        sdp.push_str("s=RVOIP Session\r\n");
        
        // Connection
        sdp.push_str(&format!("c=IN IP4 {}\r\n", local_ip));
        
        // Time
        sdp.push_str("t=0 0\r\n");
        
        // Media description - codecs in preference order
        sdp.push_str(&format!("m=audio {} RTP/AVP", local_port));
        for codec in &ordered_codecs {
            sdp.push_str(&format!(" {}", codec.payload_type));
        }
        sdp.push_str("\r\n");
        
        // RTP map attributes - in preference order
        for codec in &ordered_codecs {
            if codec.channels == 1 {
                sdp.push_str(&format!("a=rtpmap:{} {}/{}\r\n", 
                                    codec.payload_type, codec.name, codec.sample_rate));
            } else {
                sdp.push_str(&format!("a=rtpmap:{} {}/{}/{}\r\n", 
                                    codec.payload_type, codec.name, codec.sample_rate, codec.channels));
            }
        }
        
        // Additional attributes
        sdp.push_str("a=sendrecv\r\n");
        
        Ok(sdp)
    }
    
    /// Parse SDP answer and determine negotiated parameters
    /// 
    /// This will be expanded with logic from src-old/media/config.rs
    /// to parse SDP answers and determine the negotiated codec and parameters.
    pub fn parse_sdp_answer(&self, sdp: &str) -> super::MediaResult<NegotiatedConfig> {
        tracing::debug!("Parsing SDP answer");
        
        // TODO: Adapt from src-old/media/config.rs SDP parsing logic
        let mut remote_ip = None;
        let mut remote_port = None;
        let mut negotiated_codec = None;
        
        for line in sdp.lines() {
            if line.starts_with("c=IN IP4 ") {
                remote_ip = Some(line[9..].trim().to_string());
            } else if line.starts_with("m=audio ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(port) = parts[1].parse::<RtpPort>() {
                        remote_port = Some(port);
                    }
                    
                    // Extract payload types
                    if parts.len() > 3 {
                        for pt_str in &parts[3..] {
                            if let Ok(pt) = pt_str.parse::<u8>() {
                                // Find matching codec
                                if let Some(codec) = self.supported_codecs.iter()
                                    .find(|c| c.payload_type == pt) {
                                    negotiated_codec = Some(codec.clone());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        
        Ok(NegotiatedConfig {
            remote_ip: remote_ip.ok_or_else(|| MediaError::SdpProcessing { 
                message: "No remote IP found in SDP".to_string() 
            })?,
            remote_port: remote_port.ok_or_else(|| MediaError::SdpProcessing { 
                message: "No remote port found in SDP".to_string() 
            })?,
            codec: negotiated_codec.ok_or_else(|| MediaError::CodecNegotiation { 
                reason: "No compatible codec found".to_string() 
            })?,
        })
    }
    
    /// Parse SDP offer and generate appropriate answer
    /// 
    /// This will be expanded with logic from src-old/media/config.rs
    /// to parse incoming SDP offers and generate compatible answers.
    pub fn generate_sdp_answer(&self, offer_sdp: &str, local_ip: &str, local_port: RtpPort) -> super::MediaResult<String> {
        tracing::debug!("Generating SDP answer for offer");
        
        // TODO: Adapt from src-old/media/config.rs answer generation logic
        
        // Parse the offer to find compatible codecs
        let mut offered_codecs = Vec::new();
        let mut media_port = None;
        
        for line in offer_sdp.lines() {
            if line.starts_with("m=audio ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(port) = parts[1].parse::<RtpPort>() {
                        media_port = Some(port);
                    }
                    
                    // Extract offered payload types
                    if parts.len() > 3 {
                        for pt_str in &parts[3..] {
                            if let Ok(pt) = pt_str.parse::<u8>() {
                                offered_codecs.push(pt);
                            }
                        }
                    }
                }
            }
        }
        
        // Find compatible codecs in preference order
        let mut compatible_codecs = Vec::new();
        for codec_name in &self.codec_preferences {
            if let Some(codec) = self.supported_codecs.iter()
                .find(|c| &c.name == codec_name && offered_codecs.contains(&c.payload_type)) {
                compatible_codecs.push(codec.clone());
            }
        }
        
        if compatible_codecs.is_empty() {
            return Err(MediaError::CodecNegotiation { 
                reason: "No compatible codecs found in offer".to_string() 
            });
        }
        
        // Generate answer SDP
        let mut sdp = String::new();
        
        sdp.push_str("v=0\r\n");
        sdp.push_str(&format!("o=rvoip {} {} IN IP4 {}\r\n", 
                             chrono::Utc::now().timestamp(), 
                             chrono::Utc::now().timestamp(), 
                             local_ip));
        sdp.push_str("s=RVOIP Session\r\n");
        sdp.push_str(&format!("c=IN IP4 {}\r\n", local_ip));
        sdp.push_str("t=0 0\r\n");
        
        // Use first compatible codec
        let selected_codec = &compatible_codecs[0];
        sdp.push_str(&format!("m=audio {} RTP/AVP {}\r\n", local_port, selected_codec.payload_type));
        
        if selected_codec.channels == 1 {
            sdp.push_str(&format!("a=rtpmap:{} {}/{}\r\n", 
                                selected_codec.payload_type, selected_codec.name, selected_codec.sample_rate));
        } else {
            sdp.push_str(&format!("a=rtpmap:{} {}/{}/{}\r\n", 
                                selected_codec.payload_type, selected_codec.name, 
                                selected_codec.sample_rate, selected_codec.channels));
        }
        
        sdp.push_str("a=sendrecv\r\n");
        
        Ok(sdp)
    }
    
    /// Convert media configuration to MediaEngine config
    pub fn to_media_config(&self, negotiated: &NegotiatedConfig) -> MediaConfig {
        MediaConfig {
            preferred_codecs: vec![negotiated.codec.name.clone()],
            port_range: Some(self.port_range),
            quality_monitoring: true,
            dtmf_support: true,
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            max_bandwidth_kbps: None,
            preferred_ptime: Some(20),
            custom_sdp_attributes: std::collections::HashMap::new(),
        }
    }
    
    /// Get supported codecs
    pub fn get_supported_codecs(&self) -> &[CodecInfo] {
        &self.supported_codecs
    }
    
    /// Get port range
    pub fn get_port_range(&self) -> (RtpPort, RtpPort) {
        self.port_range
    }
    
    /// Set codec preferences
    pub fn set_codec_preferences(&mut self, preferences: Vec<String>) {
        self.codec_preferences = preferences;
    }
}

/// Configuration determined after SDP negotiation
#[derive(Debug, Clone)]
pub struct NegotiatedConfig {
    /// Remote IP address
    pub remote_ip: String,
    
    /// Remote RTP port
    pub remote_port: RtpPort,
    
    /// Negotiated codec
    pub codec: CodecInfo,
}

impl Default for MediaConfigConverter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sdp_offer_generation() {
        let converter = MediaConfigConverter::new();
        let sdp = converter.generate_sdp_offer("127.0.0.1", 10000).unwrap();
        
        assert!(sdp.contains("v=0"));
        assert!(sdp.contains("m=audio 10000"));
        assert!(sdp.contains("a=rtpmap:0 PCMU/8000"));
        assert!(sdp.contains("a=rtpmap:8 PCMA/8000"));
    }
    
    #[test]
    fn test_sdp_answer_parsing() {
        let converter = MediaConfigConverter::new();
        let answer_sdp = 
            "v=0\r\n\
             o=remote 123 456 IN IP4 192.168.1.100\r\n\
             s=Session\r\n\
             c=IN IP4 192.168.1.100\r\n\
             t=0 0\r\n\
             m=audio 5000 RTP/AVP 0\r\n\
             a=rtpmap:0 PCMU/8000\r\n";
        
        let config = converter.parse_sdp_answer(answer_sdp).unwrap();
        
        assert_eq!(config.remote_ip, "192.168.1.100");
        assert_eq!(config.remote_port, 5000);
        assert_eq!(config.codec.name, "PCMU");
        assert_eq!(config.codec.payload_type, 0);
    }
    
    #[test]
    fn test_sdp_answer_generation() {
        let converter = MediaConfigConverter::new();
        let offer_sdp = 
            "v=0\r\n\
             o=remote 123 456 IN IP4 192.168.1.100\r\n\
             s=Session\r\n\
             c=IN IP4 192.168.1.100\r\n\
             t=0 0\r\n\
             m=audio 5000 RTP/AVP 0 8 96\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=rtpmap:96 opus/48000/2\r\n";
        
        let answer = converter.generate_sdp_answer(offer_sdp, "127.0.0.1", 10000).unwrap();
        
        assert!(answer.contains("m=audio 10000"));
        assert!(answer.contains("c=IN IP4 127.0.0.1"));
        // Should prefer opus based on codec preferences
        assert!(answer.contains("96"));
    }
} 