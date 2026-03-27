//! Registration Manager for outbound SIP REGISTER with auth challenge handling
//!
//! Manages the full lifecycle of SIP registrations:
//! - Initial REGISTER request
//! - 401/407 digest authentication challenge-response
//! - Periodic refresh before expiry
//! - Graceful unregistration (Expires: 0)
//! - State change event emission

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{RwLock, watch};
use rvoip_sip_core::builder::SimpleRequestBuilder;
use rvoip_sip_core::builder::UserAgentBuilderExt;
use rvoip_sip_core::types::{TypedHeader, expires::Expires};
use rvoip_sip_core::types::headers::HeaderName;
use rvoip_sip_core::types::auth::{
    WwwAuthenticate, ProxyAuthenticate, Challenge,
};
use crate::auth::digest::{
    DigestCredentials, extract_challenge, build_authorization, build_proxy_authorization,
};
use crate::api::client::RegistrationHandle;
use crate::errors::{Result, SessionError};
use super::SessionCoordinator;

/// Current state of a managed registration
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationState {
    /// Not registered
    Idle,
    /// REGISTER sent, waiting for response
    Registering,
    /// Successfully registered (active)
    Active,
    /// Registration refresh in progress
    Refreshing,
    /// Registration failed
    Failed(String),
    /// Registration expired (refresh failed)
    Expired,
    /// Unregistration in progress
    Unregistering,
    /// Successfully unregistered
    Unregistered,
}

impl std::fmt::Display for RegistrationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistrationState::Idle => write!(f, "Idle"),
            RegistrationState::Registering => write!(f, "Registering"),
            RegistrationState::Active => write!(f, "Active"),
            RegistrationState::Refreshing => write!(f, "Refreshing"),
            RegistrationState::Failed(reason) => write!(f, "Failed: {}", reason),
            RegistrationState::Expired => write!(f, "Expired"),
            RegistrationState::Unregistering => write!(f, "Unregistering"),
            RegistrationState::Unregistered => write!(f, "Unregistered"),
        }
    }
}

/// Configuration for a managed registration
#[derive(Debug, Clone)]
pub struct RegistrationConfig {
    pub registrar_uri: String,
    pub from_uri: String,
    pub contact_uri: String,
    pub expires: u32,
    pub username: Option<String>,
    pub password: Option<String>,
}

/// A managed registration that handles refresh and auth
pub struct ManagedRegistration {
    config: RegistrationConfig,
    state_tx: watch::Sender<RegistrationState>,
    state_rx: watch::Receiver<RegistrationState>,
    handle: RwLock<Option<RegistrationHandle>>,
    /// Shutdown signal for the refresh task
    shutdown_tx: RwLock<Option<watch::Sender<bool>>>,
}

impl ManagedRegistration {
    /// Create a new managed registration
    pub fn new(config: RegistrationConfig) -> Self {
        let (state_tx, state_rx) = watch::channel(RegistrationState::Idle);
        Self {
            config,
            state_tx,
            state_rx,
            handle: RwLock::new(None),
            shutdown_tx: RwLock::new(None),
        }
    }

    /// Get a receiver for registration state changes
    pub fn state_receiver(&self) -> watch::Receiver<RegistrationState> {
        self.state_rx.clone()
    }

    /// Get the current registration state
    pub fn state(&self) -> RegistrationState {
        self.state_rx.borrow().clone()
    }

    /// Get the current handle if active
    pub async fn handle(&self) -> Option<RegistrationHandle> {
        self.handle.read().await.clone()
    }
}

