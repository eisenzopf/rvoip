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
    RegisteredFlowRoute, RegistrarConfig, Transport,
};
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;

const REGISTER_DIGEST_NONCE_TTL: Duration = Duration::from_secs(5 * 60);
const REGISTER_DIGEST_NONCE_RETENTION: Duration = Duration::from_secs(10 * 60);
const MAX_REGISTER_DIGEST_NONCES: usize = 4_096;
const MAX_REGISTER_DIGEST_NONCE_COUNTS: usize = 16_384;
const MAX_REGISTER_DIGEST_SEQUENCES_PER_USERNAME: usize = 4_096;
const MAX_REGISTER_DIGEST_SEQUENCES_PER_NONCE: usize = 8_192;
const MAX_REGISTER_DIGEST_SEQUENCES_PER_USERNAME_NONCE: usize = 4_096;
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

    /// Process-local RFC 5626 route capabilities keyed by an opaque random
    /// token stored on the matching Contact binding.
    registered_flows: Arc<DashMap<String, RegisteredFlowBinding>>,
}

#[derive(Clone)]
struct RegisteredFlowBinding {
    aor: String,
    contact_uri: String,
    instance_id: String,
    reg_id: u32,
    remote_addr: std::net::SocketAddr,
    transport: Transport,
    process_local_flow_id: u64,
    expires: chrono::DateTime<chrono::Utc>,
    reachable: bool,
}

/// A validated, serialized AOR update that has not yet changed registrar
/// state. This is used by SIP response owners that must make the binding
/// visible only after a terminal successful response write.
#[must_use = "dropping a prepared AOR registration leaves registrar state unchanged"]
#[doc(hidden)]
pub struct PreparedAorRegistration {
    mutation: crate::registrar::PreparedRegistrationMutation,
    event_bus: Option<Arc<rvoip_infra_common::events::system::EventSystem>>,
    user: String,
    contact: ContactInfo,
}

impl PreparedAorRegistration {
    /// Apply the already-validated mutation and publish its observation.
    /// Registry application itself is infallible while the per-AOR mutation
    /// lease is held.
    pub async fn commit(self) {
        let Self {
            mutation,
            event_bus,
            user,
            contact,
        } = self;
        mutation.commit();
        if let Some(bus) = event_bus {
            let publisher = bus.create_publisher::<RegistrarEvent>();
            if publisher
                .publish(RegistrarEvent::UserRegistered { user, contact })
                .await
                .is_err()
            {
                warn!(
                    stage = "event-publish",
                    event_type = std::any::type_name::<RegistrarEvent>(),
                    "Registrar event publication failed"
                );
            }
        }
    }
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
        let config = RegistrarConfig {
            auto_buddy_lists: true,
            default_presence_enabled: true,
            ..RegistrarConfig::default()
        };

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
            registered_flows: Arc::new(DashMap::new()),
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

    /// Mint an opaque process-local token for an authenticated RFC 5626 flow.
    ///
    /// The token contains no socket address, tenant, device, or numeric flow
    /// identity. It becomes routeable only after [`Self::bind_registered_flow`]
    /// associates it with the committed Contact.
    #[doc(hidden)]
    pub fn new_registered_flow_token(&self) -> String {
        format!("rf1_{}", uuid::Uuid::new_v4().simple())
    }

