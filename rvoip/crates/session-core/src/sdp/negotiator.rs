//! SDP Negotiator - handles offer/answer negotiation

use crate::api::types::SessionId;
use crate::errors::{Result, SessionError};
use crate::media::{MediaManager, config::MediaConfigConverter, types::MediaConfig};
use super::types::{NegotiatedMediaConfig, SdpRole};
use std::sync::Arc;
use tracing::{info, debug};

/// SDP Negotiator handles the negotiation between two TUs
pub struct SdpNegotiator {
    /// Our media preferences
    media_config: MediaConfig,
    
    /// Media manager for session management
    media_manager: Arc<MediaManager>,
}

impl SdpNegotiator {
    /// Create a new negotiator with our media preferences
    pub fn new(media_config: MediaConfig, media_manager: Arc<MediaManager>) -> Self {
        Self {
            media_config,
            media_manager,
        }
    }
    
    /// Negotiate as UAC (we sent offer, received answer)
    /// 
    /// # Arguments
    /// * `session_id` - The session being negotiated
    /// * `our_offer` - The SDP offer we sent
    /// * `their_answer` - The SDP answer we received
    /// 
    /// # Returns
    /// The negotiated configuration that both parties agreed on
    pub async fn negotiate_as_uac(
        &self,
        session_id: &SessionId,
        our_offer: &str,
        their_answer: &str,
    ) -> Result<NegotiatedMediaConfig> {
        info!("Negotiating SDP as UAC for session {}", session_id);
        debug!("Our offer: {}", our_offer);
        debug!("Their answer: {}", their_answer);
        
        // Parse their answer to get the negotiated codec and remote endpoint
        let converter = MediaConfigConverter::with_media_config(&self.media_config);
        let negotiated = converter.parse_sdp_answer(their_answer)
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to parse SDP answer: {}", e) 
            })?;
        
        // Get our local endpoint from the media session
        let media_info = self.media_manager.get_media_info(session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })?
            .ok_or_else(|| SessionError::MediaIntegration { 
                message: "No media session found".to_string() 
            })?;
        
        let local_port = media_info.local_rtp_port
            .ok_or_else(|| SessionError::MediaIntegration { 
                message: "No local RTP port allocated".to_string() 
            })?;
        
        let local_addr = format!("{}:{}", 
            self.media_manager.local_bind_addr.ip(), 
            local_port
        ).parse()
            .map_err(|_| SessionError::MediaIntegration { 
                message: "Invalid local address".to_string() 
            })?;
        
        let remote_addr = format!("{}:{}", 
            negotiated.remote_ip, 
            negotiated.remote_port
        ).parse()
            .map_err(|_| SessionError::MediaIntegration { 
                message: "Invalid remote address".to_string() 
            })?;
        
        // Apply the negotiated configuration to media-core
        self.apply_negotiated_config(session_id, &negotiated.codec.name, remote_addr).await?;
        
        Ok(NegotiatedMediaConfig {
            codec: negotiated.codec.name,
            local_addr,
            remote_addr,
            local_sdp: our_offer.to_string(),
            remote_sdp: their_answer.to_string(),
            role: SdpRole::Uac,
            ptime: self.media_config.preferred_ptime,
            dtmf_enabled: self.media_config.dtmf_support,
        })
    }
    
    /// Negotiate as UAS (we received offer, generate answer)
    /// 
    /// # Arguments
    /// * `session_id` - The session being negotiated
    /// * `their_offer` - The SDP offer we received
    /// 
    /// # Returns
    /// Tuple of (our_answer, negotiated_config)
    pub async fn negotiate_as_uas(
        &self,
        session_id: &SessionId,
        their_offer: &str,
    ) -> Result<(String, NegotiatedMediaConfig)> {
        info!("Negotiating SDP as UAS for session {}", session_id);
        debug!("Their offer: {}", their_offer);
        
        // Ensure we have a media session
        if self.media_manager.get_media_info(session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })?
            .is_none() 
        {
            // Create media session if it doesn't exist
            self.media_manager.create_media_session(session_id).await
                .map_err(|e| SessionError::MediaIntegration { 
                    message: format!("Failed to create media session: {}", e) 
                })?;
        }
        
        // Get our local endpoint
        let media_info = self.media_manager.get_media_info(session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })?
            .ok_or_else(|| SessionError::MediaIntegration { 
                message: "No media session found after creation".to_string() 
            })?;
        
        let local_port = media_info.local_rtp_port
            .ok_or_else(|| SessionError::MediaIntegration { 
                message: "No local RTP port allocated".to_string() 
            })?;
        
        // Generate answer based on offer and our preferences
        let converter = MediaConfigConverter::with_media_config(&self.media_config);
        let local_ip = self.media_manager.local_bind_addr.ip().to_string();
        let our_answer = converter.generate_sdp_answer(their_offer, &local_ip, local_port)
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP answer: {}", e) 
            })?;
        
        debug!("Our answer: {}", our_answer);
        
        // Parse the offer to get remote endpoint
        let offer_info = crate::api::parse_sdp_connection(their_offer)?;
        
        // Determine which codec was selected in our answer
        // The answer generator picks the first mutually supported codec
        let selected_codec = self.find_selected_codec(their_offer, &our_answer)?;
        
        let local_addr = format!("{}:{}", local_ip, local_port).parse()
            .map_err(|_| SessionError::MediaIntegration { 
                message: "Invalid local address".to_string() 
            })?;
        
        let remote_addr = format!("{}:{}", offer_info.ip, offer_info.port).parse()
            .map_err(|_| SessionError::MediaIntegration { 
                message: "Invalid remote address".to_string() 
            })?;
        
        // Apply the negotiated configuration to media-core
        self.apply_negotiated_config(session_id, &selected_codec, remote_addr).await?;
        
        // Store both SDPs
        {
            let mut sdp_storage = self.media_manager.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.0 = Some(our_answer.clone());  // Our answer
            entry.1 = Some(their_offer.to_string()); // Their offer
        }
        
        let negotiated = NegotiatedMediaConfig {
            codec: selected_codec,
            local_addr,
            remote_addr,
            local_sdp: our_answer.clone(),
            remote_sdp: their_offer.to_string(),
            role: SdpRole::Uas,
            ptime: self.media_config.preferred_ptime,
            dtmf_enabled: self.media_config.dtmf_support,
        };
        
        Ok((our_answer, negotiated))
    }
    
    /// Apply negotiated configuration to media-core
    async fn apply_negotiated_config(
        &self,
        session_id: &SessionId,
        codec: &str,
        remote_addr: std::net::SocketAddr,
    ) -> Result<()> {
        info!("Applying negotiated config: codec={}, remote={}", codec, remote_addr);
        
        // Update media session with the negotiated codec
        let mut media_config = crate::media::types::MediaConfig::default();
        media_config.preferred_codecs = vec![codec.to_string()];
        
        // Get the dialog ID
        let dialog_id = {
            let mapping = self.media_manager.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| SessionError::MediaIntegration { 
                    message: format!("No dialog ID found for session {}", session_id) 
                })?
        };
        
        // Update media configuration
        let media_core_config = crate::media::types::convert_to_media_core_config(
            &media_config,
            self.media_manager.local_bind_addr,
            Some(remote_addr),
        );
        
        self.media_manager.controller.update_media(dialog_id, media_core_config).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to update media: {}", e) 
            })?;
        
        Ok(())
    }
    
    /// Find which codec was selected in the answer
    fn find_selected_codec(&self, offer: &str, answer: &str) -> Result<String> {
        // Parse the m= line from the answer to get the selected payload type
        for line in answer.lines() {
            if line.starts_with("m=audio ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() > 3 {
                    // The first payload type after RTP/AVP is the selected one
                    let selected_pt = parts[3];
                    
                    // Find the codec name for this payload type
                    for line in answer.lines() {
                        if line.starts_with(&format!("a=rtpmap:{} ", selected_pt)) {
                            let codec_parts: Vec<&str> = line.split_whitespace().collect();
                            if codec_parts.len() >= 2 {
                                // Extract codec name from "PCMU/8000"
                                if let Some(codec_name) = codec_parts[1].split('/').next() {
                                    return Ok(codec_name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
        
        Err(SessionError::MediaIntegration { 
            message: "Could not determine selected codec from answer".to_string() 
        })
    }
} 