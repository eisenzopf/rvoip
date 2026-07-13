//! High-level API for session-core integration

#[allow(unused_imports)] // EventPublisher trait is needed for .publish() method
use rvoip_infra_common::events::api::{EventPublisher as _, EventSystem as EventSystemTrait};
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::error::Result;
use crate::events::{PresenceEvent, RegistrarEvent};
use crate::identity::{CredentialProvider, IdentityProvider};
use crate::presence::Presence;
use crate::registrar::Registrar;
use crate::types::{
    AddressOfRecord, BuddyInfo, ContactInfo, ContactReachability, PresenceState, PresenceStatus,
    RegistrarConfig,
};

/// High-level registrar service for session-core integration
pub struct RegistrarService {
    /// User registration management
    registrar: Arc<Registrar>,

    /// Presence management
    presence: Arc<Presence>,

    /// Configuration
    config: Arc<RegistrarConfig>,

    /// Event bus for publishing events
    event_bus: Option<Arc<rvoip_infra_common::events::system::EventSystem>>,

    /// Service mode
    mode: ServiceMode,

    /// User credential store for authentication
    user_store: Option<Arc<crate::registrar::UserStore>>,

    /// Digest authenticator
    auth: Option<Arc<rvoip_auth_core::DigestAuthenticator>>,

    /// Optional external identity source.
    identity_provider: Option<Arc<dyn IdentityProvider>>,

    /// Optional external credential source.
    credential_provider: Option<Arc<dyn CredentialProvider>>,
}

/// Service operation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceMode {
    /// P2P mode - minimal features, no auto-buddy lists
    P2P,
    /// B2BUA mode - full features with auto-buddy lists
    B2BUA,
}

impl RegistrarService {
    /// Create a new registrar service with the default P2P mode.
    pub async fn new() -> Result<Self> {
        Self::new_p2p().await
    }

    /// Create a new registrar service for P2P mode
    pub async fn new_p2p() -> Result<Self> {
        Self::new_with_mode(ServiceMode::P2P, RegistrarConfig::default()).await
    }

    /// Create a new registrar service for B2BUA mode
    pub async fn new_b2bua() -> Result<Self> {
        let mut config = RegistrarConfig::default();
        config.auto_buddy_lists = true;
        config.default_presence_enabled = true;

        Self::new_with_mode(ServiceMode::B2BUA, config).await
    }

    /// Create with specific mode and configuration
    pub async fn new_with_mode(mode: ServiceMode, config: RegistrarConfig) -> Result<Self> {
        let registrar = Arc::new(Registrar::with_config(config.clone()));
        let presence = Arc::new(Presence::new());

        // Start background tasks
        registrar.start_expiry_manager().await;

        info!("RegistrarService started in {:?} mode", mode);

        Ok(Self {
            registrar,
            presence,
            config: Arc::new(config),
            event_bus: None,
            mode,
            user_store: None,
            auth: None,
            identity_provider: None,
            credential_provider: None,
        })
    }

    /// Create with authentication support
    pub async fn with_auth(
        mode: ServiceMode,
        config: RegistrarConfig,
        realm: &str,
    ) -> Result<Self> {
        let mut service = Self::new_with_mode(mode, config).await?;

        // Create auth components
        let auth = Arc::new(rvoip_auth_core::DigestAuthenticator::new(realm));
        let user_store = Arc::new(crate::registrar::UserStore::new(realm));

        service.auth = Some(auth);
        service.user_store = Some(user_store);

        Ok(service)
    }

    pub fn with_identity_provider(mut self, provider: Arc<dyn IdentityProvider>) -> Self {
        self.identity_provider = Some(provider);
        self
    }

    pub fn set_identity_provider(&mut self, provider: Arc<dyn IdentityProvider>) {
        self.identity_provider = Some(provider);
    }

    pub fn set_credential_provider(&mut self, provider: Arc<dyn CredentialProvider>) {
        self.credential_provider = Some(provider);
    }

    /// Get user store for adding users
    pub fn user_store(&self) -> Option<&Arc<crate::registrar::UserStore>> {
        self.user_store.as_ref()
    }

    /// Get digest authenticator
    pub fn authenticator(&self) -> Option<&Arc<rvoip_auth_core::DigestAuthenticator>> {
        self.auth.as_ref()
    }

    /// Set the event bus for publishing events
    pub fn set_event_bus(
        &mut self,
        event_bus: Arc<rvoip_infra_common::events::system::EventSystem>,
    ) {
        self.event_bus = Some(event_bus);
    }

    // ========== Registration Methods ==========

