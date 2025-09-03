//! Unified Session API - Clean Implementation
//!
//! This is the simplified API that works through the state table.
//! No business logic here - just event sending and state queries.

use crate::state_table::types::{Role, EventType, CallState, SessionId};
use crate::session_store::SessionStore;
use crate::state_machine::{StateMachine as StateMachineExecutor, ProcessEventResult};
use crate::adapters::{EventRouter, DialogAdapter, MediaAdapter};
use crate::errors::{Result, SessionError};
use std::sync::Arc;
use std::net::{IpAddr, SocketAddr};
use tokio::sync::{mpsc, RwLock};
use std::collections::HashMap;

/// Unified session that works for any role (UAC, UAS, B2BUA, etc.)
pub struct UnifiedSession {
    /// Unique session identifier
    pub id: SessionId,
    
    /// Reference to the coordinator
    coordinator: Arc<UnifiedCoordinator>,
    
    /// Role of this session
    role: Role,
}

impl UnifiedSession {
    /// Create a new session with the specified role
    pub async fn new(coordinator: Arc<UnifiedCoordinator>, role: Role) -> Result<Self> {
        let id = SessionId::new();
        
        // Create initial session state in the store
        coordinator.store.create_session(id.clone(), role, false).await
            .map_err(|e| SessionError::InternalError(format!("Failed to create session: {}", e)))?;
        
        Ok(Self {
            id: id.clone(),
            coordinator,
            role,
        })
    }
    
    // ===== Core Operations =====
    
    /// Make an outbound call (UAC role required)
    pub async fn make_call(&self, target: &str) -> Result<()> {
        // Pass target directly through the event
        self.send_event(EventType::MakeCall { 
            target: target.to_string() 
        }).await
    }
    
    /// Handle incoming call (UAS role required)
    pub async fn on_incoming_call(&self, from: &str, sdp: Option<String>) -> Result<()> {
        // Store SDP if provided
        if let Some(sdp_data) = &sdp {
            let mut session = self.coordinator.store.get_session(&self.id)
                .await
                .map_err(|e| SessionError::SessionNotFound(format!("Session {} not found: {}", self.id.0, e)))?;
            session.remote_sdp = Some(sdp_data.clone());
            self.coordinator.store.update_session(session)
                .await
                .map_err(|e| SessionError::InternalError(format!("Failed to update session: {}", e)))?;
        }
        
        self.send_event(EventType::IncomingCall { 
            from: from.to_string(),
            sdp,
        }).await
    }
    
    /// Accept the call
    pub async fn accept(&self) -> Result<()> {
        self.send_event(EventType::AcceptCall).await
    }
    
    /// Reject the call
    pub async fn reject(&self, reason: &str) -> Result<()> {
        self.send_event(EventType::RejectCall { 
            reason: reason.to_string() 
        }).await
    }
    
    /// Hangup the call
    pub async fn hangup(&self) -> Result<()> {
        self.send_event(EventType::HangupCall).await
    }
    
    /// Put call on hold
    pub async fn hold(&self) -> Result<()> {
        self.send_event(EventType::HoldCall).await
    }
    
    /// Resume from hold
    pub async fn resume(&self) -> Result<()> {
        self.send_event(EventType::ResumeCall).await
    }
    
    /// Transfer call
    pub async fn transfer(&self, target: &str, attended: bool) -> Result<()> {
        if attended {
            self.send_event(EventType::AttendedTransfer { 
                target: target.to_string() 
            }).await
        } else {
            self.send_event(EventType::BlindTransfer { 
                target: target.to_string() 
            }).await
        }
    }
    
    /// Play audio file
    pub async fn play_audio(&self, file: &str) -> Result<()> {
        self.send_event(EventType::PlayAudio { 
            file: file.to_string() 
        }).await
    }
    
    /// Start recording
    pub async fn start_recording(&self) -> Result<()> {
        self.send_event(EventType::StartRecording).await
    }
    
    /// Stop recording
    pub async fn stop_recording(&self) -> Result<()> {
        self.send_event(EventType::StopRecording).await
    }
    
    /// Send DTMF digits
    pub async fn send_dtmf(&self, digits: &str) -> Result<()> {
        self.send_event(EventType::SendDTMF { 
            digits: digits.to_string() 
        }).await
    }
    
