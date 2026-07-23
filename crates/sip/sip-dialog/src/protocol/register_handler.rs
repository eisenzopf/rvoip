//! REGISTER Request Handler for Dialog-Core
//!
//! This module handles REGISTER requests according to RFC 3261 Section 10.
//! REGISTER requests are used for SIP endpoint registration and location services.
//! Note that REGISTER requests do not create dialogs but are processed for completeness.
//!
//! ## Registration Processing
//!
//! - **Contact Registration**: Register endpoint locations
//! - **Expires Handling**: Process registration lifetimes
//! - **De-registration**: Handle contact removal (Expires: 0)
//! - **Authentication**: Support authentication challenges
//! - **Forwarding**: Route to session layer for actual registration logic
//!
//! ## Key Features
//!
//! - Extracts Contact URI and Expires values
//! - Forwards to session-core for location service handling
//! - Supports both registration and de-registration
//! - Maintains proper SIP transaction handling

use std::net::SocketAddr;
use tracing::{debug, warn};

use crate::errors::{DialogError, DialogResult};
use crate::manager::DialogManager;
use rvoip_sip_core::Request;

/// REGISTER-specific handling operations
pub trait RegisterHandler {
    /// Handle REGISTER requests (non-dialog)
    fn handle_register_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of REGISTER handling for DialogManager
impl RegisterHandler for DialogManager {
    /// Handle REGISTER requests according to RFC 3261 Section 10 with unified configuration support
    ///
    /// REGISTER requests don't create dialogs but are handled for completeness.
    /// Supports auto-response behavior based on unified configuration.
    async fn handle_register_method(
        &self,
        request: Request,
        source: SocketAddr,
    ) -> DialogResult<()> {
        debug!("Processing REGISTER request from {}", source);

        // Extract registration information
        let from_uri = request
            .from()
            .ok_or_else(|| DialogError::protocol_error("REGISTER missing From header"))?
            .uri()
            .clone();

        let contact_uri = self
            .extract_contact_uri(&request)
            .unwrap_or_else(|| from_uri.clone());
        let expires = self.extract_expires(&request);

        // Create server transaction
        let server_transaction = self
            .transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|_error| DialogError::TransactionError {
                message: "Failed to create server transaction for REGISTER".to_string(),
            })?;

        let transaction_id = server_transaction.id().clone();

        // A REGISTER 200 response is a registrar commitment: it must represent
        // a binding owned by the session/registrar layer. Dialog-core has no
        // location-service store and therefore cannot fabricate that success.
        if self.should_auto_respond_to_register() {
            warn!(
                "Legacy auto-REGISTER mode has no registrar owner; rejecting instead of fabricating a binding"
            );
            self.send_register_response_classified_terminal(
                &transaction_id,
                501,
                "Not Implemented",
                None,
                None,
                None,
            )
            .await?;
        } else {
            debug!("Delivering REGISTER request to the authoritative registrar handler");

            // Extract Authorization header if present
            use rvoip_sip_core::types::headers::HeaderAccess;
            let authorization =
                request.raw_header_value(&rvoip_sip_core::types::header::HeaderName::Authorization);

            // Publish IncomingRegister event to global event bus
            use rvoip_infra_common::events::cross_crate::{
                DialogToSessionEvent, RvoipCrossCrateEvent,
            };
            // SIP_API_DESIGN_2 §7.5: surface the original wire bytes
            // the transport cached so STIR/SHAKEN and signature-
            // preserving consumers see the upstream form unchanged.
            // Fall back to re-serialising for synthetic events / mock
            // transports that publish `raw_bytes: None`.
            let raw_request = self
                .transaction_manager
                .take_inbound_bytes(&transaction_id)
                .or_else(|| Some(bytes::Bytes::from(request.to_bytes())));
            let transport = self
                .transaction_manager
                .take_inbound_transport(&transaction_id);
            let event =
                RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::IncomingRegister {
                    transaction_id: transaction_id.to_string(),
                    from_uri: from_uri.to_string(),
                    to_uri: from_uri.to_string(), // To same as From for self-registration
                    contact_uri: contact_uri.to_string(),
                    expires,
                    authorization,
                    call_id: request
                        .call_id()
                        .map(|cid| cid.value().to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                    raw_request,
                    transport,
                });

            // Deliver through the acknowledged causal handler. Observational
            // subscribers are never a registrar and cannot authorize success.
            let registrar_accepted = if let Some(hub) = self.event_hub.read().await.as_ref() {
                match hub.try_publish_cross_crate_event(event).await {
                    Ok(true) => {
                        debug!("Delivered IncomingRegister to the authoritative registrar");
                        true
                    }
                    Ok(false) => {
                        warn!("No authoritative registrar handler accepted IncomingRegister");
                        false
                    }
                    Err(error) => {
                        warn!(
                            error = %crate::transaction::safe_diagnostics::SafeOpaqueError::new(&error),
                            "Authoritative registrar handler failed before accepting REGISTER"
                        );
                        false
                    }
                }
            } else {
                warn!("No authoritative registrar route is installed");
                false
            };

            // Once a server transaction exists, every REGISTER routing failure
            // needs an exact final response. Returning only an internal routing
            // error would strand the peer until its transaction timed out.
            if !registrar_accepted {
                self.send_register_response_classified_terminal(
                    &transaction_id,
                    503,
                    "Service Unavailable",
                    None,
                    None,
                    None,
                )
                .await?;
            }
        }

