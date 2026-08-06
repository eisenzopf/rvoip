//! Server-side REGISTER request handler
//!
//! This adapter orchestrates authentication between dialog-core (protocol layer)
//! and registrar-core (storage/validation layer). Inbound requests arrive from
//! the one authoritative typed dialog-to-session router; responses use the
//! exact dialog transaction API.
//!
//! ## Architecture
//!
//! ```text
//! dialog-core → IncomingRegister event → RegistrationAdapter → registrar-core
//!            ← exact transaction response ← RegistrationAdapter ←
//! ```

use crate::errors::{Result, SessionError};
use rvoip_infra_common::events::{
    coordinator::GlobalEventCoordinator,
    cross_crate::{CrossCrateEvent, DialogToSessionEvent, EventTypeId},
    types::EventPriority,
};
use rvoip_infra_common::planes::PlaneType;
use rvoip_sip_registrar::{
    AddressOfRecord, ContactInfo, ContactReachability, RegistrarService, Transport,
};
use std::sync::{Arc, OnceLock};
use tracing::{debug, info, warn};

/// Handles server-side REGISTER requests by coordinating authentication
pub struct RegistrationAdapter {
    registrar: Arc<RegistrarService>,
    global_coordinator: Arc<GlobalEventCoordinator>,
    dialog_adapter: OnceLock<Arc<crate::adapters::DialogAdapter>>,
}

fn classified_register_response_result(
    status_code: u16,
    result: std::result::Result<
        rvoip_sip_dialog::FinalResponseCompletionDisposition,
        rvoip_sip_dialog::ExactResponseSendError,
    >,
) -> Result<()> {
    match result {
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal)
        | Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal) => {
            Ok(())
        }
        Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) => {
            Err(RegistrationAdapter::registration_response_send_failure(
                "REGISTER response failed before transport write",
            ))
        }
        Err(error)
            if error.disposition
                == rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable =>
        {
            Err(RegistrationAdapter::registration_response_send_failure(
                error,
            ))
        }
        Err(error) => {
            // A failed transport call after crossing the first-write boundary
            // is terminal. Returning success is the causal ACK that forbids
            // dialog-core from authoring a duplicate 503.
            tracing::warn!(
                status_code,
                disposition = ?error.disposition,
                "Built-in REGISTER response became wire-unknown; retaining transaction ownership"
            );
            Ok(())
        }
    }
}

/// Private capability-bearing control message used to install the optional
/// built-in registrar into the existing authoritative dialog-to-session
/// router. It is deliberately dispatched handler-only and must never be
/// copied to the observational event bus.
pub(crate) struct RegistrationAdapterInstall {
    adapter: Arc<RegistrationAdapter>,
}

impl RegistrationAdapterInstall {
    fn new(adapter: Arc<RegistrationAdapter>) -> Self {
        Self { adapter }
    }

    pub(crate) fn adapter(&self) -> Arc<RegistrationAdapter> {
        Arc::clone(&self.adapter)
    }
}

impl std::fmt::Debug for RegistrationAdapterInstall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegistrationAdapterInstall")
            .finish_non_exhaustive()
    }
}

impl CrossCrateEvent for RegistrationAdapterInstall {
    fn event_type(&self) -> EventTypeId {
        "dialog_to_session"
    }

    fn source_plane(&self) -> PlaneType {
        PlaneType::Signaling
    }

    fn target_plane(&self) -> PlaneType {
        PlaneType::Signaling
    }

