//! Session Coordination Module
//!
//! Coordinates between dialog and media subsystems for session management.
//! Handles session lifecycle events and ensures proper synchronization.

use std::sync::Arc;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::api::types::{SessionId, CallState};
use crate::manager::events::SessionEvent;
use crate::media::coordinator::SessionMediaCoordinator;
use crate::dialog::coordinator::SessionDialogCoordinator;
use crate::errors::SessionError;
use crate::api::handlers::CallHandler;
use super::registry::SessionRegistry;

/// Session coordinator that bridges dialog and media subsystems
pub struct SessionCoordinator {
    /// Media coordinator for handling media sessions
    media_coordinator: Arc<SessionMediaCoordinator>,
    
    /// Dialog coordinator for handling SIP signaling
    dialog_coordinator: Arc<SessionDialogCoordinator>,
    
    /// Channel for receiving session events
    session_events_rx: Option<mpsc::Receiver<SessionEvent>>,
    
    /// Channel for sending session events (for internal coordination)
    session_events_tx: mpsc::Sender<SessionEvent>,
    
    /// Handler for call events
    handler: Option<Arc<dyn CallHandler>>,
    
    /// Session registry for looking up sessions
    registry: Arc<SessionRegistry>,
}

impl SessionCoordinator {
    /// Create a new session coordinator
    pub fn new(
        media_coordinator: Arc<SessionMediaCoordinator>,
        dialog_coordinator: Arc<SessionDialogCoordinator>,
        session_events_rx: mpsc::Receiver<SessionEvent>,
        session_events_tx: mpsc::Sender<SessionEvent>,
        handler: Option<Arc<dyn CallHandler>>,
        registry: Arc<SessionRegistry>,
    ) -> Self {
        Self {
            media_coordinator,
            dialog_coordinator,
            session_events_rx: Some(session_events_rx),
            session_events_tx,
            handler,
            registry,
        }
    }
    
    /// Start the coordination event loop
    pub async fn start_coordination_loop(self: Arc<Self>) -> Result<(), SessionError> {
        // Clone the Arc for the async block
        let coordinator = self.clone();
        
        // Take the receiver - this is why we need ownership
        let mut session_events_rx = {
            // We need to get mutable access to take the receiver
            // This is safe because this method is only called once during initialization
            let self_ptr = Arc::as_ptr(&self) as *mut Self;
            unsafe {
                (*self_ptr).session_events_rx.take()
                    .ok_or_else(|| SessionError::internal("Session events receiver already taken"))?
            }
        };
        
        tracing::info!("Starting session coordination loop");
        
        while let Some(event) = session_events_rx.recv().await {
            if let Err(e) = coordinator.handle_session_event(event).await {
                tracing::error!("Error handling session event: {}", e);
            }
        }
        
        tracing::info!("Session coordination loop ended");
        Ok(())
    }
    
