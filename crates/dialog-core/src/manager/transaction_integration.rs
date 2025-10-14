//! Transaction Integration for Dialog Management
//!
//! This module handles the integration between dialog-core and transaction-core,
//! managing transaction lifecycle events, request/response routing, and event processing.
//! It provides the bridge between SIP transaction reliability and dialog state management.
//!
//! ## Key Responsibilities
//!
//! - Processing transaction events and routing to appropriate dialogs
//! - Managing transaction-to-dialog associations  
//! - Handling transaction completion and cleanup
//! - Converting between transaction and dialog abstractions
//! - Coordinating request sending through transaction layer

use std::net::SocketAddr;
use tracing::{debug, warn, info, error};
use rvoip_sip_core::{Request, Response, Method};
use crate::transaction::{TransactionKey, TransactionEvent, TransactionState};
use crate::transaction::builders::{dialog_utils, dialog_quick};
use crate::transaction::dialog::{request_builder_from_dialog_template, DialogRequestTemplate};
use crate::errors::DialogResult;
use crate::dialog::DialogId;
use crate::events::{DialogEvent, SessionCoordinationEvent};
use super::core::DialogManager;

/// Trait for transaction integration operations
pub trait TransactionIntegration {
    /// Send a request within a dialog using transaction-core
    fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> impl std::future::Future<Output = DialogResult<TransactionKey>> + Send;
    
    /// Send a response using transaction-core
    fn send_transaction_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Trait for transaction helper operations
pub trait TransactionHelpers {
    /// Associate a transaction with a dialog
    fn link_transaction_to_dialog(&self, transaction_id: &TransactionKey, dialog_id: &DialogId);
    
    /// Create ACK for 2xx response using transaction-core helpers
    fn create_ack_for_success_response(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> impl std::future::Future<Output = DialogResult<Request>> + Send;
}

// Actual implementations for DialogManager
impl TransactionIntegration for DialogManager {
    /// Send a request within a dialog using transaction-core
    /// 
    /// Implements proper request creation within dialogs using Phase 3 dialog functions
    /// for significantly simplified and more maintainable code.
    async fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> DialogResult<TransactionKey> {
        debug!("Sending {} request for dialog {} using Phase 3 dialog functions", method, dialog_id);
        
        // Get destination and dialog context
        let (destination, request) = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            
            let destination = dialog.get_remote_target_address().await
                .ok_or_else(|| crate::errors::DialogError::routing_error("No remote target address available"))?;
            
            // Convert body to String if provided
            let body_string = body.map(|b| String::from_utf8_lossy(&b).to_string());
            
            // Create dialog template using the proper dialog method
            let template = dialog.create_request_template(method.clone());
            
            // Generate local tag if missing (for outgoing requests we should always have a local tag)
            let local_tag = match template.local_tag {
                Some(tag) if !tag.is_empty() => tag,
                _ => {
                    let new_tag = dialog.generate_local_tag();
                    dialog.local_tag = Some(new_tag.clone());
                    new_tag
                }
            };
            
            // Handle remote tag based on dialog state and method
            let remote_tag = match (&template.remote_tag, dialog.state.clone()) {
                // If we have a valid remote tag, use it
                (Some(tag), _) if !tag.is_empty() => Some(tag.clone()),
                
                // For certain methods in confirmed dialogs, remote tag is required
                (_, crate::dialog::DialogState::Confirmed) => {
                    error!("Dialog {} in Confirmed state but missing remote tag for {} request. Dialog details: local_tag={:?}, remote_tag={:?}", 
                           dialog_id, method, dialog.local_tag, dialog.remote_tag);
                    return Err(crate::errors::DialogError::protocol_error(
                        &format!("{} request in confirmed dialog missing remote tag", method)
                    ));
                },
                
                // For early/initial dialogs, remote tag may be None (will be set to None, not empty string)
                _ => None
            };
            
            // Build request using Phase 3 dialog quick functions (MUCH simpler!)
            let request = match method {
                Method::Invite => {
                    // Distinguish between initial INVITE and re-INVITE based on remote tag
                    match remote_tag {
                        Some(remote_tag) => {
                            // re-INVITE: We have a remote tag, so this is for an established dialog
                            // re-INVITE requires SDP content for session modification
                            let sdp_content = body_string.ok_or_else(|| {
                                crate::errors::DialogError::protocol_error("re-INVITE request requires SDP content for session modification")
                            })?;
                            
                            dialog_quick::reinvite_for_dialog(
                                &template.call_id,
                                &template.local_uri.to_string(),
                                &local_tag,
                                &template.remote_uri.to_string(),
                                &remote_tag,
                                &sdp_content,
                                template.cseq_number,
                                self.local_address,
                                if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) },
                                None // Let reinvite_for_dialog generate appropriate Contact
                            )
                        },
                        None => {
                            // Initial INVITE: No remote tag yet, creating new dialog
                            use crate::transaction::client::builders::InviteBuilder;
                            
                            let mut invite_builder = InviteBuilder::new()
                                .from_detailed(
                                    Some("User"), // Display name
                                    template.local_uri.to_string(),
                                    Some(&local_tag)
                                )
                                .to_detailed(
                                    Some("User"), // Display name  
                                    template.remote_uri.to_string(),
                                    None // No remote tag for initial INVITE
                                )
                                .call_id(&template.call_id)
                                .cseq(template.cseq_number)
                                .request_uri(template.target_uri.to_string())
                                .local_address(self.local_address);
                            
                            // Add route set if present
                            for route in &template.route_set {
                                invite_builder = invite_builder.add_route(route.clone());
                            }
                            
                            // Add SDP content if provided
                            if let Some(sdp_content) = body_string {
                                invite_builder = invite_builder.with_sdp(sdp_content);
                            }
                            
                            invite_builder.build()
                        }
                    }
                },
                
