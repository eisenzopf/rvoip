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

use std::sync::Arc;
use tracing::{info, warn, debug};
use rvoip_infra_common::events::{
    coordinator::GlobalEventCoordinator,
    cross_crate::{RvoipCrossCrateEvent, DialogToSessionEvent, SessionToDialogEvent},
};
use rvoip_registrar_core::{RegistrarService, ContactInfo, Transport};
use crate::errors::{Result, SessionError};

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
        info!("🔐 Handling incoming REGISTER from {}", from_uri);
        
        // Extract username from URI (e.g., "sip:alice@127.0.0.1" → "alice")
        let username = Self::extract_username(&from_uri)?;
        debug!("Extracted username: {}", username);
        
        // Call registrar-core to authenticate
        let (should_register, www_auth_challenge) = self.registrar
            .authenticate_register(
                &username,
                authorization.as_deref(),
                "REGISTER",
                &from_uri,
            )
            .await
            .map_err(|e| SessionError::RegistrationFailed(e.to_string()))?;
        
        if should_register {
            // Valid credentials - register user
            info!("✅ Authentication successful for {}", username);
            
            // Build ContactInfo
            let contact = ContactInfo {
                uri: contact_uri.clone(),
                instance_id: uuid::Uuid::new_v4().to_string(),
                transport: Transport::UDP,
                user_agent: "rvoip-session-core".to_string(),
                expires: chrono::Utc::now() + chrono::Duration::try_seconds(expires as i64)
                    .unwrap_or_else(|| chrono::Duration::seconds(3600)),
                q_value: 1.0,
                received: None,
                path: Vec::new(),
                methods: vec!["INVITE".to_string(), "ACK".to_string(), "BYE".to_string()],
            };
            
            // Register user in registrar-core
            self.registrar.register_user(&username, contact, Some(expires))
                .await
                .map_err(|e| SessionError::RegistrationFailed(e.to_string()))?;
            
            // Publish 200 OK response event
            let response_event = RvoipCrossCrateEvent::SessionToDialog(
                SessionToDialogEvent::SendRegisterResponse {
                    transaction_id,
                    status_code: 200,
                    reason: "OK".to_string(),
                    www_authenticate: None,
                    contact: Some(contact_uri),
                    expires: Some(expires),
                }
            );
            
            self.global_coordinator.publish(Arc::new(response_event))
                .await
                .map_err(|e| SessionError::InternalError(format!("Failed to publish 200 OK: {}", e)))?;
            
            info!("✅ User {} registered, sent 200 OK", username);
        } else {
            // Need authentication - send 401 challenge
            info!("🔐 Sending 401 challenge for {}", username);
            
            let response_event = RvoipCrossCrateEvent::SessionToDialog(
                SessionToDialogEvent::SendRegisterResponse {
                    transaction_id,
                    status_code: 401,
                    reason: "Unauthorized".to_string(),
                    www_authenticate: www_auth_challenge,
                    contact: None,
                    expires: None,
                }
            );
            
            self.global_coordinator.publish(Arc::new(response_event))
                .await
                .map_err(|e| SessionError::InternalError(format!("Failed to publish 401: {}", e)))?;
            
            info!("✅ Sent 401 challenge for {}", username);
        }
        
        Ok(())
    }
    
    /// Subscribe to IncomingRegister events and start handling them
    pub async fn start(self: Arc<Self>) -> Result<()> {
        info!("🎬 Starting RegistrationAdapter - subscribing to dialog_to_session events");
        
        // Subscribe to dialog-to-session events
        let mut receiver = self.global_coordinator
            .subscribe("dialog_to_session")
            .await
            .map_err(|e| SessionError::InternalError(format!("Subscribe failed: {}", e)))?;
        
        let handler = self.clone();
        
        tokio::spawn(async move {
            info!("🔔 RegistrationAdapter event loop started");
            
            loop {
                // Receive event from bus
                match receiver.recv().await {
                    Some(event_arc) => {
                        // Use trait-based downcasting via as_any()
                        if let Some(concrete) = event_arc.as_any().downcast_ref::<RvoipCrossCrateEvent>() {
                            // Check if it's an IncomingRegister event
                            if let RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::IncomingRegister {
                                    transaction_id,
                                    from_uri,
                                    contact_uri,
                                    expires,
                                    authorization,
                                    ..
                                }
                            ) = concrete {
                                debug!("📩 Received IncomingRegister for {}", from_uri);
                                
                                if let Err(e) = handler.handle_incoming_register(
                                    transaction_id.clone(),
                                    from_uri.clone(),
                                    contact_uri.clone(),
                                    *expires,
                                    authorization.clone(),
                                ).await {
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
    
    /// Extract username from SIP URI
    fn extract_username(uri: &str) -> Result<String> {
        // Parse URI: "sip:alice@127.0.0.1" → "alice"
        let parsed = uri.parse::<rvoip_sip_core::Uri>()
            .map_err(|e| SessionError::InvalidInput(format!("Invalid URI: {}", e)))?;
        
        parsed.user
            .ok_or_else(|| SessionError::InvalidInput("No username in URI".into()))
    }
}

