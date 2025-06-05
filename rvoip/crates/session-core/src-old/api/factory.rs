//! Session Manager Factory
//!
//! This module provides the core SessionManager creation API.
//! Clean, simple, and intuitive - just one function with clear parameters.

use std::sync::Arc;
use anyhow::{Result, Context};
use tracing::{info, debug};

use crate::session::manager::SessionManager;
use crate::session::SessionConfig;
use crate::events::EventBus;
use crate::media::MediaManager;

/// Session mode for transport configuration
#[derive(Debug, Clone)]
pub enum SessionMode {
    /// Server mode - listens for incoming SIP connections
    Server {
        /// SIP domain for the server
        domain: Option<String>,
        /// Enable automatic OPTIONS responses
        auto_options: bool,
    },
    /// Endpoint mode - makes outgoing SIP connections
    Endpoint {
        /// Remote server address for registration/proxy
        remote_server: Option<std::net::SocketAddr>,
        /// Authentication credentials
        auth_username: Option<String>,
        auth_password: Option<String>,
    },
}

impl Default for SessionMode {
    fn default() -> Self {
        SessionMode::Server {
            domain: None,
            auto_options: true,
        }
    }
}

/// Configuration for SessionManager
#[derive(Debug, Clone)]
pub struct SessionManagerConfig {
    /// Local signaling address
    pub local_signaling_addr: std::net::SocketAddr,
    
    /// Local media address
    pub local_media_addr: std::net::SocketAddr,
    
    /// Maximum concurrent sessions
    pub max_sessions: Option<usize>,
    
    /// Enable debug logging
    pub debug_logging: bool,
    
    /// Event buffer size
    pub event_buffer_size: usize,
    
    /// Display name for sessions
    pub display_name: Option<String>,
    
    /// User agent string
    pub user_agent: String,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            local_signaling_addr: "127.0.0.1:5060".parse().unwrap(),
            local_media_addr: "127.0.0.1:10000".parse().unwrap(),
            max_sessions: None,
            debug_logging: false,
            event_buffer_size: 1000,
            display_name: None,
            user_agent: "RVOIP-SessionCore/1.0".to_string(),
        }
    }
}

impl SessionManagerConfig {
    /// Create new configuration with addresses
    pub fn new(local_signaling_addr: std::net::SocketAddr, local_media_addr: std::net::SocketAddr) -> Self {
        Self {
            local_signaling_addr,
            local_media_addr,
            ..Default::default()
        }
    }
    
    /// Set maximum sessions
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = Some(max_sessions);
        self
    }
    
    /// Enable debug logging
    pub fn with_debug_logging(mut self, enable: bool) -> Self {
        self.debug_logging = enable;
        self
    }
    
    /// Set event buffer size
    pub fn with_event_buffer_size(mut self, size: usize) -> Self {
        self.event_buffer_size = size;
        self
    }
    
    /// Set display name
    pub fn with_display_name(mut self, display_name: String) -> Self {
        self.display_name = Some(display_name);
        self
    }
    
    /// Set user agent
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = user_agent;
        self
    }
    
    /// Validate configuration
    pub fn validate(&self) -> Result<()> {
        if let Some(max_sessions) = self.max_sessions {
            if max_sessions == 0 {
                return Err(anyhow::anyhow!("max_sessions must be greater than 0"));
            }
        }
        
        if self.event_buffer_size == 0 {
            return Err(anyhow::anyhow!("event_buffer_size must be greater than 0"));
        }
        
        Ok(())
    }
}

impl SessionMode {
    /// Create server mode with domain
    pub fn server(domain: &str) -> Self {
        SessionMode::Server {
            domain: Some(domain.to_string()),
            auto_options: true,
        }
    }
    
    /// Create endpoint mode with server
    pub fn endpoint(remote_server: std::net::SocketAddr) -> Self {
        SessionMode::Endpoint {
            remote_server: Some(remote_server),
            auth_username: None,
            auth_password: None,
        }
    }
    
    /// Set authentication for endpoint mode
    pub fn with_auth(mut self, username: String, password: String) -> Self {
        if let SessionMode::Endpoint { ref mut auth_username, ref mut auth_password, .. } = self {
            *auth_username = Some(username);
            *auth_password = Some(password);
        }
        self
    }
    
    /// Set domain for server mode
    pub fn with_domain(mut self, domain_name: String) -> Self {
        if let SessionMode::Server { ref mut domain, .. } = self {
            *domain = Some(domain_name);
        }
        self
    }
}

