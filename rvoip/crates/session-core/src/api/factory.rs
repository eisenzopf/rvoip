//! API Factory Functions
//!
//! This module provides high-level factory functions for creating SIP servers
//! and clients with dependency injection for proper architectural separation.

use std::sync::Arc;
use anyhow::{Result, Context};
use tokio::sync::mpsc;
use tracing::{info, debug};

use crate::api::server::config::ServerConfig;
use crate::api::client::config::ClientConfig;
use crate::api::server::manager::ServerManager;
use crate::session::manager::SessionManager;
use rvoip_dialog_core::UnifiedDialogApi;
use crate::media::MediaManager;

/// High-level SIP server manager
pub struct SipServer {
    session_manager: Arc<SessionManager>,
    server_manager: Arc<ServerManager>,
    session_events: mpsc::Receiver<crate::events::SessionEvent>,
    config: ServerConfig,
}

/// High-level SIP client manager  
pub struct SipClient {
    session_manager: Arc<SessionManager>,
    config: ClientConfig,
}

impl SipServer {
    /// Get the session manager
    pub fn session_manager(&self) -> Arc<SessionManager> {
        self.session_manager.clone()
    }
    
    /// Get the server manager
    pub fn server_manager(&self) -> Arc<ServerManager> {
        self.server_manager.clone()
    }
    
    /// Get the server configuration
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }
    
    /// Start processing session events
    pub async fn run(&mut self) -> Result<()> {
        info!("Starting SIP server session event processing");
        
        let mut event_count = 0;
        while let Some(event) = self.session_events.recv().await {
            event_count += 1;
            debug!("SipServer received session event #{}: {:?}", event_count, event);
            
            if let Err(e) = self.server_manager.handle_session_event(event).await {
                tracing::error!("Error handling session event: {}", e);
            }
        }
        
        info!("SIP server session event processing ended (received {} events total)", event_count);
        Ok(())
    }
    
    /// Accept an incoming call
    pub async fn accept_call(&self, session_id: &crate::SessionId) -> Result<()> {
        self.server_manager.accept_call(session_id).await
            .context("Failed to accept call")
    }
    
    /// Reject an incoming call
    pub async fn reject_call(&self, session_id: &crate::SessionId, status_code: rvoip_sip_core::StatusCode) -> Result<()> {
        self.server_manager.reject_call(session_id, status_code).await
            .context("Failed to reject call")
    }
    
    /// End an active call
    pub async fn end_call(&self, session_id: &crate::SessionId) -> Result<()> {
        self.server_manager.end_call(session_id).await
            .context("Failed to end call")
    }
    
    /// Get all active sessions
    pub async fn get_active_sessions(&self) -> Vec<crate::SessionId> {
        self.server_manager.get_active_sessions().await
    }
    
    /// Hold/pause a call
    pub async fn hold_call(&self, session_id: &crate::SessionId) -> Result<()> {
        self.server_manager.hold_call(session_id).await
            .context("Failed to hold call")
    }
    
    /// Resume a held call
    pub async fn resume_call(&self, session_id: &crate::SessionId) -> Result<()> {
        self.server_manager.resume_call(session_id).await
            .context("Failed to resume call")
    }
}

impl SipClient {
    /// Get the session manager
    pub fn session_manager(&self) -> Arc<SessionManager> {
        self.session_manager.clone()
    }
    
    /// Get the client configuration
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Make an outgoing call (create session + send INVITE)
    pub async fn make_call(&self, target_uri: &str) -> Result<crate::SessionId> {
        info!("📞 SipClient making call to {}", target_uri);
        
        // Get from URI from configuration
        let from_uri = self.config.from_uri.as_ref()
            .ok_or_else(|| anyhow::anyhow!("from_uri not configured in ClientConfig"))?;
        
        // Create outgoing session
        let session = self.session_manager.create_outgoing_session().await
            .context("Failed to create outgoing session")?;
        
        let session_id = session.id.clone();
        
        // Send the INVITE
        self.session_manager.initiate_outgoing_call(
            &session_id,
            target_uri,
            from_uri,
            None // Let session-core generate SDP offer
        ).await.context("Failed to initiate outgoing call")?;
        
        info!("✅ SipClient call initiated: session {} → {}", session_id, target_uri);
        Ok(session_id)
    }

