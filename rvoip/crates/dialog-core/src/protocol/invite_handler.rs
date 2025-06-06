//! INVITE Request Handler for Dialog-Core
//!
//! This module provides specialized handling for INVITE requests according to RFC 3261.
//! INVITE is the most complex SIP method as it can create dialogs, modify sessions,
//! and requires careful state management.
//!
//! ## INVITE Types Handled
//!
//! - **Initial INVITE**: Creates new dialogs and establishes sessions
//! - **Re-INVITE**: Modifies existing sessions within established dialogs  
//! - **Refresh INVITE**: Refreshes session timers and state
//!
//! ## Dialog Creation Process
//!
//! 1. Parse INVITE request and validate headers
//! 2. Create early dialog upon receiving INVITE
//! 3. Send provisional responses (100 Trying, 180 Ringing)
//! 4. Confirm dialog with 2xx response
//! 5. Complete with ACK reception

use std::net::SocketAddr;
use tracing::{debug, info};

use rvoip_sip_core::Request;
use rvoip_transaction_core::TransactionKey;
use crate::dialog::{DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use crate::manager::{DialogManager, DialogLookup, SessionCoordinator};

/// INVITE-specific handling operations
pub trait InviteHandler {
    /// Handle INVITE requests (dialog-creating and re-INVITE)
    fn handle_invite_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of INVITE handling for DialogManager
impl InviteHandler for DialogManager {
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
}

/// INVITE-specific helper methods for DialogManager
impl DialogManager {
    /// Handle initial INVITE (dialog-creating)
    pub async fn handle_initial_invite(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<()> {
        println!("🔍 INVITE HANDLER: Processing initial INVITE request from {}", source);
        debug!("Processing initial INVITE request");
        
        // Create early dialog
        let dialog_id = self.create_early_dialog_from_invite(&request).await?;
        println!("🔍 INVITE HANDLER: Created early dialog {}", dialog_id);
        
        // Associate transaction with dialog
        self.associate_transaction_with_dialog(&transaction_id, &dialog_id);
        println!("🔍 INVITE HANDLER: Associated transaction {} with dialog {}", transaction_id, dialog_id);
        
        // Send session coordination event
        let event = SessionCoordinationEvent::IncomingCall {
            dialog_id: dialog_id.clone(),
            transaction_id: transaction_id.clone(),
            request: request.clone(),
            source,
        };
        
        println!("🔍 INVITE HANDLER: About to send SessionCoordinationEvent::IncomingCall for dialog {}", dialog_id);
        self.notify_session_layer(event).await?;
        println!("🔍 INVITE HANDLER: Successfully sent SessionCoordinationEvent::IncomingCall for dialog {}", dialog_id);
        info!("Initial INVITE processed, created dialog {}", dialog_id);
        Ok(())
    }
    
    /// Handle re-INVITE (session modification)
    pub async fn handle_reinvite(
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
    
    /// Process ACK within a dialog (related to INVITE processing)
    pub async fn process_ack_in_dialog(&self, request: Request, dialog_id: DialogId) -> DialogResult<()> {
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
            session_answer: self.extract_body_string(&request),
        };
        
        self.notify_session_layer(event).await?;
        debug!("ACK processed for dialog {}", dialog_id);
        Ok(())
    }
    
    /// Extract body as string helper for INVITE/ACK processing
    fn extract_body_string(&self, request: &Request) -> String {
        if request.body().is_empty() {
            String::new()
        } else {
            String::from_utf8_lossy(request.body()).to_string()
        }
    }
} 