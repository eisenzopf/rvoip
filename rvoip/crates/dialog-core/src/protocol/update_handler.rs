//! UPDATE Request Handler for Dialog-Core
//!
//! This module handles UPDATE requests according to RFC 3311.
//! UPDATE provides a mechanism for modifying sessions within established
//! SIP dialogs without the complexity of re-INVITE.
//!
//! ## UPDATE vs Re-INVITE
//!
//! - **UPDATE**: Lightweight session modification within dialogs
//! - **Re-INVITE**: Full session renegotiation, can create new dialogs
//! - **Use Cases**: Media parameter changes, codec switches, hold/unhold
//!
//! ## Processing Rules
//!
//! - UPDATE can only be sent within confirmed dialogs
//! - Must increment CSeq number like other in-dialog requests
//! - Supports SDP offer/answer model for media changes
//! - Generates appropriate responses (200 OK, 4xx/5xx errors)

use tracing::debug;

use rvoip_sip_core::{Request, StatusCode};
use rvoip_transaction_core::utils::response_builders;
use crate::dialog::DialogId;
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use crate::manager::{DialogManager, SessionCoordinator, SourceExtractor};

/// UPDATE-specific handling operations
pub trait UpdateHandler {
    /// Handle UPDATE requests (session modification)
    fn handle_update_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of UPDATE handling for DialogManager
impl UpdateHandler for DialogManager {
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
}

/// UPDATE-specific helper methods for DialogManager
impl DialogManager {
    /// Process UPDATE within a dialog
    pub async fn process_update_in_dialog(&self, request: Request, dialog_id: DialogId) -> DialogResult<()> {
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
} 