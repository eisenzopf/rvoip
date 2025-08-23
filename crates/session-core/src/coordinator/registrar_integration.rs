//! Registrar Integration Coordinator
//! 
//! This module provides integration between session-core and registrar-core,
//! handling user registration, presence management, and OAuth authentication.

use std::sync::Arc;
use std::collections::HashMap;
use std::str::FromStr;
use tokio::sync::RwLock;
use async_trait::async_trait;
use anyhow::{Result, anyhow};
use tracing::{debug, info, warn, error};
use dashmap::DashMap;

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::Message as SipMessage;
use rvoip_dialog_core::Dialog;
use rvoip_registrar_core::{
    RegistrarService, 
    api::ServiceMode,
    types::RegistrarConfig,
    events::{RegistrarEvent, PresenceEvent},
};
use infra_common::events::{
    system::EventSystem,
    api::EventSystem as EventSystemTrait,
};

use crate::auth::oauth::{OAuth2Validator, OAuth2Config};
use crate::api::types::SessionId;
use crate::api::builder::SessionManagerConfig;

/// Maps subscription Call-IDs to their corresponding dialogs
type SubscriptionDialogMap = Arc<DashMap<String, Arc<Dialog>>>;

/// Maps user AORs to their active subscriptions
type UserSubscriptionMap = Arc<DashMap<String, Vec<String>>>;

/// Registrar Integration Coordinator
/// 
/// Manages the integration between session-core and registrar-core,
/// handling:
/// - User registration (REGISTER)
/// - Presence publishing (PUBLISH)
/// - Presence subscriptions (SUBSCRIBE/NOTIFY)
/// - OAuth 2.0 authentication
pub struct RegistrarIntegration {
    /// The registrar service from registrar-core
    registrar: Arc<RegistrarService>,
    
    /// OAuth 2.0 validator for authentication
    oauth_validator: Arc<OAuth2Validator>,
    
    /// Maps subscription Call-IDs to dialogs
    subscription_dialogs: SubscriptionDialogMap,
    
    /// Maps user AORs to their active subscriptions
    user_subscriptions: UserSubscriptionMap,
    
    /// Event system for receiving registrar events
    event_system: Arc<EventSystem>,
    
    /// Configuration
    config: Arc<RegistrarConfig>,
    
    /// Service mode (P2P or B2BUA)
    mode: ServiceMode,
}

impl RegistrarIntegration {
    /// Create a new RegistrarIntegration
    pub async fn new(
        session_config: &SessionManagerConfig,
        oauth_config: OAuth2Config,
        mode: ServiceMode,
    ) -> Result<Self> {
        // Create registrar configuration from session config
        let registrar_config = RegistrarConfig {
            default_expires: 3600,
            min_expires: 60,
            max_expires: 86400,
            auto_buddy_lists: mode == ServiceMode::B2BUA,
            default_presence_enabled: true,
            max_contacts_per_user: 10,
            max_subscriptions_per_user: 100,
            expiry_check_interval: 60,
        };

        // Create event system  
        let event_system = Arc::new(EventSystem::default());

        // Create registrar service
        let mut registrar = RegistrarService::new_with_mode(
            mode.clone(),
            registrar_config.clone(),
        ).await?;
        
        // Set the event bus
        registrar.set_event_bus(event_system.clone());

        // Create OAuth validator
        let oauth_validator = OAuth2Validator::new(oauth_config);

        Ok(Self {
            registrar: Arc::new(registrar),
            oauth_validator: Arc::new(oauth_validator),
            subscription_dialogs: Arc::new(DashMap::new()),
            user_subscriptions: Arc::new(DashMap::new()),
            event_system,
            config: Arc::new(registrar_config),
            mode,
        })
    }

