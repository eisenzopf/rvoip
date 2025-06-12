//! Session Control API
//!
//! This module provides the main control interface for managing SIP sessions.

use crate::api::types::*;
use crate::api::handlers::CallHandler;
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};
use std::sync::Arc;

/// Main session control trait
pub trait SessionControl {
    /// Prepare an outgoing call by allocating resources and generating SDP
    /// This does NOT send the INVITE yet
    async fn prepare_outgoing_call(
        &self,
        from: &str,
        to: &str,
    ) -> Result<PreparedCall>;
    
    /// Initiate a prepared call by sending the INVITE
    async fn initiate_prepared_call(
        &self,
        prepared_call: &PreparedCall,
    ) -> Result<CallSession>;
    
    /// Create an outgoing call (legacy method - prepares and initiates in one step)
    async fn create_outgoing_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<CallSession>;
    
    /// Terminate an active session
    async fn terminate_session(&self, session_id: &SessionId) -> Result<()>;
    
    /// Get session information
    async fn get_session(&self, session_id: &SessionId) -> Result<Option<CallSession>>;
    
    /// List all active sessions
    async fn list_active_sessions(&self) -> Result<Vec<SessionId>>;
    
    /// Get session statistics
    async fn get_stats(&self) -> Result<SessionStats>;
    
    /// Get the configured call handler
    fn get_handler(&self) -> Option<Arc<dyn CallHandler>>;
    
    /// Get the bound SIP address
    fn get_bound_address(&self) -> std::net::SocketAddr;
    
    /// Start the session manager
    async fn start(&self) -> Result<()>;
    
    /// Stop the session manager
    async fn stop(&self) -> Result<()>;
    
    /// Put a session on hold
    async fn hold_session(&self, session_id: &SessionId) -> Result<()>;
    
    /// Resume a held session
    async fn resume_session(&self, session_id: &SessionId) -> Result<()>;
    
    /// Transfer a session to another party
    async fn transfer_session(&self, session_id: &SessionId, target: &str) -> Result<()>;
    
    /// Update session media (e.g., for codec changes)
    async fn update_media(&self, session_id: &SessionId, sdp: &str) -> Result<()>;
    
    /// Get media information for a session
    async fn get_media_info(&self, session_id: &SessionId) -> Result<Option<MediaInfo>>;
    
    /// Mute/unmute audio
    async fn set_audio_muted(&self, session_id: &SessionId, muted: bool) -> Result<()>;
    
    /// Enable/disable video
    async fn set_video_enabled(&self, session_id: &SessionId, enabled: bool) -> Result<()>;
}

/// Implementation of SessionControl for SessionCoordinator
impl SessionControl for Arc<SessionCoordinator> {
    async fn prepare_outgoing_call(
        &self,
        from: &str,
        to: &str,
    ) -> Result<PreparedCall> {
        // Create a session ID
        let session_id = SessionId::new();
        
        // Create the call session in preparing state
        let call = CallSession {
            id: session_id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            state: CallState::Initiating,
            started_at: Some(std::time::Instant::now()),
        };
        
        // Register the session
        self.registry.register_session(session_id.clone(), call.clone()).await?;
        
        // Create media session to allocate port
        self.media_manager.create_media_session(&session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to create media session: {}", e) 
            })?;
        
        // Generate SDP offer with allocated port
        let sdp_offer = self.media_manager.generate_sdp_offer(&session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP offer: {}", e) 
            })?;
        
        // Get the allocated port from media info
        let media_info = self.media_manager.get_media_info(&session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })?;
        
        let local_rtp_port = media_info
            .and_then(|info| info.local_rtp_port)
            .unwrap_or(0);
        
        Ok(PreparedCall {
            session_id,
            from: from.to_string(),
            to: to.to_string(),
            sdp_offer,
            local_rtp_port,
        })
    }
    
    async fn initiate_prepared_call(
        &self,
        prepared_call: &PreparedCall,
    ) -> Result<CallSession> {
        // Send the INVITE with the prepared SDP
        self.dialog_manager
            .create_outgoing_call(
                prepared_call.session_id.clone(),
                &prepared_call.from,
                &prepared_call.to,
                Some(prepared_call.sdp_offer.clone()),
            )
            .await
            .map_err(|e| SessionError::internal(&format!("Failed to initiate call: {}", e)))?;
        
        // Return the session
        self.get_session(&prepared_call.session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&prepared_call.session_id.0))
    }
    
    async fn create_outgoing_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<CallSession> {
        SessionCoordinator::create_outgoing_call(self, from, to, sdp).await
    }
    
    async fn terminate_session(&self, session_id: &SessionId) -> Result<()> {
        SessionCoordinator::terminate_session(self, session_id).await
    }
    
    async fn get_session(&self, session_id: &SessionId) -> Result<Option<CallSession>> {
        SessionCoordinator::find_session(self, session_id).await
    }
    
    async fn list_active_sessions(&self) -> Result<Vec<SessionId>> {
        SessionCoordinator::list_active_sessions(self).await
    }
    
    async fn get_stats(&self) -> Result<SessionStats> {
        SessionCoordinator::get_stats(self).await
    }
    
    fn get_handler(&self) -> Option<Arc<dyn CallHandler>> {
        self.handler.clone()
    }
    
    fn get_bound_address(&self) -> std::net::SocketAddr {
        SessionCoordinator::get_bound_address(self)
    }
    
    async fn start(&self) -> Result<()> {
        SessionCoordinator::start(self).await
    }
    
    async fn stop(&self) -> Result<()> {
        SessionCoordinator::stop(self).await
    }
    
    async fn hold_session(&self, session_id: &SessionId) -> Result<()> {
        // TODO: Implement hold functionality
        tracing::warn!("Hold session not yet implemented for {}", session_id);
        Ok(())
    }
    
    async fn resume_session(&self, session_id: &SessionId) -> Result<()> {
        // TODO: Implement resume functionality
        tracing::warn!("Resume session not yet implemented for {}", session_id);
        Ok(())
    }
    
    async fn transfer_session(&self, session_id: &SessionId, target: &str) -> Result<()> {
        // TODO: Implement transfer functionality
        tracing::warn!("Transfer session not yet implemented for {} to {}", session_id, target);
        Ok(())
    }
    
    async fn update_media(&self, session_id: &SessionId, sdp: &str) -> Result<()> {
        // TODO: Implement media update
        tracing::warn!("Update media not yet implemented for {}", session_id);
        Ok(())
    }
    
    async fn get_media_info(&self, session_id: &SessionId) -> Result<Option<MediaInfo>> {
        // TODO: Implement get media info
        tracing::warn!("Get media info not yet implemented for {}", session_id);
        Ok(None)
    }
    
    async fn set_audio_muted(&self, session_id: &SessionId, muted: bool) -> Result<()> {
        // TODO: Implement audio mute
        tracing::warn!("Set audio muted not yet implemented for {}: {}", session_id, muted);
        Ok(())
    }
    
    async fn set_video_enabled(&self, session_id: &SessionId, enabled: bool) -> Result<()> {
        // TODO: Implement video enable/disable
        tracing::warn!("Set video enabled not yet implemented for {}: {}", session_id, enabled);
        Ok(())
    }
} 