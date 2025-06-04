//! SIP Protocol Handlers for Dialog Management
//!
//! This module implements handlers for all SIP methods that are relevant to
//! dialog management, following RFC 3261 and related specifications.

use std::net::SocketAddr;
use tracing::{debug, error, info};

use rvoip_sip_core::{Request, Response, Method, StatusCode};
use rvoip_transaction_core::TransactionKey;
use rvoip_transaction_core::utils::response_builders;
use crate::dialog::{DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use super::core::DialogManager;
use super::utils::{MessageExtensions, SourceExtractor};
use super::dialog_operations::DialogLookup;
use super::session_coordination::SessionCoordinator;

/// Trait for SIP method handling
pub trait ProtocolHandlers {
    /// Handle INVITE requests (dialog-creating and re-INVITE)
    fn handle_invite_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle BYE requests (dialog-terminating)
    fn handle_bye_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle CANCEL requests (transaction-cancelling)
    fn handle_cancel_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle ACK requests (transaction-completing)
    fn handle_ack_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle OPTIONS requests (capability discovery)
    fn handle_options_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle UPDATE requests (session modification)
    fn handle_update_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle responses to client transactions
    fn handle_response_message(
        &self,
        response: Response,
        transaction_id: TransactionKey,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Trait for specific method handling
pub trait MethodHandler {
    /// Handle REGISTER requests (non-dialog)
    fn handle_register_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle INFO requests (mid-dialog)
    fn handle_info_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle REFER requests (call transfer)
    fn handle_refer_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle SUBSCRIBE requests (event subscription)
    fn handle_subscribe_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
    
    /// Handle NOTIFY requests (event notification)
    fn handle_notify_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

// Implement ProtocolHandlers for DialogManager
impl ProtocolHandlers for DialogManager {
    /// Handle INVITE requests according to RFC 3261 Section 14
    /// 
    /// Supports both initial INVITE (dialog-creating) and re-INVITE (session modification).
    async fn handle_invite_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing INVITE request from {}", source);
        
        // Create server transaction using transaction-core
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for INVITE: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        // Check if this is an initial INVITE or re-INVITE
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            // This is a re-INVITE within existing dialog
            self.handle_reinvite(transaction_id, request, dialog_id).await
        } else {
            // This is an initial INVITE - create new dialog
            self.handle_initial_invite(transaction_id, request, source).await
        }
    }
    
    /// Handle BYE requests according to RFC 3261 Section 15
    /// 
    /// Terminates the dialog and sends appropriate responses.
    async fn handle_bye_method(&self, request: Request) -> DialogResult<()> {
        debug!("Processing BYE request");
        
        let source = SourceExtractor::extract_from_request(&request);
        
        // Create server transaction
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for BYE: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        // Find the dialog for this BYE
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            self.process_bye_in_dialog(transaction_id, request, dialog_id).await
        } else {
            // Send 481 Call/Transaction Does Not Exist using transaction-core helper
            let response = response_builders::create_response(&request, StatusCode::CallOrTransactionDoesNotExist);
            self.transaction_manager.send_response(&transaction_id, response).await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to BYE: {}", e),
                })?;
            
            Err(DialogError::dialog_not_found("BYE request dialog"))
        }
    }
    
    /// Handle CANCEL requests according to RFC 3261 Section 9
    /// 
    /// Cancels pending INVITE transactions and terminates early dialogs.
    async fn handle_cancel_method(&self, request: Request) -> DialogResult<()> {
        debug!("Processing CANCEL request");
        
        // Find the INVITE transaction this CANCEL is for
        let invite_tx_id = self.transaction_manager
            .find_invite_transaction_for_cancel(&request)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to find INVITE transaction for CANCEL: {}", e),
            })?;
        
        if let Some(invite_tx_id) = invite_tx_id {
            self.process_cancel_for_invite(request, invite_tx_id).await
        } else {
            // No matching INVITE found, send 481
            let source = SourceExtractor::extract_from_request(&request);
            let server_transaction = self.transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for CANCEL: {}", e),
                })?;
            
            let transaction_id = server_transaction.id().clone();
            let response = response_builders::create_response(&request, StatusCode::CallOrTransactionDoesNotExist);
            
            self.transaction_manager.send_response(&transaction_id, response).await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to CANCEL: {}", e),
                })?;
            
            debug!("CANCEL processed with 481 response (no matching INVITE)");
            Ok(())
        }
    }
    
    /// Handle ACK requests according to RFC 3261 Section 17
    /// 
    /// Processes ACK for both 2xx and non-2xx responses.
    async fn handle_ack_method(&self, request: Request) -> DialogResult<()> {
        debug!("Processing ACK request");
        
        // ACK can be for 2xx response (goes to dialog) or non-2xx response (goes to transaction)
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            // Dialog-level ACK (for 2xx responses)
            self.process_ack_in_dialog(request, dialog_id).await
        } else {
            // Transaction-level ACK (for non-2xx responses)
            // These are handled automatically by transaction-core
            debug!("ACK for non-2xx response - handled by transaction layer");
            Ok(())
        }
    }
    
    /// Handle OPTIONS requests according to RFC 3261 Section 11
    /// 
    /// Provides capability discovery and keep-alive functionality.
    async fn handle_options_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing OPTIONS request from {}", source);
        
        // Create server transaction
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for OPTIONS: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        // Send session coordination event for capability query
        let event = SessionCoordinationEvent::CapabilityQuery {
            transaction_id: transaction_id.clone(),
            request: request.clone(),
            source,
        };
        
        if let Err(e) = self.notify_session_layer(event).await {
            error!("Failed to notify session layer of OPTIONS: {}", e);
            
            // Fallback: send basic 200 OK with supported methods
            self.send_basic_options_response(&transaction_id, &request).await?;
        }
        
        debug!("OPTIONS request processed");
        Ok(())
    }
    
    /// Handle UPDATE requests according to RFC 3311
    /// 
    /// Provides session modification within dialogs.
    async fn handle_update_method(&self, request: Request) -> DialogResult<()> {
        debug!("Processing UPDATE request");
        
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            self.process_update_in_dialog(request, dialog_id).await
        } else {
            // Send 481 Call/Transaction Does Not Exist
            let source = SourceExtractor::extract_from_request(&request);
            let server_transaction = self.transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for UPDATE: {}", e),
                })?;
            
            let transaction_id = server_transaction.id().clone();
            let response = response_builders::create_response(&request, StatusCode::CallOrTransactionDoesNotExist);
            
            self.transaction_manager.send_response(&transaction_id, response).await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to UPDATE: {}", e),
                })?;
            
            debug!("UPDATE processed with 481 response (no dialog found)");
            Ok(())
        }
    }
    
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

