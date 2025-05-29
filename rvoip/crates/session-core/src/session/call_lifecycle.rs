//! Call Lifecycle Coordination for Session Layer
//!
//! This module manages the complete call lifecycle coordination at the session level.
//! It handles the application logic for call acceptance, rejection, and state management,
//! coordinating between SessionManager, DialogManager, and MediaManager.
//!
//! **ARCHITECTURAL NOTE**: This was moved from dialog layer to session layer to fix
//! RFC 3261 separation violations. Call coordination belongs in the session layer,
//! not the protocol layer.

use std::sync::Arc;
use anyhow::Result;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};
use chrono;

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{HeaderName, TypedHeader};
use rvoip_transaction_core::TransactionKey;
use crate::media::MediaManager;
use crate::session::SessionId;
use uuid;

/// Session-level call lifecycle coordinator
///
/// This struct manages the complete call lifecycle from the session perspective,
/// coordinating between session state, dialog operations, and media setup.
/// It belongs in SessionManager (coordination layer), not DialogManager (protocol layer).
pub struct CallLifecycleCoordinator {
    media_manager: Arc<MediaManager>,
}

impl CallLifecycleCoordinator {
    /// Create a new session-level call lifecycle coordinator
    pub fn new(media_manager: Arc<MediaManager>) -> Self {
        Self {
            media_manager,
        }
    }

    /// Coordinate complete session establishment
    ///
    /// This method manages the session-level coordination for call establishment:
    /// 1. Coordinate with media for SDP negotiation
    /// 2. Manage session state transitions
    /// 3. Coordinate bi-directional media flow setup
    pub async fn coordinate_session_establishment(
        &self,
        session_id: &SessionId,
        offer_sdp: &str,
    ) -> Result<String> {
        info!(
            session_id = %session_id,
            "🎯 Coordinating session establishment with media setup"
        );

        // Step 1: Create SDP answer through media-core coordination
        let answer_sdp = self.create_media_answer(session_id, offer_sdp).await?;
        debug!(
            session_id = %session_id,
            "🎵 Created SDP answer through media-core"
        );

        // Step 2: Establish bi-directional media flow
        if let Err(e) = self.establish_media_flow_for_session(session_id, offer_sdp).await {
            warn!(
                session_id = %session_id,
                error = %e,
                "Failed to establish media flow - session will continue without audio"
            );
        } else {
            info!(
                session_id = %session_id,
                "✅ Bi-directional media flow established successfully"
            );
        }

        info!(
            session_id = %session_id,
            "✅ Session establishment coordination completed successfully"
        );

        Ok(answer_sdp)
    }

    /// Coordinate complete session termination
    ///
    /// This method manages the session-level coordination for call termination:
    /// 1. Coordinate media session cleanup
    /// 2. Manage session state transitions
    /// 3. Emit session termination events
    pub async fn coordinate_session_termination(&self, session_id: &SessionId) -> Result<()> {
        info!(
            session_id = %session_id,
            "🛑 Coordinating session termination with media cleanup"
        );

        // Step 1: Coordinate media session cleanup
        self.coordinate_media_cleanup(session_id).await?;

        // Step 2: Emit session termination events
        self.emit_session_termination_events(session_id).await?;

        info!(
            session_id = %session_id,
            "✅ Session termination coordination completed successfully"
        );

        Ok(())
    }
    
    /// Establish bi-directional media flow for a session
    async fn establish_media_flow_for_session(&self, session_id: &SessionId, offer_sdp: &str) -> Result<()> {
        debug!(session_id = %session_id, "🔗 Establishing bi-directional media flow");
        
        // Get the media session for this session
        if let Some(media_session_id) = self.media_manager.get_media_session(session_id).await {
            // Parse remote address from SDP
            let (remote_port, _codec) = self.parse_offer_sdp(offer_sdp)?;
            let remote_addr = format!("127.0.0.1:{}", remote_port).parse()
                .map_err(|e| anyhow::anyhow!("Invalid remote address: {}", e))?;
            
            // Establish media flow with bi-directional audio transmission
            self.media_manager.establish_media_flow(&media_session_id, remote_addr).await?;
            
            info!(
                session_id = %session_id,
                media_session_id = %media_session_id,
                remote_addr = %remote_addr,
                "✅ Bi-directional media flow established"
            );
        } else {
            return Err(anyhow::anyhow!("No media session found for session: {}", session_id));
        }
        
        Ok(())
    }

