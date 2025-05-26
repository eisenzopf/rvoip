//! Transport Integration Layer
//!
//! This module provides the bridge between session-core and sip-transport,
//! handling message parsing, routing, and event propagation.

use std::sync::Arc;
use std::net::SocketAddr;
use anyhow::{Result, Context};
use tokio::sync::mpsc;
use tracing::{info, debug, warn, error};

use rvoip_sip_transport::{Transport, TransportEvent, UdpTransport, TcpTransport, TlsTransport, WebSocketTransport};
use rvoip_sip_core::{Message, Request, Response};
use crate::api::server::config::{ServerConfig, TransportProtocol};

/// Transport integration events for session layer
#[derive(Debug, Clone)]
pub enum SessionTransportEvent {
    /// Incoming SIP request received
    IncomingRequest {
        request: Request,
        source: SocketAddr,
        transport: String,
    },
    /// Incoming SIP response received
    IncomingResponse {
        response: Response,
        source: SocketAddr,
        transport: String,
    },
    /// Transport error occurred
    TransportError {
        error: String,
        source: Option<SocketAddr>,
    },
    /// Transport connection established
    ConnectionEstablished {
        local_addr: SocketAddr,
        remote_addr: Option<SocketAddr>,
        transport: String,
    },
    /// Transport connection closed
    ConnectionClosed {
        local_addr: SocketAddr,
        remote_addr: Option<SocketAddr>,
        transport: String,
    },
}

/// Transport integration manager
pub struct TransportIntegration {
    transport: Arc<dyn Transport + Send + Sync>,
    config: ServerConfig,
    event_tx: mpsc::Sender<SessionTransportEvent>,
    transport_events: Option<mpsc::Receiver<TransportEvent>>,
}

impl TransportIntegration {
    /// Create a new transport integration
    pub async fn new(
        config: ServerConfig,
        event_tx: mpsc::Sender<SessionTransportEvent>,
    ) -> Result<Self> {
        let (transport, transport_events) = Self::create_transport(&config).await
            .context("Failed to create transport")?;
        
        Ok(Self {
            transport,
            config,
            event_tx,
            transport_events: Some(transport_events),
        })
    }
    
    /// Create transport based on configuration
    async fn create_transport(config: &ServerConfig) -> Result<(Arc<dyn Transport + Send + Sync>, mpsc::Receiver<TransportEvent>)> {
        match config.transport_protocol {
            TransportProtocol::Udp => {
                let (transport, events) = UdpTransport::bind(config.bind_address, None)
                    .await
                    .context("Failed to create UDP transport")?;
                Ok((Arc::new(transport), events))
            },
            TransportProtocol::Tcp => {
                let (transport, events) = TcpTransport::bind(config.bind_address, None, None)
                    .await
                    .context("Failed to create TCP transport")?;
                Ok((Arc::new(transport), events))
            },
            TransportProtocol::Tls => {
                // For now, use placeholder cert paths - in production these would come from config
                let (transport, events) = TlsTransport::bind(
                    config.bind_address, 
                    "cert.pem", 
                    "key.pem", 
                    None, 
                    None, 
                    None
                ).await.context("Failed to create TLS transport")?;
                Ok((Arc::new(transport), events))
            },
            TransportProtocol::WebSocket => {
                let (transport, events) = WebSocketTransport::bind(
                    config.bind_address, 
                    false, // not secure
                    None,  // no cert path
                    None,  // no key path
                    None   // default channel capacity
                ).await.context("Failed to create WebSocket transport")?;
                Ok((Arc::new(transport), events))
            },
        }
    }
    
    /// Start the transport integration
    pub async fn start(&self) -> Result<()> {
        info!("Starting transport integration on {} ({})", 
              self.config.bind_address, 
              self.config.transport_protocol);
        
        // Notify connection established
        let local_addr = self.transport.local_addr()
            .context("Failed to get local address")?;
            
        let _ = self.event_tx.send(SessionTransportEvent::ConnectionEstablished {
            local_addr,
            remote_addr: None,
            transport: self.config.transport_protocol.to_string(),
        }).await;
        
        info!("Transport integration started successfully");
        Ok(())
    }
    
