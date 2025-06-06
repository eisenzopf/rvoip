//! Core SessionManager Implementation
//!
//! Contains the main SessionManager struct with high-level session orchestration logic.
//! Coordinates dialog and media integration at comparable abstraction levels.

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::api::{
    types::{CallSession, SessionId, SessionStats, MediaInfo},
    handlers::CallHandler,
    builder::SessionManagerConfig,
};
use crate::errors::Result;
use super::{registry::SessionRegistry, events::SessionEventProcessor, cleanup::CleanupManager};

// High-level integration with dialog and media modules (parallel abstraction levels)
use crate::dialog::{DialogManager, SessionDialogCoordinator, DialogBuilder};
use crate::media::MediaManager; // TODO: Add MediaManager when implemented
use rvoip_dialog_core::events::SessionCoordinationEvent;

/// Main SessionManager that coordinates all session operations
/// Now uses high-level DialogManager and MediaManager at comparable abstraction levels
pub struct SessionManager {
    config: SessionManagerConfig,
    registry: Arc<SessionRegistry>,
    event_processor: Arc<SessionEventProcessor>,
    cleanup_manager: Arc<CleanupManager>,
    handler: Option<Arc<dyn CallHandler>>,
    
    // High-level integration managers (parallel abstraction levels)
    dialog_manager: Arc<DialogManager>,
    dialog_coordinator: Arc<SessionDialogCoordinator>,
    // media_manager: Arc<MediaManager>, // TODO: Add when MediaManager is implemented
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("config", &self.config)
            .field("registry", &self.registry)
            .field("event_processor", &self.event_processor)
            .field("cleanup_manager", &self.cleanup_manager)
            .field("handler", &self.handler.is_some())
            .field("dialog_manager", &self.dialog_manager)
            .field("dialog_coordinator", &self.dialog_coordinator)
            .finish()
    }
}

impl SessionManager {
    /// Create a new SessionManager with the given configuration  
    pub async fn new(
        config: SessionManagerConfig,
        handler: Option<Arc<dyn CallHandler>>,
    ) -> Result<Arc<Self>> {
        let registry = Arc::new(SessionRegistry::new());
        let event_processor = Arc::new(SessionEventProcessor::new());
        let cleanup_manager = Arc::new(CleanupManager::new());

        // Create dialog integration using DialogBuilder (high-level abstraction)
        let dialog_builder = DialogBuilder::new(config.clone());
        let dialog_api = dialog_builder.build().await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to create dialog API: {}", e)))?;

        // Create high-level dialog integration components
        let dialog_to_session = Arc::new(dashmap::DashMap::new());
        let dialog_manager = Arc::new(DialogManager::new(
            dialog_api.clone(),
            registry.clone(),
            dialog_to_session.clone(),
        ));

        // Create dialog coordination channel (for dialog-core to coordinator communication)
        let (dialog_coordination_tx, dialog_coordination_rx) = mpsc::channel(1000);
        
        // Create session events channel (for coordinator to event processor communication)
        let (session_events_tx, session_events_rx) = mpsc::channel(1000);
        
        let dialog_coordinator = Arc::new(SessionDialogCoordinator::new(
            dialog_api,
            registry.clone(),
            handler.clone(),
            session_events_tx,
            dialog_to_session,
        ));

        let manager = Arc::new(Self {
            config,
            registry,
            event_processor,
            cleanup_manager,
            handler,
            dialog_manager,
            dialog_coordinator,
        });

        // Initialize subsystems and coordination
        manager.initialize(dialog_coordination_tx, dialog_coordination_rx, session_events_rx).await?;

        Ok(manager)
    }

    /// Initialize the session manager and all subsystems
    async fn initialize(
        &self, 
        dialog_coordination_tx: mpsc::Sender<SessionCoordinationEvent>,
        dialog_coordination_rx: mpsc::Receiver<SessionCoordinationEvent>,
        mut session_events_rx: mpsc::Receiver<super::events::SessionEvent>
    ) -> Result<()> {
        // Initialize dialog coordination (high-level delegation)
        println!("🔗 SETUP: Initializing dialog coordination via DialogCoordinator");
        self.dialog_coordinator
            .initialize(dialog_coordination_tx)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to initialize dialog coordinator: {}", e)))?;
        
        // Start dialog event loop (delegated to coordinator)
        println!("🎬 SPAWN: Starting dialog coordination event loop");
        self.dialog_coordinator
            .start_event_loop(dialog_coordination_rx)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to start dialog event loop: {}", e)))?;

        // Bridge session events from coordinator to event processor
        println!("🌉 BRIDGE: Setting up session event bridge");
        let event_processor = self.event_processor.clone();
        tokio::spawn(async move {
            while let Some(session_event) = session_events_rx.recv().await {
                if let Err(e) = event_processor.publish_event(session_event).await {
                    tracing::error!("Failed to publish session event: {}", e);
                }
            }
        });

        tracing::info!("SessionManager initialized on port {}", self.config.sip_port);
        Ok(())
    }

    /// Start the session manager
    pub async fn start(&self) -> Result<()> {
        // Start dialog manager (high-level delegation)
        self.dialog_manager.start()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to start dialog manager: {}", e)))?;
        
        self.event_processor.start().await?;
        self.cleanup_manager.start().await?;
        tracing::info!("SessionManager started");
        Ok(())
    }

