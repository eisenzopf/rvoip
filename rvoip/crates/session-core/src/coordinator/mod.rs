//! Top-level Session Coordinator
//!
//! This is the main orchestrator for the entire session-core system.
//! It coordinates between dialog, media, and other subsystems.

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::api::{
    types::{CallSession, SessionId, SessionStats, MediaInfo, CallState},
    handlers::CallHandler,
    builder::SessionManagerConfig,
};
use crate::errors::{Result, SessionError};
use crate::manager::{
    registry::SessionRegistry,
    events::{SessionEventProcessor, SessionEvent},
    cleanup::CleanupManager,
};
use crate::dialog::{DialogManager, SessionDialogCoordinator, DialogBuilder};
use crate::media::{MediaManager, SessionMediaCoordinator};
use rvoip_dialog_core::events::SessionCoordinationEvent;

/// The main coordinator for the entire session system
pub struct SessionCoordinator {
    // Core services
    pub registry: Arc<SessionRegistry>,
    pub event_processor: Arc<SessionEventProcessor>,
    pub cleanup_manager: Arc<CleanupManager>,
    
    // Subsystem managers
    pub dialog_manager: Arc<DialogManager>,
    pub media_manager: Arc<MediaManager>,
    
    // Subsystem coordinators
    pub dialog_coordinator: Arc<SessionDialogCoordinator>,
    pub media_coordinator: Arc<SessionMediaCoordinator>,
    
    // User handler
    pub handler: Option<Arc<dyn CallHandler>>,
    
    // Configuration
    pub config: SessionManagerConfig,
    
    // Event channels
    pub event_tx: mpsc::Sender<SessionEvent>,
}

impl SessionCoordinator {
    /// Create and initialize the entire system
    pub async fn new(
        config: SessionManagerConfig,
        handler: Option<Arc<dyn CallHandler>>,
    ) -> Result<Arc<Self>> {
        // Create core services
        let registry = Arc::new(SessionRegistry::new());
        let event_processor = Arc::new(SessionEventProcessor::new());
        let cleanup_manager = Arc::new(CleanupManager::new());

        // Create dialog subsystem
        let dialog_builder = DialogBuilder::new(config.clone());
        let dialog_api = dialog_builder.build().await
            .map_err(|e| SessionError::internal(&format!("Failed to create dialog API: {}", e)))?;

        let dialog_to_session = Arc::new(dashmap::DashMap::new());
        let dialog_manager = Arc::new(DialogManager::new(
            dialog_api.clone(),
            registry.clone(),
            dialog_to_session.clone(),
        ));

        // Create media subsystem
        let local_bind_addr = "127.0.0.1:0".parse().unwrap();
        let media_manager = Arc::new(MediaManager::with_port_range(
            local_bind_addr,
            config.media_port_start,
            config.media_port_end,
        ));

        // Create event channels
        let (event_tx, event_rx) = mpsc::channel(1000);
        let (dialog_coord_tx, dialog_coord_rx) = mpsc::channel(1000);

        // Create subsystem coordinators
        let dialog_coordinator = Arc::new(SessionDialogCoordinator::new(
            dialog_api,
            registry.clone(),
            handler.clone(),
            event_tx.clone(),
            dialog_to_session,
        ));

        let media_coordinator = Arc::new(SessionMediaCoordinator::new(
            media_manager.clone()
        ));

        let coordinator = Arc::new(Self {
            registry,
            event_processor,
            cleanup_manager,
            dialog_manager,
            media_manager,
            dialog_coordinator,
            media_coordinator,
            handler,
            config,
            event_tx: event_tx.clone(),
        });

        // Initialize subsystems
        coordinator.initialize(event_rx, dialog_coord_tx, dialog_coord_rx).await?;

        Ok(coordinator)
    }

    /// Initialize all subsystems and start event loops
    async fn initialize(
        self: &Arc<Self>,
        event_rx: mpsc::Receiver<SessionEvent>,
        dialog_coord_tx: mpsc::Sender<SessionCoordinationEvent>,
        dialog_coord_rx: mpsc::Receiver<SessionCoordinationEvent>,
    ) -> Result<()> {
        // Start event processor
        self.event_processor.start().await?;

        // Initialize dialog coordination
        self.dialog_coordinator
            .initialize(dialog_coord_tx)
            .await
            .map_err(|e| SessionError::internal(&format!("Failed to initialize dialog coordinator: {}", e)))?;

        // Start dialog event loop
        let dialog_coordinator = self.dialog_coordinator.clone();
        tokio::spawn(async move {
            if let Err(e) = dialog_coordinator.start_event_loop(dialog_coord_rx).await {
                tracing::error!("Dialog event loop error: {}", e);
            }
        });

        // Start main event loop
        let coordinator = self.clone();
        tokio::spawn(async move {
            coordinator.run_event_loop(event_rx).await;
        });

        tracing::info!("SessionCoordinator initialized on port {}", self.config.sip_port);
        Ok(())
    }

