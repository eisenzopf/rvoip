//! Media Configuration Conversion
//!
//! This module provides conversion between session-core media configuration
//! and media-core configuration types, implementing the coordination layer
//! between the two systems.

use std::net::SocketAddr;
use anyhow::Result;
use tracing::{debug, warn};

use crate::sdp::SdpSession;
use crate::media::{MediaConfig, AudioCodecType, SessionMediaType, SessionMediaDirection};

// Import media-core types
use rvoip_media_core::prelude::*;

/// MediaConfigConverter handles conversion between session-core and media-core types
pub struct MediaConfigConverter;

impl MediaConfigConverter {
    /// Convert session-core MediaConfig to media-core MediaSessionParams
    pub fn to_media_session_params(config: &MediaConfig) -> MediaSessionParams {
        debug!("Converting MediaConfig to MediaSessionParams: {:?}", config);
        
        // Create basic audio-only params
        let params = MediaSessionParams::audio_only();
        
        // Set preferred codec
        let params = params.with_preferred_codec(config.audio_codec.to_payload_type());
        
        // Note: Sample rate configuration may not be available in all media-core versions
        // For now, we'll rely on the codec's default sample rate
        debug!("Converted to MediaSessionParams");
        params
    }
    
    /// Convert media-core codec info to session-core AudioCodecType
    pub fn from_payload_type(payload_type: u8) -> AudioCodecType {
        match payload_type {
            0 => AudioCodecType::PCMU,
            8 => AudioCodecType::PCMA,
            9 => AudioCodecType::G722,
            111 => AudioCodecType::Opus,
            _ => {
                warn!("Unknown payload type {}, defaulting to PCMU", payload_type);
                AudioCodecType::PCMU
            }
        }
    }
    
    /// Create MediaConfig from SDP session and local preferences
    pub fn from_sdp_session(
        sdp: &SdpSession,
        local_addr: SocketAddr,
        preferred_codec: AudioCodecType,
    ) -> Result<MediaConfig> {
        debug!("Creating MediaConfig from SDP session");
        
        // Get first media description (typically audio)
        let media_desc = sdp.media_descriptions.first()
            .ok_or_else(|| anyhow::anyhow!("No media description found in SDP"))?;
        
        // Extract remote port
        let remote_port = media_desc.port;
        
        // Extract connection address
        let connection_addr = if let Some(ref conn) = media_desc.connection_info {
            conn.connection_address.clone()
        } else {
            sdp.connection_info.as_ref()
                .map(|c| c.connection_address.clone())
                .unwrap_or_else(|| "127.0.0.1".to_string())
        };
        
        // Create remote address
        let remote_addr: SocketAddr = format!("{}:{}", connection_addr, remote_port)
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse remote address: {}", e))?;
        
        // Extract payload type from formats
        let payload_type = media_desc.formats.first()
            .and_then(|f| f.parse::<u8>().ok())
            .unwrap_or(preferred_codec.to_payload_type());
        
        // Determine actual codec
        let audio_codec = Self::from_payload_type(payload_type);
        
        // Extract media direction
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
        
        debug!("Created MediaConfig: {:?}", config);
        Ok(config)
    }
    
    /// Create default MediaConfig for outgoing calls
    pub fn create_default_outgoing(local_addr: SocketAddr) -> MediaConfig {
        MediaConfig {
            local_addr,
            remote_addr: None, // Will be set after SDP answer
            media_type: SessionMediaType::Audio,
            payload_type: 0, // PCMU
            clock_rate: 8000,
            audio_codec: AudioCodecType::PCMU,
            direction: SessionMediaDirection::SendRecv,
        }
    }
    
    /// Create MediaConfig for incoming calls from SDP offer
    pub fn create_for_incoming(
        offer_sdp: &SdpSession,
        local_addr: SocketAddr,
    ) -> Result<MediaConfig> {
        Self::from_sdp_session(offer_sdp, local_addr, AudioCodecType::PCMU)
    }
    
    /// Update MediaConfig with SDP answer
    pub fn update_with_answer(
        mut config: MediaConfig,
        answer_sdp: &SdpSession,
    ) -> Result<MediaConfig> {
        debug!("Updating MediaConfig with SDP answer");
        
        // Extract remote address from answer
        let media_desc = answer_sdp.media_descriptions.first()
            .ok_or_else(|| anyhow::anyhow!("No media description in SDP answer"))?;
        
        let remote_port = media_desc.port;
        let connection_addr = if let Some(ref conn) = media_desc.connection_info {
            conn.connection_address.clone()
        } else {
            answer_sdp.connection_info.as_ref()
                .map(|c| c.connection_address.clone())
                .unwrap_or_else(|| "127.0.0.1".to_string())
        };
        
        let remote_addr: SocketAddr = format!("{}:{}", connection_addr, remote_port)
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse remote address from answer: {}", e))?;
        
        config.remote_addr = Some(remote_addr);
        
        // Update codec if different in answer
        if let Some(format) = media_desc.formats.first() {
            if let Ok(payload_type) = format.parse::<u8>() {
                config.payload_type = payload_type;
                config.audio_codec = Self::from_payload_type(payload_type);
                config.clock_rate = config.audio_codec.clock_rate();
            }
        }
        
        debug!("Updated MediaConfig: {:?}", config);
        Ok(config)
    }
    
    /// Convert media direction between session-core and media-core
    pub fn convert_direction_to_media_core(direction: SessionMediaDirection) -> rvoip_media_core::MediaDirection {
        match direction {
            SessionMediaDirection::SendRecv => rvoip_media_core::MediaDirection::SendRecv,
            SessionMediaDirection::SendOnly => rvoip_media_core::MediaDirection::SendOnly,
            SessionMediaDirection::RecvOnly => rvoip_media_core::MediaDirection::RecvOnly,
            SessionMediaDirection::Inactive => rvoip_media_core::MediaDirection::Inactive,
        }
    }
    
    /// Convert media direction from media-core to session-core
    pub fn convert_direction_from_media_core(direction: rvoip_media_core::MediaDirection) -> SessionMediaDirection {
        match direction {
            rvoip_media_core::MediaDirection::SendRecv => SessionMediaDirection::SendRecv,
            rvoip_media_core::MediaDirection::SendOnly => SessionMediaDirection::SendOnly,
            rvoip_media_core::MediaDirection::RecvOnly => SessionMediaDirection::RecvOnly,
            rvoip_media_core::MediaDirection::Inactive => SessionMediaDirection::Inactive,
        }
    }
} 