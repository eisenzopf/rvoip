//! High-level API for session-core integration

#[allow(unused_imports)] // EventPublisher trait is needed for .publish() method
use rvoip_infra_common::events::api::{EventPublisher as _, EventSystem as EventSystemTrait};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use zeroize::Zeroize;

use crate::error::Result;
use crate::events::{PresenceEvent, RegistrarEvent};
use crate::identity::{CredentialProvider, IdentityProvider};
use crate::presence::Presence;
use crate::registrar::Registrar;
use crate::types::{
    AddressOfRecord, BuddyInfo, ContactInfo, ContactReachability, PresenceState, PresenceStatus,
    RegistrarConfig,
};

const REGISTER_DIGEST_NONCE_TTL: Duration = Duration::from_secs(5 * 60);
const REGISTER_DIGEST_NONCE_RETENTION: Duration = Duration::from_secs(10 * 60);
const MAX_REGISTER_DIGEST_NONCES: usize = 4_096;
const MAX_REGISTER_DIGEST_NONCE_COUNTS: usize = 16_384;
const MAX_REGISTER_AUTHORIZATION_BYTES: usize = 8 * 1024;

#[derive(Clone)]
struct IssuedDigestNonce {
    realm: String,
    algorithm: rvoip_auth_core::DigestAlgorithm,
    qop: Option<Vec<String>>,
    opaque: Option<String>,
    expires_at: Instant,
    retain_until: Instant,
}

#[derive(Default)]
struct RegisterDigestReplayState {
    nonces: HashMap<String, IssuedDigestNonce>,
    nonce_counts: HashMap<(String, String, String), u32>,
}

enum IssuedNonceStatus {
    Current(IssuedDigestNonce),
    Expired,
    Unknown,
}

impl RegisterDigestReplayState {
    fn sweep(&mut self, now: Instant) {
        self.nonces.retain(|_, issued| issued.retain_until > now);
        let retained_nonces: HashSet<&str> = self.nonces.keys().map(String::as_str).collect();
        self.nonce_counts
            .retain(|(_, nonce, _), _| retained_nonces.contains(nonce.as_str()));
    }

    /// Reclaim only expired challenges when admission is under pressure.
    /// Active challenges are never evicted: a client must be able to complete
    /// the proof it was just asked to compute even during unauthenticated
    /// challenge churn.
    fn reclaim_expired_for_admission(&mut self, now: Instant) {
        if self.nonces.len() < MAX_REGISTER_DIGEST_NONCES {
            return;
        }
        self.nonces.retain(|_, issued| issued.expires_at > now);
        let retained_nonces: HashSet<&str> = self.nonces.keys().map(String::as_str).collect();
        self.nonce_counts
            .retain(|(_, nonce, _), _| retained_nonces.contains(nonce.as_str()));
    }
}