    /// Bind a committed outbound Contact to the exact stream transport that
    /// carried its authenticated REGISTER.
    #[doc(hidden)]
    pub fn bind_registered_flow(
        &self,
        aor: &AddressOfRecord,
        contact: &ContactInfo,
        process_local_flow_id: u64,
    ) -> Result<()> {
        let token = contact
            .flow_id
            .as_deref()
            .filter(|token| {
                token.len() == 36
                    && token.starts_with("rf1_")
                    && token[4..].bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .ok_or_else(|| {
                crate::error::RegistrarError::InvalidRegistration(
                    "registered flow token is absent or invalid".into(),
                )
            })?;
        let reg_id = contact
            .reg_id
            .filter(|reg_id| *reg_id != 0)
            .ok_or_else(|| {
                crate::error::RegistrarError::InvalidRegistration(
                    "registered flow reg-id is absent or invalid".into(),
                )
            })?;
        if contact.instance_id.is_empty() || process_local_flow_id == 0 {
            return Err(crate::error::RegistrarError::InvalidRegistration(
                "registered flow identity is incomplete".into(),
            ));
        }
        if !matches!(
            contact.transport,
            Transport::TCP | Transport::TLS | Transport::WS | Transport::WSS
        ) {
            return Err(crate::error::RegistrarError::InvalidRegistration(
                "registered flow transport is not connection-oriented".into(),
            ));
        }
        let remote_addr = contact
            .received
            .as_deref()
            .ok_or_else(|| {
                crate::error::RegistrarError::InvalidRegistration(
                    "registered flow observed address is absent".into(),
                )
            })?
            .parse()
            .map_err(|_| {
                crate::error::RegistrarError::InvalidRegistration(
                    "registered flow observed address is invalid".into(),
                )
            })?;

        let binding = RegisteredFlowBinding {
            aor: aor.as_str().to_string(),
            contact_uri: contact.uri.clone(),
            instance_id: contact.instance_id.clone(),
            reg_id,
            remote_addr,
            transport: contact.transport,
            process_local_flow_id,
            expires: contact.expires,
            reachable: true,
        };

        match self.registered_flows.entry(token.to_string()) {
            Entry::Vacant(entry) => {
                entry.insert(binding);
                Ok(())
            }
            Entry::Occupied(_) => Err(crate::error::RegistrarError::InvalidRegistration(
                "registered flow token is already bound".into(),
            )),
        }
    }

    /// Promote a staged flow after the matching Contact mutation commits.
    ///
    /// Replacement bindings for the same AOR, instance, and reg-id remain
    /// available until this point. That preserves the previous live route
    /// when the final REGISTER response is proven to have reached zero wire.
    #[doc(hidden)]
    pub fn commit_registered_flow(&self, aor: &AddressOfRecord, contact: &ContactInfo) -> bool {
        let Some(token) = contact.flow_id.as_deref() else {
            return false;
        };
        let Some(reg_id) = contact.reg_id else {
            return false;
        };
        self.registered_flows.retain(|existing_token, existing| {
            existing_token == token
                || existing.aor != aor.as_str()
                || existing.instance_id != contact.instance_id
                || existing.reg_id != reg_id
        });
        self.registered_flows
            .get(token)
            .is_some_and(|binding| binding.reachable)
    }

    /// Remove a flow route after the matching Contact is unregistered.
    #[doc(hidden)]
    pub fn remove_registered_flow(&self, aor: &AddressOfRecord, contact: &ContactInfo) {
        self.registered_flows.retain(|_, binding| {
            binding.aor != aor.as_str()
                || (binding.contact_uri != contact.uri
                    && (binding.instance_id != contact.instance_id
                        || Some(binding.reg_id) != contact.reg_id))
        });
    }

    /// Discard a staged flow token when the final REGISTER response was
    /// proven not to reach the wire.
    #[doc(hidden)]
    pub fn discard_registered_flow_token(&self, flow_token: &str) {
        self.registered_flows.remove(flow_token);
    }

    /// Resolve a Contact into a verified, process-local registered-flow route.
    ///
    /// Stale tokens, copied tokens for another AOR/device, expired contacts,
    /// and unreachable contacts all fail closed.
    pub async fn resolve_registered_flow(
        &self,
        aor: &AddressOfRecord,
        contact: &ContactInfo,
    ) -> Result<RegisteredFlowRoute> {
        let token = contact.flow_id.as_deref().ok_or_else(|| {
            crate::error::RegistrarError::InvalidRegistration(
                "contact has no registered flow".into(),
            )
        })?;
        let now = chrono::Utc::now();
        self.registered_flows
            .retain(|_, binding| binding.expires > now);
        let binding = self
            .registered_flows
            .get(token)
            .map(|binding| binding.clone())
            .ok_or_else(|| {
                crate::error::RegistrarError::RegistrationExpired(
                    "registered flow token is stale".into(),
                )
            })?;
        if binding.aor != aor.as_str()
            || binding.contact_uri != contact.uri
            || binding.instance_id != contact.instance_id
            || Some(binding.reg_id) != contact.reg_id
        {
            return Err(crate::error::RegistrarError::InvalidRegistration(
                "registered flow does not own this contact".into(),
            ));
        }
        if !binding.reachable {
            return Err(crate::error::RegistrarError::InvalidRegistration(
                "registered flow is unreachable".into(),
            ));
        }

        let current = self.registrar.lookup_live_contacts(aor, "INVITE").await?;
        if !current.iter().any(|candidate| {
            candidate.flow_id.as_deref() == Some(token)
                && candidate.instance_id == contact.instance_id
                && candidate.reg_id == contact.reg_id
                && candidate.uri == contact.uri
        }) {
            return Err(crate::error::RegistrarError::RegistrationExpired(
                "registered flow contact is no longer live".into(),
            ));
        }

        Ok(RegisteredFlowRoute {
            remote_addr: binding.remote_addr,
            transport: binding.transport,
            process_local_flow_id: binding.process_local_flow_id,
            expires: contact.expires,
        })
    }

    /// Change one exact flow's reachability and publish the authoritative
    /// degraded/recovered transition without exposing its token.
    pub async fn set_registered_flow_reachability(
        &self,
        aor: &AddressOfRecord,
        flow_token: &str,
        reachability: ContactReachability,
    ) -> Result<()> {
        if reachability != ContactReachability::Reachable {
            if let Some(mut binding) = self.registered_flows.get_mut(flow_token) {
                binding.reachable = false;
            }
        }
        let (contact, changed) = self
            .registrar
            .set_flow_reachability(aor, flow_token, reachability)
            .await?;
        if reachability == ContactReachability::Reachable {
            if let Some(mut binding) = self.registered_flows.get_mut(flow_token) {
                binding.reachable = true;
            }
        }
        if !changed {
            return Ok(());
        }
        let Some(reg_id) = contact.reg_id else {
            return Ok(());
        };
        let event = match reachability {
            ContactReachability::Unreachable => RegistrarEvent::RegistrationFlowDegraded {
                user: aor.user().to_string(),
                instance_id: contact.instance_id,
                reg_id,
            },
            ContactReachability::Reachable => RegistrarEvent::RegistrationFlowRecovered {
                user: aor.user().to_string(),
                instance_id: contact.instance_id,
                reg_id,
            },
            ContactReachability::Unknown => return Ok(()),
        };
        self.publish_event(event).await;
        Ok(())
    }

    /// Mark every binding owned by one closed process-local transport flow as
    /// unreachable. The numeric flow identity is never persisted or emitted;
    /// public observations contain only the registered device identity.
    #[doc(hidden)]
    pub async fn mark_process_local_flow_unreachable(&self, process_local_flow_id: u64) -> usize {
        let bindings = self.stage_process_local_flow_unreachable(process_local_flow_id);
        let mut changed = 0;
        for (aor, token) in bindings {
            let Ok(aor) = AddressOfRecord::parse(&aor) else {
                continue;
            };
            if self
                .set_registered_flow_reachability(&aor, &token, ContactReachability::Unreachable)
                .await
                .is_ok()
            {
                changed += 1;
            }
        }
        changed
    }

    fn stage_process_local_flow_unreachable(
        &self,
        process_local_flow_id: u64,
    ) -> Vec<(String, String)> {
        if process_local_flow_id == 0 {
            return Vec::new();
        }
        let bindings: Vec<(String, String)> = self
            .registered_flows
            .iter()
            .filter(|binding| binding.process_local_flow_id == process_local_flow_id)
            .map(|binding| (binding.aor.clone(), binding.key().clone()))
            .collect();
        for (_, token) in &bindings {
            if let Some(mut binding) = self.registered_flows.get_mut(token) {
                binding.reachable = false;
            }
        }
        bindings
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
        if replay
            .nonces
            .get(nonce)
            .is_none_or(|issued| issued.expires_at <= now)
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

        let mut username_sequences = 0;
        let mut nonce_sequences = 0;
        let mut username_nonce_sequences = 0;
        for (recorded_username, recorded_nonce, _) in replay.nonce_counts.keys() {
            if recorded_username == username {
                username_sequences += 1;
            }
            if recorded_nonce == nonce {
                nonce_sequences += 1;
            }
            if recorded_username == username && recorded_nonce == nonce {
                username_nonce_sequences += 1;
            }
        }
        if replay.nonce_counts.len() >= MAX_REGISTER_DIGEST_NONCE_COUNTS
            || username_sequences >= MAX_REGISTER_DIGEST_SEQUENCES_PER_USERNAME
            || nonce_sequences >= MAX_REGISTER_DIGEST_SEQUENCES_PER_NONCE
            || username_nonce_sequences >= MAX_REGISTER_DIGEST_SEQUENCES_PER_USERNAME_NONCE
        {
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
        self.prepare_register_aor(aor, contact, expires)
            .await?
            .commit()
            .await;
        Ok(())
    }

    /// Validate and serialize an AOR mutation without publishing it to the
    /// authoritative registry. Dropping the returned value is a no-op. The
    /// returned lease must be committed or dropped before preparing another
    /// update for the same AOR.
    #[doc(hidden)]
    pub async fn prepare_register_aor(
        &self,
        aor: &AddressOfRecord,
        contact: ContactInfo,
        expires: Option<u32>,
    ) -> Result<PreparedAorRegistration> {
        self.validate_identity_for_registration(aor).await?;
        let expires = expires.unwrap_or(self.config.default_expires);
        let mutation = self
            .registrar
            .prepare_register_aor(aor, contact.clone(), expires)
            .await?;
        Ok(PreparedAorRegistration {
            mutation,
            event_bus: self.event_bus.clone(),
            user: aor.to_string(),
            contact,
        })
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

    #[tokio::test]
    async fn register_replay_quota_is_fair_between_usernames() {
        let (service, challenge) = service_and_challenge().await;
        {
            let replay = service.digest_replay.as_ref().unwrap();
            let mut replay = replay.lock().unwrap();
            for index in 0..MAX_REGISTER_DIGEST_SEQUENCES_PER_USERNAME {
                replay.nonce_counts.insert(
                    (
                        "noisy-user".to_string(),
                        challenge.nonce.clone(),
                        format!("client-{index}"),
                    ),
                    1,
                );
            }
        }

        assert!(!service.accept_register_nonce_count(
            "noisy-user",
            &challenge.nonce,
            "new-client",
            1,
        ));
        assert!(service.accept_register_nonce_count(
            "unrelated-user",
            &challenge.nonce,
            "new-client",
            1,
        ));
    }
}

#[cfg(test)]
mod registered_flow_tests {
    use super::*;

    fn contact(token: String, reg_id: u32) -> ContactInfo {
        ContactInfo {
            uri: "sip:alice@10.0.0.20:5061;transport=tls;ob".into(),
            instance_id: "urn:uuid:11111111-2222-4333-8444-555555555555".into(),
            transport: Transport::TLS,
            user_agent: "independent-test-ua/1.0".into(),
            expires: chrono::Utc::now() + chrono::Duration::minutes(10),
            q_value: 1.0,
            received: Some("198.51.100.20:41000".into()),
            path: vec!["<sip:edge.example.test;lr>".into()],
            methods: vec!["INVITE".into()],
            reg_id: Some(reg_id),
            flow_id: Some(token),
            reachability: ContactReachability::Reachable,
        }
    }

    #[tokio::test]
    async fn registered_flow_is_opaque_owned_live_and_fail_closed() {
        let service = RegistrarService::new().await.unwrap();
        let aor = AddressOfRecord::parse("sip:alice@example.test").unwrap();
        let other = AddressOfRecord::parse("sip:bob@example.test").unwrap();
        let token = service.new_registered_flow_token();
        assert_eq!(token.len(), 36);
        assert!(token.starts_with("rf1_"));
        assert!(!token.contains("198.51.100.20"));

        let contact = contact(token.clone(), 2);
        let prepared = service
            .prepare_register_aor(&aor, contact.clone(), Some(600))
            .await
            .unwrap();
        service.bind_registered_flow(&aor, &contact, 42).unwrap();
        prepared.commit().await;

        let route = service
            .resolve_registered_flow(&aor, &contact)
            .await
            .unwrap();
        assert_eq!(route.remote_addr().to_string(), "198.51.100.20:41000");
        assert_eq!(route.process_local_flow_id(), 42);
        let diagnostic = format!("{route:?}");
        assert!(!diagnostic.contains("198.51.100.20"));
        assert!(!diagnostic.contains("42"));
        assert!(service
            .resolve_registered_flow(&other, &contact)
            .await
            .is_err());

        service
            .set_registered_flow_reachability(&aor, &token, ContactReachability::Unreachable)
            .await
            .unwrap();
        assert!(service
            .resolve_registered_flow(&aor, &contact)
            .await
            .is_err());

        service.remove_registered_flow(&aor, &contact);
        assert!(service
            .resolve_registered_flow(&aor, &contact)
            .await
            .is_err());
        service.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn staged_replacement_preserves_old_flow_until_contact_commit() {
        let service = RegistrarService::new().await.unwrap();
        let aor = AddressOfRecord::parse("sip:alice@example.test").unwrap();

        let old_token = service.new_registered_flow_token();
        let old_contact = contact(old_token.clone(), 2);
        let old_prepared = service
            .prepare_register_aor(&aor, old_contact.clone(), Some(600))
            .await
            .unwrap();
        service
            .bind_registered_flow(&aor, &old_contact, 42)
            .unwrap();
        old_prepared.commit().await;
        assert!(service.commit_registered_flow(&aor, &old_contact));

        let rejected_token = service.new_registered_flow_token();
        let rejected_contact = contact(rejected_token.clone(), 2);
        let rejected_prepared = service
            .prepare_register_aor(&aor, rejected_contact.clone(), Some(600))
            .await
            .unwrap();
        service
            .bind_registered_flow(&aor, &rejected_contact, 43)
            .unwrap();
        service.discard_registered_flow_token(&rejected_token);
        drop(rejected_prepared);

        let current = service.lookup_aor(&aor).await.unwrap();
        let current_old = current
            .iter()
            .find(|candidate| candidate.flow_id.as_deref() == Some(old_token.as_str()))
            .unwrap();
        assert_eq!(
            service
                .resolve_registered_flow(&aor, current_old)
                .await
                .unwrap()
                .process_local_flow_id(),
            42
        );

        let replacement_token = service.new_registered_flow_token();
        let replacement_contact = contact(replacement_token.clone(), 2);
        let replacement_prepared = service
            .prepare_register_aor(&aor, replacement_contact.clone(), Some(600))
            .await
            .unwrap();
        service
            .bind_registered_flow(&aor, &replacement_contact, 44)
            .unwrap();
        replacement_prepared.commit().await;
        assert!(service.commit_registered_flow(&aor, &replacement_contact));

        assert!(service
            .resolve_registered_flow(&aor, &old_contact)
            .await
            .is_err());
        let current = service.lookup_aor(&aor).await.unwrap();
        assert_eq!(
            service
                .resolve_registered_flow(&aor, &current[0])
                .await
                .unwrap()
                .process_local_flow_id(),
            44
        );

        let closed_token = service.new_registered_flow_token();
        let closed_contact = contact(closed_token.clone(), 2);
        let closed_prepared = service
            .prepare_register_aor(&aor, closed_contact.clone(), Some(600))
            .await
            .unwrap();
        service
            .bind_registered_flow(&aor, &closed_contact, 45)
            .unwrap();
        assert_eq!(service.stage_process_local_flow_unreachable(45).len(), 1);
        closed_prepared.commit().await;
        assert!(!service.commit_registered_flow(&aor, &closed_contact));
        service
            .set_registered_flow_reachability(&aor, &closed_token, ContactReachability::Unreachable)
            .await
            .unwrap();
        let current = service.lookup_aor(&aor).await.unwrap();
        assert_eq!(current[0].reachability, ContactReachability::Unreachable);
        assert!(service
            .resolve_registered_flow(&aor, &current[0])
            .await
            .is_err());
        service.shutdown().await.unwrap();
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