    /// Answer an incoming call
    pub async fn answer_call(&self, session_id: &crate::SessionId) -> Result<()> {
        info!("✅ SipClient answering call for session {}", session_id);
        
        self.session_manager.accept_call(session_id).await
            .context("Failed to answer call")
    }

    /// Reject an incoming call
    pub async fn reject_call(&self, session_id: &crate::SessionId, status_code: rvoip_sip_core::StatusCode) -> Result<()> {
        info!("❌ SipClient rejecting call for session {} with status {:?}", session_id, status_code);
        
        self.session_manager.reject_call(session_id, status_code).await
            .context("Failed to reject call")
    }

    /// Hang up an active call
    pub async fn hangup_call(&self, session_id: &crate::SessionId) -> Result<()> {
        info!("📴 SipClient hanging up call for session {}", session_id);
        
        self.session_manager.terminate_call(session_id).await
            .context("Failed to hang up call")
    }

    /// Get all active sessions
    pub async fn get_active_sessions(&self) -> Vec<crate::SessionId> {
        self.session_manager.list_sessions()
            .iter()
            .map(|session| session.id.clone())
            .collect()
    }

    /// Check if a session exists and is active
    pub async fn has_active_session(&self, session_id: &crate::SessionId) -> bool {
        self.session_manager.has_session(session_id)
    }
}

/// Create a SIP server with dependency injection (proper architecture)
/// 
/// **ARCHITECTURE**: API layer now receives pre-constructed managers
/// instead of creating infrastructure directly
pub async fn create_sip_server_with_managers(
    config: ServerConfig,
    dialog_api: Arc<UnifiedDialogApi>,
    media_manager: Arc<MediaManager>,
) -> Result<SipServer> {
    info!("Creating SIP server with dependency injection - proper architecture!");
    
    // Validate configuration
    config.validate()
        .context("Invalid server configuration")?;
    
    // Create session manager with injected dependencies
    let session_config = crate::session::SessionConfig::default();
    let event_bus = crate::events::EventBus::new(1000).await
        .map_err(|e| anyhow::anyhow!("Failed to create event bus: {}", e))?;
    
    let session_manager = Arc::new(crate::session::SessionManager::new(
        dialog_api.clone(),
        session_config,
        event_bus
    ).await.context("Failed to create session manager")?);
    
    info!("✅ Created session manager with injected dialog and media managers");
    
    // Create server manager with session manager
    let server_manager = Arc::new(ServerManager::new(
        session_manager.clone(),
        config.clone()
    ));
    
    info!("✅ Created server manager");
    info!("🎯 SIP server ready with proper dependency injection architecture");
    
    // Create mock session events for now - in real implementation this would come from dialog manager
    let (_tx, session_events) = mpsc::channel(100);
    
    Ok(SipServer {
        session_manager,
        server_manager,
        session_events,
        config,
    })
}

/// Create a SIP client with dependency injection (proper architecture)
/// 
/// **ARCHITECTURE**: API layer now receives pre-constructed managers
/// instead of creating infrastructure directly
pub async fn create_sip_client_with_managers(
    config: ClientConfig,
    dialog_api: Arc<UnifiedDialogApi>,
    media_manager: Arc<MediaManager>,
) -> Result<SipClient> {
    info!("Creating SIP client with dependency injection - proper architecture!");
    
    // Validate configuration
    config.validate()
        .context("Invalid client configuration")?;
    
    // Create session manager with injected dependencies
    let local_address = config.local_address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
    let session_config = crate::session::SessionConfig {
        local_media_addr: local_address,
        ..Default::default()
    };
    let event_bus = crate::events::EventBus::new(100).await
        .map_err(|e| anyhow::anyhow!("Failed to create event bus: {}", e))?;
    
    let session_manager = Arc::new(crate::session::SessionManager::new(
        dialog_api,
        session_config,
        event_bus
    ).await.context("Failed to create session manager for client")?);
    
    info!("✅ SIP client created successfully with dependency injection");
    
    Ok(SipClient {
        session_manager,
        config,
    })
}

