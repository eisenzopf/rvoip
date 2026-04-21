//! Simplified Dialog Adapter for session-core
//!
//! Thin translation layer between dialog-core and state machine.
//! Focuses only on essential dialog operations and events.
//!
//! ## API Design
//! 
//! This adapter provides a clean interface for dialog operations:
//! 
//! ### Primary Methods
//! - `send_invite_with_details()` - Creates dialog and sends INVITE in one atomic operation
//! - `send_response()` - Sends SIP responses for incoming calls
//! - `send_bye()` - Terminates calls
//! - `send_ack()` - Acknowledges responses
//! 
//! ### Removed Methods
//! The following methods were removed to avoid confusion:
//! - `create_dialog()` - Did not actually create a dialog in dialog-core
//! - `send_invite()` - Did not actually send an INVITE
//! 
//! All dialog creation is now done through `send_invite_with_details()` which
//! properly creates the dialog in dialog-core and sends the INVITE.

use std::sync::Arc;
use dashmap::DashMap;
use rvoip_dialog_core::{
    api::unified::UnifiedDialogApi,
    DialogId as RvoipDialogId,
    transaction::TransactionKey,
};
use rvoip_sip_core::{Response, StatusCode};
use rvoip_infra_common::events::{
    coordinator::GlobalEventCoordinator,
    cross_crate::{RvoipCrossCrateEvent, SessionToDialogEvent},
};
use crate::state_table::types::{SessionId, DialogId};
use crate::errors::{Result, SessionError};
use crate::session_store::SessionStore;
use crate::api::types::DialogIdentity;

/// Minimal dialog adapter - just translates between dialog-core and state machine
pub struct DialogAdapter {
    /// Dialog-core unified API
    pub(crate) dialog_api: Arc<UnifiedDialogApi>,
    
    /// Session store for updating IDs
    pub(crate) store: Arc<SessionStore>,
    
    /// Simple mapping of session IDs to dialog IDs
    pub(crate) session_to_dialog: Arc<DashMap<SessionId, RvoipDialogId>>,
    pub(crate) dialog_to_session: Arc<DashMap<RvoipDialogId, SessionId>>,
    
    /// Store Call-ID to session mapping for correlation
    pub(crate) callid_to_session: Arc<DashMap<String, SessionId>>,
    
    /// Store outgoing INVITE transaction IDs for UAC ACK sending
    pub(crate) outgoing_invite_tx: Arc<DashMap<SessionId, TransactionKey>>,
    
    /// Global event coordinator for publishing events
    pub(crate) global_coordinator: Arc<GlobalEventCoordinator>,

    /// State machine reference for triggering events (needed for REGISTER
    /// response handling). Wired post-construction via
    /// [`DialogAdapter::init_state_machine`] because the `StateMachine`
    /// transitively depends on this adapter — classic circular init. The
    /// `OnceLock` makes the initialization soundly observable by any task
    /// without requiring `&mut self`.
    pub(crate) state_machine: Arc<std::sync::OnceLock<Arc<crate::state_machine::StateMachine>>>,
}