    /// Process a REGISTER request
    pub async fn process_register(
        &self,
        request: &SipMessage,
        dialog: Option<Arc<Dialog>>,
    ) -> Result<SipMessage> {
        // Extract authorization header
        let auth_header = request.headers().get_first(&HeaderName::Authorization)
            .ok_or_else(|| anyhow!("Missing Authorization header"))?;

        // Validate OAuth token
        let token = self.extract_bearer_token(&auth_header)?;
        let token_info = self.oauth_validator.validate(&token).await?;

        // Check if user in token matches the From header
        let from_header = request.headers().get_first(&HeaderName::From)
            .ok_or_else(|| anyhow!("Missing From header"))?;
        let from_uri = match &from_header.value {
            HeaderValue::From(f) => f.uri.to_string(),
            _ => return Err(anyhow!("Invalid From header")),
        };
        if !self.validate_user_match(&token_info.subject, &from_uri)? {
            return Ok(self.create_error_response(request, 403, "Forbidden"));
        }

        // Extract contact information from request
        let contact_header = request.headers().get_first(&HeaderName::Contact)
            .ok_or_else(|| anyhow!("Missing Contact header"))?;
        
        // Extract expires value
        let expires_header = request.headers().get_first(&HeaderName::Expires);
        let expires = expires_header.and_then(|h| h.value.as_integer()).map(|v| v as u32);
        
        // Register user with registrar-core
        let contact_info = rvoip_registrar_core::types::ContactInfo {
            uri: contact_header.value.to_string(),
            expires: expires.unwrap_or(3600),
            q_value: 1.0,
            transport: rvoip_registrar_core::types::Transport::Udp,
        };
        
        self.registrar.register_user(&from_uri, contact_info, expires).await?;
        
        // Create success response
        let response = request.create_response(200, "OK");
        info!("User {} registered successfully", from_uri);

        Ok(response)
    }

    /// Process a PUBLISH request for presence
    pub async fn process_publish(
        &self,
        request: &SipMessage,
        dialog: Option<Arc<Dialog>>,
    ) -> Result<SipMessage> {
        // Validate OAuth token
        let auth_header = request.headers().get_first(&HeaderName::Authorization)
            .ok_or_else(|| anyhow!("Missing Authorization header"))?;
        let token = self.extract_bearer_token(&auth_header)?;
        let token_info = self.oauth_validator.validate(&token).await?;

        // Check authorization
        let from_header = request.headers().get_first(&HeaderName::From)
            .ok_or_else(|| anyhow!("Missing From header"))?;
        let from_uri = match &from_header.value {
            HeaderValue::From(f) => f.uri.to_string(),
            _ => return Err(anyhow!("Invalid From header")),
        };
        if !self.validate_user_match(&token_info.subject, &from_uri)? {
            return Ok(self.create_error_response(request, 403, "Forbidden"));
        }

        // Extract presence information from request body
        // For now, just update to available status
        use rvoip_registrar_core::types::PresenceStatus;
        self.registrar.update_presence(
            &from_uri,
            PresenceStatus::Available,
            Some("Online".to_string())
        ).await?;
        
        // Create success response
        let response = request.create_response(200, "OK");
        debug!("Presence updated for user {}", from_uri);

        Ok(response)
    }

    /// Process a SUBSCRIBE request for presence
    pub async fn process_subscribe(
        &self,
        request: &SipMessage,
        dialog: Arc<Dialog>,
    ) -> Result<SipMessage> {
        // Validate OAuth token
        let auth_header = request.headers().get_first(&HeaderName::Authorization)
            .ok_or_else(|| anyhow!("Missing Authorization header"))?;
        let token = self.extract_bearer_token(&auth_header)?;
        let token_info = self.oauth_validator.validate(&token).await?;

        // Check authorization
        let from_header = request.headers().get_first(&HeaderName::From)
            .ok_or_else(|| anyhow!("Missing From header"))?;
        let from_uri = match &from_header.value {
            HeaderValue::From(f) => f.uri.to_string(),
            _ => return Err(anyhow!("Invalid From header")),
        };
        if !self.validate_user_match(&token_info.subject, &from_uri)? {
            return Ok(self.create_error_response(request, 403, "Forbidden"));
        }

        // Extract subscription details
        let event_header = request.headers().get_first(&HeaderName::Event)
            .ok_or_else(|| anyhow!("Missing Event header"))?;
        
        // Only handle presence subscriptions
        if !self.is_presence_subscription(&event_header)? {
            return Ok(self.create_error_response(request, 489, "Bad Event"));
        }

        // Store the dialog for this subscription
        let call_id_header = request.headers().get_first(&HeaderName::CallId)
            .ok_or_else(|| anyhow!("Missing Call-ID header"))?;
        let call_id = call_id_header.value.to_string();
        self.subscription_dialogs.insert(call_id.clone(), dialog.clone());

        // Track subscription for the user
        let target_uri = request.request_line().uri.to_string();
        self.user_subscriptions
            .entry(target_uri.clone())
            .or_insert_with(Vec::new)
            .push(call_id.clone());

        // Subscribe through registrar-core
        let subscription_id = self.registrar.subscribe_presence(
            &from_uri,
            &target_uri,
            3600
        ).await?;
        
        // Create success response  
        let response = request.create_response(200, "OK");

        if response.is_success() {
            info!("Subscription created for {} to {}", from_uri, target_uri);
            
            // Generate initial NOTIFY
            self.generate_notify(&call_id, &target_uri, dialog).await?;
        }

        Ok(response)
    }