impl SessionManager {
    /// ✅ **PRIMARY API**: Create SessionManager with mode and configuration
    /// 
    /// This is the main API - clean, simple, and intuitive.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// // Server that accepts calls
    /// let config = SessionManagerConfig::new(signaling_addr, media_addr);
    /// let session_manager = SessionManager::create(
    ///     SessionMode::server("example.com"), 
    ///     config
    /// ).await?;
    /// 
    /// // Endpoint that makes calls
    /// let mode = SessionMode::endpoint(server_addr).with_auth("user".into(), "pass".into());
    /// let session_manager = SessionManager::create(mode, config).await?;
    /// ```
    pub async fn create(mode: SessionMode, config: SessionManagerConfig) -> Result<Arc<Self>> {
        info!("Creating SessionManager with mode: {:?}", mode);
        
        // Validate configuration
        config.validate()
            .context("Invalid SessionManager configuration")?;
        
        // Create dialog configuration based on mode
        let dialog_config = match &mode {
            SessionMode::Server { domain, auto_options } => {
                let mut dialog_config = rvoip_dialog_core::config::DialogManagerConfig::server(config.local_signaling_addr);
                
                if let Some(domain) = domain {
                    dialog_config = dialog_config.with_domain(domain);
                } else {
                    dialog_config = dialog_config.with_domain(&format!("{}", config.local_signaling_addr.ip()));
                }
                
                if *auto_options {
                    dialog_config = dialog_config.with_auto_options();
                }
                
                dialog_config.build()
            },
            SessionMode::Endpoint { remote_server, auth_username, auth_password } => {
                let mut dialog_config = rvoip_dialog_core::config::DialogManagerConfig::client(config.local_signaling_addr);
                
                if let (Some(username), Some(password)) = (auth_username, auth_password) {
                    dialog_config = dialog_config.with_auth(username.clone(), password.clone());
                }
                
                dialog_config.build()
            }
        };
        
        // Create dialog API internally
        let dialog_api = Arc::new(rvoip_dialog_core::UnifiedDialogApi::create(dialog_config).await
            .context("Failed to create dialog API")?);
        
        debug!("✅ Created dialog API internally");
        
        // Create media manager internally
        let media_manager = Arc::new(MediaManager::new().await
            .context("Failed to create media manager")?);
        
        debug!("✅ Created media manager internally");
        
        // Create event bus
        let event_bus = EventBus::new(config.event_buffer_size).await
            .map_err(|e| anyhow::anyhow!("Failed to create event bus: {}", e))?;
        
        debug!("✅ Created event bus with buffer size {}", config.event_buffer_size);
        
        // Create session configuration
        let session_config = SessionConfig {
            local_signaling_addr: config.local_signaling_addr,
            local_media_addr: config.local_media_addr,
            supported_codecs: vec![crate::media::AudioCodecType::PCMU, crate::media::AudioCodecType::PCMA],
            display_name: config.display_name.clone(),
            user_agent: config.user_agent.clone(),
            max_duration: 0, // Unlimited by default
            max_sessions: config.max_sessions,
        };
        
        // Create session manager with injected dependencies
        let session_manager = Arc::new(SessionManager::new(
            dialog_api,
            session_config.clone(),
            event_bus.clone()
        ).await.context("Failed to create session manager")?);
        
        info!("✅ Created SessionManager with clean API");
        
        Ok(session_manager)
    }
}

// ============================================================================
// DEPRECATED APIs - For backward compatibility only  
// ============================================================================

/// Legacy session infrastructure container
#[deprecated(note = "Use SessionManager::create(mode, config) instead")]
pub struct SessionInfrastructure {
    pub session_manager: Arc<SessionManager>,
    pub media_manager: Arc<MediaManager>,
    pub event_bus: EventBus,
    pub config: SessionConfig,
}

/// Legacy configuration - just an alias to the new one
#[deprecated(note = "Use SessionManagerConfig instead")]
pub type SessionInfrastructureConfig = SessionManagerConfig;

#[deprecated(note = "Use SessionManager::create(SessionMode::server(domain), config) instead")]
pub async fn create_session_manager_for_sip_server(
    config: SessionManagerConfig,
) -> Result<Arc<SessionManager>> {
    let mode = SessionMode::Server { domain: None, auto_options: true };
    SessionManager::create(mode, config).await
}

#[deprecated(note = "Use SessionManager::create(SessionMode::endpoint(server), config) instead")]  
pub async fn create_session_manager_for_sip_endpoint(
    config: SessionManagerConfig,
) -> Result<Arc<SessionManager>> {
    let mode = SessionMode::Endpoint { 
        remote_server: None, 
        auth_username: None, 
        auth_password: None 
    };
    SessionManager::create(mode, config).await
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_session_manager_config_validation() {
        let mut config = SessionManagerConfig::default();
        assert!(config.validate().is_ok());
        
        // Test invalid event_buffer_size
        config.event_buffer_size = 0;
        assert!(config.validate().is_err());
        
        // Test invalid max_sessions
        config.event_buffer_size = 1000;
        config.max_sessions = Some(0);
        assert!(config.validate().is_err());
    }
    
    #[test]
    fn test_session_mode_builders() {
        // Test server mode
        let server_mode = SessionMode::server("example.com").with_domain("updated.com".to_string());
        match server_mode {
            SessionMode::Server { domain, .. } => assert_eq!(domain, Some("updated.com".to_string())),
            _ => panic!("Expected server mode"),
        }
        
        // Test endpoint mode
        let endpoint_mode = SessionMode::endpoint("127.0.0.1:5060".parse().unwrap())
            .with_auth("user".to_string(), "pass".to_string());
        match endpoint_mode {
            SessionMode::Endpoint { auth_username, auth_password, .. } => {
                assert_eq!(auth_username, Some("user".to_string()));
                assert_eq!(auth_password, Some("pass".to_string()));
            },
            _ => panic!("Expected endpoint mode"),
        }
    }
} 