/// Send a REGISTER request and handle auth challenge if needed.
///
/// Returns the RegistrationHandle on success.
pub(crate) async fn send_register_with_auth(
    coordinator: &Arc<SessionCoordinator>,
    config: &RegistrationConfig,
    expires: u32,
) -> Result<RegistrationHandle> {
    let call_id = format!("reg-{}-{}", std::process::id(), uuid::Uuid::new_v4());
    let from_tag = format!("tag-{}", uuid::Uuid::new_v4().simple());
    let branch = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
    let local_addr = coordinator.get_bound_address();

    // Build initial REGISTER
    let request = SimpleRequestBuilder::register(&config.registrar_uri)
        .map_err(|e| SessionError::invalid_uri(&format!("Invalid registrar URI: {}", e)))?
        .from("", &config.from_uri, Some(&from_tag))
        .to("", &config.from_uri, None)
        .call_id(&call_id)
        .cseq(1)
        .via(&local_addr.to_string(), "UDP", Some(&branch))
        .contact(&config.contact_uri, None)
        .header(TypedHeader::Expires(Expires::new(expires)))
        .max_forwards(70)
        .user_agent("RVoIP-SessionCore/1.0")
        .build();

    // Resolve destination
    let uri: rvoip_sip_core::Uri = config.registrar_uri.parse()
        .map_err(|e| SessionError::invalid_uri(&format!("Invalid registrar URI: {}", e)))?;
    let destination = rvoip_dialog_core::dialog::dialog_utils::uri_resolver::resolve_uri_to_socketaddr(&uri)
        .await
        .ok_or_else(|| SessionError::network_error(&format!(
            "Failed to resolve registrar address: {}", config.registrar_uri
        )))?;

    tracing::info!(
        registrar = %config.registrar_uri,
        destination = %destination,
        expires = expires,
        "Sending REGISTER"
    );

    // Send first REGISTER
    let response = coordinator.dialog_coordinator.dialog_api()
        .send_non_dialog_request(request, destination, Duration::from_secs(32))
        .await
        .map_err(|e| SessionError::internal(&format!("REGISTER failed: {}", e)))?;

    let status_code = response.status_code();

    // Success on first try
    if status_code == 200 {
        tracing::info!(registrar = %config.registrar_uri, "Registration successful (no auth required)");
        let transaction_id = response.call_id()
            .map(|cid| cid.to_string())
            .unwrap_or_else(|| call_id.clone());

        return Ok(RegistrationHandle {
            transaction_id,
            expires,
            contact_uri: config.contact_uri.clone(),
            registrar_uri: config.registrar_uri.clone(),
        });
    }

    // Handle 401 / 407 auth challenge
    if status_code == 401 || status_code == 407 {
        let (username, password) = match (&config.username, &config.password) {
            (Some(u), Some(p)) => (u.clone(), p.clone()),
            _ => {
                return Err(SessionError::ProtocolError {
                    message: format!(
                        "Registrar returned {} but no credentials configured",
                        status_code
                    ),
                });
            }
        };

        let credentials = DigestCredentials { username, password };

        // Extract the challenge from the response
        let challenge = if status_code == 401 {
            extract_www_authenticate_challenge(&response)?
        } else {
            extract_proxy_authenticate_challenge(&response)?
        };

        let digest_challenge = extract_challenge(&challenge)?;

        tracing::debug!(
            realm = %digest_challenge.realm,
            algorithm = ?digest_challenge.algorithm,
            qop = ?digest_challenge.qop_options,
            "Received auth challenge, computing digest response"
        );

        // Build authenticated REGISTER (CSeq incremented to 2)
        let branch2 = format!("z9hG4bK{}", uuid::Uuid::new_v4().simple());
        let request_uri: rvoip_sip_core::Uri = config.registrar_uri.parse()
            .map_err(|e| SessionError::invalid_uri(&format!("Invalid registrar URI: {}", e)))?;

        let mut builder = SimpleRequestBuilder::register(&config.registrar_uri)
            .map_err(|e| SessionError::invalid_uri(&format!("Invalid registrar URI: {}", e)))?
            .from("", &config.from_uri, Some(&from_tag))
            .to("", &config.from_uri, None)
            .call_id(&call_id)
            .cseq(2)
            .via(&local_addr.to_string(), "UDP", Some(&branch2))
            .contact(&config.contact_uri, None)
            .header(TypedHeader::Expires(Expires::new(expires)))
            .max_forwards(70)
            .user_agent("RVoIP-SessionCore/1.0");

        // Add the appropriate authorization header
        if status_code == 401 {
            let auth_header = build_authorization(
                &credentials,
                &digest_challenge,
                "REGISTER",
                &request_uri,
            )?;
            builder = builder.header(TypedHeader::Authorization(auth_header));
        } else {
            let proxy_auth_header = build_proxy_authorization(
                &credentials,
                &digest_challenge,
                "REGISTER",
                &request_uri,
            )?;
            builder = builder.header(TypedHeader::ProxyAuthorization(proxy_auth_header));
        }

        let auth_request = builder.build();

        tracing::info!(
            registrar = %config.registrar_uri,
            "Sending authenticated REGISTER (CSeq 2)"
        );

        let auth_response = coordinator.dialog_coordinator.dialog_api()
            .send_non_dialog_request(auth_request, destination, Duration::from_secs(32))
            .await
            .map_err(|e| SessionError::internal(&format!("Authenticated REGISTER failed: {}", e)))?;

        let auth_status = auth_response.status_code();
        if auth_status == 200 {
            tracing::info!(registrar = %config.registrar_uri, "Registration successful (authenticated)");
            let transaction_id = auth_response.call_id()
                .map(|cid| cid.to_string())
                .unwrap_or_else(|| call_id.clone());

            return Ok(RegistrationHandle {
                transaction_id,
                expires,
                contact_uri: config.contact_uri.clone(),
                registrar_uri: config.registrar_uri.clone(),
            });
        }

        return Err(SessionError::ProtocolError {
            message: format!(
                "Authenticated REGISTER failed: {} {}",
                auth_status,
                auth_response.reason_phrase()
            ),
        });
    }

    // Any other status code is an error
    Err(SessionError::ProtocolError {
        message: format!(
            "REGISTER failed: {} {}",
            status_code,
            response.reason_phrase()
        ),
    })
}