// Implement MethodHandler for DialogManager
impl MethodHandler for DialogManager {
    /// Handle REGISTER requests according to RFC 3261 Section 10
    /// 
    /// REGISTER requests don't create dialogs but are handled for completeness.
    async fn handle_register_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing REGISTER request from {}", source);
        
        // Extract registration information
        let from_uri = request.from()
            .ok_or_else(|| DialogError::protocol_error("REGISTER missing From header"))?
            .uri().clone();
        
        let contact_uri = self.extract_contact_uri(&request).unwrap_or_else(|| from_uri.clone());
        let expires = self.extract_expires(&request);
        
        // Create server transaction and send coordination event
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for REGISTER: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        let event = SessionCoordinationEvent::RegistrationRequest {
            transaction_id,
            from_uri,
            contact_uri,
            expires,
        };
        
        self.notify_session_layer(event).await?;
        debug!("REGISTER request forwarded to session layer");
        Ok(())
    }
    
    /// Handle INFO requests according to RFC 6086
    /// 
    /// Provides application-level information exchange within dialogs.
    async fn handle_info_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing INFO request from {}", source);
        
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            // Forward to session layer for application-specific handling
            let server_transaction = self.transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for INFO: {}", e),
                })?;
            
            let transaction_id = server_transaction.id().clone();
            
            // Create custom event for INFO (could extend SessionCoordinationEvent)
            let event = SessionCoordinationEvent::ReInvite {
                dialog_id: dialog_id.clone(),
                transaction_id,
                request: request.clone(),
            };
            
            self.notify_session_layer(event).await?;
            debug!("INFO request forwarded to session layer for dialog {}", dialog_id);
            Ok(())
        } else {
            // Send 481 Call/Transaction Does Not Exist
            let server_transaction = self.transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for INFO: {}", e),
                })?;
            
            let transaction_id = server_transaction.id().clone();
            let response = response_builders::create_response(&request, StatusCode::CallOrTransactionDoesNotExist);
            
            self.transaction_manager.send_response(&transaction_id, response).await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to INFO: {}", e),
                })?;
            
            debug!("INFO processed with 481 response (no dialog found)");
            Ok(())
        }
    }
    
    /// Handle REFER requests according to RFC 3515
    /// 
    /// Implements call transfer functionality.
    async fn handle_refer_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing REFER request from {}", source);
        
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            // Forward to session layer for call transfer handling
            let server_transaction = self.transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for REFER: {}", e),
                })?;
            
            let transaction_id = server_transaction.id().clone();
            
            // Use ReInvite event type for now (could be extended for REFER)
            let event = SessionCoordinationEvent::ReInvite {
                dialog_id: dialog_id.clone(),
                transaction_id,
                request: request.clone(),
            };
            
            self.notify_session_layer(event).await?;
            debug!("REFER request forwarded to session layer for dialog {}", dialog_id);
            Ok(())
        } else {
            // REFER outside dialog - send 481
            let server_transaction = self.transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for REFER: {}", e),
                })?;
            
            let transaction_id = server_transaction.id().clone();
            let response = response_builders::create_response(&request, StatusCode::CallOrTransactionDoesNotExist);
            
            self.transaction_manager.send_response(&transaction_id, response).await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to REFER: {}", e),
                })?;
            
            debug!("REFER processed with 481 response (no dialog found)");
            Ok(())
        }
    }
    
    /// Handle SUBSCRIBE requests according to RFC 6665
    /// 
    /// Implements event subscription functionality.
    async fn handle_subscribe_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing SUBSCRIBE request from {}", source);
        
        // SUBSCRIBE can create dialog-like state for event subscriptions
        // For now, forward to session layer for handling
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for SUBSCRIBE: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        // Use CapabilityQuery event type for now (could be extended for SUBSCRIBE)
        let event = SessionCoordinationEvent::CapabilityQuery {
            transaction_id,
            request: request.clone(),
            source,
        };
        
        self.notify_session_layer(event).await?;
        debug!("SUBSCRIBE request forwarded to session layer");
        Ok(())
    }
    
    /// Handle NOTIFY requests according to RFC 6665
    /// 
    /// Implements event notification functionality.
    async fn handle_notify_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing NOTIFY request from {}", source);
        
        // NOTIFY is typically within an existing subscription dialog
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            let server_transaction = self.transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for NOTIFY: {}", e),
                })?;
            
            let transaction_id = server_transaction.id().clone();
            
            let event = SessionCoordinationEvent::ReInvite {
                dialog_id: dialog_id.clone(),
                transaction_id,
                request: request.clone(),
            };
            
            self.notify_session_layer(event).await?;
            debug!("NOTIFY request forwarded to session layer for dialog {}", dialog_id);
            Ok(())
        } else {
            // NOTIFY outside dialog - could be unsolicited, send 481
            let server_transaction = self.transaction_manager
                .create_server_transaction(request.clone(), source)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to create server transaction for NOTIFY: {}", e),
                })?;
            
            let transaction_id = server_transaction.id().clone();
            let response = response_builders::create_response(&request, StatusCode::CallOrTransactionDoesNotExist);
            
            self.transaction_manager.send_response(&transaction_id, response).await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to NOTIFY: {}", e),
                })?;
            
            debug!("NOTIFY processed with 481 response (no dialog found)");
            Ok(())
        }
    }
}

