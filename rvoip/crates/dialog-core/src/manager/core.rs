//! Core Dialog Manager Implementation
//!
//! This module contains the main DialogManager struct and its core lifecycle methods.
//! It serves as the central coordinator for SIP dialog management.

use std::sync::Arc;
use std::net::SocketAddr;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{debug, info, warn, error};

use rvoip_transaction_core::{TransactionManager, TransactionKey, TransactionEvent};
use rvoip_sip_core::{Request, Response, Method};

use crate::dialog::{DialogId, Dialog, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::{DialogEvent, SessionCoordinationEvent};
use crate::config::DialogManagerConfig;


#[derive(Debug, Clone)]
pub struct DialogManager {
    /// Reference to transaction manager (handles transport for us)
    pub(crate) transaction_manager: Arc<TransactionManager>,
    
    /// Local address for this dialog manager (used in Via headers)
    pub(crate) local_address: SocketAddr,
    
    /// **NEW**: Optional unified configuration for behavioral modes
    /// When present, enables mode-specific behavior (auto-responses, etc.)
    pub(crate) config: Option<DialogManagerConfig>,
    
    /// Active dialogs by dialog ID
    pub(crate) dialogs: Arc<DashMap<DialogId, Dialog>>,
    
    /// Dialog lookup by call-id + tags (key: "call-id:local-tag:remote-tag")
    pub(crate) dialog_lookup: Arc<DashMap<String, DialogId>>,
    
    /// Transaction to dialog mapping
    pub(crate) transaction_to_dialog: Arc<DashMap<TransactionKey, DialogId>>,
    
    /// Channel for sending session coordination events to session-core
    pub(crate) session_coordinator: Arc<tokio::sync::RwLock<Option<mpsc::Sender<SessionCoordinationEvent>>>>,
    
    /// Channel for sending dialog events to external consumers (session-core)
    pub(crate) dialog_event_sender: Arc<tokio::sync::RwLock<Option<mpsc::Sender<DialogEvent>>>>,
    
    /// Shutdown signal for global event processor
    pub(crate) shutdown_signal: Arc<tokio::sync::Notify>,
}

impl DialogManager {
    /// Create a new dialog manager
    /// 
    /// **ARCHITECTURE**: dialog-core receives TransactionManager via dependency injection.
    /// The application level is responsible for creating the transaction layer.
    /// 
    /// # Arguments
    /// * `transaction_manager` - The transaction manager to use for SIP message reliability
    /// * `local_address` - The local address to use in Via headers and Contact headers
    /// 
    /// # Returns
    /// A new DialogManager instance ready for use
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        local_address: SocketAddr,
    ) -> DialogResult<Self> {
        info!("Creating new DialogManager with local address {}", local_address);
        
        Ok(Self {
            transaction_manager,
            local_address,
            config: None,
            dialogs: Arc::new(DashMap::new()),
            dialog_lookup: Arc::new(DashMap::new()),
            transaction_to_dialog: Arc::new(DashMap::new()),
            session_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
        })
    }
    
    /// Create a new dialog manager with global transaction events (RECOMMENDED)
    /// 
    /// This constructor follows the working pattern from transaction-core examples
    /// by receiving global transaction events for proper event consumption.
    /// 
    /// # Arguments
    /// * `transaction_manager` - The transaction manager to use for SIP message reliability
    /// * `transaction_events` - Global transaction event receiver
    /// * `local_address` - The local address to use in Via headers and Contact headers
    /// 
    /// # Returns
    /// A new DialogManager instance with proper event consumption
    pub async fn with_global_events(
        transaction_manager: Arc<TransactionManager>,
        transaction_events: mpsc::Receiver<TransactionEvent>,
        local_address: SocketAddr,
    ) -> DialogResult<Self> {
        info!("Creating new DialogManager with global transaction events and local address {}", local_address);
        
        let manager = Self {
            transaction_manager,
            local_address,
            config: None,
            dialogs: Arc::new(DashMap::new()),
            dialog_lookup: Arc::new(DashMap::new()),
            transaction_to_dialog: Arc::new(DashMap::new()),
            session_coordinator: Arc::new(tokio::sync::RwLock::new(None)),
            dialog_event_sender: Arc::new(tokio::sync::RwLock::new(None)),
            shutdown_signal: Arc::new(tokio::sync::Notify::new()),
        };
        
        // Spawn global transaction event processor
        let event_processor = manager.clone();
        tokio::spawn(async move {
            event_processor.process_global_transaction_events(transaction_events).await;
        });
        
        Ok(manager)
    }
    
    /// Process global transaction events (similar to working transaction-core examples)
    /// 
    /// This follows the exact pattern from working examples that use global event consumption
    /// instead of individual transaction subscriptions.
    async fn process_global_transaction_events(&self, mut events: mpsc::Receiver<TransactionEvent>) {
        info!("🔄 Starting global transaction event processor for dialog-core");
        
        loop {
            tokio::select! {
                // Process transaction events
                event = events.recv() => {
                    match event {
                        Some(event) => {
                            // Extract transaction ID from the event
                            let transaction_id = self.extract_transaction_id(&event);
                            
                            // Find the dialog associated with this transaction
                            if let Some(dialog_id) = self.find_dialog_for_transaction_event(&transaction_id) {
                                if let Err(e) = self.process_transaction_event(&transaction_id, &dialog_id, event).await {
                                    error!("Failed to process transaction event for dialog {}: {}", dialog_id, e);
                                }
                            } else {
                                // No dialog found using transaction-to-dialog mapping
                                
                                // Special handling for AckReceived events: use dialog-based matching
                                if let TransactionEvent::AckReceived { request, .. } = &event {
                                    // Find dialog using Call-ID, From tag, To tag from the ACK request
                                    if let Some(dialog_id) = self.find_dialog_for_request(request).await {
                                        if let Err(e) = self.process_transaction_event(&transaction_id, &dialog_id, event).await {
                                            error!("Failed to process AckReceived event for dialog {}: {}", dialog_id, e);
                                        }
                                    } else {
                                        // Still treat as unassociated event
                                        if let Err(e) = self.handle_unassociated_transaction_event(&transaction_id, event).await {
                                            error!("Failed to handle unassociated AckReceived event {}: {}", transaction_id, e);
                                        }
                                    }
                                } else {
                                    // Event for transaction not associated with any dialog
                                    // Check if this is a new incoming INVITE that should create a dialog
                                    if let Err(e) = self.handle_unassociated_transaction_event(&transaction_id, event).await {
                                        error!("Failed to handle unassociated transaction event {}: {}", transaction_id, e);
                                    }
                                }
                            }
                        },
                        None => {
                            // Channel closed
                            debug!("Transaction events channel closed");
                            break;
                        }
                    }
                },
                
                // Wait for shutdown signal
                _ = self.shutdown_signal.notified() => {
                    info!("🛑 Global transaction event processor received shutdown signal");
                    break;
                }
            }
        }
        
        info!("🏁 Global transaction event processor for dialog-core stopped");
    }
    
    /// Extract transaction ID from any TransactionEvent variant
    fn extract_transaction_id(&self, event: &TransactionEvent) -> TransactionKey {
        match event {
            TransactionEvent::AckReceived { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::CancelReceived { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::ProvisionalResponse { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::SuccessResponse { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::FailureResponse { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::ProvisionalResponseSent { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::FinalResponseSent { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::TransactionTimeout { transaction_id } => transaction_id.clone(),
            TransactionEvent::AckTimeout { transaction_id } => transaction_id.clone(),
            TransactionEvent::TransportError { transaction_id } => transaction_id.clone(),
            TransactionEvent::Error { transaction_id, .. } => {
                transaction_id.clone().unwrap_or_else(|| TransactionKey::new("unknown".to_string(), Method::Info, false))
            },
            TransactionEvent::TransactionTerminated { transaction_id } => transaction_id.clone(),
            TransactionEvent::StateChanged { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::TimerTriggered { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::CancelRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::AckRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::InviteRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::NonInviteRequest { transaction_id, .. } => transaction_id.clone(),
            TransactionEvent::StrayRequest { .. } => TransactionKey::new("stray".to_string(), Method::Info, false),
            TransactionEvent::StrayResponse { .. } => TransactionKey::new("stray".to_string(), Method::Info, false),
            TransactionEvent::StrayAck { .. } => TransactionKey::new("stray".to_string(), Method::Info, false),
            TransactionEvent::StrayCancel { .. } => TransactionKey::new("stray".to_string(), Method::Info, false),
            TransactionEvent::StrayAckRequest { .. } => TransactionKey::new("stray".to_string(), Method::Info, false),
        }
    }
    
    /// Find dialog associated with a transaction event
    fn find_dialog_for_transaction_event(&self, transaction_id: &TransactionKey) -> Option<DialogId> {
        self.transaction_to_dialog.get(transaction_id).map(|entry| entry.clone())
    }
    
    /// Handle transaction events not associated with any existing dialog
    /// 
    /// This handles new incoming requests that should create dialogs.
    async fn handle_unassociated_transaction_event(&self, transaction_id: &TransactionKey, event: TransactionEvent) -> DialogResult<()> {
        match event {
            TransactionEvent::InviteRequest { request, source, .. } => {
                println!("🎯 FOUND UNASSOCIATED INVITE: Processing new incoming INVITE from {}", source);
                debug!("Processing new incoming INVITE request from transaction {}", transaction_id);
                
                // This is a new incoming INVITE - create dialog and process it
                self.handle_initial_invite(transaction_id.clone(), request, source).await?;
                
                debug!("Successfully processed new incoming INVITE from {}", source);
                Ok(())
            },
            
            TransactionEvent::NonInviteRequest { request, source, .. } => {
                debug!("Processing new incoming {} request from transaction {}", request.method(), transaction_id);
                
                // Handle non-INVITE requests (REGISTER, OPTIONS, etc.)
                self.handle_request(request, source).await
            },
            
            _ => {
                // Other unassociated events (responses, timeouts, etc.) - just log them
                debug!("Received unassociated transaction event: {:?}", event);
                Ok(())
            }
        }
    }
    
    /// Get the configured local address
    /// 
    /// Returns the local address that this DialogManager uses for Via headers
    /// and Contact headers when creating SIP requests.
    pub fn local_address(&self) -> SocketAddr {
        self.local_address
    }
    
    /// Set the session coordinator for sending events to session-core
    /// 
    /// This establishes the communication channel between dialog-core and session-core,
    /// maintaining the proper architectural layer separation.
    /// 
    /// # Arguments
    /// * `sender` - Channel sender for session coordination events
    pub async fn set_session_coordinator(&self, sender: mpsc::Sender<SessionCoordinationEvent>) {
        *self.session_coordinator.write().await = Some(sender);
        debug!("Session coordinator configured");
    }
    
    /// Set the dialog event sender for external consumers (session-core)
    /// 
    /// This establishes the dialog event communication channel that session-core
    /// can use to receive high-level dialog state changes and events.
    /// 
    /// # Arguments
    /// * `sender` - Channel sender for dialog events
    pub async fn set_dialog_event_sender(&self, sender: mpsc::Sender<DialogEvent>) {
        *self.dialog_event_sender.write().await = Some(sender);
        debug!("Dialog event sender configured for session-core");
    }
    
    /// Subscribe to dialog events
    /// 
    /// Returns a receiver for dialog events that session-core can use to monitor
    /// dialog state changes and other dialog-level events.
    /// 
    /// # Returns
    /// A receiver for dialog events
    pub fn subscribe_to_dialog_events(&self) -> mpsc::Receiver<DialogEvent> {
        let (tx, rx) = mpsc::channel(100);
        
        // Store the sender for future use
        tokio::spawn({
            let dialog_event_sender = self.dialog_event_sender.clone();
            async move {
                *dialog_event_sender.write().await = Some(tx);
            }
        });
        
        rx
    }
    
    /// Emit a dialog event to external consumers
    /// 
    /// Sends dialog events to session-core for high-level dialog state management.
    /// This maintains the proper architectural separation where dialog-core handles
    /// SIP protocol details and session-core handles session logic.
    pub async fn emit_dialog_event(&self, event: DialogEvent) {
        if let Some(sender) = self.dialog_event_sender.read().await.as_ref() {
            if let Err(e) = sender.send(event.clone()).await {
                warn!("Failed to send dialog event to session-core: {}", e);
            } else {
                debug!("Emitted dialog event: {:?}", event);
            }
        }
    }
    
    /// Emit a session coordination event
    /// 
    /// Sends session coordination events for legacy compatibility and specific
    /// session management operations.
    pub async fn emit_session_coordination_event(&self, event: SessionCoordinationEvent) {
        if let Some(sender) = self.session_coordinator.read().await.as_ref() {
            if let Err(e) = sender.send(event.clone()).await {
                warn!("Failed to send session coordination event: {}", e);
            } else {
                debug!("Emitted session coordination event: {:?}", event);
            }
        }
    }
    
    /// **CENTRAL DISPATCHER**: Handle incoming SIP messages
    /// 
    /// This is the main entry point for processing SIP messages in dialog-core.
    /// It routes messages to the appropriate method-specific handlers while maintaining
    /// RFC 3261 compliance for dialog state management.
    /// 
    /// # Arguments
    /// * `message` - The SIP message (Request or Response)
    /// * `source` - Source address of the message
    /// 
    /// # Returns
    /// Result indicating success or the specific error encountered
    pub async fn handle_message(&self, message: rvoip_sip_core::Message, source: SocketAddr) -> DialogResult<()> {
        match message {
            rvoip_sip_core::Message::Request(request) => {
                self.handle_request(request, source).await
            },
            rvoip_sip_core::Message::Response(_response) => {
                // For responses, we need the transaction ID to route properly
                // This would typically come from the transaction layer
                warn!("Response handling requires transaction ID - use handle_response() directly");
                Err(DialogError::protocol_error("Response handling requires transaction context"))
            }
        }
    }
    
    /// Handle incoming SIP requests
    /// 
    /// Routes requests to appropriate method handlers based on the SIP method.
    /// Implements RFC 3261 Section 12 dialog handling requirements.
    /// 
    /// # Arguments
    /// * `request` - The SIP request to handle
    /// * `source` - Source address of the request
    async fn handle_request(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Handling {} request from {}", request.method(), source);
        
        // Dispatch request to appropriate handler based on method
        match request.method() {
            Method::Invite => self.handle_invite(request, source).await,
            Method::Bye => self.handle_bye(request).await,
            Method::Cancel => self.handle_cancel(request).await,
            Method::Ack => self.handle_ack(request).await,
            Method::Options => self.handle_options(request, source).await,
            Method::Register => self.handle_register(request, source).await,
            Method::Update => self.handle_update(request).await,
            Method::Info => self.handle_info(request, source).await,
            Method::Refer => self.handle_refer(request, source).await,
            Method::Subscribe => self.handle_subscribe(request, source).await,
            Method::Notify => self.handle_notify(request, source).await,
            method => {
                warn!("Unsupported SIP method: {}", method);
                Err(DialogError::protocol_error(&format!("Unsupported method: {}", method)))
            }
        }
    }
    
    /// Start the dialog manager
    /// 
    /// Initializes the dialog manager for processing. This can include starting
    /// background tasks for dialog cleanup, recovery, and maintenance.
    pub async fn start(&self) -> DialogResult<()> {
        info!("DialogManager starting");
        
        // TODO: Start background processing tasks (cleanup, recovery, etc.)
        // - Dialog timeout monitoring
        // - Orphaned dialog cleanup
        // - Recovery coordination
        // - Statistics collection
        
        info!("DialogManager started successfully");
        Ok(())
    }
    
    /// Stop the dialog manager
    /// 
    /// Gracefully shuts down the dialog manager, terminating all active dialogs
    /// and cleaning up resources according to RFC 3261 requirements.
    pub async fn stop(&self) -> DialogResult<()> {
        info!("DialogManager stopping");
        
        // Signal shutdown to global event processor first
        self.shutdown_signal.notify_one();
        debug!("Sent shutdown signal to global event processor");
        
        // Terminate all active dialogs gracefully
        let dialog_ids: Vec<DialogId> = self.dialogs.iter()
            .map(|entry| entry.key().clone())
            .collect();
        
        debug!("Terminating {} active dialogs", dialog_ids.len());
        
        for dialog_id in dialog_ids {
            if let Err(e) = self.terminate_dialog(&dialog_id).await {
                debug!("Failed to terminate dialog {}: {}", dialog_id, e);
            }
        }
        
        // Clear all mappings
        self.dialogs.clear();
        self.dialog_lookup.clear();
        self.transaction_to_dialog.clear();
        
        info!("DialogManager stopped successfully");
        Ok(())
    }
    
    /// Get the transaction manager reference
    /// 
    /// Provides access to the underlying transaction manager for cases where
    /// direct transaction operations are needed.
    pub fn transaction_manager(&self) -> &Arc<TransactionManager> {
        &self.transaction_manager
    }
    
    /// Get dialog count
    /// 
    /// Returns the current number of active dialogs.
    pub fn dialog_count(&self) -> usize {
        self.dialogs.len()
    }
    
    /// Check if a dialog exists
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to check
    /// 
    /// # Returns
    /// true if the dialog exists, false otherwise
    pub fn has_dialog(&self, dialog_id: &DialogId) -> bool {
        self.dialogs.contains_key(dialog_id)
    }
    
    /// Clean up completed transaction event receivers
    /// 
    /// This method removes transaction-to-dialog mappings for completed transactions.
    /// 
    /// # Arguments
    /// * `transaction_id` - The transaction ID to clean up
    pub fn cleanup_transaction_receiver(&self, transaction_id: &TransactionKey) {
        // Remove from transaction-to-dialog mapping if present
        if self.transaction_to_dialog.remove(transaction_id).is_some() {
            debug!("Cleaned up transaction-dialog mapping for completed transaction {}", transaction_id);
        }
    }
    
    // ========================================
    // **NEW**: UNIFIED CONFIGURATION SUPPORT
    // ========================================
    
    /// Set the unified configuration for this DialogManager
    /// 
    /// Enables mode-specific behavior based on configuration.
    /// This method allows the UnifiedDialogManager to inject configuration.
    /// 
    /// # Arguments
    /// * `config` - Unified configuration determining behavior mode
    pub fn set_config(&mut self, config: DialogManagerConfig) {
        debug!("Setting unified configuration to {:?} mode", Self::config_mode_name(&config));
        self.config = Some(config);
    }
    
    /// Get the current configuration (if any)
    /// 
    /// Returns the unified configuration if it was provided.
    pub fn config(&self) -> Option<&DialogManagerConfig> {
        self.config.as_ref()
    }
    
    /// Check if auto-response to OPTIONS requests is enabled
    /// 
    /// Returns true if the unified configuration enables automatic OPTIONS responses.
    /// If no configuration is set, defaults to false (session layer handling).
    pub fn should_auto_respond_to_options(&self) -> bool {
        self.config
            .as_ref()
            .map(|config| config.auto_options_enabled())
            .unwrap_or(false)
    }
    
    /// Check if auto-response to REGISTER requests is enabled
    /// 
    /// Returns true if the unified configuration enables automatic REGISTER responses.
    /// If no configuration is set, defaults to false (session layer handling).
    pub fn should_auto_respond_to_register(&self) -> bool {
        self.config
            .as_ref()
            .map(|config| config.auto_register_enabled())
            .unwrap_or(false)
    }
    
    /// Check if outgoing calls are supported
    /// 
    /// Returns true if the configuration supports outgoing calls (Client/Hybrid modes).
    /// If no configuration is set, defaults to true for backward compatibility.
    pub fn supports_outgoing_calls(&self) -> bool {
        self.config
            .as_ref()
            .map(|config| config.supports_outgoing_calls())
            .unwrap_or(true) // Default to true for backward compatibility
    }
    
    /// Check if incoming calls are supported
    /// 
    /// Returns true if the configuration supports incoming calls (Server/Hybrid modes).
    /// If no configuration is set, defaults to true for backward compatibility.
    pub fn supports_incoming_calls(&self) -> bool {
        self.config
            .as_ref()
            .map(|config| config.supports_incoming_calls())
            .unwrap_or(true) // Default to true for backward compatibility
    }
    
    /// Get configuration mode name for logging
    fn config_mode_name(config: &DialogManagerConfig) -> &'static str {
        match config {
            DialogManagerConfig::Client(_) => "Client",
            DialogManagerConfig::Server(_) => "Server",
            DialogManagerConfig::Hybrid(_) => "Hybrid",
        }
    }
}

// Forward declarations for methods that will be implemented in other modules
impl DialogManager {
    // Dialog Operations (delegated to dialog_operations.rs)
    pub async fn create_dialog(&self, request: &Request) -> DialogResult<DialogId> {
        <Self as super::dialog_operations::DialogStore>::create_dialog(self, request).await
    }
    
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> DialogResult<()> {
        <Self as super::dialog_operations::DialogStore>::terminate_dialog(self, dialog_id).await
    }
    
    pub fn get_dialog(&self, dialog_id: &DialogId) -> DialogResult<Dialog> {
        <Self as super::dialog_operations::DialogStore>::get_dialog(self, dialog_id)
    }
    
    pub fn get_dialog_mut(&self, dialog_id: &DialogId) -> DialogResult<dashmap::mapref::one::RefMut<DialogId, Dialog>> {
        <Self as super::dialog_operations::DialogStore>::get_dialog_mut(self, dialog_id)
    }
    
    pub async fn store_dialog(&self, dialog: Dialog) -> DialogResult<()> {
        <Self as super::dialog_operations::DialogStore>::store_dialog(self, dialog).await
    }
    
    pub fn list_dialogs(&self) -> Vec<DialogId> {
        <Self as super::dialog_operations::DialogStore>::list_dialogs(self)
    }
    
    pub fn get_dialog_state(&self, dialog_id: &DialogId) -> DialogResult<DialogState> {
        <Self as super::dialog_operations::DialogStore>::get_dialog_state(self, dialog_id)
    }
    
    pub async fn update_dialog_state(&self, dialog_id: &DialogId, new_state: DialogState) -> DialogResult<()> {
        <Self as super::dialog_operations::DialogStore>::update_dialog_state(self, dialog_id, new_state).await
    }
    
    pub async fn create_outgoing_dialog(&self, local_uri: rvoip_sip_core::Uri, remote_uri: rvoip_sip_core::Uri, call_id: Option<String>) -> DialogResult<DialogId> {
        <Self as super::dialog_operations::DialogStore>::create_outgoing_dialog(self, local_uri, remote_uri, call_id).await
    }
    
    // Protocol Handlers (delegated to protocol_handlers.rs)
    pub async fn handle_invite(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_invite_method(self, request, source).await
    }
    
    pub async fn handle_bye(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_bye_method(self, request).await
    }
    
    pub async fn handle_cancel(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_cancel_method(self, request).await
    }
    
    pub async fn handle_ack(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_ack_method(self, request).await
    }
    
    pub async fn handle_options(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_options_method(self, request, source).await
    }
    
    pub async fn handle_register(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_register_method(self, request, source).await
    }
    
    pub async fn handle_update(&self, request: Request) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_update_method(self, request).await
    }
    
    pub async fn handle_info(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_info_method(self, request, source).await
    }
    
    pub async fn handle_refer(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_refer_method(self, request, source).await
    }
    
    pub async fn handle_subscribe(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_subscribe_method(self, request, source).await
    }
    
    pub async fn handle_notify(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        <Self as super::protocol_handlers::MethodHandler>::handle_notify_method(self, request, source).await
    }
    
    pub async fn handle_response(&self, response: Response, transaction_id: TransactionKey) -> DialogResult<()> {
        <Self as super::protocol_handlers::ProtocolHandlers>::handle_response_message(self, response, transaction_id).await
    }
    
    // Message Routing (delegated to message_routing.rs)
    pub async fn find_dialog_for_request(&self, request: &Request) -> Option<DialogId> {
        <Self as super::dialog_operations::DialogLookup>::find_dialog_for_request(self, request).await
    }
    
    pub fn find_dialog_for_transaction(&self, transaction_id: &TransactionKey) -> DialogResult<DialogId> {
        <Self as super::message_routing::DialogMatcher>::match_transaction(self, transaction_id)
    }
    
    // Transaction Integration (delegated to transaction_integration.rs)
    pub async fn send_request(&self, dialog_id: &DialogId, method: Method, body: Option<bytes::Bytes>) -> DialogResult<TransactionKey> {
        <Self as super::transaction_integration::TransactionIntegration>::send_request_in_dialog(self, dialog_id, method, body).await
    }
    
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> DialogResult<()> {
        <Self as super::transaction_integration::TransactionIntegration>::send_transaction_response(self, transaction_id, response).await
    }
    
    pub fn associate_transaction_with_dialog(&self, transaction_id: &TransactionKey, dialog_id: &DialogId) {
        <Self as super::transaction_integration::TransactionHelpers>::link_transaction_to_dialog(self, transaction_id, dialog_id)
    }
    
    pub async fn send_ack_for_2xx_response(&self, dialog_id: &DialogId, original_invite_tx_id: &TransactionKey, response: &Response) -> DialogResult<()> {
        debug!("Sending ACK for 2xx response for dialog {}", dialog_id);
        
        // Use transaction-core's send_ack_for_2xx method to actually send the ACK
        self.transaction_manager
            .send_ack_for_2xx(original_invite_tx_id, response)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to send ACK for 2xx response: {}", e),
            })?;
        
        debug!("Successfully sent ACK for 2xx response for dialog {}", dialog_id);
        Ok(())
    }
    
    pub async fn create_ack_for_2xx_response(&self, original_invite_tx_id: &TransactionKey, response: &Response) -> DialogResult<Request> {
        <Self as super::transaction_integration::TransactionHelpers>::create_ack_for_success_response(self, original_invite_tx_id, response).await
    }
    
    pub async fn find_transaction_by_message(&self, message: &rvoip_sip_core::Message) -> DialogResult<Option<TransactionKey>> {
        debug!("Finding transaction for message using transaction-core");
        
        self.transaction_manager.find_transaction_by_message(message).await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to find transaction by message: {}", e),
            })
    }
} 