    /// Handle a session event and coordinate between dialog and media
    async fn handle_session_event(&self, event: SessionEvent) -> Result<(), SessionError> {
        tracing::debug!("Coordinating session event: {:?}", event);
        
        match event {
            SessionEvent::SessionCreated { session_id, from, to, call_state } => {
                self.handle_session_created(session_id, from, to, call_state).await?;
            }
            
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                self.handle_state_changed(session_id, old_state, new_state).await?;
            }
            
            SessionEvent::SessionTerminated { session_id, reason } => {
                self.handle_session_terminated(session_id, reason).await?;
            }
            
            SessionEvent::DtmfReceived { session_id, digits } => {
                self.handle_dtmf_received(session_id, digits).await?;
            }
            
            SessionEvent::SessionHeld { session_id } => {
                self.handle_session_held(session_id).await?;
            }
            
            SessionEvent::SessionResumed { session_id } => {
                self.handle_session_resumed(session_id).await?;
            }
            
            SessionEvent::MediaUpdate { session_id, offered_sdp } => {
                self.handle_media_update(session_id, offered_sdp).await?;
            }
            
            SessionEvent::MediaEvent { session_id, event } => {
                self.handle_media_event(session_id, event).await?;
            }
            
            SessionEvent::SdpEvent { session_id, event_type, sdp } => {
                self.handle_sdp_event(session_id, event_type, sdp).await?;
            }
            
            SessionEvent::Error { session_id, error } => {
                self.handle_error_event(session_id, error).await?;
            }
            
            // NEW: RTP Processing Events (Phase 16.2)
            SessionEvent::RtpPacketProcessed { session_id, processing_type, performance_metrics } => {
                self.handle_rtp_packet_processed(session_id, processing_type, performance_metrics).await?;
            }
            
            SessionEvent::RtpProcessingModeChanged { session_id, old_mode, new_mode } => {
                self.handle_rtp_processing_mode_changed(session_id, old_mode, new_mode).await?;
            }
            
            SessionEvent::RtpProcessingError { session_id, error, fallback_applied } => {
                self.handle_rtp_processing_error(session_id, error, fallback_applied).await?;
            }
            
            SessionEvent::RtpBufferPoolUpdate { stats } => {
                self.handle_rtp_buffer_pool_update(stats).await?;
            }
            
            SessionEvent::RegistrationRequest { .. } => {
                // Registration requests are handled by the main coordinator
                // and forwarded to the application
                tracing::debug!("Registration request received - handled by main coordinator");
            }
        }
        