// Private helper methods for DialogManager
impl DialogManager {
    /// Handle initial INVITE (dialog-creating)
    async fn handle_initial_invite(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<()> {
        debug!("Processing initial INVITE request");
        
        // Create early dialog
        let dialog_id = self.create_early_dialog_from_invite(&request).await?;
        
        // Associate transaction with dialog
        self.associate_transaction_with_dialog(&transaction_id, &dialog_id);
        
        // Send session coordination event
        let event = SessionCoordinationEvent::IncomingCall {
            dialog_id: dialog_id.clone(),
            transaction_id: transaction_id.clone(),
            request: request.clone(),
            source,
        };
        
        self.notify_session_layer(event).await?;
        info!("Initial INVITE processed, created dialog {}", dialog_id);
        Ok(())
    }
    
    /// Handle re-INVITE (session modification)
    async fn handle_reinvite(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        debug!("Processing re-INVITE for dialog {}", dialog_id);
        
        // Associate transaction with dialog
        self.associate_transaction_with_dialog(&transaction_id, &dialog_id);
        
        // Update dialog sequence number
        {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            dialog.update_remote_sequence(&request)?;
        }
        
        // Send session coordination event
        let event = SessionCoordinationEvent::ReInvite {
            dialog_id: dialog_id.clone(),
            transaction_id: transaction_id.clone(),
            request: request.clone(),
        };
        
        self.notify_session_layer(event).await?;
        info!("Re-INVITE processed for dialog {}", dialog_id);
        Ok(())
    }
    
    /// Process BYE within a dialog
    async fn process_bye_in_dialog(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        debug!("Processing BYE for dialog {}", dialog_id);
        
        // Update dialog and terminate
        {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            dialog.update_remote_sequence(&request)?;
            dialog.terminate();
        }
        
        // Send 200 OK response
        let response = response_builders::create_response(&request, StatusCode::Ok);
        self.transaction_manager.send_response(&transaction_id, response).await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send 200 OK to BYE: {}", e),
            })?;
        