    /// Coordinate media session cleanup
    async fn coordinate_media_cleanup(&self, session_id: &SessionId) -> Result<()> {
        info!(
            session_id = %session_id,
            "🎵 Coordinating media session cleanup"
        );

        if let Some(media_session_id) = self.media_manager.get_media_session(session_id).await {
            // Terminate media flow (stops audio transmission)
            if let Err(e) = self.media_manager.terminate_media_flow(&media_session_id).await {
                warn!(
                    session_id = %session_id,
                    media_session_id = %media_session_id,
                    error = %e,
                    "Failed to terminate media flow - continuing with cleanup"
                );
            } else {
                info!(
                    session_id = %session_id,
                    media_session_id = %media_session_id,
                    "🛑 Media flow terminated successfully"
                );
            }

            // Stop the media session completely
            if let Err(e) = self.media_manager.stop_media(&media_session_id, "Session termination".to_string()).await {
                warn!(
                    session_id = %session_id,
                    media_session_id = %media_session_id,
                    error = %e,
                    "Failed to stop media session completely"
                );
            } else {
                info!(
                    session_id = %session_id,
                    media_session_id = %media_session_id,
                    "🧹 Media session stopped completely"
                );
            }
        } else {
            debug!(
                session_id = %session_id,
                "No media session found for cleanup - may have already been cleaned up"
            );
        }

        Ok(())
    }

    /// Emit session termination events
    async fn emit_session_termination_events(&self, session_id: &SessionId) -> Result<()> {
        info!(
            session_id = %session_id,
            "📡 Emitting session termination events"
        );

        // TODO: Integrate with event bus to emit proper session termination events
        // For now, we'll just log the event emission
        info!(
            session_id = %session_id,
            "✅ Session termination events emitted (placeholder implementation)"
        );

        Ok(())
    }

    /// Create SDP answer using media-core integration
    async fn create_media_answer(&self, session_id: &SessionId, offer_sdp: &str) -> Result<String> {
        info!(
            session_id = %session_id,
            "🎵 Creating SDP answer with media-core integration"
        );

        // Get supported codecs from media-core
        let supported_codecs = self.media_manager.get_supported_codecs().await;
        info!("🎼 Media-core supported codecs: {:?}", supported_codecs);
        
        // Convert u8 payload types to String format for negotiation
        let supported_codec_strings: Vec<String> = supported_codecs.iter().map(|&pt| pt.to_string()).collect();
        
        // Parse the offer to extract remote capabilities
        let (remote_port, offered_codec) = self.parse_offer_sdp(offer_sdp)?;
        
        // Create media configuration for this session
        let local_addr = "127.0.0.1:0".parse().unwrap(); // Let MediaManager allocate port
        let media_config = crate::media::MediaConfig {
            local_addr,
            remote_addr: Some(format!("127.0.0.1:{}", remote_port).parse().unwrap()),
            media_type: crate::media::SessionMediaType::Audio,
            payload_type: offered_codec.to_payload_type(),
            clock_rate: offered_codec.clock_rate(),
            audio_codec: offered_codec,
            direction: crate::media::SessionMediaDirection::SendRecv,
        };
        
        // Create media session which will allocate the actual RTP port
        let media_session_id = self.media_manager.create_media_session(media_config).await?;
        
        // Get the actual allocated port from the media session
        let local_port = if let Some(session_info) = self.media_manager.media_controller().get_session_info(media_session_id.as_str()).await {
            session_info.rtp_port.unwrap_or(10000)
        } else {
            10000 // fallback port
        };
        
        info!("🔌 Using allocated RTP port: {} (remote: {})", local_port, remote_port);
        
        // Negotiate codecs
        let negotiated_codecs = self.negotiate_codecs(offer_sdp, &supported_codec_strings).await?;
        info!("🤝 Negotiated codecs: {:?}", negotiated_codecs);
        
        // Generate SDP answer with proper media coordination
        let answer_sdp = self.create_sdp_answer_with_media_coordination(local_port, &negotiated_codecs).await?;
        
        // Start media for this session by associating it with the session_id
        self.media_manager.start_media(session_id, &media_session_id).await?;
        
        info!(
            session_id = %session_id,
            "✅ SDP answer created with media-core integration"
        );
        
        Ok(answer_sdp)
    }

