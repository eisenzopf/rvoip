//! Core SessionManager Implementation
//!
//! Contains the main SessionManager struct with core coordination logic.

use std::sync::Arc;
use crate::api::{
    types::{CallSession, SessionId, SessionStats, MediaInfo},
    handlers::CallHandler,
    builder::SessionManagerConfig,
};
use crate::errors::Result;
use super::{registry::SessionRegistry, events::EventProcessor, cleanup::CleanupManager};

/// Main SessionManager that coordinates all session operations
#[derive(Debug)]
pub struct SessionManager {
    config: SessionManagerConfig,
    registry: Arc<SessionRegistry>,
    event_processor: Arc<EventProcessor>,
    cleanup_manager: Arc<CleanupManager>,
    handler: Option<Arc<dyn CallHandler>>,
}

impl SessionManager {
    /// Create a new SessionManager with the given configuration
    pub async fn new(
        config: SessionManagerConfig,
        handler: Option<Arc<dyn CallHandler>>,
    ) -> Result<Arc<Self>> {
        let registry = Arc::new(SessionRegistry::new());
        let event_processor = Arc::new(EventProcessor::new());
        let cleanup_manager = Arc::new(CleanupManager::new());

        let manager = Arc::new(Self {
            config,
            registry,
            event_processor,
            cleanup_manager,
            handler,
        });

        // Initialize subsystems
        manager.initialize().await?;

        Ok(manager)
    }

    /// Initialize the session manager and all subsystems
    async fn initialize(&self) -> Result<()> {
        // TODO: Initialize SIP transport, media subsystem, etc.
        tracing::info!("SessionManager initialized on port {}", self.config.sip_port);
        Ok(())
    }

    /// Start the session manager
    pub async fn start(&self) -> Result<()> {
        self.event_processor.start().await?;
        self.cleanup_manager.start().await?;
        tracing::info!("SessionManager started");
        Ok(())
    }

    /// Stop the session manager
    pub async fn stop(&self) -> Result<()> {
        self.cleanup_manager.stop().await?;
        self.event_processor.stop().await?;
        tracing::info!("SessionManager stopped");
        Ok(())
    }

    /// Create an outgoing call session
    pub async fn create_outgoing_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<CallSession> {
        let session_id = SessionId::new();
        
        // TODO: Create actual SIP INVITE and dialog
        let call = CallSession {
            id: session_id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            state: crate::api::types::CallState::Initiating,
            started_at: Some(std::time::Instant::now()),
            manager: Arc::new(self.clone()),
        };

        // Register the session
        self.registry.register_session(session_id, call.clone()).await?;

        tracing::info!("Created outgoing call: {} -> {}", from, to);
        Ok(call)
    }

    /// Accept an incoming call
    pub async fn accept_incoming_call(&self, session_id: &SessionId) -> Result<CallSession> {
        let call = self.registry.get_session(session_id).await?
            .ok_or_else(|| crate::errors::SessionError::session_not_found(&session_id.0))?;
        
        // TODO: Send 200 OK response
        tracing::info!("Accepted incoming call: {}", session_id);
        Ok(call)
    }

    /// Hold a session
    pub async fn hold_session(&self, session_id: &SessionId) -> Result<()> {
        // TODO: Send re-INVITE with hold SDP
        tracing::info!("Holding session: {}", session_id);
        Ok(())
    }

    /// Resume a session from hold
    pub async fn resume_session(&self, session_id: &SessionId) -> Result<()> {
        // TODO: Send re-INVITE with active SDP
        tracing::info!("Resuming session: {}", session_id);
        Ok(())
    }

    /// Transfer a session to another destination
    pub async fn transfer_session(&self, session_id: &SessionId, target: &str) -> Result<()> {
        // TODO: Send REFER request
        tracing::info!("Transferring session {} to {}", session_id, target);
        Ok(())
    }

    /// Terminate a session
    pub async fn terminate_session(&self, session_id: &SessionId) -> Result<()> {
        // TODO: Send BYE request
        self.registry.unregister_session(session_id).await?;
        tracing::info!("Terminated session: {}", session_id);
        Ok(())
    }

    /// Send DTMF tones
    pub async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()> {
        // TODO: Send INFO or RFC 2833 DTMF
        tracing::info!("Sending DTMF {} to session {}", digits, session_id);
        Ok(())
    }

    /// Mute/unmute a session
    pub async fn mute_session(&self, session_id: &SessionId, muted: bool) -> Result<()> {
        // TODO: Update media stream
        tracing::info!("Muting session {}: {}", session_id, muted);
        Ok(())
    }

    /// Get media information for a session
    pub async fn get_media_info(&self, session_id: &SessionId) -> Result<MediaInfo> {
        // TODO: Get actual media info from media subsystem
        Ok(MediaInfo {
            local_sdp: None,
            remote_sdp: None,
            local_rtp_port: None,
            remote_rtp_port: None,
            codec: None,
        })
    }

    /// Update media for a session
    pub async fn update_media(&self, session_id: &SessionId, sdp: &str) -> Result<()> {
        // TODO: Send re-INVITE with new SDP
        tracing::info!("Updating media for session {}", session_id);
        Ok(())
    }

    /// Get statistics about active sessions
    pub async fn get_stats(&self) -> Result<SessionStats> {
        self.registry.get_stats().await
    }

    /// List all active sessions
    pub async fn list_active_sessions(&self) -> Result<Vec<SessionId>> {
        self.registry.list_active_sessions().await
    }

    /// Find a session by ID
    pub async fn find_session(&self, session_id: &SessionId) -> Result<Option<CallSession>> {
        self.registry.get_session(session_id).await
    }

    /// Get the call handler
    pub fn get_handler(&self) -> Option<&Arc<dyn CallHandler>> {
        self.handler.as_ref()
    }
}

impl Clone for SessionManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            registry: Arc::clone(&self.registry),
            event_processor: Arc::clone(&self.event_processor),
            cleanup_manager: Arc::clone(&self.cleanup_manager),
            handler: self.handler.clone(),
        }
    }
} 