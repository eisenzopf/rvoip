// Session Core SDP Integration
//
// This module provides integration between the session-core layer and sip-core's SDP implementation.
// It focuses on SDP operations needed specifically for the session layer, building on top of
// the more generic SDP implementation in the sip-core crate.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use tracing::{debug, warn, trace, error};
use thiserror::Error;

// Import types from sip-core's SDP implementation
use rvoip_sip_core::sdp::{
    SdpBuilder, 
    attributes::MediaDirection
};
use rvoip_sip_core::types::sdp::{
    SdpSession,
    MediaDescription,
    ParsedAttribute,
    RtpMapAttribute
};
use rvoip_sip_core::error::Result as SipResult;

use crate::media::{MediaConfig, MediaType, AudioCodecType};

/// Errors that can occur during SDP operations at the session layer
#[derive(Error, Debug)]
pub enum SdpError {
    #[error("Failed to parse or build SDP: {0}")]
    SdpProcessingError(String),
    
    #[error("Missing required SDP field: {0}")]
    MissingField(String),
    
    #[error("Media negotiation failed: {0}")]
    MediaNegotiationFailed(String),
    
    #[error("Unsupported codec or media type: {0}")]
    UnsupportedMedia(String),
}

/// Result type for SDP operations at the session layer
pub type Result<T> = std::result::Result<T, SdpError>;

/// Session Description - A convenient re-export of SdpSession from sip-core
/// This allows existing code to continue using SessionDescription without changes
pub type SessionDescription = SdpSession;

/// SDP negotiation state for tracking offer/answer model
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NegotiationState {
    /// No offer has been sent or received
    Initial,
    
    /// An offer has been sent, waiting for an answer
    OfferSent,
    
    /// An offer has been received, waiting to send an answer
    OfferReceived,
    
    /// A complete offer/answer exchange has happened
    Complete,
}

/// Direction of SDP exchange
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdpDirection {
    /// Local to remote
    Outgoing,
    
    /// Remote to local
    Incoming,
}

/// SDP context for a dialog or session
#[derive(Debug, Clone)]
pub struct SdpContext {
    /// Current local SDP
    pub local_sdp: Option<SessionDescription>,
    
    /// Current remote SDP
    pub remote_sdp: Option<SessionDescription>,
    
    /// Current negotiation state
    pub state: NegotiationState,
    
    /// Direction of the last exchange
    pub direction: SdpDirection,
}

impl SdpContext {
    /// Create a new SDP context
    pub fn new() -> Self {
        Self {
            local_sdp: None,
            remote_sdp: None,
            state: NegotiationState::Initial,
            direction: SdpDirection::Outgoing,
        }
    }
    
    /// Create a new SDP context with existing SDPs
    pub fn with_sdps(
        local_sdp: Option<SessionDescription>,
        remote_sdp: Option<SessionDescription>,
        state: NegotiationState,
        direction: SdpDirection,
    ) -> Self {
        Self {
            local_sdp,
            remote_sdp,
            state,
            direction,
        }
    }
    
    /// Update with a new local SDP offer
    pub fn update_with_local_offer(&mut self, offer: SessionDescription) {
        self.local_sdp = Some(offer);
        self.state = NegotiationState::OfferSent;
        self.direction = SdpDirection::Outgoing;
    }
    
    /// Update with a remote SDP offer
    pub fn update_with_remote_offer(&mut self, offer: SessionDescription) {
        self.remote_sdp = Some(offer);
        self.state = NegotiationState::OfferReceived;
        self.direction = SdpDirection::Incoming;
    }
    
    /// Update with a local answer to a remote offer
    pub fn update_with_local_answer(&mut self, answer: SessionDescription) {
        if self.state == NegotiationState::OfferReceived {
            self.local_sdp = Some(answer);
            self.state = NegotiationState::Complete;
        } else {
            warn!("Attempted to update with local answer when no remote offer was received");
        }
    }
    
    /// Update with a remote answer to a local offer
    pub fn update_with_remote_answer(&mut self, answer: SessionDescription) {
        if self.state == NegotiationState::OfferSent {
            self.remote_sdp = Some(answer);
            self.state = NegotiationState::Complete;
        } else {
            warn!("Attempted to update with remote answer when no local offer was sent");
        }
    }
    
    /// Check if the negotiation is complete
    pub fn is_complete(&self) -> bool {
        self.state == NegotiationState::Complete
    }
    
    /// Reset the negotiation state (e.g., for a re-INVITE)
    pub fn reset_for_renegotiation(&mut self) {
        self.state = NegotiationState::Initial;
    }
}

