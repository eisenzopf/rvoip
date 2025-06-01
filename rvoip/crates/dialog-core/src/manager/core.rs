//! Core Dialog Manager Implementation
//!
//! This module contains the main DialogManager struct and its core lifecycle methods.
//! It serves as the central coordinator for SIP dialog management.

use std::sync::Arc;
use std::net::SocketAddr;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use rvoip_transaction_core::{TransactionManager, TransactionKey};
use rvoip_sip_core::{Request, Response, Method};

use crate::dialog::{DialogId, Dialog, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;

use super::{
    dialog_operations::DialogStore,
    protocol_handlers::ProtocolHandlers,
    message_routing::MessageRouter,
    transaction_integration::TransactionIntegration,
    session_coordination::SessionCoordinator,
    utils::MessageExtensions,
};

#[derive(Debug)]
pub struct DialogManager {
    /// Reference to transaction manager (handles transport for us)
    pub(crate) transaction_manager: Arc<TransactionManager>,
    
    /// Local address for this dialog manager (used in Via headers)
    pub(crate) local_address: SocketAddr,
    
    /// Active dialogs by dialog ID
    pub(crate) dialogs: DashMap<DialogId, Dialog>,
    
    /// Dialog lookup by call-id + tags (key: "call-id:local-tag:remote-tag")
    pub(crate) dialog_lookup: DashMap<String, DialogId>,
    
    /// Transaction to dialog mapping
    pub(crate) transaction_to_dialog: DashMap<TransactionKey, DialogId>,
    
    /// Channel for sending session coordination events to session-core
    pub(crate) session_coordinator: tokio::sync::RwLock<Option<mpsc::Sender<SessionCoordinationEvent>>>,
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
            dialogs: DashMap::new(),
            dialog_lookup: DashMap::new(),
            transaction_to_dialog: DashMap::new(),
            session_coordinator: tokio::sync::RwLock::new(None),
        })
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
            rvoip_sip_core::Message::Response(response) => {
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
        let _ack_request = self.create_ack_for_2xx_response(original_invite_tx_id, response).await?;
        // The transaction-core helper should handle sending the ACK
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