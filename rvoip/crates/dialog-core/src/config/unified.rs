//! Unified Configuration for DialogManager
//!
//! This module provides a unified configuration system that replaces the split
//! DialogClient/DialogServer configuration with a single DialogManager that
//! supports client, server, and hybrid modes based on configuration.
//!
//! ## Overview
//!
//! The unified configuration system eliminates the artificial split between
//! "client" and "server" implementations by recognizing that most SIP endpoints
//! act as both UAC (User Agent Client) and UAS (User Agent Server) depending
//! on the specific transaction, not the application type.
//!
//! ## Architecture
//!
//! ```text
//! DialogManagerConfig
//!        │
//!        ├── Client(ClientBehavior)     ← Primarily outgoing calls
//!        ├── Server(ServerBehavior)     ← Primarily incoming calls  
//!        └── Hybrid(HybridBehavior)     ← Both directions (most common)
//!
//! All modes share:
//! - Dialog management (RFC 3261 compliance)
//! - Transaction coordination
//! - Response building and sending
//! - SIP method operations (BYE, REFER, etc.)
//! ```
//!
//! ## Examples
//!
//! ### Client Mode Configuration
//!
//! ```rust
//! use rvoip_dialog_core::config::unified::{DialogManagerConfig, ClientBehavior};
//! use rvoip_dialog_core::api::{DialogConfig, Credentials};
//!
//! let client_config = DialogManagerConfig::Client(ClientBehavior {
//!     dialog: DialogConfig::new("127.0.0.1:0".parse().unwrap()),
//!     from_uri: Some("sip:alice@example.com".to_string()),
//!     auto_auth: true,
//!     credentials: Some(Credentials::new("username", "password")),
//! });
//! ```
//!
//! ### Server Mode Configuration
//!
//! ```rust
//! use rvoip_dialog_core::config::unified::{DialogManagerConfig, ServerBehavior};
//! use rvoip_dialog_core::api::DialogConfig;
//!
//! let server_config = DialogManagerConfig::Server(ServerBehavior {
//!     dialog: DialogConfig::new("0.0.0.0:5060".parse().unwrap()),
//!     domain: Some("sip.company.com".to_string()),
//!     auto_options_response: true,
//!     auto_register_response: false,
//! });
//! ```
//!
//! ### Hybrid Mode Configuration
//!
//! ```rust
//! use rvoip_dialog_core::config::unified::{DialogManagerConfig, HybridBehavior};
//! use rvoip_dialog_core::api::{DialogConfig, Credentials};
//!
//! let hybrid_config = DialogManagerConfig::Hybrid(HybridBehavior {
//!     dialog: DialogConfig::new("192.168.1.100:5060".parse().unwrap()),
//!     from_uri: Some("sip:endpoint@company.com".to_string()),
//!     domain: Some("company.com".to_string()),
//!     auto_auth: true,
//!     credentials: Some(Credentials::new("user", "pass")),
//!     auto_options_response: true,
//!     auto_register_response: false,
//! });
//! ```

use std::net::SocketAddr;
use serde::{Deserialize, Serialize};

use crate::api::{DialogConfig, Credentials};

/// Unified configuration for DialogManager
///
/// Replaces the separate DialogClient and DialogServer configurations with
/// a single configuration system that supports different behavioral modes
/// based on the application's primary use case.
///
/// ## Behavioral Modes
///
/// - **Client**: Optimized for outgoing calls with authentication support
/// - **Server**: Optimized for incoming calls with auto-response features  
/// - **Hybrid**: Supports both incoming and outgoing calls (most flexible)
///
/// ## Examples
///
/// ### Simple Client Setup
///
/// ```rust
/// use rvoip_dialog_core::config::unified::DialogManagerConfig;
///
/// let config = DialogManagerConfig::client("127.0.0.1:0".parse().unwrap())
///     .with_from_uri("sip:alice@example.com")
///     .with_auth("alice", "secret123");
/// ```
///
/// ### Simple Server Setup
///
/// ```rust
/// use rvoip_dialog_core::config::unified::DialogManagerConfig;
///
/// let config = DialogManagerConfig::server("0.0.0.0:5060".parse().unwrap())
///     .with_domain("sip.company.com")
///     .with_auto_options();
/// ```
///
/// ### Full-Featured Hybrid Setup
///
/// ```rust
/// use rvoip_dialog_core::config::unified::DialogManagerConfig;
///
/// let config = DialogManagerConfig::hybrid("192.168.1.100:5060".parse().unwrap())
///     .with_from_uri("sip:pbx@company.com")
///     .with_domain("company.com")
///     .with_auth("pbx_user", "pbx_pass")
///     .with_auto_options();
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DialogManagerConfig {
    /// Client mode - optimized for outgoing calls
    Client(ClientBehavior),
    
    /// Server mode - optimized for incoming calls
    Server(ServerBehavior),
    
    /// Hybrid mode - supports both incoming and outgoing calls
    Hybrid(HybridBehavior),
}

