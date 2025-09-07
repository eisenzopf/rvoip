//! Simplified Unified Session API
//!
//! This is a thin wrapper over the state machine helpers.
//! All business logic is in the state table.

use crate::state_table::types::{EventType, CallState, SessionId};
use crate::state_machine::{StateMachine, StateMachineHelpers};
use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::errors::{Result, SessionError};
use crate::types::{SessionInfo, IncomingCallInfo};
use crate::session_store::SessionStore;
use crate::session_registry::SessionRegistry;
use rvoip_media_core::types::AudioFrame;
use std::sync::Arc;
use std::net::{IpAddr, SocketAddr};
use tokio::sync::{mpsc, RwLock};
use infra_common::events::coordinator::GlobalEventCoordinator;

/// Configuration for the unified coordinator
#[derive(Debug, Clone)]
pub struct Config {
    /// Local IP address for media
    pub local_ip: IpAddr,
    /// SIP port
    pub sip_port: u16,
    /// Starting port for media
    pub media_port_start: u16,
    /// Ending port for media
    pub media_port_end: u16,
    /// Bind address for SIP
    pub bind_addr: SocketAddr,
    /// Optional path to custom state table YAML
    pub state_table_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            local_ip: "127.0.0.1".parse().unwrap(),
            sip_port: 5060,
            media_port_start: 16000,
            media_port_end: 17000,
            bind_addr: "127.0.0.1:5060".parse().unwrap(),
            state_table_path: None,
        }
    }
}

/// Simplified coordinator that uses state machine helpers
pub struct UnifiedCoordinator {
    /// State machine helpers
    helpers: Arc<StateMachineHelpers>,
    
    /// Media adapter for audio operations
    media_adapter: Arc<MediaAdapter>,
    
    /// Dialog adapter for SIP operations
    dialog_adapter: Arc<DialogAdapter>,
    
    /// Incoming call receiver
    incoming_rx: Arc<RwLock<mpsc::Receiver<IncomingCallInfo>>>,
    
    /// Configuration
    config: Config,
}

impl UnifiedCoordinator {
    /// Create a new coordinator
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        // Create global event coordinator
        let global_coordinator = Arc::new(
            GlobalEventCoordinator::monolithic()
                .await
                .map_err(|e| SessionError::InternalError(format!("Failed to create global coordinator: {}", e)))?
        );
        
        // Create core components
        let store = Arc::new(SessionStore::new());
        let registry = Arc::new(SessionRegistry::new());
        
        // Create adapters
        let dialog_api = Self::create_dialog_api(&config).await?;
        let dialog_adapter = Arc::new(DialogAdapter::new(
            dialog_api,
            store.clone(),
        ));
        
        let media_controller = Self::create_media_controller(&config).await?;
        let media_adapter = Arc::new(MediaAdapter::new(
            media_controller,
            store.clone(),
            config.local_ip,
            config.media_port_start,
            config.media_port_end,
        ));
        
        // Create state machine
        let (event_tx, _event_rx) = mpsc::channel(1000);
        let state_machine = Arc::new(StateMachine::new_with_adapters(
            store.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
            event_tx,
        ));
        
        // Create helpers
        let helpers = Arc::new(StateMachineHelpers::new(state_machine.clone()));
        
        // Create incoming call channel
        let (_incoming_tx, incoming_rx) = mpsc::channel(100);
        
        let coordinator = Arc::new(Self {
            helpers,
            media_adapter: media_adapter.clone(),
            dialog_adapter: dialog_adapter.clone(),
            incoming_rx: Arc::new(RwLock::new(incoming_rx)),
            config,
        });
        
        // Start the dialog adapter
        dialog_adapter.start().await?;
        
        // Create and start the centralized event handler
        let event_handler = crate::adapters::SessionCrossCrateEventHandler::new(
            state_machine.clone(),
            global_coordinator.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
            registry.clone(),
        );
        
        // Start the event handler (sets up channels and subscriptions)
        event_handler.start().await?;
        