        debug!("REGISTER request processed");
        Ok(())
    }
}

/// REGISTER-specific helper methods for DialogManager
impl DialogManager {
    /// Extract Contact URI from request
    pub fn extract_contact_uri(&self, request: &Request) -> Option<rvoip_sip_core::Uri> {
        request
            .typed_header::<rvoip_sip_core::types::contact::Contact>()
            .and_then(|contact| contact.0.first())
            .and_then(|contact_val| match contact_val {
                rvoip_sip_core::types::contact::ContactValue::Params(params) => {
                    params.first().map(|p| p.address.uri.clone())
                }
                _ => None,
            })
    }

    /// Extract Expires value from request
    pub fn extract_expires(&self, request: &Request) -> u32 {
        request
            .typed_header::<rvoip_sip_core::types::expires::Expires>()
            .map(|exp| exp.0)
            .unwrap_or(3600) // Default to 1 hour
    }

    /// Compatibility facade for the retired auto-registrar mode.
    ///
    /// Dialog-core does not own a location-service binding, so this method
    /// preserves its signature but sends an honest 501 response rather than a
    /// fabricated 200 registration success.
    pub async fn send_basic_register_response(
        &self,
        transaction_id: &crate::transaction::TransactionKey,
        _request: &Request,
        _expires: u32,
    ) -> DialogResult<()> {
        self.send_register_response(transaction_id, 501, "Not Implemented", None, None, None)
            .await
    }

    /// Send REGISTER response based on event from session-core
    ///
    /// This is called directly by the registrar/dialog response owner.
    /// after processing authentication
    pub async fn send_register_response(
        &self,
        transaction_id: &crate::transaction::TransactionKey,
        status_code: u16,
        reason: &str,
        www_authenticate: Option<&str>,
        contact: Option<&str>,
        expires: Option<u32>,
    ) -> DialogResult<()> {
        self.send_register_response_with_extras(
            transaction_id,
            status_code,
            reason,
            www_authenticate,
            contact,
            expires,
            None,
            &[],
            false,
            &[],
            &[],
        )
        .await
    }

    /// Author one exact fallback response and preserve transaction-core's
    /// first-write classification. A wire-unknown error is terminal because a
    /// second final response could duplicate bytes already accepted by the
    /// transport; only a proven zero-wire result returns an error.
    async fn send_register_response_classified_terminal(
        &self,
        transaction_id: &crate::transaction::TransactionKey,
        status_code: u16,
        reason: &str,
        www_authenticate: Option<&str>,
        contact: Option<&str>,
        expires: Option<u32>,
    ) -> DialogResult<crate::transaction::server::FinalResponseCompletionDisposition> {
        use crate::transaction::server::FinalResponseCompletionDisposition as Disposition;

        match self
            .send_register_response(
                transaction_id,
                status_code,
                reason,
                www_authenticate,
                contact,
                expires,
            )
            .await
        {
            Ok(()) => Ok(Disposition::WrittenSuccessTerminal),
            Err(error) => {
                match self
                    .transaction_manager
                    .classify_final_response_completion(transaction_id)
                    .await
                {
                    Disposition::ZeroWireRetryable => Err(error),
                    disposition @ (Disposition::WrittenSuccessTerminal
                    | Disposition::WireUnknownErrorTerminal) => {
                        warn!(
                            status_code,
                            ?disposition,
                            "REGISTER fallback became terminal at the transport boundary; suppressing duplicate response"
                        );
                        Ok(disposition)
                    }
                }
            }
        }
    }