    /// Stop the session manager
    pub async fn stop(&self) -> Result<()> {
        self.cleanup_manager.stop().await?;
        self.event_processor.stop().await?;
        
        // Stop dialog manager (high-level delegation)
        self.dialog_manager.stop()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to stop dialog manager: {}", e)))?;
            
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
        
        // Create SIP INVITE and dialog using DialogManager (high-level delegation)
        let _dialog_handle = self.dialog_manager
            .create_outgoing_call(session_id.clone(), from, to, sdp)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to create call via dialog manager: {}", e)))?;
        
        let call = CallSession {
            id: session_id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            state: crate::api::types::CallState::Initiating,
            started_at: Some(std::time::Instant::now()),
        };

        // Register the session
        self.registry.register_session(session_id.clone(), call.clone()).await?;

        // Send session created event
        self.send_session_event(super::events::SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: call.from.clone(),
            to: call.to.clone(),
            call_state: call.state.clone(),
        }).await?;

        tracing::info!("Created outgoing call: {} -> {}", from, to);
        Ok(call)
    }

    /// Accept an incoming call
    pub async fn accept_incoming_call(&self, session_id: &SessionId) -> Result<CallSession> {
        let call = self.registry.get_session(session_id).await?
            .ok_or_else(|| crate::errors::SessionError::session_not_found(&session_id.0))?;
        
        // Accept incoming call using DialogManager (high-level delegation)
        self.dialog_manager
            .accept_incoming_call(session_id)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to accept call via dialog manager: {}", e)))?;
        
        tracing::info!("Accepted incoming call: {}", session_id);
        Ok(call)
    }

    /// Hold a session
    pub async fn hold_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session exists first
        if self.registry.get_session(session_id).await?.is_none() {
            return Err(crate::errors::SessionError::session_not_found(&session_id.0));
        }
        
        // Hold session using DialogManager (high-level delegation)
        self.dialog_manager
            .hold_session(session_id)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to hold session: {}", e)))?;
            
        tracing::info!("Holding session: {}", session_id);
        Ok(())
    }

    /// Resume a session from hold
    pub async fn resume_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session exists first
        if self.registry.get_session(session_id).await?.is_none() {
            return Err(crate::errors::SessionError::session_not_found(&session_id.0));
        }
        
        // Resume session using DialogManager (high-level delegation)
        self.dialog_manager
            .resume_session(session_id)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to resume session: {}", e)))?;
            
        tracing::info!("Resuming session: {}", session_id);
        Ok(())
    }

    /// Transfer a session to another destination
    pub async fn transfer_session(&self, session_id: &SessionId, target: &str) -> Result<()> {
        // Check if session exists first
        if self.registry.get_session(session_id).await?.is_none() {
            return Err(crate::errors::SessionError::session_not_found(&session_id.0));
        }
        
        // Transfer session using DialogManager (high-level delegation)
        self.dialog_manager
            .transfer_session(session_id, target)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to transfer session: {}", e)))?;
            
        tracing::info!("Transferring session {} to {}", session_id, target);
        Ok(())
    }

    /// Terminate a session
    pub async fn terminate_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session exists first
        if self.registry.get_session(session_id).await?.is_none() {
            return Err(crate::errors::SessionError::session_not_found(&session_id.0));
        }
        
        // Terminate session using DialogManager (high-level delegation)
        self.dialog_manager
            .terminate_session(session_id)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to terminate session: {}", e)))?;
            
        // Remove the session from registry
        self.registry.unregister_session(session_id).await?;
        
        tracing::info!("Terminated session: {}", session_id);
        Ok(())
    }

    /// Send DTMF tones
    pub async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()> {
        // Check if session exists first
        if self.registry.get_session(session_id).await?.is_none() {
            return Err(crate::errors::SessionError::session_not_found(&session_id.0));
        }
        
        // Send DTMF using DialogManager (high-level delegation)
        self.dialog_manager
            .send_dtmf(session_id, digits)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to send DTMF: {}", e)))?;
            
        tracing::info!("Sending DTMF {} to session {}", digits, session_id);
        Ok(())
    }
    


    /// Mute/unmute a session
    pub async fn mute_session(&self, session_id: &SessionId, muted: bool) -> Result<()> {
        // TODO: Delegate to media-core for actual media stream control
        // This would update the media streams without SIP signaling
        tracing::info!("Muting session {}: {}", session_id, muted);
        Ok(())
    }

    /// Get media information for a session
    pub async fn get_media_info(&self, session_id: &SessionId) -> Result<MediaInfo> {
        // TODO: Delegate to media-core for actual media info
        // This would get the current SDP, ports, codecs from media-core
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
        // Check if session exists first
        if self.registry.get_session(session_id).await?.is_none() {
            return Err(crate::errors::SessionError::session_not_found(&session_id.0));
        }
        
        // Update media using DialogManager (high-level delegation)
        self.dialog_manager
            .update_media(session_id, sdp)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to update media: {}", e)))?;
            
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

    /// Get the event processor (for testing)
    pub fn get_event_processor(&self) -> &Arc<SessionEventProcessor> {
        &self.event_processor
    }

    /// Get the actual bound address (for testing and discovery)
    pub fn get_bound_address(&self) -> std::net::SocketAddr {
        self.dialog_manager.get_bound_address()
    }

    /// Send a session event
    async fn send_session_event(&self, event: super::events::SessionEvent) -> Result<()> {
        self.event_processor.publish_event(event).await
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
            dialog_manager: Arc::clone(&self.dialog_manager),
            dialog_coordinator: Arc::clone(&self.dialog_coordinator),
        }
    }
} 