                Method::Bye => {
                    // BYE requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("BYE request requires remote tag in established dialog")
                    })?;
                    
                    dialog_quick::bye_for_dialog(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        template.cseq_number,
                        self.local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) }
                    )
                },
                
                Method::Refer => {
                    // REFER requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("REFER request requires remote tag in established dialog")
                    })?;
                    
                    // Extract the target URI from the body if it's in the old format ("Refer-To: <uri>")
                    // Otherwise use it directly as the target URI
                    let target_uri = if let Some(body) = body_string.clone() {
                        // Check if it's in the old format with "Refer-To: " prefix
                        if body.starts_with("Refer-To: ") {
                            body.trim_start_matches("Refer-To: ").trim_end_matches("\r\n").to_string()
                        } else {
                            body
                        }
                    } else {
                        "sip:unknown".to_string()
                    };
                    
                    dialog_quick::refer_for_dialog(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &target_uri,
                        template.cseq_number,
                        self.local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) }
                    )
                },
                
                Method::Update => {
                    // UPDATE requires both tags in established dialogs  
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("UPDATE request requires remote tag in established dialog")
                    })?;
                    
                    dialog_quick::update_for_dialog(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        body_string.clone(),
                        template.cseq_number,
                        self.local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) }
                    )
                },
                
                Method::Info => {
                    // INFO requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("INFO request requires remote tag in established dialog")
                    })?;
                    
                    let content = body_string.unwrap_or_else(|| "Application info".to_string());
                    dialog_quick::info_for_dialog(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &content,
                        Some("application/info".to_string()),
                        template.cseq_number,
                        self.local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) }
                    )
                },
                
                Method::Notify => {
                    // NOTIFY requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("NOTIFY request requires remote tag in established dialog")
                    })?;

                    // Get event type and subscription state from dialog (RFC 6665 compliance)
                    let (event_type, subscription_state) = {
                        let dialog_ref = self.get_dialog(&dialog_id)?;
                        let event = dialog_ref.event_package.clone().unwrap_or_else(|| "dialog".to_string());
                        let sub_state = dialog_ref.subscription_state.as_ref().map(|s| s.to_header_value());
                        (event, sub_state)
                    };

                    dialog_quick::notify_for_dialog(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &event_type,
                        body_string,
                        subscription_state,
                        template.cseq_number,
                        self.local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) }
                    )
                },
                
                Method::Message => {
                    // MESSAGE requires both tags in established dialogs
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error("MESSAGE request requires remote tag in established dialog")
                    })?;
                    
                    let content = body_string.unwrap_or_else(|| "".to_string());
                    dialog_quick::message_for_dialog(
                        &template.call_id,
                        &template.local_uri.to_string(),
                        &local_tag,
                        &template.remote_uri.to_string(),
                        &remote_tag,
                        &content,
                        Some("text/plain".to_string()),
                        template.cseq_number,
                        self.local_address,
                        if template.route_set.is_empty() { None } else { Some(template.route_set.clone()) }
                    )
                },
                
                _ => {
                    // For any other method, require established dialog
                    let remote_tag = remote_tag.ok_or_else(|| {
                        crate::errors::DialogError::protocol_error(&format!("{} request requires remote tag in established dialog", method))
                    })?;
                    
                    // Use dialog template + utility function
                    let template_struct = DialogRequestTemplate {
                        call_id: template.call_id,
                        from_uri: template.local_uri.to_string(),
                        from_tag: local_tag,
                        to_uri: template.remote_uri.to_string(),
                        to_tag: remote_tag,
                        request_uri: template.target_uri.to_string(),
                        cseq: template.cseq_number,
                        local_address: self.local_address,
                        route_set: template.route_set.clone(),
                        contact: None,
                    };
                    
                    request_builder_from_dialog_template(
                        &template_struct,
                        method.clone(),
                        body_string,
                        None // Auto-detect content type
                    )
                }
            }.map_err(|e| crate::errors::DialogError::InternalError {
                message: format!("Failed to build {} request using Phase 3 dialog functions: {}", method, e),
                context: None,
            })?;
            
            (destination, request)
        };
        
        // Use transaction-core helpers to create appropriate transaction
        let transaction_id = if method == Method::Invite {
            self.transaction_manager
                .create_invite_client_transaction(request, destination)
                .await
        } else {
            self.transaction_manager
                .create_non_invite_client_transaction(request, destination)
                .await
        }.map_err(|e| crate::errors::DialogError::TransactionError {
            message: format!("Failed to create {} transaction: {}", method, e),
        })?;
        
        // Associate transaction with dialog BEFORE sending
        self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
        debug!("✅ Associated {} transaction {} with dialog {}", method, transaction_id, dialog_id);
        
        // Send the request using transaction-core
        self.transaction_manager
            .send_request(&transaction_id)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to send request: {}", e),
            })?;
        
        debug!("Successfully sent {} request for dialog {} (transaction: {}) using Phase 3 dialog functions", method, dialog_id, transaction_id);
        
        Ok(transaction_id)
    }
    
    /// Send a response using transaction-core
    /// 
    /// Delegates response sending to transaction-core while maintaining dialog state.
    async fn send_transaction_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        debug!("Sending response {} for transaction {}", response.status_code(), transaction_id);
        
        // Use transaction-core to send the response
        self.transaction_manager
            .send_response(transaction_id, response)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to send response: {}", e),
            })?;
        
        debug!("Successfully sent response for transaction {}", transaction_id);
        Ok(())
    }
}