// Session Layer SDP Operations

/// Create a default audio SDP offer for an outgoing call
pub fn create_audio_offer(
    local_address: IpAddr,
    local_port: u16,
    supported_codecs: &[AudioCodecType]
) -> Result<SessionDescription> {
    // Create a random session ID
    let session_id = format!("{}", rand::random::<u64>());
    
    // Start building the SDP
    let mut builder = SdpBuilder::new("RVOIP Call")
        .origin("-", &session_id, "1", "IN", 
                if local_address.is_ipv4() { "IP4" } else { "IP6" },
                &local_address.to_string())
        .connection("IN", 
                if local_address.is_ipv4() { "IP4" } else { "IP6" },
                &local_address.to_string())
        .time("0", "0"); // Always active session
    
    // Add an audio media section
    let mut media_builder = builder.media_audio(local_port, "RTP/AVP");
    
    // Add formats based on supported codecs
    let format_strings = convert_codecs_to_formats(supported_codecs);
    media_builder = media_builder.formats(&format_strings);
    
    // Add rtpmap attributes for each codec
    for codec in supported_codecs {
        match codec {
            AudioCodecType::PCMU => {
                media_builder = media_builder.rtpmap("0", "PCMU/8000");
            },
            AudioCodecType::PCMA => {
                media_builder = media_builder.rtpmap("8", "PCMA/8000");
            },
            // Add more codec types as needed
        }
    }
    
    // Set direction to sendrecv and complete the media section
    media_builder = media_builder.direction(MediaDirection::SendRecv);
    
    // Build the final SDP
    let sdp = media_builder.done().build()
        .map_err(|e| SdpError::SdpProcessingError(e.to_string()))?;
    
    Ok(sdp)
}

/// Create an SDP answer based on a received offer
pub fn create_audio_answer(
    offer: &SessionDescription,
    local_address: IpAddr,
    local_port: u16,
    supported_codecs: &[AudioCodecType]
) -> Result<SessionDescription> {
    // Extract session information from offer for the answer
    let session_id = format!("{}", rand::random::<u64>());
    
    // Start building the SDP
    let mut builder = SdpBuilder::new("RVOIP Answer")
        .origin("-", &session_id, "1", "IN", 
                if local_address.is_ipv4() { "IP4" } else { "IP6" },
                &local_address.to_string())
        .connection("IN", 
                if local_address.is_ipv4() { "IP4" } else { "IP6" },
                &local_address.to_string())
        .time("0", "0"); // Always active session
    
    // Find audio media description in the offer
    let audio_media = offer.media_descriptions.iter()
        .find(|m| m.media == "audio")
        .ok_or_else(|| SdpError::MissingField("No audio media section in offer".to_string()))?;
    
    // Determine supported codecs that match the offer
    let mut matching_codecs = Vec::new();
    let mut matching_formats = Vec::new();
    
    // Extract payload types and their associated codecs from the offer
    for format in &audio_media.formats {
        // Look for rtpmap attributes to identify codecs
        for attr in &audio_media.generic_attributes {
            if let ParsedAttribute::RtpMap(RtpMapAttribute {
                payload_type, 
                encoding_name, 
                ..
            }) = attr {
                if format == &payload_type.to_string() {
                    // Check if we support this codec
                    if encoding_name == "PCMU" && supported_codecs.contains(&AudioCodecType::PCMU) {
                        matching_codecs.push(AudioCodecType::PCMU);
                        matching_formats.push(format.clone());
                    } else if encoding_name == "PCMA" && supported_codecs.contains(&AudioCodecType::PCMA) {
                        matching_codecs.push(AudioCodecType::PCMA);
                        matching_formats.push(format.clone());
                    }
                    // Add more codec checks as needed
                }
            }
        }
    }
    
    if matching_formats.is_empty() {
        return Err(SdpError::MediaNegotiationFailed(
            "No matching codecs found between offer and supported codecs".to_string()));
    }
    
    // Add an audio media section with negotiated codecs
    let mut media_builder = builder.media_audio(local_port, "RTP/AVP");
    
    // Add formats based on matched codecs
    media_builder = media_builder.formats(&matching_formats);
    
    // Add rtpmap attributes for each matched codec
    for (i, codec) in matching_codecs.iter().enumerate() {
        match codec {
            AudioCodecType::PCMU => {
                media_builder = media_builder.rtpmap(&matching_formats[i], "PCMU/8000");
            },
            AudioCodecType::PCMA => {
                media_builder = media_builder.rtpmap(&matching_formats[i], "PCMA/8000");
            },
            // Add more codec types as needed
        }
    }
    
    // Match the direction from offer or set default
    let direction = match audio_media.direction {
        Some(MediaDirection::SendOnly) => MediaDirection::RecvOnly,
        Some(MediaDirection::RecvOnly) => MediaDirection::SendOnly,
        _ => MediaDirection::SendRecv,
    };
    
    media_builder = media_builder.direction(direction);
    
    // Build the final SDP
    let sdp = media_builder.done().build()
        .map_err(|e| SdpError::SdpProcessingError(e.to_string()))?;
    
    Ok(sdp)
}

