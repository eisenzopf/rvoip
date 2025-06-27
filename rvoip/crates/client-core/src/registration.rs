//! Registration management for SIP client
//!
//! This module provides registration information structures.
//! All actual SIP registration operations are delegated to session-core.
//!
//! PROPER LAYER SEPARATION:
//! client-core -> session-core -> {transaction-core, media-core, sip-transport, sip-core}

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use rvoip_session_core::api::RegistrationHandle;

/// Registration configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationConfig {
    /// SIP server URI (e.g., "sip:registrar.example.com")
    pub server_uri: String,
    
    /// From URI (e.g., "sip:alice@example.com")
    pub from_uri: String,
    
    /// Contact URI (e.g., "sip:alice@192.168.1.100:5060")
    pub contact_uri: String,
    
    /// Registration expiration in seconds
    pub expires: u32,
    
    /// Authentication username (optional)
    pub username: Option<String>,
    
    /// Authentication password (optional)
    pub password: Option<String>,
    
    /// Authentication realm (optional)
    pub realm: Option<String>,
}

impl RegistrationConfig {
    /// Create a new registration configuration
    pub fn new(server_uri: String, from_uri: String, contact_uri: String) -> Self {
        Self {
            server_uri,
            from_uri,
            contact_uri,
            expires: 3600, // Default to 1 hour
            username: None,
            password: None,
            realm: None,
        }
    }
    
    /// Set authentication credentials
    pub fn with_credentials(mut self, username: String, password: String) -> Self {
        self.username = Some(username);
        self.password = Some(password);
        self
    }
    
    /// Set authentication realm
    pub fn with_realm(mut self, realm: String) -> Self {
        self.realm = Some(realm);
        self
    }
    
    /// Set expiration time
    pub fn with_expires(mut self, expires: u32) -> Self {
        self.expires = expires;
        self
    }
}

/// Registration status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RegistrationStatus {
    /// Registration is pending
    Pending,
    
    /// Registration is active
    Active,
    
    /// Registration failed
    Failed,
    
    /// Registration expired
    Expired,
    
    /// Registration was cancelled
    Cancelled,
}

impl std::fmt::Display for RegistrationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistrationStatus::Pending => write!(f, "Pending"),
            RegistrationStatus::Active => write!(f, "Active"),
            RegistrationStatus::Failed => write!(f, "Failed"),
            RegistrationStatus::Expired => write!(f, "Expired"),
            RegistrationStatus::Cancelled => write!(f, "Cancelled"),
        }
    }
}

/// Registration information
#[derive(Debug, Clone)]
pub struct RegistrationInfo {
    /// Unique registration ID
    pub id: Uuid,
    
    /// Server URI
    pub server_uri: String,
    
    /// From URI
    pub from_uri: String,
    
    /// Contact URI
    pub contact_uri: String,
    
    /// Registration expiration in seconds
    pub expires: u32,
    
    /// Current registration status
    pub status: RegistrationStatus,
    
    /// When the registration was created
    pub registration_time: DateTime<Utc>,
    
    /// When the registration was last refreshed
    pub refresh_time: Option<DateTime<Utc>>,
    
    /// Registration handle from session-core
    pub handle: Option<RegistrationHandle>,
}

/// Statistics about registrations
#[derive(Debug, Clone)]
pub struct RegistrationStats {
    pub total_registrations: usize,
    pub active_registrations: usize,
    pub failed_registrations: usize,
} 