use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::select;
use tracing::{debug, error, info, trace, warn};

use rvoip_sip_core::Message;
use crate::factory::{TransportFactory, TransportType, TransportFactoryConfig};
use crate::transport::{Transport, TransportEvent};
use crate::error::{Error, Result};

/// Configuration for the transport manager
#[derive(Clone, Debug)]
pub struct TransportManagerConfig {
    /// Configuration for the underlying transport factory
    pub factory_config: TransportFactoryConfig,
    /// Maximum number of messages to buffer
    pub max_event_buffer: usize,
}

impl Default for TransportManagerConfig {
    fn default() -> Self {
        Self {
            factory_config: TransportFactoryConfig::default(),
            max_event_buffer: 1000,
        }
    }
}

/// Manages multiple SIP transports and routes messages
pub struct TransportManager {
    /// Configuration for this manager
    config: TransportManagerConfig,
    /// Transport factory for creating new transports
    factory: TransportFactory,
    /// Active transports by bound address
    transports: Mutex<HashMap<SocketAddr, Arc<dyn Transport>>>,
    /// Channel for sending events to the application
    event_tx: mpsc::Sender<TransportEvent>,
    /// Task handle for the event listener
    _event_listener_handle: tokio::task::JoinHandle<()>,
}

impl TransportManager {
    /// Creates a new transport manager with the given configuration
    pub async fn new(config: TransportManagerConfig) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let factory = TransportFactory::new(config.factory_config.clone());
        let (event_tx, event_rx) = mpsc::channel(config.max_event_buffer);
        
        let manager = Self {
            config,
            factory,
            transports: Mutex::new(HashMap::new()),
            event_tx: event_tx.clone(),
            _event_listener_handle: tokio::spawn(async {}), // Placeholder, will be replaced
        };
        
        // Initialize the real event listener
        let event_handle = manager.init_event_listener(event_tx.clone());
        
        // Replace the placeholder with the real handle
        let manager = Self {
            config: manager.config,
            factory: manager.factory,
            transports: manager.transports,
            event_tx,
            _event_listener_handle: event_handle,
        };
        
