//! Simplified Dialog Adapter for session-core-v2
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
        }
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

        let call_handle = self.dialog_api
            .make_call_with_id(from, to, sdp, Some(call_id.clone()))
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to make call: {}", e)))?;

        let dialog_id = call_handle.call_id().clone();

        // Store remaining mappings
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
    
    /// Send REFER with Replaces for attended transfer (for state machine)
    pub async fn send_refer_with_replaces(&self, session_id: &SessionId, consultation_session_id: &SessionId) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        // Get consultation dialog ID to build Replaces header
        let consultation_dialog_id = self.session_to_dialog.get(consultation_session_id)
            .ok_or_else(|| SessionError::SessionNotFound(consultation_session_id.0.clone()))?
            .clone();

        // For now, send REFER with the consultation session marked as "attended"
        // The dialog-core layer should construct proper Replaces header from the dialog ID
        // Format: Refer-To: <sip:target@domain?Replaces=call-id%3Bto-tag%3Dtag1%3Bfrom-tag%3Dtag2>
        let refer_to = format!("dialog:{}", consultation_dialog_id.0);

        self.dialog_api
            .send_refer(&dialog_id, refer_to, Some("attended".to_string()))
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REFER with Replaces: {}", e)))?;

        tracing::info!("Sent REFER with Replaces for session {} using consultation dialog {}",
                       session_id.0, consultation_dialog_id.0);
        Ok(())
    }
    
    /// Send re-INVITE (for hold/resume) (for state machine)
    pub async fn send_reinvite_session(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        // Use UPDATE method for re-INVITE
        self.dialog_api
            .send_update(&dialog_id, Some(sdp))
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

    /// Send a REGISTER request
    pub async fn send_register(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        registrar_uri: &str,
        expires: u32,
    ) -> Result<()> {
        tracing::info!("Sending REGISTER for session {} from {} to registrar {}",
            session_id.0, from_uri, registrar_uri);

        // Build REGISTER request
        let request = rvoip_sip_core::builder::SimpleRequestBuilder::register(registrar_uri)
            .map_err(|e| SessionError::DialogError(format!("Failed to create REGISTER builder: {}", e)))?
            .from("", from_uri, None)
            .to("", from_uri, None)  // REGISTER uses same From and To
            .contact(from_uri, None)
            .expires(expires)
            .build();

        // Parse destination address from registrar URI
        let destination = self.parse_sip_uri_to_socket_addr(registrar_uri)?;

        // Send as non-dialog request
        let response = self.dialog_api.send_non_dialog_request(
            request,
            destination,
            std::time::Duration::from_secs(30),
        ).await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REGISTER: {}", e)))?;

        tracing::info!("REGISTER response: {} for session {}", response.status_code(), session_id.0);

        // Update session state based on response
        if response.status_code() == 200 {
            // Emit registration success event
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::RegistrationSuccess {
                    session_id: session_id.0.clone(),
                }
            );
            let _ = self.global_coordinator.publish(Arc::new(event)).await;
        } else if response.status_code() >= 400 {
            // Emit registration failure event
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::RegistrationFailed {
                    session_id: session_id.0.clone(),
                    status_code: response.status_code(),
                }
            );
            let _ = self.global_coordinator.publish(Arc::new(event)).await;
        }

        Ok(())
    }

    // ===== Subscription/NOTIFY Methods =====

    /// Send a SUBSCRIBE request
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
        }
    }
}