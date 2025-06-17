//! Media Control API
//!
//! High-level API for controlling media sessions, including audio transmission.

use std::sync::Arc;
use crate::api::types::{SessionId, MediaInfo};
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;

/// Extension trait for media control operations
pub trait MediaControl {
    /// Start audio transmission for a session
    /// This will begin sending generated audio (440Hz sine wave in G.711 µ-law format)
    async fn start_audio_transmission(&self, session_id: &SessionId) -> Result<()>;
    
    /// Stop audio transmission for a session
    async fn stop_audio_transmission(&self, session_id: &SessionId) -> Result<()>;
    
    /// Establish media flow by setting remote RTP address and starting audio
    /// The remote address should be in the format "ip:port" (e.g., "127.0.0.1:30000")
    async fn establish_media_flow(&self, session_id: &SessionId, remote_addr: &str) -> Result<()>;
    
    /// Update media session with SDP answer/offer
    /// This will parse the SDP to extract remote RTP address and codec information
    async fn update_media_with_sdp(&self, session_id: &SessionId, sdp: &str) -> Result<()>;
    
    /// Check if audio transmission is active for a session
    async fn is_audio_transmission_active(&self, session_id: &SessionId) -> Result<bool>;
    
    /// Get detailed media information for a session
    async fn get_media_info(&self, session_id: &SessionId) -> Result<Option<MediaInfo>>;
    
    /// Generate SDP offer for a session using dynamically allocated ports
    /// This creates a media session if needed and returns SDP with the allocated port
    async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String>;
    
    /// Get RTP/RTCP statistics for a session
    async fn get_rtp_statistics(&self, session_id: &SessionId) -> Result<Option<rvoip_rtp_core::session::RtpSessionStats>>;
    
    /// Get comprehensive media statistics including quality metrics
    async fn get_media_statistics(&self, session_id: &SessionId) -> Result<Option<rvoip_media_core::types::MediaStatistics>>;
    
    /// Start periodic statistics monitoring with the specified interval
    async fn start_statistics_monitoring(&self, session_id: &SessionId, interval: std::time::Duration) -> Result<()>;
    
    /// Create a media session without generating SDP
    /// This is useful when you need to prepare media before SDP negotiation,
    /// particularly in UAS scenarios where you receive an offer first
    async fn create_media_session(&self, session_id: &SessionId) -> Result<()>;
    
    /// Update media session with remote SDP without starting transmission
    /// This separates SDP handling from media flow establishment, allowing
    /// more control over when audio transmission begins
    async fn update_remote_sdp(&self, session_id: &SessionId, remote_sdp: &str) -> Result<()>;
    
    /// Generate SDP answer based on received offer
    /// This provides proper offer/answer negotiation for UAS scenarios
    /// without requiring direct access to internal components
    async fn generate_sdp_answer(&self, session_id: &SessionId, offer: &str) -> Result<String>;
}