    /// Generate a NOTIFY for a subscription
    async fn generate_notify(
        &self,
        call_id: &str,
        target_aor: &str,
        dialog: Arc<Dialog>,
    ) -> Result<()> {
        // Get presence state from registrar
        let presence_state = self.registrar.get_presence(target_aor).await?;
        
        // Create NOTIFY request
        let mut notify = dialog.create_request(Method::NOTIFY)?;
        
        // Add Event header
        notify.headers.push(Header::new(
            HeaderName::Event,
            HeaderValue::text("presence"),
        ));
        
        // Add Subscription-State header
        use rvoip_sip_core::types::subscription_state::{SubscriptionState, SubState};
        let sub_state = SubscriptionState::active(3600);
        notify.headers.push(sub_state.to_header());
        
        // Add Content-Type for PIDF
        notify.headers.push(Header::new(
            HeaderName::ContentType,
            HeaderValue::text("application/pidf+xml"),
        ));
        
        // Generate PIDF body from presence state
        let pidf_body = self.registrar.generate_pidf(target_aor).await?;
        notify.body = Some(pidf_body.into_bytes());
        
        // Update Content-Length
        notify.headers.push(Header::new(
            HeaderName::ContentLength,
            HeaderValue::integer(notify.body.as_ref().map(|b| b.len()).unwrap_or(0) as i64),
        ));
        
        // Send NOTIFY through dialog
        dialog.send_request(notify).await?;
        
        Ok(())
    }

    /// Handle registrar events
    pub async fn handle_registrar_event(&self, event: RegistrarEvent) -> Result<()> {
        match event {
            RegistrarEvent::UserRegistered { user, contact } => {
                info!("User {} registered with contact {:?}", user, contact);
            }
            RegistrarEvent::UserUnregistered { user } => {
                info!("User {} unregistered", user);
                // Update presence to offline for unregistered users
                if let Some(subscriptions) = self.user_subscriptions.get(&user) {
                    for call_id in subscriptions.value() {
                        if let Some(dialog) = self.subscription_dialogs.get(call_id) {
                            // Generate NOTIFY with offline presence
                            self.generate_notify(call_id, &user, dialog.clone()).await?;
                        }
                    }
                }
            }
            RegistrarEvent::RegistrationExpired { user } => {
                info!("Registration expired for {}", user);
                // Similar to unregister, update presence
            }
            _ => {
                debug!("Received registrar event: {:?}", event);
            }
        }
        Ok(())
    }
    
    /// Handle presence events
    pub async fn handle_presence_event(&self, event: PresenceEvent) -> Result<()> {
        match event {
            PresenceEvent::Updated { user, status, note, watchers_notified } => {
                info!("Presence updated for {}: {:?}, notified {} watchers", user, status, watchers_notified);
                // Find all subscriptions for this user
                if let Some(subscriptions) = self.user_subscriptions.get(&user) {
                    for call_id in subscriptions.value() {
                        if let Some(dialog) = self.subscription_dialogs.get(call_id) {
                            // Generate NOTIFY for this subscription
                            self.generate_notify(call_id, &user, dialog.clone()).await?;
                        }
                    }
                }
            }
            PresenceEvent::SubscriptionExpired { subscription_id, subscriber, target } => {
                self.handle_subscription_expired(&target, &subscription_id).await?;
            }
            _ => {
                debug!("Received presence event: {:?}", event);
            }
        }
        Ok(())
    }