    fn priority(&self) -> EventPriority {
        EventPriority::Critical
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// How the `Contact` header of an inbound REGISTER should be treated, per
/// RFC 3261 §10.3 step 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterContactDisposition {
    /// No wildcard. Ordinary binding refresh, addition or single removal.
    Ordinary,
    /// `Contact: *` with `Expires: 0`. Every binding for the AOR goes.
    RemoveAllBindings,
    /// A wildcard alongside other contacts, or with a non-zero expiration.
    /// §10.3 requires 400 rather than a best-effort interpretation.
    InvalidWildcard,
}

/// Classify the `Contact` header of an inbound REGISTER.
///
/// > the registrar checks if there is one Contact field value that contains
/// > the special value "*" and an Expires field. If the request has additional
/// > Contact fields or an expiration time other than zero, the request is
/// > invalid, and the server MUST return a 400 (Invalid Request)
///
/// The expiration consulted here is the `Expires` header, since a wildcard
/// contact has no parameters of its own to carry one.
fn classify_register_contacts(request: &rvoip_sip_core::Request) -> RegisterContactDisposition {
    use rvoip_sip_core::types::contact::{Contact, ContactValue};
    use rvoip_sip_core::types::headers::HeaderAccess;

    let contacts: Vec<&Contact> = request.typed_headers::<Contact>();
    let has_wildcard = contacts
        .iter()
        .any(|contact| contact.0.iter().any(|value| *value == ContactValue::Star));

    if !has_wildcard {
        return RegisterContactDisposition::Ordinary;
    }

    // A wildcard must stand alone: one header, one value, and nothing else.
    let total_values: usize = contacts.iter().map(|contact| contact.0.len()).sum();
    if total_values != 1 {
        return RegisterContactDisposition::InvalidWildcard;
    }

    let expires = request
        .typed_header::<rvoip_sip_core::types::expires::Expires>()
        .map(|expires| expires.0);
    match expires {
        Some(0) => RegisterContactDisposition::RemoveAllBindings,
        _ => RegisterContactDisposition::InvalidWildcard,
    }
}

impl RegistrationAdapter {
    /// Create a registration adapter whose dialog transaction owner is
    /// installed by the authoritative router when [`Self::start`] runs.
    ///
    /// A `GlobalEventCoordinator` is not itself an exact response capability,
    /// so an adapter that is used before installation still fails closed and
    /// never falls back to a response command on the event bus.
    pub fn new(
        registrar: Arc<RegistrarService>,
        global_coordinator: Arc<GlobalEventCoordinator>,
    ) -> Self {
        Self {
            registrar,
            global_coordinator,
            dialog_adapter: OnceLock::new(),
        }
    }

    pub(crate) fn with_dialog_adapter(
        registrar: Arc<RegistrarService>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<crate::adapters::DialogAdapter>,
    ) -> Self {
        let response_owner = OnceLock::new();
        assert!(response_owner.set(dialog_adapter).is_ok());
        Self {
            registrar,
            global_coordinator,
            dialog_adapter: response_owner,
        }
    }

    pub(crate) fn install_registration_response_owner(
        &self,
        dialog_adapter: Arc<crate::adapters::DialogAdapter>,
    ) -> Result<()> {
        if let Some(installed) = self.dialog_adapter.get() {
            return if Arc::ptr_eq(installed, &dialog_adapter) {
                Ok(())
            } else {
                Err(Self::registration_response_owner_conflict())
            };
        }

        match self.dialog_adapter.set(dialog_adapter) {
            Ok(()) => Ok(()),
            Err(dialog_adapter)
                if self
                    .dialog_adapter
                    .get()
                    .is_some_and(|installed| Arc::ptr_eq(installed, &dialog_adapter)) =>
            {
                Ok(())
            }
            Err(_) => Err(Self::registration_response_owner_conflict()),
        }
    }

    fn registration_response_owner(&self) -> Result<&Arc<crate::adapters::DialogAdapter>> {
        self.dialog_adapter
            .get()
            .ok_or_else(Self::registration_response_owner_unavailable)
    }

    async fn send_register_response(
        &self,
        response: &crate::api::respond::register_response::RegisterResponseEventFields,
    ) -> Result<()> {
        let result = self
            .registration_response_owner()?
            .send_register_response_fields_classified(response)
            .await;
        classified_register_response_result(response.status_code, result)
    }