    /// SIP_API_DESIGN_2 Phase D — registrar response with full
    /// RFC 3327 / 3608 / 3455 field set plus generic
    /// application-staged extras.
    #[allow(clippy::too_many_arguments)]
    pub async fn send_register_response_with_extras(
        &self,
        transaction_id: &crate::transaction::TransactionKey,
        status_code: u16,
        reason: &str,
        www_authenticate: Option<&str>,
        contact: Option<&str>,
        expires: Option<u32>,
        min_expires: Option<u32>,
        service_route: &[String],
        path_echo: bool,
        associated_uri: &[String],
        extra_headers: &[(String, String)],
    ) -> DialogResult<()> {
        let response = self
            .build_register_response_with_extras(
                transaction_id,
                status_code,
                reason,
                www_authenticate,
                contact,
                expires,
                min_expires,
                service_route,
                path_echo,
                associated_uri,
                extra_headers,
            )
            .await?;
        self.transaction_manager
            .send_response(transaction_id, response)
            .await
            .map_err(|_error| DialogError::TransactionError {
                message: "Failed to send REGISTER response".to_string(),
            })?;

        debug!(
            "Sent REGISTER response (extras): {} reason_present={}",
            status_code,
            !reason.is_empty()
        );
        Ok(())
    }

    /// Materialize the one RFC-compliant REGISTER response snapshot without
    /// selecting a response-completion policy. Both compatibility sends and
    /// the classified exact primitive use this builder.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn build_register_response_with_extras(
        &self,
        transaction_id: &crate::transaction::TransactionKey,
        status_code: u16,
        reason: &str,
        www_authenticate: Option<&str>,
        contact: Option<&str>,
        expires: Option<u32>,
        min_expires: Option<u32>,
        service_route: &[String],
        path_echo: bool,
        associated_uri: &[String],
        extra_headers: &[(String, String)],
    ) -> DialogResult<rvoip_sip_core::Response> {
        use rvoip_sip_core::types::header::HeaderName;
        use rvoip_sip_core::types::headers::header_value::HeaderValue;
        use rvoip_sip_core::{StatusCode, TypedHeader};

        if status_code == 423 && min_expires.is_none() {
            return Err(DialogError::protocol_error(
                "423 REGISTER response requires Min-Expires",
            ));
        }
        if min_expires.is_some() && status_code != 423 {
            return Err(DialogError::protocol_error(
                "Min-Expires is only valid on a 423 REGISTER response",
            ));
        }
        if matches!(status_code, 401 | 407) && www_authenticate.is_none() {
            return Err(DialogError::protocol_error(
                "401/407 REGISTER response requires an authentication challenge",
            ));
        }
        let success = (200..300).contains(&status_code);
        if !success
            && (contact.is_some()
                || expires.is_some()
                || !service_route.is_empty()
                || path_echo
                || !associated_uri.is_empty())
        {
            return Err(DialogError::protocol_error(
                "REGISTER binding headers are only valid on a successful response",
            ));
        }

        debug!(
            "Sending REGISTER response (extras): {} reason_present={} (service_route={}, path_echo={}, associated_uri={}, extras={})",
            status_code,
            !reason.is_empty(),
            service_route.len(),
            path_echo,
            associated_uri.len(),
            extra_headers.len()
        );

        // Resolve the inbound request to template the response.
        let request = self
            .transaction_manager
            .original_request(transaction_id)
            .await
            .map_err(|_error| DialogError::TransactionError {
                message: "Failed to get request for transaction".to_string(),
            })?
            .ok_or_else(|| DialogError::TransactionError {
                message: "No request found for transaction".into(),
            })?;

        let status = StatusCode::from_u16(status_code).map_err(|_error| {
            DialogError::protocol_error(&format!("Invalid status code {}", status_code))
        })?;

        let mut response =
            crate::transaction::utils::response_builders::create_response(&request, status);

        // RFC 3261 §20.23 — Min-Expires lives on 423 Interval Too Brief.
        if let Some(min) = min_expires {
            response.headers.push(TypedHeader::MinExpires(
                rvoip_sip_core::types::min_expires::MinExpires::new(min),
            ));
        }

        // RFC 3261 §10.3: a successful registrar response lists the current
        // binding. Use the registrar-supplied binding when present; otherwise
        // preserve the inbound Contact for the common single-binding case.
        if success {
            if let Some(contact_uri) = contact {
                response.headers.push(TypedHeader::Other(
                    HeaderName::Contact,
                    HeaderValue::Raw(contact_uri.as_bytes().to_vec()),
                ));
            } else if let Some(contact_header) = request.header(&HeaderName::Contact) {
                response.headers.push(contact_header.clone());
            }
        }
        if let Some(exp) = expires {
            response.headers.push(TypedHeader::Expires(
                rvoip_sip_core::types::expires::Expires::new(exp),
            ));
        }

        // RFC 3261 §22: 401 is end-to-end WWW authentication; 407 is
        // hop-by-hop proxy authentication.
        if status_code == 401 || status_code == 407 {
            if let Some(www_auth) = www_authenticate {
                response.headers.push(TypedHeader::Other(
                    if status_code == 407 {
                        HeaderName::ProxyAuthenticate
                    } else {
                        HeaderName::WwwAuthenticate
                    },
                    HeaderValue::Raw(www_auth.as_bytes().to_vec()),
                ));
            }
        }

        // RFC 3608 Service-Route — each entry rendered as `<uri>` and
        // concatenated comma-separated. Stored as a `Other` raw header
        // until sip-core grows a typed `ServiceRoute`.
        if !service_route.is_empty() {
            let rendered = service_route
                .iter()
                .map(|u| format!("<{}>", u))
                .collect::<Vec<_>>()
                .join(", ");
            response.headers.push(TypedHeader::Other(
                HeaderName::Other("Service-Route".to_string()),
                HeaderValue::Raw(rendered.into_bytes()),
            ));
        }

        // RFC 3327 Path echo — copy every inbound `Path:` header onto
        // the 2xx so subsequent re-targeted requests reach the UA via
        // the same waypoints.
        if path_echo {
            for hdr in request.headers.iter() {
                if matches!(
                    hdr,
                    TypedHeader::Other(HeaderName::Other(name), _)
                        if name.eq_ignore_ascii_case("Path")
                ) {
                    response.headers.push(hdr.clone());
                }
            }
        }

        // RFC 3455 P-Associated-URI — each AOR rendered as `<uri>` and
        // concatenated.
        if !associated_uri.is_empty() {
            let rendered = associated_uri
                .iter()
                .map(|u| format!("<{}>", u))
                .collect::<Vec<_>>()
                .join(", ");
            response.headers.push(TypedHeader::Other(
                HeaderName::Other("P-Associated-URI".to_string()),
                HeaderValue::Raw(rendered.into_bytes()),
            ));
        }

        // Generic application-staged extras — `(name, value)` wire
        // tuples. The receiving side reconstructs the typed header
        // here; infra-common stays SIP-agnostic.
        for (name, value) in extra_headers {
            let header_name = match name.parse::<HeaderName>() {
                Ok(n) => n,
                Err(_) => HeaderName::Other(name.clone()),
            };
            response.headers.push(TypedHeader::Other(
                header_name,
                HeaderValue::Raw(value.as_bytes().to_vec()),
            ));
        }

        Ok(response)
    }
}

