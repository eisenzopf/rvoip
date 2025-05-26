//! Client Configuration
//!
//! This module provides configuration types for the session-core client API.
//! It handles client settings, transport configuration, and validation.

use std::net::SocketAddr;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use anyhow::{Result, Context};

use crate::api::server::config::TransportProtocol;

/// Client configuration for session-core SIP client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Local binding address (optional, will use system default if None)
    pub local_address: Option<SocketAddr>,
    
    /// Preferred transport protocol
    pub transport_protocol: TransportProtocol,
    
    /// Maximum number of concurrent outbound sessions
    pub max_sessions: usize,
    
    /// Session timeout duration
    pub session_timeout: Duration,
    
    /// Transaction timeout duration
    pub transaction_timeout: Duration,
    
    /// Enable media coordination
    pub enable_media: bool,
    
    /// User agent string for SIP headers
    pub user_agent: String,
    
    /// Default contact URI for the client
    pub contact_uri: Option<String>,
    
    /// Default From URI for outbound calls
    pub from_uri: Option<String>,
    
    /// Registration server (if using registration)
    pub registrar_uri: Option<String>,
    
    /// Authentication credentials
    pub credentials: Option<ClientCredentials>,
}

/// Authentication credentials for SIP client
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientCredentials {
    /// Username for authentication
    pub username: String,
    
    /// Password for authentication
    pub password: String,
    
    /// Authentication realm (optional)
    pub realm: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            local_address: None, // System will choose
            transport_protocol: TransportProtocol::Udp,
            max_sessions: 100,
            session_timeout: Duration::from_secs(300), // 5 minutes
            transaction_timeout: Duration::from_secs(32), // RFC 3261 Timer B
            enable_media: true,
            user_agent: "rvoip-session-core-client".to_string(),
            contact_uri: None,
            from_uri: None,
            registrar_uri: None,
            credentials: None,
        }
    }
}

impl ClientConfig {
    /// Create a new client configuration
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set the local binding address
    pub fn with_local_address(mut self, address: SocketAddr) -> Self {
        self.local_address = Some(address);
        self
    }
    
    /// Set the transport protocol
    pub fn with_transport(mut self, protocol: TransportProtocol) -> Self {
        self.transport_protocol = protocol;
        self
    }
    
