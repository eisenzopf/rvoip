//! Server API for RVOIP Session Core
//!
//! This module provides server-specific functionality for building SIP servers,
//! including factory functions, configuration, and server-oriented helper methods.

pub mod config;
pub mod manager;

// Re-export the new config types
pub use config::{ServerConfig, TransportProtocol};

// Re-export the new manager types
pub use manager::ServerManager;

use crate::{
    session::{SessionManager, SessionConfig, SessionDirection},
    events::{EventBus, SessionEvent},
    Error, SessionId, Session
};
use std::sync::Arc;
use std::collections::HashMap;
use rvoip_transaction_core::TransactionManager;
use rvoip_sip_core::{Request, Response, Uri};
use tokio::sync::RwLock;

/// Legacy server configuration for backward compatibility
#[derive(Debug, Clone)]
pub struct LegacyServerConfig {
    /// Server name
    pub server_name: String,
    
    /// Domain name
    pub domain: String,
    
    /// Maximum sessions allowed
    pub max_sessions: usize,
    
    /// Session timeout (in seconds)
    pub session_timeout: u32,
    
    /// Maximum concurrent calls per user
    pub max_calls_per_user: usize,
    
    /// Enable call routing
    pub enable_routing: bool,
    
    /// Enable call transfer
    pub enable_transfer: bool,
    
    /// Enable conference calls
    pub enable_conference: bool,
    
    /// User agent string
    pub user_agent: String,
    
    /// Session configuration
    pub session_config: SessionConfig,
}

impl Default for LegacyServerConfig {
    fn default() -> Self {
        Self {
            server_name: "RVOIP Server".to_string(),
            domain: "example.com".to_string(),
            max_sessions: 10000,
            session_timeout: 3600,
            max_calls_per_user: 5,
            enable_routing: true,
            enable_transfer: true,
            enable_conference: false,
            user_agent: "RVOIP-Server/1.0".to_string(),
            session_config: SessionConfig::default(),
        }
    }
}

/// Call routing information
#[derive(Debug, Clone)]
pub struct RouteInfo {
    /// Target URI
    pub target_uri: Uri,
    
    /// Route priority (lower is higher priority)
    pub priority: u32,
    
    /// Route weight for load balancing
    pub weight: u32,
    
    /// Route description
    pub description: String,
}

/// User registration information
#[derive(Debug, Clone)]
pub struct UserRegistration {
    /// User URI
    pub user_uri: Uri,
    
    /// Contact URI
    pub contact_uri: Uri,
    
    /// Registration expiry
    pub expires: std::time::SystemTime,
    
    /// User agent
    pub user_agent: Option<String>,
}

/// Server-specific session manager with enhanced server functionality
pub struct ServerSessionManager {
    /// Core session manager
    session_manager: Arc<SessionManager>,
    
    /// Server configuration
    config: ServerConfig,
    
    /// User registrations
    registrations: Arc<RwLock<HashMap<String, UserRegistration>>>,
    
    /// Call routing table
    routes: Arc<RwLock<HashMap<String, Vec<RouteInfo>>>>,
    
    /// Active calls per user
    user_call_counts: Arc<RwLock<HashMap<String, usize>>>,
}