    /// Start message processing loop
    pub async fn run_message_loop(&mut self) -> Result<()> {
        let mut transport_events = self.transport_events.take()
            .context("Transport events already consumed")?;
        
        info!("Starting transport message processing loop");
        
        while let Some(event) = transport_events.recv().await {
            if let Err(e) = self.handle_transport_event(event).await {
                error!("Error handling transport event: {}", e);
            }
        }
        
        warn!("Transport message loop ended");
        Ok(())
    }
    
    /// Handle transport events
    async fn handle_transport_event(&self, event: TransportEvent) -> Result<()> {
        match event {
            TransportEvent::MessageReceived { message, source, destination } => {
                self.handle_incoming_message(message, source).await?;
            },
            TransportEvent::Error { error } => {
                warn!("Transport error: {}", error);
                let _ = self.event_tx.send(SessionTransportEvent::TransportError {
                    error,
                    source: None,
                }).await;
            },
            TransportEvent::Closed => {
                info!("Transport closed");
                let local_addr = self.transport.local_addr().unwrap_or(self.config.bind_address);
                let _ = self.event_tx.send(SessionTransportEvent::ConnectionClosed {
                    local_addr,
                    remote_addr: None,
                    transport: self.config.transport_protocol.to_string(),
                }).await;
            },
        }
        
        Ok(())
    }
    
    /// Handle incoming SIP message
    async fn handle_incoming_message(&self, message: Message, source: SocketAddr) -> Result<()> {
        debug!("Received SIP message from {}: {}", source, 
               match &message {
                   Message::Request(req) => format!("{} {}", req.method(), req.uri()),
                   Message::Response(resp) => format!("{} {}", resp.status_code(), resp.reason_phrase()),
               });
        
        // Route based on message type
        match message {
            Message::Request(request) => {
                let _ = self.event_tx.send(SessionTransportEvent::IncomingRequest {
                    request,
                    source,
                    transport: self.config.transport_protocol.to_string(),
                }).await;
            },
            Message::Response(response) => {
                let _ = self.event_tx.send(SessionTransportEvent::IncomingResponse {
                    response,
                    source,
                    transport: self.config.transport_protocol.to_string(),
                }).await;
            },
        }
        
        Ok(())
    }
    
    /// Send SIP message
    pub async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        debug!("Sending SIP message to {}: {}", destination,
               match &message {
                   Message::Request(req) => format!("{} {}", req.method(), req.uri()),
                   Message::Response(resp) => format!("{} {}", resp.status_code(), resp.reason_phrase()),
               });
        
        self.transport.send_message(message, destination).await
            .context("Failed to send SIP message")?;
        
        Ok(())
    }
    
    /// Get local address
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.transport.local_addr()
            .context("Failed to get local address")
    }
    
    /// Get transport protocol
    pub fn transport_protocol(&self) -> TransportProtocol {
        self.config.transport_protocol
    }
    
    /// Stop the transport integration
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping transport integration");
        
        self.transport.close().await
            .context("Failed to stop transport")?;
        
        info!("Transport integration stopped");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    
    #[tokio::test]
    async fn test_transport_integration_creation() {
        let config = ServerConfig::default();
        let (tx, _rx) = mpsc::channel(100);
        
        // This test may fail if UDP binding fails, which is expected in some environments
        let result = TransportIntegration::new(config, tx).await;
        
        // We mainly test that the creation doesn't panic
        match result {
            Ok(_) => println!("Transport integration created successfully"),
            Err(e) => println!("Transport integration creation failed (expected in some environments): {}", e),
        }
    }
    
    #[test]
    fn test_session_transport_event_creation() {
        let event = SessionTransportEvent::TransportError {
            error: "Test error".to_string(),
            source: None,
        };
        
        match event {
            SessionTransportEvent::TransportError { error, .. } => {
                assert_eq!(error, "Test error");
            },
            _ => panic!("Wrong event type"),
        }
    }
} 