    /// Main event loop that handles all session events
    async fn run_event_loop(self: Arc<Self>, mut event_rx: mpsc::Receiver<SessionEvent>) {
        tracing::info!("Starting main coordinator event loop");

        while let Some(event) = event_rx.recv().await {
            if let Err(e) = self.handle_event(event).await {
                tracing::error!("Error handling event: {}", e);
            }
        }

        tracing::info!("Main coordinator event loop ended");
    }

    /// Handle a session event
    async fn handle_event(&self, event: SessionEvent) -> Result<()> {
        println!("🎯 COORDINATOR: Handling event: {:?}", event);
        tracing::debug!("Handling event: {:?}", event);

        match event {
            SessionEvent::SessionCreated { session_id, from, to, call_state } => {
                self.handle_session_created(session_id, from, to, call_state).await?;
            }
            
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                self.handle_state_changed(session_id, old_state, new_state).await?;
            }
            
            SessionEvent::SessionTerminated { session_id, reason } => {
                println!("🎯 COORDINATOR: Matched SessionTerminated event for {} - {}", session_id, reason);
                self.handle_session_terminated(session_id, reason).await?;
            }
            
            SessionEvent::MediaEvent { session_id, event } => {
                self.handle_media_event(session_id, event).await?;
            }
            
            SessionEvent::SdpEvent { session_id, event_type, sdp } => {
                self.handle_sdp_event(session_id, event_type, sdp).await?;
            }
            
            _ => {
                tracing::debug!("Unhandled event type");
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
    ) -> Result<()> {
        tracing::info!("Session {} created with state {:?}", session_id, call_state);

        // Media is created later when session becomes active
        match call_state {
            CallState::Ringing | CallState::Initiating => {
                tracing::debug!("Session {} in early state, deferring media setup", session_id);
            }
            CallState::Active => {
                tracing::warn!("Session {} created in Active state, starting media", session_id);
                self.start_media_session(&session_id).await?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle session state change
    async fn handle_state_changed(
        &self,
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
    ) -> Result<()> {
        tracing::info!("Session {} state changed: {:?} -> {:?}", session_id, old_state, new_state);

        match (old_state, new_state.clone()) {
            // Call becomes active
            (CallState::Ringing, CallState::Active) |
            (CallState::Initiating, CallState::Active) => {
                self.start_media_session(&session_id).await?;
                
                // Notify handler that call is established
                if let Some(handler) = &self.handler {
                    if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                        // Get SDP information if available
                        let media_info = self.media_manager.get_media_info(&session_id).await.ok().flatten();
                        let local_sdp = media_info.as_ref().and_then(|m| m.local_sdp.clone());
                        let remote_sdp = media_info.as_ref().and_then(|m| m.remote_sdp.clone());
                        
                        tracing::info!("Notifying handler about call {} establishment", session_id);
                        handler.on_call_established(session, local_sdp, remote_sdp).await;
                    }
                }
            }
            
            // Call goes on hold
            (CallState::Active, CallState::OnHold) => {
                self.media_coordinator.on_session_hold(&session_id).await
                    .map_err(|e| SessionError::internal(&format!("Failed to hold media: {}", e)))?;
            }
            
            // Call resumes
            (CallState::OnHold, CallState::Active) => {
                self.media_coordinator.on_session_resume(&session_id).await
                    .map_err(|e| SessionError::internal(&format!("Failed to resume media: {}", e)))?;
            }
            
            // Call ends
            (_, CallState::Failed(_)) |
            (_, CallState::Terminated) => {
                self.stop_media_session(&session_id).await?;
            }
            
            _ => {}
        }

        Ok(())
    }

    /// Handle session terminated event
    async fn handle_session_terminated(
        &self,
        session_id: SessionId,
        reason: String,
    ) -> Result<()> {
        println!("🔴 COORDINATOR: handle_session_terminated called for session {} with reason: {}", session_id, reason);
        tracing::info!("Session {} terminated: {}", session_id, reason);

        // Stop media
        self.stop_media_session(&session_id).await?;

        // Notify handler
        if let Some(handler) = &self.handler {
            println!("🔔 COORDINATOR: Handler exists, checking for session {}", session_id);
            if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                println!("✅ COORDINATOR: Found session {}, calling handler.on_call_ended", session_id);
                tracing::info!("Notifying handler about session {} termination", session_id);
                handler.on_call_ended(session, &reason).await;
            } else {
                println!("❌ COORDINATOR: Session {} not found in registry", session_id);
            }
        } else {
            println!("⚠️ COORDINATOR: No handler configured");
        }

        // Clean up session
        self.registry.unregister_session(&session_id).await?;

        Ok(())
    }

    /// Handle media event
    async fn handle_media_event(
        &self,
        session_id: SessionId,
        event: String,
    ) -> Result<()> {
        tracing::debug!("Media event for session {}: {}", session_id, event);

        match event.as_str() {
            "rfc_compliant_media_creation_uac" | "rfc_compliant_media_creation_uas" => {
                tracing::info!("Media creation event for {}: {}", session_id, event);
                
                // Just update session state to Active - the state change handler will create media
                if let Ok(Some(mut session)) = self.registry.get_session(&session_id).await {
                    let old_state = session.state.clone();
                    
                    // Only update if not already Active
                    if !matches!(old_state, CallState::Active) {
                        session.state = CallState::Active;
                        
                        if let Err(e) = self.registry.register_session(session_id.clone(), session).await {
                            tracing::error!("Failed to update session state: {}", e);
                        } else {
                            // Publish state change event - this will trigger media creation
                            let _ = self.event_tx.send(SessionEvent::StateChanged {
                                session_id,
                                old_state,
                                new_state: CallState::Active,
                            }).await;
                        }
                    } else {
                        tracing::debug!("Session {} already Active, skipping state update", session_id);
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle SDP event
    async fn handle_sdp_event(
        &self,
        session_id: SessionId,
        event_type: String,
        sdp: String,
    ) -> Result<()> {
        tracing::debug!("SDP event for session {}: {}", session_id, event_type);

        match event_type.as_str() {
            "remote_sdp_answer" | "final_negotiated_sdp" => {
                if let Ok(Some(_)) = self.media_manager.get_media_info(&session_id).await {
                    if let Err(e) = self.media_manager.update_media_session(&session_id, &sdp).await {
                        tracing::error!("Failed to update media session with SDP: {}", e);
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Start media session
    async fn start_media_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if media session already exists
        if let Ok(Some(_)) = self.media_manager.get_media_info(session_id).await {
            tracing::debug!("Media session already exists for {}, skipping creation", session_id);
            return Ok(());
        }
        
        self.media_coordinator.on_session_created(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to start media: {}", e)))?;
        Ok(())
    }

    /// Stop media session
    async fn stop_media_session(&self, session_id: &SessionId) -> Result<()> {
        self.media_coordinator.on_session_terminated(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to stop media: {}", e)))?;
        Ok(())
    }

    // ===== Public API Methods =====

    /// Start all subsystems
    pub async fn start(&self) -> Result<()> {
        self.dialog_manager.start().await
            .map_err(|e| SessionError::internal(&format!("Failed to start dialog manager: {}", e)))?;
        
        self.cleanup_manager.start().await?;
        
        tracing::info!("SessionCoordinator started");
        Ok(())
    }

    /// Stop all subsystems
    pub async fn stop(&self) -> Result<()> {
        self.cleanup_manager.stop().await?;
        self.event_processor.stop().await?;
        
        self.dialog_manager.stop().await
            .map_err(|e| SessionError::internal(&format!("Failed to stop dialog manager: {}", e)))?;
            
        tracing::info!("SessionCoordinator stopped");
        Ok(())
    }

    /// Create an outgoing call
    pub async fn create_outgoing_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<CallSession> {
        let session_id = SessionId::new();
        
        let call = CallSession {
            id: session_id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            state: CallState::Initiating,
            started_at: Some(std::time::Instant::now()),
        };

        // Register session
        self.registry.register_session(session_id.clone(), call.clone()).await?;

        // Send events
        if let Some(ref local_sdp) = sdp {
            self.event_tx.send(SessionEvent::SdpEvent {
                session_id: session_id.clone(),
                event_type: "local_sdp_offer".to_string(),
                sdp: local_sdp.clone(),
            }).await.map_err(|_| SessionError::internal("Failed to send SDP event"))?;
        }

        self.event_tx.send(SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: call.from.clone(),
            to: call.to.clone(),
            call_state: call.state.clone(),
        }).await.map_err(|_| SessionError::internal("Failed to send session created event"))?;
        
        // Create dialog
        self.dialog_manager
            .create_outgoing_call(session_id.clone(), from, to, sdp)
            .await
            .map_err(|e| SessionError::internal(&format!("Failed to create call: {}", e)))?;
            
        Ok(call)
    }

    /// Terminate a session
    pub async fn terminate_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session exists
        if self.registry.get_session(session_id).await?.is_none() {
            return Err(SessionError::session_not_found(&session_id.0));
        }
        
        // Terminate via dialog
        self.dialog_manager
            .terminate_session(session_id)
            .await
            .map_err(|e| SessionError::internal(&format!("Failed to terminate session: {}", e)))?;
            
        Ok(())
    }

    /// Get session statistics
    pub async fn get_stats(&self) -> Result<SessionStats> {
        self.registry.get_stats().await
    }

    /// List active sessions
    pub async fn list_active_sessions(&self) -> Result<Vec<SessionId>> {
        self.registry.list_active_sessions().await
    }

    /// Find a session by ID
    pub async fn find_session(&self, session_id: &SessionId) -> Result<Option<CallSession>> {
        self.registry.get_session(session_id).await
    }

    /// Get the bound address
    pub fn get_bound_address(&self) -> std::net::SocketAddr {
        self.dialog_manager.get_bound_address()
    }

    /// Generate SDP offer for a session
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String> {
        self.media_manager.generate_sdp_offer(session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP offer: {}", e) 
            })
    }

    // Additional API methods would be implemented here...
}

impl std::fmt::Debug for SessionCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCoordinator")
            .field("config", &self.config)
            .field("has_handler", &self.handler.is_some())
            .finish()
    }
} 