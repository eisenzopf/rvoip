use std::net::SocketAddr;
use anyhow::Result;
use tracing::{debug, warn};

// Import media-core components
use rvoip_media_core::prelude::*;

use crate::sdp::{SessionDescription, SdpDirection};
use super::{AudioCodecType, MediaConfig, SessionMediaType, SessionMediaDirection};

/// Media configuration converter between session-core and media-core
pub struct MediaConfigConverter;

impl MediaConfigConverter {
    /// Convert session-core AudioCodecType to media-core PayloadType
    pub fn audio_codec_to_payload_type(codec: AudioCodecType) -> PayloadType {
        match codec {
            AudioCodecType::PCMU => payload_types::PCMU,
            AudioCodecType::PCMA => payload_types::PCMA,
            AudioCodecType::G722 => payload_types::G722,
            AudioCodecType::Opus => payload_types::OPUS,
        }
    }
    
    /// Convert media-core PayloadType to session-core AudioCodecType
    pub fn payload_type_to_audio_codec(payload_type: PayloadType) -> Option<AudioCodecType> {
        // Note: PayloadType is likely a newtype, so we need to compare differently
        if payload_type == payload_types::PCMU {
            Some(AudioCodecType::PCMU)
        } else if payload_type == payload_types::PCMA {
            Some(AudioCodecType::PCMA)
        } else if payload_type == payload_types::G722 {
            Some(AudioCodecType::G722)
        } else if payload_type == payload_types::OPUS {
            Some(AudioCodecType::Opus)
        } else {
            None
        }
    }
    
    /// Convert session-core MediaConfig to media-core MediaSessionParams
    pub fn to_media_session_params(config: &MediaConfig) -> MediaSessionParams {
        let payload_type = Self::audio_codec_to_payload_type(config.audio_codec);
        
        // Create basic audio-only params
        let mut params = MediaSessionParams::audio_only()
            .with_preferred_codec(payload_type);
        
        // Try to set sample rate if the method exists
        if let Ok(sample_rate) = SampleRate::from_hz(config.clock_rate) {
            // Note: with_sample_rate might not exist, so we'll handle this gracefully
            // params = params.with_sample_rate(sample_rate);
        }
        
        params
    }
    
    /// Create MediaConfig from SDP and codec preferences
    pub fn from_sdp_negotiation(
        sdp: &SessionDescription, 
        preferred_codec: AudioCodecType,
        local_addr: SocketAddr
    ) -> Result<MediaConfig> {
        debug!("Creating MediaConfig from SDP negotiation");
        
        // Extract media information from SDP
        let (rtp_port, remote_addr, direction) = Self::extract_media_info_from_sdp(sdp)?;
        
        // Create local address with extracted or default port
        let local_media_addr = SocketAddr::new(local_addr.ip(), rtp_port.unwrap_or(10000));
        
        let config = MediaConfig {
            local_addr: local_media_addr,
            remote_addr,
            media_type: SessionMediaType::Audio,
            payload_type: Self::audio_codec_to_payload_type(preferred_codec).0,
            clock_rate: preferred_codec.clock_rate(),
            audio_codec: preferred_codec,
            direction: direction.unwrap_or(SessionMediaDirection::SendRecv),
        };
        
        debug!("Created MediaConfig from SDP: {:?}", config);
        Ok(config)
    }
    
    /// Extract media information from SDP
    fn extract_media_info_from_sdp(sdp: &SessionDescription) -> Result<(Option<u16>, Option<SocketAddr>, Option<SessionMediaDirection>)> {
        debug!("Extracting media info from SDP");
        
        // For now, we'll use a simplified approach since SessionDescription structure is complex
        // TODO: Implement proper SDP parsing using the SessionDescription fields
        
        // Return defaults for now
        Ok((Some(10000), None, Some(SessionMediaDirection::SendRecv)))
    }
    
    /// Create SDP offer for outgoing calls
    pub fn create_sdp_offer(
        config: &MediaConfig,
        session_id: u64,
        session_version: u64
    ) -> Result<SessionDescription> {
        debug!("Creating SDP offer for config: {:?}", config);
        
        // For now, create a basic SDP structure
        // TODO: Use proper SDP builder from sip-core
        
        let sdp_content = format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP {}\r\n\
             a=rtpmap:{} {}/{}\r\n\
             a={}\r\n",
            session_id,
            session_version,
            config.local_addr.ip(),
            config.local_addr.ip(),
            config.local_addr.port(),
            config.payload_type,
            config.payload_type,
            Self::get_codec_name(config.audio_codec),
            config.clock_rate,
            Self::direction_to_sdp_attribute(config.direction)
        );
        