#[cfg(test)]
mod single_register_authority_tests {
    #[test]
    fn register_has_one_response_materializer_and_no_false_success_fallback() {
        let source = include_str!("register_handler.rs");
        let wire_send = [".send_response(transaction_id,", " response)"].concat();
        assert_eq!(
            source.matches(&wire_send).count(),
            1,
            "REGISTER responses must have one transaction wire materializer"
        );

        let basic = source
            .split("pub async fn send_basic_register_response(")
            .nth(1)
            .and_then(|tail| tail.split("pub async fn send_register_response(").next())
            .expect("legacy auto-REGISTER facade");
        assert!(basic.contains("501"));
        assert!(basic.contains("send_register_response("));
        assert!(!basic.contains("create_response("));

        let false_success = ["falling back to basic ", "200 OK"].concat();
        assert!(!source.contains(&false_success));
        assert!(source.contains("try_publish_cross_crate_event(event)"));
        assert!(source.contains("if !registrar_accepted"));
        assert!(source.contains("send_register_response_classified_terminal"));
        assert!(source.contains("Disposition::ZeroWireRetryable => Err(error)"));
        assert!(source.contains("Disposition::WireUnknownErrorTerminal"));
        assert!(source.contains("Authoritative registrar handler failed before accepting REGISTER"));
        assert!(source.contains("HeaderName::ProxyAuthenticate"));
        assert!(source.contains("HeaderName::Contact"));
    }
}