    /// Handle incoming REGISTER request from dialog-core
    async fn handle_incoming_register(
        &self,
        transaction_id: String,
        from_uri: String,
        request_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>,
        remove_all_bindings: bool,
    ) -> Result<()> {
        // Refuse the request before authentication or registrar mutation when
        // no exact transaction response capability is installed. Accepting a
        // binding without being able to author its final SIP response would be
        // a false success and replaying it through the event bus would restore
        // the duplicate causal route removed by RT-305.
        self.registration_response_owner()?;

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
            .authenticate_register_request(
                &username,
                authorization.as_deref(),
                "REGISTER",
                &request_uri,
                aor.as_str(),
            )
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

            // Fully validate and serialize the registrar mutation before
            // authoring 200, but keep the authoritative binding unchanged
            // until the exact response reaches a terminal written/wire-unknown
            // outcome. Proven ZeroWire drops this prepared value and preserves
            // the prior binding, including replacement and removal cases.
            // RFC 3261 §10.3 step 6: `Contact: *` with `Expires: 0` asks for
            // every binding of this AOR to go, not for one contact to be
            // matched and removed. Only the staged mutation differs; the
            // ordering below is the same, so the binding still survives a
            // response that never reaches the wire.
            let prepared_registration = if remove_all_bindings {
                self.registrar
                    .prepare_clear_bindings(&aor)
                    .await
                    .map_err(Self::registrar_storage_failure)?
            } else {
                self.registrar
                    .prepare_register_aor(&aor, contact, Some(expires))
                    .await
                    .map_err(Self::registrar_storage_failure)?
            };

            let response = crate::api::respond::register_response::RegisterResponseEventFields {
                transaction_id,
                status_code: 200,
                reason: "OK".to_string(),
                www_authenticate: None,
                contact: if remove_all_bindings {
                    None
                } else {
                    Some(contact_uri)
                },
                expires: Some(expires),
                min_expires: None,
                service_route: Vec::new(),
                path_echo: false,
                associated_uri: Vec::new(),
                extra_headers: Vec::new(),
            };
            self.send_register_response(&response).await?;
            prepared_registration.commit().await;

            info!("REGISTER accepted and response sent");
        } else {
            // Need authentication - send 401 challenge
            info!("REGISTER rejected; sending authentication challenge");

            let response = crate::api::respond::register_response::RegisterResponseEventFields {
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
            };
            self.send_register_response(&response).await?;

            info!("REGISTER authentication challenge sent");
        }

        Ok(())
    }

