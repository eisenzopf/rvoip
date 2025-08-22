//! UAC-specific types

use std::sync::Arc;
use tokio::sync::RwLock;
use crate::api::types::{SessionId, CallState};
use crate::errors::Result;

/// UAC configuration
#[derive(Debug, Clone)]
pub struct UacConfig {
    /// SIP identity (e.g., "sip:alice@example.com")
    pub identity: String,
    
    /// SIP server address (e.g., "192.168.1.100:5060")
    pub server_addr: String,
    
    /// Local bind address (e.g., "0.0.0.0:5061")
    pub local_addr: String,
    
    /// Username for authentication (optional)
    pub username: Option<String>,
    
    /// Password for authentication (optional)
    pub password: Option<String>,
    
    /// User agent string
    pub user_agent: String,
    
    /// Enable auto-answer for testing
    pub auto_answer: bool,
    
    /// Registration expiry in seconds
    pub registration_expiry: u32,
    
    /// Call timeout in seconds
    pub call_timeout: u32,
}

impl Default for UacConfig {
    fn default() -> Self {
        Self {
            identity: String::new(),
            server_addr: String::new(),
            local_addr: "0.0.0.0:0".to_string(),
            username: None,
            password: None,
            user_agent: format!("session-core-uac/{}", env!("CARGO_PKG_VERSION")),
            auto_answer: false,
            registration_expiry: 3600,
            call_timeout: 30,
        }
    }
}

/// Call options for UAC
#[derive(Debug, Clone, Default)]
pub struct CallOptions {
    /// Custom SDP offer (if not provided, will be auto-generated)
    pub custom_sdp: Option<String>,
    
    /// Custom headers to include in INVITE
    pub custom_headers: Vec<(String, String)>,
    
    /// Enable early media
    pub early_media: bool,
    
    /// Preferred codecs in order
    pub preferred_codecs: Vec<String>,
    
    /// Call timeout override in seconds
    pub timeout: Option<u32>,
}

/// Registration state
#[derive(Debug, Clone, PartialEq)]
pub enum RegistrationState {
    /// Not registered
    Unregistered,
    /// Registration in progress
    Registering,
    /// Successfully registered
    Registered,
    /// Registration failed
    Failed(String),
}

/// UAC statistics
#[derive(Debug, Clone, Default)]
pub struct UacStats {
    /// Total calls made
    pub calls_made: u64,
    /// Successful calls
    pub calls_succeeded: u64,
    /// Failed calls
    pub calls_failed: u64,
    /// Current active calls
    pub active_calls: u32,
    /// Registration attempts
    pub registration_attempts: u64,
    /// Successful registrations
    pub registrations_succeeded: u64,
}