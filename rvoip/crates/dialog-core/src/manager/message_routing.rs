//! Message Routing for Dialog Management
//!
//! This module handles routing of SIP messages to appropriate dialogs
//! and implements RFC 3261 compliant message matching rules.

use crate::dialog::DialogId;
use crate::errors::DialogResult;
use rvoip_sip_core::Request;
use rvoip_transaction_core::TransactionKey;
use super::core::DialogManager;

/// Trait for message routing operations
pub trait MessageRouter {
    /// Route an incoming request to the appropriate dialog
    fn route_request(&self, request: &Request) -> impl std::future::Future<Output = Option<DialogId>> + Send;
}

/// Trait for dialog matching operations
pub trait DialogMatcher {
    /// Match a transaction to its associated dialog
    fn match_transaction(&self, transaction_id: &TransactionKey) -> DialogResult<DialogId>;
}

// Implementation will be added in the full version
impl MessageRouter for DialogManager {
    async fn route_request(&self, request: &Request) -> Option<DialogId> {
        // Use the dialog lookup implementation
        self.find_dialog_for_request(request).await
    }
}

impl DialogMatcher for DialogManager {
    fn match_transaction(&self, transaction_id: &TransactionKey) -> DialogResult<DialogId> {
        self.find_dialog_for_transaction(transaction_id)
    }
} 