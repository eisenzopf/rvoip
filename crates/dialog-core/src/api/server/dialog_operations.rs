//! Dialog Operations for DialogServer
//!
//! This module provides dialog-level operations for session coordination including
//! creating dialogs, querying dialog state, and managing dialog lifecycle.

use tracing::debug;

use rvoip_sip_core::{Response, Method, Uri};
use rvoip_transaction_core::TransactionKey;
use crate::dialog::{DialogId, DialogState, Dialog};
use super::super::{ApiResult, ApiError};
use super::super::common::DialogHandle;
use super::core::DialogServer;

/// Dialog operation implementations for DialogServer
impl DialogServer {
    /// Send a SIP request within an existing dialog
    /// 
    /// This method provides direct access to sending arbitrary SIP methods
    /// within established dialogs, which is essential for session-core coordination.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to send the request within
    /// * `method` - SIP method to send (BYE, REFER, NOTIFY, etc.)
    /// * `body` - Optional message body
    /// 
    /// # Returns
    /// Transaction key for tracking the request
    pub async fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>
    ) -> ApiResult<TransactionKey> {
        debug!("Sending {} request in dialog {}", method, dialog_id);
        
        self.dialog_manager.send_request(dialog_id, method, body).await
            .map_err(ApiError::from)
    }
    
    /// Create an outgoing dialog for client-initiated communications
    /// 
    /// This method allows session-core to create dialogs for outgoing calls
    /// and other client-initiated SIP operations.
    /// 
    /// # Arguments
    /// * `local_uri` - Local URI (From header)
    /// * `remote_uri` - Remote URI (To header)
    /// * `call_id` - Optional Call-ID (will be generated if None)
    /// 
    /// # Returns
    /// The created dialog ID
    pub async fn create_outgoing_dialog(
        &self,
        local_uri: Uri,
        remote_uri: Uri,
        call_id: Option<String>
    ) -> ApiResult<DialogId> {
        debug!("Creating outgoing dialog from {} to {}", local_uri, remote_uri);
        
        self.dialog_manager.create_outgoing_dialog(local_uri, remote_uri, call_id).await
            .map_err(ApiError::from)
    }
    
    /// Get detailed information about a dialog
    /// 
    /// Provides access to the complete dialog state for session coordination
    /// and monitoring purposes.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to query
    /// 
    /// # Returns
    /// Complete dialog information
    pub async fn get_dialog_info(&self, dialog_id: &DialogId) -> ApiResult<Dialog> {
        self.dialog_manager.get_dialog(dialog_id)
            .map_err(ApiError::from)
    }
    
    /// Get the current state of a dialog
    /// 
    /// Provides quick access to dialog state without retrieving the full
    /// dialog information.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to query
    /// 
    /// # Returns
    /// Current dialog state
    pub async fn get_dialog_state(&self, dialog_id: &DialogId) -> ApiResult<DialogState> {
        self.dialog_manager.get_dialog_state(dialog_id)
            .map_err(ApiError::from)
    }
    
    /// Terminate a dialog and clean up resources
    /// 
    /// This method provides direct control over dialog termination,
    /// which is essential for session lifecycle management.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog ID to terminate
    /// 
    /// # Returns
    /// Success or error
    pub async fn terminate_dialog(&self, dialog_id: &DialogId) -> ApiResult<()> {
        debug!("Terminating dialog {}", dialog_id);
        
        self.dialog_manager.terminate_dialog(dialog_id).await
            .map_err(ApiError::from)
    }
    
    /// List all active dialog IDs
    /// 
    /// Provides access to all active dialogs for monitoring and
    /// management purposes.
    /// 
    /// # Returns
    /// Vector of active dialog IDs
    pub async fn list_active_dialogs(&self) -> Vec<DialogId> {
        self.dialog_manager.list_dialogs()
    }
    
    /// Get active calls as DialogHandles
    /// 
    /// Provides access to all active calls for management purposes.
    /// 
    /// # Returns
    /// Vector of active call handles
    pub async fn active_calls(&self) -> Vec<DialogHandle> {
        let dialog_ids = self.dialog_manager.list_dialogs();
        
        dialog_ids.into_iter()
            .filter_map(|dialog_id| {
                // Only include dialogs that represent active calls
                if let Ok(dialog) = self.dialog_manager.get_dialog(&dialog_id) {
                    if dialog.state == DialogState::Confirmed || dialog.state == DialogState::Early {
                        Some(DialogHandle::new(dialog_id, self.dialog_manager.clone()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }
    
    /// Send a SIP response for a transaction
    /// 
    /// Provides direct control over response generation, which is essential
    /// for custom response handling in session coordination.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `response` - Complete SIP response
    /// 
    /// # Returns
    /// Success or error
    pub async fn send_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response
    ) -> ApiResult<()> {
        debug!("Sending response for transaction {}", transaction_id);
        
        self.dialog_manager.send_response(transaction_id, response).await
            .map_err(ApiError::from)
    }
} 