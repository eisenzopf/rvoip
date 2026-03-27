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

use rvoip_sip_core::{Request, Method};
use crate::transaction::TransactionKey;
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

        // INVITE uses 407 Proxy-Authenticate (not 401) per RFC 3261 §22.2
        if let Some(auth) = self.auth_provider() {
            match auth.check_request(&request, source).await {
                crate::auth::AuthResult::Authenticated { username } => {
                    debug!("INVITE authenticated for user {}", username);
                }
                crate::auth::AuthResult::Challenge => {
                    debug!("Sending 407 challenge for INVITE from {}", source);
                    let nonce = crate::auth::generate_nonce();
                    let response = crate::transaction::utils::response_builders::create_proxy_auth_response(
                        &request, auth.realm(), &nonce,
                    );
                    self.transaction_manager.send_response(&transaction_id, response).await
                        .map_err(|e| DialogError::TransactionError {
                            message: format!("Failed to send 407 for INVITE: {}", e),
                        })?;
                    return Ok(());
                }
                crate::auth::AuthResult::Skip => {}
            }
        }

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
        tracing::debug!("🔍 INVITE HANDLER: Processing initial INVITE request from {}", source);
        debug!("Processing initial INVITE request");

        // ── Proxy routing gate ────────────────────────────────────────────────
        // If a ProxyRouter is configured, ask it whether to forward or reject
        // the request before falling through to B2BUA / session-core handling.
        if let Some(router) = self.proxy_router() {
            match router.route_request(&request, source).await {
                crate::auth::ProxyAction::Forward { destination } => {
                    info!("Proxy-forwarding INVITE from {} to {}", source, destination);

                    // RFC 3261 §16.4: reject if Max-Forwards == 0.
                    let mf = request
                        .typed_header::<rvoip_sip_core::types::max_forwards::MaxForwards>()
                        .map(|m| m.0)
                        .unwrap_or(70);
                    if mf == 0 {
                        let response = crate::transaction::utils::response_builders::create_response(
                            &request,
                            rvoip_sip_core::StatusCode::TooManyHops,
                        );
                        self.transaction_manager
                            .send_response(&transaction_id, response)
                            .await
                            .map_err(|e| DialogError::TransactionError {
                                message: format!("Failed to send 483 TooManyHops: {}", e),
                            })?;
                        return Ok(());
                    }

                    let local = self.local_address;
                    let forwarded = crate::transaction::utils::request_builders::create_forwarded_request(
                        &request,
                        &local.ip().to_string(),
                        local.port(),
                        "UDP",
                        &request.uri().to_string(),
                    )
                    .map_err(|e| DialogError::TransactionError {
                        message: format!("Failed to build forwarded INVITE: {}", e),
                    })?;

                    self.transaction_manager
                        .forward_request(forwarded, destination)
                        .await
                        .map_err(|e| DialogError::TransactionError {
                            message: format!("Failed to forward INVITE to {}: {}", destination, e),
                        })?;

                    return Ok(());
                }
                crate::auth::ProxyAction::Reject { status, reason } => {
                    info!("Proxy rejecting INVITE from {} with {} {}", source, status, reason);
                    let status_code = rvoip_sip_core::StatusCode::from_u16(status)
                        .unwrap_or(rvoip_sip_core::StatusCode::ServerInternalError);
                    let response = crate::transaction::utils::response_builders::create_response(
                        &request, status_code,
                    );
                    self.transaction_manager
                        .send_response(&transaction_id, response)
                        .await
                        .map_err(|e| DialogError::TransactionError {
                            message: format!("Failed to send proxy rejection: {}", e),
                        })?;
                    return Ok(());
                }
                crate::auth::ProxyAction::LocalB2BUA => {
                    // Fall through to B2BUA / session-core handling below.
                    debug!("ProxyRouter: LocalB2BUA — proceeding with normal INVITE handling");
                }
            }
        }
        // ─────────────────────────────────────────────────────────────────────

        // Create early dialog
        let dialog_id = self.create_early_dialog_from_invite(&request).await?;
        tracing::debug!("🔍 INVITE HANDLER: Created early dialog {}", dialog_id);
        
        // Associate transaction with dialog
        self.associate_transaction_with_dialog(&transaction_id, &dialog_id);
        tracing::debug!("🔍 INVITE HANDLER: Associated transaction {} with dialog {}", transaction_id, dialog_id);
        
        // Send session coordination event
        let event = SessionCoordinationEvent::IncomingCall {
            dialog_id: dialog_id.clone(),
            transaction_id: transaction_id.clone(),
            request: request.clone(),
            source,
        };
        
        tracing::debug!("🔍 INVITE HANDLER: About to send SessionCoordinationEvent::IncomingCall for dialog {}", dialog_id);
        self.notify_session_layer(event).await?;
        tracing::debug!("🔍 INVITE HANDLER: Successfully sent SessionCoordinationEvent::IncomingCall for dialog {}", dialog_id);
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
        info!("✅ RFC 3261: ACK received for dialog {} - time to start media (UAS side)", dialog_id);
        
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
        
        // Extract any SDP from the ACK (though typically ACK doesn't have SDP for 2xx responses)
        let negotiated_sdp = if !request.body().is_empty() {
            let sdp = self.extract_body_string(&request);
            debug!("ACK contains SDP body: {}", sdp);
            Some(sdp)
        } else {
            debug!("ACK has no SDP body (normal for 2xx ACK)");
            None
        };
        
        // RFC 3261 COMPLIANT: Send AckReceived event for UAS side media creation  
        let event = SessionCoordinationEvent::AckReceived {
            dialog_id: dialog_id.clone(),
            transaction_id: TransactionKey::new(format!("ack-{}", dialog_id), Method::Ack, false), // Dummy transaction ID for ACK
            negotiated_sdp,
        };
        
        self.notify_session_layer(event).await?;
        debug!("🚀 RFC 3261: Emitted AckReceived event for UAS side media creation");
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