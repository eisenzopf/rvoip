//! Transport Factory
//!
//! This module provides factory functions for creating transport integrations
//! based on configuration, with protocol-specific setup and lifecycle management.

use std::sync::Arc;
use anyhow::{Result, Context};
use tokio::sync::mpsc;
use tracing::{info, debug};

use crate::api::server::config::{ServerConfig, TransportProtocol};
use crate::transport::integration::{TransportIntegration, SessionTransportEvent};

/// Factory for creating transport integrations
pub struct TransportFactory;

impl TransportFactory {
    /// Create a transport integration from configuration
    pub async fn create_transport(
        config: ServerConfig,
        event_tx: mpsc::Sender<SessionTransportEvent>,
    ) -> Result<Arc<TransportIntegration>> {
        info!("Creating transport integration for {} on {}", 
              config.transport_protocol, 
              config.bind_address);
        
        // Validate configuration
        config.validate()
            .context("Invalid server configuration")?;
        
        // Create transport integration
        let integration = TransportIntegration::new(config.clone(), event_tx).await
            .context("Failed to create transport integration")?;
        
        debug!("Transport integration created successfully for {}", 
               config.transport_protocol);
        
        Ok(Arc::new(integration))
    }
    
    /// Create and start a transport integration
    pub async fn create_and_start_transport(
        config: ServerConfig,
        event_tx: mpsc::Sender<SessionTransportEvent>,
    ) -> Result<Arc<TransportIntegration>> {
        let integration = Self::create_transport(config, event_tx).await?;
        
        // Start the transport
        integration.start().await
            .context("Failed to start transport integration")?;
        
        info!("Transport integration started successfully");
        Ok(integration)
    }
    
    /// Get default configuration for a transport protocol
    pub fn default_config_for_protocol(protocol: TransportProtocol) -> ServerConfig {
        let mut config = ServerConfig::default();
        config.transport_protocol = protocol;
        
        // Adjust default settings based on protocol
        match protocol {
            TransportProtocol::Udp => {
                // UDP defaults are already set
            },
            TransportProtocol::Tcp => {
                // For TCP, we might want to increase timeouts to handle connection overhead
                config.transaction_timeout = std::time::Duration::from_secs(64);
            },
            TransportProtocol::Tls => {
                // For TLS, we need longer timeouts due to handshake overhead
                config.transaction_timeout = std::time::Duration::from_secs(64);
            },
            TransportProtocol::WebSocket => {
                // WebSocket connections might need different timing
            },
            TransportProtocol::WebSocketSecure => {
                // WebSocket Secure connections need additional handshake time
                config.transaction_timeout = std::time::Duration::from_secs(64);
            },
        }
        
        config
    }
    
    /// Validate transport configuration
    pub fn validate_transport_config(config: &ServerConfig) -> Result<()> {
        config.validate()?;
        
        // Additional transport-specific validation
        match config.transport_protocol {
            TransportProtocol::Udp => {
                // UDP-specific validation
                if config.bind_address.port() == 0 {
                    return Err(anyhow::anyhow!("UDP transport requires a specific port"));
                }
            },
            TransportProtocol::Tcp => {
                // TCP-specific validation
                if config.max_sessions > 10_000 {
                    return Err(anyhow::anyhow!("TCP transport should limit max sessions for connection management"));
                }
            },
            TransportProtocol::Tls => {
                // TLS-specific validation
                if config.bind_address.port() == 5060 {
                    return Err(anyhow::anyhow!("TLS transport should not use standard SIP port 5060"));
                }
            },
            TransportProtocol::WebSocket => {
                // WebSocket-specific validation
                if config.bind_address.port() == 5060 || config.bind_address.port() == 5061 {
                    return Err(anyhow::anyhow!("WebSocket transport should not use standard SIP ports"));
                }
            },
            TransportProtocol::WebSocketSecure => {
                // WebSocket Secure-specific validation
                if config.bind_address.port() == 5060 || config.bind_address.port() == 5061 {
                    return Err(anyhow::anyhow!("WebSocket Secure transport should not use standard SIP ports"));
                }
            },
        }
        
        Ok(())
    }
    
    /// Get recommended buffer sizes for transport protocol
    pub fn recommended_buffer_sizes(protocol: TransportProtocol) -> (usize, usize) {
        match protocol {
            TransportProtocol::Udp => (1000, 1000), // (send_buffer, recv_buffer)
            TransportProtocol::Tcp => (500, 500),   // TCP has connection management
            TransportProtocol::Tls => (200, 200),   // TLS has encryption overhead
            TransportProtocol::WebSocket => (300, 300), // WebSocket has framing overhead
            TransportProtocol::WebSocketSecure => (250, 250), // WebSocket Secure has framing + encryption overhead
        }
    }
    
    /// Create event channel with appropriate buffer size
    pub fn create_event_channel(protocol: TransportProtocol) -> (mpsc::Sender<SessionTransportEvent>, mpsc::Receiver<SessionTransportEvent>) {
        let (_, buffer_size) = Self::recommended_buffer_sizes(protocol);
        mpsc::channel(buffer_size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    
    #[test]
    fn test_default_config_for_protocols() {
        let udp_config = TransportFactory::default_config_for_protocol(TransportProtocol::Udp);
        assert_eq!(udp_config.transport_protocol, TransportProtocol::Udp);
        assert_eq!(udp_config.bind_address.port(), 5060);
        
        let tls_config = TransportFactory::default_config_for_protocol(TransportProtocol::Tls);
        assert_eq!(tls_config.transport_protocol, TransportProtocol::Tls);
        assert_eq!(tls_config.bind_address.port(), 5061);
        
        let ws_config = TransportFactory::default_config_for_protocol(TransportProtocol::WebSocket);
        assert_eq!(ws_config.transport_protocol, TransportProtocol::WebSocket);
        assert_eq!(ws_config.bind_address.port(), 8080);
    }
    
    #[test]
    fn test_transport_validation() {
        let mut config = ServerConfig::default();
        
        // Valid UDP config
        config.transport_protocol = TransportProtocol::Udp;
        assert!(TransportFactory::validate_transport_config(&config).is_ok());
        
        // Invalid TLS config (using standard SIP port)
        config.transport_protocol = TransportProtocol::Tls;
        config.bind_address = "127.0.0.1:5060".parse().unwrap();
        assert!(TransportFactory::validate_transport_config(&config).is_err());
        
        // Valid TLS config
        config.bind_address = "127.0.0.1:5061".parse().unwrap();
        assert!(TransportFactory::validate_transport_config(&config).is_ok());
    }
    
    #[test]
    fn test_buffer_sizes() {
        let (send, recv) = TransportFactory::recommended_buffer_sizes(TransportProtocol::Udp);
        assert_eq!(send, 1000);
        assert_eq!(recv, 1000);
        
        let (send, recv) = TransportFactory::recommended_buffer_sizes(TransportProtocol::Tcp);
        assert_eq!(send, 500);
        assert_eq!(recv, 500);
    }
    
    #[test]
    fn test_event_channel_creation() {
        let (tx, mut rx) = TransportFactory::create_event_channel(TransportProtocol::Udp);
        
        // Test that we can send and receive
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let event = SessionTransportEvent::TransportError {
                error: "test".to_string(),
                source: None,
            };
            
            tx.send(event).await.unwrap();
            let received = rx.recv().await.unwrap();
            
            match received {
                SessionTransportEvent::TransportError { error, .. } => {
                    assert_eq!(error, "test");
                },
                _ => panic!("Wrong event type"),
            }
        });
    }
} 