        Ok(())
    }
    
    /// Handle session created event
    async fn handle_session_created(
        &self,
        session_id: SessionId,
        _from: String,
        _to: String,
        call_state: CallState,
    ) -> Result<(), SessionError> {
        tracing::info!("Coordinating session creation for {}: state={:?}", session_id, call_state);
        
        // For incoming calls in Ringing state, we don't start media yet
        // Media will be started when the call transitions to Active
        match call_state {
            CallState::Ringing => {
                tracing::debug!("Session {} is ringing, deferring media setup", session_id);
            }
            
            CallState::Initiating => {
                tracing::debug!("Session {} is initiating, deferring media setup", session_id);
            }
            
            CallState::Active => {
                // This shouldn't happen normally, but handle it
                tracing::warn!("Session {} created directly in Active state, starting media", session_id);
                self.start_media_session(&session_id).await?;
            }
            
            _ => {
                tracing::debug!("Session {} created in state {:?}, no media action needed", session_id, call_state);
            }
        }
        
        Ok(())
    }
    
    /// Handle session state change event
    async fn handle_state_changed(
        &self,
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
    ) -> Result<(), SessionError> {
        tracing::info!("Coordinating state change for {}: {:?} -> {:?}", session_id, old_state, new_state);
        
        match (old_state.clone(), new_state.clone()) {
            // Call becomes active - start media
            (CallState::Ringing, CallState::Active) |
            (CallState::Initiating, CallState::Active) => {
                tracing::info!("Call {} became active, starting media session", session_id);
                self.start_media_session(&session_id).await?;
            }
            
            // Call goes on hold - pause media
            (CallState::Active, CallState::OnHold) => {
                tracing::info!("Call {} went on hold, pausing media", session_id);
                self.media_coordinator.on_session_hold(&session_id).await
                    .map_err(|e| SessionError::internal(&format!("Failed to hold media session: {}", e)))?;
            }
            
            // Call resumes from hold - resume media
            (CallState::OnHold, CallState::Active) => {
                tracing::info!("Call {} resumed from hold, resuming media", session_id);
                self.media_coordinator.on_session_resume(&session_id).await
                    .map_err(|e| SessionError::internal(&format!("Failed to resume media session: {}", e)))?;
            }
            
            // Call failed or ended - stop media
            (_, CallState::Failed(_)) |
            (_, CallState::Terminated) => {
                tracing::info!("Call {} ended/failed, stopping media session", session_id);
                self.stop_media_session(&session_id).await?;
            }
            
            _ => {
                tracing::debug!("State change {:?} -> {:?} for {} requires no media coordination", old_state, new_state, session_id);
            }
        }
        
        Ok(())
    }
    
    /// Handle session terminated event
    async fn handle_session_terminated(
        &self,
        session_id: SessionId,
        reason: String,
    ) -> Result<(), SessionError> {
        tracing::info!("Coordinating session termination for {}: {}", session_id, reason);
        
        // Ensure media session is stopped
        self.stop_media_session(&session_id).await?;
        
        // Notify the handler about call termination
        if let Some(handler) = &self.handler {
            // Get the session from registry to pass to handler
            if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                tracing::info!("Notifying handler about call termination for session {}", session_id);
                handler.on_call_ended(session, &reason).await;
            } else {
                tracing::warn!("Session {} not found in registry when notifying handler about termination", session_id);
            }
        }
        
        Ok(())
    }
    
    /// Handle DTMF received event
    async fn handle_dtmf_received(
        &self,
        session_id: SessionId,
        digits: String,
    ) -> Result<(), SessionError> {
        tracing::info!("Coordinating DTMF for {}: {}", session_id, digits);
        
        // Forward DTMF to media coordinator for processing
        // This could trigger media events like call transfer, conference join, etc.
        tracing::debug!("DTMF '{}' received for session {} - forwarding to media coordinator", digits, session_id);
        
        Ok(())
    }
    
    /// Handle session held event
    async fn handle_session_held(&self, session_id: SessionId) -> Result<(), SessionError> {
        tracing::info!("Coordinating session hold for {}", session_id);
        
        // Media hold is already handled in state_changed, but we can add additional logic here
        tracing::debug!("Session {} hold coordination complete", session_id);
        
        Ok(())
    }
    
    /// Handle session resumed event
    async fn handle_session_resumed(&self, session_id: SessionId) -> Result<(), SessionError> {
        tracing::info!("Coordinating session resume for {}", session_id);
        
        // Media resume is already handled in state_changed, but we can add additional logic here
        tracing::debug!("Session {} resume coordination complete", session_id);
        
        Ok(())
    }
    
    /// Handle media update event (e.g., re-INVITE)
    async fn handle_media_update(
        &self,
        session_id: SessionId,
        offered_sdp: Option<String>,
    ) -> Result<(), SessionError> {
        tracing::info!("Coordinating media update for {}", session_id);
        
        if let Some(sdp) = offered_sdp {
            tracing::debug!("Processing SDP offer for session {}: {} bytes", session_id, sdp.len());
            
            // Process the SDP offer through media coordinator
            self.media_coordinator.process_sdp_answer(&session_id, &sdp).await
                .map_err(|e| SessionError::internal(&format!("Failed to process SDP: {}", e)))?;
            
            // Generate SDP answer
            let answer_sdp = self.media_coordinator.generate_sdp_offer(&session_id).await
                .map_err(|e| SessionError::internal(&format!("Failed to generate SDP answer: {}", e)))?;
            
            tracing::debug!("Generated SDP answer for session {}: {} bytes", session_id, answer_sdp.len());
            
            // TODO: Send the SDP answer back through dialog coordinator
            // This would require extending the dialog coordinator API
        } else {
            tracing::debug!("Media update for session {} has no SDP offer", session_id);
        }
        
        Ok(())
    }
    
    /// Handle media event
    async fn handle_media_event(
        &self,
        session_id: SessionId,
        event: String,
    ) -> Result<(), SessionError> {
        tracing::debug!("Coordinating media event for {}: {}", session_id, event);
        
        // Forward media events to appropriate handlers
        // This could include quality updates, codec changes, etc.
        
        Ok(())
    }
    
    /// Handle SDP event (offer, answer, update)
    async fn handle_sdp_event(
        &self,
        session_id: SessionId,
        event_type: String,
        sdp: String,
    ) -> Result<(), SessionError> {
        tracing::info!("Coordinating SDP event for {}: {} ({} bytes)", session_id, event_type, sdp.len());
        
        match event_type.as_str() {
            "local_sdp_offer" => {
                tracing::debug!("Processing local SDP offer for session {}", session_id);
                // Local SDP offer is handled by the SIP layer, coordinator just logs it
            }
            
            "remote_sdp_answer" => {
                tracing::debug!("Processing remote SDP answer for session {}", session_id);
                // Apply remote SDP to media session
                self.media_coordinator.process_sdp_answer(&session_id, &sdp).await
                    .map_err(|e| SessionError::internal(&format!("Failed to process remote SDP answer: {}", e)))?;
            }
            
            "sdp_update" => {
                tracing::debug!("Processing SDP update for session {}", session_id);
                // Handle SDP update (e.g., from re-INVITE)
                self.media_coordinator.process_sdp_answer(&session_id, &sdp).await
                    .map_err(|e| SessionError::internal(&format!("Failed to process SDP update: {}", e)))?;
            }
            
            "final_negotiated_sdp" => {
                tracing::info!("✅ RFC 3261: Processing final negotiated SDP for session {} after ACK exchange", session_id);
                // Apply final negotiated SDP to media session - this is the RFC 3261 compliant 
                // point where we have the complete SDP negotiation after ACK exchange
                self.media_coordinator.process_sdp_answer(&session_id, &sdp).await
                    .map_err(|e| SessionError::internal(&format!("Failed to process final negotiated SDP: {}", e)))?;
                
                tracing::debug!("Final negotiated SDP applied to media session {}", session_id);
            }
            
            _ => {
                tracing::warn!("Unknown SDP event type '{}' for session {}", event_type, session_id);
            }
        }
        
        Ok(())
    }
    
    /// Handle error event
    async fn handle_error_event(
        &self,
        session_id: Option<SessionId>,
        error: String,
    ) -> Result<(), SessionError> {
        if let Some(session_id) = session_id {
            tracing::error!("Coordinating error for session {}: {}", session_id, error);
            
            // On error, ensure media session is cleaned up
            self.stop_media_session(&session_id).await?;
        } else {
            tracing::error!("Global coordination error: {}", error);
        }
        
        Ok(())
    }
    
    /// Start a media session for the given session ID
    async fn start_media_session(&self, session_id: &SessionId) -> Result<(), SessionError> {
        tracing::debug!("Starting media session for {}", session_id);
        
        self.media_coordinator.on_session_created(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to start media session: {}", e)))?;
        
        tracing::info!("Media session started for {}", session_id);
        Ok(())
    }
    
    /// Stop a media session for the given session ID
    async fn stop_media_session(&self, session_id: &SessionId) -> Result<(), SessionError> {
        tracing::debug!("Stopping media session for {}", session_id);
        
        self.media_coordinator.on_session_terminated(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to stop media session: {}", e)))?;
        
        tracing::info!("Media session stopped for {}", session_id);
        Ok(())
    }
    
    /// Generate SDP offer for a session
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String, SessionError> {
        self.media_coordinator.generate_sdp_offer(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to generate SDP offer: {}", e)))
    }
    
    /// Process SDP answer for a session
    pub async fn process_sdp_answer(&self, session_id: &SessionId, sdp: &str) -> Result<(), SessionError> {
        self.media_coordinator.process_sdp_answer(session_id, sdp).await
            .map_err(|e| SessionError::internal(&format!("Failed to process SDP answer: {}", e)))
    }
    
    // ========== NEW: RTP Processing Event Handlers (Phase 16.2) ==========
    
    /// Handle RTP packet processed event
    async fn handle_rtp_packet_processed(
        &self,
        session_id: SessionId,
        processing_type: crate::media::types::RtpProcessingType,
        performance_metrics: crate::media::types::RtpProcessingMetrics,
    ) -> Result<(), SessionError> {
        tracing::debug!(
            "RTP packet processed for session {}: {:?} - zero_copy: {}, traditional: {}, fallbacks: {}",
            session_id,
            processing_type,
            performance_metrics.zero_copy_packets_processed,
            performance_metrics.traditional_packets_processed,
            performance_metrics.fallback_events
        );
        
        // Log performance improvements
        if performance_metrics.allocation_reduction_percentage > 0.0 {
            tracing::debug!(
                "RTP processing efficiency for session {}: {}% allocation reduction",
                session_id,
                performance_metrics.allocation_reduction_percentage
            );
        }
        
        // Could trigger media coordination actions based on performance metrics
        // For example, adjust media session parameters based on processing efficiency
        
        Ok(())
    }
    
    /// Handle RTP processing mode changed event
    async fn handle_rtp_processing_mode_changed(
        &self,
        session_id: SessionId,
        old_mode: crate::media::types::RtpProcessingMode,
        new_mode: crate::media::types::RtpProcessingMode,
    ) -> Result<(), SessionError> {
        tracing::info!(
            "RTP processing mode changed for session {}: {:?} → {:?}",
            session_id, old_mode, new_mode
        );
        
        // Notify media coordinator about processing mode change
        // This could affect how media sessions are managed
        match new_mode {
            crate::media::types::RtpProcessingMode::ZeroCopyPreferred => {
                tracing::debug!("Session {} now using zero-copy RTP processing", session_id);
            }
            crate::media::types::RtpProcessingMode::TraditionalOnly => {
                tracing::debug!("Session {} using traditional RTP processing", session_id);
            }
            crate::media::types::RtpProcessingMode::Adaptive => {
                tracing::debug!("Session {} using adaptive RTP processing", session_id);
            }
        }
        
        Ok(())
    }
    
    /// Handle RTP processing error event
    async fn handle_rtp_processing_error(
        &self,
        session_id: SessionId,
        error: String,
        fallback_applied: bool,
    ) -> Result<(), SessionError> {
        if fallback_applied {
            tracing::warn!(
                "RTP processing error for session {} (fallback applied): {}",
                session_id, error
            );
            
            // Fallback was applied, continue with degraded performance
            // Could notify monitoring systems about the degradation
        } else {
            tracing::error!(
                "Critical RTP processing error for session {} (no fallback): {}",
                session_id, error
            );
            
            // No fallback available - this is a serious issue
            // Consider terminating the media session to prevent further issues
            tracing::warn!("Considering media session termination for session {} due to critical RTP error", session_id);
            
            // TODO: Implement policy for handling critical RTP processing failures
            // For now, we'll just log the error and continue
        }
        
        Ok(())
    }
    
    /// Handle RTP buffer pool update event
    async fn handle_rtp_buffer_pool_update(
        &self,
        stats: crate::media::types::RtpBufferPoolStats,
    ) -> Result<(), SessionError> {
        tracing::debug!(
            "RTP buffer pool update: {}/{} buffers in use ({}% efficiency)",
            stats.in_use_buffers,
            stats.total_buffers,
            stats.efficiency_percentage
        );
        
        // Monitor buffer pool health and efficiency
        if stats.efficiency_percentage < 50.0 {
            tracing::warn!(
                "Low RTP buffer pool efficiency: {}% - consider tuning pool size",
                stats.efficiency_percentage
            );
        }
        
        if stats.available_buffers == 0 {
            tracing::warn!("RTP buffer pool exhausted - all {} buffers in use", stats.total_buffers);
            // Could trigger automatic pool expansion or session throttling
        }
        
        Ok(())
    }
}

impl std::fmt::Debug for SessionCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCoordinator")
            .field("has_media_coordinator", &true)
            .field("has_dialog_coordinator", &true)
            .field("has_events_rx", &self.session_events_rx.is_some())
            .finish()
    }
} 