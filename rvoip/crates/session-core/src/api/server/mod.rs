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
    media::{MediaManager, MediaConfig},
    Error, SessionId, Session,
    
    // **NEW**: Import bridge types for multi-session bridging
    session::bridge::{
        SessionBridge, BridgeId, BridgeState, BridgeInfo, BridgeConfig,
        BridgeEvent, BridgeEventType, BridgeStats, BridgeError
    },
};
use rvoip_dialog_core::api::DialogServer;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{RwLock, mpsc};
use rvoip_sip_core::{Request, Response, StatusCode, Uri};
use async_trait::async_trait;
use std::net::SocketAddr;
use anyhow::{Result, Context};
use tracing::warn;

use crate::session::SessionState;

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
    /// 
    /// **ARCHITECTURE**: Server receives DialogServer via dependency injection
    /// and coordinates with dialog-core for SIP protocol handling.
    pub async fn new(
        dialog_manager: Arc<DialogServer>,
        config: ServerConfig
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let event_bus = EventBus::new(1000).await?;
        
        // Convert new ServerConfig to old SessionConfig for compatibility
        let session_config = SessionConfig {
            local_media_addr: "127.0.0.1:0".parse().unwrap(), // Default media address
            ..Default::default()
        };
        
        let session_manager = SessionManager::new(
            dialog_manager,
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
    /// 
    /// **ARCHITECTURE**: Server receives DialogServer via dependency injection
    /// and coordinates with dialog-core for SIP protocol handling.
    pub fn new_sync(
        dialog_manager: Arc<DialogServer>,
        config: ServerConfig
    ) -> Self {
        let event_bus = EventBus::new_simple(1000);
        
        // Convert new ServerConfig to old SessionConfig for compatibility
        let session_config = SessionConfig {
            local_media_addr: "127.0.0.1:0".parse().unwrap(), // Default media address
            ..Default::default()
        };
        
        let session_manager = SessionManager::new_sync(
            dialog_manager,
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
    dialog_manager: Arc<DialogServer>,
    config: ServerConfig
) -> Result<Arc<SessionManager>, Box<dyn std::error::Error>> {
    let server_manager = ServerSessionManager::new(dialog_manager, config).await?;
    Ok(server_manager.session_manager().clone())
}

/// Create a session manager configured for server use (synchronous)
pub fn create_server_session_manager_sync(
    dialog_manager: Arc<DialogServer>,
    config: ServerConfig
) -> Arc<SessionManager> {
    let server_manager = ServerSessionManager::new_sync(dialog_manager, config);
    server_manager.session_manager().clone()
}

/// Create a full-featured server session manager
pub async fn create_full_server_manager(
    dialog_manager: Arc<DialogServer>,
    config: ServerConfig
) -> Result<Arc<ServerSessionManager>, Box<dyn std::error::Error>> {
    let server_manager = ServerSessionManager::new(dialog_manager, config).await?;
    Ok(Arc::new(server_manager))
}

/// Create a full-featured server session manager (synchronous)
pub fn create_full_server_manager_sync(
    dialog_manager: Arc<DialogServer>,
    config: ServerConfig
) -> Arc<ServerSessionManager> {
    let server_manager = ServerSessionManager::new_sync(dialog_manager, config);
    Arc::new(server_manager)
}

/// Incoming call notification event
#[derive(Debug, Clone)]
pub struct IncomingCallEvent {
    /// The session ID created for this call
    pub session_id: SessionId,
    
    /// The original INVITE request
    pub request: Request,
    
    /// Source address of the INVITE
    pub source: SocketAddr,
    
    /// Caller information extracted from the request
    pub caller_info: CallerInfo,
    
    /// SDP offer (if present in the INVITE)
    pub sdp_offer: Option<String>,
}

/// Caller information extracted from SIP headers
#[derive(Debug, Clone)]
pub struct CallerInfo {
    /// From header (caller identity)
    pub from: String,
    
    /// To header (called party)
    pub to: String,
    
    /// Call-ID header
    pub call_id: String,
    
    /// Contact header (if present)
    pub contact: Option<String>,
    
    /// User-Agent header (if present)  
    pub user_agent: Option<String>,
}

impl CallerInfo {
    /// Extract caller information from a SIP request
    pub fn from_request(request: &Request, source: SocketAddr) -> Self {
        // **CLEAN API**: Delegate SIP parsing to underlying layers
        // API layer should provide clean abstractions, not parse SIP directly
        
        let from = request.from()
            .map(|h| h.to_string())
            .unwrap_or_else(|| "unknown".to_string());
            
        let to = request.to()
            .map(|h| h.to_string())
            .unwrap_or_else(|| "unknown".to_string());
            
        let call_id = request.call_id()
            .map(|h| h.to_string())
            .unwrap_or_else(|| "unknown".to_string());
            
        // **SIMPLIFIED**: Basic contact info without complex SIP parsing
        let contact = Some(format!("sip:user@{}", source.ip()));
        
        // **CLEAN DELEGATION**: Get user agent through simple method
        let user_agent = request.header(&rvoip_sip_core::HeaderName::UserAgent)
            .and_then(|h| match h {
                rvoip_sip_core::TypedHeader::UserAgent(ua) => {
                    // UserAgent contains Vec<String>, join them with space
                    if ua.is_empty() {
                        None
                    } else {
                        Some(ua.join(" "))
                    }
                },
                _ => None,
            });
        
        Self {
            from,
            to,
            call_id,
            contact,
            user_agent,
        }
    }
}

/// Call decision result from ServerManager policy
#[derive(Debug, Clone)]
pub enum CallDecision {
    /// Accept the call
    Accept,
    
    /// Reject the call with a specific status code and reason
    Reject { status_code: StatusCode, reason: Option<String> },
    
    /// Defer the decision (keep ringing, decide later)
    Defer,
}

/// Registration decision result from ServerManager policy
#[derive(Debug, Clone)]
pub enum RegistrationDecision {
    /// Accept the registration
    Accept,
    
    /// Reject the registration with a specific status code and reason
    Reject { status_code: StatusCode, reason: Option<String> },
}

/// Notification trait for ServerManager to receive call events
#[async_trait]
pub trait IncomingCallNotification: Send + Sync {
    /// Called when a new incoming call is received
    async fn on_incoming_call(&self, event: IncomingCallEvent) -> CallDecision;
    
    /// Called when a call is terminated by the remote party (BYE received)
    async fn on_call_terminated_by_remote(&self, session_id: SessionId, call_id: String);
    
    /// Called when a call is ended by the server
    async fn on_call_ended_by_server(&self, session_id: SessionId, call_id: String);
    
    /// **NEW**: Called when a SIP REGISTER request is received
    async fn on_user_registration_request(&self, registration: UserRegistration) -> RegistrationDecision;
    
    /// **NEW**: Called when a user unregisters (REGISTER with Expires: 0)
    async fn on_user_unregistered(&self, user_uri: Uri);
}

// ========================================================================================
// BRIDGE MANAGEMENT APIs - FOR CALL-ENGINE ORCHESTRATION
// ========================================================================================

impl ServerSessionManager {
    /// **CALL-ENGINE API**: Create a new session bridge
    /// 
    /// Creates a bridge for connecting multiple sessions. Call-engine uses this
    /// to set up audio routing between UACs.
    pub async fn create_bridge(&self, config: BridgeConfig) -> Result<BridgeId, BridgeError> {
        // Check server capacity before creating bridge
        let active_sessions = self.session_manager.list_sessions();
        if active_sessions.len() + config.max_sessions > self.config.max_sessions {
            return Err(BridgeError::Internal {
                message: format!(
                    "Creating bridge would exceed server capacity ({} + {} > {})",
                    active_sessions.len(), config.max_sessions, self.config.max_sessions
                ),
            });
        }
        
        self.session_manager.create_bridge(config).await
    }
    
    /// **CALL-ENGINE API**: Destroy a session bridge
    /// 
    /// Removes all sessions from the bridge and cleans up resources.
    pub async fn destroy_bridge(&self, bridge_id: &BridgeId) -> Result<(), BridgeError> {
        self.session_manager.destroy_bridge(bridge_id).await
    }
    
    /// **CALL-ENGINE API**: Add a session to a bridge
    /// 
    /// Connects a session to a bridge for audio routing. This is the core
    /// API that call-engine uses to bridge calls together.
    pub async fn add_session_to_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<(), BridgeError> {
        self.session_manager.add_session_to_bridge(bridge_id, session_id).await
    }
    
    /// **CALL-ENGINE API**: Remove a session from a bridge
    /// 
    /// Disconnects a session from its bridge.
    pub async fn remove_session_from_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<(), BridgeError> {
        self.session_manager.remove_session_from_bridge(bridge_id, session_id).await
    }
    
    /// **CALL-ENGINE API**: Get bridge information
    /// 
    /// Returns detailed information about a bridge for monitoring and management.
    pub async fn get_bridge_info(&self, bridge_id: &BridgeId) -> Result<BridgeInfo, BridgeError> {
        self.session_manager.get_bridge_info(bridge_id).await
    }
    
    /// **CALL-ENGINE API**: List all active bridges
    /// 
    /// Returns information about all bridges for overview and management.
    pub async fn list_bridges(&self) -> Vec<BridgeInfo> {
        self.session_manager.list_bridges().await
    }
    
    /// **CALL-ENGINE API**: Pause a bridge
    /// 
    /// Stops audio flow through the bridge while keeping sessions connected.
    /// Useful for call hold scenarios.
    pub async fn pause_bridge(&self, bridge_id: &BridgeId) -> Result<(), BridgeError> {
        self.session_manager.pause_bridge(bridge_id).await
    }
    
    /// **CALL-ENGINE API**: Resume a bridge
    /// 
    /// Resumes audio flow through a paused bridge.
    pub async fn resume_bridge(&self, bridge_id: &BridgeId) -> Result<(), BridgeError> {
        self.session_manager.resume_bridge(bridge_id).await
    }
    
    /// **CALL-ENGINE API**: Get bridge for a session
    /// 
    /// Returns the bridge ID that a session is currently in (if any).
    pub async fn get_session_bridge(&self, session_id: &SessionId) -> Option<BridgeId> {
        self.session_manager.get_session_bridge(session_id).await
    }
    
    /// **CALL-ENGINE API**: Subscribe to bridge events
    /// 
    /// Allows call-engine to receive real-time bridge event notifications.
    /// This is essential for orchestration and monitoring.
    pub async fn subscribe_to_bridge_events(&self) -> mpsc::UnboundedReceiver<BridgeEvent> {
        self.session_manager.subscribe_to_bridge_events().await
    }
    
    /// **CALL-ENGINE API**: Get bridge statistics
    /// 
    /// Returns aggregated statistics across all bridges for monitoring.
    pub async fn get_bridge_statistics(&self) -> HashMap<BridgeId, BridgeStats> {
        self.session_manager.get_bridge_statistics().await
    }
    
    /// **CALL-ENGINE API**: Bridge two specific sessions
    /// 
    /// High-level convenience method that creates a bridge and adds two sessions.
    /// This is a common pattern for simple call bridging.
    pub async fn bridge_sessions(&self, session_a: &SessionId, session_b: &SessionId) -> Result<BridgeId, BridgeError> {
        // Create a simple 2-party bridge
        let config = BridgeConfig {
            max_sessions: 2,
            name: Some(format!("Bridge: {} ↔ {}", session_a, session_b)),
            ..Default::default()
        };
        
        let bridge_id = self.create_bridge(config).await?;
        
        // Add both sessions
        if let Err(e) = self.add_session_to_bridge(&bridge_id, session_a).await {
            // Clean up bridge on failure
            let _ = self.destroy_bridge(&bridge_id).await;
            return Err(e);
        }
        
        if let Err(e) = self.add_session_to_bridge(&bridge_id, session_b).await {
            // Clean up bridge on failure
            let _ = self.destroy_bridge(&bridge_id).await;
            return Err(e);
        }
        
        Ok(bridge_id)
    }
    
    /// **CALL-ENGINE API**: Auto-bridge available sessions
    /// 
    /// Automatically finds unbridged sessions and bridges them together.
    /// This is useful for simple auto-attendant scenarios.
    pub async fn auto_bridge_available_sessions(&self) -> Result<Vec<BridgeId>, BridgeError> {
        let mut created_bridges = Vec::new();
        
        // Get all active sessions
        let sessions = self.session_manager.list_sessions();
        let mut unbridged_sessions = Vec::new();
        
        // Find sessions that aren't in bridges
        for session in sessions {
            if self.get_session_bridge(&session.id).await.is_none() {
                unbridged_sessions.push(session.id.clone());
            }
        }
        
        // Bridge sessions in pairs
        while unbridged_sessions.len() >= 2 {
            let session_a = unbridged_sessions.remove(0);
            let session_b = unbridged_sessions.remove(0);
            
            match self.bridge_sessions(&session_a, &session_b).await {
                Ok(bridge_id) => {
                    created_bridges.push(bridge_id);
                },
                Err(e) => {
                    warn!("Failed to auto-bridge sessions {} and {}: {}", session_a, session_b, e);
                }
            }
        }
        
        Ok(created_bridges)
    }
} 