impl DialogManagerConfig {
    /// Create a client mode configuration
    ///
    /// # Arguments
    /// * `local_address` - Address to bind to for outgoing calls
    ///
    /// # Returns
    /// New client configuration with defaults
    pub fn client(local_address: SocketAddr) -> ClientConfigBuilder {
        ClientConfigBuilder {
            behavior: ClientBehavior {
                dialog: DialogConfig::new(local_address),
                from_uri: None,
                auto_auth: false,
                credentials: None,
            }
        }
    }
    
    /// Create a server mode configuration
    ///
    /// # Arguments
    /// * `local_address` - Address to bind to for incoming calls
    ///
    /// # Returns
    /// New server configuration with defaults
    pub fn server(local_address: SocketAddr) -> ServerConfigBuilder {
        ServerConfigBuilder {
            behavior: ServerBehavior {
                dialog: DialogConfig::new(local_address),
                domain: None,
                auto_options_response: false,
                auto_register_response: false,
            }
        }
    }
    
    /// Create a hybrid mode configuration
    ///
    /// # Arguments
    /// * `local_address` - Address to bind to for both directions
    ///
    /// # Returns
    /// New hybrid configuration with defaults
    pub fn hybrid(local_address: SocketAddr) -> HybridConfigBuilder {
        HybridConfigBuilder {
            behavior: HybridBehavior {
                dialog: DialogConfig::new(local_address),
                from_uri: None,
                domain: None,
                auto_auth: false,
                credentials: None,
                auto_options_response: false,
                auto_register_response: false,
            }
        }
    }
    
    /// Get the base dialog configuration
    pub fn dialog_config(&self) -> &DialogConfig {
        match self {
            DialogManagerConfig::Client(c) => &c.dialog,
            DialogManagerConfig::Server(s) => &s.dialog,
            DialogManagerConfig::Hybrid(h) => &h.dialog,
        }
    }
    
    /// Get the local address for binding
    pub fn local_address(&self) -> SocketAddr {
        self.dialog_config().local_address
    }
    
    /// Check if this configuration supports outgoing calls
    pub fn supports_outgoing_calls(&self) -> bool {
        match self {
            DialogManagerConfig::Client(_) => true,
            DialogManagerConfig::Server(_) => false,
            DialogManagerConfig::Hybrid(_) => true,
        }
    }
    
    /// Check if this configuration supports incoming calls  
    pub fn supports_incoming_calls(&self) -> bool {
        match self {
            DialogManagerConfig::Client(_) => false,
            DialogManagerConfig::Server(_) => true,
            DialogManagerConfig::Hybrid(_) => true,
        }
    }
    
    /// Get the from URI for outgoing requests (if available)
    pub fn from_uri(&self) -> Option<&str> {
        match self {
            DialogManagerConfig::Client(c) => c.from_uri.as_deref(),
            DialogManagerConfig::Server(_) => None,
            DialogManagerConfig::Hybrid(h) => h.from_uri.as_deref(),
        }
    }
    
    /// Get the domain for server operations (if available)
    pub fn domain(&self) -> Option<&str> {
        match self {
            DialogManagerConfig::Client(_) => None,
            DialogManagerConfig::Server(s) => s.domain.as_deref(),
            DialogManagerConfig::Hybrid(h) => h.domain.as_deref(),
        }
    }
    
    /// Check if automatic authentication is enabled
    pub fn auto_auth_enabled(&self) -> bool {
        match self {
            DialogManagerConfig::Client(c) => c.auto_auth,
            DialogManagerConfig::Server(_) => false,
            DialogManagerConfig::Hybrid(h) => h.auto_auth,
        }
    }
    
