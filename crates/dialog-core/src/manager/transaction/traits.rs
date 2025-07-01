//! Transaction Integration Traits
//!
//! This module defines the core traits for transaction integration between
//! dialog-core and transaction-core. These traits provide clean interfaces
//! for request sending, response handling, and transaction management.

use rvoip_sip_core::{Request, Response, Method};
use rvoip_transaction_core::TransactionKey;
use crate::errors::DialogResult;
use crate::dialog::DialogId;

/// Trait for transaction integration operations
/// 
/// Provides the core interface for sending requests and responses through
/// the transaction layer while maintaining dialog context.
pub trait TransactionIntegration {
    /// Send a request within a dialog using transaction-core
    /// 
    /// This method creates and sends SIP requests within established dialogs,
    /// handling proper transaction creation and request routing.
    /// 
    /// # Arguments
    /// * `dialog_id` - The dialog to send the request within
    /// * `method` - SIP method to send
    /// * `body` - Optional message body
    /// 
    /// # Returns
    /// Transaction key for tracking the request
    fn send_request_in_dialog(
        &self,
        dialog_id: &DialogId,
        method: Method,
        body: Option<bytes::Bytes>,
    ) -> impl std::future::Future<Output = DialogResult<TransactionKey>> + Send;
    
    /// Send a response using transaction-core
    /// 
    /// Delegates response sending to transaction-core while maintaining
    /// proper dialog state and routing information.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to respond to
    /// * `response` - Complete SIP response
    /// 
    /// # Returns
    /// Success or error
    fn send_transaction_response(
        &self,
        transaction_id: &TransactionKey,
        response: Response,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Trait for transaction helper operations
/// 
/// Provides additional transaction-related utilities for dialog management
/// including transaction-dialog associations and ACK creation.
pub trait TransactionHelpers {
    /// Associate a transaction with a dialog
    /// 
    /// Creates the mapping between transactions and dialogs for proper
    /// message routing and event correlation.
    /// 
    /// # Arguments
    /// * `transaction_id` - Transaction to associate
    /// * `dialog_id` - Dialog to associate with
    fn link_transaction_to_dialog(&self, transaction_id: &TransactionKey, dialog_id: &DialogId);
    
    /// Create ACK for 2xx response using transaction-core helpers
    /// 
    /// Uses transaction-core's ACK creation helpers while maintaining
    /// dialog-core concerns for proper 2xx ACK handling.
    /// 
    /// # Arguments
    /// * `original_invite_tx_id` - Original INVITE transaction
    /// * `response` - 2xx response to ACK
    /// 
    /// # Returns
    /// ACK request ready for sending
    fn create_ack_for_success_response(
        &self,
        original_invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> impl std::future::Future<Output = DialogResult<Request>> + Send;
} 