use tracing::{debug, error, warn, info};
use std::str::FromStr;

use rvoip_sip_core::{
    Request, Response, Method, StatusCode, TypedHeader, HeaderName
};

use rvoip_transaction_core::{
    TransactionEvent, 
    TransactionKey,
    TransactionKind
};

use super::manager::DialogManager;
use super::dialog_state::DialogState;
use crate::events::SessionEvent;
use crate::session::SessionId;

impl DialogManager {
    /// Process a transaction event and update dialogs accordingly
    pub async fn process_transaction_event(&self, event: TransactionEvent) {
        debug!("Processing transaction event: {:?}", event);
        
        match event {
            TransactionEvent::Response { transaction_id: tx_key, response, source: _ } => {
                self.handle_response_event(tx_key, response).await;
            },
            TransactionEvent::AckReceived { transaction_id: tx_key, request } => {
                self.handle_ack_received_event(tx_key, request).await;
            },
            TransactionEvent::NewRequest { transaction_id: tx_key, request, source } => {
                self.handle_new_request_event(tx_key, request, source).await;
            },
            TransactionEvent::InviteRequest { transaction_id: tx_key, request, source } => {
                self.handle_invite_request_event(tx_key, request, source).await;
            },
            TransactionEvent::NonInviteRequest { transaction_id: tx_key, request, source } => {
                self.handle_non_invite_request_event(tx_key, request, source).await;
            },
            TransactionEvent::Error { transaction_id, error } => {
                self.handle_error_event(transaction_id, error).await;
            },
            // Catch-all for any other events
            _ => {
                debug!("Received unhandled transaction event: {:?}", event);
            }
        }
    }
    
    /// Handle response events
    async fn handle_response_event(&self, tx_key: TransactionKey, response: Response) {
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
            self.handle_early_media_response(&mut dialog, &dialog_id, &tx_key, &response).await;
        }
        // Handle success responses for UPDATE method
        else if response.status == StatusCode::Ok || response.status == StatusCode::Accepted {
            self.handle_success_response_internal(&mut dialog, &dialog_id, &tx_key, &response).await;
        }
        
