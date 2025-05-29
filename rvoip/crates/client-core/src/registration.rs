//! SIP registration management for client
//!
//! This module handles SIP registration with servers, including authentication,
//! registration refresh, and server connectivity management.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use rvoip_transaction_core::TransactionManager;

use crate::error::{ClientResult, ClientError};
use crate::events::Credentials;

/// Current registration status
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistrationStatus {
    /// Not registered
    Unregistered,
    /// Registration in progress
    Registering,
    /// Successfully registered
    Registered,
    /// Registration failed
    Failed,
    /// Registration expired and needs renewal
    Expired,
    /// Authentication required
    AuthenticationRequired,
}

/// Configuration for SIP registration
#[derive(Debug, Clone)]
pub struct RegistrationConfig {
    /// SIP server URI (e.g., sip:example.com)
    pub server_uri: String,
    /// User URI (e.g., sip:user@example.com)
    pub user_uri: String,
    /// Username for authentication
    pub username: String,
    /// Password for authentication
    pub password: String,
    /// Registration expiration time in seconds (default: 3600)
    pub expires: u32,
    /// How often to refresh registration (default: 80% of expires)
    pub refresh_interval: Option<Duration>,
    /// Additional contact parameters
    pub contact_params: HashMap<String, String>,
}

impl RegistrationConfig {
    /// Create a new registration configuration
    pub fn new(
        server_uri: String,
        user_uri: String,
        username: String,
        password: String,
    ) -> Self {
        Self {
            server_uri,
            user_uri,
            username,
            password,
            expires: 3600, // 1 hour default
            refresh_interval: None,
            contact_params: HashMap::new(),
        }
    }

    /// Set custom expiration time
    pub fn with_expires(mut self, expires: u32) -> Self {
        self.expires = expires;
        self
    }

    /// Set custom refresh interval
    pub fn with_refresh_interval(mut self, interval: Duration) -> Self {
        self.refresh_interval = Some(interval);
        self
    }

    /// Add contact parameter
    pub fn with_contact_param(mut self, key: String, value: String) -> Self {
        self.contact_params.insert(key, value);
        self
    }

    /// Get the effective refresh interval
    pub fn get_refresh_interval(&self) -> Duration {
        self.refresh_interval
            .unwrap_or_else(|| Duration::from_secs((self.expires as f64 * 0.8) as u64))
    }
}

/// Information about a registration session
#[derive(Debug, Clone)]
pub struct RegistrationInfo {
    /// Registration ID
    pub registration_id: Uuid,
    /// Server we're registering with
    pub server_uri: String,
    /// Our user URI
    pub user_uri: String,
    /// Current status
    pub status: RegistrationStatus,
    /// When registration was last attempted
    pub last_attempt: Option<DateTime<Utc>>,
    /// When registration was successful
    pub registered_at: Option<DateTime<Utc>>,
    /// When registration expires
    pub expires_at: Option<DateTime<Utc>>,
    /// Number of registration attempts
    pub attempt_count: u32,
    /// Last error (if any)
    pub last_error: Option<String>,
    /// Contact URI we registered with
    pub contact_uri: Option<String>,
}

/// Internal registration session
#[derive(Debug)]
struct RegistrationSession {
    /// Basic registration info
    pub info: RegistrationInfo,
    /// Configuration
    pub config: RegistrationConfig,
    /// Current transaction ID (if registering)
    pub transaction_id: Option<String>,
    /// Authentication realm (from server challenge)
    pub auth_realm: Option<String>,
    /// Authentication nonce (from server challenge)
    pub auth_nonce: Option<String>,
    /// Registration refresh timer
    pub refresh_timer: Option<tokio::task::JoinHandle<()>>,
}

impl RegistrationSession {
    /// Create a new registration session
    fn new(config: RegistrationConfig) -> Self {
        let registration_id = Uuid::new_v4();
        
        let info = RegistrationInfo {
            registration_id,
            server_uri: config.server_uri.clone(),
            user_uri: config.user_uri.clone(),
            status: RegistrationStatus::Unregistered,
            last_attempt: None,
            registered_at: None,
            expires_at: None,
            attempt_count: 0,
            last_error: None,
            contact_uri: None,
        };

        Self {
            info,
            config,
            transaction_id: None,
            auth_realm: None,
            auth_nonce: None,
            refresh_timer: None,
        }
    }

    /// Update registration status
    fn update_status(&mut self, new_status: RegistrationStatus) {
        self.info.status = new_status.clone();
        
        match new_status {
            RegistrationStatus::Registered => {
                self.info.registered_at = Some(Utc::now());
                self.info.expires_at = Some(
                    Utc::now() + chrono::Duration::seconds(self.config.expires as i64)
                );
                self.info.last_error = None;
            }
            RegistrationStatus::Failed | RegistrationStatus::Expired => {
                self.info.registered_at = None;
                self.info.expires_at = None;
            }
            _ => {}
        }
    }

    /// Check if registration is expired
    fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.info.expires_at {
            Utc::now() > expires_at
        } else {
            false
        }
    }
}