        // Parse the SDP content into SessionDescription
        // TODO: Use proper SDP parsing
        let sdp = crate::sdp::SessionDescription::from_str(&sdp_content)
            .map_err(|e| anyhow::anyhow!("Failed to parse SDP: {}", e))?;
        
        debug!("Created SDP offer");
        Ok(sdp)
    }
    
    /// Create SDP answer for incoming calls
    pub fn create_sdp_answer(
        config: &MediaConfig,
        offer_sdp: &SessionDescription,
        session_id: u64,
        session_version: u64
    ) -> Result<SessionDescription> {
        debug!("Creating SDP answer for config: {:?}", config);
        
        // For now, create a basic SDP answer
        // TODO: Implement proper SDP answer generation based on offer
        
        let sdp_content = format!(
            "v=0\r\n\
             o=- {} {} IN IP4 {}\r\n\
             s=Session\r\n\
             c=IN IP4 {}\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP {}\r\n\
             a=rtpmap:{} {}/{}\r\n\
             a={}\r\n",
            session_id,
            session_version,
            config.local_addr.ip(),
            config.local_addr.ip(),
            config.local_addr.port(),
            config.payload_type,
            config.payload_type,
            Self::get_codec_name(config.audio_codec),
            config.clock_rate,
            Self::direction_to_sdp_attribute(config.direction)
        );
        
        // Parse the SDP content into SessionDescription
        let sdp = crate::sdp::SessionDescription::from_str(&sdp_content)
            .map_err(|e| anyhow::anyhow!("Failed to parse SDP: {}", e))?;
        
        debug!("Created SDP answer");
        Ok(sdp)
    }
    
    /// Get codec name for SDP rtpmap
    fn get_codec_name(codec: AudioCodecType) -> &'static str {
        match codec {
            AudioCodecType::PCMU => "PCMU",
            AudioCodecType::PCMA => "PCMA",
            AudioCodecType::G722 => "G722",
            AudioCodecType::Opus => "opus",
        }
    }
    
    /// Convert SessionMediaDirection to SDP direction attribute
    fn direction_to_sdp_attribute(direction: SessionMediaDirection) -> &'static str {
        match direction {
            SessionMediaDirection::SendRecv => "sendrecv",
            SessionMediaDirection::SendOnly => "sendonly",
            SessionMediaDirection::RecvOnly => "recvonly",
            SessionMediaDirection::Inactive => "inactive",
        }
    }
    
    /// Convert SDP direction to SessionMediaDirection
    pub fn sdp_direction_to_media_direction(sdp_direction: SdpDirection) -> SessionMediaDirection {
        // Note: SdpDirection in session-core is different (Outgoing/Incoming)
        // For now, default to SendRecv
        SessionMediaDirection::SendRecv
    }
    
    /// Convert SessionMediaDirection to SDP direction
    pub fn media_direction_to_sdp_direction(media_direction: SessionMediaDirection) -> SdpDirection {
        // Note: SdpDirection in session-core is different (Outgoing/Incoming)
        // For now, default to Outgoing
        SdpDirection::Outgoing
    }
    
    /// Negotiate codec from SDP offer and supported codecs
    pub fn negotiate_codec_from_sdp(
        offer_sdp: &SessionDescription,
        supported_codecs: &[PayloadType]
    ) -> Result<AudioCodecType> {
        debug!("Negotiating codec from SDP offer");
        
        // For now, default to PCMU
        // TODO: Implement proper codec negotiation
        warn!("Codec negotiation not fully implemented, defaulting to PCMU");
        Ok(AudioCodecType::PCMU)
    }
    
    /// Validate media configuration
    pub fn validate_config(config: &MediaConfig) -> Result<()> {
        debug!("Validating media config: {:?}", config);
        
        // Validate port range
        if config.local_addr.port() < 1024 {
            return Err(anyhow::anyhow!("Local port {} is in reserved range", config.local_addr.port()));
        }
        
        // Validate clock rate
        let expected_clock_rate = config.audio_codec.clock_rate();
        if config.clock_rate != expected_clock_rate {
            warn!("Clock rate {} doesn't match expected {} for codec {:?}", 
                  config.clock_rate, expected_clock_rate, config.audio_codec);
        }
        
        // Validate payload type range
        if config.payload_type > 127 {
            return Err(anyhow::anyhow!("Payload type {} is out of valid range (0-127)", config.payload_type));
        }
        
        debug!("Media config validation passed");
        Ok(())
    }
} 