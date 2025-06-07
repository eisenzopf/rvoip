//! Media Manager for Session-Core
//!
//! Main interface for media operations, using real MediaSessionController from media-core.
//! This manager coordinates between SIP sessions and media-core components.

use crate::api::types::SessionId;
use crate::errors::Result;
use super::types::*;
use super::MediaError;
use std::sync::Arc;
use std::collections::HashMap;
use std::net::SocketAddr;

/// Main media manager for session-core using real media-core components
pub struct MediaManager {
    /// Real MediaSessionController from media-core
    controller: Arc<MediaSessionController>,
    
    /// Session ID mapping (SIP SessionId -> Media DialogId)
    session_mapping: Arc<tokio::sync::RwLock<HashMap<SessionId, DialogId>>>,
    
    /// Default local bind address for media sessions
    local_bind_addr: SocketAddr,
}

impl MediaManager {
    /// Create a new MediaManager with real MediaSessionController
    pub fn new(local_bind_addr: SocketAddr) -> Self {
        Self {
            controller: Arc::new(MediaSessionController::new()),
            session_mapping: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            local_bind_addr,
        }
    }
    
    /// Create a MediaManager with custom port range
    pub fn with_port_range(local_bind_addr: SocketAddr, base_port: u16, max_port: u16) -> Self {
        Self {
            controller: Arc::new(MediaSessionController::with_port_range(base_port, max_port)),
            session_mapping: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            local_bind_addr,
        }
    }
    
    /// Get the underlying MediaSessionController
    pub fn controller(&self) -> Arc<MediaSessionController> {
        self.controller.clone()
    }
    
    /// Create a new media session for a SIP session using real MediaSessionController
    pub async fn create_media_session(&self, session_id: &SessionId) -> super::MediaResult<MediaSessionInfo> {
        tracing::debug!("Creating media session for SIP session: {}", session_id);
        
        // Create dialog ID for media session (use session ID as base)
        let dialog_id = format!("media-{}", session_id);
        
        // Create media configuration using conversion helper
        let session_config = MediaConfig::default();
        let media_config = convert_to_media_core_config(
            &session_config,
            self.local_bind_addr,
            None, // Will be set later when remote SDP is processed
        );
        
        // Start media session using real MediaSessionController
        self.controller.start_media(dialog_id.clone(), media_config).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        // Get session info from controller
        let media_session_info = self.controller.get_session_info(&dialog_id).await
            .ok_or_else(|| MediaError::SessionNotFound { session_id: dialog_id.clone() })?;
        
        // Store session mapping
        {
            let mut mapping = self.session_mapping.write().await;
            mapping.insert(session_id.clone(), dialog_id.clone());
        }
        
        // Convert to our MediaSessionInfo type
        let session_info = MediaSessionInfo::from(media_session_info);
        
        tracing::info!("✅ Created media session: {} for SIP session: {} with real MediaSessionController", 
                      dialog_id, session_id);
        
        Ok(session_info)
    }
    
