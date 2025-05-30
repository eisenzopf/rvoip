//! Main SIP client coordination
//!
//! This module provides the main ClientManager that coordinates all client subsystems
//! (registration, calls, media) and provides a unified API for SIP client applications.

use std::sync::Arc;
use std::collections::HashMap;
use std::net::SocketAddr;
use tokio::sync::RwLock;
use uuid::Uuid;
use tracing::{info, debug, warn, error};

use rvoip_transaction_core::{TransactionManager, TransactionEvent};
use rvoip_media_core::MediaEngine;
use rvoip_sip_transport::{UdpTransport, Transport};
use rvoip_sip_core::{Request, Response, Method, StatusCode, HeaderName};
use rvoip_sip_core::types::headers::HeaderAccess;
use infra_common::EventBus;

use crate::call::{CallManager, CallId, CallInfo, CallState};
use crate::registration::{RegistrationManager, RegistrationConfig, RegistrationInfo};
use crate::events::{ClientEventHandler, ClientEvent, CallStatusInfo, RegistrationStatusInfo, MediaEventType};
use crate::error::{ClientResult, ClientError};

/// Configuration for the SIP client
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Local SIP listening address
    pub local_sip_addr: SocketAddr,
    /// Local media address for RTP
    pub local_media_addr: SocketAddr,
    /// User agent string
    pub user_agent: String,
    /// Default codec preferences
    pub preferred_codecs: Vec<String>,
    /// Maximum number of concurrent calls
    pub max_concurrent_calls: usize,
    /// Enable detailed logging
    pub enable_logging: bool,
    /// Additional configuration parameters
    pub extra_params: HashMap<String, String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            local_sip_addr: "127.0.0.1:0".parse().unwrap(), // Use random port
            local_media_addr: "127.0.0.1:0".parse().unwrap(), // Use random port
            user_agent: "rvoip-client/0.1.0".to_string(),
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            max_concurrent_calls: 10,
            enable_logging: true,
            extra_params: HashMap::new(),
        }
    }
}

impl ClientConfig {
    /// Create a new client configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Set local SIP listening address
    pub fn with_sip_addr(mut self, addr: SocketAddr) -> Self {
        self.local_sip_addr = addr;
        self
    }

    /// Set local media address
    pub fn with_media_addr(mut self, addr: SocketAddr) -> Self {
        self.local_media_addr = addr;
        self
    }

    /// Set user agent string
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = user_agent;
        self
    }

    /// Set preferred codecs
    pub fn with_codecs(mut self, codecs: Vec<String>) -> Self {
        self.preferred_codecs = codecs;
        self
    }

    /// Set maximum concurrent calls
    pub fn with_max_calls(mut self, max_calls: usize) -> Self {
        self.max_concurrent_calls = max_calls;
        self
    }

    /// Add extra configuration parameter
    pub fn with_param(mut self, key: String, value: String) -> Self {
        self.extra_params.insert(key, value);
        self
    }
}