    /// Get authentication credentials (if available)
    pub fn credentials(&self) -> Option<&Credentials> {
        match self {
            DialogManagerConfig::Client(c) => c.credentials.as_ref(),
            DialogManagerConfig::Server(_) => None,
            DialogManagerConfig::Hybrid(h) => h.credentials.as_ref(),
        }
    }
    
    /// Check if automatic OPTIONS response is enabled
    pub fn auto_options_enabled(&self) -> bool {
        match self {
            DialogManagerConfig::Client(_) => false,
            DialogManagerConfig::Server(s) => s.auto_options_response,
            DialogManagerConfig::Hybrid(h) => h.auto_options_response,
        }
    }
    
    /// Check if automatic REGISTER response is enabled
    pub fn auto_register_enabled(&self) -> bool {
        match self {
            DialogManagerConfig::Client(_) => false,
            DialogManagerConfig::Server(s) => s.auto_register_response,
            DialogManagerConfig::Hybrid(h) => h.auto_register_response,
        }
    }
    
    /// Validate the configuration
    pub fn validate(&self) -> Result<(), String> {
        // Validate base dialog config
        self.dialog_config().validate()?;
        
        // Mode-specific validation
        match self {
            DialogManagerConfig::Client(c) => c.validate(),
            DialogManagerConfig::Server(s) => s.validate(),
            DialogManagerConfig::Hybrid(h) => h.validate(),
        }
    }
}

/// Client behavior configuration
///
/// Configures DialogManager for primarily outgoing call scenarios.
/// Includes authentication support and from URI configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientBehavior {
    /// Base dialog configuration
    pub dialog: DialogConfig,
    
    /// Default from URI for outgoing requests
    pub from_uri: Option<String>,
    
    /// Enable automatic authentication
    pub auto_auth: bool,
    
    /// Credentials for authentication
    pub credentials: Option<Credentials>,
}

impl ClientBehavior {
    /// Validate client configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.auto_auth && self.credentials.is_none() {
            return Err("auto_auth enabled but no credentials provided".to_string());
        }
        
        if let Some(from_uri) = &self.from_uri {
            if !from_uri.starts_with("sip:") && !from_uri.starts_with("sips:") {
                return Err("from_uri must be a valid SIP URI".to_string());
            }
        }
        
        Ok(())
    }
}

/// Server behavior configuration
///
/// Configures DialogManager for primarily incoming call scenarios.
/// Includes auto-response features and domain configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerBehavior {
    /// Base dialog configuration
    pub dialog: DialogConfig,
    
    /// Server domain name
    pub domain: Option<String>,
    
    /// Enable automatic OPTIONS response
    pub auto_options_response: bool,
    
    /// Enable automatic REGISTER response
    pub auto_register_response: bool,
}

impl ServerBehavior {
    /// Validate server configuration
    pub fn validate(&self) -> Result<(), String> {
        // Server validation is generally permissive
        // Domain is optional for simple servers
        Ok(())
    }
}

/// Hybrid behavior configuration
///
/// Configures DialogManager for both incoming and outgoing calls.
/// Combines features from both client and server modes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridBehavior {
    /// Base dialog configuration
    pub dialog: DialogConfig,
    
    /// Default from URI for outgoing requests
    pub from_uri: Option<String>,
    
    /// Server domain name
    pub domain: Option<String>,
    
    /// Enable automatic authentication
    pub auto_auth: bool,
    
    /// Credentials for authentication
    pub credentials: Option<Credentials>,
    
    /// Enable automatic OPTIONS response
    pub auto_options_response: bool,
    
    /// Enable automatic REGISTER response
    pub auto_register_response: bool,
}

impl HybridBehavior {
    /// Validate hybrid configuration
    pub fn validate(&self) -> Result<(), String> {
        if self.auto_auth && self.credentials.is_none() {
            return Err("auto_auth enabled but no credentials provided".to_string());
        }
        
        if let Some(from_uri) = &self.from_uri {
            if !from_uri.starts_with("sip:") && !from_uri.starts_with("sips:") {
                return Err("from_uri must be a valid SIP URI".to_string());
            }
        }
        
        Ok(())
    }
}

/// Builder for client configuration
pub struct ClientConfigBuilder {
    behavior: ClientBehavior,
}