    /// Update a media session with new SDP (for re-INVITE, etc.)
    pub async fn update_media_session(&self, session_id: &SessionId, sdp: &str) -> super::MediaResult<()> {
        tracing::debug!("Updating media session for SIP session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        // Parse SDP to extract remote address and codec information
        let remote_addr = self.parse_remote_address_from_sdp(sdp);
        let codec = self.parse_codec_from_sdp(sdp);
        
        if let Some(remote_addr) = remote_addr {
            // Create enhanced media configuration with remote address and codec
            let mut session_config = MediaConfig::default();
            if let Some(codec_name) = codec {
                session_config.preferred_codecs = vec![codec_name];
            }
            
            let updated_config = convert_to_media_core_config(
                &session_config,
                self.local_bind_addr,
                Some(remote_addr),
            );
            
            self.controller.update_media(dialog_id, updated_config).await
                .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
                
            tracing::info!("✅ Updated media session for SIP session: {} with remote: {} and codecs: {:?}", 
                          session_id, remote_addr, session_config.preferred_codecs);
        } else {
            tracing::warn!("Could not parse SDP for session: {}, skipping media update", session_id);
        }
        
        Ok(())
    }
    
    /// Terminate a media session
    pub async fn terminate_media_session(&self, session_id: &SessionId) -> super::MediaResult<()> {
        tracing::debug!("Terminating media session for SIP session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mut mapping = self.session_mapping.write().await;
            mapping.remove(session_id)
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        // Stop media session using real MediaSessionController
        self.controller.stop_media(dialog_id.clone()).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Terminated media session: {} for SIP session: {}", dialog_id, session_id);
        Ok(())
    }
    
    /// Get media information for a session
    pub async fn get_media_info(&self, session_id: &SessionId) -> super::MediaResult<Option<MediaSessionInfo>> {
        tracing::debug!("Getting media info for SIP session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
        };
        
        if let Some(dialog_id) = dialog_id {
            // Get session info from controller
            if let Some(media_session_info) = self.controller.get_session_info(&dialog_id).await {
                let session_info = MediaSessionInfo::from(media_session_info);
                Ok(Some(session_info))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
    
    /// Generate SDP offer for a session using real media session information
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> super::MediaResult<String> {
        tracing::debug!("Generating SDP offer for session: {}", session_id);
        
        // Find dialog ID for this session
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
        };
        
        // If we have a media session, get its info for SDP generation
        let media_info = if let Some(dialog_id) = dialog_id {
            self.controller.get_session_info(&dialog_id).await
        } else {
            None
        };
        
        // Generate SDP using MediaConfigConverter 
        use crate::media::config::MediaConfigConverter;
        let converter = MediaConfigConverter::new();
        
        let local_ip = self.local_bind_addr.ip().to_string();
        let local_port = if let Some(info) = media_info {
            info.rtp_port.unwrap_or(10000)
        } else {
            10000 // Default port if no media session exists yet
        };
        
        let sdp = converter.generate_sdp_offer(&local_ip, local_port)
            .map_err(|e| MediaError::Configuration { message: e.to_string() })?;
        
        tracing::info!("✅ Generated SDP offer for session: {} with port: {}", session_id, local_port);
        Ok(sdp)
    }
    
    /// Helper method to parse remote address from SDP (improved implementation)
    fn parse_remote_address_from_sdp(&self, sdp: &str) -> Option<SocketAddr> {
        // Enhanced SDP parsing to extract remote address and port
        let mut remote_ip = None;
        let mut remote_port = None;
        
        for line in sdp.lines() {
            if line.starts_with("c=IN IP4 ") {
                if let Some(ip_str) = line.strip_prefix("c=IN IP4 ") {
                    remote_ip = ip_str.trim().parse().ok();
                }
            } else if line.starts_with("m=audio ") {
                // Parse m=audio line: "m=audio 10001 RTP/AVP 96"
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    remote_port = parts[1].parse().ok();
                }
            }
        }
        
        if let (Some(ip), Some(port)) = (remote_ip, remote_port) {
            tracing::debug!("Parsed remote address from SDP: {}:{}", ip, port);
            Some(SocketAddr::new(ip, port))
        } else {
            tracing::warn!("Could not parse remote address from SDP - ip: {:?}, port: {:?}", remote_ip, remote_port);
            None
        }
    }
    
    /// Parse codec information from SDP
    fn parse_codec_from_sdp(&self, sdp: &str) -> Option<String> {
        for line in sdp.lines() {
            if line.starts_with("a=rtpmap:") {
                // Parse a=rtpmap:96 opus/48000/2 -> return "opus"
                if let Some(codec_part) = line.split_whitespace().nth(1) {
                    if let Some(codec_name) = codec_part.split('/').next() {
                        tracing::debug!("Parsed codec from SDP: {}", codec_name);
                        return Some(codec_name.to_string());
                    }
                }
            }
        }
        None
    }
    
    /// Process SDP answer and configure media session
    pub async fn process_sdp_answer(&self, session_id: &SessionId, sdp: &str) -> super::MediaResult<()> {
        tracing::debug!("Processing SDP answer for session: {}", session_id);
        
        // Parse remote address from SDP and update media session
        if let Some(remote_addr) = self.parse_remote_address_from_sdp(sdp) {
            self.update_media_session(session_id, sdp).await?;
            tracing::info!("✅ Processed SDP answer and updated remote address to: {}", remote_addr);
        } else {
            tracing::warn!("Could not parse remote address from SDP answer");
        }
        
        Ok(())
    }
    
    /// List all active media sessions
    pub async fn list_active_sessions(&self) -> Vec<MediaSessionInfo> {
        let mut sessions = Vec::new();
        let mapping = self.session_mapping.read().await;
        
        for dialog_id in mapping.values() {
            if let Some(media_session_info) = self.controller.get_session_info(dialog_id).await {
                sessions.push(MediaSessionInfo::from(media_session_info));
            }
        }
        
        sessions
    }
    
    /// Get the local bind address
    pub fn get_local_bind_addr(&self) -> SocketAddr {
        self.local_bind_addr
    }
    
    /// Start audio transmission for a session
    pub async fn start_audio_transmission(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.start_audio_transmission(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Started audio transmission for session: {}", session_id);
        Ok(())
    }
    
    /// Stop audio transmission for a session
    pub async fn stop_audio_transmission(&self, session_id: &SessionId) -> super::MediaResult<()> {
        let dialog_id = {
            let mapping = self.session_mapping.read().await;
            mapping.get(session_id).cloned()
                .ok_or_else(|| MediaError::SessionNotFound { session_id: session_id.to_string() })?
        };
        
        self.controller.stop_audio_transmission(&dialog_id).await
            .map_err(|e| MediaError::MediaEngine { source: Box::new(e) })?;
        
        tracing::info!("✅ Stopped audio transmission for session: {}", session_id);
        Ok(())
    }
}

impl std::fmt::Debug for MediaManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaManager")
            .field("local_bind_addr", &self.local_bind_addr)
            .field("session_mapping_count", &"<async>")
            .finish_non_exhaustive()
    }
}

/// Builder for MediaManager configuration
pub struct MediaManagerBuilder {
    local_bind_addr: Option<SocketAddr>,
    port_range: Option<(u16, u16)>,
}

impl MediaManagerBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set the local bind address for media sessions
    pub fn with_local_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.local_bind_addr = Some(addr);
        self
    }
    