impl TransactionHelpers for DialogManager {
    /// Associate a transaction with a dialog
    /// 
    /// Creates the mapping between transactions and dialogs for proper message routing.
    fn link_transaction_to_dialog(&self, transaction_id: &TransactionKey, dialog_id: &DialogId) {
        self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
        debug!("Linked transaction {} to dialog {}", transaction_id, dialog_id);
    }
    
    /// Create ACK for 2xx response using transaction-core helpers
    /// 
    /// Uses transaction-core's ACK creation helpers while maintaining dialog-core concerns.
    async fn create_ack_for_success_response(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> DialogResult<Request> {
        debug!("Creating ACK for 2xx response using transaction-core helpers");
        
        // Use transaction-core's helper method to create ACK for 2xx response
        // This ensures proper ACK construction according to RFC 3261
        let ack_request = self.transaction_manager
            .create_ack_for_2xx(original_invite_tx_id, response)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to create ACK for 2xx using transaction-core: {}", e),
            })?;
        
        debug!("Successfully created ACK for 2xx response");
        Ok(ack_request)
    }
}

// Transaction Event Processing Implementation
impl DialogManager {
    /// Process a transaction event and update dialog state accordingly
    /// 
    /// This is the core event-driven state management for dialogs based on
    /// transaction layer events. It implements proper RFC 3261 dialog state transitions.
    pub async fn process_transaction_event(
        &self,
        transaction_id: &TransactionKey,
        dialog_id: &DialogId,
        event: TransactionEvent,
    ) -> DialogResult<()> {
        debug!("Processing transaction event for {} in dialog {}: {:?}", transaction_id, dialog_id, event);
        
        match event {
            TransactionEvent::StateChanged { previous_state, new_state, .. } => {
                self.handle_transaction_state_change(dialog_id, transaction_id, previous_state, new_state).await
            },
            
            TransactionEvent::SuccessResponse { response, .. } => {
                self.handle_transaction_success_response(dialog_id, transaction_id, response).await
            },
            
            TransactionEvent::FailureResponse { response, .. } => {
                self.handle_transaction_failure_response(dialog_id, transaction_id, response).await
            },
            
            TransactionEvent::ProvisionalResponse { response, .. } => {
                self.handle_transaction_provisional_response(dialog_id, transaction_id, response).await
            },
            
            TransactionEvent::TransactionTerminated { .. } => {
                self.handle_transaction_terminated(dialog_id, transaction_id).await
            },
            
            TransactionEvent::TimerTriggered { timer, .. } => {
                debug!("Timer {} triggered for transaction {} in dialog {}", timer, transaction_id, dialog_id);
                Ok(()) // Most timer events don't require dialog-level action
            },
            
            TransactionEvent::AckReceived { request, .. } => {
                self.handle_ack_received_event(dialog_id, transaction_id, request).await
            },
            
            _ => {
                debug!("Unhandled transaction event type for dialog {}: {:?}", dialog_id, event);
                Ok(())
            }
        }
    }
    
