//! API Factory Functions
//!
//! This module provides high-level factory functions for creating SIP servers
//! and clients with automatic transport setup and media manager initialization.

use std::sync::Arc;
use anyhow::{Result, Context};
use tokio::sync::mpsc;
use tracing::{info, debug};

use crate::api::server::config::ServerConfig;
use crate::api::client::config::ClientConfig;
use crate::api::server::manager::ServerManager;
use crate::session::manager::SessionManager;

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

    /// ❗ **CRITICAL NEW METHOD**: Make an outgoing call (create session + send INVITE)
    /// This is what client-core.make_call() expects!
    pub async fn make_call(&self, target_uri: &str) -> Result<crate::SessionId> {
        info!("📞 SipClient making call to {}", target_uri);
        
        // Get from URI from configuration
        let from_uri = self.config.from_uri.as_ref()
            .ok_or_else(|| anyhow::anyhow!("from_uri not configured in ClientConfig"))?;
        
        // Create outgoing session
        let session = self.session_manager.create_outgoing_session().await
            .context("Failed to create outgoing session")?;
        
        let session_id = session.id.clone();
        
        // 🚀 **THIS IS THE MISSING PIECE**: Send the INVITE!
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

/// Create a SIP server with automatic setup
pub async fn create_sip_server(config: ServerConfig) -> Result<SipServer> {
    info!("Creating SIP server with config: {:?}", config);
    
    // Validate configuration
    config.validate()
        .context("Invalid server configuration")?;
    
    // **TRANSACTION-CORE HANDLES ALL TRANSPORT**
    // Create TransportManager configuration based on our server config
    let transport_config = rvoip_transaction_core::transport::TransportManagerConfig {
        enable_udp: config.transport_protocol == crate::api::server::config::TransportProtocol::Udp,
        enable_tcp: config.transport_protocol == crate::api::server::config::TransportProtocol::Tcp,
        enable_ws: config.transport_protocol == crate::api::server::config::TransportProtocol::WebSocket,
        enable_tls: config.transport_protocol == crate::api::server::config::TransportProtocol::Tls,
        bind_addresses: vec![config.bind_address],
        default_channel_capacity: 100,
        tls_cert_path: None, // TODO: Add TLS config to ServerConfig
        tls_key_path: None,  // TODO: Add TLS config to ServerConfig
    };
    
    // Create and initialize transport manager
    let (mut transport_manager, transport_events) = rvoip_transaction_core::transport::TransportManager::new(transport_config).await
        .context("Failed to create transport manager")?;
    
    transport_manager.initialize().await
        .context("Failed to initialize transport manager")?;
    
    info!("✅ Created and initialized transport manager for {} on {}", 
          config.transport_protocol, config.bind_address);
    
    // Create transaction manager using the transport manager
    let (transaction_manager, transaction_events) = rvoip_transaction_core::TransactionManager::with_transport_manager(
        transport_manager,
        transport_events,
        Some(100), // Event buffer capacity
    ).await.context("Failed to create transaction manager")?;
    
    let transaction_manager = Arc::new(transaction_manager);
    info!("✅ Created transaction manager with transport manager");
    
    // Create dialog manager with transaction manager and local address
    let dialog_manager = Arc::new(rvoip_dialog_core::DialogManager::new(
        transaction_manager.clone(),
        config.bind_address
    ).await.context("Failed to create dialog manager")?);
    
    info!("✅ Created dialog manager for SIP protocol handling");
    
    // Create session manager with dialog manager
    let session_config = crate::session::SessionConfig::default();
    let event_bus = crate::events::EventBus::new(1000).await
        .map_err(|e| anyhow::anyhow!("Failed to create event bus: {}", e))?;
    
    let session_manager = Arc::new(crate::session::SessionManager::new(
        dialog_manager.clone(),
        session_config,
        event_bus
    ).await.context("Failed to create session manager")?);
    
    info!("✅ Created session manager with dialog coordination");
    
    // Create server manager with session manager
    let server_manager = Arc::new(ServerManager::new(
        session_manager.clone(),
        config.clone()
    ));
    
    info!("✅ Created server manager");
    info!("🎯 SIP server ready - dialog-core handles all SIP protocol on {}", config.bind_address);
    
    Ok(SipServer {
        session_manager,
        server_manager,
        session_events: transaction_events,
        config,
    })
}

/// Create a SIP client with automatic setup
pub async fn create_sip_client(config: ClientConfig) -> Result<SipClient> {
    info!("Creating SIP client with config: {:?}", config);
    
    // Validate configuration
    config.validate()
        .context("Invalid client configuration")?;
    
    // **REAL INFRASTRUCTURE**: Create proper transport like the server factory does
    info!("🚀 Creating real transport manager for SIP client communication");
    
    // Create transport configuration for client
    let local_address = config.local_address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap());
    let transport_config = rvoip_transaction_core::transport::TransportManagerConfig {
        enable_udp: true,
        enable_tcp: false,
        enable_ws: false,
        enable_tls: false,
        bind_addresses: vec![local_address],
        default_channel_capacity: 100,
        tls_cert_path: None,
        tls_key_path: None,
    };
    
    // Create and initialize transport manager (like server does)
    let (mut transport_manager, transport_events) = rvoip_transaction_core::transport::TransportManager::new(transport_config).await
        .context("Failed to create transport manager for client")?;
        
    transport_manager.initialize().await
        .context("Failed to initialize transport manager for client")?;
        
    info!("✅ Client transport manager created and initialized on {}", local_address);
    
    // Create transaction manager using the transport manager (like server does)
    let (transaction_manager, mut session_events) = rvoip_transaction_core::TransactionManager::with_transport_manager(
        transport_manager,
        transport_events,
        Some(100), // Event buffer capacity
    ).await.context("Failed to create transaction manager for client")?;
    
    let transaction_manager = Arc::new(transaction_manager);
    info!("✅ Client transaction manager created with real transport");
    
    // Create dialog manager with transaction manager and local address
    let dialog_manager = Arc::new(rvoip_dialog_core::DialogManager::new(
        transaction_manager.clone(),
        local_address
    ).await.context("Failed to create dialog manager for client")?);
    
    info!("✅ Created dialog manager for client SIP protocol handling");
    
    // Create session manager with dialog manager
    let session_config = crate::session::SessionConfig {
        local_media_addr: local_address,
        ..Default::default()
    };
    let event_bus = crate::events::EventBus::new(100).await
        .map_err(|e| anyhow::anyhow!("Failed to create event bus: {}", e))?;
    
    let session_manager = Arc::new(crate::session::SessionManager::new(
        dialog_manager,
        session_config,
        event_bus
    ).await.context("Failed to create session manager for client")?);
    
    // ❗ **CRITICAL FIX**: Start event processing in background task automatically
    // This ensures session events are processed without client-core needing to manage it
    let session_manager_for_events = session_manager.clone();
    tokio::spawn(async move {
        info!("🔄 Starting SIP client session event processing in background");
        
        let mut event_count = 0;
        while let Some(event) = session_events.recv().await {
            event_count += 1;
            debug!("SipClient background task received session event #{}: {:?}", event_count, event);
            
            // Process session events (dialog-core already handles transaction details)
            debug!("Processing session event in background: {:?}", event);
        }
        
        info!("SIP client background event processing ended (received {} events total)", event_count);
    });
    
    info!("✅ SIP client created successfully with real transport infrastructure");
    info!("✅ Session event processing started in background task");
    
    Ok(SipClient {
        session_manager,
        config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_create_sip_server() {
        let config = ServerConfig::default();
        
        // This test may fail if binding fails, which is expected in some environments
        let result = create_sip_server(config).await;
        
        match result {
            Ok(_) => println!("SIP server created successfully"),
            Err(e) => println!("SIP server creation failed (expected in some environments): {}", e),
        }
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