/// Extract a MediaConfig from a negotiated SDP
pub fn extract_media_config(
    local_sdp: &SessionDescription,
    remote_sdp: &SessionDescription
) -> Result<MediaConfig> {
    // Find audio media in both SDPs
    let local_audio = local_sdp.media_descriptions.iter()
        .find(|m| m.media == "audio")
        .ok_or_else(|| SdpError::MissingField("No audio media section in local SDP".to_string()))?;
    
    let remote_audio = remote_sdp.media_descriptions.iter()
        .find(|m| m.media == "audio")
        .ok_or_else(|| SdpError::MissingField("No audio media section in remote SDP".to_string()))?;
    
    // Get local port
    let local_port = local_audio.port;
    
    // Determine the remote connection information and port
    let remote_conn = remote_sdp.connection_info.as_ref()
        .or_else(|| remote_audio.connection_info.as_ref())
        .ok_or_else(|| SdpError::MissingField("No connection info in remote SDP".to_string()))?;
    
    let remote_addr = remote_conn.connection_address.parse::<IpAddr>()
        .map_err(|_| SdpError::SdpProcessingError("Invalid remote IP address".to_string()))?;
    
    let remote_port = remote_audio.port;
    let remote_socket = SocketAddr::new(remote_addr, remote_port);
    
    // Determine the negotiated codec
    // We'll use the first format in the answer as the negotiated codec
    if remote_audio.formats.is_empty() {
        return Err(SdpError::MissingField("No formats in remote SDP".to_string()));
    }
    
    let negotiated_format = &remote_audio.formats[0];
    
    // Find the rtpmap for this format to determine codec
    let mut payload_type = 0;
    let mut codec_type = AudioCodecType::PCMU; // Default
    let mut clock_rate = 8000;
    
    for attr in &remote_audio.generic_attributes {
        if let ParsedAttribute::RtpMap(RtpMapAttribute {
            payload_type: pt, 
            encoding_name,
            clock_rate: cr,
            ..
        }) = attr {
            if pt.to_string() == *negotiated_format {
                payload_type = *pt;
                clock_rate = *cr;
                
                // Determine codec type
                match encoding_name.as_str() {
                    "PCMU" => codec_type = AudioCodecType::PCMU,
                    "PCMA" => codec_type = AudioCodecType::PCMA,
                    _ => return Err(SdpError::UnsupportedMedia(format!(
                        "Unsupported codec: {}", encoding_name))),
                }
                
                break;
            }
        }
    }
    
    // Create the media config
    // Parse the IP address from the connection address string
    let local_addr = if let Some(conn) = local_sdp.connection_info.as_ref() {
        conn.connection_address.parse::<IpAddr>()
            .unwrap_or_else(|_| IpAddr::from([127, 0, 0, 1]))
    } else {
        IpAddr::from([127, 0, 0, 1])
    };
    
    let local_socket = SocketAddr::new(local_addr, local_port);
    
    let config = MediaConfig {
        local_addr: local_socket,
        remote_addr: Some(remote_socket),
        media_type: MediaType::Audio,
        payload_type,
        clock_rate,
        audio_codec: codec_type,
    };
    
    Ok(config)
}