    /// Handle transaction state changes and update dialog state accordingly
    async fn handle_transaction_state_change(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        previous_state: TransactionState,
        new_state: TransactionState,
    ) -> DialogResult<()> {
        debug!("Transaction {} state: {:?} → {:?} for dialog {}", transaction_id, previous_state, new_state, dialog_id);
        
        // Update dialog state based on transaction state changes
        match new_state {
            TransactionState::Completed => {
                debug!("Transaction {} completed for dialog {}", transaction_id, dialog_id);
                // Transaction completed successfully - dialog remains active
            },
            
            TransactionState::Terminated => {
                debug!("Transaction {} terminated for dialog {}", transaction_id, dialog_id);
                // Clean up transaction association
                self.transaction_to_dialog.remove(transaction_id);
            },
            
            _ => {
                // Other state changes are informational
                debug!("Transaction {} in state {:?} for dialog {}", transaction_id, new_state, dialog_id);
            }
        }
        
        Ok(())
    }
    
    /// Handle successful responses from transactions
    async fn handle_transaction_success_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        info!("Transaction {} received success response {} {} for dialog {}", 
              transaction_id, response.status_code(), response.reason_phrase(), dialog_id);
        
        // Update dialog state based on successful response
        let dialog_state_changed = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            let old_state = dialog.state.clone();
            
