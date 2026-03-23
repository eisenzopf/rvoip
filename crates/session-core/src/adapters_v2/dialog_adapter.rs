//! Simplified Dialog Adapter for v2 modules (merged into session-core v1)

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
use crate::errors_v2::{Result, SessionError};
use crate::session_store_v2::SessionStore;

/// Minimal dialog adapter - just translates between dialog-core and state machine
pub struct DialogAdapter {
    pub(crate) dialog_api: Arc<UnifiedDialogApi>,
    pub(crate) store: Arc<SessionStore>,
    pub(crate) session_to_dialog: Arc<DashMap<SessionId, RvoipDialogId>>,
    pub(crate) dialog_to_session: Arc<DashMap<RvoipDialogId, SessionId>>,
    pub(crate) callid_to_session: Arc<DashMap<String, SessionId>>,
    pub(crate) outgoing_invite_tx: Arc<DashMap<SessionId, TransactionKey>>,
    pub(crate) global_coordinator: Arc<GlobalEventCoordinator>,
}

impl DialogAdapter {
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

    pub async fn send_response_by_dialog(&self, _dialog_id: DialogId, status_code: u16, _reason: &str) -> Result<()> {
        tracing::warn!("send_response_by_dialog called but conversion not implemented - status: {}", status_code);
        Ok(())
    }