        // Send session coordination event
        let event = SessionCoordinationEvent::CallTerminated {
            dialog_id: dialog_id.clone(),
            reason: "BYE received".to_string(),
        };
        
        self.notify_session_layer(event).await?;
        info!("BYE processed for dialog {}", dialog_id);
        Ok(())
    }
    
    /// Process CANCEL for INVITE transaction
    async fn process_cancel_for_invite(
        &self,
        _request: Request,
        invite_tx_id: TransactionKey,
    ) -> DialogResult<()> {
        debug!("Processing CANCEL for INVITE transaction {}", invite_tx_id);
        
        // Find associated dialog and terminate it
        if let Ok(dialog_id) = self.find_dialog_for_transaction(&invite_tx_id) {
            {
                let mut dialog = self.get_dialog_mut(&dialog_id)?;
                dialog.terminate();
            }
            
            // Send session coordination event
            let event = SessionCoordinationEvent::CallCancelled {
                dialog_id: dialog_id.clone(),
                reason: "CANCEL received".to_string(),
            };
            
            self.notify_session_layer(event).await?;
        }
        
        // Cancel the INVITE transaction
        let _cancel_tx_id = self.transaction_manager
            .cancel_invite_transaction(&invite_tx_id)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to cancel INVITE transaction: {}", e),
            })?;
        
        info!("CANCEL processed for INVITE transaction {}", invite_tx_id);
        Ok(())
    }
    
    /// Process ACK within a dialog
    async fn process_ack_in_dialog(&self, request: Request, dialog_id: DialogId) -> DialogResult<()> {
        debug!("Processing ACK for dialog {}", dialog_id);
        
        // Update dialog state if in Early state
        {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            
            if dialog.state == DialogState::Early {
                // Extract local tag from ACK
                if let Some(to_tag) = request.to().and_then(|to| to.tag()) {
                    dialog.confirm_with_tag(to_tag.to_string());
                    debug!("Confirmed dialog {} with ACK", dialog_id);
                }
            }
            
            dialog.update_remote_sequence(&request)?;
        }
        
        // Send session coordination event
        let event = SessionCoordinationEvent::CallAnswered {
            dialog_id: dialog_id.clone(),
            session_answer: request.body_string().unwrap_or_default(),
        };
        
        self.notify_session_layer(event).await?;
        debug!("ACK processed for dialog {}", dialog_id);
        Ok(())
    }
    
    /// Process UPDATE within a dialog
    async fn process_update_in_dialog(&self, request: Request, dialog_id: DialogId) -> DialogResult<()> {
        debug!("Processing UPDATE for dialog {}", dialog_id);
        
        // Update dialog sequence number
        {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            dialog.update_remote_sequence(&request)?;
        }
        
        // Create server transaction and forward to session layer
        let source = SourceExtractor::extract_from_request(&request);
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for UPDATE: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        let event = SessionCoordinationEvent::ReInvite {
            dialog_id: dialog_id.clone(),
            transaction_id,
            request: request.clone(),
        };
        
        self.notify_session_layer(event).await?;
        debug!("UPDATE processed for dialog {}", dialog_id);
        Ok(())
    }
    
    /// Process response within a dialog
    async fn process_response_in_dialog(
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
    
    /// Send basic OPTIONS response with supported methods
    async fn send_basic_options_response(
        &self,
        transaction_id: &TransactionKey,
        request: &Request,
    ) -> DialogResult<()> {
        // Use transaction-core helper for OPTIONS response with Allow header
        let allowed_methods = vec![
            Method::Invite,
            Method::Bye,
            Method::Cancel,
            Method::Ack,
            Method::Options,
            Method::Update,
            Method::Info,
            Method::Refer,
        ];
        
        let response = response_builders::create_ok_response_for_options(request, &allowed_methods);
        
        self.transaction_manager.send_response(transaction_id, response).await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send OPTIONS response: {}", e),
            })?;
        
        debug!("Sent basic OPTIONS response");
        Ok(())
    }
    
    /// Extract Contact URI from request
    fn extract_contact_uri(&self, request: &Request) -> Option<rvoip_sip_core::Uri> {
        request.typed_header::<rvoip_sip_core::types::contact::Contact>()
            .and_then(|contact| contact.0.first())
            .and_then(|contact_val| {
                match contact_val {
                    rvoip_sip_core::types::contact::ContactValue::Params(params) => {
                        params.first().map(|p| p.address.uri.clone())
                    },
                    _ => None,
                }
            })
    }
    
    /// Extract Expires value from request
    fn extract_expires(&self, request: &Request) -> u32 {
        request.typed_header::<rvoip_sip_core::types::expires::Expires>()
            .map(|exp| exp.0)
            .unwrap_or(3600) // Default to 1 hour
    }
} 