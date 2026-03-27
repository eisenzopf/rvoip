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
use tracing::debug;

use rvoip_sip_core::Request;
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use crate::manager::{DialogManager, SessionCoordinator};

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
    async fn handle_register_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing REGISTER request from {}", source);
        
        // Extract registration information
        let from_uri = request.from()
            .ok_or_else(|| DialogError::protocol_error("REGISTER missing From header"))?
            .uri().clone();
        
        let contact_uri = self.extract_contact_uri(&request).unwrap_or_else(|| from_uri.clone());
        let expires = self.extract_expires(&request);

        // Remember the registration route for INVITE forwarding
        self.transaction_manager
            .remember_registration_route(&from_uri, source, expires)
            .await;

        // Create server transaction
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for REGISTER: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();

        if let Some(auth) = self.auth_provider() {
            match auth.check_request(&request, source).await {
                crate::auth::AuthResult::Authenticated { username } => {
                    debug!("REGISTER authenticated for user {}", username);
                }
                crate::auth::AuthResult::Challenge => {
                    debug!("Sending 401 challenge for REGISTER from {}", source);
                    let nonce = crate::auth::generate_nonce();
                    let mut response = crate::transaction::utils::response_builders::create_unauthorized_response(
                        &request, auth.realm(), &nonce,
                    );
                    crate::transaction::utils::response_builders::fix_via_nat(&mut response, source);
                    self.transaction_manager.send_response(&transaction_id, response).await
                        .map_err(|e| DialogError::TransactionError {
                            message: format!("Failed to send 401 for REGISTER: {}", e),
                        })?;
                    return Ok(());
                }
                crate::auth::AuthResult::Skip => {
                    debug!("Auth skipped for REGISTER from {}", source);
                }
            }
        }

        // **NEW**: Check unified configuration for auto-response behavior
        // If the manager is configured for auto-REGISTER response, send immediate response
        // Otherwise, forward to session layer for application handling
        if self.should_auto_respond_to_register() {
            debug!("Auto-responding to REGISTER request (configured for auto-response)");
            self.send_basic_register_response(&transaction_id, &request, expires, source).await?;
        } else {
            debug!("Forwarding REGISTER request to session layer (auto-response disabled)");

            let event = SessionCoordinationEvent::RegistrationRequest {
                transaction_id: transaction_id.clone(),
                from_uri,
                contact_uri,
                expires,
            };

            if let Err(e) = self.notify_session_layer(event).await {
                debug!("Failed to notify session layer of REGISTER: {}, sending fallback response", e);

                // Fallback: send basic 200 OK response
                self.send_basic_register_response(&transaction_id, &request, expires, source).await?;
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
        request.typed_header::<rvoip_sip_core::types::contact::Contact>()
            .and_then(|contact| contact.0.first())
            .and_then(|contact_val| {
                match contact_val {
                    rvoip_sip_core::types::contact::ContactValue::Params(params) => {
                        params.first().map(|p| p.address.uri.clone())
                    },
                    _ => None,
                }
            })
    }
    
    /// Extract Expires value from request
    pub fn extract_expires(&self, request: &Request) -> u32 {
        request.typed_header::<rvoip_sip_core::types::expires::Expires>()
            .map(|exp| exp.0)
            .unwrap_or(3600) // Default to 1 hour
    }
    
    /// Send basic REGISTER response (for auto-response mode)
    pub async fn send_basic_register_response(
        &self,
        transaction_id: &crate::transaction::TransactionKey,
        request: &Request,
        expires: u32,
        source: std::net::SocketAddr,
    ) -> DialogResult<()> {
        use rvoip_sip_core::StatusCode;

        // Create 200 OK response for REGISTER with Contact + Expires headers
        let mut response = crate::transaction::utils::response_builders::create_response(request, StatusCode::Ok);

        // RFC 3261 §18.2.2 / RFC 3581: fix Via header for NAT traversal
        crate::transaction::utils::response_builders::fix_via_nat(&mut response, source);

        // Copy the Contact header from the request so SIP.js recognizes the registration
        for h in &request.headers {
            if matches!(h, rvoip_sip_core::TypedHeader::Contact(_)) {
                response.headers.push(h.clone());
            }
        }
        // Add Expires header
        response.headers.push(rvoip_sip_core::TypedHeader::Expires(
            rvoip_sip_core::types::expires::Expires(expires),
        ));

        self.transaction_manager.send_response(transaction_id, response).await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send REGISTER response: {}", e),
            })?;

        debug!("Sent basic REGISTER response with expires {}", expires);
        Ok(())
    }
} 