    /// Handle one authoritative typed REGISTER request.
    pub(crate) async fn handle_incoming_register_event(
        &self,
        event: &DialogToSessionEvent,
    ) -> Result<()> {
        let DialogToSessionEvent::IncomingRegister {
            transaction_id,
            from_uri,
            contact_uri,
            expires,
            authorization,
            raw_request,
            ..
        } = event
        else {
            return Err(SessionError::InvalidInput(
                "registration adapter received a non-REGISTER event".to_string(),
            ));
        };

        debug!(
            from_present = !from_uri.is_empty(),
            from_len = from_uri.len(),
            authorization_present = authorization.is_some(),
            "Received authoritative IncomingRegister"
        );

        // Digest authentication is bound to the exact Request-URI from the
        // wire. Synthetic compatibility events have no raw request, so retain
        // the historical From-URI fallback for those events only.
        let request_uri = raw_request
            .as_ref()
            .and_then(|bytes| {
                rvoip_sip_core::parse_message_with_mode(bytes, rvoip_sip_core::ParseMode::Strict)
                    .ok()
            })
            .and_then(|message| match message {
                rvoip_sip_core::Message::Request(request) => Some(request.uri().to_string()),
                rvoip_sip_core::Message::Response(_) => None,
            })
            .unwrap_or_else(|| from_uri.clone());

        // RFC 3261 §10.3 step 6, read off the wire because the flattened
        // event cannot express a wildcard: `extract_contact_uri` has no `*` to
        // return, so it falls back to the From URI and the request becomes
        // indistinguishable from an ordinary self-registration.
        let contact_disposition = raw_request
            .as_ref()
            .and_then(|bytes| {
                rvoip_sip_core::parse_message_with_mode(bytes, rvoip_sip_core::ParseMode::Strict)
                    .ok()
            })
            .and_then(|message| match message {
                rvoip_sip_core::Message::Request(request) => {
                    Some(classify_register_contacts(&request))
                }
                rvoip_sip_core::Message::Response(_) => None,
            })
            .unwrap_or(RegisterContactDisposition::Ordinary);

        // "If the request has additional Contact fields or an expiration time
        // other than zero, the request is invalid, and the server MUST return
        // a 400 (Invalid Request)."
        //
        // Answered before authentication on purpose: this is a syntactic
        // defect in the request itself, refusing it leaks nothing, and it
        // keeps a malformed wildcard from reaching registrar mutation at all.
        if contact_disposition == RegisterContactDisposition::InvalidWildcard {
            warn!("REGISTER carries a malformed wildcard Contact; rejecting with 400");
            let rejection = crate::api::respond::register_response::RegisterResponseEventFields {
                transaction_id: transaction_id.clone(),
                status_code: 400,
                reason: "Bad Request".to_string(),
                www_authenticate: None,
                contact: None,
                expires: None,
                min_expires: None,
                service_route: Vec::new(),
                path_echo: false,
                associated_uri: Vec::new(),
                extra_headers: Vec::new(),
            };
            return self.send_register_response(&rejection).await;
        }

        self.handle_incoming_register(
            transaction_id.clone(),
            from_uri.clone(),
            request_uri,
            contact_uri.clone(),
            *expires,
            authorization.clone(),
            contact_disposition == RegisterContactDisposition::RemoveAllBindings,
        )
        .await
    }

    /// Install this adapter into the one authoritative typed REGISTER route.
    pub async fn start(self: Arc<Self>) -> Result<()> {
        info!("🎬 Starting RegistrationAdapter on authoritative typed ingress");
        let coordinator = Arc::clone(&self.global_coordinator);
        let installed = coordinator
            .dispatch_authoritative_handler(Arc::new(RegistrationAdapterInstall::new(Arc::clone(
                &self,
            ))))
            .await
            .map_err(Self::registration_event_install_failure)?;
        if !installed {
            return Err(Self::registration_event_install_failure(
                "no authoritative dialog-to-session handler",
            ));
        }
        self.registration_response_owner()?;

        info!("✅ RegistrationAdapter installed on authoritative typed ingress");
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

    fn registration_response_send_failure<E>(_source: E) -> SessionError {
        SessionError::InternalError("registration response send failed".into())
    }

    fn registration_response_owner_unavailable() -> SessionError {
        SessionError::InternalError("registration response owner unavailable".into())
    }

    fn registration_response_owner_conflict() -> SessionError {
        SessionError::InternalError("registration response owner already installed".into())
    }

    fn registration_event_install_failure<E>(_source: E) -> SessionError {
        SessionError::InternalError("registration event handler installation failed".into())
    }
}

#[cfg(test)]
mod wildcard_contact_tests {
    use super::{classify_register_contacts, RegisterContactDisposition};
    use rvoip_sip_core::{parse_message, Message, Request};

    fn register_with(headers: &str) -> Request {
        let raw = format!(
            "REGISTER sip:example.test SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:alice@example.test>;tag=alice-tag\r\n\
             To: <sip:alice@example.test>\r\n\
             Call-ID: wildcard-contact-test\r\n\
             CSeq: 3 REGISTER\r\n\
             {}\
             Content-Length: 0\r\n\
             \r\n",
            headers
        );
        match parse_message(raw.as_bytes()).expect("parse REGISTER") {
            Message::Request(request) => request,
            Message::Response(_) => panic!("expected a request"),
        }
    }