        Ok(coordinator)
    }
    
    // ===== Simple Call Operations =====
    
    /// Make an outgoing call
    pub async fn make_call(&self, from: &str, to: &str) -> Result<SessionId> {
        self.helpers.make_call(from, to).await
    }
    
    /// Accept an incoming call
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.accept_call(session_id).await
    }
    
    /// Reject an incoming call
    pub async fn reject_call(&self, session_id: &SessionId, reason: &str) -> Result<()> {
        self.helpers.reject_call(session_id, reason).await
    }
    
    /// Hangup a call
    pub async fn hangup(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.hangup(session_id).await
    }
    
    /// Put a call on hold
    pub async fn hold(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::HoldCall,
        ).await?;
        Ok(())
    }
    
    /// Resume a call from hold
    pub async fn resume(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::ResumeCall,
        ).await?;
        Ok(())
    }
    
    // ===== Conference Operations =====
    
    /// Create a conference from an active call
    pub async fn create_conference(&self, session_id: &SessionId, name: &str) -> Result<()> {
        self.helpers.create_conference(session_id, name).await
    }
    
    /// Add a participant to a conference
    pub async fn add_to_conference(
        &self,
        host_session_id: &SessionId,
        participant_session_id: &SessionId,
    ) -> Result<()> {
        self.helpers.add_to_conference(host_session_id, participant_session_id).await
    }
    
    /// Join an existing conference
    pub async fn join_conference(&self, session_id: &SessionId, conference_id: &str) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::JoinConference { conference_id: conference_id.to_string() },
        ).await?;
        Ok(())
    }
    
    // ===== Transfer Operations =====
    
    /// Blind transfer
    pub async fn blind_transfer(&self, session_id: &SessionId, target: &str) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::BlindTransfer { target: target.to_string() },
        ).await?;
        Ok(())
    }
    
    /// Start attended transfer
    pub async fn start_attended_transfer(&self, session_id: &SessionId, target: &str) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::StartAttendedTransfer { target: target.to_string() },
        ).await?;
        Ok(())
    }
    
    /// Complete attended transfer
    pub async fn complete_attended_transfer(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::CompleteAttendedTransfer,
        ).await?;
        Ok(())
    }
    
    // ===== DTMF Operations =====
    
    /// Send DTMF digit
    pub async fn send_dtmf(&self, session_id: &SessionId, digit: char) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::SendDTMF { digits: digit.to_string() },
        ).await?;
        Ok(())
    }
    
    // ===== Recording Operations =====
    
    /// Start recording a call
    pub async fn start_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::StartRecording,
        ).await?;
        Ok(())
    }
    
    /// Stop recording a call
    pub async fn stop_recording(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::StopRecording,
        ).await?;
        Ok(())
    }
    
    // ===== Query Operations =====
    
    /// Get session information
    pub async fn get_session_info(&self, session_id: &SessionId) -> Result<SessionInfo> {
        self.helpers.get_session_info(session_id).await
    }
    
    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        self.helpers.list_sessions().await
    }
    
    /// Get current state of a session
    pub async fn get_state(&self, session_id: &SessionId) -> Result<CallState> {
        self.helpers.get_state(session_id).await
    }
    
    /// Check if session is in conference
    pub async fn is_in_conference(&self, session_id: &SessionId) -> Result<bool> {
        self.helpers.is_in_conference(session_id).await
    }
    
    // ===== Audio Operations =====
    
    /// Subscribe to audio frames for a session
    pub async fn subscribe_to_audio(
        &self,
        session_id: &SessionId,
    ) -> Result<crate::types::AudioFrameSubscriber> {
        self.media_adapter.subscribe_to_audio_frames(session_id).await
    }
    
    /// Send audio frame to a session
    pub async fn send_audio(&self, session_id: &SessionId, frame: AudioFrame) -> Result<()> {
        self.media_adapter.send_audio_frame(session_id, frame).await
    }
    
    // ===== Event Subscriptions =====
    
    /// Subscribe to session events
    pub async fn subscribe<F>(&self, session_id: SessionId, callback: F)
    where
        F: Fn(crate::state_machine::helpers::SessionEvent) + Send + Sync + 'static,
    {
        self.helpers.subscribe(session_id, callback).await
    }
    
    /// Unsubscribe from session events
    pub async fn unsubscribe(&self, session_id: &SessionId) {
        self.helpers.unsubscribe(session_id).await
    }
    
    // ===== Incoming Call Handling =====
    
    /// Get the next incoming call
    pub async fn get_incoming_call(&self) -> Option<IncomingCallInfo> {
        self.incoming_rx.write().await.recv().await
    }
    
    // ===== Internal Helpers =====
    
    async fn create_dialog_api(config: &Config) -> Result<Arc<rvoip_dialog_core::api::unified::UnifiedDialogApi>> {
        use rvoip_dialog_core::config::DialogManagerConfig;
        use rvoip_dialog_core::api::unified::UnifiedDialogApi;
        use rvoip_dialog_core::transaction::{TransactionManager, transport::{TransportManager, TransportManagerConfig}};
        
        // Create transport manager first (dialog-core's own transport manager)
        let transport_config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec![config.bind_addr],
            ..Default::default()
        };
        
        let (mut transport_manager, transport_event_rx) = TransportManager::new(transport_config)
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to create transport manager: {}", e)))?;
        
        // Initialize the transport manager
        transport_manager.initialize()
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to initialize transport: {}", e)))?;
        
        // Create transaction manager using transport manager
        let (transaction_manager, _event_rx) = TransactionManager::with_transport_manager(
            transport_manager,
            transport_event_rx,
            None, // No max transactions limit
        )
        .await
        .map_err(|e| SessionError::InternalError(format!("Failed to create transaction manager: {}", e)))?;
        
        let transaction_manager = Arc::new(transaction_manager);
        
        // Create dialog config
        let dialog_config = DialogManagerConfig::client(config.bind_addr)
            .with_from_uri(&format!("sip:user@{}", config.local_ip))
            .build();
        
        // Create dialog API
        let dialog_api = Arc::new(
            UnifiedDialogApi::new(transaction_manager, dialog_config)
                .await
                .map_err(|e| SessionError::InternalError(format!("Failed to create dialog API: {}", e)))?
        );
        
        dialog_api.start().await
            .map_err(|e| SessionError::InternalError(format!("Failed to start dialog API: {}", e)))?;
        
        Ok(dialog_api)
    }
    
    
    async fn create_media_controller(config: &Config) -> Result<Arc<rvoip_media_core::relay::controller::MediaSessionController>> {
        use rvoip_media_core::relay::controller::MediaSessionController;
        
        // Create media controller with port range
        let controller = Arc::new(
            MediaSessionController::with_port_range(
                config.media_port_start,
                config.media_port_end
            )
        );
        
        Ok(controller)
    }
}

/// Simple helper to create a session and make a call
impl UnifiedCoordinator {
    /// Quick method to create a UAC session and make a call
    pub async fn quick_call(&self, from: &str, to: &str) -> Result<SessionId> {
        self.make_call(from, to).await
    }
}
