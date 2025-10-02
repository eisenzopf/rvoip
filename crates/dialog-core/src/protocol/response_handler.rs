//! SIP Response Handler for Dialog-Core
//!
//! This module handles processing of SIP responses within dialogs according to RFC 3261.
//! It manages dialog state transitions based on response status codes and coordinates
//! with the session layer for proper call management.
//!
//! ## Response Categories Handled
//!
//! - **1xx Provisional**: Call progress, ringing, session progress
//! - **2xx Success**: Call answered, request completed successfully
//! - **3xx Redirection**: Call forwarding and redirect scenarios
//! - **4xx Client Error**: Authentication, not found, bad request
//! - **5xx Server Error**: Server failures and overload conditions
//! - **6xx Global Failure**: Permanent failures and rejections
//!
//! ## Dialog State Management
//!
//! - **180 Ringing**: May create early dialog with To-tag
//! - **200 OK INVITE**: Confirms dialog, transitions Early→Confirmed
//! - **4xx-6xx INVITE**: Terminates early dialogs
//! - **200 OK BYE**: Completes dialog termination

use tracing::{debug, info, warn};

use rvoip_sip_core::Response;
use crate::transaction::TransactionKey;
use crate::dialog::{DialogId, DialogState};
use crate::errors::DialogResult;
use crate::events::SessionCoordinationEvent;
use crate::manager::{DialogManager, SessionCoordinator, MessageExtensions};