impl ServerSessionManager {
    /// Create a new server session manager
    pub async fn new(
        transaction_manager: Arc<TransactionManager>,
        config: ServerConfig
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let event_bus = EventBus::new(1000).await?;
        
        // Convert new ServerConfig to old SessionConfig for compatibility
        let session_config = SessionConfig {
            local_media_addr: "127.0.0.1:0".parse().unwrap(), // Default media address
            ..Default::default()
        };
        
        let session_manager = SessionManager::new(
            transaction_manager,
            session_config,
            event_bus
        ).await?;
        
        Ok(Self {
            session_manager: Arc::new(session_manager),
            config,
            registrations: Arc::new(RwLock::new(HashMap::new())),
            routes: Arc::new(RwLock::new(HashMap::new())),
            user_call_counts: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Create a new server session manager (synchronous)
    pub fn new_sync(
        transaction_manager: Arc<TransactionManager>,
        config: ServerConfig
    ) -> Self {
        let event_bus = EventBus::new_simple(1000);
        
        // Convert new ServerConfig to old SessionConfig for compatibility
        let session_config = SessionConfig {
            local_media_addr: "127.0.0.1:0".parse().unwrap(), // Default media address
            ..Default::default()
        };
        
        let session_manager = SessionManager::new_sync(
            transaction_manager,
            session_config,
            event_bus
        );
        
        Self {
            session_manager: Arc::new(session_manager),
            config,
            registrations: Arc::new(RwLock::new(HashMap::new())),
            routes: Arc::new(RwLock::new(HashMap::new())),
            user_call_counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Get the underlying session manager
    pub fn session_manager(&self) -> &Arc<SessionManager> {
        &self.session_manager
    }
    
    /// Get the server configuration
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }
    
    /// Handle incoming call
    pub async fn handle_incoming_call(&self, request: &Request) -> Result<Arc<Session>, Error> {
        // Check server capacity
        let active_sessions = self.session_manager.list_sessions();
        if active_sessions.len() >= self.config.max_sessions {
            return Err(Error::ResourceLimitExceeded(
                format!("Maximum sessions ({}) reached", self.config.max_sessions),
                crate::errors::ErrorContext {
                    category: crate::errors::ErrorCategory::Resource,
                    severity: crate::errors::ErrorSeverity::Error,
                    recovery: crate::errors::RecoveryAction::Wait(std::time::Duration::from_secs(10)),
                    retryable: true,
                    timestamp: std::time::SystemTime::now(),
                    details: Some("Server at capacity".to_string()),
                    ..Default::default()
                }
            ));
        }
        
        // Extract user from request
        let user = self.extract_user_from_request(request)?;
        
        // Check per-user call limit - use a default since new config doesn't have this field
        let max_calls_per_user = 5; // Default limit
        let user_calls = {
            let call_counts = self.user_call_counts.read().await;
            call_counts.get(&user).copied().unwrap_or(0)
        };
        
        if user_calls >= max_calls_per_user {
            return Err(Error::ResourceLimitExceeded(
                format!("Maximum calls per user ({}) reached for {}", max_calls_per_user, user),
                crate::errors::ErrorContext {
                    category: crate::errors::ErrorCategory::Resource,
                    severity: crate::errors::ErrorSeverity::Error,
                    recovery: crate::errors::RecoveryAction::Wait(std::time::Duration::from_secs(5)),
                    retryable: true,
                    timestamp: std::time::SystemTime::now(),
                    details: Some("User at call limit".to_string()),
                    ..Default::default()
                }
            ));
        }
        
        // Create incoming session
        let session = self.session_manager.create_incoming_session().await?;
        
        // Update user call count
        {
            let mut call_counts = self.user_call_counts.write().await;
            *call_counts.entry(user).or_insert(0) += 1;
        }
        
        Ok(session)
    }
    
    /// Route a call to its destination
    pub async fn route_call(&self, session_id: &SessionId, target_uri: &Uri) -> Result<Vec<RouteInfo>, Error> {
        // For now, always allow routing since new config doesn't have enable_routing field
        let routes = self.routes.read().await;
        let target_key = target_uri.to_string();
        
        // Look for specific route
        if let Some(route_list) = routes.get(&target_key) {
            return Ok(route_list.clone());
        }
        
        // Look for domain-based route
        let domain = target_uri.host.to_string();
        if let Some(route_list) = routes.get(&domain) {
            return Ok(route_list.clone());
        }
        
        // No specific route found, return empty list
        Ok(vec![])
    }
    
    /// Transfer a call (server-side)
    pub async fn transfer_call(
        &self,
        session_id: &SessionId,
        target_uri: String,
        transfer_type: crate::session::session_types::TransferType
    ) -> Result<crate::session::session_types::TransferId, Error> {
        // For now, always allow transfer since new config doesn't have enable_transfer field
        self.session_manager.initiate_transfer(
            session_id,
            target_uri,
            transfer_type,
            Some(format!("sip:{}@{}", self.config.server_name, "localhost"))
        ).await
    }
    
    /// Register a user
    pub async fn register_user(&self, registration: UserRegistration) -> Result<(), Error> {
        let user_key = registration.user_uri.to_string();
        let mut registrations = self.registrations.write().await;
        registrations.insert(user_key, registration);
        Ok(())
    }
    
    /// Unregister a user
    pub async fn unregister_user(&self, user_uri: &Uri) -> Result<(), Error> {
        let user_key = user_uri.to_string();
        let mut registrations = self.registrations.write().await;
        registrations.remove(&user_key);
        Ok(())
    }
    
    /// Get user registration
    pub async fn get_user_registration(&self, user_uri: &Uri) -> Option<UserRegistration> {
        let user_key = user_uri.to_string();
        let registrations = self.registrations.read().await;
        registrations.get(&user_key).cloned()
    }
    
    /// Add a route
    pub async fn add_route(&self, pattern: String, route: RouteInfo) -> Result<(), Error> {
        let mut routes = self.routes.write().await;
        routes.entry(pattern).or_insert_with(Vec::new).push(route);
        Ok(())
    }
    
    /// Remove routes for a pattern
    pub async fn remove_routes(&self, pattern: &str) -> Result<(), Error> {
        let mut routes = self.routes.write().await;
        routes.remove(pattern);
        Ok(())
    }
    
    /// Get server statistics
    pub async fn get_server_stats(&self) -> ServerStats {
        let active_sessions = self.session_manager.list_sessions();
        let registrations = self.registrations.read().await;
        let user_calls = self.user_call_counts.read().await;
        
        ServerStats {
            active_sessions: active_sessions.len(),
            max_sessions: self.config.max_sessions,
            registered_users: registrations.len(),
            total_user_calls: user_calls.values().sum(),
            uptime: std::time::SystemTime::now(),
        }
    }
    
    /// Cleanup expired registrations
    pub async fn cleanup_expired_registrations(&self) -> Result<usize, Error> {
        let now = std::time::SystemTime::now();
        let mut registrations = self.registrations.write().await;
        let initial_count = registrations.len();
        
        registrations.retain(|_, reg| reg.expires > now);
        
        Ok(initial_count - registrations.len())
    }
    
    /// Extract user from SIP request
    fn extract_user_from_request(&self, request: &Request) -> Result<String, Error> {
        // Extract user from From header
        if let Some(from_header) = request.from() {
            if let Some(user) = &from_header.uri().user {
                return Ok(user.to_string());
            }
        }
        
        Err(Error::InvalidRequest(
            "Cannot extract user from request".to_string(),
            crate::errors::ErrorContext {
                category: crate::errors::ErrorCategory::Protocol,
                severity: crate::errors::ErrorSeverity::Error,
                recovery: crate::errors::RecoveryAction::None,
                retryable: false,
                timestamp: std::time::SystemTime::now(),
                details: Some("Missing or invalid From header".to_string()),
                ..Default::default()
            }
        ))
    }
}

/// Server statistics
#[derive(Debug, Clone)]
pub struct ServerStats {
    /// Number of active sessions
    pub active_sessions: usize,
    
    /// Maximum sessions allowed
    pub max_sessions: usize,
    
    /// Number of registered users
    pub registered_users: usize,
    
    /// Total active calls across all users
    pub total_user_calls: usize,
    
    /// Server uptime
    pub uptime: std::time::SystemTime,
}

/// Create a session manager configured for server use
pub async fn create_server_session_manager(
    transaction_manager: Arc<TransactionManager>,
    config: ServerConfig
) -> Result<Arc<SessionManager>, Box<dyn std::error::Error>> {
    let server_manager = ServerSessionManager::new(transaction_manager, config).await?;
    Ok(server_manager.session_manager().clone())
}

/// Create a session manager configured for server use (synchronous)
pub fn create_server_session_manager_sync(
    transaction_manager: Arc<TransactionManager>,
    config: ServerConfig
) -> Arc<SessionManager> {
    let server_manager = ServerSessionManager::new_sync(transaction_manager, config);
    server_manager.session_manager().clone()
}

/// Create a full-featured server session manager
pub async fn create_full_server_manager(
    transaction_manager: Arc<TransactionManager>,
    config: ServerConfig
) -> Result<Arc<ServerSessionManager>, Box<dyn std::error::Error>> {
    let server_manager = ServerSessionManager::new(transaction_manager, config).await?;
    Ok(Arc::new(server_manager))
}

/// Create a full-featured server session manager (synchronous)
pub fn create_full_server_manager_sync(
    transaction_manager: Arc<TransactionManager>,
    config: ServerConfig
) -> Arc<ServerSessionManager> {
    let server_manager = ServerSessionManager::new_sync(transaction_manager, config);
    Arc::new(server_manager)
} 