impl DialogAdapter {
    /// Create a new dialog adapter
    pub fn new(
        dialog_api: Arc<UnifiedDialogApi>,
        store: Arc<SessionStore>,
        global_coordinator: Arc<GlobalEventCoordinator>,
    ) -> Self {
        Self {
            dialog_api,
            store,
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            callid_to_session: Arc::new(DashMap::new()),
            outgoing_invite_tx: Arc::new(DashMap::new()),
            global_coordinator,
            state_machine: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Wire the state machine after construction. Idempotent — subsequent
    /// calls are silently ignored (returns `Err` if already set, which
    /// callers may choose to ignore or treat as a programming error).
    pub fn init_state_machine(
        &self,
        state_machine: Arc<crate::state_machine::StateMachine>,
    ) -> std::result::Result<(), Arc<crate::state_machine::StateMachine>> {
        self.state_machine.set(state_machine)
    }
    
    // ===== Direct Dialog Operations =====
    // NOTE: Removed confusing create_dialog() and send_invite() methods
    // Use send_invite_with_details() to create a dialog and send INVITE in one operation
    
    /// Send a response
    pub async fn send_response_by_dialog(&self, _dialog_id: DialogId, status_code: u16, _reason: &str) -> Result<()> {
        // We can't really convert a string to RvoipDialogId which wraps a UUID
        // This method needs to be rethought - for now just return Ok
        // since this is called from places where we have only our DialogId
        tracing::warn!("send_response_by_dialog called but conversion not implemented - status: {}", status_code);
        Ok(())
    }
    
    /// Send BYE for a specific dialog
    pub async fn send_bye(&self, dialog_id: crate::types::DialogId) -> Result<()> {
        // Convert our DialogId to RvoipDialogId
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();
        
        // Find session ID from dialog
        if let Some(entry) = self.dialog_to_session.get(&rvoip_dialog_id) {
            let session_id = entry.value().clone();
            drop(entry);
            
            // Send BYE through dialog API
            self.dialog_api
                .send_bye(&rvoip_dialog_id)
                .await
                .map_err(|e| SessionError::DialogError(format!("Failed to send BYE: {}", e)))?;
            
            tracing::info!("Sent BYE for session {}", session_id.0);
        } else {
            tracing::warn!("No session found for dialog {}", dialog_id);
        }
        
        Ok(())
    }
    
    /// Send re-INVITE with new SDP
    pub async fn send_reinvite(&self, dialog_id: crate::types::DialogId, sdp: String) -> Result<()> {
        // Convert our DialogId to RvoipDialogId
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();
        
        // Find session ID from dialog
        if let Some(entry) = self.dialog_to_session.get(&rvoip_dialog_id) {
            let session_id = entry.value().clone();
            drop(entry);
            
            // Use UPDATE method for re-INVITE
            self.dialog_api
                .send_update(&rvoip_dialog_id, Some(sdp))
                .await
                .map_err(|e| SessionError::DialogError(format!("Failed to send re-INVITE: {}", e)))?;
                
            tracing::info!("Sent re-INVITE for session {}", session_id.0);
        } else {
            tracing::warn!("No session found for dialog {}", dialog_id);
        }
        
        Ok(())
    }
    
    /// Send REFER for transfers
    pub async fn send_refer(&self, dialog_id: crate::types::DialogId, target: &str, attended: bool) -> Result<()> {
        // Convert our DialogId to RvoipDialogId
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();
        
        // Find session ID from dialog
        if let Some(entry) = self.dialog_to_session.get(&rvoip_dialog_id) {
            let session_id = entry.value().clone();
            drop(entry);
            
            // Send REFER through dialog API
            let transfer_info = if attended {
                Some("attended".to_string()) // Or use proper transfer info structure
            } else {
                None
            };
            
            self.dialog_api
                .send_refer(&rvoip_dialog_id, target.to_string(), transfer_info)
                .await
                .map_err(|e| SessionError::DialogError(format!("Failed to send REFER: {}", e)))?;
            
            tracing::info!("Sent REFER to {} for session {}", target, session_id.0);
        } else {
            tracing::warn!("No session found for dialog {}", dialog_id);
        }
        
        Ok(())
    }
    
    /// Get remote URI for a dialog
    pub async fn get_remote_uri(&self, _dialog_id: crate::types::DialogId) -> Result<String> {
        // For now, return a placeholder
        Ok("sip:remote@example.com".to_string())
    }

    /// RFC 3261 §22.2 — resend an INVITE with digest `Authorization` (or
    /// `Proxy-Authorization`) header on the same dialog after the server
    /// challenged with 401/407. Session-core-v3's `SendINVITEWithAuth` action
    /// owns the digest computation; this is a thin passthrough to dialog-core.
    ///
    /// Both REGISTER and INVITE 401/407 challenges flow through the state
    /// machine via `DialogToSessionEvent::AuthRequired` → `EventType::AuthRequired`;
    /// the previous inline REGISTER-auth shortcut (`handle_401_challenge`) was
    /// retired when INVITE auth landed. See `default.yaml`'s `Initiating` /
    /// `Registering` + `AuthRequired` transitions.
    pub async fn resend_invite_with_auth(
        &self,
        session_id: &SessionId,
        sdp: Option<String>,
        auth_header_name: &str,
        auth_header_value: String,
    ) -> Result<()> {
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .map(|e| e.value().clone())
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;

        self.dialog_api
            .send_invite_with_auth(&dialog_id, sdp, auth_header_name, auth_header_value)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!(
                    "resend_invite_with_auth failed for session {}: {}",
                    session_id.0, e
                ))
            })?;
        Ok(())
    }

    /// RFC 4028 §6 — resend an INVITE with a bumped `Session-Expires` /
    /// `Min-SE` after a 422 Session Interval Too Small. The UAS's Min-SE
    /// floor is supplied by the caller (parsed from the 422 response by
    /// dialog-core). The timer headers bypass [`DialogManagerConfig`]'s
    /// global values and use these overrides verbatim.
    pub async fn resend_invite_with_session_timer_override(
        &self,
        session_id: &SessionId,
        sdp: Option<String>,
        session_secs: u32,
        min_se: u32,
    ) -> Result<()> {
        // Fast-RTT race: when the UAS answers 422 on a loopback socket the
        // response can be processed before the initial `make_call_for_session`
        // call has returned and inserted the s2d mapping (see
        // `send_invite_with_details` below — the insert happens after the
        // await). Poll briefly for the mapping to appear. Cap at 1s; a
        // timeout here propagates as `SessionNotFound` which the retry
        // action's error path converts into a terminal `CallFailed`.
        use tokio::time::{Duration, Instant};
        let start = Instant::now();
        let dialog_id = loop {
            if let Some(entry) = self.session_to_dialog.get(session_id) {
                break entry.value().clone();
            }
            if start.elapsed() >= Duration::from_secs(1) {
                return Err(SessionError::SessionNotFound(session_id.0.clone()));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };

        self.dialog_api
            .send_invite_with_session_timer_override(&dialog_id, sdp, session_secs, min_se)
            .await
            .map_err(|e| {
                SessionError::DialogError(format!(
                    "resend_invite_with_session_timer_override failed for session {}: {}",
                    session_id.0, e
                ))
            })?;
        Ok(())
    }

    /// Does the remote peer support RFC 3262 100rel? Used to gate
    /// `send_early_media` — we only emit a reliable 183 when the caller
    /// advertised `Supported: 100rel` (or `Require: 100rel`) on the INVITE.
    /// Returns `SessionNotFound` if the session has no dialog yet.
    pub async fn peer_supports_100rel(&self, session_id: &SessionId) -> Result<bool> {
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .map(|e| e.value().clone())
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;

        let dialog = self
            .dialog_api
            .get_dialog_info(&dialog_id)
            .await
            .map_err(|e| SessionError::DialogError(format!(
                "peer_supports_100rel: failed to read dialog {}: {}",
                dialog_id, e
            )))?;

        Ok(dialog.peer_supports_100rel)
    }
    
    // ===== Outbound Actions (from state machine) =====
    
    /// Send INVITE for UAC - this is the primary method for initiating calls
    /// 
    /// This method:
    /// 1. Creates a dialog in dialog-core
    /// 2. Sends the INVITE request
    /// 3. Stores the session-to-dialog mapping
    /// 
    /// # Arguments
    /// * `session_id` - The session ID from the state machine
    /// * `from` - The From URI (e.g., "sip:alice@example.com")
    /// * `to` - The To URI (e.g., "sip:bob@example.com")
    /// * `sdp` - Optional SDP offer
    pub async fn send_invite_with_details(
        &self,
        session_id: &SessionId,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<()> {
        // Use make_call_with_id to control the Call-ID
        let call_id = format!("{}@session-core", session_id.0);

        // Store Call-ID mapping BEFORE making the call to avoid race condition
        // This ensures any events that come back immediately can find the session
        self.callid_to_session.insert(call_id.clone(), session_id.clone());

        // Use `make_call_for_session` so the session↔dialog mapping is
        // installed on dialog-core *before* the INVITE goes on the wire.
        // This closes the fast-RTT race where a 4xx response (e.g. 420 Bad
        // Extension on localhost) can be processed by the event loop before
        // the async `StoreDialogMapping` below has populated the lookup
        // tables — which would otherwise cause the CallFailed event to be
        // silently dropped by `event_hub::convert_coordination_to_cross_crate`.
        let call_handle = self.dialog_api
            .make_call_for_session(&session_id.0, from, to, sdp, Some(call_id.clone()))
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to make call: {}", e)))?;

        let dialog_id = call_handle.call_id().clone();

        // Store remaining mappings on session-core side
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());

        // Publish StoreDialogMapping event to inform dialog-core about the session-dialog mapping
        let event = SessionToDialogEvent::StoreDialogMapping {
            session_id: session_id.0.clone(),
            dialog_id: dialog_id.to_string(),
        };
        self.global_coordinator.publish(Arc::new(
            RvoipCrossCrateEvent::SessionToDialog(event)
        )).await
        .map_err(|e| SessionError::InternalError(format!("Failed to publish StoreDialogMapping: {}", e)))?;
        
        tracing::info!("Published StoreDialogMapping for session {} -> dialog {}", session_id.0, dialog_id);
        
        // Store the transaction ID for later ACK sending
        // Note: CallHandle might not expose transaction_id directly
        // For now, we'll rely on dialog-core to handle ACKs internally
        tracing::debug!("Dialog {} created for session {} - ACK will be handled by dialog-core", dialog_id, session_id.0);
        
        // Don't update session store here - the state machine will handle updating the dialog ID
        tracing::debug!("Dialog {} created for session {}", dialog_id, session_id.0);

        Ok(())
    }

    /// Like [`send_invite_with_details`] but appends caller-supplied extra
    /// headers to the outgoing INVITE. Routes through dialog-core's
    /// `make_call_with_extra_headers_for_session` so the extras (typically
    /// `P-Asserted-Identity` per RFC 3325) ride on the very first wire
    /// transmission rather than being added in a follow-up.
    ///
    /// Used by the `SendINVITE` action when `SessionState.pai_uri` is set;
    /// the action handler builds the typed PAI header from the URI and
    /// passes it through here.
    pub async fn send_invite_with_extra_headers(
        &self,
        session_id: &SessionId,
        from: &str,
        to: &str,
        sdp: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Result<()> {
        let call_id = format!("{}@session-core", session_id.0);

        self.callid_to_session.insert(call_id.clone(), session_id.clone());

        let call_handle = self
            .dialog_api
            .make_call_with_extra_headers_for_session(
                &session_id.0,
                from,
                to,
                sdp,
                Some(call_id.clone()),
                extra_headers,
            )
            .await
            .map_err(|e| SessionError::DialogError(format!(
                "Failed to make call with extra headers: {}", e
            )))?;

        let dialog_id = call_handle.call_id().clone();

        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());

        let event = SessionToDialogEvent::StoreDialogMapping {
            session_id: session_id.0.clone(),
            dialog_id: dialog_id.to_string(),
        };
        self.global_coordinator
            .publish(Arc::new(RvoipCrossCrateEvent::SessionToDialog(event)))
            .await
            .map_err(|e| SessionError::InternalError(format!(
                "Failed to publish StoreDialogMapping: {}", e
            )))?;

        tracing::info!(
            "send_invite_with_extra_headers: published StoreDialogMapping for session {} -> dialog {} ({} extra header(s))",
            session_id.0,
            dialog_id,
            self.session_to_dialog
                .get(session_id)
                .map(|_| "ok")
                .unwrap_or("missing"),
        );

        Ok(())
    }

    /// Send 200 OK response
    pub async fn send_200_ok(&self, session_id: &SessionId, sdp: Option<String>) -> Result<()> {
        self.send_response(session_id, 200, sdp).await
    }
    
    /// Send response with SDP
    pub async fn send_response_with_sdp(&self, session_id: &SessionId, code: u16, _reason: &str, sdp: &str) -> Result<()> {
        self.send_response(session_id, code, Some(sdp.to_string())).await
    }
    
    /// Send response without SDP
    pub async fn send_response_session(&self, session_id: &SessionId, code: u16, _reason: &str) -> Result<()> {
        self.send_response(session_id, code, None).await
    }
    
    /// Send error response
    pub async fn send_error_response(&self, session_id: &SessionId, code: StatusCode, _reason: &str) -> Result<()> {
        self.send_response(session_id, code.as_u16(), None).await
    }
    
    /// Send a 3xx redirect response with one or more `Contact:` URIs
    /// (RFC 3261 §8.1.3.4). Thin wrapper over
    /// `UnifiedDialogApi::send_redirect_response_for_session`.
    pub async fn send_redirect_response(
        &self,
        session_id: &SessionId,
        status: u16,
        contacts: Vec<String>,
    ) -> Result<()> {
        tracing::info!(
            "DialogAdapter sending {} redirect for session {} with {} contact(s)",
            status,
            session_id.0,
            contacts.len()
        );
        self.dialog_api
            .send_redirect_response_for_session(&session_id.0, status, contacts)
            .await
            .map_err(|e| {
                tracing::error!("Failed to send redirect for session {}: {}", session_id.0, e);
                SessionError::DialogError(format!("Failed to send redirect: {}", e))
            })
    }

    /// Send response (for UAS)
    pub async fn send_response(
        &self,
        session_id: &SessionId,
        code: u16,
        sdp: Option<String>,
    ) -> Result<()> {
        tracing::info!("DialogAdapter sending {} response for session {} with SDP: {}", 
            code, session_id.0, sdp.is_some());
        
        // Use dialog-core's session-based response method
        self.dialog_api
            .send_response_for_session(&session_id.0, code, sdp)
            .await
            .map_err(|e| {
                tracing::error!("Failed to send response for session {}: {}", session_id.0, e);
                SessionError::DialogError(format!("Failed to send response: {}", e))
            })
    }
    
    /// Send ACK (for UAC after 200 OK)
    pub async fn send_ack(&self, session_id: &SessionId, response: &Response) -> Result<()> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        // Check if we have the original INVITE transaction ID stored
        if let Some(tx_id) = self.outgoing_invite_tx.get(session_id) {
            // Use the proper ACK method with transaction ID
            self.dialog_api
                .send_ack_for_2xx_response(&dialog_id, &tx_id, response)
                .await
                .map_err(|e| SessionError::DialogError(format!("Failed to send ACK: {}", e)))?;
            
            // Clean up the stored transaction ID after successful ACK
            self.outgoing_invite_tx.remove(session_id);
        } else {
            // Fallback: Try to send ACK without transaction ID (may not work properly)
            tracing::debug!("No transaction ID stored for session {}, ACK may fail", session_id.0);
            // The dialog-core API doesn't have a direct send_ack without transaction ID
            // so we'll need to handle this case differently in production
        }
        
        Ok(())
    }
    
    /// Send BYE to terminate call (for state machine)
    pub async fn send_bye_session(&self, session_id: &SessionId) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        self.dialog_api
            .send_bye(&dialog_id)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send BYE: {}", e)))?;
        
        Ok(())
    }
    
    /// Send CANCEL to cancel pending INVITE
    pub async fn send_cancel(&self, session_id: &SessionId) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        self.dialog_api
            .send_cancel(&dialog_id)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send CANCEL: {}", e)))?;
        
        Ok(())
    }
    
    /// Send an in-dialog INFO request (RFC 6086) with a caller-chosen
    /// `Content-Type`. Used for SIP-INFO DTMF (`application/dtmf-relay`),
    /// fax flow control (`application/sipfrag`), and other application-level
    /// mid-dialog signalling.
    pub async fn send_info(
        &self,
        session_id: &SessionId,
        content_type: &str,
        body: &[u8],
    ) -> Result<()> {
        let dialog_id = self
            .session_to_dialog
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        self.dialog_api
            .send_info_with_content_type(
                &dialog_id,
                content_type.to_string(),
                bytes::Bytes::copy_from_slice(body),
            )
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send INFO: {}", e)))?;

        tracing::debug!(
            session = %session_id.0,
            content_type = %content_type,
            body_len = body.len(),
            "Sent INFO"
        );
        Ok(())
    }

    /// Send REFER for blind transfer (for state machine)
    pub async fn send_refer_session(&self, session_id: &SessionId, refer_to: &str) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        // Send REFER through dialog API
        self.dialog_api
            .send_refer(&dialog_id, refer_to.to_string(), None)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REFER: {}", e)))?;
        
        tracing::info!("Sent REFER to {} for session {}", refer_to, session_id.0);
        Ok(())
    }
    
    /// Send REFER with a pre-built `Replaces` header value (RFC 3891).
    ///
    /// This is the attended-transfer primitive: the caller is responsible for
    /// constructing the Replaces value from the target dialog's Call-ID,
    /// to-tag, and from-tag (accessible via `SessionHandle::call_id()` etc.
    /// on the consultation session). Linking original + consultation sessions
    /// is an orchestration concern that lives outside this crate.
    ///
    /// The emitted header is:
    /// `Refer-To: <sip:target?Replaces=<url-encoded-replaces>>`
    pub async fn send_refer_with_replaces(
        &self,
        session_id: &SessionId,
        target_uri: &str,
        replaces: &str,
    ) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        // The target URI in the Refer-To header needs a URI-escaped Replaces
        // query parameter. Semicolons and equals signs must be percent-encoded
        // so the URI parses as a single unit (RFC 3891 §3).
        let encoded_replaces = url_escape_replaces(replaces);
        let refer_to = format!("<{}?Replaces={}>", target_uri, encoded_replaces);

        self.dialog_api
            .send_refer(&dialog_id, refer_to, None)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REFER with Replaces: {}", e)))?;

        tracing::info!(
            session = %session_id.0,
            target = %target_uri,
            "Sent REFER with Replaces"
        );
        Ok(())
    }

    /// Fetch the SIP-level dialog identity (`Call-ID`, `local_tag`, `remote_tag`)
    /// for a session. Returns `None` if the session has no dialog yet
    /// (e.g., the INVITE hasn't been sent) or the dialog was lost.
    ///
    /// Callers use this to construct a Replaces header value when driving
    /// attended transfer from a higher layer.
    pub async fn dialog_identity(&self, session_id: &SessionId) -> Result<Option<DialogIdentity>> {
        let dialog_id = match self.session_to_dialog.get(session_id) {
            Some(entry) => entry.clone(),
            None => return Ok(None),
        };

        let dialog = match self.dialog_api.get_dialog_info(&dialog_id).await {
            Ok(d) => d,
            Err(_) => return Ok(None),
        };

        Ok(Some(DialogIdentity {
            call_id: dialog.call_id,
            local_tag: dialog.local_tag,
            remote_tag: dialog.remote_tag,
        }))
    }
    
    /// Send a re-INVITE for hold/resume or mid-call SDP updates.
    ///
    /// RFC 3261 §14 — re-INVITE is the standard mechanism for modifying an
    /// established dialog's session parameters (SDP direction attributes for
    /// hold/resume, codec changes, etc.). This previously routed through
    /// UPDATE (RFC 3311) which caused Timer F timeouts when the remote
    /// didn't answer an UPDATE promptly; re-INVITE is both more widely
    /// supported and the RFC-recommended method here.
    pub async fn send_reinvite_session(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        use rvoip_sip_core::Method;

        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        self.dialog_api
            .send_request_in_dialog(&dialog_id, Method::Invite, Some(bytes::Bytes::from(sdp)))
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send re-INVITE: {}", e)))?;

        Ok(())
    }
    
    /// Clean up all mappings and resources for a session
    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        // Remove from all mappings
        if let Some(dialog_id) = self.session_to_dialog.remove(session_id) {
            self.dialog_to_session.remove(&dialog_id.1);
        }

        if let Some(entry) = self.callid_to_session.iter()
            .find(|entry| entry.value() == session_id) {
            let call_id = entry.key().clone();
            drop(entry); // Release the reference before removing
            self.callid_to_session.remove(&call_id);
        }

        self.outgoing_invite_tx.remove(session_id);

        tracing::debug!("Cleaned up dialog adapter mappings for session {}", session_id.0);
        Ok(())
    }

    // ===== Registration Methods =====

    /// Send REGISTER request and process response
    pub async fn send_register(
        &self,
        session_id: &SessionId,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
        credentials: Option<&crate::types::Credentials>,
    ) -> Result<()> {
        tracing::info!("Sending REGISTER for session {} to {} (expires={})",
            session_id.0, registrar_uri, expires);

        // Build authorization header if credentials provided
        let authorization = if let Some(creds) = credentials {
            // Get challenge from session
            let session = self.store.get_session(session_id).await?;
            if let Some(ref challenge) = session.auth_challenge {
                // Compute digest response using auth-core (shared module)
                tracing::info!("🔍 CLIENT: Computing digest for user={}, realm={}, nonce={}, uri={}",
                               creds.username, challenge.realm, challenge.nonce, registrar_uri);
                
                let (response, cnonce) = crate::auth::DigestAuth::compute_response(
                    &creds.username,
                    &creds.password,
                    &challenge,
                    "REGISTER",
                    registrar_uri,
                )?;
                
                tracing::info!("🔍 CLIENT: Computed response hash: {} (cnonce: {:?})", response, cnonce);
                
                // Format authorization header using auth-core (shared module)
                // Pass the same cnonce that was used in computation!
                let auth_header = crate::auth::DigestAuth::format_authorization(
                    &creds.username,
                    &challenge,
                    registrar_uri,
                    &response,
                    cnonce.as_deref(),  // Pass cnonce from computation
                );
                
                tracing::debug!("Computed digest auth for user {}", creds.username);
                Some(auth_header)
            } else {
                tracing::debug!("No challenge stored, sending without auth");
                None
            }
        } else {
            None
        };

        // Send REGISTER through dialog-core API and get response
        // dialog-core handles CSeq incrementing automatically
        let response = self.dialog_api.send_register(
            registrar_uri,
            from_uri,
            contact_uri,
            expires,
            authorization,
        ).await
        .map_err(|e| SessionError::DialogError(format!("Failed to send REGISTER: {}", e)))?;
        
        tracing::info!("REGISTER response received: {} for session {}", response.status_code(), session_id.0);
        
        // Just update session state based on response - don't trigger events (avoids recursion)
        // The state machine will query the session state to determine next transition
        match response.status_code() {
            200..=299 => {
                // Registration successful!
                let mut session = self.store.get_session(session_id).await?;
                session.is_registered = true;
                self.store.update_session(session).await?;
                tracing::info!("✅ Registration successful - session {} marked as registered", session_id.0);
                
                // Don't trigger state machine events here to avoid recursion
                // State machine can query is_registered flag if needed
                tracing::debug!("Session marked as registered, state machine can check is_registered flag");
            }
            401 | 407 => {
                // RFC 3261 §22.2 — auth challenge on REGISTER. Unified with the
                // INVITE auth path: dispatch `EventType::AuthRequired` into the
                // state machine and let the `Registering + AuthRequired →
                // Registering` transition drive the retry via
                // `StoreAuthChallenge` + `SendREGISTERWithAuth`. No inline
                // loop here — keeps the retry policy in one place and gives
                // session-scoped observability through the state-table.
                //
                // The cap lives on `registration_retry_count`: on a second
                // 401 we mark the session unregistered and surface failure
                // instead of re-firing the event (prevents infinite loops
                // when the credentials are wrong).
                use rvoip_sip_core::types::headers::HeaderAccess;
                let header_name = if response.status_code() == 407 {
                    rvoip_sip_core::types::header::HeaderName::ProxyAuthenticate
                } else {
                    rvoip_sip_core::types::header::HeaderName::WwwAuthenticate
                };
                let challenge_opt = response.raw_header_value(&header_name);

                let session_snapshot = self.store.get_session(session_id).await?;
                let retry_count = session_snapshot.registration_retry_count;

                if let Some(challenge) = challenge_opt {
                    if retry_count >= 1 {
                        tracing::error!(
                            "❌ REGISTER auth failed (retry count {}); invalid credentials",
                            retry_count
                        );
                        let mut session = self.store.get_session(session_id).await?;
                        session.is_registered = false;
                        self.store.update_session(session).await?;
                        return Ok(());
                    }
                    {
                        let mut session = self.store.get_session(session_id).await?;
                        session.registration_retry_count += 1;
                        self.store.update_session(session).await?;
                    }
                    tracing::info!(
                        "🔄 REGISTER {} challenge for session {} — dispatching AuthRequired",
                        response.status_code(),
                        session_id.0,
                    );
                    if let Some(state_machine) = self.state_machine.get() {
                        // Box::pin: AuthRequired → SendREGISTERWithAuth →
                        // send_register forms an async recursion the compiler
                        // can't size inline.
                        Box::pin(state_machine.process_event(
                            session_id,
                            crate::state_table::types::EventType::AuthRequired {
                                status_code: response.status_code(),
                                challenge,
                            },
                        ))
                        .await
                        .map_err(|e| SessionError::InternalError(format!(
                            "REGISTER AuthRequired dispatch failed: {}",
                            e
                        )))?;
                    } else {
                        tracing::warn!(
                            "No state_machine wired into DialogAdapter; REGISTER auth cannot retry"
                        );
                    }
                } else {
                    tracing::warn!(
                        "REGISTER {} without challenge header — marking unregistered",
                        response.status_code()
                    );
                    let mut session = self.store.get_session(session_id).await?;
                    session.is_registered = false;
                    self.store.update_session(session).await?;
                }
            }
            423 => {
                // RFC 3261 §10.2.8 — Interval Too Brief. The registrar requires
                // a minimum expiry; it MUST include a Min-Expires header with
                // its minimum acceptable value. Retry once using that value.
                use rvoip_sip_core::types::headers::HeaderAccess;
                let min_expires = response
                    .raw_header_value(&rvoip_sip_core::types::header::HeaderName::MinExpires)
                    .and_then(|s| s.trim().parse::<u32>().ok());

                let session = self.store.get_session(session_id).await?;
                // Cap retries at 2 attempts to avoid loops if a broken registrar
                // keeps sending 423 regardless of the expiry we send.
                if session.registration_retry_count >= 2 {
                    tracing::error!(
                        "❌ Registration failed with repeated 423 — giving up (retry count {})",
                        session.registration_retry_count
                    );
                    let mut session = self.store.get_session(session_id).await?;
                    session.is_registered = false;
                    self.store.update_session(session).await?;
                    return Ok(());
                }

                let new_expires = match min_expires {
                    Some(min) if min > 0 && min <= 7200 => min,
                    Some(min) => {
                        tracing::warn!("423 Min-Expires={} out of sane range; clamping to 3600", min);
                        min.min(3600)
                    }
                    None => {
                        tracing::error!("423 Interval Too Brief without Min-Expires header — cannot retry");
                        let mut session = self.store.get_session(session_id).await?;
                        session.is_registered = false;
                        self.store.update_session(session).await?;
                        return Ok(());
                    }
                };

                tracing::info!(
                    "🔄 423 Interval Too Brief — retrying REGISTER with Expires={} (server required min)",
                    new_expires
                );

                // Persist new expiry and bump the retry counter.
                let mut session = self.store.get_session(session_id).await?;
                session.registration_expires = Some(new_expires);
                session.registration_retry_count += 1;
                self.store.update_session(session).await?;

                // Re-issue with the required expiry. Credentials, if any, get
                // reused (we have the challenge stored). `Box::pin` to prevent
                // the recursive async future from blowing up its size on the
                // stack, matching the 401/407 path above.
                Box::pin(self.send_register(
                    session_id,
                    registrar_uri,
                    from_uri,
                    contact_uri,
                    new_expires,
                    credentials,
                )).await?;
            }
            _ => {
                // Registration failed
                tracing::warn!("❌ Registration failed with status {}", response.status_code());
                let mut session = self.store.get_session(session_id).await?;
                session.is_registered = false;
                self.store.update_session(session).await?;
            }
        }

        Ok(())
    }

    pub async fn send_subscribe(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        to_uri: &str,
        event_package: &str,
        expires: u32,
    ) -> Result<()> {
        tracing::info!("Sending SUBSCRIBE for session {} from {} to {} for event {}",
            session_id.0, from_uri, to_uri, event_package);

        // Build SUBSCRIBE request
        let request = rvoip_sip_core::builder::SimpleRequestBuilder::subscribe(to_uri, event_package, expires)
            .map_err(|e| SessionError::DialogError(format!("Failed to create SUBSCRIBE builder: {}", e)))?
            .from("", from_uri, None)
            .to("", to_uri, None)
            .build();

        // Parse destination address
        let destination = self.parse_sip_uri_to_socket_addr(to_uri)?;

        // Send as non-dialog request (creates dialog on 2xx)
        let response = self.dialog_api.send_non_dialog_request(
            request,
            destination,
            std::time::Duration::from_secs(30),
        ).await
            .map_err(|e| SessionError::DialogError(format!("Failed to send SUBSCRIBE: {}", e)))?;

        tracing::info!("SUBSCRIBE response: {} for session {}", response.status_code(), session_id.0);

        // Handle response and potentially store dialog ID
        if response.status_code() == 200 || response.status_code() == 202 {
            // Extract dialog ID from response if present
            // This would normally come from the response headers
            // For now, emit subscription accepted event
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::SubscriptionAccepted {
                    session_id: session_id.0.clone(),
                }
            );
            let _ = self.global_coordinator.publish(Arc::new(event)).await;
        } else if response.status_code() >= 400 {
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::SubscriptionFailed {
                    session_id: session_id.0.clone(),
                    status_code: response.status_code(),
                }
            );
            let _ = self.global_coordinator.publish(Arc::new(event)).await;
        }

        Ok(())
    }

    /// Send a NOTIFY request within a subscription dialog
    pub async fn send_notify(
        &self,
        session_id: &SessionId,
        event_package: &str,
        body: Option<String>,
        subscription_state: Option<String>
    ) -> Result<()> {
        tracing::info!("Sending NOTIFY for session {} with event {} and state {:?}",
            session_id.0, event_package, subscription_state);

        // Get dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::DialogError("No dialog for session".to_string()))?
            .clone();

        // Send NOTIFY within the dialog
        self.dialog_api.send_notify(&dialog_id, event_package.to_string(), body, subscription_state).await
            .map_err(|e| SessionError::DialogError(format!("Failed to send NOTIFY: {}", e)))?;

        tracing::info!("NOTIFY sent successfully for session {}", session_id.0);
        Ok(())
    }

    /// Send NOTIFY for REFER implicit subscription (RFC 3515)
    ///
    /// Convenience method that automatically formats NOTIFY for transfer progress
    pub async fn send_refer_notify(
        &self,
        session_id: &SessionId,
        status_code: u16,
        reason: &str
    ) -> Result<()> {
        tracing::info!("Sending REFER NOTIFY for session {} with status {} {}",
            session_id.0, status_code, reason);

        // Get dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::DialogError("No dialog for session".to_string()))?
            .clone();

        // Send REFER NOTIFY using dialog-core convenience method
        self.dialog_api.send_refer_notify(&dialog_id, status_code, reason).await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REFER NOTIFY: {}", e)))?;

        tracing::info!("REFER NOTIFY sent successfully for session {}", session_id.0);
        Ok(())
    }

    // ===== MESSAGE Methods =====

    /// Send a MESSAGE request (can be in-dialog or out-of-dialog)
    pub async fn send_message(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        to_uri: &str,
        body: String,
        in_dialog: bool,
    ) -> Result<()> {
        tracing::info!("Sending MESSAGE for session {} from {} to {} (in_dialog: {})",
            session_id.0, from_uri, to_uri, in_dialog);

        if in_dialog {
            // Send MESSAGE within existing dialog
            let dialog_id = self.session_to_dialog.get(session_id)
                .ok_or_else(|| SessionError::DialogError("No dialog for session".to_string()))?
                .clone();

            self.dialog_api.send_request_in_dialog(
                &dialog_id,
                rvoip_sip_core::Method::Message,
                Some(bytes::Bytes::from(body)),
            ).await
                .map_err(|e| SessionError::DialogError(format!("Failed to send MESSAGE in dialog: {}", e)))?;
        } else {
            // Send MESSAGE as standalone (no dialog)
            let request = rvoip_sip_core::builder::SimpleRequestBuilder::new(
                rvoip_sip_core::Method::Message,
                to_uri
            ).map_err(|e| SessionError::DialogError(format!("Failed to create MESSAGE builder: {}", e)))?
                .from("", from_uri, None)
                .to("", to_uri, None)
                .body(bytes::Bytes::from(body))
                .build();

            let destination = self.parse_sip_uri_to_socket_addr(to_uri)?;

            let response = self.dialog_api.send_non_dialog_request(
                request,
                destination,
                std::time::Duration::from_secs(10),
            ).await
                .map_err(|e| SessionError::DialogError(format!("Failed to send MESSAGE: {}", e)))?;

            // Handle response
            if response.status_code() == 200 {
                let event = RvoipCrossCrateEvent::DialogToSession(
                    rvoip_infra_common::events::cross_crate::DialogToSessionEvent::MessageDelivered {
                        session_id: session_id.0.clone(),
                    }
                );
                let _ = self.global_coordinator.publish(Arc::new(event)).await;
            } else if response.status_code() >= 400 {
                let event = RvoipCrossCrateEvent::DialogToSession(
                    rvoip_infra_common::events::cross_crate::DialogToSessionEvent::MessageFailed {
                        session_id: session_id.0.clone(),
                        status_code: response.status_code(),
                    }
                );
                let _ = self.global_coordinator.publish(Arc::new(event)).await;
            }
        }

        tracing::info!("MESSAGE sent successfully for session {}", session_id.0);
        Ok(())
    }

    // ===== Helper Methods =====

    /// Parse SIP URI to SocketAddr
    fn parse_sip_uri_to_socket_addr(&self, uri: &str) -> Result<std::net::SocketAddr> {
        // Simple parsing - in production would use proper SIP URI parser
        // Expected format: sip:user@host:port or sip:user@host (default port 5060)
        let parts: Vec<&str> = uri.split('@').collect();
        if parts.len() != 2 {
            return Err(SessionError::DialogError(format!("Invalid SIP URI: {}", uri)));
        }

        let host_part = parts[1];
        let addr = if host_part.contains(':') {
            host_part.parse()
        } else {
            format!("{}:5060", host_part).parse()
        };

        addr.map_err(|e| SessionError::DialogError(format!("Failed to parse address from {}: {}", uri, e)))
    }
    
    // ===== Inbound Events (from dialog-core) =====
    
    /// Start the dialog API (no event handling here)
    pub async fn start(&self) -> Result<()> {
        // Start the dialog API
        self.dialog_api
            .start()
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to start dialog API: {}", e)))?;
        
        Ok(())
    }
}

