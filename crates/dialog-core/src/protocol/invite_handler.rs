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
use tracing::{debug, info, warn};

use rvoip_sip_core::{HeaderName, Request, Method, StatusCode};
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::types::header::TypedHeaderTrait;
use rvoip_sip_core::types::unsupported::Unsupported;
use rvoip_sip_core::types::min_se::MinSE;
use crate::api::config::RelUsage;
use crate::manager::transaction_integration::detect_peer_100rel_support;
use crate::transaction::TransactionKey;
use crate::transaction::utils::response_builders;
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
            // This is an initial INVITE — `handle_initial_invite` does the
            // 100rel policy check and may short-circuit with 420 before
            // creating a dialog.
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

        // RFC 3262 §4: enforce our 100rel policy before creating a dialog.
        // A mismatch short-circuits with 420 Bad Extension + `Unsupported:
        // 100rel`. Performed here (not in `handle_invite_method`) because
        // the transaction-event path at `manager/core.rs` bypasses that
        // entry point and calls `handle_initial_invite` directly.
        let (peer_supports_100rel, peer_requires_100rel) = detect_peer_100rel_support(&request);
        let our_policy = self.config_100rel_policy();
        let policy_mismatch = matches!(
            (our_policy, peer_supports_100rel, peer_requires_100rel),
            (RelUsage::NotSupported, _, true)
                | (RelUsage::Required, false, _)
        );
        if policy_mismatch {
            warn!(
                "100rel policy mismatch on initial INVITE (our={:?}, peer_supports={}, peer_requires={}) — rejecting with 420",
                our_policy, peer_supports_100rel, peer_requires_100rel
            );
            let mut response = response_builders::create_response(&request, StatusCode::BadExtension);
            response.headers.push(TypedHeader::Unsupported(
                Unsupported::with_tags(vec!["100rel".to_string()]),
            ));
            let _ = self.transaction_manager.send_response(&transaction_id, response).await;
            return Ok(());
        }

        // RFC 4028 §6: If the peer's `Min-SE:` exceeds our configured
        // `session_timer_secs`, respond 422 Session Interval Too Small with
        // our own Min-SE so the peer can retry with a valid value. Only
        // enforced when session timers are enabled on our side.
        if let Some((our_session_secs, our_min_se)) = self.config_session_timer_settings() {
            let peer_min_se = request.headers.iter().find_map(|h| {
                if let TypedHeader::MinSE(min) = h { Some(min.delta_seconds) } else { None }
            });
            if let Some(peer_min) = peer_min_se {
                if peer_min > our_session_secs {
                    warn!(
                        "Peer's Min-SE {} exceeds our Session-Expires {} — rejecting with 422",
                        peer_min, our_session_secs
                    );
                    let mut response = response_builders::create_response(
                        &request,
                        StatusCode::SessionIntervalTooSmall,
                    );
                    response.headers.push(TypedHeader::MinSE(MinSE::new(our_min_se)));
                    let _ = self.transaction_manager.send_response(&transaction_id, response).await;
                    return Ok(());
                }
            }
        }

        // Create early dialog
        let dialog_id = self.create_early_dialog_from_invite(&request).await?;
        tracing::debug!("🔍 INVITE HANDLER: Created early dialog {}", dialog_id);

        // Capture INVITE CSeq + peer 100rel support on the dialog so the UAS
        // can emit reliable 18x and the PRACK handler can validate `RAck`.
        let invite_cseq = request.header(&HeaderName::CSeq).and_then(|h| {
            if let TypedHeader::CSeq(cseq) = h { Some(cseq.sequence()) } else { None }
        });
        // RFC 4028: parse peer's Session-Expires and compute negotiated
        // interval. UAS is refresher when peer's Session-Expires had
        // `refresher=uas` or when peer didn't name a refresher and our
        // config has session timers enabled.
        let peer_session_expires = request.headers.iter().find_map(|h| {
            if let TypedHeader::SessionExpires(se) = h { Some(se.clone()) } else { None }
        });
        let (negotiated_session_secs, uas_is_refresher) = match (peer_session_expires, self.config_session_timer_settings()) {
            (Some(peer_se), Some((our_secs, _))) => {
                let secs = peer_se.delta_seconds.min(our_secs);
                let uas = matches!(
                    peer_se.refresher,
                    Some(rvoip_sip_core::types::session_expires::Refresher::Uas)
                );
                (Some(secs), uas)
            }
            (None, Some((our_secs, _))) => (Some(our_secs), true),
            _ => (None, false),
        };

        if let Ok(mut dialog) = self.get_dialog_mut(&dialog_id) {
            dialog.peer_supports_100rel = peer_supports_100rel;
            dialog.invite_cseq = invite_cseq;
            dialog.session_expires_secs = negotiated_session_secs;
            dialog.is_session_refresher = uas_is_refresher;
        }

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

        // RFC 4028: dialog is now fully confirmed on the UAS side. Start
        // the refresh task if a session timer was negotiated and we're
        // the designated refresher. No-op when `is_session_refresher`
        // is false (the UAC refreshes for us).
        if let Ok(dlg) = self.get_dialog(&dialog_id) {
            if let Some(secs) = dlg.session_expires_secs {
                let is_refresher = dlg.is_session_refresher;
                drop(dlg);
                crate::manager::session_timer::spawn_refresh_task(
                    self.clone(),
                    dialog_id.clone(),
                    secs,
                    is_refresher,
                );
            }
        }

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