/// Extract the first WWW-Authenticate Digest challenge from a response
fn extract_www_authenticate_challenge(
    response: &rvoip_sip_core::Response,
) -> Result<Challenge> {
    // Try to find WWW-Authenticate header
    if let Some(TypedHeader::WwwAuthenticate(www_auth)) = response.header(&HeaderName::WwwAuthenticate) {
        if let Some(digest) = www_auth.first_digest() {
            return Ok(digest.clone());
        }
        return Err(SessionError::ProtocolError {
            message: "WWW-Authenticate header present but no Digest challenge found".to_string(),
        });
    }

    Err(SessionError::ProtocolError {
        message: "401 response missing WWW-Authenticate header".to_string(),
    })
}

/// Extract the first Proxy-Authenticate Digest challenge from a response
fn extract_proxy_authenticate_challenge(
    response: &rvoip_sip_core::Response,
) -> Result<Challenge> {
    if let Some(TypedHeader::ProxyAuthenticate(proxy_auth)) = response.header(&HeaderName::ProxyAuthenticate) {
        if let Some(digest) = proxy_auth.first_digest() {
            return Ok(digest.clone());
        }
        return Err(SessionError::ProtocolError {
            message: "Proxy-Authenticate header present but no Digest challenge found".to_string(),
        });
    }

    Err(SessionError::ProtocolError {
        message: "407 response missing Proxy-Authenticate header".to_string(),
    })
}