    /// Handle REGISTER request with authentication
    ///
    /// This method:
    /// 1. Checks for Authorization header
    /// 2. If present, validates credentials
    /// 3. If valid, processes registration
    /// 4. If invalid or missing, returns 401 challenge
    ///
    /// Returns a tuple: (should_process, challenge_header)
    pub async fn authenticate_register(
        &self,
        username: &str,
        authorization: Option<&str>,
        method: &str,
        uri: &str,
    ) -> Result<(bool, Option<String>)> {
        // If no auth configured, allow all
        if self.auth.is_none() {
            return Ok((true, None));
        }

        let auth = self.auth.as_ref().unwrap();
        let external_password = if let Some(provider) = &self.credential_provider {
            match AddressOfRecord::parse(uri) {
                Ok(aor) => provider.sip_digest_secret(&aor).await?,
                Err(_) => {
                    warn!(
                        stage = "credential-lookup",
                        uri_present = !uri.is_empty(),
                        uri_bytes = uri.len(),
                        "Unable to parse AOR for credential lookup"
                    );
                    None
                }
            }
        } else {
            None
        };
        let local_password = self
            .user_store
            .as_ref()
            .and_then(|user_store| user_store.get_password(username));
        let Some(password) = external_password.or(local_password) else {
            warn!(
                stage = "credential-lookup",
                username_present = !username.is_empty(),
                username_bytes = username.len(),
                "Registration credential was not found"
            );
            // Still send challenge (don't reveal user doesn't exist)
            let challenge = auth.generate_challenge();
            let www_auth = auth.format_www_authenticate(&challenge);
            return Ok((false, Some(www_auth)));
        };

        // Check for Authorization header
        if let Some(auth_header) = authorization {
            // Parse authorization header
            let digest_response = rvoip_auth_core::DigestAuthenticator::parse_authorization(
                auth_header,
            )
            .map_err(|e| {
                crate::error::RegistrarError::Internal(format!("Failed to parse auth: {}", e))
            })?;

            // Validate digest response
            info!(
                stage = "digest-validation",
                username_present = !digest_response.username.is_empty(),
                username_bytes = digest_response.username.len(),
                realm_present = !digest_response.realm.is_empty(),
                realm_bytes = digest_response.realm.len(),
                nonce_present = !digest_response.nonce.is_empty(),
                nonce_bytes = digest_response.nonce.len(),
                uri_present = !digest_response.uri.is_empty(),
                uri_bytes = digest_response.uri.len(),
                response_present = !digest_response.response.is_empty(),
                response_bytes = digest_response.response.len(),
                "Validating SIP digest response"
            );

            let is_valid = auth
                .validate_response(&digest_response, method, &password)
                .map_err(|e| {
                    crate::error::RegistrarError::Internal(format!(
                        "Failed to validate digest: {}",
                        e
                    ))
                })?;

            info!(
                stage = "digest-validation",
                accepted = is_valid,
                "SIP digest validation completed"
            );

            if is_valid {
                info!(
                    stage = "digest-validation",
                    username_present = !username.is_empty(),
                    username_bytes = username.len(),
                    "SIP registration authenticated"
                );
                Ok((true, None))
            } else {
                warn!(
                    stage = "digest-validation",
                    username_present = !username.is_empty(),
                    username_bytes = username.len(),
                    "SIP registration authentication failed"
                );
                let challenge = auth.generate_challenge();
                let www_auth = auth.format_www_authenticate(&challenge);
                Ok((false, Some(www_auth)))
            }
        } else {
            // No Authorization header - send challenge
            let challenge = auth.generate_challenge();
            let www_auth = auth.format_www_authenticate(&challenge);
            Ok((false, Some(www_auth)))
        }
    }

    /// Register a user with contact information
    ///
    /// Called when session-core receives a REGISTER request
    pub async fn register_user(
        &self,
        user_id: &str,
        contact: ContactInfo,
        expires: Option<u32>,
    ) -> Result<()> {
        let expires = expires.unwrap_or(self.config.default_expires);

        // Register the user
        self.registrar
            .register_user(user_id, contact.clone(), expires)
            .await?;

        // In B2BUA mode, set up automatic buddy lists
        if self.mode == ServiceMode::B2BUA && self.config.auto_buddy_lists {
            self.setup_auto_buddy_list(user_id).await?;
        }

        // Publish event
        self.publish_event(RegistrarEvent::UserRegistered {
            user: user_id.to_string(),
            contact,
        })
        .await;

        info!(
            stage = "registration-update",
            operation = "register",
            user_present = !user_id.is_empty(),
            user_bytes = user_id.len(),
            "Registrar operation completed"
        );
        Ok(())
    }