impl ClientConfigBuilder {
    /// Set the from URI for outgoing requests
    pub fn with_from_uri(mut self, from_uri: impl Into<String>) -> Self {
        self.behavior.from_uri = Some(from_uri.into());
        self
    }
    
    /// Enable automatic authentication with credentials
    pub fn with_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.behavior.auto_auth = true;
        self.behavior.credentials = Some(Credentials::new(username, password));
        self
    }
    
    /// Set custom credentials without enabling auto-auth
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.behavior.credentials = Some(credentials);
        self
    }
    
    /// Customize the dialog configuration
    pub fn with_dialog_config<F>(mut self, f: F) -> Self 
    where 
        F: FnOnce(DialogConfig) -> DialogConfig
    {
        self.behavior.dialog = f(self.behavior.dialog);
        self
    }
    
    /// Build the final configuration
    pub fn build(self) -> DialogManagerConfig {
        DialogManagerConfig::Client(self.behavior)
    }
}

/// Builder for server configuration  
pub struct ServerConfigBuilder {
    behavior: ServerBehavior,
}

impl ServerConfigBuilder {
    /// Set the server domain
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.behavior.domain = Some(domain.into());
        self
    }
    
    /// Enable automatic OPTIONS responses
    pub fn with_auto_options(mut self) -> Self {
        self.behavior.auto_options_response = true;
        self
    }
    
    /// Enable automatic REGISTER responses
    pub fn with_auto_register(mut self) -> Self {
        self.behavior.auto_register_response = true;
        self
    }
    
    /// Customize the dialog configuration
    pub fn with_dialog_config<F>(mut self, f: F) -> Self 
    where 
        F: FnOnce(DialogConfig) -> DialogConfig
    {
        self.behavior.dialog = f(self.behavior.dialog);
        self
    }
    
    /// Build the final configuration
    pub fn build(self) -> DialogManagerConfig {
        DialogManagerConfig::Server(self.behavior)
    }
}

/// Builder for hybrid configuration
pub struct HybridConfigBuilder {
    behavior: HybridBehavior,
}

impl HybridConfigBuilder {
    /// Set the from URI for outgoing requests
    pub fn with_from_uri(mut self, from_uri: impl Into<String>) -> Self {
        self.behavior.from_uri = Some(from_uri.into());
        self
    }
    
    /// Set the server domain
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.behavior.domain = Some(domain.into());
        self
    }
    
    /// Enable automatic authentication with credentials
    pub fn with_auth(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.behavior.auto_auth = true;
        self.behavior.credentials = Some(Credentials::new(username, password));
        self
    }
    
    /// Set custom credentials without enabling auto-auth
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.behavior.credentials = Some(credentials);
        self
    }
    
    /// Enable automatic OPTIONS responses
    pub fn with_auto_options(mut self) -> Self {
        self.behavior.auto_options_response = true;
        self
    }
    
    /// Enable automatic REGISTER responses
    pub fn with_auto_register(mut self) -> Self {
        self.behavior.auto_register_response = true;
        self
    }
    
    /// Customize the dialog configuration
    pub fn with_dialog_config<F>(mut self, f: F) -> Self 
    where 
        F: FnOnce(DialogConfig) -> DialogConfig
    {
        self.behavior.dialog = f(self.behavior.dialog);
        self
    }
    
    /// Build the final configuration
    pub fn build(self) -> DialogManagerConfig {
        DialogManagerConfig::Hybrid(self.behavior)
    }
}

// Convenience conversions for backward compatibility
impl From<crate::api::ClientConfig> for DialogManagerConfig {
    fn from(client_config: crate::api::ClientConfig) -> Self {
        DialogManagerConfig::Client(ClientBehavior {
            dialog: client_config.dialog,
            from_uri: client_config.from_uri,
            auto_auth: client_config.auto_auth,
            credentials: client_config.credentials,
        })
    }
}

impl From<crate::api::ServerConfig> for DialogManagerConfig {
    fn from(server_config: crate::api::ServerConfig) -> Self {
        DialogManagerConfig::Server(ServerBehavior {
            dialog: server_config.dialog,
            domain: server_config.domain,
            auto_options_response: server_config.auto_options_response,
            auto_register_response: server_config.auto_register_response,
        })
    }
} 