    /// Set the maximum number of sessions
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = max_sessions;
        self
    }
    
    /// Set the session timeout
    pub fn with_session_timeout(mut self, timeout: Duration) -> Self {
        self.session_timeout = timeout;
        self
    }
    
    /// Set the user agent string
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = user_agent;
        self
    }
    
    /// Set the contact URI
    pub fn with_contact_uri(mut self, uri: String) -> Self {
        self.contact_uri = Some(uri);
        self
    }
    
    /// Set the from URI
    pub fn with_from_uri(mut self, uri: String) -> Self {
        self.from_uri = Some(uri);
        self
    }
    
    /// Set the registrar URI
    pub fn with_registrar_uri(mut self, uri: String) -> Self {
        self.registrar_uri = Some(uri);
        self
    }
    
    /// Set authentication credentials
    pub fn with_credentials(mut self, username: String, password: String) -> Self {
        self.credentials = Some(ClientCredentials {
            username,
            password,
            realm: None,
        });
        self
    }
    
    /// Set authentication credentials with realm
    pub fn with_credentials_and_realm(mut self, username: String, password: String, realm: String) -> Self {
        self.credentials = Some(ClientCredentials {
            username,
            password,
            realm: Some(realm),
        });
        self
    }
    
    /// Enable or disable media coordination
    pub fn with_media(mut self, enable: bool) -> Self {
        self.enable_media = enable;
        self
    }
    
    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate max sessions
        if self.max_sessions == 0 {
            return Err(anyhow::anyhow!("Max sessions must be greater than 0"));
        }
        
        if self.max_sessions > 10_000 {
            return Err(anyhow::anyhow!("Max sessions cannot exceed 10,000 for client"));
        }
        
        // Validate timeouts
        if self.session_timeout.as_secs() < 30 {
            return Err(anyhow::anyhow!("Session timeout must be at least 30 seconds"));
        }
        
        if self.transaction_timeout.as_secs() < 1 {
            return Err(anyhow::anyhow!("Transaction timeout must be at least 1 second"));
        }
        
        // Validate user agent
        if self.user_agent.is_empty() {
            return Err(anyhow::anyhow!("User agent cannot be empty"));
        }
        
        // Validate URIs if provided
        if let Some(ref uri) = self.contact_uri {
            if uri.is_empty() {
                return Err(anyhow::anyhow!("Contact URI cannot be empty if provided"));
            }
            
            if !uri.starts_with("sip:") && !uri.starts_with("sips:") {
                return Err(anyhow::anyhow!("Contact URI must be a valid SIP URI"));
            }
        }
        
        if let Some(ref uri) = self.from_uri {
            if uri.is_empty() {
                return Err(anyhow::anyhow!("From URI cannot be empty if provided"));
            }
            
            if !uri.starts_with("sip:") && !uri.starts_with("sips:") {
                return Err(anyhow::anyhow!("From URI must be a valid SIP URI"));
            }
        }
        
        if let Some(ref uri) = self.registrar_uri {
            if uri.is_empty() {
                return Err(anyhow::anyhow!("Registrar URI cannot be empty if provided"));
            }
            
            if !uri.starts_with("sip:") && !uri.starts_with("sips:") {
                return Err(anyhow::anyhow!("Registrar URI must be a valid SIP URI"));
            }
        }
        
        // Validate credentials if provided
        if let Some(ref creds) = self.credentials {
            if creds.username.is_empty() {
                return Err(anyhow::anyhow!("Username cannot be empty if credentials provided"));
            }
            
            if creds.password.is_empty() {
                return Err(anyhow::anyhow!("Password cannot be empty if credentials provided"));
            }
        }
        
        Ok(())
    }
    
    /// Get the effective contact URI
    pub fn effective_contact_uri(&self) -> String {
        self.contact_uri.clone().unwrap_or_else(|| {
            if let Some(local_addr) = self.local_address {
                format!("sip:client@{}:{}", local_addr.ip(), local_addr.port())
            } else {
                "sip:client@localhost".to_string()
            }
        })
    }
    
    /// Get the effective from URI
    pub fn effective_from_uri(&self) -> String {
        self.from_uri.clone().unwrap_or_else(|| {
            if let Some(ref creds) = self.credentials {
                format!("sip:{}@localhost", creds.username)
            } else {
                "sip:anonymous@localhost".to_string()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        let config = ClientConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.transport_protocol, TransportProtocol::Udp);
        assert_eq!(config.max_sessions, 100);
        assert!(config.enable_media);
        assert!(config.local_address.is_none());
    }
    
    #[test]
    fn test_config_builder() {
        let config = ClientConfig::new()
            .with_transport(TransportProtocol::Tcp)
            .with_max_sessions(50)
            .with_user_agent("test-client".to_string())
            .with_credentials("user".to_string(), "pass".to_string());
            
        assert!(config.validate().is_ok());
        assert_eq!(config.transport_protocol, TransportProtocol::Tcp);
        assert_eq!(config.max_sessions, 50);
        assert_eq!(config.user_agent, "test-client");
        assert!(config.credentials.is_some());
    }
    
    #[test]
    fn test_validation_errors() {
        let mut config = ClientConfig::default();
        
        // Test invalid max sessions
        config.max_sessions = 0;
        assert!(config.validate().is_err());
        
        // Test invalid timeout
        config.max_sessions = 100;
        config.session_timeout = Duration::from_secs(10);
        assert!(config.validate().is_err());
        
        // Test invalid URI
        config.session_timeout = Duration::from_secs(300);
        config.contact_uri = Some("invalid-uri".to_string());
        assert!(config.validate().is_err());
    }
    
    #[test]
    fn test_effective_uris() {
        let config = ClientConfig::new()
            .with_credentials("testuser".to_string(), "testpass".to_string());
        
        let from_uri = config.effective_from_uri();
        assert!(from_uri.contains("testuser"));
        
        let contact_uri = config.effective_contact_uri();
        assert!(contact_uri.starts_with("sip:"));
    }
} 