impl MediaControl for Arc<SessionCoordinator> {
    async fn start_audio_transmission(&self, session_id: &SessionId) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Start audio transmission
        media_manager.start_audio_transmission(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to start audio transmission: {}", e) 
            })?;
        
        tracing::info!("Started audio transmission for session {}", session_id);
        Ok(())
    }
    
    async fn stop_audio_transmission(&self, session_id: &SessionId) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Stop audio transmission
        media_manager.stop_audio_transmission(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to stop audio transmission: {}", e) 
            })?;
        
        tracing::info!("Stopped audio transmission for session {}", session_id);
        Ok(())
    }
    
    async fn establish_media_flow(&self, session_id: &SessionId, remote_addr: &str) -> Result<()> {
        // Parse the remote address
        let socket_addr: std::net::SocketAddr = remote_addr.parse()
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Invalid remote address '{}': {}", remote_addr, e) 
            })?;
        
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get the dialog ID for this session
        let dialog_id = {
            let mapping = media_manager.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| crate::errors::SessionError::MediaIntegration { 
                    message: format!("No media session found for {}", session_id) 
                })?
        };
        
        // Call the controller's establish_media_flow which handles everything
        media_manager.controller.establish_media_flow(&dialog_id, socket_addr).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to establish media flow: {}", e) 
            })?;
        
        // Store the remote SDP info
        let minimal_sdp = format!(
            "v=0\r\nc=IN IP4 {}\r\nm=audio {} RTP/AVP 0\r\n",
            socket_addr.ip(),
            socket_addr.port()
        );
        
        {
            let mut sdp_storage = media_manager.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.1 = Some(minimal_sdp);
        }
        
        tracing::info!("Established media flow for session {} to {}", session_id, remote_addr);
        Ok(())
    }
    
    async fn update_media_with_sdp(&self, session_id: &SessionId, sdp: &str) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Update media session with SDP
        media_manager.update_media_session(session_id, sdp).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to update media with SDP: {}", e) 
            })?;
        
        // Extract remote RTP port from SDP and start audio if found
        if let Some(remote_port) = parse_rtp_port_from_sdp(sdp) {
            // Assume remote IP is same as local for now (127.0.0.1)
            let remote_addr = format!("127.0.0.1:{}", remote_port);
            
            // Start audio transmission
            media_manager.start_audio_transmission(session_id).await
                .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                    message: format!("Failed to start audio transmission: {}", e) 
                })?;
            
            tracing::info!("Updated media and started audio for session {} with remote {}", 
                         session_id, remote_addr);
        }
        
        Ok(())
    }
    
    async fn is_audio_transmission_active(&self, session_id: &SessionId) -> Result<bool> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Check if we have a media session
        if let Ok(Some(_)) = media_manager.get_media_info(session_id).await {
            // For now, assume audio is active if media session exists
            // TODO: Add proper check when available in media manager
            Ok(true)
        } else {
            Ok(false)
        }
    }
    
    async fn get_media_info(&self, session_id: &SessionId) -> Result<Option<MediaInfo>> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get media info
        if let Some(media_session_info) = media_manager.get_media_info(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })? {
            
            // Get RTP statistics
            let rtp_stats = self.get_rtp_statistics(session_id).await.ok().flatten();
            
            // Get quality metrics from media statistics
            let quality_metrics = self.get_media_statistics(session_id).await
                .ok()
                .flatten()
                .and_then(|stats| stats.quality_metrics);
            
            // Convert to API MediaInfo type
            Ok(Some(MediaInfo {
                local_sdp: media_session_info.local_sdp,
                remote_sdp: media_session_info.remote_sdp,
                local_rtp_port: media_session_info.local_rtp_port,
                remote_rtp_port: media_session_info.remote_rtp_port,
                codec: media_session_info.codec,
                rtp_stats,
                quality_metrics,
            }))
        } else {
            Ok(None)
        }
    }
    
    async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Generate SDP offer using the media manager
        // This will create a media session if needed and use the allocated port
        media_manager.generate_sdp_offer(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP offer: {}", e) 
            })
    }
    
    async fn get_rtp_statistics(&self, session_id: &SessionId) -> Result<Option<rvoip_rtp_core::session::RtpSessionStats>> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get RTP statistics
        media_manager.get_rtp_statistics(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get RTP statistics: {}", e) 
            })
    }
    
    async fn get_media_statistics(&self, session_id: &SessionId) -> Result<Option<rvoip_media_core::types::MediaStatistics>> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Get media statistics
        media_manager.get_media_statistics(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to get media statistics: {}", e) 
            })
    }
    
    async fn start_statistics_monitoring(&self, session_id: &SessionId, interval: std::time::Duration) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Start statistics monitoring
        media_manager.start_statistics_monitoring(session_id, interval).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to start statistics monitoring: {}", e) 
            })
    }
    
    async fn create_media_session(&self, session_id: &SessionId) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Create media session without generating SDP
        media_manager.create_media_session(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to create media session: {}", e) 
            })?;
        
        tracing::info!("Created media session for {}", session_id);
        Ok(())
    }
    
    async fn update_remote_sdp(&self, session_id: &SessionId, remote_sdp: &str) -> Result<()> {
        // Get the media manager through the coordinator
        let media_manager = &self.media_manager;
        
        // Update media session with remote SDP but don't start transmission
        media_manager.update_media_session(session_id, remote_sdp).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to update remote SDP: {}", e) 
            })?;
        
        // Store the remote SDP
        {
            let mut sdp_storage = media_manager.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.1 = Some(remote_sdp.to_string());
        }
        
        tracing::info!("Updated remote SDP for session {}", session_id);
        Ok(())
    }
    
    async fn generate_sdp_answer(&self, session_id: &SessionId, offer: &str) -> Result<String> {
        // First ensure we have a media session
        let media_manager = &self.media_manager;
        
        // Create media session if it doesn't exist
        if media_manager.get_media_info(session_id).await.ok().flatten().is_none() {
            self.create_media_session(session_id).await?;
        }
        
        // Update with the offer
        self.update_remote_sdp(session_id, offer).await?;
        
        // Generate answer based on our capabilities and the offer
        // For now, we'll generate a standard offer which acts as our answer
        let answer = media_manager.generate_sdp_offer(session_id).await
            .map_err(|e| crate::errors::SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP answer: {}", e) 
            })?;
        
        // Store our local SDP
        {
            let mut sdp_storage = media_manager.sdp_storage.write().await;
            let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
            entry.0 = Some(answer.clone());
        }
        
        tracing::info!("Generated SDP answer for session {}", session_id);
        Ok(answer)
    }
}

/// Helper function to parse RTP port from SDP
fn parse_rtp_port_from_sdp(sdp: &str) -> Option<u16> {
    for line in sdp.lines() {
        if line.starts_with("m=audio ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                return parts[1].parse().ok();
            }
        }
    }
    None
} 