/// Update an existing SDP for a re-INVITE (for media updates)
pub fn update_sdp_for_reinvite(
    original_sdp: &SessionDescription,
    new_local_port: Option<u16>,
    direction: Option<MediaDirection>
) -> Result<SessionDescription> {
    // Find the position of the audio media
    let audio_index = original_sdp.media_descriptions.iter()
        .position(|m| m.media == "audio")
        .ok_or_else(|| SdpError::MissingField("No audio media section in original SDP".to_string()))?;
    
    // Get the original media description
    let original_audio = &original_sdp.media_descriptions[audio_index];
    let port = new_local_port.unwrap_or(original_audio.port);
    
    // Build a new SDP with updated fields
    let mut builder = SdpBuilder::new(&original_sdp.session_name);
    
    // Update origin with incremented version
    let current_version = original_sdp.origin.sess_version.parse::<u64>().unwrap_or(0);
    let new_version = (current_version + 1).to_string();
    
    builder = builder.origin(
        &original_sdp.origin.username,
        &original_sdp.origin.sess_id,
        &new_version,
        &original_sdp.origin.net_type,
        &original_sdp.origin.addr_type,
        &original_sdp.origin.unicast_address
    );
    
    // Copy connection info
    if let Some(conn) = &original_sdp.connection_info {
        builder = builder.connection(
            &conn.net_type,
            &conn.addr_type,
            &conn.connection_address
        );
    }
    
    // Copy timing info
    if !original_sdp.time_descriptions.is_empty() {
        let time = &original_sdp.time_descriptions[0];
        builder = builder.time(&time.start_time, &time.stop_time);
    }
    
    // Add updated audio media
    let mut media_builder = builder.media_audio(
        port, 
        &original_audio.protocol
    );
    
    // Copy formats
    media_builder = media_builder.formats(&original_audio.formats);
    
    // Copy rtpmap attributes and other attributes
    for attr in &original_audio.generic_attributes {
        match attr {
            ParsedAttribute::RtpMap(rtpmap) => {
                media_builder = media_builder.rtpmap(
                    &rtpmap.payload_type.to_string(),
                    &format!("{}/{}", rtpmap.encoding_name, rtpmap.clock_rate)
                );
            },
            ParsedAttribute::Fmtp(fmtp) => {
                media_builder = media_builder.fmtp(
                    &fmtp.format,
                    &fmtp.parameters
                );
            },
            // Skip direction as it will be set below
            ParsedAttribute::Direction(_) => {},
            // Add handlers for other attribute types as needed
            _ => {}
        }
    }
    
    // Set the direction
    let media_direction = direction.unwrap_or_else(|| {
        original_audio.direction.unwrap_or(MediaDirection::SendRecv)
    });
    
    media_builder = media_builder.direction(media_direction);
    
    // Build the updated SDP
    let updated_sdp = media_builder.done().build()
        .map_err(|e| SdpError::SdpProcessingError(e.to_string()))?;
    
    Ok(updated_sdp)
}

/// Extract the RTP port from an SDP (version that preserves functionality from original)
pub fn extract_rtp_port_from_sdp(sdp: &[u8]) -> Option<u16> {
    trace!("Extracting RTP port from SDP bytes, length: {}", sdp.len());
    
    // First, convert the SDP bytes to string
    let sdp_str = match std::str::from_utf8(sdp) {
        Ok(s) => s,
        Err(e) => {
            warn!("Invalid UTF-8 in SDP: {}", e);
            return None;
        }
    };
    
    debug!("Parsing SDP:\n{}", sdp_str);
    
    // Try to parse the SDP using sip-core's parser
    match SdpSession::from_str(sdp_str) {
        Ok(session) => {
            // Find the audio media description
            for media in &session.media_descriptions {
                if media.media == "audio" {
                    debug!("Found audio media with port {}", media.port);
                    return Some(media.port);
                }
            }
            
            warn!("No audio media found in SDP");
        },
        Err(e) => {
            warn!("Failed to parse SDP: {}", e);
        }
    }
    
    // Fall back to manual parsing if structured parsing fails
    debug!("Falling back to manual parsing to extract RTP port");
    for (i, line) in sdp_str.lines().enumerate() {
        trace!("SDP line {}: {}", i, line);
        
        if line.starts_with("m=audio ") {
            debug!("Found audio media line: {}", line);
            // Format is "m=audio <port> RTP/AVP..."
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                match parts[1].parse::<u16>() {
                    Ok(port) => {
                        debug!("Manually extracted audio RTP port: {}", port);
                        return Some(port);
                    },
                    Err(e) => {
                        warn!("Failed to parse port number '{}': {}", parts[1], e);
                    }
                }
            } else {
                warn!("Invalid audio media line format, expected at least 3 parts: {}", line);
            }
        }
    }
    
    warn!("Could not extract RTP port from SDP");
    None
}

// Utility Functions

