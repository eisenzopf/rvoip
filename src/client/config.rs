//! Client configuration types and builders
//!
//! This module contains all configuration-related types for the SIP client,
//! including the main ClientConfig struct and its builder methods.

use std::collections::HashMap;
use std::net::SocketAddr;

/// Configuration for the SIP client
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Local SIP listening address
    pub local_sip_addr: SocketAddr,
    /// Local media address for RTP
    pub local_media_addr: SocketAddr,
    /// User agent string
    pub user_agent: String,
    /// Default codec preferences
    pub preferred_codecs: Vec<String>,
    /// Maximum number of concurrent calls
    pub max_concurrent_calls: usize,
    /// Enable detailed logging
    pub enable_logging: bool,
    /// Additional configuration parameters
    pub extra_params: HashMap<String, String>,
    
    // === SIP Identity Configuration ===
    /// Default From URI for outgoing calls (e.g., "sip:alice@example.com")
    pub from_uri: Option<String>,
    /// Default Contact URI (e.g., "sip:alice@192.168.1.100:5060")
    pub contact_uri: Option<String>,
    /// Display name for outgoing calls
    pub display_name: Option<String>,
    /// Default call rejection status code  
    pub default_reject_status: u16,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            local_sip_addr: "127.0.0.1:5060".parse().unwrap(),
            local_media_addr: "127.0.0.1:10000".parse().unwrap(),
            user_agent: "rvoip-client/0.1.0".to_string(),
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            max_concurrent_calls: 10,
            enable_logging: true,
            extra_params: HashMap::new(),
            from_uri: None,
            contact_uri: None,
            display_name: None,
            default_reject_status: 486, // Busy Here
        }
    }
}

impl ClientConfig {
    /// Create a new client configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Get From URI with fallback to default
    pub fn get_from_uri(&self) -> String {
        self.from_uri.clone().unwrap_or_else(|| {
            format!("sip:user@{}", self.local_sip_addr.ip())
        })
    }
    
    /// Get Contact URI with fallback to default
    pub fn get_contact_uri(&self) -> String {
        self.contact_uri.clone().unwrap_or_else(|| {
            format!("sip:user@{}", self.local_sip_addr)
        })
    }

    /// Set local SIP listening address
    pub fn with_sip_addr(mut self, addr: SocketAddr) -> Self {
        self.local_sip_addr = addr;
        self
    }

    /// Set local media address
    pub fn with_media_addr(mut self, addr: SocketAddr) -> Self {
        self.local_media_addr = addr;
        self
    }

    /// Set user agent string
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = user_agent;
        self
    }

    /// Set preferred codecs
    pub fn with_codecs(mut self, codecs: Vec<String>) -> Self {
        self.preferred_codecs = codecs;
        self
    }

    /// Set maximum concurrent calls
    pub fn with_max_calls(mut self, max_calls: usize) -> Self {
        self.max_concurrent_calls = max_calls;
        self
    }

    /// Add extra configuration parameter
    pub fn with_param(mut self, key: String, value: String) -> Self {
        self.extra_params.insert(key, value);
        self
    }

    /// Set default From URI for outgoing calls
    pub fn with_from_uri(mut self, from_uri: String) -> Self {
        self.from_uri = Some(from_uri);
        self
    }
    
    /// Set Contact URI  
    pub fn with_contact_uri(mut self, contact_uri: String) -> Self {
        self.contact_uri = Some(contact_uri);
        self
    }
    
    /// Set display name
    pub fn with_display_name(mut self, display_name: String) -> Self {
        self.display_name = Some(display_name);
        self
    }
    
    /// Set default call rejection status code
    pub fn with_default_reject_status(mut self, status: u16) -> Self {
        self.default_reject_status = status;
        self
    }
} 