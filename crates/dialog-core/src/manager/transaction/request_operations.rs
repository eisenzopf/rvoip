//! Transaction Request Operations
//!
//! This module implements request sending operations through the transaction layer,
//! including proper dialog-aware request building using Phase 3 dialog functions.

use tracing::debug;

use rvoip_sip_core::{Request, Response, Method};
use rvoip_dialog_core::TransactionKey;
use rvoip_dialog_core::builders::{dialog_utils, dialog_quick};
use rvoip_dialog_core::utils::DialogRequestTemplate;
use crate::errors::DialogResult;
use crate::dialog::DialogId;
use crate::manager::core::DialogManager;
use super::traits::{TransactionIntegration, TransactionHelpers};

/// Implementation of TransactionIntegration for DialogManager
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
            let local_tag = match &template.local_tag {
                Some(tag) if !tag.is_empty() => tag.clone(),
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
                    return Err(crate::errors::DialogError::protocol_error(
                        &format!("{} request in confirmed dialog missing remote tag", method)
                    ));
                },
                
                // For early/initial dialogs, remote tag may be None (will be set to None, not empty string)
                _ => None
            };
            
            // Build request using Phase 3 dialog quick functions (MUCH simpler!)
            let request = self.build_dialog_request(
                &template, 
                method.clone(), 
                local_tag, 
                remote_tag, 
                body_string
            )?;
            
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

/// Implementation of TransactionHelpers for DialogManager
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

/// Helper methods for request building
impl DialogManager {
    /// Build a SIP request using Phase 3 dialog functions
    /// 
    /// This method encapsulates the complex request building logic using
    /// transaction-core's Phase 3 dialog quick functions.
    fn build_dialog_request(
        &self,
        template: &DialogRequestTemplate,
        method: Method,
        local_tag: String,
        remote_tag: Option<String>,
        body_string: Option<String>,
    ) -> DialogResult<Request> {
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
                        use rvoip_dialog_core::client::builders::InviteBuilder;
                        
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
                
                let target_uri = body_string.clone().unwrap_or_else(|| "sip:unknown".to_string());
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
                    body_string, // SDP content
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
                
                let event_type = "dialog"; // This should come from dialog context
                dialog_quick::notify_for_dialog(
                    &template.call_id,
                    &template.local_uri.to_string(),
                    &local_tag,
                    &template.remote_uri.to_string(),
                    &remote_tag,
                    event_type,
                    body_string,
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
                let template_struct = dialog_utils::DialogRequestTemplate {
                    call_id: template.call_id.clone(),
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
                
                dialog_utils::request_builder_from_dialog_template(
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
        
        Ok(request)
    }
} 