    /// Create SDP answer with media coordination
    async fn create_sdp_answer_with_media_coordination(&self, local_port: u16, codecs: &[String]) -> Result<String> {
        let session_id = chrono::Utc::now().timestamp();
        let session_version = 1;
        
        let sdp_answer = format!(
            "v=0\r\n\
             o=server {} {} IN IP4 127.0.0.1\r\n\
             s=Media Session\r\n\
             c=IN IP4 127.0.0.1\r\n\
             t=0 0\r\n\
             m=audio {} RTP/AVP {}\r\n\
             a=rtpmap:0 PCMU/8000\r\n\
             a=rtpmap:8 PCMA/8000\r\n\
             a=sendrecv\r\n",
            session_id,
            session_version,
            local_port,
            codecs.join(" ")
        );
        
        Ok(sdp_answer)
    }

    /// Negotiate codecs between SDP offer and supported codecs
    async fn negotiate_codecs(&self, offer_sdp: &str, supported_codecs: &[String]) -> Result<Vec<String>> {
        // Extract codecs from SDP offer
        let mut offered_codecs = Vec::new();
        
        // Look for "m=audio" line to get format list
        if let Some(audio_line) = offer_sdp.lines().find(|line| line.starts_with("m=audio")) {
            let parts: Vec<&str> = audio_line.split_whitespace().collect();
            if parts.len() > 3 {
                // Skip "m=audio", port, "RTP/AVP", then collect format numbers
                for format in &parts[3..] {
                    offered_codecs.push(format.to_string());
                }
            }
        }
        
        info!("📋 Offered codecs: {:?}", offered_codecs);
        info!("🎼 Supported codecs: {:?}", supported_codecs);
        
        // Find intersection (common codecs)
        let mut negotiated = Vec::new();
        for offered in &offered_codecs {
            if supported_codecs.contains(offered) {
                negotiated.push(offered.clone());
            }
        }
        
        // If no common codecs, fall back to PCMU (0)
        if negotiated.is_empty() {
            negotiated.push("0".to_string()); // PCMU
        }
        
        info!("🤝 Negotiated codecs: {:?}", negotiated);
        Ok(negotiated)
    }

    /// Parse SDP offer to extract connection information
    fn parse_offer_sdp(&self, offer_sdp: &str) -> Result<(u16, crate::media::AudioCodecType)> {
        debug!("📋 Parsing SDP offer for connection info");
        
        // Extract port from "m=audio PORT RTP/AVP ..." line
        let port = offer_sdp
            .lines()
            .find(|line| line.starts_with("m=audio"))
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|port_str| port_str.parse::<u16>().ok())
            .unwrap_or(6000);
        
        // Extract first codec from the format list
        let codec = offer_sdp
            .lines()
            .find(|line| line.starts_with("m=audio"))
            .and_then(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 3 {
                    // Get first format number after "RTP/AVP"
                    parts.get(3).and_then(|codec_str| codec_str.parse::<u8>().ok())
                } else {
                    None
                }
            })
            .unwrap_or(0); // Default to PCMU (0)
        
        let audio_codec = match codec {
            0 => crate::media::AudioCodecType::PCMU,
            8 => crate::media::AudioCodecType::PCMA,
            _ => crate::media::AudioCodecType::PCMU, // Default fallback
        };
        
        debug!("🔌 Parsed SDP: port={}, codec={:?}", port, audio_codec);
        Ok((port, audio_codec))
    }
} 