    /// The form this exists for: log out of every device at once.
    #[test]
    fn a_lone_wildcard_with_expires_zero_removes_every_binding() {
        let request = register_with("Contact: *\r\nExpires: 0\r\n");
        assert_eq!(
            classify_register_contacts(&request),
            RegisterContactDisposition::RemoveAllBindings
        );
    }

    /// "If the request has additional Contact fields [...] the request is
    /// invalid, and the server MUST return a 400."
    #[test]
    fn a_wildcard_beside_another_contact_is_invalid() {
        let request = register_with(
            "Contact: *\r\n\
             Contact: <sip:alice@192.0.2.10:5060>\r\n\
             Expires: 0\r\n",
        );
        assert_eq!(
            classify_register_contacts(&request),
            RegisterContactDisposition::InvalidWildcard
        );
    }

    /// "[...] or an expiration time other than zero [...]"
    #[test]
    fn a_wildcard_with_a_nonzero_expiration_is_invalid() {
        let request = register_with("Contact: *\r\nExpires: 3600\r\n");
        assert_eq!(
            classify_register_contacts(&request),
            RegisterContactDisposition::InvalidWildcard
        );
    }

    /// A wildcard says nothing about how long, so an absent Expires cannot be
    /// read as zero. Treating it as a removal would delete bindings the peer
    /// never asked to drop.
    #[test]
    fn a_wildcard_without_an_expires_header_is_invalid() {
        let request = register_with("Contact: *\r\n");
        assert_eq!(
            classify_register_contacts(&request),
            RegisterContactDisposition::InvalidWildcard
        );
    }

    /// Ordinary registrations must be untouched by any of this, including the
    /// single-contact removal that already worked.
    #[test]
    fn ordinary_contacts_are_left_alone() {
        for headers in [
            "Contact: <sip:alice@192.0.2.10:5060>\r\n",
            "Contact: <sip:alice@192.0.2.10:5060>;expires=0\r\n",
            "Contact: <sip:alice@192.0.2.10:5060>\r\nExpires: 0\r\n",
            "Contact: <sip:alice@192.0.2.10:5060>\r\nContact: <sip:alice@10.0.0.5:5060>\r\n",
        ] {
            let request = register_with(headers);
            assert_eq!(
                classify_register_contacts(&request),
                RegisterContactDisposition::Ordinary,
                "{headers:?} carries no wildcard"
            );
        }
    }

    /// A REGISTER with no Contact at all is a query for current bindings
    /// (§10.3 step 6 skips to the last step), never a removal.
    #[test]
    fn a_register_without_any_contact_is_ordinary() {
        let request = register_with("");
        assert_eq!(
            classify_register_contacts(&request),
            RegisterContactDisposition::Ordinary
        );
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use rvoip_infra_common::events::coordinator::CrossCrateEventHandler;
    use rvoip_infra_common::events::{EventCoordinatorConfig, GlobalEventCoordinator};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const CANARY: &str = "peer-value\r\nX-Registration-Canary: exposed";

    #[derive(Debug)]
    struct MaliciousLowerError;

    impl std::fmt::Display for MaliciousLowerError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(CANARY)
        }
    }

    impl std::error::Error for MaliciousLowerError {}

