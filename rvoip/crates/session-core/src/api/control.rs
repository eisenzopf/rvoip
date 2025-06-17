//! Session Control API
//!
//! This module provides the main control interface for managing SIP sessions.

use crate::api::types::*;
use crate::api::handlers::CallHandler;
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};
use crate::manager::events::SessionEvent;
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
    
    /// Send DTMF tones on an active session
    /// 
    /// # Arguments
    /// * `session_id` - The ID of the session to send DTMF on
    /// * `digits` - The DTMF digits to send (0-9, *, #, A-D)
    /// 
    /// # Returns
    /// * `Ok(())` if the DTMF was sent successfully
    /// * `Err(SessionError)` if the session doesn't exist or is not in an active state
    async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()>;
    
    /// Wait for an outgoing call to be answered
    /// 
    /// This method blocks until the call transitions to Active state (answered)
    /// or fails/times out.
    /// 
    /// # Arguments
    /// * `session_id` - The ID of the session to wait for
    /// * `timeout` - Maximum time to wait for answer
    /// 
    /// # Returns
    /// * `Ok(())` if the call was answered
    /// * `Err(SessionError)` if the call failed, was cancelled, or timed out
    async fn wait_for_answer(&self, session_id: &SessionId, timeout: std::time::Duration) -> Result<()>;
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
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only hold if session is active
        if !matches!(session.state(), CallState::Active) {
            return Err(SessionError::invalid_state(
                &format!("Cannot hold session in state {:?}", session.state())
            ));
        }
        
        // Use dialog manager to send hold request
        self.dialog_manager.hold_session(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to hold session: {}", e)))?;
        
        // Update session state
        if let Ok(Some(mut session)) = self.registry.get_session(session_id).await {
            let old_state = session.state.clone();
            session.state = CallState::OnHold;
            self.registry.register_session(session_id.clone(), session).await?;
            
            // Emit state change event
            let _ = self.event_tx.send(SessionEvent::StateChanged {
                session_id: session_id.clone(),
                old_state,
                new_state: CallState::OnHold,
            }).await;
        }
        
        Ok(())
    }
    
    async fn resume_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only resume if session is on hold
        if !matches!(session.state(), CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot resume session in state {:?}", session.state())
            ));
        }
        
        // Use dialog manager to send resume request
        self.dialog_manager.resume_session(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to resume session: {}", e)))?;
        
        // Update session state
        if let Ok(Some(mut session)) = self.registry.get_session(session_id).await {
            let old_state = session.state.clone();
            session.state = CallState::Active;
            self.registry.register_session(session_id.clone(), session).await?;
            
            // Emit state change event
            let _ = self.event_tx.send(SessionEvent::StateChanged {
                session_id: session_id.clone(),
                old_state,
                new_state: CallState::Active,
            }).await;
        }
        
        Ok(())
    }
    
    async fn transfer_session(&self, session_id: &SessionId, target: &str) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only transfer if session is active or on hold
        if !matches!(session.state(), CallState::Active | CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot transfer session in state {:?}", session.state())
            ));
        }
        
        // Use dialog manager to send transfer request
        self.dialog_manager.transfer_session(session_id, target).await
            .map_err(|e| SessionError::internal(&format!("Failed to transfer session: {}", e)))?;
        
        // Update session state
        if let Ok(Some(mut session)) = self.registry.get_session(session_id).await {
            let old_state = session.state.clone();
            session.state = CallState::Transferring;
            self.registry.register_session(session_id.clone(), session).await?;
            
            // Emit state change event
            let _ = self.event_tx.send(SessionEvent::StateChanged {
                session_id: session_id.clone(),
                old_state,
                new_state: CallState::Transferring,
            }).await;
        }
        
        Ok(())
    }
    
    async fn update_media(&self, session_id: &SessionId, sdp: &str) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only update media if session is active or on hold
        if !matches!(session.state(), CallState::Active | CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot update media in state {:?}", session.state())
            ));
        }
        
        // Use dialog manager to send UPDATE/re-INVITE with new SDP
        self.dialog_manager.update_media(session_id, sdp).await
            .map_err(|e| SessionError::internal(&format!("Failed to update media: {}", e)))?;
        
        // Create media session if it doesn't exist
        if self.media_manager.get_media_info(session_id).await.ok().flatten().is_none() {
            self.media_manager.create_media_session(session_id).await
                .map_err(|e| SessionError::MediaIntegration { 
                    message: format!("Failed to create media session: {}", e) 
                })?;
        }
        
        // Also update media manager with new SDP
        self.media_manager.update_media_session(session_id, sdp).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to update media session: {}", e) 
            })?;
        
        // Send SDP event
        let _ = self.event_tx.send(SessionEvent::SdpEvent {
            session_id: session_id.clone(),
            event_type: "media_update".to_string(),
            sdp: sdp.to_string(),
        }).await;
        
        Ok(())
    }
    
    async fn get_media_info(&self, session_id: &SessionId) -> Result<Option<MediaInfo>> {
        // Check if session exists
        let _ = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Get media info from media manager
        let media_session_info = self.media_manager.get_media_info(session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })?;
        
        if let Some(info) = media_session_info {
            // Get stored SDP
            let (local_sdp, remote_sdp) = {
                let sdp_storage = self.media_manager.sdp_storage.read().await;
                sdp_storage.get(session_id).cloned().unwrap_or((None, None))
            };
            
            // Get RTP statistics
            let rtp_stats = self.media_manager.get_rtp_statistics(session_id).await
                .ok()
                .flatten();
            
            // Get quality metrics from media statistics
            let quality_metrics = self.media_manager.get_media_statistics(session_id).await
                .ok()
                .flatten()
                .and_then(|stats| stats.quality_metrics.clone());
            
            Ok(Some(MediaInfo {
                local_sdp,
                remote_sdp,
                local_rtp_port: info.local_rtp_port,
                remote_rtp_port: info.remote_rtp_port,
                codec: info.codec,
                rtp_stats,
                quality_metrics,
            }))
        } else {
            Ok(None)
        }
    }
    
    async fn set_audio_muted(&self, session_id: &SessionId, muted: bool) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only mute/unmute if session is active or on hold
        if !matches!(session.state(), CallState::Active | CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot change audio mute in state {:?}", session.state())
            ));
        }
        
        // For now, we'll use the media manager to stop/start audio transmission
        if muted {
            self.media_manager.stop_audio_transmission(session_id).await
                .map_err(|e| SessionError::MediaIntegration { 
                    message: format!("Failed to mute audio: {}", e) 
                })?;
        } else {
            self.media_manager.start_audio_transmission(session_id).await
                .map_err(|e| SessionError::MediaIntegration { 
                    message: format!("Failed to unmute audio: {}", e) 
                })?;
        }
        
        // Send media event
        let _ = self.event_tx.send(SessionEvent::MediaEvent {
            session_id: session_id.clone(),
            event: format!("audio_muted={}", muted),
        }).await;
        
        Ok(())
    }
    
    async fn set_video_enabled(&self, session_id: &SessionId, enabled: bool) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only enable/disable video if session is active or on hold
        if !matches!(session.state(), CallState::Active | CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot change video in state {:?}", session.state())
            ));
        }
        
        // Send media event (actual video implementation would require SDP renegotiation)
        let _ = self.event_tx.send(SessionEvent::MediaEvent {
            session_id: session_id.clone(),
            event: format!("video_enabled={}", enabled),
        }).await;
        
        tracing::info!("Video {} for session {} (requires SDP renegotiation)", 
                      if enabled { "enabled" } else { "disabled" }, session_id);
        
        Ok(())
    }
    
    async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()> {
        SessionCoordinator::send_dtmf(self, session_id, digits).await
    }
    
    async fn wait_for_answer(&self, session_id: &SessionId, timeout: std::time::Duration) -> Result<()> {
        use tokio::time::timeout as tokio_timeout;
        
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Check current state
        match session.state() {
            CallState::Active => {
                // Already answered
                return Ok(());
            }
            CallState::Failed(_) | CallState::Terminated | CallState::Cancelled => {
                // Already in a terminal state
                return Err(SessionError::invalid_state(
                    &format!("Call already ended with state: {:?}", session.state())
                ));
            }
            CallState::Initiating | CallState::Ringing => {
                // Expected states - continue waiting
            }
            _ => {
                // Unexpected state (OnHold, Transferring, etc.)
                return Err(SessionError::invalid_state(
                    &format!("Cannot wait for answer in state: {:?}", session.state())
                ));
            }
        }
        
        // Subscribe to events for this session
        let mut event_subscriber = self.event_processor.subscribe().await
            .map_err(|_| SessionError::internal("Failed to subscribe to events"))?;
        
        // Wait for state change with timeout
        let wait_future = async {
            loop {
                match event_subscriber.receive().await {
                    Ok(event) => {
                        if let SessionEvent::StateChanged { 
                            session_id: event_session_id, 
                            new_state, 
                            .. 
                        } = event {
                            if event_session_id == *session_id {
                                match new_state {
                                    CallState::Active => {
                                        // Call answered!
                                        return Ok(());
                                    }
                                    CallState::Failed(reason) => {
                                        return Err(SessionError::Other(
                                            format!("Call failed: {}", reason)
                                        ));
                                    }
                                    CallState::Terminated => {
                                        return Err(SessionError::Other(
                                            "Call was terminated".to_string()
                                        ));
                                    }
                                    CallState::Cancelled => {
                                        return Err(SessionError::Other(
                                            "Call was cancelled".to_string()
                                        ));
                                    }
                                    _ => {
                                        // Continue waiting for other states
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        return Err(SessionError::internal(&format!("Event error: {}", e)));
                    }
                }
            }
        };
        
        // Apply timeout
        match tokio_timeout(timeout, wait_future).await {
            Ok(result) => result,
            Err(_) => Err(SessionError::Timeout(
                format!("Timeout waiting for call {} to be answered", session_id)
            )),
        }
    }
} 