            // Update dialog with response information (remote tag, etc.)
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    info!("Updating remote tag for dialog {} to: {}", dialog_id, to_tag);
                    dialog.set_remote_tag(to_tag.to_string());
                } else {
                    warn!("200 OK response has no To tag for dialog {}", dialog_id);
                }
            } else {
                warn!("200 OK response has no To header for dialog {}", dialog_id);
            }
            
            // Update dialog state based on response status and current state
            let state_changed = match response.status_code() {
                200 => {
                    if dialog.state == crate::dialog::DialogState::Early {
                        dialog.state = crate::dialog::DialogState::Confirmed;
                        
                        // CRITICAL FIX: Update dialog lookup now that we have both tags
                        if let Some(tuple) = dialog.dialog_id_tuple() {
                            let key = crate::manager::utils::DialogUtils::create_lookup_key(&tuple.0, &tuple.1, &tuple.2);
                            self.dialog_lookup.insert(key, dialog_id.clone());
                            info!("Updated dialog lookup for confirmed dialog {}", dialog_id);
                        }
                        
                        true
                    } else {
                        false
                    }
                },
                _ => false
            };
            
            if state_changed {
                Some((old_state, dialog.state.clone()))
            } else {
                None
            }
        };
        
        // Emit dialog events for session-core
        if let Some((old_state, new_state)) = dialog_state_changed {
            self.emit_dialog_event(DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            }).await;
        }
        
        // Emit session coordination events for session-core
        self.emit_session_coordination_event(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
        }).await;
        
        // Handle specific successful response types
        match response.status_code() {
            200 => {
                // Check if this is a 200 OK to INVITE - need to send ACK
                if transaction_id.method() == &rvoip_sip_core::Method::Invite {
                    info!("✅ Received 200 OK to INVITE, sending automatic ACK for dialog {}", dialog_id);

                    // Send ACK using transaction-core's send_ack_for_2xx method
                    match self.transaction_manager.send_ack_for_2xx(transaction_id, &response).await {
                        Ok(_) => {
                            info!("Successfully sent automatic ACK for 200 OK to INVITE");

                            // Notify session-core that ACK was sent (for state machine transition)
                            let negotiated_sdp = if !response.body().is_empty() {
                                Some(String::from_utf8_lossy(response.body()).to_string())
                            } else {
                                None
                            };

                            self.emit_session_coordination_event(SessionCoordinationEvent::AckSent {
                                dialog_id: dialog_id.clone(),
                                transaction_id: transaction_id.clone(),
                                negotiated_sdp,
                            }).await;
                        }
                        Err(e) => {
                            warn!("Failed to send automatic ACK for 200 OK to INVITE: {}", e);
                        }
                    }
                }
                
                // Check if this is a 200 OK to BYE - dialog is terminating
                if transaction_id.method() == &rvoip_sip_core::Method::Bye {
                    info!("✅ Received 200 OK to BYE, dialog {} is terminating", dialog_id);
                    
                    // Emit CallTerminating event to notify session-core
                    self.emit_session_coordination_event(SessionCoordinationEvent::CallTerminating {
                        dialog_id: dialog_id.clone(),
                        reason: "BYE completed successfully".to_string(),
                    }).await;
                }

                // Successful completion - could be call answered, request completed, etc.
                if !response.body().is_empty() {
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.emit_session_coordination_event(SessionCoordinationEvent::CallAnswered {
                        dialog_id: dialog_id.clone(),
                        session_answer: sdp,
                    }).await;
                }
            },
            _ => {
                debug!("Other successful response {} for dialog {}", response.status_code(), dialog_id);
            }
        }

        Ok(())
    }
    
    /// Handle failure responses from transactions
    async fn handle_transaction_failure_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        warn!("Transaction {} received failure response {} {} for dialog {}", 
              transaction_id, response.status_code(), response.reason_phrase(), dialog_id);
        
        // Handle specific failure cases and emit appropriate events
        match response.status_code() {
            487 => {
                // Request Terminated (CANCEL received)
                info!("Call cancelled for dialog {}", dialog_id);
                
                // Emit dialog event
                self.emit_dialog_event(DialogEvent::Terminated {
                    dialog_id: dialog_id.clone(),
                    reason: "Request terminated".to_string(),
                }).await;
                
                // Emit session coordination event
                self.emit_session_coordination_event(SessionCoordinationEvent::CallCancelled {
                    dialog_id: dialog_id.clone(),
                    reason: "Request terminated".to_string(),
                }).await;
            },
            
            status if status >= 400 && status < 500 => {
                // Client error - may require dialog termination
                warn!("Client error {} for dialog {} - considering termination", status, dialog_id);
                
                // Emit session coordination event for failed requests
                self.emit_session_coordination_event(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: "Unknown".to_string(), // TODO: Extract from transaction context
                }).await;
            },
            
            status if status >= 500 => {
                // Server error - may require retry or termination
                warn!("Server error {} for dialog {} - considering retry", status, dialog_id);
                
                // Emit session coordination event for server errors
                self.emit_session_coordination_event(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: "Unknown".to_string(), // TODO: Extract from transaction context
                }).await;
            },
            
            _ => {
                debug!("Other failure response {} for dialog {}", response.status_code(), dialog_id);
            }
        }
        
        // Always emit the response received event for session-core to handle
        self.emit_session_coordination_event(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
        }).await;
        
        Ok(())
    }
    
    /// Handle provisional responses from transactions
    async fn handle_transaction_provisional_response(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> DialogResult<()> {
        debug!("Transaction {} received provisional response {} {} for dialog {}", 
               transaction_id, response.status_code(), response.reason_phrase(), dialog_id);
        
        // Update dialog state for early dialogs
        let dialog_created = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            let old_state = dialog.state.clone();
            
            // For provisional responses with to-tag, create early dialog
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    if dialog.remote_tag.is_none() {
                        dialog.set_remote_tag(to_tag.to_string());
                        if dialog.state == crate::dialog::DialogState::Initial {
                            dialog.state = crate::dialog::DialogState::Early;
                            Some((old_state, dialog.state.clone()))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };
        
        // Emit dialog state change if early dialog was created
        if let Some((old_state, new_state)) = dialog_created {
            self.emit_dialog_event(DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            }).await;
        }
        
        // Handle specific provisional responses and emit session coordination events
        match response.status_code() {
            180 => {
                info!("Call ringing for dialog {}", dialog_id);
                
                self.emit_session_coordination_event(SessionCoordinationEvent::CallRinging {
                    dialog_id: dialog_id.clone(),
                }).await;
            },
            
            183 => {
                info!("Session progress for dialog {}", dialog_id);
                
                // Check for early media (SDP in 183)
                if !response.body().is_empty() {
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.emit_session_coordination_event(SessionCoordinationEvent::EarlyMedia {
                        dialog_id: dialog_id.clone(),
                        sdp,
                    }).await;
                } else {
                    self.emit_session_coordination_event(SessionCoordinationEvent::CallProgress {
                        dialog_id: dialog_id.clone(),
                        status_code: response.status_code(),
                        reason_phrase: response.reason_phrase().to_string(),
                    }).await;
                }
            },
            
            _ => {
                debug!("Other provisional response {} for dialog {}", response.status_code(), dialog_id);
                
                // Emit general call progress event
                self.emit_session_coordination_event(SessionCoordinationEvent::CallProgress {
                    dialog_id: dialog_id.clone(),
                    status_code: response.status_code(),
                    reason_phrase: response.reason_phrase().to_string(),
                }).await;
            }
        }
        
        Ok(())
    }
    
    /// Handle transaction termination
    async fn handle_transaction_terminated(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
    ) -> DialogResult<()> {
        debug!("Transaction {} terminated for dialog {}", transaction_id, dialog_id);
        
        // Clean up transaction-dialog association
        self.transaction_to_dialog.remove(transaction_id);
        
        // Note: We don't automatically terminate dialogs when transactions terminate
        // because dialogs can have multiple transactions. Dialog termination is
        // handled by higher-level logic (session-core) or explicit BYE requests.
        
        Ok(())
    }
    
    /// Handle ACK received event (RFC 3261 compliant media start point for UAS)
    async fn handle_ack_received_event(
        &self,
        dialog_id: &DialogId,
        transaction_id: &TransactionKey,
        request: rvoip_sip_core::Request,
    ) -> DialogResult<()> {
        info!("✅ RFC 3261: ACK received for transaction {} in dialog {} - time to start media (UAS side)", transaction_id, dialog_id);

        // Extract any SDP from the ACK (though typically ACK doesn't have SDP for 2xx responses)
        let negotiated_sdp = if !request.body().is_empty() {
            let sdp = String::from_utf8_lossy(request.body()).to_string();
            info!("ACK contains SDP body: {}", sdp);
            Some(sdp)
        } else {
            info!("ACK has no SDP body (normal for 2xx ACK)");
            None
        };

        info!("🔔 About to emit AckReceived event for dialog {}", dialog_id);

        // RFC 3261 COMPLIANT: Emit ACK received event for UAS side media creation
        self.emit_session_coordination_event(SessionCoordinationEvent::AckReceived {
            dialog_id: dialog_id.clone(),
            transaction_id: transaction_id.clone(),
            negotiated_sdp,
        }).await;

        info!("🚀 RFC 3261: Emitted AckReceived event for UAS side media creation");
        Ok(())
    }
}