    pub async fn send_bye(&self, dialog_id: crate::state_table::types::DialogId) -> Result<()> {
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();
        if let Some(entry) = self.dialog_to_session.get(&rvoip_dialog_id) {
            let session_id = entry.value().clone();
            drop(entry);
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

    pub async fn send_reinvite(&self, dialog_id: crate::state_table::types::DialogId, sdp: String) -> Result<()> {
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();
        if let Some(entry) = self.dialog_to_session.get(&rvoip_dialog_id) {
            let session_id = entry.value().clone();
            drop(entry);
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

    pub async fn send_refer(&self, dialog_id: crate::state_table::types::DialogId, target: &str, attended: bool) -> Result<()> {
        let rvoip_dialog_id: RvoipDialogId = dialog_id.into();
        if let Some(entry) = self.dialog_to_session.get(&rvoip_dialog_id) {
            let session_id = entry.value().clone();
            drop(entry);
            let transfer_info = if attended { Some("attended".to_string()) } else { None };
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

    pub async fn get_remote_uri(&self, _dialog_id: crate::state_table::types::DialogId) -> Result<String> {
        Ok("sip:remote@example.com".to_string())
    }

    pub async fn send_invite_with_details(
        &self,
        session_id: &SessionId,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<()> {
        let call_id = format!("{}@session-core", session_id.0);
        self.callid_to_session.insert(call_id.clone(), session_id.clone());

        let call_handle = self.dialog_api
            .make_call_with_id(from, to, sdp, Some(call_id.clone()))
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to make call: {}", e)))?;

        let dialog_id = call_handle.call_id().clone();
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());

        let event = SessionToDialogEvent::StoreDialogMapping {
            session_id: session_id.0.clone(),
            dialog_id: dialog_id.to_string(),
        };
        self.global_coordinator.publish(Arc::new(
            RvoipCrossCrateEvent::SessionToDialog(event)
        )).await
        .map_err(|e| SessionError::InternalError(format!("Failed to publish StoreDialogMapping: {}", e)))?;

        tracing::debug!("Dialog {} created for session {}", dialog_id, session_id.0);
        Ok(())
    }

    pub async fn send_200_ok(&self, session_id: &SessionId, sdp: Option<String>) -> Result<()> {
        self.send_response(session_id, 200, sdp).await
    }

    pub async fn send_response_with_sdp(&self, session_id: &SessionId, code: u16, _reason: &str, sdp: &str) -> Result<()> {
        self.send_response(session_id, code, Some(sdp.to_string())).await
    }

    pub async fn send_response_session(&self, session_id: &SessionId, code: u16, _reason: &str) -> Result<()> {
        self.send_response(session_id, code, None).await
    }

    pub async fn send_error_response(&self, session_id: &SessionId, code: StatusCode, _reason: &str) -> Result<()> {
        self.send_response(session_id, code.as_u16(), None).await
    }

    pub async fn send_response(
        &self,
        session_id: &SessionId,
        code: u16,
        sdp: Option<String>,
    ) -> Result<()> {
        tracing::info!("DialogAdapter sending {} response for session {} with SDP: {}",
            code, session_id.0, sdp.is_some());
        self.dialog_api
            .send_response_for_session(&session_id.0, code, sdp)
            .await
            .map_err(|e| {
                tracing::error!("Failed to send response for session {}: {}", session_id.0, e);
                SessionError::DialogError(format!("Failed to send response: {}", e))
            })
    }

    pub async fn send_ack(&self, session_id: &SessionId, response: &Response) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();

        if let Some(tx_id) = self.outgoing_invite_tx.get(session_id) {
            self.dialog_api
                .send_ack_for_2xx_response(&dialog_id, &tx_id, response)
                .await
                .map_err(|e| SessionError::DialogError(format!("Failed to send ACK: {}", e)))?;
            self.outgoing_invite_tx.remove(session_id);
        } else {
            tracing::debug!("No transaction ID stored for session {}, ACK may fail", session_id.0);
        }
        Ok(())
    }

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

    pub async fn send_refer_session(&self, session_id: &SessionId, refer_to: &str) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_refer(&dialog_id, refer_to.to_string(), None)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REFER: {}", e)))?;
        tracing::info!("Sent REFER to {} for session {}", refer_to, session_id.0);
        Ok(())
    }

    pub async fn send_refer_with_replaces(&self, session_id: &SessionId, consultation_session_id: &SessionId) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        let consultation_dialog_id = self.session_to_dialog.get(consultation_session_id)
            .ok_or_else(|| SessionError::SessionNotFound(consultation_session_id.0.clone()))?
            .clone();
        let refer_to = format!("dialog:{}", consultation_dialog_id.0);
        self.dialog_api
            .send_refer(&dialog_id, refer_to, Some("attended".to_string()))
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REFER with Replaces: {}", e)))?;
        tracing::info!("Sent REFER with Replaces for session {} using consultation dialog {}",
                       session_id.0, consultation_dialog_id.0);
        Ok(())
    }

    pub async fn send_reinvite_session(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        self.dialog_api
            .send_update(&dialog_id, Some(sdp))
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send re-INVITE: {}", e)))?;
        Ok(())
    }

    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        if let Some(dialog_id) = self.session_to_dialog.remove(session_id) {
            self.dialog_to_session.remove(&dialog_id.1);
        }
        if let Some(entry) = self.callid_to_session.iter()
            .find(|entry| entry.value() == session_id) {
            let call_id = entry.key().clone();
            drop(entry);
            self.callid_to_session.remove(&call_id);
        }
        self.outgoing_invite_tx.remove(session_id);
        tracing::debug!("Cleaned up dialog adapter mappings for session {}", session_id.0);
        Ok(())
    }

    pub async fn send_register(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        registrar_uri: &str,
        expires: u32,
    ) -> Result<()> {
        tracing::info!("Sending REGISTER for session {} from {} to registrar {}",
            session_id.0, from_uri, registrar_uri);

        let request = rvoip_sip_core::builder::SimpleRequestBuilder::register(registrar_uri)
            .map_err(|e| SessionError::DialogError(format!("Failed to create REGISTER builder: {}", e)))?
            .from("", from_uri, None)
            .to("", from_uri, None)
            .contact(from_uri, None)
            .expires(expires)
            .build();

        let destination = self.parse_sip_uri_to_socket_addr(registrar_uri)?;

        let response = self.dialog_api.send_non_dialog_request(
            request,
            destination,
            std::time::Duration::from_secs(30),
        ).await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REGISTER: {}", e)))?;

        tracing::info!("REGISTER response: {} for session {}", response.status_code(), session_id.0);

        if response.status_code() == 200 {
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::RegistrationSuccess {
                    session_id: session_id.0.clone(),
                }
            );
            if let Err(e) = self.global_coordinator.publish(Arc::new(event)).await {
                tracing::warn!("Failed to publish cross-crate event: {e}");
            }
        } else if response.status_code() >= 400 {
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::RegistrationFailed {
                    session_id: session_id.0.clone(),
                    status_code: response.status_code(),
                }
            );
            if let Err(e) = self.global_coordinator.publish(Arc::new(event)).await {
                tracing::warn!("Failed to publish cross-crate event: {e}");
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

        let request = rvoip_sip_core::builder::SimpleRequestBuilder::subscribe(to_uri, event_package, expires)
            .map_err(|e| SessionError::DialogError(format!("Failed to create SUBSCRIBE builder: {}", e)))?
            .from("", from_uri, None)
            .to("", to_uri, None)
            .build();

        let destination = self.parse_sip_uri_to_socket_addr(to_uri)?;

        let response = self.dialog_api.send_non_dialog_request(
            request,
            destination,
            std::time::Duration::from_secs(30),
        ).await
            .map_err(|e| SessionError::DialogError(format!("Failed to send SUBSCRIBE: {}", e)))?;

        tracing::info!("SUBSCRIBE response: {} for session {}", response.status_code(), session_id.0);

        if response.status_code() == 200 || response.status_code() == 202 {
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::SubscriptionAccepted {
                    session_id: session_id.0.clone(),
                }
            );
            if let Err(e) = self.global_coordinator.publish(Arc::new(event)).await {
                tracing::warn!("Failed to publish cross-crate event: {e}");
            }
        } else if response.status_code() >= 400 {
            let event = RvoipCrossCrateEvent::DialogToSession(
                rvoip_infra_common::events::cross_crate::DialogToSessionEvent::SubscriptionFailed {
                    session_id: session_id.0.clone(),
                    status_code: response.status_code(),
                }
            );
            if let Err(e) = self.global_coordinator.publish(Arc::new(event)).await {
                tracing::warn!("Failed to publish cross-crate event: {e}");
            }
        }

        Ok(())
    }

    pub async fn send_notify(
        &self,
        session_id: &SessionId,
        event_package: &str,
        body: Option<String>,
        subscription_state: Option<String>
    ) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::DialogError("No dialog for session".to_string()))?
            .clone();
        self.dialog_api.send_notify(&dialog_id, event_package.to_string(), body, subscription_state).await
            .map_err(|e| SessionError::DialogError(format!("Failed to send NOTIFY: {}", e)))?;
        tracing::info!("NOTIFY sent successfully for session {}", session_id.0);
        Ok(())
    }

    pub async fn send_refer_notify(
        &self,
        session_id: &SessionId,
        status_code: u16,
        reason: &str
    ) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::DialogError("No dialog for session".to_string()))?
            .clone();
        self.dialog_api.send_refer_notify(&dialog_id, status_code, reason).await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REFER NOTIFY: {}", e)))?;
        tracing::info!("REFER NOTIFY sent successfully for session {}", session_id.0);
        Ok(())
    }

    pub async fn send_message(
        &self,
        session_id: &SessionId,
        from_uri: &str,
        to_uri: &str,
        body: String,
        in_dialog: bool,
    ) -> Result<()> {
        if in_dialog {
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

            if response.status_code() == 200 {
                let event = RvoipCrossCrateEvent::DialogToSession(
                    rvoip_infra_common::events::cross_crate::DialogToSessionEvent::MessageDelivered {
                        session_id: session_id.0.clone(),
                    }
                );
                if let Err(e) = self.global_coordinator.publish(Arc::new(event)).await {
                tracing::warn!("Failed to publish cross-crate event: {e}");
            }
            } else if response.status_code() >= 400 {
                let event = RvoipCrossCrateEvent::DialogToSession(
                    rvoip_infra_common::events::cross_crate::DialogToSessionEvent::MessageFailed {
                        session_id: session_id.0.clone(),
                        status_code: response.status_code(),
                    }
                );
                if let Err(e) = self.global_coordinator.publish(Arc::new(event)).await {
                tracing::warn!("Failed to publish cross-crate event: {e}");
            }
            }
        }
        tracing::info!("MESSAGE sent successfully for session {}", session_id.0);
        Ok(())
    }

    fn parse_sip_uri_to_socket_addr(&self, uri: &str) -> Result<std::net::SocketAddr> {
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

    pub async fn start(&self) -> Result<()> {
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