    /// Set custom port range for RTP sessions
    pub fn with_port_range(mut self, base_port: u16, max_port: u16) -> Self {
        self.port_range = Some((base_port, max_port));
        self
    }
    
    /// Build the MediaManager
    pub fn build(self) -> MediaManager {
        let local_bind_addr = self.local_bind_addr
            .unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
        
        if let Some((base_port, max_port)) = self.port_range {
            MediaManager::with_port_range(local_bind_addr, base_port, max_port)
        } else {
            MediaManager::new(local_bind_addr)
        }
    }
}

impl std::fmt::Debug for MediaManagerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaManagerBuilder")
            .field("local_bind_addr", &self.local_bind_addr)
            .field("port_range", &self.port_range)
            .finish()
    }
}

impl Default for MediaManagerBuilder {
    fn default() -> Self {
        Self {
            local_bind_addr: None,
            port_range: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_media_manager_creation() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::new(local_addr);
        
        assert_eq!(manager.get_local_bind_addr(), local_addr);
    }
    
    #[tokio::test]
    async fn test_media_session_creation() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::with_port_range(local_addr, 10000, 20000);
        let session_id = SessionId::new();
        
        let result = manager.create_media_session(&session_id).await;
        assert!(result.is_ok());
        
        let media_session = result.unwrap();
        assert!(!media_session.session_id.is_empty());
        assert!(media_session.local_rtp_port.is_some());
        
        // Verify session is tracked
        let sessions = manager.list_active_sessions().await;
        assert_eq!(sessions.len(), 1);
    }
    
    #[tokio::test]
    async fn test_sdp_generation() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::with_port_range(local_addr, 10000, 20000);
        let session_id = SessionId::new();
        
        // First create a media session
        let _media_session = manager.create_media_session(&session_id).await.unwrap();
        
        // Then generate SDP
        let sdp = manager.generate_sdp_offer(&session_id).await;
        assert!(sdp.is_ok());
        
        let sdp_content = sdp.unwrap();
        assert!(sdp_content.contains("m=audio"));
        assert!(sdp_content.contains("a=rtpmap:0 PCMU/8000"));
        assert!(sdp_content.contains("a=rtpmap:8 PCMA/8000"));
        
        // Verify SDP contains allocated port
        assert!(sdp_content.contains("1000")); // Should contain port from 10000-20000 range
    }
    
    #[tokio::test]
    async fn test_media_session_termination() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let manager = MediaManager::with_port_range(local_addr, 10000, 20000);
        let session_id = SessionId::new();
        
        // Create and then terminate session
        let _media_session = manager.create_media_session(&session_id).await.unwrap();
        assert_eq!(manager.list_active_sessions().await.len(), 1);
        
        let result = manager.terminate_media_session(&session_id).await;
        assert!(result.is_ok());
        
        // Verify session is removed
        assert_eq!(manager.list_active_sessions().await.len(), 0);
    }
} 