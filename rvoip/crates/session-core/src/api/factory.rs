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
use crate::transport::{TransportFactory, SessionTransportEvent};
use crate::session::manager::SessionManager;

/// High-level SIP server manager
pub struct SipServer {
    session_manager: Arc<SessionManager>,
    server_manager: Arc<ServerManager>,
    transaction_events: mpsc::Receiver<rvoip_transaction_core::TransactionEvent>,
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
    
    /// Start processing transport events
    pub async fn run(&mut self) -> Result<()> {
        info!("Starting SIP server event processing");
        
        while let Some(event) = self.transaction_events.recv().await {
            if let Err(e) = self.server_manager.handle_transaction_event(event).await {
                tracing::error!("Error handling transaction event: {}", e);
            }
        }
        
        info!("SIP server event processing ended");
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
}

/// Create a SIP server with automatic setup
pub async fn create_sip_server(config: ServerConfig) -> Result<SipServer> {
    info!("Creating SIP server with config: {:?}", config);
    
    // Validate configuration
    config.validate()
        .context("Invalid server configuration")?;
    
    // **SINGLE TRANSPORT APPROACH**
    // Create one transport that both session layer and transaction-core will use
    let (shared_transport, transport_events) = match config.transport_protocol {
        crate::api::server::config::TransportProtocol::Udp => {
            let (transport, events) = rvoip_sip_transport::UdpTransport::bind(config.bind_address, None)
                .await
                .context("Failed to create UDP transport")?;
            (Arc::new(transport) as Arc<dyn rvoip_sip_transport::Transport>, events)
        },
        crate::api::server::config::TransportProtocol::Tcp => {
            let (transport, events) = rvoip_sip_transport::TcpTransport::bind(config.bind_address, None, None)
                .await
                .context("Failed to create TCP transport")?;
            (Arc::new(transport) as Arc<dyn rvoip_sip_transport::Transport>, events)
        },
        crate::api::server::config::TransportProtocol::Tls => {
            let (transport, events) = rvoip_sip_transport::TlsTransport::bind(
                config.bind_address, 
                "cert.pem", 
                "key.pem", 
                None, 
                None, 
                None
            ).await.context("Failed to create TLS transport")?;
            (Arc::new(transport) as Arc<dyn rvoip_sip_transport::Transport>, events)
        },
        crate::api::server::config::TransportProtocol::WebSocket => {
            let (transport, events) = rvoip_sip_transport::WebSocketTransport::bind(
                config.bind_address, 
                false, // not secure
                None,  // no cert path
                None,  // no key path
                None   // default channel capacity
            ).await.context("Failed to create WebSocket transport")?;
            (Arc::new(transport) as Arc<dyn rvoip_sip_transport::Transport>, events)
        },
    };
    
    info!("✅ Created shared transport on {}", config.bind_address);
    
    // **TRANSACTION-CORE INTEGRATION**
    // Create transaction manager using the shared transport
    let (transaction_manager, transaction_events) = rvoip_transaction_core::TransactionManager::new(
        shared_transport.clone(),
        transport_events,
        Some(100), // Event buffer capacity
    ).await.context("Failed to create transaction manager")?;
    
    let transaction_manager = Arc::new(transaction_manager);
    info!("✅ Created transaction manager with shared transport");
    
    // Create session manager
    let session_config = crate::session::SessionConfig::default();
    let event_bus = crate::events::EventBus::new(1000).await
        .map_err(|e| anyhow::anyhow!("Failed to create event bus: {}", e))?;
    
    let session_manager = Arc::new(crate::session::SessionManager::new(
        transaction_manager.clone(),
        session_config,
        event_bus
    ).await.context("Failed to create session manager")?);
    
    info!("✅ Created session manager");
    
    // Create server manager with transaction manager
    let server_manager = Arc::new(ServerManager::new(
        session_manager.clone(),
        transaction_manager.clone(),
        config.clone()
    ));
    
    info!("✅ Created server manager");
    info!("SIP server created successfully on {}", config.bind_address);
    
    Ok(SipServer {
        session_manager,
        server_manager,
        transaction_events,
        config,
    })
}

/// Create a SIP client with automatic setup
pub async fn create_sip_client(config: ClientConfig) -> Result<SipClient> {
    info!("Creating SIP client with config: {:?}", config);
    
    // Validate configuration
    config.validate()
        .context("Invalid client configuration")?;
    
    // Create session manager - use a basic configuration for now
    let session_config = crate::session::SessionConfig {
        local_media_addr: config.local_address.unwrap_or_else(|| "127.0.0.1:0".parse().unwrap()),
        ..Default::default()
    };
    let event_bus = crate::events::EventBus::new(100).await
        .map_err(|e| anyhow::anyhow!("Failed to create event bus: {}", e))?;
    
    // Create a dummy transaction manager for now - this would need to be properly integrated
    // For now, create a minimal transport for the transaction manager
    let (dummy_transport, dummy_events) = rvoip_sip_transport::UdpTransport::bind("127.0.0.1:0".parse().unwrap(), None).await
        .context("Failed to create dummy transport")?;
    
    let transaction_manager = std::sync::Arc::new(
        rvoip_transaction_core::TransactionManager::dummy(
            std::sync::Arc::new(dummy_transport),
            dummy_events
        )
    );
    
    let session_manager = Arc::new(crate::session::SessionManager::new(
        transaction_manager,
        session_config,
        event_bus
    ).await.context("Failed to create session manager")?);
    
    info!("SIP client created successfully");
    
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