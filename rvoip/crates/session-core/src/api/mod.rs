//! RVOIP Session Core API
//!
//! This module provides high-level APIs for building SIP clients and servers
//! using the RVOIP session-core library. It includes factory functions,
//! configuration structures, and specialized functionality for both client
//! and server applications.
//!
//! # Client API
//!
//! The client API provides functionality for building SIP clients such as
//! softphones, SIP endpoints, and mobile applications.
//!
//! ```rust
//! use rvoip_session_core::api::client::*;
//!
//! let config = ClientConfig {
//!     display_name: "My SIP Phone".to_string(),
//!     uri: "sip:alice@example.com".to_string(),
//!     ..Default::default()
//! };
//!
//! let client = create_full_client_manager(transaction_manager, config).await?;
//! let session = client.make_call(destination_uri).await?;
//! ```
//!
//! # Server API
//!
//! The server API provides functionality for building SIP servers such as
//! PBX systems, SIP proxies, and call centers.
//!
//! ```rust
//! use rvoip_session_core::api::server::*;
//!
//! let config = ServerConfig {
//!     server_name: "My PBX".to_string(),
//!     domain: "example.com".to_string(),
//!     max_sessions: 10000,
//!     ..Default::default()
//! };
//!
//! let server = create_full_server_manager(transaction_manager, config).await?;
//! let session = server.handle_incoming_call(&request).await?;
//! ```

pub mod client;
pub mod server;

// Re-export the main types and functions for convenience
pub use client::{
    ClientConfig, ClientSessionManager,
    create_client_session_manager, create_client_session_manager_sync,
    create_full_client_manager, create_full_client_manager_sync,
};

pub use server::{
    ServerConfig, ServerSessionManager, RouteInfo, UserRegistration, ServerStats,
    create_server_session_manager, create_server_session_manager_sync,
    create_full_server_manager, create_full_server_manager_sync,
};

/// API version information
pub const API_VERSION: &str = "1.0.0";

/// Supported SIP protocol versions
pub const SUPPORTED_SIP_VERSIONS: &[&str] = &["2.0"];

/// Default user agent string for the API
pub const DEFAULT_USER_AGENT: &str = "RVOIP-SessionCore/1.0";

/// API capabilities
#[derive(Debug, Clone)]
pub struct ApiCapabilities {
    /// Supports call transfer
    pub call_transfer: bool,
    
    /// Supports media coordination
    pub media_coordination: bool,
    
    /// Supports call hold/resume
    pub call_hold: bool,
    
    /// Supports call routing
    pub call_routing: bool,
    
    /// Supports user registration
    pub user_registration: bool,
    
    /// Supports conference calls
    pub conference_calls: bool,
    
    /// Maximum concurrent sessions
    pub max_sessions: usize,
}

impl Default for ApiCapabilities {
    fn default() -> Self {
        Self {
            call_transfer: true,
            media_coordination: true,
            call_hold: true,
            call_routing: true,
            user_registration: true,
            conference_calls: false, // Not yet implemented
            max_sessions: 10000,
        }
    }
}

/// Get the current API capabilities
pub fn get_api_capabilities() -> ApiCapabilities {
    ApiCapabilities::default()
}

/// Check if a feature is supported
pub fn is_feature_supported(feature: &str) -> bool {
    let capabilities = get_api_capabilities();
    
    match feature {
        "call_transfer" => capabilities.call_transfer,
        "media_coordination" => capabilities.media_coordination,
        "call_hold" => capabilities.call_hold,
        "call_routing" => capabilities.call_routing,
        "user_registration" => capabilities.user_registration,
        "conference_calls" => capabilities.conference_calls,
        _ => false,
    }
}

/// API configuration for both client and server
#[derive(Debug, Clone)]
pub struct ApiConfig {
    /// API version to use
    pub version: String,
    
    /// User agent string
    pub user_agent: String,
    
    /// Enable debug logging
    pub debug_logging: bool,
    
    /// Enable metrics collection
    pub enable_metrics: bool,
    
    /// Event buffer size
    pub event_buffer_size: usize,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            version: API_VERSION.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            debug_logging: false,
            enable_metrics: true,
            event_buffer_size: 1000,
        }
    }
} 