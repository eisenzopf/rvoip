//! Top-level Session Coordinator
//!
//! This is the main orchestrator for the entire session-core system.
//! It coordinates between dialog, media, and other subsystems.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use std::time::Instant;
use crate::api::{
    types::{CallSession, SessionId, SessionStats, MediaInfo, CallState},
    handlers::CallHandler,
    builder::SessionManagerConfig,
    bridge::{BridgeId, BridgeInfo, BridgeEvent, BridgeEventType},
};
use crate::errors::{Result, SessionError};
use crate::manager::{
    registry::SessionRegistry,
    events::{SessionEventProcessor, SessionEvent},
    cleanup::CleanupManager,
};
use crate::dialog::{DialogManager, SessionDialogCoordinator, DialogBuilder};
use crate::media::{MediaManager, SessionMediaCoordinator};
use crate::conference::{ConferenceManager, ConferenceId, ConferenceConfig, ConferenceApi};
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
    pub conference_manager: Arc<ConferenceManager>,
    
    // Subsystem coordinators
    pub dialog_coordinator: Arc<SessionDialogCoordinator>,
    pub media_coordinator: Arc<SessionMediaCoordinator>,
    
    // User handler
    pub handler: Option<Arc<dyn CallHandler>>,
    
    // Configuration
    pub config: SessionManagerConfig,
    
    // Event channels
    pub event_tx: mpsc::Sender<SessionEvent>,
    
    // Bridge event subscribers
    pub bridge_event_subscribers: Arc<RwLock<Vec<mpsc::UnboundedSender<BridgeEvent>>>>,
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

        // Create conference manager
        let conference_manager = Arc::new(ConferenceManager::new());

        let coordinator = Arc::new(Self {
            registry,
            event_processor,
            cleanup_manager,
            dialog_manager,
            media_manager,
            conference_manager,
            dialog_coordinator,
            media_coordinator,
            handler,
            config,
            event_tx: event_tx.clone(),
            bridge_event_subscribers: Arc::new(RwLock::new(Vec::new())),
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

        // Publish event to subscribers BEFORE internal processing
        // This ensures subscribers see events in real-time, even if processing takes time
        if let Err(e) = self.event_processor.publish_event(event.clone()).await {
            tracing::error!("Failed to publish event to subscribers: {}", e);
            // Continue processing even if publishing fails
        }

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
            
            SessionEvent::RegistrationRequest { transaction_id, from_uri, contact_uri, expires } => {
                self.handle_registration_request(transaction_id, from_uri, contact_uri, expires).await?;
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
                        
                        tracing::info!("Media info available: {}", media_info.is_some());
                        if let Some(ref info) = media_info {
                            tracing::info!("Local SDP length: {:?}", info.local_sdp.as_ref().map(|s| s.len()));
                            tracing::info!("Remote SDP length: {:?}", info.remote_sdp.as_ref().map(|s| s.len()));
                        }
                        
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

        // Update session state to Terminated before notifying handler
        let mut session_for_handler = None;
        if let Ok(Some(mut session)) = self.registry.get_session(&session_id).await {
            let old_state = session.state.clone();
            session.state = CallState::Terminated;
            
            // Store the session for handler notification
            session_for_handler = Some(session.clone());
            
            // Update the session in registry
            if let Err(e) = self.registry.register_session(session_id.clone(), session).await {
                tracing::error!("Failed to update session to Terminated state: {}", e);
            } else {
                // Emit state change event
                let _ = self.event_tx.send(SessionEvent::StateChanged {
                    session_id: session_id.clone(),
                    old_state,
                    new_state: CallState::Terminated,
                }).await;
            }
        }

        // Notify handler
        if let Some(handler) = &self.handler {
            println!("🔔 COORDINATOR: Handler exists, checking for session {}", session_id);
            if let Some(session) = session_for_handler {
                println!("✅ COORDINATOR: Found session {}, calling handler.on_call_ended", session_id);
                tracing::info!("Notifying handler about session {} termination", session_id);
                handler.on_call_ended(session, &reason).await;
            } else {
                println!("❌ COORDINATOR: Session {} not found in registry", session_id);
            }
        } else {
            println!("⚠️ COORDINATOR: No handler configured");
        }

        // Don't unregister immediately - let cleanup handle it later
        // This allows tests and other components to verify the Terminated state
        // self.registry.unregister_session(&session_id).await?;

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

    /// Handle registration request
    async fn handle_registration_request(
        &self,
        transaction_id: String,
        from_uri: String,
        contact_uri: String,
        expires: u32,
    ) -> Result<()> {
        tracing::info!("REGISTER request forwarded to application: {} -> {} (expires: {})", from_uri, contact_uri, expires);
        
        // Forward to application handler if available
        // In a complete implementation, the CallCenterEngine would subscribe to these events
        // and process them with its SipRegistrar
        
        // For now, we just log it - the application should subscribe to SessionEvent::RegistrationRequest
        // and handle it appropriately by sending a response back through dialog-core
        
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

    /// Send DTMF tones on an active session
    pub async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()> {
        // Verify session exists and is active
        if let Some(session) = self.find_session(session_id).await? {
            match session.state {
                CallState::Active => {
                    // Send DTMF through the dialog manager
                    self.dialog_manager
                        .send_dtmf(session_id, digits)
                        .await
                        .map_err(|e| SessionError::internal(&format!("Failed to send DTMF: {}", e)))?;
                    
                    tracing::info!("Sent DTMF '{}' for session {}", digits, session_id);
                    Ok(())
                }
                _ => {
                    Err(SessionError::invalid_state(&format!("Session {} is not active, current state: {:?}", session_id, session.state)))
                }
            }
        } else {
            Err(SessionError::session_not_found(&session_id.0))
        }
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

    // ===== Bridge Management Methods =====
    
    /// Create a bridge between two sessions
    pub async fn bridge_sessions(
        &self,
        session1: &SessionId,
        session2: &SessionId,
    ) -> Result<BridgeId> {
        // Use conference module to create a 2-party conference
        let bridge_id = BridgeId::new();
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        
        // Create conference config for bridge (2-party conference)
        let config = ConferenceConfig {
            name: bridge_id.0.clone(),
            max_participants: 2,
            audio_mixing_enabled: true,
            audio_sample_rate: 8000,  // Standard telephony rate
            audio_channels: 1,        // Mono
            rtp_port_range: Some((10000, 20000)),
            timeout: None,            // No timeout for bridges
        };
        
        // Create the conference
        self.conference_manager.create_named_conference(conf_id.clone(), config).await
            .map_err(|e| SessionError::internal(&format!("Failed to create bridge: {}", e)))?;
        
        // Join both sessions to the conference
        self.conference_manager.join_conference(&conf_id, session1).await
            .map_err(|e| SessionError::internal(&format!("Failed to add session1 to bridge: {}", e)))?;
            
        self.conference_manager.join_conference(&conf_id, session2).await
            .map_err(|e| SessionError::internal(&format!("Failed to add session2 to bridge: {}", e)))?;
        
        // Emit bridge created event
        // Note: BridgeEvent enum doesn't have a Created variant, emit participant added events instead
        self.emit_bridge_event(BridgeEvent::ParticipantAdded {
            bridge_id: bridge_id.clone(),
            session_id: session1.clone(),
        }).await;
        
        self.emit_bridge_event(BridgeEvent::ParticipantAdded {
            bridge_id: bridge_id.clone(),
            session_id: session2.clone(),
        }).await;
        
        Ok(bridge_id)
    }
    
    /// Destroy a bridge
    pub async fn destroy_bridge(&self, bridge_id: &BridgeId) -> Result<()> {
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        self.conference_manager.terminate_conference(&conf_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to destroy bridge: {}", e)))?;
        
        self.emit_bridge_event(BridgeEvent::BridgeDestroyed {
            bridge_id: bridge_id.clone(),
        }).await;
        
        Ok(())
    }
    
    /// Get the bridge a session is part of
    pub async fn get_session_bridge(&self, session_id: &SessionId) -> Result<Option<BridgeId>> {
        // Iterate through all conferences to find which one contains this session
        let conference_ids = self.conference_manager.list_conferences().await
            .map_err(|e| SessionError::internal(&format!("Failed to list conferences: {}", e)))?;
            
        for conf_id in conference_ids {
            let participants = self.conference_manager.list_participants(&conf_id).await
                .map_err(|e| SessionError::internal(&format!("Failed to list participants: {}", e)))?;
                
            if participants.iter().any(|p| &p.session_id == session_id) {
                return Ok(Some(BridgeId(conf_id.0)));
            }
        }
        
        Ok(None)
    }
    
    /// Remove a session from a bridge
    pub async fn remove_session_from_bridge(
        &self,
        bridge_id: &BridgeId,
        session_id: &SessionId,
    ) -> Result<()> {
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        self.conference_manager.leave_conference(&conf_id, session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to remove session from bridge: {}", e)))?;
        
        self.emit_bridge_event(BridgeEvent::ParticipantRemoved {
            bridge_id: bridge_id.clone(),
            session_id: session_id.clone(),
            reason: "Manually removed from bridge".to_string(),
        }).await;
        
        Ok(())
    }
    
    /// List all active bridges
    pub async fn list_bridges(&self) -> Vec<BridgeInfo> {
        match self.conference_manager.list_conferences().await {
            Ok(conference_ids) => {
                let mut bridges = Vec::new();
                
                for conf_id in conference_ids {
                    // Get participants for each conference
                    if let Ok(participants) = self.conference_manager.list_participants(&conf_id).await {
                        // Only include conferences that act as bridges (2-party conferences)
                        if participants.len() <= 2 {
                            let session_ids: Vec<SessionId> = participants.iter()
                                .map(|p| p.session_id.clone())
                                .collect();
                                
                            bridges.push(BridgeInfo {
                                id: BridgeId(conf_id.0),
                                sessions: session_ids,
                                created_at: Instant::now(), // Conference doesn't track creation time yet
                                participant_count: participants.len(),
                            });
                        }
                    }
                }
                
                bridges
            }
            Err(e) => {
                tracing::error!("Failed to list conferences: {}", e);
                Vec::new()
            }
        }
    }
    
    /// Subscribe to bridge events
    pub async fn subscribe_to_bridge_events(&self) -> mpsc::UnboundedReceiver<BridgeEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.bridge_event_subscribers.write().await.push(tx);
        rx
    }
    
    /// Create a pre-allocated outgoing session (for agent registration)
    pub async fn create_outgoing_session(&self) -> Result<SessionId> {
        let session_id = SessionId::new();
        
        // Pre-register session in registry without creating dialog yet
        let session = CallSession {
            id: session_id.clone(),
            from: String::new(), // Will be set when actually used
            to: String::new(),
            state: CallState::Initiating,
            started_at: None,
        };
        
        self.registry.register_session(session_id.clone(), session).await?;
        
        Ok(session_id)
    }
    
    /// Emit a bridge event to all subscribers
    async fn emit_bridge_event(&self, event: BridgeEvent) {
        let subscribers = self.bridge_event_subscribers.read().await;
        for subscriber in subscribers.iter() {
            // Ignore send errors (subscriber may have dropped)
            let _ = subscriber.send(event.clone());
        }
    }
    
    // ===== Additional Bridge Management Methods for Call-Engine Compatibility =====
    
    /// Create a bridge (conference) with no initial sessions
    pub async fn create_bridge(&self) -> Result<BridgeId> {
        let bridge_id = BridgeId::new();
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        
        // Create conference config for bridge
        let config = ConferenceConfig {
            name: bridge_id.0.clone(),
            max_participants: 10, // Allow more than 2 for conferences
            audio_mixing_enabled: true,
            audio_sample_rate: 8000,
            audio_channels: 1,
            rtp_port_range: Some((10000, 20000)),
            timeout: None,
        };
        
        // Create the conference
        self.conference_manager.create_named_conference(conf_id.clone(), config).await
            .map_err(|e| SessionError::internal(&format!("Failed to create bridge: {}", e)))?;
        
        // No emit for bridge creation without participants
        // The BridgeEvent enum only has participant-related events and destruction
        
        Ok(bridge_id)
    }
    
    /// Add a session to an existing bridge
    pub async fn add_session_to_bridge(
        &self,
        bridge_id: &BridgeId,
        session_id: &SessionId,
    ) -> Result<()> {
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        
        self.conference_manager.join_conference(&conf_id, session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to add session to bridge: {}", e)))?;
        
        self.emit_bridge_event(BridgeEvent::ParticipantAdded {
            bridge_id: bridge_id.clone(),
            session_id: session_id.clone(),
        }).await;
        
        Ok(())
    }
    
    /// Get information about a bridge
    pub async fn get_bridge_info(&self, bridge_id: &BridgeId) -> Result<Option<BridgeInfo>> {
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        
        // Check if conference exists
        if !self.conference_manager.conference_exists(&conf_id).await {
            return Ok(None);
        }
        
        // Get participants
        let participants = self.conference_manager.list_participants(&conf_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to list participants: {}", e)))?;
            
        let session_ids: Vec<SessionId> = participants.iter()
            .map(|p| p.session_id.clone())
            .collect();
            
        Ok(Some(BridgeInfo {
            id: bridge_id.clone(),
            sessions: session_ids,
            created_at: Instant::now(), // Conference doesn't track creation time yet
            participant_count: participants.len(),
        }))
    }
    

    

}

impl std::fmt::Debug for SessionCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCoordinator")
            .field("config", &self.config)
            .field("has_handler", &self.handler.is_some())
            .finish()
    }
} 