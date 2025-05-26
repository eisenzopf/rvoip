//! SDP Coordination for Session Core
//!
//! This module provides coordination between SDP negotiation results (handled by transaction-core)
//! and media configuration (handled by media-core). session-core does NOT handle SDP creation,
//! parsing, or negotiation - that's the responsibility of sip-core and transaction-core.
//!
//! **ARCHITECTURAL PRINCIPLE**: session-core coordinates, it doesn't handle SIP protocol details.

use std::net::SocketAddr;
use anyhow::{Result, Context};
use serde::{Serialize, Deserialize};
use tracing::{debug, warn};

// Import SDP types from sip-core (the authoritative source)
pub use rvoip_sip_core::types::sdp::SdpSession;
pub use rvoip_sip_core::sdp::attributes::MediaDirection as SdpDirection;

// Import media types for coordination
use crate::media::{MediaConfig, SessionMediaType, AudioCodecType, SessionMediaDirection};

/// SDP negotiation state for coordination (simplified from old complex state machine)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NegotiationState {
    /// Initial state - no SDP exchanged
    Initial,
    /// Offer sent, waiting for answer
    OfferSent,
    /// Offer received, need to send answer
    OfferReceived,
    /// SDP negotiation complete
    Complete,
}

impl Default for NegotiationState {
    fn default() -> Self {
        Self::Initial
    }
}

/// SDP coordination context for session-core
/// 
/// This tracks the coordination state between SDP negotiation (handled by transaction-core)
/// and media setup (handled by media-core). session-core does NOT manage SDP state directly.
#[derive(Debug, Clone)]
pub struct SdpCoordinator {
    /// Whether SDP negotiation is complete (from transaction-core)
    pub negotiation_complete: bool,
    
    /// Current media configuration extracted from SDP
    pub media_config: Option<MediaConfig>,
    
    /// Whether media has been set up based on SDP
    pub media_coordinated: bool,
}

/// Legacy SdpContext for backward compatibility during refactoring
/// 
/// **NOTE**: This is a temporary bridge during the architectural refactoring.
/// Eventually, this should be replaced with proper event-driven coordination.
#[derive(Debug, Clone)]
pub struct SdpContext {
    /// Current SDP negotiation state
    pub state: NegotiationState,
    /// Local SDP session (from sip-core)
    pub local_sdp: Option<SdpSession>,
    /// Remote SDP session (from sip-core)
    pub remote_sdp: Option<SdpSession>,
    /// Negotiated media configuration
    pub media_config: Option<MediaConfig>,
}

impl Default for SdpContext {
    fn default() -> Self {
        Self {
            state: NegotiationState::Initial,
            local_sdp: None,
            remote_sdp: None,
            media_config: None,
        }
    }
}

impl SdpContext {
    /// Create a new SDP context
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set local SDP offer
    pub fn set_local_offer(&mut self, sdp: SdpSession) -> Result<()> {
        self.local_sdp = Some(sdp);
        self.state = NegotiationState::OfferSent;
        Ok(())
    }
    
    /// Set remote SDP offer
    pub fn set_remote_offer(&mut self, sdp: SdpSession) -> Result<()> {
        self.remote_sdp = Some(sdp);
        self.state = NegotiationState::OfferReceived;
        Ok(())
    }
    
    /// Set local SDP answer
    pub fn set_local_answer(&mut self, sdp: SdpSession) -> Result<()> {
        self.local_sdp = Some(sdp);
        self.state = NegotiationState::Complete;
        Ok(())
    }
    
    /// Set remote SDP answer
    pub fn set_remote_answer(&mut self, sdp: SdpSession) -> Result<()> {
        self.remote_sdp = Some(sdp);
        self.state = NegotiationState::Complete;
        Ok(())
    }
    
    /// Check if SDP negotiation is complete
    pub fn is_complete(&self) -> bool {
        self.state == NegotiationState::Complete
    }
    
    /// Get negotiated media configuration
    pub fn get_media_config(&self) -> Option<&MediaConfig> {
        self.media_config.as_ref()
    }
    
    /// Legacy method aliases for backward compatibility
    pub fn update_with_remote_answer(&mut self, sdp: SdpSession) -> Result<()> {
        self.set_remote_answer(sdp)
    }
    
    pub fn update_with_remote_offer(&mut self, sdp: SdpSession) -> Result<()> {
        self.set_remote_offer(sdp)
    }
    
