//! SDP media handling (moved from rtp-core)
//!
//! This module handles SDP (Session Description Protocol) media line
//! processing for codec negotiation and media session setup.

use std::collections::HashMap;
use std::str::FromStr;
use tracing::{debug, warn};

use crate::api::{error::MediaError, types::{MediaCodec, MediaDirection, MediaStreamConfig}};
use super::{
    registry::{PayloadTypeInfo, get_global_registry},
    negotiation::{CodecCapability, CodecNegotiator, NegotiationPreferences}
};

/// SDP media line representation
#[derive(Debug, Clone, PartialEq)]
pub struct SdpMediaLine {
    /// Media type (audio, video, application)
    pub media_type: String,
    /// Port number
    pub port: u16,
    /// Protocol (typically "RTP/AVP" or "RTP/SAVP")
    pub protocol: String,
    /// List of payload types
    pub payload_types: Vec<u8>,
}

/// SDP attribute representation
#[derive(Debug, Clone, PartialEq)]
pub struct SdpAttribute {
    /// Attribute name
    pub name: String,
    /// Attribute value (if any)
    pub value: Option<String>,
}

/// SDP format parameter (a=fmtp)
#[derive(Debug, Clone, PartialEq)]
pub struct SdpFormatParameter {
    /// Payload type
    pub payload_type: u8,
    /// Format parameters
    pub parameters: HashMap<String, String>,
}

/// SDP media description
#[derive(Debug, Clone, PartialEq)]
pub struct SdpMediaDescription {
    /// Media line
    pub media_line: SdpMediaLine,
    /// Media direction
    pub direction: MediaDirection,
    /// RTP map attributes (a=rtpmap)
    pub rtp_maps: HashMap<u8, SdpRtpMap>,
    /// Format parameters (a=fmtp)
    pub format_params: HashMap<u8, SdpFormatParameter>,
    /// Other attributes
    pub attributes: Vec<SdpAttribute>,
}

/// SDP RTP map (a=rtpmap)
#[derive(Debug, Clone, PartialEq)]
pub struct SdpRtpMap {
    /// Payload type
    pub payload_type: u8,
    /// Encoding name
    pub encoding_name: String,
    /// Clock rate
    pub clock_rate: u32,
    /// Number of channels (audio only)
    pub channels: Option<u8>,
}

impl FromStr for SdpRtpMap {
    type Err = MediaError;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Format: "payload_type encoding_name/clock_rate[/channels]"
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 2 {
            return Err(MediaError::FormatError("Invalid rtpmap format".to_string()));
        }
        
        let payload_type = parts[0].parse::<u8>()
            .map_err(|_| MediaError::FormatError("Invalid payload type".to_string()))?;
        
        let encoding_parts: Vec<&str> = parts[1].split('/').collect();
        if encoding_parts.len() < 2 {
            return Err(MediaError::FormatError("Invalid encoding format".to_string()));
        }
        
        let encoding_name = encoding_parts[0].to_string();
        let clock_rate = encoding_parts[1].parse::<u32>()
            .map_err(|_| MediaError::FormatError("Invalid clock rate".to_string()))?;
        
        let channels = if encoding_parts.len() > 2 {
            Some(encoding_parts[2].parse::<u8>()
                .map_err(|_| MediaError::FormatError("Invalid channel count".to_string()))?)
        } else {
            None
        };
        
        Ok(SdpRtpMap {
            payload_type,
            encoding_name,
            clock_rate,
            channels,
        })
    }
}

impl ToString for SdpRtpMap {
    fn to_string(&self) -> String {
        if let Some(channels) = self.channels {
            format!("{} {}/{}/{}", self.payload_type, self.encoding_name, self.clock_rate, channels)
        } else {
            format!("{} {}/{}", self.payload_type, self.encoding_name, self.clock_rate)
        }
    }
}

/// SDP media processor for codec negotiation
pub struct SdpMediaProcessor {
    /// Negotiation preferences
    preferences: NegotiationPreferences,
}

impl SdpMediaProcessor {
    /// Create a new SDP media processor
    pub fn new() -> Self {
        Self {
            preferences: NegotiationPreferences::default(),
        }
    }
    
    /// Set negotiation preferences
    pub fn with_preferences(mut self, preferences: NegotiationPreferences) -> Self {
        self.preferences = preferences;
        self
    }
    