        // For server transactions sending responses, we need to create dialogs
        if tx_key.is_server() && response.status().is_success() {
            // Get the original request to create the dialog
            if let Ok(Some(request)) = self.get_transaction_request(&tx_key).await {
                debug!("Creating dialog from successful response {} for transaction {}", response.status(), tx_key);
                
                // Create dialog from the transaction
                if let Some(dialog_id) = self.create_dialog_from_transaction(&tx_key, &request, &response, false).await {
                    debug!("Created dialog {} from response event", dialog_id);
                    
                    // Emit dialog created event if associated with a session
                    if let Some(session_id) = self.find_session_for_transaction(&tx_key) {
                        debug!("Associating dialog {} with session {}", dialog_id, session_id);
                        let _ = self.associate_with_session(&dialog_id, &session_id);
                        
                        // Emit dialog updated event
                        self.event_bus.publish(SessionEvent::DialogCreated {
                            session_id,
                            dialog_id,
                        });
                    }
                }
            }
        }
    }
    
    /// Handle early media responses
    async fn handle_early_media_response(
        &self,
        dialog: &mut dashmap::mapref::one::RefMut<'_, super::dialog_id::DialogId, super::dialog_impl::Dialog>,
        dialog_id: &super::dialog_id::DialogId,
        tx_key: &TransactionKey,
        response: &Response
    ) {
        // Check for SDP in early media
        if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
            if content_type.to_string() == "application/sdp" {
                if let Ok(sdp_str) = std::str::from_utf8(&response.body) {
                    if let Ok(sdp) = crate::sdp::SessionDescription::from_str(sdp_str) {
                        // Update the dialog with the remote SDP answer (early media)
                        if dialog.sdp_context.state == crate::sdp::NegotiationState::OfferSent {
                            dialog.sdp_context.update_with_remote_answer(sdp.clone());
                            
                            // Fire SDP answer received event
                            if let Some(session_id) = self.find_session_for_transaction(tx_key) {
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
    
    /// Handle success responses (internal method to avoid naming conflicts)
    async fn handle_success_response_internal(
        &self,
        dialog: &mut dashmap::mapref::one::RefMut<'_, super::dialog_id::DialogId, super::dialog_impl::Dialog>,
        dialog_id: &super::dialog_id::DialogId,
        tx_key: &TransactionKey,
        response: &Response
    ) {
        // Handle SDP in the response if it exists
        if let Some(TypedHeader::ContentType(content_type)) = response.header(&HeaderName::ContentType) {
            if content_type.to_string() == "application/sdp" {
                if let Ok(sdp_str) = std::str::from_utf8(&response.body) {
                    if let Ok(sdp) = crate::sdp::SessionDescription::from_str(sdp_str) {
                        // Update the dialog with the remote SDP answer
                        if dialog.sdp_context.state == crate::sdp::NegotiationState::OfferSent {
                            dialog.sdp_context.update_with_remote_answer(sdp.clone());
                            
                            // Fire SDP answer received event
                            if let Some(session_id) = self.find_session_for_transaction(tx_key) {
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
        if let Some(session_id) = self.find_session_for_transaction(tx_key) {
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
    
    /// Handle ACK received events
    async fn handle_ack_received_event(&self, tx_key: TransactionKey, request: Request) {
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
    }
    
    /// Handle new request events
    async fn handle_new_request_event(&self, tx_key: TransactionKey, request: Request, source: std::net::SocketAddr) {
        debug!("Received new request {}:\n{}", tx_key, request);
        
        // Handle ACK requests specially - they need different handling based on response type
        if request.method() == Method::Ack {
            self.handle_ack_request(request, source).await;
            return;
        }
        
        // Process other new requests as usual...
        // For new dialogs, create a server transaction
        self.create_server_transaction_for_request(tx_key, request, source).await;
    }
    
    /// Handle ACK requests at dialog level
    async fn handle_ack_request(&self, request: Request, source: std::net::SocketAddr) {
        debug!("Processing ACK request");
        
        // **CRITICAL FIX**: ACK for 2xx responses should be handled at dialog level
        // According to RFC 3261, ACK for 2xx responses is end-to-end and doesn't 
        // belong to the INVITE transaction (which is already terminated)
        
        // Try to find the dialog this ACK belongs to
        if let Some(dialog_id) = self.find_dialog_for_request(&request) {
            debug!("Found dialog {} for ACK request - handling at dialog level", dialog_id);
            
            // Handle ACK at dialog level (this confirms the dialog)
            if let Some(dialog) = self.dialogs.get(&dialog_id) {
                debug!("Confirming dialog {} with ACK", dialog_id);
                // The dialog is already established, ACK just confirms it
                // No further action needed - the call is now fully established
            }
            
            // Notify about ACK received
            let branch = request.first_via()
                .and_then(|v| v.branch().map(|b| b.to_string()))
                .unwrap_or_else(|| "unknown".to_string());
                
            let _ = self.event_sender.send(TransactionEvent::AckRequest {
                transaction_id: TransactionKey::new(
                    branch,
                    Method::Ack,
                    false // ACK is not a server transaction
                ),
                request,
                source,
            }).await;
        } else {
            debug!("No dialog found for ACK request - treating as stray ACK");
            
            // This is a stray ACK (no matching dialog)
            let _ = self.event_sender.send(TransactionEvent::StrayAckRequest {
                request,
                source,
            }).await;
        }
    }
    
    /// Handle incoming INVITE request events
    async fn handle_invite_request_event(&self, tx_key: TransactionKey, request: Request, source: std::net::SocketAddr) {
        debug!("Received INVITE request from transaction-core: {}", tx_key);
        
        // ✅ **CORRECT ARCHITECTURE**: DialogManager handles SIP protocol directly
        if let Err(e) = self.handle_invite_protocol(tx_key, request, source).await {
            error!("Failed to handle INVITE protocol: {}", e);
        }
    }
    
    /// Handle non-INVITE request events
    async fn handle_non_invite_request_event(&self, tx_key: TransactionKey, request: Request, source: std::net::SocketAddr) {
        debug!("Received non-INVITE request from transaction-core: {} (method: {})", tx_key, request.method());
        
        match request.method() {
            Method::Bye => {
                debug!("Processing BYE request for transaction {}", tx_key);
                
                // ✅ **CORRECT ARCHITECTURE**: DialogManager handles BYE protocol directly
                if let Err(e) = self.handle_bye_protocol(tx_key, request).await {
                    error!("Failed to handle BYE protocol: {}", e);
                }
            },
            Method::Register => {
                debug!("Processing REGISTER request for transaction {}", tx_key);
                
                // ✅ **CORRECT ARCHITECTURE**: DialogManager handles REGISTER protocol directly
                if let Err(e) = self.handle_register_protocol(tx_key, request, source).await {
                    error!("Failed to handle REGISTER protocol: {}", e);
                }
            },
            _ => {
                debug!("Received {} request - protocol handling not yet implemented", request.method());
                
                // Emit event for the request for other protocol handlers
                self.event_bus.publish(crate::events::SessionEvent::Custom {
                    session_id: SessionId::new(),
                    event_type: format!("new_{}", request.method().to_string().to_lowercase()),
                    data: serde_json::json!({
                        "transaction_id": tx_key.to_string(),
                    }),
                });
            }
        }
    }
    
    /// Handle error events
    async fn handle_error_event(&self, transaction_id: Option<TransactionKey>, error: String) {
        debug!("Transaction error event received for {:?}: {}", transaction_id, error);
        
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
        let error_string = error.clone(); // error is already a String
        
        // Spawn a task to check for recovery needs and handle it asynchronously
        tokio::spawn(async move {
            if dialog_manager.needs_recovery(&dialog_id_clone).await {
                debug!("Initiating recovery for dialog {} due to transaction error", dialog_id_clone);
                
                let reason = format!("Transaction error: {}", error_string);
                
                if let Err(e) = dialog_manager.recover_dialog(&dialog_id_clone, &reason).await {
                    error!("Failed to initiate recovery for dialog {}: {}", dialog_id_clone, e);
                }
            }
        });
    }
} 