/// Main SIP client manager that coordinates all subsystems
pub struct ClientManager {
    /// Client configuration
    config: ClientConfig,
    /// Call management
    call_manager: Arc<CallManager>,
    /// Registration management
    registration_manager: Arc<RegistrationManager>,
    /// Transaction manager (reused infrastructure)
    transaction_manager: Arc<TransactionManager>,
    /// Media manager (reused infrastructure)
    media_manager: Arc<MediaEngine>,
    /// Event bus for internal coordination
    event_bus: Arc<EventBus>,
    /// Event handler for UI integration
    event_handler: Arc<RwLock<Option<Arc<dyn ClientEventHandler>>>>,
    /// Client state
    is_running: Arc<RwLock<bool>>,
    /// Transaction event processing task handle
    event_task_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl ClientManager {
    /// Create a new client manager
    pub async fn new(config: ClientConfig) -> ClientResult<Self> {
        info!("🔧 Creating ClientManager with rvoip infrastructure integration");
        
        // Create transport layer (reuse infrastructure)
        let (transport, transport_rx) = UdpTransport::bind(config.local_sip_addr, None)
            .await
            .map_err(|e| ClientError::network_error(format!("Failed to bind transport: {}", e)))?;

        let actual_sip_addr = transport.local_addr()
            .map_err(|e| ClientError::network_error(format!("Failed to get local address: {}", e)))?;

        info!("✅ Transport bound to: {}", actual_sip_addr);

        // Create transaction manager (reuse infrastructure)
        let (transaction_manager, event_rx) = TransactionManager::new(
            Arc::new(transport),
            transport_rx,
            Some(32000), // 32 second transaction timeout (RFC 3261 Timer B)
        )
        .await
        .map_err(|e| ClientError::TransactionError(e.into()))?;

        let transaction_manager = Arc::new(transaction_manager);
        info!("✅ TransactionManager created");

        // Create event bus
        let event_bus = Arc::new(EventBus::new());

        // Create media manager (reuse infrastructure)  
        let media_manager = MediaEngine::new(Default::default())
            .await
            .map_err(|e| ClientError::MediaError(format!("Failed to create media engine: {}", e)))?;
        
        info!("✅ MediaEngine created");

        // Create subsystem managers
        let call_manager = Arc::new(CallManager::new(
            Arc::clone(&transaction_manager),
            Arc::clone(&media_manager),
        ));

        let registration_manager = Arc::new(RegistrationManager::new(Arc::clone(&transaction_manager)));

        info!("✅ Subsystem managers created");

        // Update config with actual bound address
        let mut updated_config = config;
        updated_config.local_sip_addr = actual_sip_addr;

        let client = Self {
            config: updated_config,
            call_manager,
            registration_manager,
            transaction_manager,
            media_manager,
            event_bus,
            event_handler: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            event_task_handle: Arc::new(RwLock::new(None)),
        };

        // Start transaction event processing
        client.start_transaction_event_processing(event_rx).await;

        Ok(client)
    }

    /// Start transaction event processing in background
    async fn start_transaction_event_processing(&self, mut event_rx: tokio::sync::mpsc::Receiver<TransactionEvent>) {
        let call_manager = Arc::clone(&self.call_manager);
        let registration_manager = Arc::clone(&self.registration_manager);
        let event_handler = Arc::clone(&self.event_handler);
        
        let handle = tokio::spawn(async move {
            debug!("🔄 Starting transaction event processing loop");
            
            while let Some(event) = event_rx.recv().await {
                debug!("📨 Received transaction event: {:?}", event);
                
                match event {
                    TransactionEvent::NewRequest { request, source, .. } => {
                        Self::handle_incoming_request(
                            &call_manager,
                            &registration_manager, 
                            &event_handler,
                            request
                        ).await;
                    },
                    TransactionEvent::ProvisionalResponse { response, .. } |
                    TransactionEvent::SuccessResponse { response, .. } |
                    TransactionEvent::FailureResponse { response, .. } => {
                        Self::handle_response(
                            &call_manager,
                            &registration_manager,
                            &event_handler,
                            response
                        ).await;
                    },
                    TransactionEvent::TransactionTimeout { transaction_id } => {
                        Self::handle_transaction_timeout(
                            &call_manager,
                            &registration_manager,
                            &event_handler,
                            Some(transaction_id.to_string())
                        ).await;
                    },
                    _ => {
                        debug!("🔄 Unhandled transaction event type: {:?}", event);
                    }
                }
            }
            
            debug!("🛑 Transaction event processing loop ended");
        });

        let mut task_handle = self.event_task_handle.write().await;
        *task_handle = Some(handle);
        
        info!("✅ Transaction event processing started");
    }

    /// Handle incoming SIP requests
    async fn handle_incoming_request(
        call_manager: &Arc<CallManager>,
        _registration_manager: &Arc<RegistrationManager>,
        _event_handler: &Arc<RwLock<Option<Arc<dyn ClientEventHandler>>>>,
        request: Request,
    ) {
        debug!("📨 Handling incoming {} request", request.method());
        
        match request.method() {
            Method::Invite => {
                debug!("📞 Incoming INVITE request");
                if let Err(e) = call_manager.handle_incoming_invite(request).await {
                    error!("Failed to handle incoming INVITE: {}", e);
                }
            },
            Method::Bye => {
                debug!("📴 Incoming BYE request");
                if let Err(e) = call_manager.handle_incoming_bye(request).await {
                    error!("Failed to handle incoming BYE: {}", e);
                }
            },
            Method::Ack => {
                debug!("✅ Incoming ACK request");
                if let Err(e) = call_manager.handle_incoming_ack(request).await {
                    error!("Failed to handle incoming ACK: {}", e);
                }
            },
            Method::Cancel => {
                debug!("🚫 Incoming CANCEL request");
                if let Err(e) = call_manager.handle_incoming_cancel(request).await {
                    error!("Failed to handle incoming CANCEL: {}", e);
                }
            },
            _ => {
                debug!("🔄 Unhandled request method: {}", request.method());
            }
        }
    }

    /// Handle SIP responses
    async fn handle_response(
        call_manager: &Arc<CallManager>,
        registration_manager: &Arc<RegistrationManager>,
        _event_handler: &Arc<RwLock<Option<Arc<dyn ClientEventHandler>>>>,
        response: Response,
    ) {
        debug!("📨 Handling response: {} {}", response.status_code(), response.reason_phrase());
        
        // Determine if this is a registration or call response based on CSeq method
        if let Some(cseq_header) = response.raw_header_value(&HeaderName::CSeq) {
            if cseq_header.contains("REGISTER") {
                debug!("📝 Registration response");
                if let Err(e) = registration_manager.handle_registration_response(response).await {
                    error!("Failed to handle registration response: {}", e);
                }
            } else if cseq_header.contains("INVITE") {
                debug!("📞 INVITE response");
                if let Err(e) = call_manager.handle_invite_response(response).await {
                    error!("Failed to handle INVITE response: {}", e);
                }
            } else if cseq_header.contains("BYE") {
                debug!("📴 BYE response");
                if let Err(e) = call_manager.handle_bye_response(response).await {
                    error!("Failed to handle BYE response: {}", e);
                }
            } else {
                debug!("🔄 Unhandled response method in CSeq: {}", cseq_header);
            }
        } else {
            warn!("⚠️ Response missing CSeq header");
        }
    }

    /// Handle transaction timeouts
    async fn handle_transaction_timeout(
        call_manager: &Arc<CallManager>,
        registration_manager: &Arc<RegistrationManager>,
        _event_handler: &Arc<RwLock<Option<Arc<dyn ClientEventHandler>>>>,
        transaction_key: Option<String>,
    ) {
        if let Some(key) = transaction_key {
            warn!("⏰ Transaction timeout for key: {}", key);
            
            // Notify subsystems about timeout
            if let Err(e) = call_manager.handle_transaction_timeout(&key).await {
                error!("Failed to handle call transaction timeout: {}", e);
            }
            
            if let Err(e) = registration_manager.handle_transaction_timeout(&key).await {
                error!("Failed to handle registration transaction timeout: {}", e);
            }
        }
    }

    /// Start the client (begin processing events)
    pub async fn start(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if *running {
            return Err(ClientError::internal_error("Client is already running"));
        }

        info!("▶️ Starting SIP client systems...");

        // Start subsystems
        self.call_manager.start().await?;
        self.registration_manager.start().await?;

        *running = true;
        info!("✅ SIP client started on {}", self.config.local_sip_addr);
        Ok(())
    }

    /// Stop the client
    pub async fn stop(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if !*running {
            return Ok(());
        }

        info!("🛑 Stopping SIP client...");

        // Stop subsystems
        self.call_manager.stop().await?;
        self.registration_manager.stop().await?;

        // Stop transaction event processing
        if let Some(handle) = self.event_task_handle.write().await.take() {
            handle.abort();
            info!("✅ Transaction event processing stopped");
        }

        *running = false;
        info!("✅ SIP client stopped");
        Ok(())
    }

    /// Set event handler for UI integration
    pub async fn set_event_handler(&self, handler: Arc<dyn ClientEventHandler>) {
        let mut event_handler = self.event_handler.write().await;
        *event_handler = Some(handler);
    }

    /// Remove event handler
    pub async fn remove_event_handler(&self) {
        let mut event_handler = self.event_handler.write().await;
        *event_handler = None;
    }

    // === Registration API ===

    /// Register with a SIP server
    pub async fn register(&self, config: RegistrationConfig) -> ClientResult<Uuid> {
        self.registration_manager.register(config).await
    }

    /// Unregister from a SIP server
    pub async fn unregister(&self, server_uri: &str) -> ClientResult<()> {
        self.registration_manager.unregister(server_uri).await
    }

    /// Check if registered with a server
    pub async fn is_registered(&self, server_uri: &str) -> bool {
        self.registration_manager.is_registered(server_uri).await
    }

    /// Get registration status
    pub async fn get_registration_status(&self, server_uri: &str) -> Option<RegistrationInfo> {
        self.registration_manager.get_registration_status(server_uri).await
    }

    /// List all registrations
    pub async fn list_registrations(&self) -> Vec<RegistrationInfo> {
        self.registration_manager.list_registrations().await
    }

    // === Call API ===

    /// Make an outgoing call
    pub async fn make_call(
        &self,
        local_uri: String,
        remote_uri: String,
        subject: Option<String>,
    ) -> ClientResult<CallId> {
        // Check if we're registered with the appropriate server
        // TODO: Extract server from remote_uri and check registration

        // Check concurrent call limit
        let current_calls = self.call_manager.list_calls().await;
        if current_calls.len() >= self.config.max_concurrent_calls {
            return Err(ClientError::call_setup_failed("Maximum concurrent calls reached"));
        }

        // Create the call
        let call_id = self
            .call_manager
            .create_outgoing_call(local_uri, remote_uri, subject)
            .await?;

        // TODO: Send INVITE via transaction manager
        // TODO: Set up media session
        // TODO: Handle provisional and final responses

        Ok(call_id)
    }

    /// Answer an incoming call
    pub async fn answer_call(&self, call_id: &CallId) -> ClientResult<()> {
        self.call_manager.answer_call(call_id).await
    }

    /// Reject an incoming call
    pub async fn reject_call(&self, call_id: &CallId) -> ClientResult<()> {
        self.call_manager.reject_call(call_id).await
    }

    /// Hangup a call
    pub async fn hangup_call(&self, call_id: &CallId) -> ClientResult<()> {
        self.call_manager.hangup_call(call_id).await
    }

    /// Get call information
    pub async fn get_call(&self, call_id: &CallId) -> ClientResult<CallInfo> {
        self.call_manager.get_call(call_id).await
    }

    /// List all active calls
    pub async fn list_calls(&self) -> Vec<CallInfo> {
        self.call_manager.list_calls().await
    }

    /// Get calls by state
    pub async fn get_calls_by_state(&self, state: CallState) -> Vec<CallInfo> {
        self.call_manager.get_calls_by_state(state).await
    }

    // === Media API ===

    /// Mute/unmute microphone for a call
    pub async fn set_microphone_mute(&self, _call_id: &CallId, _muted: bool) -> ClientResult<()> {
        // TODO: Implement microphone control via media manager
        // TODO: Emit media event
        Ok(())
    }

    /// Mute/unmute speaker for a call
    pub async fn set_speaker_mute(&self, _call_id: &CallId, _muted: bool) -> ClientResult<()> {
        // TODO: Implement speaker control via media manager
        // TODO: Emit media event
        Ok(())
    }

    /// Get available audio codecs
    pub async fn get_available_codecs(&self) -> Vec<String> {
        // TODO: Query media manager for available codecs
        self.config.preferred_codecs.clone()
    }

    // === Utility API ===

    /// Get client configuration
    pub fn get_config(&self) -> &ClientConfig {
        &self.config
    }

    /// Get client status
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }

    /// Get detailed client statistics
    pub async fn get_client_stats(&self) -> ClientStats {
        let call_stats = self.call_manager.get_call_stats().await;
        let registration_stats = self.registration_manager.get_registration_stats().await;
        let is_running = self.is_running().await;

        ClientStats {
            is_running,
            total_calls: call_stats.total_active_calls,
            connected_calls: call_stats.connected_calls,
            total_registrations: registration_stats.total_registrations,
            active_registrations: registration_stats.active_registrations,
            local_sip_addr: self.config.local_sip_addr,
            local_media_addr: self.config.local_media_addr,
        }
    }

    // === Internal event handling ===

    /// Emit an event to the registered handler
    async fn emit_event(&self, event: ClientEvent) {
        let handler = self.event_handler.read().await;
        if let Some(ref handler) = *handler {
            match &event {
                ClientEvent::IncomingCall(info) => {
                    let action = handler.on_incoming_call(info.clone()).await;
                    // Handle the user's action
                    match action {
                        crate::events::CallAction::Accept => {
                            if let Err(e) = self.answer_call(&info.call_id).await {
                                error!("Failed to accept call: {}", e);
                            }
                        },
                        crate::events::CallAction::Reject => {
                            if let Err(e) = self.reject_call(&info.call_id).await {
                                error!("Failed to reject call: {}", e);
                            }
                        },
                        crate::events::CallAction::Forward { target } => {
                            debug!("Call forwarded to: {}", target);
                            // TODO: Implement call forwarding
                        },
                        crate::events::CallAction::Voicemail => {
                            debug!("Call sent to voicemail");
                            // TODO: Implement voicemail handling
                        }
                    }
                }
                ClientEvent::CallStateChanged(info) => {
                    handler.on_call_state_changed(info.clone()).await;
                }
                ClientEvent::RegistrationStatusChanged(info) => {
                    handler.on_registration_status_changed(info.clone()).await;
                }
                ClientEvent::NetworkStatusChanged { connected, server, message } => {
                    handler.on_network_status_changed(*connected, server.clone(), message.clone()).await;
                }
                ClientEvent::MediaEvent { call_id, event_type, description } => {
                    handler.on_media_event(*call_id, event_type.clone(), description.clone()).await;
                }
                ClientEvent::ErrorOccurred { error, recoverable, context } => {
                    handler.on_error(error.clone(), *recoverable, context.clone()).await;
                }
            }
        }
    }

