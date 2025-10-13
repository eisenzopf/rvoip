//! Simplified Unified Session API
//!
//! This is a thin wrapper over the state machine helpers.
//! All business logic is in the state table.

use crate::state_table::types::{EventType, SessionId};
use crate::types::CallState;
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
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;

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
    /// Priority: 1) This config path, 2) RVOIP_STATE_TABLE env var, 3) Embedded default
    pub state_table_path: Option<String>,
    /// Local SIP URI (e.g., "sip:alice@127.0.0.1:5060")
    pub local_uri: String,
}

impl Default for Config {
    fn default() -> Self {
        let ip = "127.0.0.1".parse::<IpAddr>().unwrap();
        let port = 5060;
        Self {
            local_ip: ip,
            sip_port: port,
            media_port_start: 16000,
            media_port_end: 17000,
            bind_addr: SocketAddr::new(ip, port),
            state_table_path: None,
            local_uri: format!("sip:user@{}:{}", ip, port),
        }
    }
}

/// Simplified coordinator that uses state machine helpers
#[allow(dead_code)]
pub struct UnifiedCoordinator {
    /// State machine helpers
    helpers: Arc<StateMachineHelpers>,

    /// Media adapter for audio operations
    media_adapter: Arc<MediaAdapter>,

    /// Dialog adapter for SIP operations
    dialog_adapter: Arc<DialogAdapter>,

    /// Transfer coordinator for blind/attended/managed transfers
    transfer_coordinator: Arc<crate::transfer::TransferCoordinator>,

    /// Incoming call receiver
    incoming_rx: Arc<RwLock<mpsc::Receiver<IncomingCallInfo>>>,

    /// Configuration
    config: Config,
}