fn parse_nonce_count(value: &str) -> Option<u32> {
    if value.len() != 8 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let count = u32::from_str_radix(value, 16).ok()?;
    (count != 0).then_some(count)
}

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

    /// Bounded, server-issued nonce and nonce-count state for the legacy
    /// registrar API. Clustered listeners should use the provider-backed
    /// replay store exposed by `rvoip-sip`.
    digest_replay: Option<Mutex<RegisterDigestReplayState>>,

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
            digest_replay: None,
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
        service.digest_replay = Some(Mutex::new(RegisterDigestReplayState::default()));

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
        self.authenticate_register_request(username, authorization, method, uri, uri)
            .await
    }

    /// Authenticate a REGISTER while binding the Digest proof to the actual
    /// Request-URI and looking credentials up by the registration AOR.
    ///
    /// The older [`Self::authenticate_register`] API uses one URI for both
    /// values and remains available for source compatibility.
    pub async fn authenticate_register_request(
        &self,
        username: &str,
        authorization: Option<&str>,
        method: &str,
        request_uri: &str,
        credential_aor_uri: &str,
    ) -> Result<(bool, Option<String>)> {
        // If no auth configured, allow all
        if self.auth.is_none() {
            return Ok((true, None));
        }

        let auth = self.auth.as_ref().unwrap();
        let Some(auth_header) = authorization else {
            return Ok((false, Some(self.issue_register_digest_challenge(false))));
        };
        if auth_header.len() > MAX_REGISTER_AUTHORIZATION_BYTES {
            return Ok((false, Some(self.issue_register_digest_challenge(false))));
        }

        let digest_response =
            match rvoip_auth_core::DigestAuthenticator::parse_authorization(auth_header) {
                Ok(response) => response,
                Err(_) => {
                    return Ok((false, Some(self.issue_register_digest_challenge(false))));
                }
            };

        let issued = match self.issued_register_nonce_status(&digest_response.nonce) {
            IssuedNonceStatus::Current(issued) => issued,
            IssuedNonceStatus::Expired => {
                return Ok((false, Some(self.issue_register_digest_challenge(true))));
            }
            IssuedNonceStatus::Unknown => {
                return Ok((false, Some(self.issue_register_digest_challenge(false))));
            }
        };

        // Bind every client-controlled Digest field back to the challenge and
        // request. In particular, accepting a valid hash for a different URI
        // turns Digest into a reusable bearer credential.
        let cnonce_is_valid = digest_response
            .cnonce
            .as_deref()
            .is_some_and(|value| !value.is_empty() && value.len() <= 256);
        let nonce_count = digest_response.nc.as_deref().and_then(parse_nonce_count);
        if digest_response.username != username
            || digest_response.realm != issued.realm
            || digest_response.uri != request_uri
            || digest_response.algorithm != issued.algorithm
            || digest_response.opaque != issued.opaque
            || digest_response.qop.as_deref() != Some("auth")
            || !cnonce_is_valid
            || nonce_count.is_none()
        {
            return Ok((false, Some(self.issue_register_digest_challenge(false))));
        }

        let external_secret = if let Some(provider) = &self.credential_provider {
            match AddressOfRecord::parse(credential_aor_uri) {
                Ok(aor) => {
                    let password = provider.sip_digest_secret(&aor).await?;
                    password.map(|mut password| {
                        let ha1 = digest_response.algorithm.compute_ha1(
                            &digest_response.username,
                            &digest_response.realm,
                            &password,
                        );
                        password.zeroize();
                        rvoip_auth_core::DigestSecret::Ha1(ha1)
                    })
                }
                Err(_) => {
                    warn!(
                        stage = "credential-lookup",
                        uri_present = !credential_aor_uri.is_empty(),
                        uri_bytes = credential_aor_uri.len(),
                        "Unable to parse AOR for credential lookup"
                    );
                    None
                }
            }
        } else {
            None
        };
        let local_secret = self.user_store.as_ref().and_then(|user_store| {
            user_store.get_digest_secret(
                username,
                &digest_response.realm,
                digest_response.algorithm,
            )
        });
        let Some(secret) = external_secret.or(local_secret) else {
            warn!(
                stage = "credential-lookup",
                username_present = !username.is_empty(),
                username_bytes = username.len(),
                "Registration credential was not found"
            );
            // Still send challenge (don't reveal user doesn't exist)
            return Ok((false, Some(self.issue_register_digest_challenge(false))));
        };

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
            .validate_response_with_secret(&digest_response, method, &secret)
            .unwrap_or(false);
        let accepted = is_valid
            && self.accept_register_nonce_count(
                &digest_response.username,
                &digest_response.nonce,
                digest_response.cnonce.as_deref().expect("validated above"),
                nonce_count.expect("validated above"),
            );

        info!(
            stage = "digest-validation",
            accepted, "SIP digest validation completed"
        );

        if accepted {
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
            Ok((false, Some(self.issue_register_digest_challenge(false))))
        }
    }

    fn issue_register_digest_challenge(&self, stale: bool) -> String {
        let auth = self
            .auth
            .as_ref()
            .expect("digest challenges require configured authentication");
        let mut challenge = auth.generate_challenge();
        let now = Instant::now();
        if let Some(replay) = &self.digest_replay {
            let mut replay = replay
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            replay.sweep(now);
            replay.reclaim_expired_for_admission(now);
            if replay.nonces.len() >= MAX_REGISTER_DIGEST_NONCES {
                if let Some((nonce, issued)) = replay
                    .nonces
                    .iter()
                    .max_by_key(|(_, issued)| issued.expires_at)
                    .map(|(nonce, issued)| (nonce.clone(), issued.clone()))
                {
                    // Admission is saturated with active challenges. Reuse a
                    // still-valid challenge instead of evicting one and
                    // invalidating an in-flight legitimate proof.
                    challenge = rvoip_auth_core::DigestChallenge {
                        realm: issued.realm,
                        nonce,
                        algorithm: issued.algorithm,
                        qop: issued.qop,
                        opaque: issued.opaque,
                    };
                }
            } else {
                replay.nonces.insert(
                    challenge.nonce.clone(),
                    IssuedDigestNonce {
                        realm: challenge.realm.clone(),
                        algorithm: challenge.algorithm,
                        qop: challenge.qop.clone(),
                        opaque: challenge.opaque.clone(),
                        expires_at: now + REGISTER_DIGEST_NONCE_TTL,
                        retain_until: now + REGISTER_DIGEST_NONCE_RETENTION,
                    },
                );
            }
        }
        auth.format_www_authenticate_with_stale(&challenge, stale)
    }

    fn issued_register_nonce_status(&self, nonce: &str) -> IssuedNonceStatus {
        let Some(replay) = &self.digest_replay else {
            return IssuedNonceStatus::Unknown;
        };
        let now = Instant::now();
        let mut replay = replay
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        replay.sweep(now);
        match replay.nonces.get(nonce) {
            Some(issued) if issued.expires_at > now => IssuedNonceStatus::Current(issued.clone()),
            Some(_) => IssuedNonceStatus::Expired,
            None => IssuedNonceStatus::Unknown,
        }
    }

    fn accept_register_nonce_count(
        &self,
        username: &str,
        nonce: &str,
        cnonce: &str,
        count: u32,
    ) -> bool {
        let Some(replay) = &self.digest_replay else {
            return false;
        };
        let now = Instant::now();
        let mut replay = replay
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        replay.sweep(now);
        if !replay
            .nonces
            .get(nonce)
            .is_some_and(|issued| issued.expires_at > now)
        {
            return false;
        }

        let key = (username.to_string(), nonce.to_string(), cnonce.to_string());
        if let Some(previous) = replay.nonce_counts.get_mut(&key) {
            if count <= *previous {
                return false;
            }
            *previous = count;
            return true;
        }
        if replay.nonce_counts.len() >= MAX_REGISTER_DIGEST_NONCE_COUNTS {
            return false;
        }
        replay.nonce_counts.insert(key, count);
        true
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
mod digest_replay_tests {
    use super::*;
    use rvoip_auth_core::{DigestAuthenticator, DigestClient};

    async fn service_and_challenge() -> (RegistrarService, rvoip_auth_core::DigestChallenge) {
        let service = RegistrarService::with_auth(
            ServiceMode::P2P,
            RegistrarConfig::default(),
            "registrar.test",
        )
        .await
        .unwrap();
        service
            .user_store()
            .unwrap()
            .add_user("alice", "correct horse")
            .unwrap();
        let (accepted, challenge) = service
            .authenticate_register("alice", None, "REGISTER", "sip:registrar.test")
            .await
            .unwrap();
        assert!(!accepted);
        let challenge = DigestAuthenticator::parse_challenge(&challenge.unwrap()).unwrap();
        (service, challenge)
    }

    fn authorization(challenge: &rvoip_auth_core::DigestChallenge, uri: &str, nc: u32) -> String {
        let computed = DigestClient::compute_response_with_state(
            "alice",
            "correct horse",
            challenge,
            "REGISTER",
            uri,
            nc,
            None,
        )
        .unwrap();
        DigestClient::format_authorization_with_state("alice", challenge, uri, &computed)
    }

    #[tokio::test]
    async fn register_digest_accepts_increasing_nonce_counts_and_rejects_replay() {
        let uri = "sip:registrar.test";
        let (service, challenge) = service_and_challenge().await;
        let first = authorization(&challenge, uri, 1);
        assert_eq!(
            service
                .authenticate_register("alice", Some(&first), "REGISTER", uri)
                .await
                .unwrap(),
            (true, None)
        );

        let replay = service
            .authenticate_register("alice", Some(&first), "REGISTER", uri)
            .await
            .unwrap();
        assert!(
            !replay.0,
            "the same nonce-count must not authenticate twice"
        );

        let second = authorization(&challenge, uri, 2);
        assert!(
            service
                .authenticate_register("alice", Some(&second), "REGISTER", uri)
                .await
                .unwrap()
                .0
        );
    }

    #[tokio::test]
    async fn register_digest_rejects_unissued_nonce_uri_swap_and_missing_qop() {
        let uri = "sip:registrar.test";
        let (service, challenge) = service_and_challenge().await;

        let mut unissued = challenge.clone();
        unissued.nonce = "not-issued-by-this-registrar".to_string();
        let attempt = authorization(&unissued, uri, 1);
        assert!(
            !service
                .authenticate_register("alice", Some(&attempt), "REGISTER", uri)
                .await
                .unwrap()
                .0
        );

        let wrong_uri = authorization(&challenge, "sip:other.test", 1);
        assert!(
            !service
                .authenticate_register("alice", Some(&wrong_uri), "REGISTER", uri)
                .await
                .unwrap()
                .0
        );

        let mut legacy = challenge.clone();
        legacy.qop = None;
        let missing_qop = authorization(&legacy, uri, 1);
        assert!(
            !service
                .authenticate_register("alice", Some(&missing_qop), "REGISTER", uri)
                .await
                .unwrap()
                .0
        );
    }

    #[tokio::test]
    async fn challenge_churn_never_evicts_an_active_legitimate_nonce() {
        let uri = "sip:registrar.test";
        let (service, legitimate) = service_and_challenge().await;
        let now = Instant::now();
        let replay = service.digest_replay.as_ref().unwrap();
        {
            let mut replay = replay.lock().unwrap();
            for index in 0..(MAX_REGISTER_DIGEST_NONCES - 1) {
                replay.nonces.insert(
                    format!("attacker-{index}"),
                    IssuedDigestNonce {
                        realm: legitimate.realm.clone(),
                        algorithm: legitimate.algorithm,
                        qop: legitimate.qop.clone(),
                        opaque: Some(format!("opaque-{index}")),
                        expires_at: now + REGISTER_DIGEST_NONCE_TTL,
                        retain_until: now + REGISTER_DIGEST_NONCE_RETENTION,
                    },
                );
            }
            assert_eq!(replay.nonces.len(), MAX_REGISTER_DIGEST_NONCES);
        }

        for _ in 0..32 {
            let challenge = service.issue_register_digest_challenge(false);
            assert!(!challenge.is_empty());
        }

        {
            let replay = replay.lock().unwrap();
            assert_eq!(replay.nonces.len(), MAX_REGISTER_DIGEST_NONCES);
            assert!(replay.nonces.contains_key(&legitimate.nonce));
        }

        let proof = authorization(&legitimate, uri, 1);
        assert!(
            service
                .authenticate_register("alice", Some(&proof), "REGISTER", uri)
                .await
                .unwrap()
                .0,
            "a legitimate proof must survive unauthenticated challenge churn"
        );

        let shared =
            DigestAuthenticator::parse_challenge(&service.issue_register_digest_challenge(false))
                .unwrap();
        let first_client = authorization(&shared, uri, 1);
        let second_client = authorization(&shared, uri, 1);
        let first_cnonce = DigestAuthenticator::parse_authorization(&first_client)
            .unwrap()
            .cnonce;
        let second_cnonce = DigestAuthenticator::parse_authorization(&second_client)
            .unwrap()
            .cnonce;
        assert_ne!(first_cnonce, second_cnonce);
        assert!(
            service
                .authenticate_register("alice", Some(&first_client), "REGISTER", uri)
                .await
                .unwrap()
                .0
        );
        assert!(
            service
                .authenticate_register("alice", Some(&second_client), "REGISTER", uri)
                .await
                .unwrap()
                .0,
            "distinct clients sharing a saturated nonce must each start at nc=1"
        );
        assert!(
            !service
                .authenticate_register("alice", Some(&second_client), "REGISTER", uri)
                .await
                .unwrap()
                .0,
            "replaying the same cnonce/nc sequence must still fail"
        );
    }

    #[tokio::test]
    async fn expired_register_digest_nonce_is_rechallenged_as_stale() {
        let uri = "sip:registrar.test";
        let (service, challenge) = service_and_challenge().await;
        {
            let mut replay = service
                .digest_replay
                .as_ref()
                .unwrap()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let nonce = replay.nonces.get_mut(&challenge.nonce).unwrap();
            nonce.expires_at = Instant::now() - Duration::from_secs(1);
        }
        let attempt = authorization(&challenge, uri, 1);
        let result = service
            .authenticate_register("alice", Some(&attempt), "REGISTER", uri)
            .await
            .unwrap();
        assert!(!result.0);
        assert!(result.1.unwrap().contains("stale=true"));
    }
}

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