/// Response-specific handling operations
pub trait ResponseHandler {
    /// Handle responses to client transactions
    fn handle_response_message(
        &self,
        response: Response,
        transaction_id: TransactionKey,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of response handling for DialogManager
impl ResponseHandler for DialogManager {
    /// Handle responses to client transactions
    /// 
    /// Processes responses and updates dialog state accordingly.
    async fn handle_response_message(&self, response: Response, transaction_id: TransactionKey) -> DialogResult<()> {
        debug!("Processing response {} for transaction {}", response.status_code(), transaction_id);
        
        // Find associated dialog
        if let Ok(dialog_id) = self.find_dialog_for_transaction(&transaction_id) {
            self.process_response_in_dialog(response, transaction_id, dialog_id).await
        } else {
            debug!("Response for transaction {} has no associated dialog", transaction_id);
            Ok(())
        }
    }
}

/// Response-specific helper methods for DialogManager
impl DialogManager {
    /// Process response within a dialog
    pub async fn process_response_in_dialog(
        &self,
        response: Response,
        _transaction_id: TransactionKey,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        debug!("Processing response {} for dialog {}", response.status_code(), dialog_id);
        
        // Update dialog state based on response
        {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            
            if response.status_code() >= 200 && response.status_code() < 300 {
                // 2xx response - confirm dialog if in Early state
                if dialog.state == DialogState::Early {
                    if let Some(to_tag) = response.to().and_then(|to| to.tag()) {
                        dialog.confirm_with_tag(to_tag.to_string());
                        debug!("Confirmed dialog {} with 2xx response", dialog_id);
                    }
                }
            } else if response.status_code() >= 300 {
                // 3xx+ response - terminate dialog
                dialog.terminate();
                debug!("Terminated dialog {} due to final non-2xx response", dialog_id);
            }
        }
        
        // Send appropriate session coordination event
        let event = if response.status_code() >= 200 && response.status_code() < 300 {
            SessionCoordinationEvent::CallAnswered {
                dialog_id: dialog_id.clone(),
                session_answer: response.body_string().unwrap_or_default(),
            }
        } else if response.status_code() >= 300 {
            SessionCoordinationEvent::CallTerminated {
                dialog_id: dialog_id.clone(),
                reason: format!("{} {}", response.status_code(), response.reason_phrase()),
            }
        } else {
            SessionCoordinationEvent::CallProgress {
                dialog_id: dialog_id.clone(),
                status_code: response.status_code(),
                reason_phrase: response.reason_phrase().to_string(),
            }
        };
        
        self.notify_session_layer(event).await?;
        debug!("Response processed for dialog {}", dialog_id);
        Ok(())
    }
    
    /// Handle provisional responses (1xx)
    pub async fn handle_provisional_response(
        &self,
        response: Response,
        _transaction_id: TransactionKey,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        debug!("Processing provisional response {} for dialog {}", response.status_code(), dialog_id);
        
        // Update dialog state for early dialogs
        let dialog_created = {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            let old_state = dialog.state.clone();
            
            // For provisional responses with to-tag, create early dialog
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    if dialog.remote_tag.is_none() {
                        dialog.set_remote_tag(to_tag.to_string());
                        if dialog.state == DialogState::Initial {
                            dialog.state = DialogState::Early;
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
            self.emit_dialog_event(crate::events::DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            }).await;
        }
        
        // Handle specific provisional responses and emit session coordination events
        match response.status_code() {
            180 => {
                info!("Call ringing for dialog {}", dialog_id);
                
                self.notify_session_layer(SessionCoordinationEvent::CallRinging {
                    dialog_id: dialog_id.clone(),
                }).await?;
            },
            
            183 => {
                info!("Session progress for dialog {}", dialog_id);
                
                // Check for early media (SDP in 183)
                if !response.body().is_empty() {
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.notify_session_layer(SessionCoordinationEvent::EarlyMedia {
                        dialog_id: dialog_id.clone(),
                        sdp,
                    }).await?;
                } else {
                    self.notify_session_layer(SessionCoordinationEvent::CallProgress {
                        dialog_id: dialog_id.clone(),
                        status_code: response.status_code(),
                        reason_phrase: response.reason_phrase().to_string(),
                    }).await?;
                }
            },
            
            _ => {
                debug!("Other provisional response {} for dialog {}", response.status_code(), dialog_id);
                
                // Emit general call progress event
                self.notify_session_layer(SessionCoordinationEvent::CallProgress {
                    dialog_id: dialog_id.clone(),
                    status_code: response.status_code(),
                    reason_phrase: response.reason_phrase().to_string(),
                }).await?;
            }
        }
        
        Ok(())
    }
    
    /// Handle successful responses (2xx)
    pub async fn handle_success_response(
        &self,
        response: Response,
        transaction_id: TransactionKey,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        info!("Processing success response {} for dialog {}", response.status_code(), dialog_id);
        
        // Update dialog state based on successful response
        let dialog_state_changed = {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            let old_state = dialog.state.clone();
            
            // Update dialog with response information (remote tag, etc.)
            if let Some(to_header) = response.to() {
                if let Some(to_tag) = to_header.tag() {
                    dialog.set_remote_tag(to_tag.to_string());
                }
            }
            
            // Update dialog state based on response status and current state
            let state_changed = match response.status_code() {
                200 => {
                    if dialog.state == DialogState::Early {
                        dialog.state = DialogState::Confirmed;
                        
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
            self.emit_dialog_event(crate::events::DialogEvent::StateChanged {
                dialog_id: dialog_id.clone(),
                old_state,
                new_state,
            }).await;
        }
        
        // Emit session coordination events for session-core
        self.notify_session_layer(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
        }).await?;
        
        // Handle specific successful response types
        match response.status_code() {
            200 => {
                println!("🎯 RESPONSE HANDLER: Processing 200 OK, checking if INVITE response needs ACK");
                
                // For 200 OK responses to INVITE, automatically send ACK
                // Check if this is a response to an INVITE by looking at the transaction
                if let Some(original_request_method) = self.get_transaction_method(&transaction_id) {
                    if original_request_method == rvoip_sip_core::Method::Invite {
                        println!("🚀 RESPONSE HANDLER: This is a 200 OK to INVITE - sending automatic ACK");
                        
                        // Create and send ACK for this 2xx response
                        if let Err(e) = self.send_automatic_ack_for_2xx(&transaction_id, &response, &dialog_id).await {
                            warn!("Failed to send automatic ACK for 200 OK to INVITE: {}", e);
                        } else {
                            info!("Successfully sent automatic ACK for 200 OK to INVITE");

                            // Notify session-core that ACK was sent (for state machine transition)
                            // Extract SDP if present for final negotiation
                            let negotiated_sdp = if !response.body().is_empty() {
                                Some(String::from_utf8_lossy(response.body()).to_string())
                            } else {
                                None
                            };

                            if let Err(e) = self.notify_session_layer(SessionCoordinationEvent::AckSent {
                                dialog_id: dialog_id.clone(),
                                transaction_id: transaction_id.clone(),
                                negotiated_sdp,
                            }).await {
                                warn!("Failed to notify session layer of ACK sent: {}", e);
                            }
                        }
                    }
                }
                
                // Successful completion - could be call answered, request completed, etc.
                if !response.body().is_empty() {
                    let sdp = String::from_utf8_lossy(response.body()).to_string();
                    self.notify_session_layer(SessionCoordinationEvent::CallAnswered {
                        dialog_id: dialog_id.clone(),
                        session_answer: sdp,
                    }).await?;
                }
            },
            _ => {
                debug!("Other successful response {} for dialog {}", response.status_code(), dialog_id);
            }
        }
        
        Ok(())
    }
    
    /// Handle failure responses (4xx, 5xx, 6xx)
    pub async fn handle_failure_response(
        &self,
        response: Response,
        transaction_id: TransactionKey,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        warn!("Processing failure response {} for dialog {}", response.status_code(), dialog_id);
        
        // Handle specific failure cases and emit appropriate events
        match response.status_code() {
            487 => {
                // Request Terminated (CANCEL received)
                info!("Call cancelled for dialog {}", dialog_id);
                
                // Emit dialog event
                self.emit_dialog_event(crate::events::DialogEvent::Terminated {
                    dialog_id: dialog_id.clone(),
                    reason: "Request terminated".to_string(),
                }).await;
                
                // Emit session coordination event
                self.notify_session_layer(SessionCoordinationEvent::CallCancelled {
                    dialog_id: dialog_id.clone(),
                    reason: "Request terminated".to_string(),
                }).await?;
            },
            
            status if status >= 400 && status < 500 => {
                // Client error - may require dialog termination
                warn!("Client error {} for dialog {} - considering termination", status, dialog_id);
                
                // Emit session coordination event for failed requests
                self.notify_session_layer(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: "Unknown".to_string(), // TODO: Extract from transaction context
                }).await?;
            },
            
            status if status >= 500 => {
                // Server error - may require retry or termination
                warn!("Server error {} for dialog {} - considering retry", status, dialog_id);
                
                // Emit session coordination event for server errors
                self.notify_session_layer(SessionCoordinationEvent::RequestFailed {
                    dialog_id: Some(dialog_id.clone()),
                    transaction_id: transaction_id.clone(),
                    status_code: status,
                    reason_phrase: response.reason_phrase().to_string(),
                    method: "Unknown".to_string(), // TODO: Extract from transaction context
                }).await?;
            },
            
            _ => {
                debug!("Other failure response {} for dialog {}", response.status_code(), dialog_id);
            }
        }
        
        // Always emit the response received event for session-core to handle
        self.notify_session_layer(SessionCoordinationEvent::ResponseReceived {
            dialog_id: dialog_id.clone(),
            response: response.clone(),
            transaction_id: transaction_id.clone(),
        }).await?;
        
        Ok(())
    }
    
    /// Get the original request method for a transaction
    /// 
    /// This is a simplified implementation - in a real system this would
    /// query the transaction manager for the original request method.
    fn get_transaction_method(&self, transaction_id: &TransactionKey) -> Option<rvoip_sip_core::Method> {
        // Extract method from transaction key (simplified approach)
        // The transaction key typically contains the method information
        if transaction_id.to_string().contains("INVITE") {
            Some(rvoip_sip_core::Method::Invite)
        } else if transaction_id.to_string().contains("BYE") {
            Some(rvoip_sip_core::Method::Bye)
        } else {
            // For now, assume it's INVITE if we can't determine
            // In a real implementation, this would query the transaction manager
            Some(rvoip_sip_core::Method::Invite)
        }
    }
    
    /// Send automatic ACK for 2xx response to INVITE
    /// 
    /// Uses the existing dialog-core → transaction-core → transport architecture
    /// to properly send ACKs according to RFC 3261 while maintaining separation of concerns.
    async fn send_automatic_ack_for_2xx(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
        dialog_id: &DialogId,
    ) -> DialogResult<()> {
        debug!("Sending automatic ACK for 2xx response to INVITE using proper architecture");
        
        println!("📧 RESPONSE HANDLER: Using existing send_ack_for_2xx_response method");
        
        // Use the existing dialog-core method that properly delegates to transaction-core
        // This maintains separation of concerns: dialog-core → transaction-core → transport
        self.send_ack_for_2xx_response(dialog_id, original_invite_tx_id, response).await?;
        
        println!("✅ RESPONSE HANDLER: Successfully sent ACK for 2xx response via proper channels");
        Ok(())
    }
} 