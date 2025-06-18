//! CallHandler implementation for the call center
//!
//! This module provides the CallHandler trait implementation that integrates
//! with session-core to receive and process incoming calls.

use std::sync::Weak;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};
use rvoip_session_core::{CallHandler, IncomingCall, CallDecision, CallSession};
use std::time::Instant;
use rvoip_sip_core::Contact;

use super::core::CallCenterEngine;
use crate::error::CallCenterError;

/// CallHandler implementation for the call center
#[derive(Clone, Debug)]
pub struct CallCenterCallHandler {
    pub(super) engine: Weak<CallCenterEngine>,
}

#[async_trait]
impl CallHandler for CallCenterCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        debug!("CallCenterCallHandler: Received incoming call {}", call.id);
        
        // Try to get a strong reference to the engine
        if let Some(engine) = self.engine.upgrade() {
            // Process the incoming call through the call center's routing logic
            match engine.process_incoming_call(call).await {
                Ok(decision) => decision,
                Err(e) => {
                    error!("Failed to process incoming call: {}", e);
                    CallDecision::Reject("Call center processing error".to_string())
                }
            }
        } else {
            warn!("Call center engine has been dropped");
            CallDecision::Reject("Call center not available".to_string())
        }
    }
    
    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        info!("CallCenterCallHandler: Call {} ended: {}", call.id, reason);
        
        if let Some(engine) = self.engine.upgrade() {
            if let Err(e) = engine.handle_call_termination(call.id).await {
                error!("Failed to handle call termination: {}", e);
            }
        }
    }
    
    async fn on_call_established(&self, call: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        info!("CallCenterCallHandler: Call {} established", call.id);
        debug!("Local SDP available: {}, Remote SDP available: {}", 
               local_sdp.is_some(), remote_sdp.is_some());
        
        // Update call state to active/bridged
        if let Some(engine) = self.engine.upgrade() {
            engine.update_call_established(call.id).await;
        }
    }
}

impl CallCenterEngine {
    /// Handle SIP REGISTER request forwarded from session-core
    /// This is called when dialog-core receives a REGISTER and forwards it to us
    pub async fn handle_register_request(
        &self,
        from_uri: String,
        contact_uri: String,
        expires: u32,
    ) -> Result<(), CallCenterError> {
        tracing::info!("Processing REGISTER: from={}, contact={}, expires={}", 
                      from_uri, contact_uri, expires);
        
        // Parse the AOR (Address of Record) from the from_uri
        let aor = from_uri; // In practice, might need to normalize this
        
        // Create a Contact header structure for the registrar
        // Note: This is simplified - in practice you'd parse the full Contact header
        let contact = match self.create_contact_from_uri(&contact_uri) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to parse contact URI {}: {}", contact_uri, e);
                return Err(CallCenterError::InvalidInput(
                    format!("Invalid contact URI: {}", e)
                ));
            }
        };
        
        // Process the registration with our SIP registrar
        let mut registrar = self.sip_registrar.lock().await;
        let response = registrar.process_register(
            &aor,
            &contact,
            Some(expires),
            None, // User-Agent would come from SIP headers
            "unknown".to_string(), // Remote address would come from transport layer
        )?;
        
        tracing::info!("REGISTER processed: {:?} for {}", response.status, aor);
        
        // TODO: Send appropriate SIP response back through session-core
        // For now, session-core will auto-respond if we don't send anything
        
        Ok(())
    }
    
    /// Helper to create a Contact from a URI string
    fn create_contact_from_uri(&self, uri_str: &str) -> Result<Contact, CallCenterError> {
        use rvoip_sip_core::{Uri, Address};
        use rvoip_sip_core::prelude::ContactParamInfo;
        
        let uri: Uri = uri_str.parse()
            .map_err(|e| CallCenterError::InvalidInput(
                format!("Failed to parse URI: {}", e)
            ))?;
        
        let address = Address::new(uri);
        let contact_info = ContactParamInfo { address };
        Ok(Contact::new_params(vec![contact_info]))
    }
} 