/// Start a background refresh task for a managed registration.
///
/// The task will re-REGISTER at ~85% of the expiry interval.
/// It stops when the shutdown signal is received.
pub(crate) fn spawn_refresh_task(
    coordinator: Arc<SessionCoordinator>,
    registration: Arc<ManagedRegistration>,
) {
    let config = registration.config.clone();
    let state_tx = registration.state_tx.clone();
    let reg_ref = registration.clone();

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);

    // Store shutdown sender so we can signal it later
    let reg_for_store = registration.clone();
    tokio::spawn(async move {
        *reg_for_store.shutdown_tx.write().await = Some(shutdown_tx);
    });

    tokio::spawn(async move {
        // Compute refresh interval: 85% of expires
        let refresh_secs = (config.expires as f64 * 0.85) as u64;
        let refresh_interval = Duration::from_secs(refresh_secs.max(30)); // minimum 30s

        // Retry configuration
        const MAX_CONSECUTIVE_FAILURES: u32 = 5;
        const MAX_BACKOFF_SECS: u64 = 30;
        let mut consecutive_failures: u32 = 0;

        tracing::info!(
            registrar = %config.registrar_uri,
            expires = config.expires,
            refresh_interval_secs = refresh_interval.as_secs(),
            "Registration refresh task started"
        );

        loop {
            tokio::select! {
                _ = tokio::time::sleep(refresh_interval) => {
                    // Time to refresh
                    tracing::debug!(
                        registrar = %config.registrar_uri,
                        "Refreshing registration"
                    );

                    if let Err(e) = state_tx.send(RegistrationState::Refreshing) {
                        tracing::debug!("Failed to send registration state update (receiver dropped): {e}");
                    }

                    match send_register_with_auth(&coordinator, &config, config.expires).await {
                        Ok(new_handle) => {
                            if consecutive_failures > 0 {
                                tracing::info!(
                                    registrar = %config.registrar_uri,
                                    previous_failures = consecutive_failures,
                                    "Registration refresh recovered after failures"
                                );
                            }
                            consecutive_failures = 0;
                            *reg_ref.handle.write().await = Some(new_handle);
                            if let Err(e) = state_tx.send(RegistrationState::Active) {
                                tracing::debug!("Failed to send registration Active state (receiver dropped): {e}");
                            }
                        }
                        Err(e) => {
                            consecutive_failures += 1;
                            tracing::error!(
                                registrar = %config.registrar_uri,
                                error = %e,
                                attempt = consecutive_failures,
                                max_attempts = MAX_CONSECUTIVE_FAILURES,
                                "Registration refresh failed"
                            );

                            // After max consecutive failures, transition to Failed and stop
                            if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                                tracing::error!(
                                    registrar = %config.registrar_uri,
                                    consecutive_failures = consecutive_failures,
                                    "Registration permanently failed after {} consecutive refresh failures",
                                    consecutive_failures
                                );
                                if let Err(e2) = state_tx.send(RegistrationState::Failed(
                                    format!("Refresh failed {} consecutive times: {}", consecutive_failures, e)
                                )) {
                                    tracing::debug!("Failed to send registration Failed state (receiver dropped): {e2}");
                                }
                                break;
                            }

                            // Exponential backoff: 1s, 2s, 4s, 8s, ... capped at MAX_BACKOFF_SECS
                            let backoff_secs = (1u64 << (consecutive_failures - 1)).min(MAX_BACKOFF_SECS);
                            let backoff = Duration::from_secs(backoff_secs);

                            if let Err(e2) = state_tx.send(RegistrationState::Refreshing) {
                                tracing::debug!("Failed to send registration Refreshing state (receiver dropped): {e2}");
                            }

                            tracing::info!(
                                registrar = %config.registrar_uri,
                                backoff_secs = backoff_secs,
                                attempt = consecutive_failures,
                                "Retrying registration refresh after backoff"
                            );

                            // Wait for backoff, but allow shutdown to interrupt
                            tokio::select! {
                                _ = tokio::time::sleep(backoff) => {}
                                _ = shutdown_rx.changed() => {
                                    if *shutdown_rx.borrow() {
                                        tracing::info!(
                                            registrar = %config.registrar_uri,
                                            "Registration refresh task shutting down during retry backoff"
                                        );
                                        break;
                                    }
                                }
                            }

                            // Retry after backoff
                            match send_register_with_auth(&coordinator, &config, config.expires).await {
                                Ok(new_handle) => {
                                    tracing::info!(
                                        registrar = %config.registrar_uri,
                                        attempt = consecutive_failures,
                                        "Registration refresh retry succeeded"
                                    );
                                    consecutive_failures = 0;
                                    *reg_ref.handle.write().await = Some(new_handle);
                                    if let Err(e) = state_tx.send(RegistrationState::Active) {
                                        tracing::debug!("Failed to send registration Active state (receiver dropped): {e}");
                                    }
                                }
                                Err(retry_err) => {
                                    tracing::warn!(
                                        registrar = %config.registrar_uri,
                                        error = %retry_err,
                                        attempt = consecutive_failures,
                                        "Registration refresh retry also failed, will try again at next interval"
                                    );
                                    if let Err(e2) = state_tx.send(RegistrationState::Expired) {
                                        tracing::debug!("Failed to send registration Expired state (receiver dropped): {e2}");
                                    }
                                    // Continue the loop; next iteration will sleep for refresh_interval
                                    // then try again, incrementing consecutive_failures if it fails
                                }
                            }
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        tracing::info!(
                            registrar = %config.registrar_uri,
                            "Registration refresh task shutting down"
                        );
                        break;
                    }
                }
            }
        }
    });
}