        Ok((manager, event_rx))
    }
    
    /// Creates a new transport manager with default configuration
    pub async fn with_defaults() -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::new(TransportManagerConfig::default()).await
    }
    
    /// Initializes the event listener task that forwards events from all transports
    fn init_event_listener(&self, event_tx: mpsc::Sender<TransportEvent>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            info!("Transport event listener started");
            
            // In a more advanced implementation, we would maintain a list of transport receivers
            // and select across them to forward events to the main channel.
            // Since we can't know all transports at startup, we would need to use a dynamic
            // selection mechanism like tokio::select! with dynamically added branches,
            // or use an mpsc channel for each transport that feeds into a single task.
            
            // For now, this is just a placeholder since individual transports are registered later
        })
    }
    
    /// Registers a transport with the manager
    pub async fn register_transport(&self, transport: Arc<dyn Transport>) -> Result<()> {
        let local_addr = transport.local_addr()?;
        let mut transports = self.transports.lock().await;
        
        // Check if we already have a transport bound to this address
        if transports.contains_key(&local_addr) {
            return Err(Error::AlreadyBound);
        }
        
        // Add the transport to our map
        transports.insert(local_addr, transport);
        info!("Registered transport bound to {}", local_addr);
        
        Ok(())
    }
    
    /// Creates and registers a transport of the specified type
    pub async fn create_transport(
        &self,
        transport_type: TransportType,
        bind_addr: SocketAddr,
    ) -> Result<SocketAddr> {
        let (transport, rx) = self.factory.create_transport(transport_type, bind_addr).await?;
        let actual_addr = transport.local_addr()?;
        
        // Register the transport
        self.register_transport(transport.clone()).await?;
        
        // Forward events from this transport to the main event channel
        self.spawn_event_forwarder(rx);
        
        Ok(actual_addr)
    }
    
    /// Spawns a task to forward events from a transport to the main event channel
    fn spawn_event_forwarder(&self, mut rx: mpsc::Receiver<TransportEvent>) {
        let event_tx = self.event_tx.clone();
        
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Err(e) = event_tx.send(event).await {
                    error!("Failed to forward transport event: {}", e);
                    break;
                }
            }
            
            debug!("Transport event forwarder terminated");
        });
    }
    
    /// Gets a transport for the given address
    pub async fn get_transport_for_addr(&self, addr: &SocketAddr) -> Result<Arc<dyn Transport>> {
        let transports = self.transports.lock().await;
        
        // In a more advanced implementation, we would use knowledge of the SIP protocol
        // to select the best transport based on the destination address, URI parameters,
        // and other factors. For now, we just look for an exact match.
        
        if let Some(transport) = transports.get(addr) {
            return Ok(transport.clone());
        }
        
        // No exact match, try to find a transport bound to the same IP with a different port
        // (typical for clients that bind to any port but send to specific port)
        for (bound_addr, transport) in transports.iter() {
            if bound_addr.ip() == addr.ip() {
                return Ok(transport.clone());
            }
        }
        
        // No matching transport found
        Err(Error::InvalidState(format!("No transport found for address: {}", addr)))
    }
    
    /// Sends a SIP message to the specified destination
    pub async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        // Get a transport for this destination
        let transport = self.get_transport_for_addr(&destination).await?;
        
        // Send the message using the transport
        transport.send_message(message, destination).await
    }
    
    /// Closes all transports
    pub async fn close_all(&self) -> Result<()> {
        let mut transports = self.transports.lock().await;
        
        for (addr, transport) in transports.drain() {
            if let Err(e) = transport.close().await {
                error!("Error closing transport bound to {}: {}", addr, e);
            }
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_udp;
    use rvoip_sip_core::{Method, Request};
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    
    #[tokio::test]
    async fn test_transport_manager_create() {
        let (manager, _rx) = TransportManager::with_defaults().await.unwrap();
        
        // Create a UDP transport
        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let actual_addr = manager.create_transport(TransportType::Udp, bind_addr).await.unwrap();
        
        assert_ne!(actual_addr.port(), 0);
        
        // Clean up
        manager.close_all().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_transport_manager_register() {
        let (manager, _rx) = TransportManager::with_defaults().await.unwrap();
        
        // Create a UDP transport directly
        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (transport, _transport_rx) = bind_udp(bind_addr).await.unwrap();
        let actual_addr = transport.local_addr().unwrap();
        
        // Register it with the manager
        manager.register_transport(Arc::new(transport)).await.unwrap();
        
        // Try to get it back by address
        let retrieved = manager.get_transport_for_addr(&actual_addr).await.unwrap();
        assert_eq!(retrieved.local_addr().unwrap(), actual_addr);
        
        // Clean up
        manager.close_all().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_transport_manager_send() {
        // Set up a manager
        let (manager, mut rx) = TransportManager::with_defaults().await.unwrap();
        
        // Create server and client transports
        let server_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let server_bound_addr = manager.create_transport(TransportType::Udp, server_addr).await.unwrap();
        
        let client_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let client_bound_addr = manager.create_transport(TransportType::Udp, client_addr).await.unwrap();
        
        // Create a test message
        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag1"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("call1@example.com")
            .cseq(1)
            .build();
        
        // Send from client to server
        manager.send_message(request.into(), server_bound_addr).await.unwrap();
        
        // Wait for the message to be received
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
            .unwrap();
        
        // Verify the event
        match event {
            TransportEvent::MessageReceived { message, source, destination } => {
                assert_eq!(destination, server_bound_addr);
                // Source may be different due to how UDP works
                assert!(message.is_request());
                if let Message::Request(req) = message {
                    assert_eq!(req.method(), Method::Register);
                } else {
                    panic!("Expected a request");
                }
            },
            _ => panic!("Expected MessageReceived event"),
        }
        
        // Clean up
        manager.close_all().await.unwrap();
    }
} 