    pub fn update_with_local_offer(&mut self, sdp: SdpSession) -> Result<()> {
        self.set_local_offer(sdp)
    }
    
    pub fn update_with_local_answer(&mut self, sdp: SdpSession) -> Result<()> {
        self.set_local_answer(sdp)
    }
    
    pub fn reset_for_renegotiation(&mut self) {
        self.state = NegotiationState::Initial;
        self.local_sdp = None;
        self.remote_sdp = None;
        self.media_config = None;
    }
}

/// Legacy SessionDescription for backward compatibility
/// 
/// **NOTE**: This is a temporary alias during refactoring. 
/// Code should be updated to use sip-core's SdpSession directly.
pub type SessionDescription = SdpSession;

/// Helper functions for SessionDescription compatibility
pub fn parse_session_description(sdp_str: &str) -> Result<SessionDescription> {
    let bytes = bytes::Bytes::from(sdp_str.to_string());
    rvoip_sip_core::sdp::parser::parse_sdp(&bytes)
        .map_err(|e| anyhow::anyhow!("Failed to parse SDP: {}", e))
}

impl Default for SdpCoordinator {
    fn default() -> Self {
        Self {
            negotiation_complete: false,
            media_config: None,
            media_coordinated: false,
        }
    }
}

impl SdpCoordinator {
    /// Create a new SDP coordinator
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Mark SDP negotiation as complete (called when transaction-core completes negotiation)
    pub fn mark_negotiation_complete(&mut self, media_config: MediaConfig) {
        self.negotiation_complete = true;
        self.media_config = Some(media_config);
        debug!("SDP negotiation marked complete, media config extracted");
    }
    
    /// Mark media as coordinated (called when media-core setup is complete)
    pub fn mark_media_coordinated(&mut self) {
        self.media_coordinated = true;
        debug!("Media coordination marked complete");
    }
    
    /// Check if both SDP and media coordination are complete
    pub fn is_fully_coordinated(&self) -> bool {
        self.negotiation_complete && self.media_coordinated
    }
    
    /// Get the coordinated media configuration
    pub fn get_media_config(&self) -> Option<&MediaConfig> {
        self.media_config.as_ref()
    }
    
    /// Reset coordination state (for re-negotiation scenarios)
    pub fn reset(&mut self) {
        self.negotiation_complete = false;
        self.media_config = None;
        self.media_coordinated = false;
        debug!("SDP coordination state reset");
    }
}

/// Extract media configuration from completed SDP session (from sip-core)
/// 
/// This function takes an SDP session that has been negotiated by transaction-core
/// and extracts the media configuration needed by media-core.
pub fn extract_media_config_from_sdp(
    sdp: &SdpSession, 
    local_addr: SocketAddr,
    is_offerer: bool
) -> Result<MediaConfig> {
    debug!("Extracting media config from negotiated SDP");
    
    // Get the first media description (typically audio)
    let media_desc = sdp.media_descriptions.first()
        .ok_or_else(|| anyhow::anyhow!("No media description found in SDP"))?;
    
    // Extract port from media description
    let remote_port = media_desc.port;
    
    // Extract connection address from SDP (use connection_info instead of connection_data)
    let connection_addr = if let Some(ref conn) = media_desc.connection_info {
        conn.connection_address.clone()
    } else if let Some(ref conn) = sdp.connection_info {
        conn.connection_address.clone()
    } else {
        "127.0.0.1".to_string()
    };
    
    // Create remote address
    let remote_addr: SocketAddr = format!("{}:{}", connection_addr, remote_port)
        .parse()
        .context("Failed to parse remote media address from SDP")?;
    
    // Extract codec information from first format
    let payload_type = media_desc.formats.first()
        .and_then(|f| f.parse::<u8>().ok())
        .unwrap_or(0);
    
    // Determine codec from payload type (standard mappings)
    let audio_codec = match payload_type {
        0 => AudioCodecType::PCMU,
        8 => AudioCodecType::PCMA,
        9 => AudioCodecType::G722,
        111 => AudioCodecType::Opus,
        _ => {
            warn!("Unknown payload type {}, defaulting to PCMU", payload_type);
            AudioCodecType::PCMU
        }
    };
    
    // Extract media direction from SDP attributes
    let direction = media_desc.direction
        .map(|d| match d {
            rvoip_sip_core::sdp::attributes::MediaDirection::SendRecv => SessionMediaDirection::SendRecv,
            rvoip_sip_core::sdp::attributes::MediaDirection::SendOnly => SessionMediaDirection::SendOnly,
            rvoip_sip_core::sdp::attributes::MediaDirection::RecvOnly => SessionMediaDirection::RecvOnly,
            rvoip_sip_core::sdp::attributes::MediaDirection::Inactive => SessionMediaDirection::Inactive,
        })
        .unwrap_or(SessionMediaDirection::SendRecv);
    
    let config = MediaConfig {
        local_addr,
        remote_addr: Some(remote_addr),
        media_type: SessionMediaType::Audio,
        payload_type,
        clock_rate: audio_codec.clock_rate(),
        audio_codec,
        direction,
    };
    
    debug!("Extracted media config from SDP: {:?}", config);
    Ok(config)
}

