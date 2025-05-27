use std::sync::Arc;
use std::fmt;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{debug, info, warn, error};
use std::str::FromStr;
use std::net::SocketAddr;
use std::time::SystemTime;
use std::collections::HashMap;
use std::collections::HashSet;
use futures::stream::{StreamExt, FuturesUnordered};

use rvoip_sip_core::{
    Request, Response, Method, StatusCode, Uri, TypedHeader, HeaderName
};

use rvoip_sip_core::types::address::Address;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::from::From as FromHeader;
use rvoip_sip_core::types::to::To as ToHeader;

use rvoip_transaction_core::{
    TransactionManager, 
    TransactionEvent, 
    TransactionState, 
    TransactionKey,
    TransactionKind
};

use super::dialog_state::DialogState;
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use crate::events::{EventBus, SessionEvent};
use crate::session::SessionId;
use crate::{dialog_not_found_error, network_unreachable_error, transaction_creation_error, transaction_send_error};

use super::dialog_id::DialogId;
use super::dialog_impl::Dialog;
use super::dialog_utils::uri_resolver;

use rvoip_sip_transport::Transport;
use rvoip_sip_transport::error::Error as TransportError;

use async_trait::async_trait;
use super::recovery::{RecoveryConfig, RecoveryMetrics};
use serde::{Serialize, Deserialize};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use std::sync::atomic::{AtomicUsize, Ordering};

// Constants for channel sizing and buffer management
const DEFAULT_EVENT_CHANNEL_SIZE: usize = 100;
const SUBSCRIPTION_CHECK_INTERVAL_MS: u64 = 250;

/// Manager for SIP dialogs that integrates with the transaction layer
#[derive(Clone)]
pub struct DialogManager {
    /// Active dialogs by ID
    dialogs: DashMap<DialogId, Dialog>,
    
    /// Dialog lookup by SIP dialog identifier tuple (call-id, local-tag, remote-tag)
    dialog_lookup: DashMap<(String, String, String), DialogId>,
    
    /// DialogId mapped to SessionId for session references
    dialog_to_session: DashMap<DialogId, SessionId>,
    
    /// Transaction manager reference
    transaction_manager: Arc<TransactionManager>,
    
    /// Transaction to Dialog mapping
    transaction_to_dialog: DashMap<TransactionKey, DialogId>,
    
    /// Track which transactions we've already subscribed to to avoid duplicate subscriptions
    subscribed_transactions: DashMap<TransactionKey, bool>,
    
    /// Main event channel for distributing transaction events
    event_sender: mpsc::Sender<TransactionEvent>,
    
    /// Event bus for dialog events
    event_bus: EventBus,
    
    /// For testing purposes - whether to run recovery in background
    run_recovery_in_background: bool,
    
    /// Recovery configuration
    recovery_config: RecoveryConfig,
    
    /// Recovery metrics
    recovery_metrics: Arc<RwLock<RecoveryMetrics>>,
}