/// Registration manager handles SIP registrations
pub struct RegistrationManager {
    /// Active registrations indexed by server URI
    registrations: Arc<RwLock<HashMap<String, RegistrationSession>>>,
    /// Reference to transaction manager (reused from infrastructure)
    transaction_manager: Arc<TransactionManager>,
}

impl RegistrationManager {
    /// Create a new registration manager
    pub fn new(transaction_manager: Arc<TransactionManager>) -> Self {
        Self {
            registrations: Arc::new(RwLock::new(HashMap::new())),
            transaction_manager,
        }
    }

    /// Start registration with a server
    pub async fn register(&self, config: RegistrationConfig) -> ClientResult<Uuid> {
        let server_uri = config.server_uri.clone();
        let mut session = RegistrationSession::new(config);
        let registration_id = session.info.registration_id;

        // TODO: Implement actual registration logic
        // 1. Build REGISTER request
        // 2. Send via transaction_manager
        // 3. Handle responses (including authentication challenges)
        // 4. Set up refresh timer

        session.update_status(RegistrationStatus::Registering);
        session.info.last_attempt = Some(Utc::now());
        session.info.attempt_count += 1;

        {
            let mut registrations = self.registrations.write().await;
            registrations.insert(server_uri, session);
        }

        Ok(registration_id)
    }

    /// Unregister from a server
    pub async fn unregister(&self, server_uri: &str) -> ClientResult<()> {
        // TODO: Implement unregistration logic
        // 1. Send REGISTER with Expires: 0
        // 2. Cancel refresh timer
        // 3. Remove from active registrations

        let mut registrations = self.registrations.write().await;
        if let Some(mut session) = registrations.remove(server_uri) {
            // Cancel refresh timer
            if let Some(timer) = session.refresh_timer.take() {
                timer.abort();
            }
            session.update_status(RegistrationStatus::Unregistered);
            Ok(())
        } else {
            Err(ClientError::registration_failed("No registration found for server"))
        }
    }

    /// Get registration status for a server
    pub async fn get_registration_status(&self, server_uri: &str) -> Option<RegistrationInfo> {
        let registrations = self.registrations.read().await;
        registrations.get(server_uri).map(|session| session.info.clone())
    }

    /// Check if registered with a server
    pub async fn is_registered(&self, server_uri: &str) -> bool {
        let registrations = self.registrations.read().await;
        registrations
            .get(server_uri)
            .map(|session| session.info.status == RegistrationStatus::Registered && !session.is_expired())
            .unwrap_or(false)
    }

    /// List all registrations
    pub async fn list_registrations(&self) -> Vec<RegistrationInfo> {
        let registrations = self.registrations.read().await;
        registrations.values().map(|session| session.info.clone()).collect()
    }

    /// Refresh a specific registration
    pub async fn refresh_registration(&self, server_uri: &str) -> ClientResult<()> {
        // TODO: Implement registration refresh
        // 1. Check if registration exists
        // 2. Send new REGISTER request
        // 3. Handle authentication if needed
        // 4. Update expiration time

        let registrations = self.registrations.read().await;
        if registrations.contains_key(server_uri) {
            // Implementation placeholder
            Ok(())
        } else {
            Err(ClientError::registration_failed("No registration found for server"))
        }
    }

    /// Handle authentication challenge from server
    pub async fn handle_auth_challenge(
        &self,
        server_uri: &str,
        realm: String,
        nonce: String,
        _credentials: Credentials,
    ) -> ClientResult<()> {
        // TODO: Implement digest authentication
        // 1. Calculate digest response
        // 2. Send authenticated REGISTER
        // 3. Update registration status

        let mut registrations = self.registrations.write().await;
        if let Some(session) = registrations.get_mut(server_uri) {
            session.auth_realm = Some(realm);
            session.auth_nonce = Some(nonce);
            session.update_status(RegistrationStatus::Registering);
            Ok(())
        } else {
            Err(ClientError::registration_failed("No registration found for server"))
        }
    }

    /// Check for expired registrations and refresh them
    pub async fn check_expired_registrations(&self) -> ClientResult<()> {
        let registrations = self.registrations.read().await;
        let expired_servers: Vec<String> = registrations
            .iter()
            .filter(|(_, session)| session.is_expired())
            .map(|(server_uri, _)| server_uri.clone())
            .collect();

        drop(registrations);

        for server_uri in expired_servers {
            if let Err(e) = self.refresh_registration(&server_uri).await {
                tracing::warn!("Failed to refresh expired registration for {}: {}", server_uri, e);
            }
        }

        Ok(())
    }

    /// Get registration statistics
    pub async fn get_registration_stats(&self) -> RegistrationStats {
        let registrations = self.registrations.read().await;
        let total = registrations.len();
        let registered = registrations
            .values()
            .filter(|s| s.info.status == RegistrationStatus::Registered)
            .count();
        let failed = registrations
            .values()
            .filter(|s| s.info.status == RegistrationStatus::Failed)
            .count();

        RegistrationStats {
            total_registrations: total,
            active_registrations: registered,
            failed_registrations: failed,
        }
    }
}

/// Statistics about registrations
#[derive(Debug, Clone)]
pub struct RegistrationStats {
    pub total_registrations: usize,
    pub active_registrations: usize,
    pub failed_registrations: usize,
} 