/// Legacy extract_media_config function for backward compatibility
pub fn extract_media_config(
    local_sdp: &SdpSession,
    remote_sdp: &SdpSession,
) -> Result<MediaConfig> {
    // Use the remote SDP as the source of truth for media configuration
    extract_media_config_from_sdp(remote_sdp, "127.0.0.1:10000".parse().unwrap(), false)
}

/// Convert SDP direction to session media direction
pub fn sdp_direction_to_media_direction(sdp_dir: SdpDirection) -> SessionMediaDirection {
    match sdp_dir {
        SdpDirection::SendRecv => SessionMediaDirection::SendRecv,
        SdpDirection::SendOnly => SessionMediaDirection::SendOnly,
        SdpDirection::RecvOnly => SessionMediaDirection::RecvOnly,
        SdpDirection::Inactive => SessionMediaDirection::Inactive,
    }
}

/// Convert session media direction to SDP direction
pub fn media_direction_to_sdp_direction(media_dir: SessionMediaDirection) -> SdpDirection {
    match media_dir {
        SessionMediaDirection::SendRecv => SdpDirection::SendRecv,
        SessionMediaDirection::SendOnly => SdpDirection::SendOnly,
        SessionMediaDirection::RecvOnly => SdpDirection::RecvOnly,
        SessionMediaDirection::Inactive => SdpDirection::Inactive,
    }
}

/// Create media configuration for outgoing calls (before SDP negotiation)
/// 
/// This creates a default media configuration that can be used to generate
/// SDP offers via sip-core's SdpBuilder.
pub fn create_default_media_config(local_addr: SocketAddr) -> MediaConfig {
    MediaConfig {
        local_addr,
        remote_addr: None, // Will be filled after SDP answer
        media_type: SessionMediaType::Audio,
        payload_type: 0, // PCMU
        clock_rate: 8000,
        audio_codec: AudioCodecType::PCMU,
        direction: SessionMediaDirection::SendRecv,
    }
}

/// Update media configuration after SDP re-negotiation
/// 
/// This is used when an existing session receives a re-INVITE or UPDATE
/// with new SDP, requiring media coordination updates.
pub fn update_media_config_from_renegotiation(
    current_config: &MediaConfig,
    new_sdp: &SdpSession,
    is_offerer: bool
) -> Result<MediaConfig> {
    debug!("Updating media config from SDP re-negotiation");
    
    // Extract new configuration from SDP
    let mut new_config = extract_media_config_from_sdp(new_sdp, current_config.local_addr, is_offerer)?;
    
    // Preserve local address from current config
    new_config.local_addr = current_config.local_addr;
    
    debug!("Updated media config: {:?}", new_config);
    Ok(new_config)
}

/// Legacy function for SDP updates (placeholder)
pub fn update_sdp_for_reinvite(
    _current_sdp: &SdpSession,
    _new_media_config: &MediaConfig,
) -> Result<SdpSession> {
    // TODO: Implement proper SDP update logic using sip-core
    // For now, return a basic SDP
    let sdp = rvoip_sip_core::sdp::SdpBuilder::new("Updated Session")
        .origin("-", &chrono::Utc::now().timestamp().to_string(), "1", "IN", "IP4", "127.0.0.1")
        .connection("IN", "IP4", "127.0.0.1")
        .time("0", "0")
        .media_audio(10000, "RTP/AVP")
            .formats(&["0"])
            .rtpmap("0", "PCMU/8000")
            .direction(rvoip_sip_core::sdp::attributes::MediaDirection::SendRecv)
            .done()
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to build updated SDP: {}", e))?;
    
    Ok(sdp)
} 