// Additional transaction integration methods for DialogManager
impl DialogManager {
    /// Create server transaction for incoming request
    /// 
    /// Helper to create server transactions with proper error handling.
    pub async fn create_server_transaction_for_request(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<TransactionKey> {
        debug!("Creating server transaction for {} request from {}", request.method(), source);
        
        let server_transaction = self.transaction_manager
            .create_server_transaction(request, source)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to create server transaction: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        debug!("Created server transaction {} for request", transaction_id);
        Ok(transaction_id)
    }
    
    /// Create client transaction for outgoing request
    /// 
    /// Helper to create client transactions with method-specific handling.
    pub async fn create_client_transaction_for_request(
        &self,
        request: Request,
        destination: SocketAddr,
        method: &Method,
    ) -> DialogResult<TransactionKey> {
        debug!("Creating client transaction for {} request to {}", method, destination);
        
        let transaction_id = if *method == Method::Invite {
            self.transaction_manager
                .create_invite_client_transaction(request, destination)
                .await
        } else {
            self.transaction_manager
                .create_non_invite_client_transaction(request, destination)
                .await
        }.map_err(|e| crate::errors::DialogError::TransactionError {
            message: format!("Failed to create {} client transaction: {}", method, e),
        })?;
        
        debug!("Created client transaction {} for {} request", transaction_id, method);
        Ok(transaction_id)
    }
    
    /// Cancel an INVITE transaction using transaction-core
    /// 
    /// Properly cancels INVITE transactions while updating associated dialogs.
    pub async fn cancel_invite_transaction_with_dialog(
        &self,
        invite_tx_id: &TransactionKey,
    ) -> DialogResult<TransactionKey> {
        debug!("Cancelling INVITE transaction {} with dialog cleanup", invite_tx_id);
        
        // Find and terminate associated dialog
        if let Some(dialog_id) = self.transaction_to_dialog.get(invite_tx_id) {
            let dialog_id = dialog_id.clone();
            
            {
                if let Ok(mut dialog) = self.get_dialog_mut(&dialog_id) {
                    dialog.terminate();
                    debug!("Terminated dialog {} due to INVITE cancellation", dialog_id);
                }
            }
            
            // Send session coordination event
            if let Some(ref coordinator) = self.session_coordinator.read().await.as_ref() {
                let event = crate::events::SessionCoordinationEvent::CallCancelled {
                    dialog_id: dialog_id.clone(),
                    reason: "INVITE transaction cancelled".to_string(),
                };
                
                if let Err(e) = coordinator.send(event).await {
                    warn!("Failed to send call cancellation event: {}", e);
                }
            }
        }
        
        // Cancel the transaction using transaction-core
        let cancel_tx_id = self.transaction_manager
            .cancel_invite_transaction(invite_tx_id)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to cancel INVITE transaction: {}", e),
            })?;
        