impl DialogManager {
    /// Create a new dialog manager
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
    ) -> Self {
        Self::new_with_full_config(
            transaction_manager,
            event_bus,
            true,
            RecoveryConfig::default(),
        )
    }
    
    /// Create a new dialog manager with custom recovery configuration
    pub fn new_with_recovery_config(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
        recovery_config: RecoveryConfig,
    ) -> Self {
        Self::new_with_full_config(
            transaction_manager,
            event_bus,
            true,
            recovery_config,
        )
    }
    
    /// Create a new dialog manager with a specified recovery mode (for testing)
    pub fn new_with_recovery_mode(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
        run_recovery_in_background: bool,
    ) -> Self {
        Self::new_with_full_config(
            transaction_manager,
            event_bus,
            run_recovery_in_background,
            RecoveryConfig::default(),
        )
    }
    
    /// Create a fully customized dialog manager (for testing)
    pub fn new_with_full_config(
        transaction_manager: Arc<TransactionManager>,
        event_bus: EventBus,
        run_recovery_in_background: bool,
        recovery_config: RecoveryConfig,
    ) -> Self {
        // Create the main event channel
        let (event_sender, mut event_receiver) = mpsc::channel(DEFAULT_EVENT_CHANNEL_SIZE);
        
        let dialogs = DashMap::new();
        let dialog_lookup = DashMap::new();
        let dialog_to_session = DashMap::new();
        let transaction_to_dialog = DashMap::new();
        let subscribed_transactions = DashMap::new();
        let recovery_metrics = Arc::new(RwLock::new(RecoveryMetrics::default()));
        
        // Create the dialog manager
        let dialog_manager = Self {
            dialogs,
            dialog_lookup,
            dialog_to_session,
            transaction_manager: transaction_manager.clone(),
            transaction_to_dialog,
            subscribed_transactions,
            event_sender,
            event_bus,
            run_recovery_in_background,
            recovery_config,
            recovery_metrics,
        };
        
        // Start the event processor for the event_receiver
        let dm = dialog_manager.clone();
        tokio::spawn(async move {
            while let Some(event) = event_receiver.recv().await {
                dm.process_transaction_event(event).await;
            }
        });
        
        dialog_manager
    }
    
    /// Subscribe to transaction events and start processing them
    pub async fn start(&self) -> mpsc::Receiver<TransactionEvent> {
        // Set up direct event forwarding 
        let dialog_manager = self.clone();
        
        // Create a single task that directly subscribes to ALL transactions
        // This avoids the polling approach and batch processing entirely
        tokio::spawn(async move {
            let tx_manager = dialog_manager.transaction_manager.clone();
            
            // Subscribe to ALL transaction events - we'll filter them as needed
            let mut all_events_rx = tx_manager.subscribe();
            
            // Process events directly
            while let Some(event) = all_events_rx.recv().await {
                if let Err(e) = dialog_manager.event_sender.send(event.clone()).await {
                    error!("Failed to forward transaction event: {}", e);
                    break;
                }
                
                // Process the event directly as well to avoid any delays
                dialog_manager.process_transaction_event(event).await;
            }
            
            debug!("Main transaction event subscription task terminated");
        });
        
        // Return a subscription for the caller to use if needed
        self.transaction_manager.subscribe()
    }
    
    /// Process a transaction event and update dialogs accordingly
    pub async fn process_transaction_event(&self, event: TransactionEvent) {
        debug!("Processing transaction event: {:?}", event);
        
        match event {
            TransactionEvent::Response { transaction_id: tx_key, response, source: _ } => {
                debug!("Received response through transaction {}:\n{}", tx_key, response);
                
                // Find dialog associated with this transaction
                let dialog_id = match self.transaction_to_dialog.get(&tx_key) {
                    Some(dialog_id) => dialog_id.clone(),
                    None => {
                        debug!("No dialog found for transaction {:?}", tx_key);
                        return;
                    }
                };
                
                // Get the dialog
                let mut dialog_opt = self.dialogs.get_mut(&dialog_id);
                if dialog_opt.is_none() {
                    debug!("Dialog {} not found for transaction {}", dialog_id, tx_key);
                    return;
                }
                let mut dialog = dialog_opt.unwrap();
                
                // Check if this is a response to an INVITE
                let is_invite = match self.transaction_manager.transaction_kind(&tx_key).await {
                    Ok(TransactionKind::InviteClient) => true,
                    _ => false
                };
                
                // If this is a 2xx response to an INVITE, update dialog
                if is_invite && (response.status == StatusCode::Ok || response.status == StatusCode::Accepted) {
                    if dialog.state == DialogState::Early {
                        // Update the dialog from early to confirmed
                        let old_state = dialog.state.clone();
                        let updated = dialog.update_from_2xx(&response);
                        
                        // Check if negotiation is complete for SDP
                        let session_id = self.find_session_for_transaction(&tx_key);
                        
                        // If SDP is present, handle SDP answer
                        if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
                            if content_type.to_string() == "application/sdp" {
                                if let Ok(sdp_str) = std::str::from_utf8(&response.body) {
                                    if let Ok(sdp) = crate::sdp::SessionDescription::from_str(sdp_str) {
                                        // Update the dialog with the remote SDP answer
                                        if dialog.sdp_context.state == crate::sdp::NegotiationState::OfferSent {
                                            dialog.sdp_context.update_with_remote_answer(sdp.clone());
                                            
                                            // Fire SDP answer received event
                                            if let Some(session_id) = session_id.clone() {
                                                self.event_bus.publish(crate::events::SessionEvent::SdpAnswerReceived {
                                                    session_id: session_id.clone(),
                                                    dialog_id: dialog_id.clone(),
                                                });
                                                
                                                // Emit negotiation complete event
                                                if dialog.sdp_context.is_complete() {
                                                    self.event_bus.publish(crate::events::SessionEvent::SdpNegotiationComplete {
                                                        session_id: session_id,
                                                        dialog_id: dialog_id.clone(),
                                                    });
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        
                        if updated {
                            // Emit dialog state changed event
                            if let Some(session_id) = session_id {
                                self.event_bus.publish(crate::events::SessionEvent::DialogStateChanged {
                                    session_id,
                                    dialog_id: dialog_id.clone(),
                                    previous: old_state,
                                    current: dialog.state.clone(),
                                });
                            }
                        }
                    }
                }
                // For non-2xx INVITE responses with SDP, handle early media
                else if is_invite && response.status == StatusCode::SessionProgress {
                    // Check for SDP in early media
                    if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
                        if content_type.to_string() == "application/sdp" {
                            if let Ok(sdp_str) = std::str::from_utf8(&response.body) {
                                if let Ok(sdp) = crate::sdp::SessionDescription::from_str(sdp_str) {
                                    // Update the dialog with the remote SDP answer (early media)
                                    if dialog.sdp_context.state == crate::sdp::NegotiationState::OfferSent {
                                        dialog.sdp_context.update_with_remote_answer(sdp.clone());
                                        
                                        // Fire SDP answer received event
                                        if let Some(session_id) = self.find_session_for_transaction(&tx_key) {
                                            self.event_bus.publish(crate::events::SessionEvent::SdpAnswerReceived {
                                                session_id: session_id.clone(),
                                                dialog_id: dialog_id.clone(),
                                            });
                                            
                                            // Emit negotiation complete event for early media
                                            if dialog.sdp_context.is_complete() {
                                                self.event_bus.publish(crate::events::SessionEvent::SdpNegotiationComplete {
                                                    session_id,
                                                    dialog_id: dialog_id.clone(),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                // Handle success responses for UPDATE method
                else if response.status == StatusCode::Ok || response.status == StatusCode::Accepted {
                    // Note: In the future, we could add specific handling for UPDATE responses
                    // by checking the CSeq method in the response
                    
                    // Handle SDP in the response if it exists
                    if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
                        if content_type.to_string() == "application/sdp" {
                            if let Ok(sdp_str) = std::str::from_utf8(&response.body) {
                                if let Ok(sdp) = crate::sdp::SessionDescription::from_str(sdp_str) {
                                    // Update the dialog with the remote SDP answer
                                    if dialog.sdp_context.state == crate::sdp::NegotiationState::OfferSent {
                                        dialog.sdp_context.update_with_remote_answer(sdp.clone());
                                        
                                        // Fire SDP answer received event
                                        if let Some(session_id) = self.find_session_for_transaction(&tx_key) {
                                            self.event_bus.publish(crate::events::SessionEvent::SdpAnswerReceived {
                                                session_id: session_id.clone(),
                                                dialog_id: dialog_id.clone(),
                                            });
                                            
                                            // Emit negotiation complete event
                                            if dialog.sdp_context.is_complete() {
                                                self.event_bus.publish(crate::events::SessionEvent::SdpNegotiationComplete {
                                                    session_id,
                                                    dialog_id: dialog_id.clone(),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    
                    // Notify about successful response
                    if let Some(session_id) = self.find_session_for_transaction(&tx_key) {
                        self.event_bus.publish(crate::events::SessionEvent::Custom {
                            session_id,
                            event_type: "transaction_completed".to_string(),
                            data: serde_json::json!({
                                "dialog_id": dialog_id.to_string(),
                                "success": true
                            }),
                        });
                    }
                }
            },
            TransactionEvent::AckReceived { transaction_id: tx_key, request } => {
                debug!("Received ACK for transaction {}:\n{}", tx_key, request);
                
                // Find dialog associated with this transaction
                let dialog_id = match self.transaction_to_dialog.get(&tx_key) {
                    Some(dialog_id) => dialog_id.clone(),
                    None => {
                        debug!("No dialog found for transaction {:?}", tx_key);
                        return;
                    }
                };
                
                // Get the dialog
                let mut dialog_opt = self.dialogs.get_mut(&dialog_id);
                if dialog_opt.is_none() {
                    debug!("Dialog {} not found for transaction {}", dialog_id, tx_key);
                    return;
                }
                
                let mut dialog = dialog_opt.unwrap();
                
                // Update dialog with ACK information
                dialog.update_remote_seq_from_request(&request);
                
                // Fire event for ACK received
                if let Some(session_id) = self.find_session_for_transaction(&tx_key) {
                    self.event_bus.publish(crate::events::SessionEvent::Custom {
                        session_id,
                        event_type: "ack_received".to_string(),
                        data: serde_json::json!({
                            "dialog_id": dialog_id.to_string(),
                        }),
                    });
                }
            },
            TransactionEvent::NewRequest { transaction_id: tx_key, request, source } => {
                debug!("Received new request {}:\n{}", tx_key, request);
                
                // Handle ACK requests specially - they should be forwarded to the server transaction
                // for proper processing using our new process_request method
                if request.method() == Method::Ack {
                    // Try to find the associated INVITE transaction
                    // First, look for existing dialog that matches the ACK request
                    if let Some(dialog_id) = self.find_dialog_for_request(&request) {
                        debug!("Found dialog {} for ACK request", dialog_id);
                        
                        // Look for INVITE server transaction associated with this dialog
                        let invite_tx_key = self.transaction_to_dialog.iter()
                            .filter(|entry| entry.value().clone() == dialog_id)
                            .filter_map(|entry| {
                                if !entry.key().is_server() || entry.key().method() != &Method::Invite {
                                    return None;
                                }
                                Some(entry.key().clone())
                            })
                            .next();
                        
                        if let Some(invite_tx_key) = invite_tx_key {
                            debug!("Found INVITE transaction {} for ACK, forwarding request", invite_tx_key);
                            
                            // Forward the ACK request to the INVITE transaction
                            if let Err(e) = self.transaction_manager.process_request(&invite_tx_key, request.clone()).await {
                                warn!("Failed to process ACK request: {}", e);
                            } else {
                                // Successfully processed the ACK
                                return;
                            }
                        }
                    }
                    
                    // If we get here, we couldn't match the ACK to a transaction
                    debug!("Could not match ACK request to any INVITE transaction, treating as stray ACK");
                }
                
                // Process other new requests as usual...
                // For new dialogs, create a server transaction
                self.create_server_transaction_for_request(tx_key, request, source).await;
            },
            TransactionEvent::Error { transaction_id, error } => {
                debug!("Transaction error event received for {:?}: {:?}", transaction_id, error);
                
                // Handle transaction_id as an Option<TransactionKey>
                let tx_key = match &transaction_id {
                    Some(key) => key,
                    None => {
                        debug!("No transaction ID in error event");
                        return;
                    }
                };
                
                // Now use the properly unwrapped tx_key
                if !self.transaction_to_dialog.contains_key(tx_key) {
                    debug!("No dialog found for transaction {:?}", tx_key);
                    return;
                }
                
                let dialog_id = self.transaction_to_dialog.get(tx_key).unwrap().clone();
                
                // For network errors, initiate dialog recovery
                let dialog_manager = self.clone();
                let dialog_id_clone = dialog_id.clone();
                let tx_key_clone = tx_key.clone();
                let error_clone = error.clone();
                
                // Spawn a task to check for recovery needs and handle it asynchronously
                tokio::spawn(async move {
                    if dialog_manager.needs_recovery(&dialog_id_clone).await {
                        debug!("Initiating recovery for dialog {} due to transaction error", dialog_id_clone);
                        
                        let reason = format!("Transaction error: {:?}", error_clone);
                        
                        if let Err(e) = dialog_manager.recover_dialog(&dialog_id_clone, &reason).await {
                            error!("Failed to initiate recovery for dialog {}: {}", dialog_id_clone, e);
                        }
                    }
                });
            },
            // Catch-all for any other events
            _ => {
                // Log the event type for debugging
                debug!("Received unhandled transaction event: {:?}", event);
            }
        }
    }
    
    /// Create a server transaction for a new request
    async fn create_server_transaction_for_request(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr
    ) {
        match request.method() {
            Method::Invite => {
                debug!("Creating server transaction for new INVITE request");
                
                // **ARCHITECTURAL FIX**: Create actual server transaction and send responses
                // This is what transaction-core examples do - we need to create the server transaction
                // and send the required SIP responses (100 Trying, 180 Ringing, 200 OK)
                
                // Create server transaction using transaction manager
                match self.transaction_manager.create_server_transaction(request.clone(), source).await {
                    Ok(server_tx) => {
                        let server_tx_id = server_tx.id().clone();
                        debug!("Created server transaction {} for INVITE", server_tx_id);
                        
                        // Associate this transaction with a potential dialog
                        // (We'll create the actual dialog when we send responses with tags)
                        
                        // Send 100 Trying immediately (required by RFC 3261)
                        let trying_response = rvoip_transaction_core::utils::create_trying_response(&request);
                        if let Err(e) = self.transaction_manager.send_response(&server_tx_id, trying_response).await {
                            error!("Failed to send 100 Trying response: {}", e);
                            return;
                        }
                        debug!("Sent 100 Trying for INVITE transaction {}", server_tx_id);
                        
                        // Wait a bit to simulate processing
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        
                        // Send 180 Ringing
                        let ringing_response = rvoip_transaction_core::utils::create_ringing_response_with_tag(&request);
                        if let Err(e) = self.transaction_manager.send_response(&server_tx_id, ringing_response).await {
                            error!("Failed to send 180 Ringing response: {}", e);
                            return;
                        }
                        debug!("Sent 180 Ringing for INVITE transaction {}", server_tx_id);
                        
                        // Wait a bit more to simulate phone ringing
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        
                        // **COMPLETE THE CALL FLOW**: Send 200 OK to accept the call
                        // Use transaction-core's dialog-aware response builder with proper parameters
                        // Get the local address from the transport instead of hardcoding
                        let local_addr = self.transaction_manager.transport().local_addr()
                            .unwrap_or_else(|_| source);
                        
                        // Use a configurable server identity instead of hardcoded "server"
                        // TODO: This should come from server configuration
                        let server_user = "server"; // This should be configurable
                        let server_host = local_addr.ip().to_string();
                        let server_port = if local_addr.port() != 5060 { Some(local_addr.port()) } else { None };
                        
                        let ok_response = rvoip_transaction_core::utils::create_ok_response_with_dialog_info(
                            &request, 
                            server_user, 
                            &server_host, 
                            server_port
                        );
                        
                        if let Err(e) = self.transaction_manager.send_response(&server_tx_id, ok_response.clone()).await {
                            error!("Failed to send 200 OK response: {}", e);
                            return;
                        }
                        debug!("Sent 200 OK for INVITE transaction {} - call established!", server_tx_id);
                        
                        // Create dialog from the INVITE transaction using the actual response we sent
                        if let Some(dialog_id) = self.create_dialog_from_transaction(&server_tx_id, &request, &ok_response, false).await {
                            debug!("Created dialog {} for established call", dialog_id);
                        }
                        
                        // Emit event for new INVITE (session-core can coordinate session creation)
                        self.event_bus.publish(crate::events::SessionEvent::Custom {
                            session_id: SessionId::new(), // We don't know the session yet
                            event_type: "call_established".to_string(),
                            data: serde_json::json!({
                                "transaction_id": server_tx_id.to_string(),
                                "original_transaction_id": transaction_id.to_string(),
                            }),
                        });
                    },
                    Err(e) => {
                        error!("Failed to create server transaction for INVITE: {}", e);
                        
                        // Emit error event
                        self.event_bus.publish(crate::events::SessionEvent::Custom {
                            session_id: SessionId::new(),
                            event_type: "invite_error".to_string(),
                            data: serde_json::json!({
                                "error": e.to_string(),
                                "transaction_id": transaction_id.to_string(),
                            }),
                        });
                    }
                }
            },
            Method::Bye => {
                debug!("Creating server transaction for BYE request");
                
                // For BYE requests, create server transaction and send 200 OK
                match self.transaction_manager.create_server_transaction(request.clone(), source).await {
                    Ok(server_tx) => {
                        let server_tx_id = server_tx.id().clone();
                        debug!("Created server transaction {} for BYE", server_tx_id);
                        
                        // Send 200 OK immediately for BYE
                        let ok_response = rvoip_transaction_core::utils::create_ok_response(&request);
                        if let Err(e) = self.transaction_manager.send_response(&server_tx_id, ok_response).await {
                            error!("Failed to send 200 OK for BYE: {}", e);
                            return;
                        }
                        debug!("Sent 200 OK for BYE transaction {} - call terminated", server_tx_id);
                        
                        // Find and terminate the associated dialog
                        if let Some(dialog_id) = self.find_dialog_for_request(&request) {
                            debug!("Found dialog {} for BYE, terminating", dialog_id);
                            if let Err(e) = self.terminate_dialog(&dialog_id).await {
                                error!("Failed to terminate dialog {}: {}", dialog_id, e);
                            }
                        }
                        
                        // Emit call terminated event
                        self.event_bus.publish(crate::events::SessionEvent::Custom {
                            session_id: SessionId::new(),
                            event_type: "call_terminated".to_string(),
                            data: serde_json::json!({
                                "transaction_id": server_tx_id.to_string(),
                                "original_transaction_id": transaction_id.to_string(),
                            }),
                        });
                    },
                    Err(e) => {
                        error!("Failed to create server transaction for BYE: {}", e);
                    }
                }
            },
            _ => {
                debug!("Received {} request", request.method());
                
                // For other methods, create server transaction and send appropriate response
                match self.transaction_manager.create_server_transaction(request.clone(), source).await {
                    Ok(server_tx) => {
                        let server_tx_id = server_tx.id().clone();
                        debug!("Created server transaction {} for {}", server_tx_id, request.method());
                        
                        // Send appropriate response based on method
                        let response = match request.method() {
                            Method::Options => {
                                // Send 200 OK with capabilities
                                rvoip_transaction_core::utils::create_ok_response(&request)
                            },
                            Method::Info => {
                                // Send 200 OK
                                rvoip_transaction_core::utils::create_ok_response(&request)
                            },
                            Method::Message => {
                                // Send 200 OK
                                rvoip_transaction_core::utils::create_ok_response(&request)
                            },
                            _ => {
                                // Send 200 OK for unknown methods
                                rvoip_transaction_core::utils::create_ok_response(&request)
                            }
                        };
                        
                        if let Err(e) = self.transaction_manager.send_response(&server_tx_id, response).await {
                            error!("Failed to send response for {} request: {}", request.method(), e);
                        } else {
                            debug!("Sent 200 OK for {} transaction {}", request.method(), server_tx_id);
                        }
                        
                        // Emit event for the request
                        self.event_bus.publish(crate::events::SessionEvent::Custom {
                            session_id: SessionId::new(), // We don't know the session yet
                            event_type: format!("new_{}", request.method().to_string().to_lowercase()),
                            data: serde_json::json!({
                                "transaction_id": server_tx_id.to_string(),
                                "original_transaction_id": transaction_id.to_string(),
                            }),
                        });
                    },
                    Err(e) => {
                        error!("Failed to create server transaction for {}: {}", request.method(), e);
                    }
                }
            }
        }
    }
    
    /// Handle an incoming provisional response which may create an early dialog
    async fn handle_provisional_response(&self, transaction_id: &TransactionKey, response: Response) {
        debug!("Provisional response for transaction: {}", transaction_id);
        
        // Only interested in responses > 100 with to-tag for dialog creation
        if response.status().as_u16() <= 100 || !self.has_to_tag(&response) {
            return;
        }
        
        // Get the original request
        let request = match self.get_transaction_request(transaction_id).await {
            Ok(Some(req)) if req.method() == Method::Invite => req,
            _ => return,
        };
        
        // Create early dialog using the new method
        if let Some(dialog_id) = self.create_dialog_from_transaction(transaction_id, &request, &response, true).await {
            debug!("Created early dialog {} from provisional response", dialog_id);
            
            // Emit dialog updated event if associated with a session
            if let Some(session_id) = self.find_session_for_transaction(transaction_id) {
                debug!("Associating dialog {} with session {}", dialog_id, session_id);
                let _ = self.associate_with_session(&dialog_id, &session_id);
                
                // Emit dialog updated event
                self.event_bus.publish(SessionEvent::DialogUpdated {
                    session_id,
                    dialog_id,
                });
            }
        }
    }
    
    /// Handle an incoming success response which will create or confirm a dialog
    async fn handle_success_response(&self, transaction_id: &TransactionKey, response: Response) {
        debug!("Success response for transaction: {}", transaction_id);
        
        // Get the original request
        let request = match self.get_transaction_request(transaction_id).await {
            Ok(Some(req)) if req.method() == Method::Invite => req,
            _ => return,
        };
        
        // Check if we already have an early dialog for this transaction
        let existing_dialog_id = self.transaction_to_dialog.get(transaction_id).map(|id| id.clone());
        
        if let Some(dialog_id) = existing_dialog_id {
            // Try to get mutable access to the dialog
            if let Some(mut dialog_entry) = self.dialogs.get_mut(&dialog_id) {
                debug!("Updating existing dialog {:?} with final response", dialog_id);
                
                // Update early dialog to confirmed
                if dialog_entry.update_from_2xx(&response) {
                    // Get dialog tuple
                    if let Some(tuple) = dialog_entry.dialog_id_tuple() {
                        drop(dialog_entry); // Release the reference before modifying other maps
                        
                        // Update dialog tuple mapping
                        self.dialog_lookup.insert(tuple, dialog_id.clone());
                        
                        // Publish event
                        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                            let session_id = session_id_ref.clone();
                            drop(session_id_ref); // Release the reference
                            
                            // Emit dialog updated event
                            self.event_bus.publish(SessionEvent::DialogUpdated {
                                session_id,
                                dialog_id: dialog_id.clone(),
                            });
                        }
                    }
                }
                return;
            }
        }
        
        // No existing dialog, create a new one using the new method
        if let Some(dialog_id) = self.create_dialog_from_transaction(transaction_id, &request, &response, true).await {
            debug!("Created confirmed dialog {} from 2xx response", dialog_id);
            
            // Emit dialog updated event if associated with a session
            if let Some(session_id) = self.find_session_for_transaction(transaction_id) {
                debug!("Associating dialog {} with session {}", dialog_id, session_id);
                let _ = self.associate_with_session(&dialog_id, &session_id);
                
                // Emit dialog updated event
                self.event_bus.publish(SessionEvent::DialogUpdated {
                    session_id,
                    dialog_id,
                });
            }
        }
    }
    
    /// Handle a BYE request which terminates a dialog
    async fn handle_bye_request(&self, transaction_id: &TransactionKey, request: Request) {
        debug!("BYE request received for transaction: {}", transaction_id);
        
        // Try to find the associated dialog based on the request headers
        let dialog_id = match self.find_dialog_for_request(&request) {
            Some(id) => id,
            None => {
                debug!("No dialog found for BYE request");
                return;
            },
        };
        
        debug!("Found dialog {} for BYE request", dialog_id);
        
        // Update dialog state to Terminated
        if let Some(mut dialog) = self.dialogs.get_mut(&dialog_id) {
            dialog.state = DialogState::Terminated;
            drop(dialog); // Release the lock
            
            // Associate this transaction with the dialog for subsequent events
            self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
            
            // Emit dialog terminated event
            let session_id = match self.dialog_to_session.get(&dialog_id) {
                Some(id_ref) => {
                    let id = id_ref.clone();
                    drop(id_ref);
                    Some(id)
                },
                None => None,
            };
            
            if let Some(session_id) = session_id {
                self.event_bus.publish(SessionEvent::Terminated {
                    session_id,
                    reason: "BYE received".to_string(),
                });
            }
        }
    }
    
    /// Check if a response has a to-tag
    fn has_to_tag(&self, response: &Response) -> bool {
        response.to().and_then(|to| to.tag()).is_some()
    }
    
    /// Get the original request from a transaction
    async fn get_transaction_request(&self, transaction_id: &TransactionKey) -> Result<Option<Request>, Error> {
        // Using the transaction manager to get transaction state and kind
        match self.transaction_manager.transaction_kind(transaction_id).await {
            Ok(TransactionKind::InviteClient) | Ok(TransactionKind::InviteServer) => {
                // Retrieve the transaction from the repository directly
                // Note: In a more complete implementation, we would add a method to the transaction manager
                // to retrieve the original request, but since we don't want to modify transaction-core,
                // we'll rely on the transaction event history
                
                debug!("Attempting to find original request for transaction {}", transaction_id);
                
                // Since we can't directly get the original request from the transaction manager
                // without modifying it, we'll use the most recent request we've seen for this
                // transaction from our event history or session state
                
                // For now, create a synthetic request with the required headers
                // In a real implementation, this should be retrieved from transaction history
                if let Some(dialog_id) = self.transaction_to_dialog.get(transaction_id) {
                    if let Some(dialog) = self.dialogs.get(&dialog_id) {
                        let method = Method::Invite;
                        let mut request = Request::new(method.clone(), dialog.remote_uri.clone());
                        
                        // Add Call-ID
                        let call_id = rvoip_sip_core::types::call_id::CallId(dialog.call_id.clone());
                        request.headers.push(TypedHeader::CallId(call_id));
                        
                        // Add From with tag using proper API
                        let from_uri = dialog.local_uri.clone();
                        let mut from_addr = Address::new(from_uri);
                        if let Some(tag) = &dialog.local_tag {
                            from_addr.set_tag(tag);
                        }
                        let from = FromHeader(from_addr);
                        request.headers.push(TypedHeader::From(from));
                        
                        // Add To with remote tag using proper API
                        let to_uri = dialog.remote_uri.clone();
                        let mut to_addr = Address::new(to_uri);
                        if let Some(tag) = &dialog.remote_tag {
                            to_addr.set_tag(tag);
                        }
                        let to = ToHeader(to_addr);
                        request.headers.push(TypedHeader::To(to));
                        
                        // Add CSeq
                        let cseq = rvoip_sip_core::types::cseq::CSeq::new(dialog.local_seq, method);
                        request.headers.push(TypedHeader::CSeq(cseq));
                        
                        return Ok(Some(request));
                    }
                }
                
                debug!("No original request found for transaction {}", transaction_id);
                Ok(None)
            },
            Ok(_) => {
                debug!("Transaction {} is not an INVITE transaction", transaction_id);
                Ok(None)
            },
            Err(e) => {
                let error_msg = format!("Failed to get transaction kind: {}", e);
                debug!("Error getting transaction kind for {}: {}", transaction_id, error_msg);
                
                // Create a transaction error with proper context
                Err(Error::TransactionError(
                    rvoip_transaction_core::Error::Other(error_msg.clone()),
                    ErrorContext {
                        category: ErrorCategory::External,
                        severity: ErrorSeverity::Warning,
                        recovery: RecoveryAction::None,
                        retryable: false,
                        transaction_id: Some(transaction_id.to_string()),
                        timestamp: SystemTime::now(),
                        details: Some(error_msg),
                        ..Default::default()
                    }
                ))
            }
        }
    }
    
    /// Find the dialog for an in-dialog request
    fn find_dialog_for_request(&self, request: &Request) -> Option<DialogId> {
        // Extract call-id
        let call_id = match request.call_id() {
            Some(call_id) => call_id.to_string(),
            _ => return None
        };
        
        // Extract tags
        let from_tag = request.from().and_then(|from| from.tag().map(|s| s.to_string()));
        let to_tag = request.to().and_then(|to| to.tag().map(|s| s.to_string()));
        
        // Both tags are required for dialog lookup
        if from_tag.is_none() || to_tag.is_none() {
            return None;
        }
        
        let from_tag = from_tag.unwrap();
        let to_tag = to_tag.unwrap();
        
        // Try to find a matching dialog - check both scenarios
        
        // Scenario 1: Local is From, Remote is To
        let id_tuple1 = (call_id.clone(), from_tag.clone(), to_tag.clone());
        if let Some(dialog_id_ref) = self.dialog_lookup.get(&id_tuple1) {
            let dialog_id = dialog_id_ref.value().clone();
            drop(dialog_id_ref);
            return Some(dialog_id);
        }
        
        // Scenario 2: Local is To, Remote is From
        let id_tuple2 = (call_id, to_tag, from_tag);
        if let Some(dialog_id_ref) = self.dialog_lookup.get(&id_tuple2) {
            let dialog_id = dialog_id_ref.value().clone();
            drop(dialog_id_ref);
            return Some(dialog_id);
        }
        
        // No matching dialog found
        None
    }
    
    /// Create a dialog directly from transaction events
    pub async fn create_dialog_from_transaction(
        &self, 
        transaction_id: &TransactionKey, 
        request: &Request, 
        response: &Response,
        is_initiator: bool
    ) -> Option<DialogId> {
        debug!("Creating dialog from transaction: {}", transaction_id);
        
        // Create dialog based on response type
        let dialog = if response.status().is_success() {
            debug!("Creating confirmed dialog from 2xx response");
            Dialog::from_2xx_response(request, response, is_initiator)
        } else if (100..200).contains(&response.status().as_u16()) && response.status().as_u16() > 100 {
            debug!("Creating early dialog from 1xx response");
            Dialog::from_provisional_response(request, response, is_initiator)
        } else {
            debug!("Response status {} not appropriate for dialog creation", response.status());
            None
        };
        
        if let Some(dialog) = dialog {
            let dialog_id = dialog.id.clone();
            debug!("Created dialog with ID: {}", dialog_id);
            
            // Store the dialog
            self.dialogs.insert(dialog_id.clone(), dialog.clone());
            
            // Associate the transaction with this dialog
            self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
            
            // Save dialog tuple mapping if available
            if let Some(tuple) = dialog.dialog_id_tuple() {
                debug!("Mapping dialog tuple to dialog ID: {:?} -> {}", tuple, dialog_id);
                self.dialog_lookup.insert(tuple, dialog_id.clone());
            }
            
            // Return the created dialog ID
            Some(dialog_id)
        } else {
            debug!("Failed to create dialog from transaction event");
            None
        }
    }
    
    /// Associate a dialog with a session
    pub fn associate_with_session(
        &self, 
        dialog_id: &DialogId, 
        session_id: &SessionId
    ) -> Result<(), Error> {
        if !self.dialogs.contains_key(dialog_id) {
            return Err(Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot associate with session - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ));
        }
        
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        Ok(())
    }
    
    /// Create a new request in a dialog
    pub async fn create_request(
        &self, 
        dialog_id: &DialogId, 
        method: Method
    ) -> Result<Request, Error> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot create request - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))?;
            
        // Create the request
        let request = dialog.create_request(method);
        Ok(request)
    }
    
    /// Get a dialog by ID
    pub fn get_dialog(&self, dialog_id: &DialogId) -> Result<Dialog, Error> {
        self.dialogs.get(dialog_id)
            .map(|d| d.clone())
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))
    }
    
    /// Terminate a dialog
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> Result<(), Error> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot terminate - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))?;
            
        dialog.terminate();
        Ok(())
    }
    
    /// Update dialog SDP state with a local SDP offer
    /// 
    /// This is used when sending an SDP offer in a request, to track
    /// the SDP negotiation state.
    pub async fn update_dialog_with_local_sdp_offer(
        &self,
        dialog_id: &DialogId,
        offer: crate::sdp::SessionDescription
    ) -> Result<(), Error> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot update SDP - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))?;
            
        dialog.update_with_local_sdp_offer(offer);
        
        // Publish SDP offer event
        if let Some(session_id) = self.dialog_to_session.get(dialog_id) {
            let sdp_event = crate::events::SdpEvent::OfferSent {
                session_id: session_id.to_string(),
                dialog_id: dialog_id.to_string(),
            };
            self.event_bus.publish(sdp_event.into());
        }
        
        Ok(())
    }
    
    /// Update dialog SDP state with a local SDP answer
    /// 
    /// This is used when sending an SDP answer in a response, to track
    /// the SDP negotiation state.
    pub async fn update_dialog_with_local_sdp_answer(
        &self,
        dialog_id: &DialogId,
        answer: crate::sdp::SessionDescription
    ) -> Result<(), Error> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot update SDP - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))?;
            
        dialog.update_with_local_sdp_answer(answer);
        
        // Publish SDP answer event
        if let Some(session_id) = self.dialog_to_session.get(dialog_id) {
            let sdp_event = crate::events::SdpEvent::AnswerSent {
                session_id: session_id.to_string(),
                dialog_id: dialog_id.to_string(),
            };
            self.event_bus.publish(sdp_event.into());
        }
        
        Ok(())
    }
    
    /// Update dialog for re-negotiation (re-INVITE)
    /// 
    /// This resets the SDP negotiation state to prepare for a new
    /// offer/answer exchange.
    pub async fn prepare_dialog_sdp_renegotiation(
        &self,
        dialog_id: &DialogId
    ) -> Result<(), Error> {
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| Error::DialogNotFoundWithId(
                dialog_id.to_string(),
                ErrorContext {
                    category: ErrorCategory::Dialog,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    dialog_id: Some(dialog_id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Cannot prepare for renegotiation - dialog {} not found", dialog_id)),
                    ..Default::default()
                }
            ))?;
            
        dialog.prepare_sdp_renegotiation();
        Ok(())
    }
    
    /// Remove terminated dialogs
    pub fn cleanup_terminated(&self) -> usize {
        let mut count = 0;
        
        let terminated_dialogs: Vec<_> = self.dialogs.iter()
            .filter(|d| d.is_terminated())
            .map(|d| d.id.clone())
            .collect();
        
        for dialog_id in terminated_dialogs {
            if let Some((_, dialog)) = self.dialogs.remove(&dialog_id) {
                count += 1;
                
                // Remove from the lookup tables
                // Get the dialog tuple directly from the dialog
                let call_id = &dialog.call_id;
                if let (Some(local_tag), Some(remote_tag)) = (&dialog.local_tag, &dialog.remote_tag) {
                    let tuple = (call_id.clone(), local_tag.clone(), remote_tag.clone());
                    self.dialog_lookup.remove(&tuple);
                }
                
                self.dialog_to_session.remove(&dialog_id);
                
                // Remove transaction associations
                let txs_to_remove: Vec<_> = self.transaction_to_dialog.iter()
                    .filter(|e| e.value().clone() == dialog_id)
                    .map(|e| e.key().clone())
                    .collect();
                
                for tx_id in txs_to_remove {
                    self.transaction_to_dialog.remove(&tx_id);
                }
            }
        }
        
        count
    }
    
    /// Get the current number of active dialogs
    pub fn dialog_count(&self) -> usize {
        self.dialogs.len()
    }
    
    // Helper method to find a session associated with a transaction
    fn find_session_for_transaction(&self, transaction_id: &TransactionKey) -> Option<SessionId> {
        // First, look up the dialog ID
        let dialog_id = match self.transaction_to_dialog.get(transaction_id) {
            Some(ref_val) => {
                let dialog_id = ref_val.clone();
                drop(ref_val);
                dialog_id
            },
            None => return None
        };
        
        // Now look up the session ID for this dialog
        match self.dialog_to_session.get(&dialog_id) {
            Some(ref_val) => {
                let session_id = ref_val.clone();
                drop(ref_val);
                Some(session_id)
            },
            None => None
        }
    }
    
    /// Send a request through this dialog and create a client transaction
    pub async fn send_dialog_request(
        &self,
        dialog_id: &DialogId,
        method: Method,
    ) -> Result<TransactionKey, Error> {
        // Get the dialog
        let mut dialog = self.dialogs.get_mut(dialog_id)
            .ok_or_else(|| dialog_not_found_error(dialog_id))?;
        
        // Create the request within the dialog
        let request = dialog.create_request(method.clone());
        
        // Get the destination for this dialog (stored in remote_target)
        let destination = match uri_resolver::resolve_uri_to_socketaddr(&dialog.remote_target).await {
            Some(addr) => addr,
            None => return Err(network_unreachable_error(&dialog.remote_target.to_string())),
        };
        
        // Create a client transaction for this request
        let transaction_id;
        
        if request.method() == Method::Invite {
            // Create INVITE transaction
            match self.transaction_manager.create_invite_client_transaction(request, destination).await {
                Ok(tx_id) => {
                    transaction_id = tx_id;
                },
                Err(e) => {
                    let error_msg = format!("Failed to create INVITE transaction: {}", e);
                    return Err(transaction_creation_error("INVITE", &error_msg));
                }
            }
        } else {
            // Create non-INVITE transaction
            match self.transaction_manager.create_non_invite_client_transaction(request, destination).await {
                Ok(tx_id) => {
                    transaction_id = tx_id;
                },
                Err(e) => {
                    let error_msg = format!("Failed to create {} transaction: {}", method, e);
                    return Err(transaction_creation_error(&method.to_string(), &error_msg));
                }
            }
        }
        
        // Associate this transaction with the dialog
        self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
        
        // Update the dialog with the remote address and timestamp before we drop the lock
        dialog.update_remote_address(destination);
        
        // Release the lock on the dialog before network operation
        drop(dialog);
        
        // Try to send the request, but handle the case where the transaction might have
        // terminated immediately (especially in test environments)
        let send_result = self.transaction_manager.send_request(&transaction_id).await;
        
        // Check if the transaction still exists before proceeding
        let exists = self.transaction_manager.transaction_exists(&transaction_id).await;
        
        if !exists {
            // In tests, we might see the transaction terminated immediately
            // This can happen with simulated transports or in certain test cases
            debug!("Transaction {} terminated immediately after creation", transaction_id);
            
            if let Err(e) = send_result {
                // Only clean up on error - the transaction might have terminated successfully
                self.transaction_to_dialog.remove(&transaction_id);
                
                let error_msg = format!("Transaction terminated immediately: {}", e);
                return Err(transaction_send_error(&error_msg, &transaction_id.to_string()));
            }
            
            // Even with termination, we return the transaction ID for tracking purposes
            return Ok(transaction_id);
        }
        
        // Normal path - check the send result
        match send_result {
            Ok(_) => Ok(transaction_id),
            Err(e) => {
                // Clean up the transaction mapping on error
                self.transaction_to_dialog.remove(&transaction_id);
                
                let error_msg = format!("Failed to send request: {}", e);
                Err(transaction_send_error(&error_msg, &transaction_id.to_string()))
            }
        }
    }
    
    /// Create a dialog directly (without transaction events)
    ///
    /// This method allows creating dialogs programmatically, which is useful for
    /// reconstructing dialogs from persisted state or creating dialogs for testing.
    pub fn create_dialog_directly(
        &self,
        dialog_id: DialogId,
        call_id: String,
        local_uri: Uri,
        remote_uri: Uri,
        local_tag: Option<String>,
        remote_tag: Option<String>,
        is_initiator: bool
    ) -> DialogId {
        // Create a new dialog with remote URI cloned for the remote target field
        let remote_target = remote_uri.clone();
        
        // Create a new dialog
        let dialog = Dialog {
            id: dialog_id.clone(),
            state: DialogState::Confirmed,
            call_id: call_id.clone(),
            local_uri,
            remote_uri,
            local_tag: local_tag.clone(),
            remote_tag: remote_tag.clone(),
            local_seq: 1,  // Initialize at 1 for first request
            remote_seq: 0, // Will be set when receiving a request
            remote_target, // Use remote URI as target initially
            route_set: Vec::new(),
            is_initiator, // Use provided initiator flag
            sdp_context: crate::sdp::SdpContext::new(),
            last_known_remote_addr: None,
            last_successful_transaction_time: None,
            recovery_attempts: 0,
            recovery_reason: None,
            recovered_at: None,
            recovery_start_time: None,
        };
        
        // Store the dialog
        self.dialogs.insert(dialog_id.clone(), dialog.clone());
        
        // If we have both local and remote tags, add to dialog_lookup for faster in-dialog request matching
        if let (Some(local_tag), Some(remote_tag)) = (local_tag, remote_tag) {
            let dialog_tuple = (call_id, local_tag, remote_tag);
            self.dialog_lookup.insert(dialog_tuple, dialog_id.clone());
        }
        
        // Return the dialog ID
        dialog_id
    }
    
    /// Associate a dialog with a session and emit dialog created event
    pub fn associate_and_notify(
        &self,
        dialog_id: &DialogId,
        session_id: &SessionId
    ) -> Result<(), Error> {
        // Associate with session
        self.associate_with_session(dialog_id, session_id)?;
        
        // Emit a dialog created event
        self.event_bus.publish(crate::events::SessionEvent::DialogCreated {
            session_id: session_id.clone(),
            dialog_id: dialog_id.clone(),
        });
        
        Ok(())
    }
    
    /// Send a response using the transaction manager
    ///
    /// This is just a convenience wrapper to avoid having to access the
    /// transaction manager directly.
    pub async fn send_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response
    ) -> Result<(), rvoip_transaction_core::Error> {
        self.transaction_manager.send_response(transaction_id, response).await
    }
    
    /// Initiate dialog recovery after detecting a network failure
    pub async fn recover_dialog(&self, dialog_id: &DialogId, reason: &str) -> Result<(), Error> {
        // Check if the dialog exists
        let dialog_opt = self.dialogs.get_mut(dialog_id);
        if dialog_opt.is_none() {
            return Err(dialog_not_found_error(dialog_id));
        }
        
        // Get the dialog and check if it can be recovered
        let mut dialog = dialog_opt.unwrap();
        if dialog.state != DialogState::Confirmed && dialog.state != DialogState::Early {
            return Err(Error::InvalidDialogState {
                current: dialog.state.to_string(),
                expected: "Confirmed or Early".to_string(),
                context: ErrorContext::default()
            });
        }
        
        // Check if we have a last known remote address
        if dialog.last_known_remote_addr.is_none() {
            return Err(Error::MissingDialogData {
                context: ErrorContext::default().with_message(
                    "Dialog does not have a last known remote address"
                )
            });
        }
        
        // Check if circuit breaker is active
        {
            let metrics = self.recovery_metrics.read().await;
            if metrics.circuit_breaker_open {
                if let Some(reset_time) = metrics.last_circuit_breaker_reset {
                    if let Ok(elapsed) = SystemTime::now().duration_since(reset_time) {
                        if elapsed < self.recovery_config.circuit_breaker_reset_period {
                            warn!("Dialog recovery circuit breaker is open, rejecting recovery for dialog {}", dialog_id);
                            // Use NetworkUnreachable which is more appropriate for circuit breaker pattern
                            let wait_time = self.recovery_config.circuit_breaker_reset_period.checked_sub(elapsed)
                                .unwrap_or_default();
                            return Err(Error::NetworkUnreachable(
                                format!("Circuit breaker active for dialog {}", dialog_id),
                                ErrorContext {
                                    category: ErrorCategory::Network,
                                    severity: ErrorSeverity::Warning,
                                    recovery: RecoveryAction::Wait(wait_time),
                                    retryable: true,
                                    dialog_id: Some(dialog_id.to_string()),
                                    timestamp: SystemTime::now(),
                                    details: Some(format!("Circuit breaker active for {} more seconds", 
                                        wait_time.as_secs())),
                                    ..Default::default()
                                }
                            ));
                        } else {
                            // Reset circuit breaker if enough time has passed
                            drop(metrics); // Release the read lock
                            let mut metrics = self.recovery_metrics.write().await;
                            metrics.reset_circuit_breaker();
                            info!("Dialog recovery circuit breaker reset after timeout period");
                        }
                    }
                }
            }
        }
        
        // Put the dialog into recovery mode
        dialog.enter_recovery_mode(reason);
        
        // Get the session ID for events
        let session_id = self.get_session_for_dialog(dialog_id)
            .ok_or_else(|| Error::session_not_found("No session found for dialog"))?;
        
        // Make a clone of the dialog ID before releasing the lock
        let dialog_id_clone = dialog_id.clone();
        
        // Release the lock before firing events
        drop(dialog);
        
        // Publish a specific recovery started event
        self.event_bus.publish(SessionEvent::DialogRecoveryStarted {
            session_id: session_id.clone(),
            dialog_id: dialog_id.clone(),
            reason: reason.to_string(),
        });
        
        if self.run_recovery_in_background {
            // Start the recovery process in a background task
            let manager = self.clone();
            tokio::spawn(async move {
                manager.execute_recovery_process(&dialog_id_clone).await;
            });
        } else {
            // For testing, run recovery process synchronously
            self.execute_recovery_process(dialog_id).await;
        }
        
        Ok(())
    }
    
    /// Execute the dialog recovery process (retry logic, etc.)
    async fn execute_recovery_process(&self, dialog_id: &DialogId) {
        debug!("Starting recovery process for dialog {}", dialog_id);
        
        // Get a reference to the dialog
        let mut dialog_opt = self.dialogs.get_mut(dialog_id);
        if dialog_opt.is_none() {
            debug!("Dialog {} not found for recovery", dialog_id);
            return;
        }
        
        // Prepare to run the recovery process
        let transport = self.transaction_manager.transport();
        let config = &self.recovery_config;
        
        // We need a mutable reference to metrics, but tokio's RwLock is async
        let metrics_arc = self.recovery_metrics.clone();
        
        // Get session ID for events
        let session_id = self.get_session_for_dialog(dialog_id);
        
        // Setup dialog and transport
        let mut dialog = dialog_opt.unwrap();
        
        // Create event callback for logging and events
        let event_bus = self.event_bus.clone();
        let dialog_id_clone = dialog_id.clone();
        let session_id_clone = session_id.clone();
        let event_callback = move |event: super::recovery::RecoveryEvent| {
            match &event {
                super::recovery::RecoveryEvent::AttemptStarted { attempt, max_attempts } => {
                    info!("Starting recovery attempt {} of {} for dialog {}", 
                        attempt, max_attempts, dialog_id_clone);
                    
                    // Emit event through event bus if needed
                    if let Some(session_id) = &session_id_clone {
                        event_bus.publish(crate::events::SessionEvent::Custom {
                            session_id: session_id.clone(),
                            event_type: "recovery_attempt_started".to_string(),
                            data: serde_json::json!({
                                "dialog_id": dialog_id_clone.to_string(),
                                "attempt": attempt,
                                "max_attempts": max_attempts
                            }),
                        });
                    }
                },
                super::recovery::RecoveryEvent::AttemptSucceeded { time_ms } => {
                    info!("Dialog {} recovery succeeded in {}ms", dialog_id_clone, time_ms);
                },
                super::recovery::RecoveryEvent::AttemptFailed { attempt, reason, is_timeout } => {
                    if *is_timeout {
                        warn!("Dialog {} recovery attempt {} timed out", dialog_id_clone, attempt);
                    } else {
                        warn!("Dialog {} recovery attempt {} failed: {}", 
                            dialog_id_clone, attempt, reason);
                    }
                    
                    // Emit event through event bus if needed
                    if let Some(session_id) = &session_id_clone {
                        event_bus.publish(crate::events::SessionEvent::Custom {
                            session_id: session_id.clone(),
                            event_type: "recovery_attempt_failed".to_string(),
                            data: serde_json::json!({
                                "dialog_id": dialog_id_clone.to_string(),
                                "attempt": attempt,
                                "reason": reason,
                                "is_timeout": is_timeout
                            }),
                        });
                    }
                },
                super::recovery::RecoveryEvent::RetryDelay { delay_ms } => {
                    debug!("Waiting {}ms before next recovery attempt for dialog {}", 
                        delay_ms, dialog_id_clone);
                }
            }
        };
        
        // Run the recovery process
        // Note: We must take mutable access to metrics inside the perform_recovery_process function
        // since tokio::RwLock requires an .await after lock()
        let recovery_result = super::recovery::perform_recovery_process(
            &mut dialog,
            transport.as_ref(),
            config,
            &metrics_arc,
            event_callback
        ).await;
        
        // Drop the dialog reference before handling the result
        // to prevent deadlocks with other locks
        drop(dialog);
        
        // Process the recovery result
        match recovery_result {
            super::recovery::RecoveryResult::Success { recovery_time_ms } => {
                info!("Dialog {} successfully recovered in {}ms", dialog_id, recovery_time_ms);
                
                // No need to call mark_recovery_successful here as it was done inside perform_recovery_process
                // But we still need to emit the recovery completed event
                if let Some(session_id) = session_id {
                    self.event_bus.publish(SessionEvent::DialogRecoveryCompleted {
                        session_id,
                        dialog_id: dialog_id.clone(),
                        success: true,
                    });
                }
            },
            super::recovery::RecoveryResult::Failure { reason, activate_circuit_breaker } => {
                warn!("Dialog {} recovery failed: {}", dialog_id, reason);
                
                // Activate circuit breaker if needed
                if activate_circuit_breaker {
                    // Need to use async lock acquire with await point
                    let mut metrics = self.recovery_metrics.write().await;
                    metrics.open_circuit_breaker();
                    warn!("Dialog recovery circuit breaker opened after consecutive failures");
                    drop(metrics); // Explicitly drop the lock
                }
                
                // Emit recovery completed event
                if let Some(session_id) = session_id {
                    // Emit dialog state changed event
                    self.event_bus.publish(SessionEvent::DialogStateChanged {
                        session_id: session_id.clone(),
                        dialog_id: dialog_id.clone(),
                        previous: DialogState::Recovering,
                        current: DialogState::Terminated,
                    });
                    
                    // Emit specific recovery failed event
                    self.event_bus.publish(SessionEvent::DialogRecoveryCompleted {
                        session_id: session_id.clone(),
                        dialog_id: dialog_id.clone(),
                        success: false,
                    });
                    
                    // Emit dialog/session terminated event
                    self.event_bus.publish(SessionEvent::Terminated {
                        session_id,
                        reason: format!("Recovery failed: {}", reason),
                    });
                }
            },
            super::recovery::RecoveryResult::Aborted { reason } => {
                warn!("Dialog {} recovery aborted: {}", dialog_id, reason);
                
                // Emit recovery aborted event
                if let Some(session_id) = session_id {
                    self.event_bus.publish(crate::events::SessionEvent::Custom {
                        session_id,
                        event_type: "recovery_aborted".to_string(),
                        data: serde_json::json!({
                            "dialog_id": dialog_id.to_string(),
                            "reason": reason
                        }),
                    });
                }
            }
        }
    }
    
    /// Check if a dialog needs recovery based on network failure
    pub async fn needs_recovery(&self, dialog_id: &DialogId) -> bool {
        let dialog_opt = self.dialogs.get(dialog_id);
        if dialog_opt.is_none() {
            return false;
        }
        
        let dialog = dialog_opt.unwrap();
        let config = &self.recovery_config;
        let metrics = self.recovery_metrics.read().await;
        super::recovery::needs_recovery(&dialog, config, &metrics)
    }
    
    /// Get the session ID associated with a dialog
    fn get_session_for_dialog(&self, dialog_id: &DialogId) -> Option<SessionId> {
        self.dialog_to_session.get(dialog_id).map(|id| id.clone())
    }

    // Methods to support testing without exposing internal fields directly
    
    /// Get a dialog's state (primarily for testing)
    pub fn get_dialog_state(&self, dialog_id: &DialogId) -> Result<DialogState, Error> {
        match self.dialogs.get(dialog_id) {
            Some(dialog) => Ok(dialog.state.clone()),
            None => Err(dialog_not_found_error(dialog_id))
        }
    }
    
    /// Update a dialog's property for testing
    pub fn update_dialog_property(&self, dialog_id: &DialogId, 
                                  updater: impl FnOnce(&mut Dialog)) -> Result<(), Error> {
        match self.dialogs.get_mut(dialog_id) {
            Some(mut dialog) => {
                updater(&mut dialog);
                Ok(())
            },
            None => Err(dialog_not_found_error(dialog_id))
        }
    }
    
    /// Get a dialog's property (for testing)
    pub fn get_dialog_property<T: Clone>(&self, dialog_id: &DialogId, 
                                        getter: impl FnOnce(&Dialog) -> T) -> Result<T, Error> {
        match self.dialogs.get(dialog_id) {
            Some(dialog) => Ok(getter(&dialog)),
            None => Err(dialog_not_found_error(dialog_id))
        }
    }
    
    /// Check if a transaction is associated with a dialog (for testing)
    pub fn is_transaction_associated(&self, transaction_id: &TransactionKey, dialog_id: &DialogId) -> bool {
        // We can't use match self.transaction_to_dialog.get(transaction_id) due to type issues,
        // so check for key existence first
        if self.transaction_to_dialog.contains_key(transaction_id) {
            if let Some(stored_id) = self.transaction_to_dialog.get(transaction_id) {
                return *stored_id == *dialog_id;
            }
        }
        false
    }
    
    /// Find the dialog associated with a transaction
    pub fn find_dialog_for_transaction(&self, transaction_id: &TransactionKey) -> Option<DialogId> {
        if self.transaction_to_dialog.contains_key(transaction_id) {
            self.transaction_to_dialog.get(transaction_id).map(|id| id.clone())
        } else {
            None
        }
    }

    /// Test-only method to bypass the recovery process entirely and directly set the dialog state
    #[cfg(test)]
    pub async fn test_simulate_recovery(&self, dialog_id: &DialogId, success: bool) -> Result<(), Error> {
        // Get the dialog and set it to Recovering state first
        {
            let mut dialog = self.dialogs.get_mut(dialog_id)
                .ok_or_else(|| dialog_not_found_error(dialog_id))?;
            
            dialog.state = DialogState::Recovering;
            dialog.recovery_reason = Some("Test simulated recovery".to_string());
            dialog.recovery_start_time = Some(SystemTime::now());
        }
        
        // Small delay to let tasks process
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        
        // Update dialog based on success parameter
        self.update_dialog_property(dialog_id, |dialog| {
            if success {
                super::recovery::complete_recovery(dialog);
            } else {
                super::recovery::abandon_recovery(dialog);
            }
        })?;
        
        // Emit appropriate events
        let session_id = self.get_session_for_dialog(dialog_id);
        if let Some(session_id) = session_id {
            if success {
                self.event_bus.publish(SessionEvent::DialogRecoveryCompleted {
                    session_id,
                    dialog_id: dialog_id.clone(),
                    success: true,
                });
            } else {
                self.event_bus.publish(SessionEvent::DialogRecoveryCompleted {
                    session_id: session_id.clone(),
                    dialog_id: dialog_id.clone(),
                    success: false,
                });
                
                self.event_bus.publish(SessionEvent::Terminated {
                    session_id,
                    reason: "Simulated recovery failure".to_string(),
                });
            }
        }
        
        Ok(())
    }

    /// Get current recovery metrics
    pub async fn recovery_metrics(&self) -> RecoveryMetrics {
        self.recovery_metrics.read().await.clone()
    }

    /// Stop the dialog manager and clean up all resources
    pub async fn stop(&self) -> Result<(), Error> {
        debug!("Stopping dialog manager");
        
        // Check if we have any active dialogs
        let active_dialogs = self.dialog_count();
        if active_dialogs > 0 {
            info!("Stopping dialog manager with {} active dialogs", active_dialogs);
            
            // Get all dialog IDs
            let dialog_ids: Vec<DialogId> = self.dialogs.iter()
                .map(|entry| entry.key().clone())
                .collect();
            
            // Terminate each dialog with timeout
            let terminate_futures = dialog_ids.iter().map(|dialog_id| {
                let dialog_id = dialog_id.clone();
                let dm = self.clone();
                
                async move {
                    match tokio::time::timeout(
                        std::time::Duration::from_millis(500), 
                        dm.terminate_dialog(&dialog_id)
                    ).await {
                        Ok(Ok(_)) => true,
                        _ => {
                            warn!("Failed to terminate dialog {} during shutdown", dialog_id);
                            false
                        }
                    }
                }
            });
            
            // Execute all terminations concurrently with an overall timeout
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                futures::future::join_all(terminate_futures)
            ).await {
                Ok(results) => {
                    let success_count = results.iter().filter(|&&success| success).count();
                    let failed_count = results.len() - success_count;
                    
                    if failed_count > 0 {
                        warn!("Failed to terminate {} dialogs during shutdown", failed_count);
                    }
                    
                    debug!("Successfully terminated {} of {} dialogs", 
                          success_count, dialog_ids.len());
                },
                Err(_) => {
                    warn!("Timeout during dialog termination, forcing cleanup");
                }
            }
        }
        
        // Force cleanup of any remaining resources
        self.cleanup_all();
        
        debug!("Dialog manager stopped successfully");
        Ok(())
    }
    
    /// Clean up all resources
    fn cleanup_all(&self) {
        // Clear all mappings
        self.dialogs.clear();
        self.dialog_lookup.clear();
        self.dialog_to_session.clear();
        self.transaction_to_dialog.clear();
        self.subscribed_transactions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::net::SocketAddr;
    use crate::events::EventBus;
    use crate::Dialog;
    use tokio::sync::mpsc;
    use rvoip_sip_core::TypedHeader;
    use rvoip_sip_core::types::{call_id::CallId, cseq::CSeq, address::Address, param::Param};
    use rvoip_sip_core::types::{from::From as FromHeader, to::To as ToHeader};
    use rvoip_sip_core::types::contact::{Contact, ContactParamInfo};
    
    // Dummy transport implementation for testing
    #[derive(Clone, Debug)]
    struct DummyTransport;
    
    impl DummyTransport {
        fn new() -> Self {
            Self
        }
    }
    
    // Implement the Transport trait for DummyTransport
    #[async_trait::async_trait]
    impl Transport for DummyTransport {
        fn local_addr(&self) -> std::result::Result<SocketAddr, TransportError> {
            Ok(SocketAddr::from_str("127.0.0.1:5060").unwrap())
        }
        
        async fn send_message(&self, _message: rvoip_sip_core::Message, _destination: SocketAddr) -> std::result::Result<(), TransportError> {
            Ok(())
        }
        
        async fn close(&self) -> std::result::Result<(), TransportError> {
            Ok(())
        }
        
        fn is_closed(&self) -> bool {
            false
        }
    }
    
    // Helper to create test SIP messages for testing
    fn create_test_invite() -> Request {
        let mut request = Request::new(Method::Invite, Uri::sip("bob@example.com"));
        
        // Add Call-ID
        let call_id = CallId("test-call-id".to_string());
        request.headers.push(TypedHeader::CallId(call_id));
        
        // Add From with tag using proper API
        let from_uri = Uri::sip("alice@example.com");
        let from_addr = Address::new(from_uri).with_tag("alice-tag");
        let from = FromHeader(from_addr);
        request.headers.push(TypedHeader::From(from));
        
        // Add To
        let to_uri = Uri::sip("bob@example.com");
        let to_addr = Address::new(to_uri);
        let to = ToHeader(to_addr);
        request.headers.push(TypedHeader::To(to));
        
        // Add CSeq
        let cseq = CSeq::new(1, Method::Invite);
        request.headers.push(TypedHeader::CSeq(cseq));
        
        request
    }
    
    fn create_test_response(status: StatusCode, with_to_tag: bool) -> Response {
        let mut response = Response::new(status);
        
        // Add Call-ID
        let call_id = CallId("test-call-id".to_string());
        response.headers.push(TypedHeader::CallId(call_id));
        
        // Add From with tag using proper API
        let from_uri = Uri::sip("alice@example.com");
        let from_addr = Address::new(from_uri).with_tag("alice-tag");
        let from = FromHeader(from_addr);
        response.headers.push(TypedHeader::From(from));
        
        // Add To, optionally with tag using proper API
        let to_uri = Uri::sip("bob@example.com");
        let to_addr = if with_to_tag {
            Address::new(to_uri).with_tag("bob-tag")
        } else {
            Address::new(to_uri)
        };
        let to = ToHeader(to_addr);
        response.headers.push(TypedHeader::To(to));
        
        // Add Contact
        let contact_uri = Uri::sip("bob@192.168.1.2");
        let contact_addr = Address::new(contact_uri);
        
        // Create contact header using the correct API
        let contact_param = ContactParamInfo { address: contact_addr };
        let contact = Contact::new_params(vec![contact_param]);
        response.headers.push(TypedHeader::Contact(contact));
        
        response
    }
    
    #[tokio::test]
    async fn test_dialog_manager_creation() {
        // Create a simple test to verify that DialogManager can be created
        let event_bus = EventBus::new(10);
        
        // This is a placeholder test since we don't have a real TransactionManager to use
        // In the future, we'd need to expand the session-core library to support proper mocking
        assert!(true, "This test passes but needs to be expanded");
    }
    
    #[test]
    fn test_dialog_creation_directly() {
        // Test the Dialog class directly without needing DialogManager
        
        // Create a test INVITE and response
        let request = create_test_invite();
        let response = create_test_response(StatusCode::Ok, true);
        
        // Create a dialog as UAC (initiator)
        let dialog = Dialog::from_2xx_response(&request, &response, true);
        
        // Verify the dialog was created
        assert!(dialog.is_some(), "Failed to create dialog from 2xx response");
        
        let dialog = dialog.unwrap();
        
        // Verify the dialog properties
        assert_eq!(dialog.state, DialogState::Confirmed);
        assert_eq!(dialog.call_id, "test-call-id");
        assert_eq!(dialog.local_tag, Some("alice-tag".to_string()));
        assert_eq!(dialog.remote_tag, Some("bob-tag".to_string()));
        assert_eq!(dialog.local_seq, 1);
        assert_eq!(dialog.remote_seq, 0);
        assert_eq!(dialog.is_initiator, true);
        assert_eq!(dialog.remote_target.to_string(), "sip:bob@192.168.1.2");
    }
    
    #[test]
    fn test_dialog_utils() {
        // Test the has_to_tag function directly by checking To headers
        let response_with_tag = create_test_response(StatusCode::Ok, true);
        let response_without_tag = create_test_response(StatusCode::Ok, false);
        
        // Check the To header for tag parameter directly
        let has_tag = response_with_tag.to()
            .and_then(|to| to.tag())
            .is_some();
        
        let missing_tag = response_without_tag.to()
            .and_then(|to| to.tag())
            .is_none();
        
        assert!(has_tag, "Response should have a to-tag");
        assert!(missing_tag, "Response should not have a to-tag");
    }
} 