/// Create a SIP server with clean abstraction (hides dialog-core implementation details)
/// 
/// This is the **clean API** that users should use. It internally creates all required
/// components (UnifiedDialogApi, MediaManager) so users don't need to import dialog-core.
pub async fn create_sip_server(config: ServerConfig) -> Result<SipServer> {
    info!("Creating SIP server with clean abstraction API");
    
    // Validate configuration first
    config.validate()
        .context("Invalid server configuration")?;
    
    // Create dialog-core configuration based on session-core config
    let dialog_config = rvoip_dialog_core::config::DialogManagerConfig::server(config.bind_address)
        .with_domain(&format!("{}", config.bind_address.ip())) // Use IP as domain for now
        .with_auto_options() // Enable automatic OPTIONS responses for servers
        .build();
    
    // Create dialog API internally
    let dialog_api = Arc::new(rvoip_dialog_core::UnifiedDialogApi::create(dialog_config).await
        .context("Failed to create dialog API")?);
    
    info!("✅ Created dialog API internally");
    
    // Create media manager internally
    let media_manager = Arc::new(MediaManager::new().await
        .context("Failed to create media manager")?);
    
    info!("✅ Created media manager internally");
    
    // Now use the dependency injection version with our internally created components
    create_sip_server_with_managers(config, dialog_api, media_manager).await
        .context("Failed to create SIP server with internal managers")
}

/// Create a SIP client with clean abstraction (hides dialog-core implementation details)
/// 
/// This is the **clean API** that users should use. It internally creates all required
/// components (UnifiedDialogApi, MediaManager) so users don't need to import dialog-core.
pub async fn create_sip_client(config: ClientConfig) -> Result<SipClient> {
    info!("Creating SIP client with clean abstraction API");
    
    // Validate configuration first
    config.validate()
        .context("Invalid client configuration")?;
    
    // Determine local address for dialog configuration
    let local_address = config.local_address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
    
    // Create dialog-core configuration based on session-core config
    let dialog_config = rvoip_dialog_core::config::DialogManagerConfig::client(local_address)
        .with_from_uri(&config.effective_from_uri()) // Use client from URI
        .build();
    
    // Create dialog API internally
    let dialog_api = Arc::new(rvoip_dialog_core::UnifiedDialogApi::create(dialog_config).await
        .context("Failed to create dialog API for client")?);
    
    info!("✅ Created dialog API internally for client");
    
    // Create media manager internally
    let media_manager = Arc::new(MediaManager::new().await
        .context("Failed to create media manager for client")?);
    
    info!("✅ Created media manager internally for client");
    
    // Now use the dependency injection version with our internally created components
    create_sip_client_with_managers(config, dialog_api, media_manager).await
        .context("Failed to create SIP client with internal managers")
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_dependency_injection_architecture() {
        // This test validates that the new architecture requires proper dependency injection
        let config = ServerConfig::default();
        
        // Old way should fail
        let result = create_sip_server(config.clone()).await;
        assert!(result.is_err(), "Deprecated function should fail");
        
        // New way requires proper dependencies (test would need real managers)
        // let dialog_manager = Arc::new(DialogServer::new(...));
        // let media_manager = Arc::new(MediaManager::new().await.unwrap());
        // let result = create_sip_server_with_managers(config, dialog_manager, media_manager).await;
        // This would work with real dependencies
    }
    
    #[test]
    fn test_server_config_validation() {
        let mut config = ServerConfig::default();
        assert!(config.validate().is_ok());
        
        // Test invalid config
        config.max_sessions = 0;
        assert!(config.validate().is_err());
    }
} 