    /// Handle subscription expiration
    async fn handle_subscription_expired(&self, aor: &str, call_id: &str) -> Result<()> {
        // Remove from subscription dialog map
        if let Some((_, dialog)) = self.subscription_dialogs.remove(call_id) {
            // Send final NOTIFY with terminated state
            let mut notify = dialog.create_request(Method::NOTIFY)?;
            
            notify.headers.push(Header::new(
                HeaderName::Event,
                HeaderValue::text("presence"),
            ));
            
            use rvoip_sip_core::types::subscription_state::{SubscriptionState, TerminationReason};
            let sub_state = SubscriptionState::terminated(TerminationReason::Timeout);
            notify.headers.push(sub_state.to_header());
            
            dialog.send_request(notify).await?;
        }
        
        // Remove from user subscriptions
        if let Some(mut subscriptions) = self.user_subscriptions.get_mut(aor) {
            subscriptions.retain(|id| id != call_id);
        }
        
        Ok(())
    }

    /// Extract bearer token from Authorization header
    fn extract_bearer_token(&self, header: &Header) -> Result<String> {
        let value = header.value.as_text()
            .ok_or_else(|| anyhow!("Invalid Authorization header format"))?;
        
        if !value.starts_with("Bearer ") {
            return Err(anyhow!("Authorization must use Bearer scheme"));
        }
        
        Ok(value[7..].to_string())
    }

    /// Validate that the token subject matches the SIP URI
    fn validate_user_match(&self, subject: &str, sip_uri: &str) -> Result<bool> {
        // Parse the SIP URI to extract user part
        let uri = Uri::from_str(sip_uri)?;
        let user = uri.user()
            .ok_or_else(|| anyhow!("SIP URI missing user part"))?;
        
        // Simple comparison - in production might need more sophisticated matching
        Ok(subject == user)
    }

    /// Check if this is a presence subscription
    fn is_presence_subscription(&self, event_header: &Header) -> Result<bool> {
        let value = event_header.value.as_text()
            .ok_or_else(|| anyhow!("Invalid Event header format"))?;
        Ok(value.to_lowercase() == "presence")
    }

    /// Create an error response
    fn create_error_response(&self, request: &SipMessage, code: u16, reason: &str) -> SipMessage {
        let mut response = request.create_response(code, reason);
        
        // Add warning header for OAuth errors if applicable
        if code == 401 || code == 403 {
            response.headers.push(Header::new(
                HeaderName::WwwAuthenticate,
                HeaderValue::text("Bearer realm=\"rvoip\""),
            ));
        }
        
        response
    }

    /// Shutdown the integration
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down RegistrarIntegration");
        
        // Clear all subscriptions
        for item in self.subscription_dialogs.iter() {
            let (call_id, dialog) = item.pair();
            // Send terminated NOTIFY
            if let Err(e) = self.handle_subscription_expired("", call_id).await {
                warn!("Error terminating subscription {}: {}", call_id, e);
            }
        }
        
        self.subscription_dialogs.clear();
        self.user_subscriptions.clear();
        
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_registrar_integration_creation() {
        // Test creation of RegistrarIntegration
        use crate::api::builder::SessionManagerConfig;
        
        let session_config = SessionManagerConfig::default();
        let oauth_config = OAuth2Config {
            jwks_url: Some("https://example.com/.well-known/jwks.json".to_string()),
            introspection_url: None,
            client_id: None,
            client_secret: None,
            required_scopes: vec!["sip".to_string()],
            cache_ttl: 300,
        };
        
        let integration = RegistrarIntegration::new(
            &session_config,
            oauth_config,
            ServiceMode::P2P,
        ).await;
        
        assert!(integration.is_ok());
    }
}