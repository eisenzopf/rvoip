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
use std::sync::Arc;
use tracing::{debug, warn};

use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use crate::manager::{DialogManager, SessionCoordinator};
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
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for REGISTER: {}", e),
            })?;

        let transaction_id = server_transaction.id().clone();

        // **NEW**: Check unified configuration for auto-response behavior
        // If the manager is configured for auto-REGISTER response, send immediate response
        // Otherwise, forward to session layer for application handling via global event bus
        if self.should_auto_respond_to_register() {
            debug!("Auto-responding to REGISTER request (configured for auto-response)");
            self.send_basic_register_response(&transaction_id, &request, expires)
                .await?;
        } else {
            debug!("Forwarding REGISTER request to session layer via global event bus");

            // Extract Authorization header if present
            use rvoip_sip_core::types::headers::HeaderAccess;
            let authorization =
                request.raw_header_value(&rvoip_sip_core::types::header::HeaderName::Authorization);

            // Publish IncomingRegister event to global event bus
            use rvoip_infra_common::events::cross_crate::{
                DialogToSessionEvent, RvoipCrossCrateEvent,
            };
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
                });

            // Publish via event hub (global event bus)
            if let Some(hub) = self.event_hub.read().await.as_ref() {
                if let Err(e) = hub.publish_cross_crate_event(event).await {
                    warn!("Failed to publish IncomingRegister event: {}", e);
                    // Fallback to basic response
                    self.send_basic_register_response(&transaction_id, &request, expires)
                        .await?;
                } else {
                    debug!("✅ Published IncomingRegister event to global bus");
                }
            } else {
                debug!("No event hub - falling back to basic 200 OK");
                self.send_basic_register_response(&transaction_id, &request, expires)
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

    /// Send basic REGISTER response (for auto-response mode)
    pub async fn send_basic_register_response(
        &self,
        transaction_id: &crate::transaction::TransactionKey,
        request: &Request,
        expires: u32,
    ) -> DialogResult<()> {
        use rvoip_sip_core::StatusCode;

        // Create basic 200 OK response for REGISTER
        let response =
            crate::transaction::utils::response_builders::create_response(request, StatusCode::Ok);

        // TODO: Could add Contact header with the registered URI and expires
        // For basic auto-response, just send 200 OK

        self.transaction_manager
            .send_response(transaction_id, response)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send REGISTER response: {}", e),
            })?;

        debug!("Sent basic REGISTER response with expires {}", expires);
        Ok(())
    }

    /// Send REGISTER response based on event from session-core
    ///
    /// This is called when session-core publishes a SendRegisterResponse event
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
        use rvoip_sip_core::types::header::HeaderName;
        use rvoip_sip_core::types::headers::header_value::HeaderValue;
        use rvoip_sip_core::{ResponseBuilder, StatusCode, TypedHeader};

        debug!("Sending REGISTER response: {} {}", status_code, reason);

        // Get the original request from the transaction
        // The transaction manager stores the original request
        let request = self
            .transaction_manager
            .original_request(transaction_id)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to get request for transaction: {}", e),
            })?
            .ok_or_else(|| DialogError::TransactionError {
                message: "No request found for transaction".into(),
            })?;

        // Verify the request has CSeq header
        use rvoip_sip_core::types::headers::HeaderAccess;
        if let Some(cseq) = request.typed_header::<rvoip_sip_core::types::cseq::CSeq>() {
            debug!(
                "Original request has CSeq: {} {}",
                cseq.sequence(),
                cseq.method()
            );
        } else {
            warn!("⚠️ Original request is missing CSeq header!");
        }

        // Parse status code
        let status = StatusCode::from_u16(status_code).map_err(|e| {
            DialogError::protocol_error(&format!("Invalid status code {}: {}", status_code, e))
        })?;

        // Build response using response builder
        let response = if status_code == 401 {
            // Build 401 Unauthorized with WWW-Authenticate header
            let mut resp =
                crate::transaction::utils::response_builders::create_response(&request, status);

            if let Some(www_auth) = www_authenticate {
                // Add WWW-Authenticate header as raw header
                resp.headers.push(TypedHeader::Other(
                    HeaderName::WwwAuthenticate,
                    HeaderValue::Raw(www_auth.as_bytes().to_vec()),
                ));
            }

            resp
        } else if status_code == 200 {
            // Build 200 OK with Contact and Expires headers
            let mut resp =
                crate::transaction::utils::response_builders::create_response(&request, status);

            // Add Contact header if provided
            if let Some(contact_uri) = contact {
                // Copy Contact header from request or use provided value
                if let Some(contact_header) = request.header(&HeaderName::Contact) {
                    resp.headers.push(contact_header.clone());
                }
            }

            // Add Expires header if provided
            if let Some(exp) = expires {
                resp.headers.push(TypedHeader::Expires(
                    rvoip_sip_core::types::expires::Expires::new(exp),
                ));
            }

            resp
        } else {
            // Build generic response
            crate::transaction::utils::response_builders::create_response(&request, status)
        };

        // Send response via transaction manager
        self.transaction_manager
            .send_response(transaction_id, response)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send REGISTER response: {}", e),
            })?;

        debug!("Sent REGISTER response: {} {}", status_code, reason);
        Ok(())
    }
}