/// Convert AudioCodecType array to format strings for SDP
fn convert_codecs_to_formats(codecs: &[AudioCodecType]) -> Vec<String> {
    let mut formats = Vec::new();
    
    for codec in codecs {
        match codec {
            AudioCodecType::PCMU => formats.push("0".to_string()),
            AudioCodecType::PCMA => formats.push("8".to_string()),
            // Add more codecs as needed
        }
    }
    
    formats
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_create_audio_offer() {
        let local_addr = "192.168.1.2".parse::<IpAddr>().unwrap();
        let supported_codecs = vec![AudioCodecType::PCMU, AudioCodecType::PCMA];
        
        let offer = create_audio_offer(local_addr, 10000, &supported_codecs).unwrap();
        
        // Verify offer properties
        assert!(offer.session_name.contains("Call"));
        
        // Check that offer has audio media section
        let audio_media = offer.media_descriptions.iter()
            .find(|m| m.media == "audio")
            .expect("No audio media section found");
            
        assert_eq!(audio_media.port, 10000);
        assert!(audio_media.formats.contains(&"0".to_string())); // PCMU
        assert!(audio_media.formats.contains(&"8".to_string())); // PCMA
    }
    
    #[test]
    fn test_create_audio_answer() {
        // Create an offer first
        let local_addr = "192.168.1.2".parse::<IpAddr>().unwrap();
        let supported_codecs = vec![AudioCodecType::PCMU, AudioCodecType::PCMA];
        let offer = create_audio_offer(local_addr, 10000, &supported_codecs).unwrap();
        
        // Create an answer from a different address/port
        let answer_addr = "192.168.1.3".parse::<IpAddr>().unwrap();
        let answer_supported_codecs = vec![AudioCodecType::PCMU]; // Only support PCMU
        
        let answer = create_audio_answer(&offer, answer_addr, 20000, &answer_supported_codecs).unwrap();
        
        // Verify answer properties
        let audio_media = answer.media_descriptions.iter()
            .find(|m| m.media == "audio")
            .expect("No audio media section found");
            
        assert_eq!(audio_media.port, 20000);
        assert!(audio_media.formats.contains(&"0".to_string())); // PCMU
        assert!(!audio_media.formats.contains(&"8".to_string())); // No PCMA
    }
    
    #[test]
    fn test_extract_media_config() {
        // Create an offer and answer
        let local_addr = "192.168.1.2".parse::<IpAddr>().unwrap();
        let remote_addr = "192.168.1.3".parse::<IpAddr>().unwrap();
        let supported_codecs = vec![AudioCodecType::PCMU, AudioCodecType::PCMA];
        
        let offer = create_audio_offer(local_addr, 10000, &supported_codecs).unwrap();
        let answer = create_audio_answer(&offer, remote_addr, 20000, &supported_codecs).unwrap();
        
        // Extract media config
        let config = extract_media_config(&offer, &answer).unwrap();
        
        // Verify config properties
        assert_eq!(config.local_addr.port(), 10000);
        assert_eq!(config.remote_addr.unwrap().port(), 20000);
        assert_eq!(config.remote_addr.unwrap().ip(), remote_addr);
        assert_eq!(config.media_type, MediaType::Audio);
        // First codec in answer should be selected
        assert_eq!(config.payload_type, 0); // PCMU
        assert_eq!(config.audio_codec, AudioCodecType::PCMU);
    }
    
    #[test]
    fn test_extract_rtp_port() {
        // Create an SDP with a known port
        let local_addr = "192.168.1.2".parse::<IpAddr>().unwrap();
        let supported_codecs = vec![AudioCodecType::PCMU];
        let test_port = 12345;
        
        let sdp = create_audio_offer(local_addr, test_port, &supported_codecs).unwrap();
        
        // Convert to string and then to bytes
        let sdp_str = sdp.to_string();
        let sdp_bytes = sdp_str.as_bytes();
        
        // Extract port using our function
        let extracted_port = extract_rtp_port_from_sdp(sdp_bytes);
        
        assert_eq!(extracted_port, Some(test_port));
    }
    
    #[test]
    fn test_update_sdp_for_reinvite() {
        // Create an original SDP
        let local_addr = "192.168.1.2".parse::<IpAddr>().unwrap();
        let supported_codecs = vec![AudioCodecType::PCMU, AudioCodecType::PCMA];
        let original_sdp = create_audio_offer(local_addr, 10000, &supported_codecs).unwrap();
        
        // Update for re-INVITE with new port and direction
        let new_port = 11000;
        let updated_sdp = update_sdp_for_reinvite(
            &original_sdp, 
            Some(new_port), 
            Some(MediaDirection::SendOnly)
        ).unwrap();
        
        // Verify updates
        let audio_media = updated_sdp.media_descriptions.iter()
            .find(|m| m.media == "audio")
            .expect("No audio media section found");
            
        assert_eq!(audio_media.port, new_port);
        
        // Check direction
        let has_sendonly = audio_media.generic_attributes.iter()
            .any(|attr| matches!(attr, ParsedAttribute::Direction(MediaDirection::SendOnly)));
            
        assert!(has_sendonly, "Media direction should be sendonly");
    }
} 