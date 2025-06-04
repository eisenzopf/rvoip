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
    /// Handle REGISTER requests according to RFC 3261 Section 10
    /// 
    /// REGISTER requests don't create dialogs but are handled for completeness.
    async fn handle_register_method(&self, request: Request, source: SocketAddr) -> DialogResult<()> {
        debug!("Processing REGISTER request from {}", source);
        
        // Extract registration information
        let from_uri = request.from()
            .ok_or_else(|| DialogError::protocol_error("REGISTER missing From header"))?
            .uri().clone();
        
        let contact_uri = self.extract_contact_uri(&request).unwrap_or_else(|| from_uri.clone());
        let expires = self.extract_expires(&request);
        
        // Create server transaction and send coordination event
        let server_transaction = self.transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for REGISTER: {}", e),
            })?;
        
        let transaction_id = server_transaction.id().clone();
        
        let event = SessionCoordinationEvent::RegistrationRequest {
            transaction_id,
            from_uri,
            contact_uri,
            expires,
        };
        
        self.notify_session_layer(event).await?;
        debug!("REGISTER request forwarded to session layer");
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
} 