    pub async fn register_aor(
        &self,
        aor: &AddressOfRecord,
        contact: ContactInfo,
        expires: Option<u32>,
    ) -> Result<()> {
        self.validate_identity_for_registration(aor).await?;
        let expires = expires.unwrap_or(self.config.default_expires);
        self.registrar
            .register_aor(aor, contact.clone(), expires)
            .await?;
        self.publish_event(RegistrarEvent::UserRegistered {
            user: aor.to_string(),
            contact,
        })
        .await;
        Ok(())
    }

    pub async fn register_contacts(
        &self,
        aor: &AddressOfRecord,
        contacts: Vec<ContactInfo>,
        expires: Option<u32>,
    ) -> Result<()> {
        self.validate_identity_for_registration(aor).await?;
        let expires = expires.unwrap_or(self.config.default_expires);
        self.registrar
            .register_contacts(aor, contacts, expires)
            .await
    }

    /// Unregister a user
    ///
    /// Called when session-core receives REGISTER with Expires: 0
    pub async fn unregister_user(&self, user_id: &str) -> Result<()> {
        // Clear presence
        self.presence.clear_presence(user_id).await?;

        // Remove registrations
        self.registrar.unregister_user(user_id).await?;

        // Publish event
        self.publish_event(RegistrarEvent::UserUnregistered {
            user: user_id.to_string(),
        })
        .await;

        info!(
            stage = "registration-update",
            operation = "unregister",
            user_present = !user_id.is_empty(),
            user_bytes = user_id.len(),
            "Registrar operation completed"
        );
        Ok(())
    }

    pub async fn unregister_aor(&self, aor: &AddressOfRecord) -> Result<()> {
        self.registrar.unregister_aor(aor).await
    }

    pub async fn unregister_contact_aor(
        &self,
        aor: &AddressOfRecord,
        contact_uri: &str,
    ) -> Result<()> {
        self.registrar
            .unregister_contact_aor(aor, contact_uri)
            .await
    }

    pub async fn unregister_all_bindings(&self, aor: &AddressOfRecord) -> Result<()> {
        self.registrar.unregister_all_bindings(aor).await
    }

    /// Lookup where a user can be reached
    ///
    /// Called when session-core needs to route an INVITE
    pub async fn lookup_user(&self, user_id: &str) -> Result<Vec<ContactInfo>> {
        self.registrar.lookup_user(user_id).await
    }

    pub async fn lookup_aor(&self, aor: &AddressOfRecord) -> Result<Vec<ContactInfo>> {
        self.registrar.lookup_aor(aor).await
    }

    pub async fn lookup_live_contacts(
        &self,
        aor: &AddressOfRecord,
        method: &str,
    ) -> Result<Vec<ContactInfo>> {
        if let Some(provider) = &self.identity_provider {
            match provider.resolve_identity(aor).await? {
                Some(identity) if identity.enabled => {}
                Some(_) => return Ok(Vec::new()),
                None => {
                    return Err(crate::error::RegistrarError::UserNotFound(aor.to_string()));
                }
            }
        }
        self.registrar.lookup_live_contacts(aor, method).await
    }

    pub async fn refresh_registration_aor(
        &self,
        aor: &AddressOfRecord,
        contact_uri: &str,
        expires: u32,
    ) -> Result<()> {
        self.registrar
            .refresh_registration_aor(aor, contact_uri, expires)
            .await
    }

    pub async fn set_contact_reachability(
        &self,
        aor: &AddressOfRecord,
        contact_uri: &str,
        reachability: ContactReachability,
    ) -> Result<()> {
        self.registrar
            .set_contact_reachability(aor, contact_uri, reachability)
            .await
    }

    pub fn add_domain_alias(&self, alias: impl Into<String>, target: impl Into<String>) {
        self.registrar.add_domain_alias(alias, target);
    }

    /// Get all registered users
    pub async fn list_registered_users(&self) -> Vec<String> {
        self.registrar.list_users().await
    }

    /// Check if a user is registered
    pub async fn is_registered(&self, user_id: &str) -> bool {
        self.registrar.is_registered(user_id).await
    }

    async fn validate_identity_for_registration(&self, aor: &AddressOfRecord) -> Result<()> {
        let Some(provider) = &self.identity_provider else {
            return Ok(());
        };
        match provider.resolve_identity(aor).await? {
            Some(identity) if identity.enabled => Ok(()),
            Some(_) => {
                let _ = self.registrar.unregister_aor(aor).await;
                Err(crate::error::RegistrarError::InvalidRegistration(format!(
                    "identity {aor} is disabled"
                )))
            }
            None => Err(crate::error::RegistrarError::UserNotFound(aor.to_string())),
        }
    }

