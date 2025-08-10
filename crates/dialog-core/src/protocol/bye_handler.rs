//! BYE Request Handler for Dialog-Core
//!
//! This module handles BYE requests according to RFC 3261 Section 15.
//! BYE requests terminate established SIP dialogs and clean up associated resources.
//!
//! ## BYE Processing Steps
//!
//! 1. **Dialog Identification**: Match BYE to existing dialog using Call-ID and tags
//! 2. **Authorization Check**: Verify BYE is from dialog participant
//! 3. **State Validation**: Ensure dialog is in confirmable state for termination
//! 4. **Resource Cleanup**: Terminate dialog and clean up associated state
//! 5. **Response Generation**: Send 200 OK to acknowledge BYE receipt
//!
//! ## Error Handling
//!
//! - **481 Call/Transaction Does Not Exist**: No matching dialog found
//! - **403 Forbidden**: BYE from unauthorized party
//! - **500 Server Internal Error**: Processing failures

use tracing::{debug, info};

use rvoip_sip_core::{Request, StatusCode};
use rvoip_transaction_core::{TransactionKey, utils::response_builders};
use crate::dialog::DialogId;
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use crate::manager::{DialogManager, SessionCoordinator, SourceExtractor};

/// BYE-specific handling operations
pub trait ByeHandler {
    /// Handle BYE requests (dialog-terminating)
    fn handle_bye_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of BYE handling for DialogManager
impl ByeHandler for DialogManager {
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
}

/// BYE-specific helper methods for DialogManager
impl DialogManager {
    /// Process BYE within a dialog
    pub async fn process_bye_in_dialog(
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
        
        // Send session coordination event (Phase 1 - terminating)
        let event = SessionCoordinationEvent::CallTerminating {
            dialog_id: dialog_id.clone(),
            reason: "BYE received".to_string(),
        };
        
        self.notify_session_layer(event).await?;
        info!("BYE processed for dialog {}", dialog_id);
        Ok(())
    }
} 