/// Stop the refresh task for a managed registration
pub(crate) async fn stop_refresh_task(registration: &ManagedRegistration) {
    if let Some(tx) = registration.shutdown_tx.write().await.take() {
        if let Err(e) = tx.send(true) {
            tracing::debug!("Failed to send shutdown signal for registration refresh (receiver dropped): {e}");
        }
    }
}

/// Perform a full registration flow: REGISTER + auth + start refresh
pub(crate) async fn register_managed(
    coordinator: &Arc<SessionCoordinator>,
    config: RegistrationConfig,
) -> Result<Arc<ManagedRegistration>> {
    let registration = Arc::new(ManagedRegistration::new(config.clone()));
    if let Err(e) = registration.state_tx.send(RegistrationState::Registering) {
        tracing::debug!("Failed to send registration Registering state (receiver dropped): {e}");
    }

    match send_register_with_auth(coordinator, &config, config.expires).await {
        Ok(handle) => {
            *registration.handle.write().await = Some(handle);
            if let Err(e) = registration.state_tx.send(RegistrationState::Active) {
                tracing::debug!("Failed to send registration Active state (receiver dropped): {e}");
            }

            // Start refresh task
            spawn_refresh_task(coordinator.clone(), registration.clone());

            Ok(registration)
        }
        Err(e) => {
            if let Err(e2) = registration.state_tx.send(RegistrationState::Failed(e.to_string())) {
                tracing::debug!("Failed to send registration Failed state (receiver dropped): {e2}");
            }
            Err(e)
        }
    }
}

/// Send an unregister request (REGISTER with Expires: 0)
pub(crate) async fn unregister_managed(
    coordinator: &Arc<SessionCoordinator>,
    registration: &ManagedRegistration,
) -> Result<()> {
    // Stop refresh task first
    stop_refresh_task(registration).await;
    if let Err(e) = registration.state_tx.send(RegistrationState::Unregistering) {
        tracing::debug!("Failed to send registration Unregistering state (receiver dropped): {e}");
    }

    // Send REGISTER with Expires: 0
    let unregister_config = registration.config.clone();
    match send_register_with_auth(coordinator, &unregister_config, 0).await {
        Ok(_) => {
            tracing::info!(
                registrar = %unregister_config.registrar_uri,
                "Unregistered successfully"
            );
            *registration.handle.write().await = None;
            if let Err(e) = registration.state_tx.send(RegistrationState::Unregistered) {
                tracing::debug!("Failed to send registration Unregistered state (receiver dropped): {e}");
            }
            Ok(())
        }
        Err(e) => {
            tracing::warn!(
                registrar = %unregister_config.registrar_uri,
                error = %e,
                "Unregister request failed (best-effort)"
            );
            // Even if unregister fails, mark as unregistered since we stopped refresh
            *registration.handle.write().await = None;
            if let Err(e) = registration.state_tx.send(RegistrationState::Unregistered) {
                tracing::debug!("Failed to send registration Unregistered state (receiver dropped): {e}");
            }
            Ok(())
        }
    }
}