    // ========== Presence Methods ==========

    /// Update user's presence status
    ///
    /// Called when session-core receives a PUBLISH request
    pub async fn update_presence(
        &self,
        user_id: &str,
        status: PresenceStatus,
        note: Option<String>,
    ) -> Result<()> {
        // Update presence
        self.presence
            .update_presence(user_id, status.clone(), note.clone())
            .await?;

        // Notify watchers
        let notified = self.presence.notify_subscribers(user_id).await?;

        // Publish event
        self.publish_event(PresenceEvent::Updated {
            user: user_id.to_string(),
            status,
            note,
            watchers_notified: notified.len(),
        })
        .await;

        debug!(
            stage = "presence-update",
            user_present = !user_id.is_empty(),
            user_bytes = user_id.len(),
            watchers_notified = notified.len(),
            "Presence operation completed"
        );
        Ok(())
    }

    /// Get user's current presence
    pub async fn get_presence(&self, user_id: &str) -> Result<PresenceState> {
        self.presence.get_presence(user_id).await
    }

    /// Subscribe to a user's presence
    ///
    /// Called when session-core receives a SUBSCRIBE request
    pub async fn subscribe_presence(
        &self,
        subscriber: &str,
        target: &str,
        expires: Option<u32>,
    ) -> Result<String> {
        let expires = expires.unwrap_or(self.config.default_expires);

        let subscription_id = self.presence.subscribe(subscriber, target, expires).await?;

        // Publish event
        self.publish_event(PresenceEvent::Subscribed {
            subscriber: subscriber.to_string(),
            target: target.to_string(),
            subscription_id: subscription_id.clone(),
        })
        .await;

        debug!(
            stage = "presence-subscribe",
            subscriber_present = !subscriber.is_empty(),
            subscriber_bytes = subscriber.len(),
            target_present = !target.is_empty(),
            target_bytes = target.len(),
            "Presence subscription created"
        );
        Ok(subscription_id)
    }

    /// Unsubscribe from presence
    pub async fn unsubscribe_presence(&self, subscription_id: &str) -> Result<()> {
        self.presence.unsubscribe(subscription_id).await?;

        // Publish event
        self.publish_event(PresenceEvent::Unsubscribed {
            subscription_id: subscription_id.to_string(),
        })
        .await;

        Ok(())
    }

    /// Get buddy list for a user
    ///
    /// In B2BUA mode, returns all registered users with their presence
    pub async fn get_buddy_list(&self, user_id: &str) -> Result<Vec<BuddyInfo>> {
        if self.mode == ServiceMode::B2BUA {
            // In B2BUA mode, all registered users are buddies
            let users = self.registrar.list_users().await;
            let mut buddies = Vec::new();

            for buddy_id in users {
                if buddy_id != user_id {
                    // Get presence if available
                    let presence = self.presence.get_presence(&buddy_id).await.ok();

                    buddies.push(BuddyInfo {
                        user_id: buddy_id.clone(),
                        display_name: Some(buddy_id.clone()),
                        status: presence
                            .as_ref()
                            .and_then(|p| p.extended_status.as_ref())
                            .map(|s| PresenceStatus::from(s.clone()))
                            .unwrap_or(PresenceStatus::Offline),
                        note: presence.as_ref().and_then(|p| p.note.clone()),
                        last_updated: presence
                            .as_ref()
                            .map(|p| p.last_updated)
                            .unwrap_or_else(chrono::Utc::now),
                        is_online: presence.is_some(),
                        active_devices: presence.as_ref().map(|p| p.devices.len()).unwrap_or(0),
                    });
                }
            }

            Ok(buddies)
        } else {
            // In P2P mode, use explicit buddy list
            self.presence.get_buddy_list(user_id).await
        }
    }

    /// Generate PIDF XML for a user's presence
    ///
    /// Used when session-core needs to send NOTIFY
    pub async fn generate_pidf(&self, user_id: &str) -> Result<String> {
        self.presence.generate_pidf(user_id).await
    }

    /// Parse PIDF XML
    ///
    /// Used when session-core receives PUBLISH
    pub async fn parse_pidf(&self, xml: &str) -> Result<PresenceState> {
        self.presence.parse_pidf(xml).await
    }

    // ========== Internal Methods ==========

