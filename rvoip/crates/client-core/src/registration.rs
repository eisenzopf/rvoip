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
use tracing::{info, debug, warn, error};

use rvoip_transaction_core::TransactionManager;
use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri, HeaderName, TypedHeader};
use rvoip_sip_core::types::headers::HeaderAccess;
use rvoip_sip_core::types::{Via, CallId, CSeq, MaxForwards, From, To, Contact, Expires, ContactParamInfo};

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
    /// Manager state
    is_running: Arc<RwLock<bool>>,
    /// Background task for handling registration refresh
    refresh_task_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl RegistrationManager {
    /// Create a new registration manager
    pub fn new(transaction_manager: Arc<TransactionManager>) -> Self {
        Self {
            registrations: Arc::new(RwLock::new(HashMap::new())),
            transaction_manager,
            is_running: Arc::new(RwLock::new(false)),
            refresh_task_handle: Arc::new(RwLock::new(None)),
        }
    }

    /// Start the registration manager
    pub async fn start(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if *running {
            return Ok(());
        }

        info!("▶️ Starting RegistrationManager");

        // Start background refresh task
        self.start_refresh_task().await;

        *running = true;
        info!("✅ RegistrationManager started");
        Ok(())
    }

    /// Stop the registration manager
    pub async fn stop(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if !*running {
            return Ok(());
        }

        info!("🛑 Stopping RegistrationManager");

        // Stop refresh task
        if let Some(handle) = self.refresh_task_handle.write().await.take() {
            handle.abort();
        }

        // Unregister all active registrations
        let server_uris: Vec<String> = {
            let registrations = self.registrations.read().await;
            registrations.keys().cloned().collect()
        };

        for server_uri in server_uris {
            if let Err(e) = self.unregister(&server_uri).await {
                warn!("Failed to unregister from {}: {}", server_uri, e);
            }
        }

        *running = false;
        info!("✅ RegistrationManager stopped");
        Ok(())
    }

    /// Start background task for registration refresh
    async fn start_refresh_task(&self) {
        let registrations = Arc::clone(&self.registrations);
        let is_running = Arc::clone(&self.is_running);

        let handle = tokio::spawn(async move {
            debug!("🔄 Starting registration refresh task");

            while *is_running.read().await {
                // Check for expired registrations every 30 seconds
                tokio::time::sleep(Duration::from_secs(30)).await;

                let registrations_guard = registrations.read().await;
                let expired_servers: Vec<String> = registrations_guard
                    .iter()
                    .filter(|(_, session)| session.is_expired())
                    .map(|(server_uri, _)| server_uri.clone())
                    .collect();
                drop(registrations_guard);

                for server_uri in expired_servers {
                    debug!("🔄 Refreshing expired registration for {}", server_uri);
                    // TODO: Implement refresh logic
                }
            }

            debug!("🛑 Registration refresh task ended");
        });

        let mut task_handle = self.refresh_task_handle.write().await;
        *task_handle = Some(handle);
    }

    /// Handle registration response from transaction manager
    pub async fn handle_registration_response(&self, response: Response) -> ClientResult<()> {
        debug!("📨 Handling registration response: {} {}", response.status_code(), response.reason_phrase());

        // Extract Call-ID to identify which registration this response is for
        let call_id = response.raw_header_value(&HeaderName::CallId)
            .ok_or_else(|| ClientError::protocol_error("Registration response missing Call-ID"))?;

        // Find the registration session by Call-ID (simplified - in real implementation we'd track this better)
        let mut registrations = self.registrations.write().await;
        let mut found_session = None;
        
        for (server_uri, session) in registrations.iter_mut() {
            if let Some(ref transaction_id) = session.transaction_id {
                if transaction_id == &call_id {
                    found_session = Some(server_uri.clone());
                    break;
                }
            }
        }

        if let Some(server_uri) = found_session {
            if let Some(session) = registrations.get_mut(&server_uri) {
                match response.status_code() {
                    200 => { // StatusCode::Ok
                        info!("✅ Registration successful for {}", server_uri);
                        session.update_status(RegistrationStatus::Registered);
                        session.transaction_id = None;
                        
                        // TODO: Parse Expires header to update actual expiration
                        // TODO: Set up refresh timer
                    },
                    401 | 407 => { // StatusCode::Unauthorized | StatusCode::ProxyAuthenticationRequired
                        info!("🔐 Authentication required for {}", server_uri);
                        session.update_status(RegistrationStatus::AuthenticationRequired);
                        
                        // TODO: Parse WWW-Authenticate header
                        // TODO: Send authenticated REGISTER
                    },
                    _ => {
                        warn!("❌ Registration failed for {} with status {}", server_uri, response.status_code());
                        session.update_status(RegistrationStatus::Failed);
                        session.info.last_error = Some(format!("Registration failed: {} {}", 
                                                               response.status_code(), response.reason_phrase()));
                        session.transaction_id = None;
                    }
                }
            }
        } else {
            warn!("⚠️ Received registration response for unknown Call-ID: {}", call_id);
        }

        Ok(())
    }

    /// Handle transaction timeout
    pub async fn handle_transaction_timeout(&self, transaction_key: &str) -> ClientResult<()> {
        debug!("⏰ Handling registration transaction timeout: {}", transaction_key);

        let mut registrations = self.registrations.write().await;
        let mut found_session = None;
        
        for (server_uri, session) in registrations.iter_mut() {
            if let Some(ref transaction_id) = session.transaction_id {
                if transaction_id == transaction_key {
                    found_session = Some(server_uri.clone());
                    break;
                }
            }
        }

        if let Some(server_uri) = found_session {
            if let Some(session) = registrations.get_mut(&server_uri) {
                warn!("⏰ Registration timeout for {}", server_uri);
                session.update_status(RegistrationStatus::Failed);
                session.info.last_error = Some("Registration timeout".to_string());
                session.transaction_id = None;
            }
        }

        Ok(())
    }

    /// Start registration with a server
    pub async fn register(&self, config: RegistrationConfig) -> ClientResult<Uuid> {
        let server_uri = config.server_uri.clone();
        info!("📝 Starting registration with server: {}", server_uri);
        
        let mut session = RegistrationSession::new(config.clone());
        let registration_id = session.info.registration_id;

        // Build REGISTER request
        let register_request = self.build_register_request(&config).await?;
        
        // Create client transaction
        let destination: std::net::SocketAddr = "127.0.0.1:5060".parse() // TODO: Parse from server_uri
            .map_err(|e| ClientError::protocol_error(&format!("Invalid destination address: {}", e)))?;
        
        let transaction_key = self.transaction_manager
            .create_client_transaction(register_request, destination)
            .await
            .map_err(|e| ClientError::TransactionError(e.into()))?;

        // Send the request
        if let Err(e) = self.transaction_manager.send_request(&transaction_key).await {
            warn!("Failed to send register request: {}", e);
        }

        debug!("📤 Sent REGISTER request with transaction key: {}", transaction_key);

        session.update_status(RegistrationStatus::Registering);
        session.info.last_attempt = Some(Utc::now());
        session.info.attempt_count += 1;
        session.transaction_id = Some(transaction_key.to_string());

        {
            let mut registrations = self.registrations.write().await;
            registrations.insert(server_uri, session);
        }

        Ok(registration_id)
    }

    /// Build a REGISTER request
    async fn build_register_request(&self, config: &RegistrationConfig) -> ClientResult<Request> {
        // Parse the server URI to extract host and port
        let server_uri = config.server_uri.parse::<Uri>()
            .map_err(|e| ClientError::protocol_error(&format!("Invalid server URI: {}", e)))?;
        
        let user_uri = config.user_uri.parse::<Uri>()
            .map_err(|e| ClientError::protocol_error(&format!("Invalid user URI: {}", e)))?;

        // Create basic REGISTER request
        let mut request = Request::new(Method::Register, server_uri.clone());
        
        // Required headers for REGISTER - use typed headers
        let from_addr = rvoip_sip_core::types::Address::new(user_uri.clone());
        let to_addr = rvoip_sip_core::types::Address::new(user_uri);
        
        request = request
            .with_header(TypedHeader::From(From::new(from_addr)))
            .with_header(TypedHeader::To(To::new(to_addr)))
            .with_header(TypedHeader::CallId(CallId::new(&uuid::Uuid::new_v4().to_string())))
            .with_header(TypedHeader::CSeq(CSeq::new(1, Method::Register)))
            .with_header(TypedHeader::Expires(Expires::new(config.expires)))
            .with_header(TypedHeader::MaxForwards(MaxForwards::new(70)));
        
        // Contact header with our address
        let contact_uri = format!("sip:{}@127.0.0.1:5060", config.username);
        let contact_addr = rvoip_sip_core::types::Address::new(contact_uri.parse::<Uri>()
            .map_err(|e| ClientError::protocol_error(&format!("Invalid contact URI: {}", e)))?);
        
        // Create contact with address
        let contact_param = rvoip_sip_core::types::ContactParamInfo {
            address: contact_addr,
        };
        
        request = request.with_header(TypedHeader::Contact(Contact::new_params(vec![contact_param])));

        debug!("🔨 Built REGISTER request for {}", config.server_uri);
        Ok(request)
    }

    /// Unregister from a server
    pub async fn unregister(&self, server_uri: &str) -> ClientResult<()> {
        info!("📤 Unregistering from server: {}", server_uri);
        
        let mut registrations = self.registrations.write().await;
        if let Some(mut session) = registrations.remove(server_uri) {
            // Cancel refresh timer
            if let Some(timer) = session.refresh_timer.take() {
                timer.abort();
            }

            // Send REGISTER with Expires: 0 to unregister
            let mut unregister_config = session.config.clone();
            unregister_config.expires = 0;
            
            if let Ok(unregister_request) = self.build_register_request(&unregister_config).await {
                let destination: std::net::SocketAddr = "127.0.0.1:5060".parse()
                    .map_err(|e| ClientError::protocol_error(&format!("Invalid destination: {}", e)))?;
                    
                if let Ok(transaction_key) = self.transaction_manager
                    .create_client_transaction(unregister_request, destination).await {
                    if let Err(e) = self.transaction_manager.send_request(&transaction_key).await {
                        warn!("Failed to send unregister request: {}", e);
                    }
                }
            }

            session.update_status(RegistrationStatus::Unregistered);
            info!("✅ Unregistered from {}", server_uri);
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