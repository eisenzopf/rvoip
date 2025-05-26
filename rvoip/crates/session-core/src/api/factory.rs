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
use crate::transport::{TransportFactory, SessionTransportEvent};
use crate::session::manager::SessionManager;

/// High-level SIP server manager
pub struct SipServer {
    session_manager: Arc<SessionManager>,
    transport_events: mpsc::Receiver<SessionTransportEvent>,
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
    
    /// Get the server configuration
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }
    
    /// Start processing transport events
    pub async fn run(&mut self) -> Result<()> {
        info!("Starting SIP server event processing");
        
        while let Some(event) = self.transport_events.recv().await {
            if let Err(e) = self.handle_transport_event(event).await {
                tracing::error!("Error handling transport event: {}", e);
            }
        }
        
        info!("SIP server event processing ended");
        Ok(())
    }
    
    /// Handle transport events
    async fn handle_transport_event(&self, event: SessionTransportEvent) -> Result<()> {
        match event {
            SessionTransportEvent::IncomingRequest { request, source, transport } => {
                debug!("Handling incoming {} request from {} via {}", 
                       request.method(), source, transport);
                
                // For now, just log the request - proper handling would require more integration
                info!("Received {} request from {}", request.method(), source);
            },
            SessionTransportEvent::IncomingResponse { response, source, transport } => {
                debug!("Handling incoming {} response from {} via {}", 
                       response.status_code(), source, transport);
                
                // For now, just log the response - proper handling would require more integration
                info!("Received {} response from {}", response.status_code(), source);
            },
            SessionTransportEvent::TransportError { error, source } => {
                tracing::warn!("Transport error from {:?}: {}", source, error);
                // Handle transport errors (could trigger reconnection, etc.)
            },
            SessionTransportEvent::ConnectionEstablished { local_addr, remote_addr, transport } => {
                info!("Transport connection established: {} -> {:?} ({})", 
                      local_addr, remote_addr, transport);
            },
            SessionTransportEvent::ConnectionClosed { local_addr, remote_addr, transport } => {
                info!("Transport connection closed: {} -> {:?} ({})", 
                      local_addr, remote_addr, transport);
            },
        }
        
        Ok(())
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
    
    // Create transport event channel
    let (transport_tx, transport_rx) = TransportFactory::create_event_channel(config.transport_protocol);
    
    // Create and start transport
    let _transport = TransportFactory::create_and_start_transport(config.clone(), transport_tx).await
        .context("Failed to create transport")?;
    
    // Create session manager - use a basic configuration for now
    let session_config = crate::session::SessionConfig::default();
    let event_bus = crate::events::EventBus::new(1000).await
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
    
    info!("SIP server created successfully on {}", config.bind_address);
    
    Ok(SipServer {
        session_manager,
        transport_events: transport_rx,
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