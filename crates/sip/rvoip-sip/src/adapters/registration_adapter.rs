//! Server-side REGISTER request handler
//!
//! This adapter orchestrates authentication between dialog-core (protocol layer)
//! and registrar-core (storage/validation layer) via the global event bus.
//!
//! ## Architecture
//!
//! ```text
//! dialog-core → IncomingRegister event → RegistrationAdapter → registrar-core
//!            ← SendRegisterResponse event ← RegistrationAdapter ←
//! ```

use crate::errors::{Result, SessionError};
use rvoip_infra_common::events::{
    coordinator::GlobalEventCoordinator,
    cross_crate::{DialogToSessionEvent, RvoipCrossCrateEvent, SessionToDialogEvent},
};
use rvoip_sip_registrar::{
    AddressOfRecord, ContactInfo, ContactReachability, RegistrarService, Transport,
};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Handles server-side REGISTER requests by coordinating authentication
pub struct RegistrationAdapter {
    registrar: Arc<RegistrarService>,
    global_coordinator: Arc<GlobalEventCoordinator>,
}

impl RegistrationAdapter {
    /// Create a new registration adapter
    pub fn new(
        registrar: Arc<RegistrarService>,
        global_coordinator: Arc<GlobalEventCoordinator>,
    ) -> Self {
        Self {
            registrar,
            global_coordinator,
        }
    }

    /// Handle incoming REGISTER request from dialog-core
    async fn handle_incoming_register(
        &self,
        transaction_id: String,
        from_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>,
    ) -> Result<()> {
        info!(
            from_present = !from_uri.is_empty(),
            from_len = from_uri.len(),
            authorization_present = authorization.is_some(),
            "Handling incoming REGISTER"
        );

        let aor = Self::extract_aor(&from_uri)?;
        let username = aor.user().to_string();
        debug!(
            username_present = !username.is_empty(),
            username_len = username.len(),
            "Extracted registration identity metadata"
        );

        // Call registrar-core to authenticate
        let (should_register, www_auth_challenge) = self
            .registrar
            .authenticate_register(&username, authorization.as_deref(), "REGISTER", &from_uri)
            .await
            .map_err(Self::registrar_authentication_failure)?;

        if should_register {
            // Valid credentials - register user
            info!(
                username_present = !username.is_empty(),
                username_len = username.len(),
                "REGISTER authentication succeeded"
            );

            // Build ContactInfo
            let contact = ContactInfo {
                uri: contact_uri.clone(),
                instance_id: uuid::Uuid::new_v4().to_string(),
                transport: Transport::UDP,
                user_agent: "rvoip-sip".to_string(),
                expires: chrono::Utc::now()
                    + chrono::Duration::try_seconds(expires as i64)
                        .unwrap_or_else(|| chrono::Duration::seconds(3600)),
                q_value: 1.0,
                received: None,
                path: Vec::new(),
                methods: vec!["INVITE".to_string(), "ACK".to_string(), "BYE".to_string()],
                reg_id: None,
                flow_id: None,
                reachability: ContactReachability::Unknown,
            };

            // Register the full AOR in registrar-core so domains do not collide.
            self.registrar
                .register_aor(&aor, contact, Some(expires))
                .await
                .map_err(Self::registrar_storage_failure)?;

            // Publish 200 OK response event
            let response_event =
                RvoipCrossCrateEvent::SessionToDialog(SessionToDialogEvent::SendRegisterResponse {
                    transaction_id,
                    status_code: 200,
                    reason: "OK".to_string(),
                    www_authenticate: None,
                    contact: Some(contact_uri),
                    expires: Some(expires),
                    min_expires: None,
                    service_route: Vec::new(),
                    path_echo: false,
                    associated_uri: Vec::new(),
                    extra_headers: Vec::new(),
                });

            self.global_coordinator
                .publish(Arc::new(response_event))
                .await
                .map_err(Self::registration_response_publish_failure)?;

            info!("REGISTER accepted and response published");
        } else {
            // Need authentication - send 401 challenge
            info!("REGISTER rejected; publishing authentication challenge");

            let response_event =
                RvoipCrossCrateEvent::SessionToDialog(SessionToDialogEvent::SendRegisterResponse {
                    transaction_id,
                    status_code: 401,
                    reason: "Unauthorized".to_string(),
                    www_authenticate: www_auth_challenge,
                    contact: None,
                    expires: None,
                    min_expires: None,
                    service_route: Vec::new(),
                    path_echo: false,
                    associated_uri: Vec::new(),
                    extra_headers: Vec::new(),
                });

            self.global_coordinator
                .publish(Arc::new(response_event))
                .await
                .map_err(Self::registration_response_publish_failure)?;

            info!("REGISTER authentication challenge published");
        }

        Ok(())
    }