    /// Internal method to emit call state change events
    pub(crate) async fn emit_call_state_change(&self, call_id: CallId, old_state: CallState, new_state: CallState, reason: Option<String>) {
        let event = ClientEvent::CallStateChanged(CallStatusInfo {
            call_id,
            previous_state: Some(old_state),
            new_state,
            reason,
            changed_at: chrono::Utc::now(),
        });
        self.emit_event(event).await;
    }

    /// Internal method to emit registration status change events  
    pub(crate) async fn emit_registration_status_change(&self, server_uri: String, status: crate::registration::RegistrationStatus, message: Option<String>) {
        let event = ClientEvent::RegistrationStatusChanged(RegistrationStatusInfo {
            server_uri,
            user_uri: "".to_string(), // TODO: Get actual user URI
            status,
            message,
            changed_at: chrono::Utc::now(),
        });
        self.emit_event(event).await;
    }

    /// Internal method to emit media events
    pub(crate) async fn emit_media_event(&self, call_id: Option<CallId>, event_type: MediaEventType, description: String) {
        let event = ClientEvent::MediaEvent {
            call_id,
            event_type,
            description,
        };
        self.emit_event(event).await;
    }

    /// Internal method to emit error events
    pub(crate) async fn emit_error(&self, error: String, recoverable: bool, context: Option<String>) {
        let event = ClientEvent::ErrorOccurred {
            error,
            recoverable,
            context,
        };
        self.emit_event(event).await;
    }
}

/// Statistics about the client
#[derive(Debug, Clone)]
pub struct ClientStats {
    pub is_running: bool,
    pub total_calls: usize,
    pub connected_calls: usize,
    pub total_registrations: usize,
    pub active_registrations: usize,
    pub local_sip_addr: SocketAddr,
    pub local_media_addr: SocketAddr,
} 