impl UnifiedCoordinator {
    /// Create a new coordinator
    pub async fn new(config: Config) -> Result<Arc<Self>> {
        // Get the global event coordinator singleton
        let global_coordinator = rvoip_infra_common::events::global_coordinator()
            .await
            .clone();
        
        // Create core components
        let store = Arc::new(SessionStore::new());
        let registry = Arc::new(SessionRegistry::new());
        
        // Create adapters
        let dialog_api = Self::create_dialog_api(&config, global_coordinator.clone()).await?;
        let dialog_adapter = Arc::new(DialogAdapter::new(
            dialog_api,
            store.clone(),
            global_coordinator.clone(),
        ));
        
        let media_controller = Self::create_media_controller(&config, global_coordinator.clone()).await?;
        let media_adapter = Arc::new(MediaAdapter::new(
            media_controller,
            store.clone(),
            config.local_ip,
            config.media_port_start,
            config.media_port_end,
        ));
        
        // Load state table based on config
        let state_table = Arc::new(
            crate::state_table::load_state_table_with_config(
                config.state_table_path.as_deref()
            )
        );
        
        // Create state machine (without event channel - using GlobalEventCoordinator)
        let state_machine = Arc::new(StateMachine::new(
            state_table,
            store.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
        ));
        
        // Create helpers
        let helpers = Arc::new(StateMachineHelpers::new(state_machine.clone()));

        // Create transfer coordinator
        let transfer_coordinator = Arc::new(crate::transfer::TransferCoordinator::new(
            store.clone(),
            helpers.clone(),
            dialog_adapter.clone(),
        ));

        // Create incoming call channel
        let (incoming_tx, incoming_rx) = mpsc::channel(100);

        let coordinator = Arc::new(Self {
            helpers,
            media_adapter: media_adapter.clone(),
            dialog_adapter: dialog_adapter.clone(),
            transfer_coordinator,
            incoming_rx: Arc::new(RwLock::new(incoming_rx)),
            config,
        });
        
        // Start the dialog adapter
        dialog_adapter.start().await?;
        
        // Create and start the centralized event handler with incoming call channel
        let mut event_handler = crate::adapters::SessionCrossCrateEventHandler::with_incoming_call_channel(
            state_machine.clone(),
            global_coordinator.clone(),
            dialog_adapter.clone(),
            media_adapter.clone(),
            registry.clone(),
            incoming_tx,
        );

        // Set transfer coordinator for auto-transfer
        event_handler.set_transfer_coordinator(transfer_coordinator.clone());

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
    
    /// Blind transfer - initiates REFER to current session
    /// This will trigger TransferRequested event when REFER is received
    pub async fn blind_transfer(&self, session_id: &SessionId, target: &str) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::BlindTransfer { target: target.to_string() },
        ).await?;
        Ok(())
    }

    /// Complete a blind transfer (called when TransferRequested event is received)
    /// This is the helper method that applications can call to complete the transfer
    ///
    /// # Arguments
    /// * `transferee_session_id` - Session receiving the REFER (Alice)
    /// * `refer_to` - Target to transfer to (Charlie's URI)
    ///
    /// # Returns
    /// New session ID for the call to transfer target
    pub async fn complete_blind_transfer(
        &self,
        transferee_session_id: &SessionId,
        refer_to: &str,
    ) -> Result<SessionId> {
        use crate::transfer::TransferOptions;

        // Use transfer coordinator with blind transfer options
        let options = TransferOptions::blind();

        let result = self
            .transfer_coordinator
            .complete_transfer(transferee_session_id, refer_to, options)
            .await
            .map_err(|e| SessionError::InternalError(e))?;

        if result.success {
            Ok(result.new_session_id)
        } else {
            Err(SessionError::InternalError(result.status_message))
        }
    }

    /// Start attended transfer - puts current call on hold and creates consultation call
    /// Returns the consultation session ID
    pub async fn start_attended_transfer(&self, session_id: &SessionId, target: &str) -> Result<SessionId> {
        // First process the event to trigger hold and state change
        self.helpers.state_machine.process_event(
            session_id,
            EventType::StartAttendedTransfer { target: target.to_string() },
        ).await?;

        // Create the actual consultation call
        let consultation_id = self.helpers.create_consultation_call(session_id, target).await?;

        Ok(consultation_id)
    }
    
    /// Complete attended transfer
    pub async fn complete_attended_transfer(&self, session_id: &SessionId) -> Result<()> {
        self.helpers.state_machine.process_event(
            session_id,
            EventType::CompleteAttendedTransfer,
        ).await?;
        Ok(())
    }

    /// Cancel attended transfer and return to original call
    pub async fn cancel_attended_transfer(&self, original_session_id: &SessionId) -> Result<()> {
        // Terminate consultation and resume original by sending HangupCall in ConsultationCall state
        self.helpers.state_machine.process_event(
            original_session_id,
            EventType::HangupCall,
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

    // ===== Auto-Transfer Handling =====

    /// Enable automatic blind transfer handling - DISABLED
    /// Auto-transfer now handled in SessionEventHandler to avoid event stealing
    pub fn enable_auto_transfer(self: &Arc<Self>) {
        tracing::info!("🔄 Auto-transfer: handled by SessionEventHandler");
    }

    /// Helper to extract field values from event debug strings
    /// Follows the exact same pattern as SessionCrossCrateEventHandler::extract_field() (lines 213-221)
    fn extract_field(event_str: &str, field_prefix: &str) -> Option<String> {
        if let Some(start) = event_str.find(field_prefix) {
            let start = start + field_prefix.len();
            if let Some(end) = event_str[start..].find('"') {
                return Some(event_str[start..start+end].to_string());
            }
        }
        None
    }

    // ===== Internal Helpers =====
    
    async fn create_dialog_api(config: &Config, global_coordinator: Arc<GlobalEventCoordinator>) -> Result<Arc<rvoip_dialog_core::api::unified::UnifiedDialogApi>> {
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
        let (transaction_manager, event_rx) = TransactionManager::with_transport_manager(
            transport_manager,
            transport_event_rx,
            None, // No max transactions limit
        )
        .await
        .map_err(|e| SessionError::InternalError(format!("Failed to create transaction manager: {}", e)))?;
        
        let transaction_manager = Arc::new(transaction_manager);
        
        // Create dialog config - use hybrid mode to support both incoming and outgoing calls
        let dialog_config = DialogManagerConfig::hybrid(config.bind_addr)
            .with_from_uri(&config.local_uri)
            .build();
        
        // Create dialog API with global event coordination AND transaction events
        let dialog_api = Arc::new(
            UnifiedDialogApi::with_global_events_and_coordinator(
                transaction_manager, 
                event_rx,
                dialog_config,
                global_coordinator.clone()
            )
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to create dialog API: {}", e)))?
        );
        
        dialog_api.start().await
            .map_err(|e| SessionError::InternalError(format!("Failed to start dialog API: {}", e)))?;
        
        Ok(dialog_api)
    }
    
    
    async fn create_media_controller(
        config: &Config,
        global_coordinator: Arc<GlobalEventCoordinator>
    ) -> Result<Arc<rvoip_media_core::relay::controller::MediaSessionController>> {
        use rvoip_media_core::relay::controller::MediaSessionController;
        
        // Create media controller with port range
        let controller = Arc::new(
            MediaSessionController::with_port_range(
                config.media_port_start,
                config.media_port_end
            )
        );
        
        // Create and set up the event hub
        let event_hub = rvoip_media_core::events::MediaEventHub::new(
            global_coordinator,
            controller.clone(),
        ).await
        .map_err(|e| SessionError::InternalError(format!("Failed to create media event hub: {}", e)))?;
        
        // Set the event hub on the media controller
        controller.set_event_hub(event_hub).await;

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