    /// Subscribe to IncomingRegister events and start handling them
    pub async fn start(self: Arc<Self>) -> Result<()> {
        info!("🎬 Starting RegistrationAdapter - subscribing to dialog_to_session events");

        // Subscribe to dialog-to-session events
        let mut receiver = self
            .global_coordinator
            .subscribe("dialog_to_session")
            .await
            .map_err(Self::registration_event_subscription_failure)?;

        let handler = self.clone();

        tokio::spawn(async move {
            info!("🔔 RegistrationAdapter event loop started");

            loop {
                // Receive event from bus
                match receiver.recv().await {
                    Some(event_arc) => {
                        // Use trait-based downcasting via as_any()
                        if let Some(concrete) =
                            event_arc.as_any().downcast_ref::<RvoipCrossCrateEvent>()
                        {
                            // Check if it's an IncomingRegister event
                            if let RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::IncomingRegister {
                                    transaction_id,
                                    from_uri,
                                    contact_uri,
                                    expires,
                                    authorization,
                                    ..
                                },
                            ) = concrete
                            {
                                debug!(
                                    from_present = !from_uri.is_empty(),
                                    from_len = from_uri.len(),
                                    authorization_present = authorization.is_some(),
                                    "Received IncomingRegister"
                                );

                                if let Err(e) = handler
                                    .handle_incoming_register(
                                        transaction_id.clone(),
                                        from_uri.clone(),
                                        contact_uri.clone(),
                                        *expires,
                                        authorization.clone(),
                                    )
                                    .await
                                {
                                    warn!("Failed to handle REGISTER: {}", e);
                                }
                            }
                        }
                    }
                    None => {
                        debug!("RegistrationAdapter event channel closed");
                        break;
                    }
                }
            }

            info!("🛑 RegistrationAdapter event loop stopped");
        });

        info!("✅ RegistrationAdapter started and subscribed to dialog_to_session events");
        Ok(())
    }

    fn extract_aor(uri: &str) -> Result<AddressOfRecord> {
        AddressOfRecord::parse(uri).map_err(|_| {
            SessionError::InvalidInput("invalid registration address-of-record".into())
        })
    }

    fn registrar_authentication_failure<E>(_source: E) -> SessionError {
        SessionError::RegistrationFailed("registrar authentication failed".into())
    }

    fn registrar_storage_failure<E>(_source: E) -> SessionError {
        SessionError::RegistrationFailed("registrar storage failed".into())
    }

    fn registration_response_publish_failure<E>(_source: E) -> SessionError {
        SessionError::InternalError("registration response publish failed".into())
    }

    fn registration_event_subscription_failure<E>(_source: E) -> SessionError {
        SessionError::InternalError("registration event subscription failed".into())
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    const CANARY: &str = "peer-value\r\nX-Registration-Canary: exposed";

    #[derive(Debug)]
    struct MaliciousLowerError;

    impl std::fmt::Display for MaliciousLowerError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(CANARY)
        }
    }

    impl std::error::Error for MaliciousLowerError {}

    fn assert_redacted(error: SessionError, expected_detail: &str) {
        let detail = match &error {
            SessionError::InvalidInput(detail)
            | SessionError::RegistrationFailed(detail)
            | SessionError::InternalError(detail) => detail,
            other => panic!("unexpected registration error: {other:?}"),
        };
        assert_eq!(detail, expected_detail);
        assert!(!error.to_string().contains(CANARY));
        assert!(!format!("{error:?}").contains(CANARY));
    }

    #[test]
    fn malformed_aor_error_does_not_echo_peer_input_or_parser_error() {
        let error = RegistrationAdapter::extract_aor(CANARY).expect_err("malformed AOR");
        assert_redacted(error, "invalid registration address-of-record");
    }

    #[test]
    fn registrar_and_event_bus_errors_collapse_to_fixed_stage_classes() {
        assert_redacted(
            RegistrationAdapter::registrar_authentication_failure(MaliciousLowerError),
            "registrar authentication failed",
        );
        assert_redacted(
            RegistrationAdapter::registrar_storage_failure(MaliciousLowerError),
            "registrar storage failed",
        );
        assert_redacted(
            RegistrationAdapter::registration_response_publish_failure(MaliciousLowerError),
            "registration response publish failed",
        );
        assert_redacted(
            RegistrationAdapter::registration_event_subscription_failure(MaliciousLowerError),
            "registration event subscription failed",
        );
    }
}