impl Clone for DialogAdapter {
    fn clone(&self) -> Self {
        Self {
            dialog_api: self.dialog_api.clone(),
            store: self.store.clone(),
            session_to_dialog: self.session_to_dialog.clone(),
            dialog_to_session: self.dialog_to_session.clone(),
            callid_to_session: self.callid_to_session.clone(),
            outgoing_invite_tx: self.outgoing_invite_tx.clone(),
            global_coordinator: self.global_coordinator.clone(),
            state_machine: self.state_machine.clone(),
        }
    }
}

// Percent-encode the characters in a Replaces header value that would
// otherwise terminate the URI header embedded in Refer-To. Per RFC 3891
// §3 + RFC 3261 §19.1.1, reserved/delimiter characters (`;`, `=`, `?`)
// must be escaped when a header value is carried as a URI header
// parameter. Space and `@` are escaped too since they may appear in
// pathological but still valid tag values.
fn url_escape_replaces(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        match b {
            b';' | b'=' | b'?' | b' ' | b'@' | b'&' | b'#' | b'<' | b'>' | b'"' | b'%' => {
                out.push_str(&format!("%{:02X}", b));
            }
            _ => out.push(b as char),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_replaces_header_value() {
        let replaces = "abc@host;to-tag=xyz;from-tag=pqr";
        let escaped = url_escape_replaces(replaces);
        assert_eq!(escaped, "abc%40host%3Bto-tag%3Dxyz%3Bfrom-tag%3Dpqr");
    }
}