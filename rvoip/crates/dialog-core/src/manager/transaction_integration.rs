//! Transaction Integration for Dialog Management
//!
//! This module handles integration with transaction-core, maintaining
//! proper separation of concerns between dialog and transaction layers.

use std::net::SocketAddr;
use tracing::{debug, warn};
use rvoip_sip_core::{Request, Response, Method};
use rvoip_transaction_core::TransactionKey;
use crate::errors::DialogResult;
use crate::dialog::DialogId;
use super::core::DialogManager;
use super::dialog_operations::DialogStore;

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
    /// Implements proper request creation within dialogs and transaction handling.
    async fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> DialogResult<TransactionKey> {
        debug!("Sending {} request for dialog {}", method, dialog_id);
        
        // Get destination from dialog first
        let destination = {
            let dialog = self.get_dialog(dialog_id)?;
            dialog.get_remote_target_address().await
                .ok_or_else(|| crate::errors::DialogError::routing_error("No remote target address available"))?
        };
        
        // Create the request using **transaction-core helper** instead of dialog.create_request()
        let request = {
            let mut dialog = self.get_dialog_mut(dialog_id)?;
            
            // First create a basic template request using the current dialog method
            // This gives us the dialog-specific headers (Call-ID, From, To, CSeq)
            let template_request = dialog.create_request(method.clone());
            
            // Convert body to String if provided
            let body_string = body.map(|b| String::from_utf8_lossy(&b).to_string());
            
            // Determine content type for common methods
            let content_type = if method == Method::Invite && body_string.is_some() {
                Some("application/sdp")
            } else if body_string.is_some() {
                Some("text/plain") // Default for other methods with body
            } else {
                None
            };
            
            // Use transaction-core helper to create a proper RFC 3261 compliant request
            rvoip_transaction_core::utils::create_in_dialog_request(
                &template_request,
                method.clone(),
                &self.local_address, // Use configured local address from DialogManager
                body_string,
                content_type,
            ).map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to create {} request using transaction-core helper: {}", method, e),
            })?
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
        
        // Associate transaction with dialog
        self.transaction_to_dialog.insert(transaction_id.clone(), dialog_id.clone());
        
        // Send the request using transaction-core
        self.transaction_manager
            .send_request(&transaction_id)
            .await
            .map_err(|e| crate::errors::DialogError::TransactionError {
                message: format!("Failed to send request: {}", e),
            })?;
        
        debug!("Successfully sent {} request for dialog {} (transaction: {})", method, dialog_id, transaction_id);
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