    #[derive(Clone)]
    struct RegisterResponseCapture {
        deliveries: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl CrossCrateEventHandler for RegisterResponseCapture {
        async fn handle(&self, _event: Arc<dyn CrossCrateEvent>) -> anyhow::Result<()> {
            self.deliveries.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

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

    fn classified_error(
        disposition: rvoip_sip_dialog::FinalResponseCompletionDisposition,
    ) -> rvoip_sip_dialog::ExactResponseSendError {
        rvoip_sip_dialog::ExactResponseSendError {
            source: rvoip_sip_dialog::ApiError::Network {
                message: "scripted REGISTER transport outcome".to_string(),
            },
            disposition,
        }
    }

    fn registrar_test_contact(uri: &str, q_value: f32) -> ContactInfo {
        ContactInfo {
            uri: uri.to_string(),
            instance_id: "registration-adapter-test-device".to_string(),
            transport: Transport::UDP,
            user_agent: "rvoip-sip test".to_string(),
            expires: chrono::Utc::now() + chrono::Duration::hours(1),
            q_value,
            received: None,
            path: Vec::new(),
            methods: vec!["REGISTER".to_string()],
            reg_id: None,
            flow_id: None,
            reachability: ContactReachability::Unknown,
        }
    }

    #[test]
    fn successful_register_zero_wire_nacks_but_wire_unknown_is_terminal() {
        use rvoip_sip_dialog::FinalResponseCompletionDisposition as Disposition;

        assert!(classified_register_response_result(
            200,
            Err(classified_error(Disposition::ZeroWireRetryable)),
        )
        .is_err());
        assert!(classified_register_response_result(
            200,
            Err(classified_error(Disposition::WireUnknownErrorTerminal)),
        )
        .is_ok());
    }

    #[tokio::test]
    async fn zero_wire_preserves_existing_replacement_and_removal_state() {
        use rvoip_sip_dialog::FinalResponseCompletionDisposition as Disposition;

        let registrar = RegistrarService::new().await.unwrap();
        let aor = AddressOfRecord::parse("sip:zero-wire@example.test").unwrap();
        let uri = "sip:zero-wire@192.0.2.40:5060";
        registrar
            .register_aor(&aor, registrar_test_contact(uri, 0.5), Some(3600))
            .await
            .unwrap();

        let replacement = registrar
            .prepare_register_aor(&aor, registrar_test_contact(uri, 1.0), Some(3600))
            .await
            .unwrap();
        assert!(classified_register_response_result(
            200,
            Err(classified_error(Disposition::ZeroWireRetryable)),
        )
        .is_err());
        drop(replacement);
        assert_eq!(registrar.lookup_aor(&aor).await.unwrap()[0].q_value, 0.5);

        let removal = registrar
            .prepare_register_aor(&aor, registrar_test_contact(uri, 1.0), Some(0))
            .await
            .unwrap();
        assert!(classified_register_response_result(
            200,
            Err(classified_error(Disposition::ZeroWireRetryable)),
        )
        .is_err());
        drop(removal);
        assert_eq!(registrar.lookup_aor(&aor).await.unwrap().len(), 1);
        registrar.shutdown().await.unwrap();
    }

    #[test]
    fn register_challenge_zero_wire_nacks_but_wire_unknown_is_terminal() {
        use rvoip_sip_dialog::FinalResponseCompletionDisposition as Disposition;

        assert!(classified_register_response_result(
            401,
            Err(classified_error(Disposition::ZeroWireRetryable)),
        )
        .is_err());
        assert!(classified_register_response_result(
            401,
            Err(classified_error(Disposition::WireUnknownErrorTerminal)),
        )
        .is_ok());
        assert!(
            classified_register_response_result(401, Ok(Disposition::WrittenSuccessTerminal),)
                .is_ok()
        );
    }

    #[test]
    fn malformed_aor_error_does_not_echo_peer_input_or_parser_error() {
        let error = RegistrationAdapter::extract_aor(CANARY).expect_err("malformed AOR");
        assert_redacted(error, "invalid registration address-of-record");
    }

    #[test]
    fn registrar_and_coordination_errors_collapse_to_fixed_stage_classes() {
        assert_redacted(
            RegistrationAdapter::registrar_authentication_failure(MaliciousLowerError),
            "registrar authentication failed",
        );
        assert_redacted(
            RegistrationAdapter::registrar_storage_failure(MaliciousLowerError),
            "registrar storage failed",
        );
        assert_redacted(
            RegistrationAdapter::registration_response_send_failure(MaliciousLowerError),
            "registration response send failed",
        );
        assert_redacted(
            RegistrationAdapter::registration_response_owner_unavailable(),
            "registration response owner unavailable",
        );
        assert_redacted(
            RegistrationAdapter::registration_event_install_failure(MaliciousLowerError),
            "registration event handler installation failed",
        );
    }

    #[test]
    fn register_response_requires_one_direct_owner_and_has_no_bus_fallback() {
        let source = include_str!("registration_adapter.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("RegistrationAdapter production source");
        let helper = source
            .split("async fn send_register_response(")
            .nth(1)
            .and_then(|tail| tail.split("/// Handle incoming REGISTER request").next())
            .expect("REGISTER response helper source");
        assert!(helper.contains("registration_response_owner()?"));
        assert!(helper.contains("send_register_response_fields_classified(response)"));
        assert!(helper.contains("classified_register_response_result"));
        assert!(!helper.contains("dispatch_authoritative_handler"));
        assert!(!helper.contains(".publish("));
        assert!(!production.contains("SessionToDialogEvent::SendRegisterResponse"));
        assert!(!production.contains("RvoipCrossCrateEvent::SessionToDialog"));
    }

    #[test]
    fn successful_register_commits_prepared_binding_only_after_terminal_response() {
        let source = include_str!("registration_adapter.rs");
        let handler = source
            .split("async fn handle_incoming_register(")
            .nth(1)
            .and_then(|tail| tail.split("/// Handle dialog-to-session").next())
            .expect("REGISTER handler source");
        let prepare = handler
            .find(".prepare_register_aor(")
            .expect("prepared registrar mutation");
        let response = handler
            .find("self.send_register_response(&response).await?")
            .expect("classified REGISTER response");
        let commit = handler
            .find("prepared_registration.commit().await")
            .expect("prepared registrar commit");
        assert!(prepare < response && response < commit);
        assert!(!handler.contains(".register_aor(&aor, contact"));
    }

    #[test]
    fn start_is_install_only_with_no_subscription_or_background_loop() {
        let source = include_str!("registration_adapter.rs");
        let start = source
            .split("pub async fn start(self: Arc<Self>)")
            .nth(1)
            .and_then(|tail| tail.split("fn extract_aor").next())
            .expect("RegistrationAdapter::start source");
        assert!(start.contains("RegistrationAdapterInstall::new("));
        assert!(start.contains("dispatch_authoritative_handler"));
        let dispatch = start
            .find("dispatch_authoritative_handler")
            .expect("authoritative install dispatch");
        let owner_validation = start
            .rfind("self.registration_response_owner()?")
            .expect("post-install owner validation");
        assert!(dispatch < owner_validation);
        assert!(!start.contains(".subscribe("));
        assert!(!start.contains("tokio::spawn"));
        assert!(!start.contains("receiver"));
    }

    #[tokio::test]
    async fn public_constructor_without_authoritative_router_fails_installation() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let mut observer = coordinator.subscribe("dialog_to_session").await.unwrap();
        let subscriptions_before = coordinator.stats().await.active_subscriptions;
        let registrar = Arc::new(RegistrarService::new().await.unwrap());
        let adapter = Arc::new(RegistrationAdapter::new(
            Arc::clone(&registrar),
            Arc::clone(&coordinator),
        ));

        let error = adapter.start().await.expect_err("missing handler");
        assert!(matches!(
            error,
            SessionError::InternalError(ref detail)
                if detail == "registration event handler installation failed"
        ));
        assert_eq!(
            coordinator.stats().await.active_subscriptions,
            subscriptions_before
        );
        assert!(matches!(
            observer.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        registrar.shutdown().await.unwrap();
        coordinator.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn public_constructor_start_acquires_exact_response_owner_from_authoritative_router() {
        let coordinator = crate::api::unified::UnifiedCoordinator::new(
            crate::api::unified::Config::local("registration-public-constructor", 0),
        )
        .await
        .expect("start integrated coordinator");
        let registrar = Arc::new(RegistrarService::new().await.unwrap());
        let adapter = Arc::new(RegistrationAdapter::new(
            Arc::clone(&registrar),
            Arc::clone(&coordinator.global_coordinator),
        ));

        Arc::clone(&adapter)
            .start()
            .await
            .expect("authoritative router installs its exact dialog response owner");
        assert!(adapter.registration_response_owner().is_ok());

        registrar.shutdown().await.unwrap();
        coordinator
            .shutdown_gracefully(Some(std::time::Duration::ZERO))
            .await
            .unwrap();
        coordinator.global_coordinator.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn public_constructor_cannot_mutate_registrar_or_dispatch_response_bus_command() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(EventCoordinatorConfig::monolithic())
                .await
                .unwrap(),
        );
        let deliveries = Arc::new(AtomicUsize::new(0));
        coordinator
            .register_handler(
                "session_to_dialog",
                RegisterResponseCapture {
                    deliveries: Arc::clone(&deliveries),
                },
            )
            .await
            .unwrap();
        let mut observer = coordinator.subscribe("dialog_to_session").await.unwrap();
        let subscriptions_before = coordinator.stats().await.active_subscriptions;

        let registrar = Arc::new(
            RegistrarService::with_auth(
                rvoip_sip_registrar::api::ServiceMode::P2P,
                rvoip_sip_registrar::types::RegistrarConfig::default(),
                "registrar.example",
            )
            .await
            .unwrap(),
        );
        registrar
            .user_store()
            .unwrap()
            .add_user("alice", "correct horse")
            .unwrap();
        let request_uri = "sip:registrar.example";
        let aor = "sip:alice@identity.example";
        let (_, challenge) = registrar
            .authenticate_register_request("alice", None, "REGISTER", request_uri, aor)
            .await
            .unwrap();
        let challenge = rvoip_auth_core::DigestAuthenticator::parse_challenge(
            &challenge.expect("REGISTER challenge"),
        )
        .unwrap();
        let computed = rvoip_auth_core::DigestClient::compute_response_with_state(
            "alice",
            "correct horse",
            &challenge,
            "REGISTER",
            request_uri,
            1,
            None,
        )
        .unwrap();
        let authorization = rvoip_auth_core::DigestClient::format_authorization_with_state(
            "alice",
            &challenge,
            request_uri,
            &computed,
        );
        let raw = bytes::Bytes::from(format!(
            "REGISTER {request_uri} SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-register-direct\r\n\
             Max-Forwards: 70\r\n\
             From: <{aor}>;tag=alice-tag\r\n\
             To: <{aor}>\r\n\
             Call-ID: register-direct@example.invalid\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:alice@127.0.0.1:5060>\r\n\
             Content-Length: 0\r\n\r\n"
        ));
        let event = DialogToSessionEvent::IncomingRegister {
            transaction_id: "z9hG4bK-register-direct:REGISTER:server".to_string(),
            from_uri: aor.to_string(),
            to_uri: aor.to_string(),
            contact_uri: "sip:alice@127.0.0.1:5060".to_string(),
            expires: 300,
            authorization: Some(authorization),
            call_id: "register-direct@example.invalid".to_string(),
            raw_request: Some(raw),
            transport: None,
        };
        let adapter = RegistrationAdapter::new(Arc::clone(&registrar), Arc::clone(&coordinator));

        let error = adapter
            .handle_incoming_register_event(&event)
            .await
            .expect_err("public constructor has no exact response owner");

        assert!(matches!(
            error,
            SessionError::InternalError(ref detail)
                if detail == "registration response owner unavailable"
        ));
        assert_eq!(deliveries.load(Ordering::SeqCst), 0);
        let parsed_aor = AddressOfRecord::parse(aor).unwrap();
        match registrar.lookup_aor(&parsed_aor).await {
            Ok(contacts) => assert!(contacts.is_empty()),
            Err(error) => assert_eq!(error.diagnostic_class(), "user-not-found"),
        }
        assert_eq!(
            coordinator.stats().await.active_subscriptions,
            subscriptions_before
        );
        assert!(matches!(
            observer.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
        registrar.shutdown().await.unwrap();
        coordinator.shutdown().await.unwrap();
    }
}