    /// Parse SDP media description from text
    pub fn parse_media_description(&self, sdp_text: &str) -> Result<SdpMediaDescription, MediaError> {
        let lines: Vec<&str> = sdp_text.lines().collect();
        let mut media_line = None;
        let mut direction = MediaDirection::SendReceive; // Default
        let mut rtp_maps = HashMap::new();
        let mut format_params = HashMap::new();
        let mut attributes = Vec::new();
        
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            
            if let Some(media_content) = line.strip_prefix("m=") {
                media_line = Some(self.parse_media_line(media_content)?);
            } else if let Some(attr_content) = line.strip_prefix("a=") {
                self.parse_attribute(attr_content, &mut direction, &mut rtp_maps, &mut format_params, &mut attributes)?;
            }
        }
        
        let media_line = media_line.ok_or_else(|| {
            MediaError::FormatError("No media line found in SDP".to_string())
        })?;
        
        Ok(SdpMediaDescription {
            media_line,
            direction,
            rtp_maps,
            format_params,
            attributes,
        })
    }
    
    /// Parse media line (m=)
    fn parse_media_line(&self, content: &str) -> Result<SdpMediaLine, MediaError> {
        let parts: Vec<&str> = content.split_whitespace().collect();
        if parts.len() < 4 {
            return Err(MediaError::FormatError("Invalid media line format".to_string()));
        }
        
        let media_type = parts[0].to_string();
        let port = parts[1].parse::<u16>()
            .map_err(|_| MediaError::FormatError("Invalid port number".to_string()))?;
        let protocol = parts[2].to_string();
        
        let payload_types: Result<Vec<u8>, _> = parts[3..]
            .iter()
            .map(|pt| pt.parse::<u8>())
            .collect();
        
        let payload_types = payload_types
            .map_err(|_| MediaError::FormatError("Invalid payload type".to_string()))?;
        
        Ok(SdpMediaLine {
            media_type,
            port,
            protocol,
            payload_types,
        })
    }
    
    /// Parse attribute line (a=)
    fn parse_attribute(
        &self,
        content: &str,
        direction: &mut MediaDirection,
        rtp_maps: &mut HashMap<u8, SdpRtpMap>,
        format_params: &mut HashMap<u8, SdpFormatParameter>,
        attributes: &mut Vec<SdpAttribute>,
    ) -> Result<(), MediaError> {
        if let Some(rtpmap_content) = content.strip_prefix("rtpmap:") {
            let rtp_map = SdpRtpMap::from_str(rtpmap_content)?;
            rtp_maps.insert(rtp_map.payload_type, rtp_map);
        } else if let Some(fmtp_content) = content.strip_prefix("fmtp:") {
            let format_param = self.parse_format_parameter(fmtp_content)?;
            format_params.insert(format_param.payload_type, format_param);
        } else if content == "sendonly" {
            *direction = MediaDirection::SendOnly;
        } else if content == "recvonly" {
            *direction = MediaDirection::ReceiveOnly;
        } else if content == "sendrecv" {
            *direction = MediaDirection::SendReceive;
        } else if content == "inactive" {
            *direction = MediaDirection::Inactive;
        } else {
            // Parse other attributes
            if let Some(colon_pos) = content.find(':') {
                let name = content[..colon_pos].to_string();
                let value = Some(content[colon_pos + 1..].to_string());
                attributes.push(SdpAttribute { name, value });
            } else {
                attributes.push(SdpAttribute {
                    name: content.to_string(),
                    value: None,
                });
            }
        }
        
        Ok(())
    }
    
    /// Parse format parameter (a=fmtp)
    fn parse_format_parameter(&self, content: &str) -> Result<SdpFormatParameter, MediaError> {
        let parts: Vec<&str> = content.splitn(2, ' ').collect();
        if parts.len() != 2 {
            return Err(MediaError::FormatError("Invalid fmtp format".to_string()));
        }
        
        let payload_type = parts[0].parse::<u8>()
            .map_err(|_| MediaError::FormatError("Invalid payload type in fmtp".to_string()))?;
        
        let mut parameters = HashMap::new();
        
        // Parse parameters (format: "key1=value1;key2=value2")
        for param in parts[1].split(';') {
            if let Some(eq_pos) = param.find('=') {
                let key = param[..eq_pos].trim().to_string();
                let value = param[eq_pos + 1..].trim().to_string();
                parameters.insert(key, value);
            }
        }
        
        Ok(SdpFormatParameter {
            payload_type,
            parameters,
        })
    }
    
    /// Convert SDP media description to codec capabilities
    pub fn sdp_to_capabilities(&self, sdp: &SdpMediaDescription) -> Result<Vec<CodecCapability>, MediaError> {
        let mut capabilities = Vec::new();
        let registry = get_global_registry();
        
        for &payload_type in &sdp.media_line.payload_types {
            let mut codec = if let Some(rtp_map) = sdp.rtp_maps.get(&payload_type) {
                // Use RTP map information
                MediaCodec::new(
                    rtp_map.encoding_name.clone(),
                    payload_type,
                    rtp_map.clock_rate,
                ).with_channels(rtp_map.channels.unwrap_or(1))
            } else if let Some(info) = registry.get_payload_info(payload_type) {
                // Use registry information
                let mut codec = MediaCodec::new(
                    info.codec_name.clone(),
                    payload_type,
                    info.clock_rate,
                );
                if let Some(channels) = info.channels {
                    codec = codec.with_channels(channels);
                }
                codec
            } else {
                // Unknown payload type
                warn!("Unknown payload type: {}", payload_type);
                continue;
            };
            
            // Add format parameters if present
            if let Some(fmtp) = sdp.format_params.get(&payload_type) {
                for (key, value) in &fmtp.parameters {
                    codec = codec.with_parameter(key.clone(), value.clone());
                }
            }
            
            // Create capability
            let mut capability = CodecCapability::new(codec, sdp.direction);
            
            // Set quality factor based on codec
            capability.quality_factor = self.estimate_codec_quality(&capability.codec.name);
            
            capabilities.push(capability);
        }
        
        Ok(capabilities)
    }
    
    /// Convert codec capabilities to SDP media description
    pub fn capabilities_to_sdp(&self, capabilities: &[CodecCapability], media_type: &str, port: u16) -> Result<SdpMediaDescription, MediaError> {
        let payload_types: Vec<u8> = capabilities.iter().map(|c| c.codec.payload_type).collect();
        
        let media_line = SdpMediaLine {
            media_type: media_type.to_string(),
            port,
            protocol: "RTP/AVP".to_string(), // Default protocol
            payload_types,
        };
        
        // Determine overall direction
        let direction = if capabilities.iter().all(|c| c.direction == MediaDirection::SendOnly) {
            MediaDirection::SendOnly
        } else if capabilities.iter().all(|c| c.direction == MediaDirection::ReceiveOnly) {
            MediaDirection::ReceiveOnly
        } else if capabilities.iter().any(|c| c.direction == MediaDirection::Inactive) {
            MediaDirection::Inactive
        } else {
            MediaDirection::SendReceive
        };
        
        let mut rtp_maps = HashMap::new();
        let mut format_params = HashMap::new();
        
        for capability in capabilities {
            let codec = &capability.codec;
            
            // Create RTP map
            let rtp_map = SdpRtpMap {
                payload_type: codec.payload_type,
                encoding_name: codec.name.clone(),
                clock_rate: codec.clock_rate,
                channels: codec.channels,
            };
            rtp_maps.insert(codec.payload_type, rtp_map);
            
            // Create format parameters if needed
            if !codec.parameters.is_empty() {
                let format_param = SdpFormatParameter {
                    payload_type: codec.payload_type,
                    parameters: codec.parameters.clone(),
                };
                format_params.insert(codec.payload_type, format_param);
            }
        }
        
        Ok(SdpMediaDescription {
            media_line,
            direction,
            rtp_maps,
            format_params,
            attributes: Vec::new(),
        })
    }
    
    /// Negotiate SDP media descriptions
    pub fn negotiate_media_descriptions(
        &self,
        local_sdp: &SdpMediaDescription,
        remote_sdp: &SdpMediaDescription,
    ) -> Result<SdpMediaDescription, MediaError> {
        // Convert to capabilities
        let local_caps = self.sdp_to_capabilities(local_sdp)?;
        let remote_caps = self.sdp_to_capabilities(remote_sdp)?;
        
        // Negotiate
        let mut negotiator = CodecNegotiator::new().with_preferences(self.preferences.clone());
        negotiator.set_local_capabilities(local_caps);
        negotiator.set_remote_capabilities(remote_caps);
        
        let negotiation_results = negotiator.negotiate()?;
        
        // Convert back to capabilities
        let negotiated_caps: Vec<CodecCapability> = negotiation_results
            .into_iter()
            .map(|result| CodecCapability::new(result.codec, local_sdp.direction))
            .collect();
        
        // Create negotiated SDP
        self.capabilities_to_sdp(&negotiated_caps, &local_sdp.media_line.media_type, local_sdp.media_line.port)
    }
    
    /// Generate SDP text from media description
    pub fn generate_sdp_text(&self, sdp: &SdpMediaDescription) -> String {
        let mut lines = Vec::new();
        
        // Media line
        let media_line = format!(
            "m={} {} {} {}",
            sdp.media_line.media_type,
            sdp.media_line.port,
            sdp.media_line.protocol,
            sdp.media_line.payload_types
                .iter()
                .map(|pt| pt.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        );
        lines.push(media_line);
        
        // Direction attribute
        let direction_attr = match sdp.direction {
            MediaDirection::SendOnly => "a=sendonly",
            MediaDirection::ReceiveOnly => "a=recvonly",
            MediaDirection::SendReceive => "a=sendrecv",
            MediaDirection::Inactive => "a=inactive",
        };
        lines.push(direction_attr.to_string());
        
        // RTP maps
        for rtp_map in sdp.rtp_maps.values() {
            lines.push(format!("a=rtpmap:{}", rtp_map.to_string()));
        }
        
        // Format parameters
        for fmtp in sdp.format_params.values() {
            let params: Vec<String> = fmtp.parameters
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect();
            lines.push(format!("a=fmtp:{} {}", fmtp.payload_type, params.join(";")));
        }
        
        // Other attributes
        for attr in &sdp.attributes {
            if let Some(ref value) = attr.value {
                lines.push(format!("a={}:{}", attr.name, value));
            } else {
                lines.push(format!("a={}", attr.name));
            }
        }
        
        lines.join("\r\n")
    }
    
    /// Estimate codec quality factor
    fn estimate_codec_quality(&self, codec_name: &str) -> f32 {
        match codec_name.to_uppercase().as_str() {
            "OPUS" => 0.9,
            "G722" => 0.7,
            "PCMU" | "PCMA" => 0.5,
            "G729" => 0.4,
            "GSM" => 0.3,
            "H264" => 0.9,
            "VP8" => 0.8,
            "VP9" => 0.85,
            "H263" => 0.6,
            _ => 0.5, // Default
        }
    }
}

