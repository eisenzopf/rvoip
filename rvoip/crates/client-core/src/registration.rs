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

/// Configuration for SIP registration
#[derive(Debug, Clone)]
pub struct RegistrationConfig {
    /// SIP server URI (registrar)
    pub server_uri: String,
    /// User URI (AOR - Address of Record)
    pub user_uri: String,
    /// Display name
    pub display_name: Option<String>,
    /// Contact URI
    pub contact_uri: String,
    /// Authentication username
    pub username: Option<String>,
    /// Authentication password  
    pub password: Option<String>,
    /// Registration expiration time (seconds)
    pub expires: Option<u32>,
    /// User agent string
    pub user_agent: Option<String>,
}

impl RegistrationConfig {
    /// Create a new registration configuration
    pub fn new(server_uri: String, user_uri: String, contact_uri: String) -> Self {
        Self {
            server_uri,
            user_uri,
            display_name: None,
            contact_uri,
            username: None,
            password: None,
            expires: Some(3600), // 1 hour default
            user_agent: None,
        }
    }

    /// Set display name
    pub fn with_display_name(mut self, display_name: String) -> Self {
        self.display_name = Some(display_name);
        self
    }

    /// Set authentication credentials
    pub fn with_auth(mut self, username: String, password: String) -> Self {
        self.username = Some(username);
        self.password = Some(password);
        self
    }

    /// Set expiration time
    pub fn with_expires(mut self, expires: u32) -> Self {
        self.expires = Some(expires);
        self
    }

    /// Set user agent
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = Some(user_agent);
        self
    }
}

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
    /// Unregistration in progress
    Unregistering,
}

impl RegistrationStatus {
    /// Check if registration is active
    pub fn is_active(&self) -> bool {
        matches!(self, RegistrationStatus::Registered)
    }

    /// Check if registration is in progress
    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            RegistrationStatus::Registering | RegistrationStatus::Unregistering
        )
    }
}

/// Information about a SIP registration
#[derive(Debug, Clone)]
pub struct RegistrationInfo {
    /// Registration ID
    pub registration_id: Uuid,
    /// Server URI
    pub server_uri: String,
    /// User URI
    pub user_uri: String,
    /// Contact URI
    pub contact_uri: String,
    /// Current status
    pub status: RegistrationStatus,
    /// Expiration time
    pub expires: Option<u32>,
    /// When registration was created
    pub created_at: DateTime<Utc>,
    /// When last registered successfully
    pub registered_at: Option<DateTime<Utc>>,
    /// Next registration refresh time
    pub next_refresh_at: Option<DateTime<Utc>>,
    /// Last error message (if any)
    pub last_error: Option<String>,
}

/// Statistics about registrations
#[derive(Debug, Clone)]
pub struct RegistrationStats {
    pub total_registrations: usize,
    pub active_registrations: usize,
    pub failed_registrations: usize,
} 