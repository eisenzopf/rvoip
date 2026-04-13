//! Server-side REGISTER request handler
//!
//! This adapter handles incoming REGISTER requests received from dialog-core,
//! coordinates with registrar-core for authentication and storage,
//! and sends appropriate responses back via the event bus.

use std::sync::Arc;
use tracing::{info, warn, debug};
use rvoip_infra_common::events::{
    coordinator::GlobalEventCoordinator,
    cross_crate::{RvoipCrossCrateEvent, DialogToSessionEvent, SessionToDialogEvent},
};
use rvoip_registrar_core::{RegistrarService, ContactInfo, Transport};
use crate::errors::Result;

/// Handles server-side REGISTER requests
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
    pub async fn handle_incoming_register(
        &self,
        transaction_id: String,
        from_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>,
        _to_uri: String,
        _call_id: String,
    ) -> Result<()> {
        info!("🔐 Handling incoming REGISTER from {}", from_uri);
        
        // Extract username from URI (e.g., "sip:alice@127.0.0.1" → "alice")
        let username = Self::extract_username(&from_uri)?;
        debug!("Extracted username: {}", username);
        
        // Authenticate via registrar-core
        let (should_register, www_auth_challenge) = self.registrar
            .authenticate_register(
                &username,
                authorization.as_deref(),
                "REGISTER",
                &from_uri,  // Use from_uri as request URI
            )
            .await
            .map_err(|e| crate::errors::SessionError::RegistrationFailed(e.to_string()))?;
        
        if should_register {
            // Valid credentials - register user
            info!("✅ Authentication successful for {}", username);
            
            // Parse contact URI into ContactInfo
            let contact = ContactInfo {
                uri: contact_uri.clone(),
                instance_id: uuid::Uuid::new_v4().to_string(),
                transport: Transport::UDP,
                user_agent: "rvoip-session-core-v3".to_string(),
                expires: chrono::Utc::now() + chrono::Duration::try_seconds(expires as i64)
                    .unwrap_or_else(|| chrono::Duration::seconds(3600)),
                q_value: 1.0,  // Default priority
                received: None,
                path: Vec::new(),
                methods: vec!["INVITE".to_string(), "ACK".to_string(), "BYE".to_string()],
            };
            
            // Register user in registrar-core
            self.registrar.register_user(&username, contact, Some(expires))
                .await
                .map_err(|e| crate::errors::SessionError::RegistrationFailed(e.to_string()))?;
            
            // Send 200 OK response via event bus
            let response_event = RvoipCrossCrateEvent::SessionToDialog(
                SessionToDialogEvent::SendRegisterResponse {
                    transaction_id: transaction_id.clone(),
                    status_code: 200,
                    reason: "OK".to_string(),
                    www_authenticate: None,
                    contact: Some(contact_uri),
                    expires: Some(expires),
                }
            );
            
            self.global_coordinator.publish(Arc::new(response_event))
                .await
                .map_err(|e| crate::errors::SessionError::InternalError(format!("Failed to publish response: {}", e)))?;
            
            info!("✅ User {} registered successfully, sent 200 OK", username);
        } else {
            // Need authentication - send 401 challenge
            info!("🔐 Sending 401 challenge for {}", username);
            
            let response_event = RvoipCrossCrateEvent::SessionToDialog(
                SessionToDialogEvent::SendRegisterResponse {
                    transaction_id: transaction_id.clone(),
                    status_code: 401,
                    reason: "Unauthorized".to_string(),
                    www_authenticate: www_auth_challenge,
                    contact: None,
                    expires: None,
                }
            );
            
            self.global_coordinator.publish(Arc::new(response_event))
                .await
                .map_err(|e| crate::errors::SessionError::InternalError(format!("Failed to publish 401: {}", e)))?;
            
            info!("✅ Sent 401 challenge for {}", username);
        }
        
        Ok(())
    }
    
    /// Subscribe to IncomingRegister events and handle them
    pub async fn start(self: Arc<Self>) -> Result<()> {
        info!("🎬 Starting RegistrationAdapter - subscribing to IncomingRegister events");
        
        // Subscribe to cross-crate events on the global bus
        let mut subscriber = self.global_coordinator
            .subscribe("rvoip_cross_crate_event")
            .await
            .map_err(|e| crate::errors::SessionError::InternalError(format!("Failed to subscribe: {}", e)))?;
        
        tokio::spawn(async move {
            info!("🔔 RegistrationAdapter event loop started");
            
            loop {
                if let Some(event_arc) = subscriber.recv().await {
                    // Downcast to RvoipCrossCrateEvent
                    if let Some(cross_crate_event) = event_arc.as_any().downcast_ref::<RvoipCrossCrateEvent>() {
                        // Filter for IncomingRegister events
                        if let RvoipCrossCrateEvent::DialogToSession(
                            DialogToSessionEvent::IncomingRegister {
                                transaction_id,
                                from_uri,
                                to_uri,
                                contact_uri,
                                expires,
                                authorization,
                                call_id,
                            }
                        ) = cross_crate_event {
                            debug!("📩 Received IncomingRegister event for {}", from_uri);
                            
                            if let Err(e) = self.handle_incoming_register(
                                transaction_id.clone(),
                                from_uri.clone(),
                                contact_uri.clone(),
                                expires,
                                authorization.clone(),
                                to_uri.clone(),
                                call_id.clone(),
                            ).await {
                                warn!("Failed to handle incoming REGISTER: {}", e);
                            }
                        }
                    }
                } else {
                    debug!("RegistrationAdapter subscriber channel closed");
                    break;
                }
            }
            
            info!("🛑 RegistrationAdapter event loop stopped");
        });
        
        info!("✅ RegistrationAdapter started");
        Ok(())
    }
    
    /// Extract username from SIP URI
    fn extract_username(uri: &str) -> Result<String> {
        // Parse URI and extract user part
        // Example: "sip:alice@127.0.0.1" → "alice"
        let parsed = uri.parse::<rvoip_sip_core::Uri>()
            .map_err(|e| crate::errors::SessionError::InvalidInput(format!("Invalid URI: {}", e)))?;
        
        parsed.user
            .ok_or_else(|| crate::errors::SessionError::InvalidInput("No username in URI".into()))
    }
}