impl Default for SdpMediaProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_rtpmap() {
        let rtpmap = SdpRtpMap::from_str("111 OPUS/48000/2").unwrap();
        assert_eq!(rtpmap.payload_type, 111);
        assert_eq!(rtpmap.encoding_name, "OPUS");
        assert_eq!(rtpmap.clock_rate, 48000);
        assert_eq!(rtpmap.channels, Some(2));
    }
    
    #[test]
    fn test_rtpmap_to_string() {
        let rtpmap = SdpRtpMap {
            payload_type: 0,
            encoding_name: "PCMU".to_string(),
            clock_rate: 8000,
            channels: Some(1),
        };
        assert_eq!(rtpmap.to_string(), "0 PCMU/8000/1");
    }
    
    #[test]
    fn test_parse_media_line() {
        let processor = SdpMediaProcessor::new();
        let media_line = processor.parse_media_line("audio 5004 RTP/AVP 0 8 111").unwrap();
        
        assert_eq!(media_line.media_type, "audio");
        assert_eq!(media_line.port, 5004);
        assert_eq!(media_line.protocol, "RTP/AVP");
        assert_eq!(media_line.payload_types, vec![0, 8, 111]);
    }
    
    #[test]
    fn test_parse_simple_sdp() {
        let processor = SdpMediaProcessor::new();
        let sdp_text = "m=audio 5004 RTP/AVP 0 8\r\na=sendrecv\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\n";
        
        let sdp = processor.parse_media_description(sdp_text).unwrap();
        
        assert_eq!(sdp.media_line.media_type, "audio");
        assert_eq!(sdp.media_line.payload_types, vec![0, 8]);
        assert_eq!(sdp.direction, MediaDirection::SendReceive);
        assert!(sdp.rtp_maps.contains_key(&0));
        assert!(sdp.rtp_maps.contains_key(&8));
    }
    
    #[test]
    fn test_sdp_to_capabilities() {
        let processor = SdpMediaProcessor::new();
        let sdp_text = "m=audio 5004 RTP/AVP 111\r\na=sendrecv\r\na=rtpmap:111 OPUS/48000/2\r\n";
        
        let sdp = processor.parse_media_description(sdp_text).unwrap();
        let capabilities = processor.sdp_to_capabilities(&sdp).unwrap();
        
        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].codec.name, "OPUS");
        assert_eq!(capabilities[0].codec.clock_rate, 48000);
        assert_eq!(capabilities[0].codec.channels, Some(2));
    }
    
    #[test]
    fn test_generate_sdp_text() {
        let processor = SdpMediaProcessor::new();
        
        let codec = MediaCodec::new("PCMU".to_string(), 0, 8000).with_channels(1);
        let capability = CodecCapability::new(codec, MediaDirection::SendReceive);
        
        let sdp = processor.capabilities_to_sdp(&[capability], "audio", 5004).unwrap();
        let text = processor.generate_sdp_text(&sdp);
        
        assert!(text.contains("m=audio 5004 RTP/AVP 0"));
        assert!(text.contains("a=sendrecv"));
        assert!(text.contains("a=rtpmap:0 PCMU/8000/1"));
    }
}