    /// Get current state
    pub async fn state(&self) -> Result<CallState> {
        self.coordinator.get_session_state(&self.id).await
    }
    
    /// Get session role
    pub fn role(&self) -> Role {
        self.role
    }
    
    /// Subscribe to events for this session
    pub async fn on_event<F>(&self, callback: F) -> Result<()> 
    where
        F: Fn(SessionEvent) + Send + Sync + 'static
    {
        self.coordinator.subscribe_to_session(self.id.clone(), callback).await
    }
    
    // ===== Internal =====
    
    /// Send an event to the state machine
    async fn send_event(&self, event: EventType) -> Result<()> {
        self.coordinator.process_event(&self.id, event).await
    }
}

/// Session event for callbacks
#[derive(Debug, Clone)]
pub enum SessionEvent {
    StateChanged { from: CallState, to: CallState },
    CallEstablished,
    CallTerminated { reason: String },
    MediaFlowEstablished { local_addr: String, remote_addr: String },
    MediaQualityAlert { level: String, metrics: String },
    DtmfReceived { digit: char },
    RecordingStarted,
    RecordingStopped,
    TransferInitiated { target: String },
    TransferCompleted,
    HoldStarted,
    HoldReleased,
    BridgeCreated { other_session: SessionId },
    BridgeDestroyed,
}

/// The main coordinator - replaces the old SessionCoordinator
pub struct UnifiedCoordinator {
    /// Session state storage
    pub(crate) store: Arc<SessionStore>,
    
    /// State machine executor
    state_machine: Arc<StateMachineExecutor>,
    
    /// Event router (handles adapters)
    event_router: Arc<EventRouter>,
    
    /// Event subscribers
    subscribers: Arc<RwLock<HashMap<SessionId, Vec<Arc<dyn Fn(SessionEvent) + Send + Sync>>>>>,
    
    /// Configuration
    config: Config,
}

impl UnifiedCoordinator {
    /// Create a new coordinator with the given configuration
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        // Create session store
        let store = Arc::new(SessionStore::new());
        
        // Create event channel for state machine
        let (state_event_tx, mut state_event_rx) = mpsc::channel(1000);
        
        // Create dialog adapter
        let dialog_api = Self::create_dialog_api(&config).await?;
        let (event_tx, event_rx) = mpsc::channel(1000);
        let dialog_adapter = Arc::new(DialogAdapter::new(
            dialog_api,
            event_tx.clone(),
            store.clone(),
        ));
        
        // Create media adapter
        let media_controller = Self::create_media_controller(&config).await?;
        let media_adapter = Arc::new(MediaAdapter::new(
            media_controller,
            event_tx.clone(),
            store.clone(),
            config.local_ip,
            config.media_port_start,
            config.media_port_end,
        ));
        
        // Create state machine executor with all required dependencies
        let state_machine = Arc::new(StateMachineExecutor::new(
            store.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
            state_event_tx,
        ));
        
        // Create event router
        let event_router = Arc::new(EventRouter::new(
            state_machine.clone(),
            store.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
        ));
        
        let coordinator = Arc::new(Self {
            store,
            state_machine,
            event_router,
            subscribers: Arc::new(RwLock::new(HashMap::new())),
            config,
        });
        
        // Start the event router
        // Note: In real implementation, would need to properly wire this up
        
