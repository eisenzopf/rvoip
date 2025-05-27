use tracing::{debug, error};
use std::str::FromStr;
use std::net::SocketAddr;
use std::time::SystemTime;

use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};
use rvoip_transaction_core::{TransactionKey, TransactionKind};

use super::manager::DialogManager;
use super::dialog_state::DialogState;
use super::dialog_id::DialogId;
use super::dialog_impl::Dialog;
use super::dialog_utils::uri_resolver;
use crate::errors::{Error, ErrorContext, ErrorCategory, ErrorSeverity, RecoveryAction};
use crate::events::SessionEvent;
use crate::session::SessionId;
use crate::{dialog_not_found_error, network_unreachable_error, transaction_creation_error, transaction_send_error};

impl DialogManager {
    /// Find the dialog for an in-dialog request
    pub(super) fn find_dialog_for_request(&self, request: &Request) -> Option<DialogId> {
        // Extract call-id
        let call_id = match request.call_id() {
            Some(call_id) => call_id.to_string(),
            _ => {
                debug!("No Call-ID found in request");
                return None;
            }
        };
        
        // Extract tags
        let from_tag = request.from().and_then(|from| from.tag().map(|s| s.to_string()));
        let to_tag = request.to().and_then(|to| to.tag().map(|s| s.to_string()));
        
        debug!("Looking for dialog: Call-ID={}, From-tag={:?}, To-tag={:?}", 
               call_id, from_tag, to_tag);
        
        // Both tags are required for dialog lookup
        if from_tag.is_none() || to_tag.is_none() {
            debug!("Missing tags - From-tag: {:?}, To-tag: {:?}", from_tag, to_tag);
            return None;
        }
        
        let from_tag = from_tag.unwrap();
        let to_tag = to_tag.unwrap();
        
        // Debug: Show all stored dialog tuples
        debug!("Currently stored dialog tuples:");
        for entry in self.dialog_lookup.iter() {
            debug!("  Stored tuple: {:?} -> {}", entry.key(), entry.value());
        }
        
        // Try to find a matching dialog - check both scenarios
        
        // Scenario 1: Local is From, Remote is To
        let id_tuple1 = (call_id.clone(), from_tag.clone(), to_tag.clone());
        debug!("Trying scenario 1 (Local=From, Remote=To): {:?}", id_tuple1);
        if let Some(dialog_id_ref) = self.dialog_lookup.get(&id_tuple1) {
            let dialog_id = dialog_id_ref.value().clone();
            drop(dialog_id_ref);
            debug!("Found dialog {} with scenario 1", dialog_id);
            return Some(dialog_id);
        }
        
        // Scenario 2: Local is To, Remote is From
        let id_tuple2 = (call_id, to_tag, from_tag);
        debug!("Trying scenario 2 (Local=To, Remote=From): {:?}", id_tuple2);
        if let Some(dialog_id_ref) = self.dialog_lookup.get(&id_tuple2) {
            let dialog_id = dialog_id_ref.value().clone();
            drop(dialog_id_ref);
            debug!("Found dialog {} with scenario 2", dialog_id);
            return Some(dialog_id);
        }
        
        // No matching dialog found
        debug!("No matching dialog found for request");
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
        debug!("Creating dialog from transaction: {} (is_initiator={})", transaction_id, is_initiator);
        
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
            debug!("Created dialog with ID: {} (is_initiator={}, local_tag={:?}, remote_tag={:?})", 
                   dialog_id, dialog.is_initiator, dialog.local_tag, dialog.remote_tag);
            
            // Store the dialog
            self.dialogs.insert(dialog_id.clone(), dialog.clone());
            
            // Associate the transaction with this dialog
            self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
            
            // Save dialog tuple mapping if available
            if let Some(tuple) = dialog.dialog_id_tuple() {
                debug!("Mapping dialog tuple to dialog ID: {:?} -> {} (call_id={}, local_tag={:?}, remote_tag={:?})", 
                       tuple, dialog_id, dialog.call_id, dialog.local_tag, dialog.remote_tag);
                self.dialog_lookup.insert(tuple, dialog_id.clone());
            } else {
                debug!("No dialog tuple available for dialog {} (missing tags)", dialog_id);
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
    
    /// Handle an incoming provisional response which may create an early dialog
    pub(super) async fn handle_provisional_response(&self, transaction_id: &TransactionKey, response: Response) {
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
    pub(super) async fn handle_success_response(&self, transaction_id: &TransactionKey, response: Response) {
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
    pub(super) async fn handle_bye_request(&self, transaction_id: &TransactionKey, request: Request) {
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
    pub(super) fn has_to_tag(&self, response: &Response) -> bool {
        response.to().and_then(|to| to.tag()).is_some()
    }
    
    /// Get the original request from a transaction
    pub(super) async fn get_transaction_request(&self, transaction_id: &TransactionKey) -> Result<Option<Request>, Error> {
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
                        request.headers.push(rvoip_sip_core::TypedHeader::CallId(call_id));
                        
                        // Add From with tag using proper API
                        let from_uri = dialog.local_uri.clone();
                        let mut from_addr = rvoip_sip_core::types::address::Address::new(from_uri);
                        if let Some(tag) = &dialog.local_tag {
                            from_addr.set_tag(tag);
                        }
                        let from = rvoip_sip_core::types::from::From(from_addr);
                        request.headers.push(rvoip_sip_core::TypedHeader::From(from));
                        
                        // Add To with remote tag using proper API
                        let to_uri = dialog.remote_uri.clone();
                        let mut to_addr = rvoip_sip_core::types::address::Address::new(to_uri);
                        if let Some(tag) = &dialog.remote_tag {
                            to_addr.set_tag(tag);
                        }
                        let to = rvoip_sip_core::types::to::To(to_addr);
                        request.headers.push(rvoip_sip_core::TypedHeader::To(to));
                        
                        // Add CSeq
                        let cseq = rvoip_sip_core::types::cseq::CSeq::new(dialog.local_seq, method);
                        request.headers.push(rvoip_sip_core::TypedHeader::CSeq(cseq));
                        
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
    
    /// Find the dialog associated with a transaction
    pub fn find_dialog_for_transaction(&self, transaction_id: &TransactionKey) -> Option<DialogId> {
        if self.transaction_to_dialog.contains_key(transaction_id) {
            self.transaction_to_dialog.get(transaction_id).map(|id| id.clone())
        } else {
            None
        }
    }
} 