    /// Set up automatic buddy list for a newly registered user
    async fn setup_auto_buddy_list(&self, user_id: &str) -> Result<()> {
        if self.mode != ServiceMode::B2BUA || !self.config.auto_buddy_lists {
            return Ok(());
        }

        // Get all other registered users
        let all_users = self.registrar.list_users().await;

        for other_user in all_users {
            if other_user != user_id {
                // Create bidirectional subscriptions
                let _ = self
                    .presence
                    .subscribe(user_id, &other_user, self.config.default_expires)
                    .await;
                let _ = self
                    .presence
                    .subscribe(&other_user, user_id, self.config.default_expires)
                    .await;
            }
        }

        debug!(
            stage = "buddy-list-setup",
            user_present = !user_id.is_empty(),
            user_bytes = user_id.len(),
            "Automatic buddy-list setup completed"
        );
        Ok(())
    }

    /// Publish an event to the event bus
    async fn publish_event<E>(&self, event: E)
    where
        E: rvoip_infra_common::events::types::Event + std::fmt::Debug + 'static,
    {
        if let Some(bus) = &self.event_bus {
            let publisher = bus.create_publisher::<E>();
            if publisher.publish(event).await.is_err() {
                warn!(
                    stage = "event-publish",
                    event_type = std::any::type_name::<E>(),
                    "Registrar event publication failed"
                );
            }
        }
    }

    /// Shutdown the service
    pub async fn shutdown(&self) -> Result<()> {
        self.registrar.stop_expiry_manager().await;
        info!("RegistrarService shutdown");
        Ok(())
    }
}

// Conversion helpers
impl From<ExtendedStatus> for PresenceStatus {
    fn from(status: ExtendedStatus) -> Self {
        use crate::types::ExtendedStatus;
        match status {
            ExtendedStatus::Available => PresenceStatus::Available,
            ExtendedStatus::Away => PresenceStatus::Away,
            ExtendedStatus::Busy => PresenceStatus::Busy,
            ExtendedStatus::DoNotDisturb => PresenceStatus::DoNotDisturb,
            ExtendedStatus::OnThePhone => PresenceStatus::InCall,
            ExtendedStatus::Offline => PresenceStatus::Offline,
            ExtendedStatus::InMeeting => PresenceStatus::Busy,
            ExtendedStatus::Custom(s) => PresenceStatus::Custom(s),
        }
    }
}

use crate::types::ExtendedStatus;

#[cfg(test)]
mod diagnostic_source_tests {
    #[test]
    fn register_authentication_logs_only_structural_metadata() {
        let source = include_str!("mod.rs");
        let start = source.find("pub async fn authenticate_register").unwrap();
        let end = source[start..]
            .find("pub async fn register_user")
            .map(|offset| start + offset)
            .unwrap();
        let authenticate_source = &source[start..end];

        for fragments in [
            ["Validating digest for ", "user={}"],
            ["Client response", " hash"],
            ["unknown user", ": {}"],
            ["User {}", " authenticated"],
            ["failed for ", "user {}"],
            ["Unable to parse AOR for credential lookup", ": {}"],
        ] {
            let forbidden = fragments.concat();
            assert!(
                !authenticate_source.contains(&forbidden),
                "REGISTER authentication regained value-bearing log: {forbidden}"
            );
        }
        for required in [
            "stage = \"credential-lookup\"",
            "stage = \"digest-validation\"",
            "username_present",
            "username_bytes",
            "realm_present",
            "realm_bytes",
            "nonce_present",
            "nonce_bytes",
            "uri_present",
            "uri_bytes",
            "response_present",
            "response_bytes",
        ] {
            assert!(
                authenticate_source.contains(required),
                "REGISTER authentication log lost structural field: {required}"
            );
        }
    }

    #[test]
    fn registrar_api_logs_do_not_render_identity_or_event_errors() {
        let source = include_str!("mod.rs");

        for fragments in [
            ["User {}", " registered"],
            ["User {}", " unregistered"],
            ["Presence updated for {}", "watchers notified"],
            ["{} subscribed to {}", "presence"],
            ["Auto buddy list set up ", "for {}"],
            ["Failed to publish ", "event: {:?}"],
        ] {
            let forbidden = fragments.concat();
            assert!(
                !source.contains(&forbidden),
                "registrar API regained value-bearing diagnostic: {forbidden}"
            );
        }

        for required in [
            "stage = \"registration-update\"",
            "stage = \"presence-update\"",
            "stage = \"presence-subscribe\"",
            "stage = \"buddy-list-setup\"",
            "stage = \"event-publish\"",
            "user_present",
            "user_bytes",
            "subscriber_present",
            "subscriber_bytes",
            "target_present",
            "target_bytes",
        ] {
            assert!(
                source.contains(required),
                "registrar API diagnostic lost structural field: {required}"
            );
        }
    }
}