        Ok(coordinator)
    }
    
    /// Process an event for a session
    pub async fn process_event(&self, session_id: &SessionId, event: EventType) -> Result<()> {
        // Process through state machine
        let result = self.state_machine.process_event(session_id, event.clone()).await?;
        
        // Execute actions through event router
        for action in &result.actions_executed {
            self.event_router.execute_action(session_id, action).await?;
        }
        
        // Publish state change events to subscribers
        if let Some(transition) = result.transition {
            if let Some(new_state) = transition.next_state {
                self.publish_event(session_id, SessionEvent::StateChanged {
                    from: result.old_state,
                    to: new_state,
                }).await;
                
                // Special events for specific states
                match new_state {
                    CallState::Active => {
                        self.publish_event(session_id, SessionEvent::CallEstablished).await;
                    }
                    CallState::Terminated => {
                        self.publish_event(session_id, SessionEvent::CallTerminated {
                            reason: "Normal".to_string(),
                        }).await;
                    }
                    CallState::OnHold => {
                        self.publish_event(session_id, SessionEvent::HoldStarted).await;
                    }
                    CallState::Bridged => {
                        // Extract other session from event
                        if let EventType::BridgeSessions { other_session } = event {
                            self.publish_event(session_id, SessionEvent::BridgeCreated {
                                other_session,
                            }).await;
                        }
                    }
                    _ => {}
                }
            }
        }
        
        Ok(())
    }
    
    /// Get the current state of a session
    pub async fn get_session_state(&self, session_id: &SessionId) -> Result<CallState> {
        let session = self.store.get_session(session_id)
            .await
            .map_err(|e| SessionError::SessionNotFound(format!("Session {} not found: {}", session_id.0, e)))?;
        Ok(session.call_state)
    }
    
    /// Bridge two sessions together
    pub async fn bridge_sessions(&self, session1: &SessionId, session2: &SessionId) -> Result<()> {
        // Send bridge event to both sessions
        self.process_event(session1, EventType::BridgeSessions {
            other_session: session2.clone(),
        }).await?;
        
        self.process_event(session2, EventType::BridgeSessions {
            other_session: session1.clone(),
        }).await?;
        
        Ok(())
    }
    
    /// Subscribe to events for a specific session
    pub async fn subscribe_to_session<F>(&self, session_id: SessionId, callback: F) -> Result<()>
    where
        F: Fn(SessionEvent) + Send + Sync + 'static
    {
        let mut subscribers = self.subscribers.write().await;
        subscribers.entry(session_id)
            .or_insert_with(Vec::new)
            .push(Arc::new(callback));
        Ok(())
    }
    
    /// Publish an event to subscribers
    async fn publish_event(&self, session_id: &SessionId, event: SessionEvent) {
        let subscribers = self.subscribers.read().await;
        if let Some(callbacks) = subscribers.get(session_id) {
            for callback in callbacks {
                callback(event.clone());
            }
        }
    }
    
    /// Create dialog API (stub - would connect to real dialog-core)
    async fn create_dialog_api(config: &Config) -> Result<Arc<rvoip_dialog_core::api::unified::UnifiedDialogApi>> {
        use rvoip_dialog_core::transaction::transport::{TransportManager, TransportManagerConfig};
        use rvoip_dialog_core::transaction::TransactionManager;
        use rvoip_dialog_core::config::DialogManagerConfig;
        
        // Create transport layer
        let transport_config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec![config.bind_addr],
            ..Default::default()
        };
        
        let (mut transport_manager, transport_rx) = TransportManager::new(transport_config)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to create transport manager: {}", e)))?;
        
        // Initialize the transport manager
        transport_manager.initialize()
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to initialize transport: {}", e)))?;
        
        // Create transaction manager
        let (transaction_manager, _global_rx) = TransactionManager::with_transport_manager(
            transport_manager,
            transport_rx,
            Some(100)
        ).await
        .map_err(|e| SessionError::DialogError(format!("Failed to create transaction manager: {}", e)))?;
        
        // Create dialog manager config
        let dialog_config = DialogManagerConfig::server(config.bind_addr)
            .with_domain("session-core.local")
            .build();
        
        // Create the dialog API
        let dialog_api = rvoip_dialog_core::api::unified::UnifiedDialogApi::new(
            Arc::new(transaction_manager),
            dialog_config,
        ).await
        .map_err(|e| SessionError::DialogError(format!("Failed to create dialog API: {}", e)))?;
        
        Ok(Arc::new(dialog_api))
    }
    
    /// Create media controller (stub - would connect to real media-core)
    async fn create_media_controller(config: &Config) -> Result<Arc<rvoip_media_core::MediaSessionController>> {
        // Create the media controller with the given configuration
        let media_controller = rvoip_media_core::MediaSessionController::new();
        
        Ok(Arc::new(media_controller))
    }
}

/// Configuration for the coordinator
#[derive(Debug, Clone)]
pub struct Config {
    pub sip_port: u16,
    pub media_port_start: u16,
    pub media_port_end: u16,
    pub local_ip: IpAddr,
    pub bind_addr: SocketAddr,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sip_port: 5060,
            media_port_start: 10000,
            media_port_end: 20000,
            local_ip: "127.0.0.1".parse().unwrap(),
            bind_addr: "127.0.0.1:5060".parse().unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_unified_session_creation() {
        // Would need mock adapters for proper testing
        // For now, just ensure the types compile correctly
    }
}