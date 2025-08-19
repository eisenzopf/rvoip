//! Message Routing for Dialog Management
//!
//! This module implements RFC 3261 compliant SIP message routing for dialog management.
//! It handles the complex logic of routing incoming requests and responses to the
//! appropriate existing dialogs or creating new dialogs as needed.
//!
//! ## Key Features
//!
//! - **Dialog Matching**: Uses Call-ID, From tag, and To tag for RFC 3261 dialog identification
//! - **Request Routing**: Routes incoming requests to existing dialogs or creates new ones
//! - **Response Routing**: Routes responses to the correct dialog using transaction context
//! - **Early Dialog Handling**: Manages multiple early dialogs from forking scenarios
//! - **Stateless Request Handling**: Processes stateless requests that don't belong to dialogs
//!
//! ## RFC 3261 Compliance
//!
//! The routing logic follows RFC 3261 Section 12.2 for dialog identification:
//! - For UAC: local tag = From tag, remote tag = To tag
//! - For UAS: local tag = To tag, remote tag = From tag

use crate::dialog::DialogId;
use crate::errors::DialogResult;
use rvoip_sip_core::Request;
use crate::transaction::TransactionKey;
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