        debug!("Successfully cancelled INVITE transaction {}, created CANCEL transaction {}", invite_tx_id, cancel_tx_id);
        Ok(cancel_tx_id)
    }
    
    /// Get transaction statistics
    /// 
    /// Provides insight into transaction-dialog associations.
    pub fn get_transaction_statistics(&self) -> (usize, usize) {
        let dialog_count = self.dialogs.len();
        let transaction_mapping_count = self.transaction_to_dialog.len();
        
        debug!("Transaction statistics: {} dialogs, {} transaction mappings", dialog_count, transaction_mapping_count);
        (dialog_count, transaction_mapping_count)
    }
    
    /// Cleanup orphaned transaction mappings
    /// 
    /// Removes transaction-dialog mappings for terminated dialogs.
    pub async fn cleanup_orphaned_transaction_mappings(&self) -> usize {
        let mut orphaned_count = 0;
        let active_dialog_ids: std::collections::HashSet<crate::dialog::DialogId> = 
            self.dialogs.iter().map(|entry| entry.key().clone()).collect();
        
        // Collect orphaned transaction IDs
        let orphaned_transactions: Vec<TransactionKey> = self.transaction_to_dialog
            .iter()
            .filter_map(|entry| {
                let dialog_id = entry.value();
                if !active_dialog_ids.contains(dialog_id) {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .collect();
        
        // Remove orphaned mappings
        for tx_id in orphaned_transactions {
            self.transaction_to_dialog.remove(&tx_id);
            orphaned_count += 1;
        }
        
        if orphaned_count > 0 {
            debug!("Cleaned up {} orphaned transaction mappings", orphaned_count);
        }
        
        orphaned_count
    }
} 