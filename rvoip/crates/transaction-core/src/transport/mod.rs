use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, trace, warn};

use rvoip_sip_core::Message;
use rvoip_sip_transport::{
    Transport, TransportEvent, UdpTransport, 
    TcpTransport, WebSocketTransport,
    error::{Error as TransportError, Result as TransportResult}
};
use rvoip_sip_transport::transport::TransportType;
use rvoip_sip_transport::factory::{TransportFactory, TransportFactoryConfig};

use crate::error::{Error, Result};

/// Configuration options for the TransportManager
#[derive(Debug, Clone)]
pub struct TransportManagerConfig {
    /// Whether to enable UDP transport
    pub enable_udp: bool,
    /// Whether to enable TCP transport
    pub enable_tcp: bool,
    /// Whether to enable WebSocket transport
    pub enable_ws: bool,
    /// Whether to enable TLS (for TCP and WebSocket)
    pub enable_tls: bool,
    /// Local addresses to bind to (if empty, will bind to all interfaces)
    pub bind_addresses: Vec<SocketAddr>,
    /// Default event channel capacity
    pub default_channel_capacity: usize,
    /// TLS certificate path
    pub tls_cert_path: Option<String>,
    /// TLS key path
    pub tls_key_path: Option<String>,
}

impl Default for TransportManagerConfig {
    fn default() -> Self {
        Self {
            enable_udp: true,
            enable_tcp: true,
            enable_ws: true,
            enable_tls: false,
            bind_addresses: vec![],
            default_channel_capacity: 100,
            tls_cert_path: None,
            tls_key_path: None,
        }
    }
}

/// Manages multiple transport types for SIP messages
#[derive(Clone)]
pub struct TransportManager {
    /// Configuration
    config: TransportManagerConfig,
    /// Collection of active transports by type and address
    transports: Arc<Mutex<HashMap<String, Arc<dyn Transport>>>>,
    /// Default transport
    default_transport: Option<Arc<dyn Transport>>,
    /// Default UDP transport (required for SIP)
    udp_transport: Option<Arc<dyn Transport>>,
    /// Transport factory
    transport_factory: Arc<TransportFactory>,
    /// Combined event channel
    event_tx: mpsc::Sender<TransportEvent>,
    /// Flag indicating whether the manager is running
    running: Arc<Mutex<bool>>,
}

impl TransportManager {
    /// Creates a new TransportManager with the given configuration
    pub async fn new(config: TransportManagerConfig) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let (event_tx, event_rx) = mpsc::channel(config.default_channel_capacity);
        
        let transports = Arc::new(Mutex::new(HashMap::new()));
        let transport_factory = Arc::new(TransportFactory::with_defaults());
        
        let manager = Self {
            config,
            transports,
            default_transport: None,
            udp_transport: None,
            transport_factory,
            event_tx,
            running: Arc::new(Mutex::new(false)),
        };
        
        Ok((manager, event_rx))
    }
    
    /// Creates a new TransportManager with default configuration
    pub async fn with_defaults() -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::new(TransportManagerConfig::default()).await
    }
    
    /// Initializes the transport manager with configured transport types
    pub async fn initialize(&mut self) -> Result<()> {
        let mut initialized = false;
        
        // Initialize UDP transport if enabled
        if self.config.enable_udp {
            let addresses = if self.config.bind_addresses.is_empty() {
                vec!["0.0.0.0:5060".parse().unwrap()]
            } else {
                self.config.bind_addresses.clone()
            };
            
            for addr in addresses {
                match self.add_udp_transport(addr).await {
                    Ok(transport) => {
                        // If this is the first transport, set it as default
                        if self.default_transport.is_none() {
                            self.default_transport = Some(transport.clone());
                        }
                        // Set as UDP transport
                        self.udp_transport = Some(transport);
                        initialized = true;
                    },
                    Err(e) => {
                        error!("Failed to initialize UDP transport on {}: {}", addr, e);
                    }
                }
            }
        }
        
        // Initialize TCP transport if enabled
        if self.config.enable_tcp {
            let addresses = if self.config.bind_addresses.is_empty() {
                vec!["0.0.0.0:5060".parse().unwrap()]
            } else {
                self.config.bind_addresses.clone()
            };
            
            for addr in addresses {
                match self.add_tcp_transport(addr, false).await {
                    Ok(_) => {
                        initialized = true;
                    },
                    Err(e) => {
                        error!("Failed to initialize TCP transport on {}: {}", addr, e);
                    }
                }
            }
            
            // Add TLS transport if enabled
            if self.config.enable_tls {
                if self.config.tls_cert_path.is_none() || self.config.tls_key_path.is_none() {
                    warn!("TLS is enabled but certificate or key path is missing");
                } else {
                    let addresses = if self.config.bind_addresses.is_empty() {
                        vec!["0.0.0.0:5061".parse().unwrap()]
                    } else {
                        self.config.bind_addresses.clone()
                    };
                    
                    for addr in addresses {
                        match self.add_tcp_transport(addr, true).await {
                            Ok(_) => {
                                initialized = true;
                            },
                            Err(e) => {
                                error!("Failed to initialize TLS transport on {}: {}", addr, e);
                            }
                        }
                    }
                }
            }
        }
        
        // Initialize WebSocket transport if enabled
        if self.config.enable_ws {
            let addresses = if self.config.bind_addresses.is_empty() {
                vec!["0.0.0.0:8080".parse().unwrap()]
            } else {
                self.config.bind_addresses.clone()
            };
            
            for addr in addresses {
                match self.add_websocket_transport(addr, false).await {
                    Ok(_) => {
                        initialized = true;
                    },
                    Err(e) => {
                        error!("Failed to initialize WebSocket transport on {}: {}", addr, e);
                    }
                }
            }
            
            // Add WSS transport if enabled
            if self.config.enable_tls {
                if self.config.tls_cert_path.is_none() || self.config.tls_key_path.is_none() {
                    warn!("TLS is enabled but certificate or key path is missing");
                } else {
                    let addresses = if self.config.bind_addresses.is_empty() {
                        vec!["0.0.0.0:8443".parse().unwrap()]
                    } else {
                        self.config.bind_addresses.clone()
                    };
                    
                    for addr in addresses {
                        match self.add_websocket_transport(addr, true).await {
                            Ok(_) => {
                                initialized = true;
                            },
                            Err(e) => {
                                error!("Failed to initialize WSS transport on {}: {}", addr, e);
                            }
                        }
                    }
                }
            }
        }
        
        // Return error if no transports were initialized
        if !initialized {
            return Err(Error::Transport("Failed to initialize any transport".into()));
        }
        
        // Start event processing
        self.start_event_processing();
        
        Ok(())
    }
    
    /// Adds a UDP transport to the manager
    pub async fn add_udp_transport(&self, bind_addr: SocketAddr) -> Result<Arc<dyn Transport>> {
        let (transport, rx) = UdpTransport::bind(bind_addr, None)
            .await
            .map_err(|e| Error::Transport(format!("Failed to bind UDP transport to {}: {}", bind_addr, e)))?;
        
        let transport_arc = Arc::new(transport);
        
        // Store the transport
        let key = format!("udp:{}", bind_addr);
        {
            let mut transports = self.transports.lock().await;
            transports.insert(key, transport_arc.clone());
        }
        
        // Process events from this transport
        self.clone().process_transport_events(transport_arc.clone(), rx);
        
        info!("Added UDP transport bound to {}", bind_addr);
        
        Ok(transport_arc)
    }
    
    /// Adds a TCP transport to the manager
    pub async fn add_tcp_transport(&self, bind_addr: SocketAddr, tls: bool) -> Result<Arc<dyn Transport>> {
        // TLS is not fully implemented yet in this function
        if tls {
            return Err(Error::Transport("TLS transport is not fully implemented in this function".into()));
        }
        
        let (transport, rx) = TcpTransport::bind(bind_addr, None, None)
            .await
            .map_err(|e| Error::Transport(format!("Failed to bind TCP transport to {}: {}", bind_addr, e)))?;
        
        let transport_arc = Arc::new(transport);
        
        // Store the transport
        let key = format!("{}:{}", if tls { "tls" } else { "tcp" }, bind_addr);
        {
            let mut transports = self.transports.lock().await;
            transports.insert(key, transport_arc.clone());
        }
        
        // Process events from this transport
        self.clone().process_transport_events(transport_arc.clone(), rx);
        
        info!("Added {} transport bound to {}", if tls { "TLS" } else { "TCP" }, bind_addr);
        
        Ok(transport_arc)
    }
    
    /// Adds a WebSocket transport to the manager
    pub async fn add_websocket_transport(&self, bind_addr: SocketAddr, secure: bool) -> Result<Arc<dyn Transport>> {
        let cert_path = if secure {
            self.config.tls_cert_path.as_deref()
        } else {
            None
        };
        
        let key_path = if secure {
            self.config.tls_key_path.as_deref()
        } else {
            None
        };
        
        #[cfg(feature = "ws")]
        let result = WebSocketTransport::bind(bind_addr, secure, cert_path, key_path, None).await;
        
        #[cfg(not(feature = "ws"))]
        let result: Result<(WebSocketTransport, mpsc::Receiver<TransportEvent>)> = 
            Err(Error::Transport("WebSocket support is not enabled".into()));
            
        let (transport, rx) = result
            .map_err(|e| Error::Transport(format!("Failed to bind WebSocket transport to {}: {}", bind_addr, e)))?;
        
        let transport_arc = Arc::new(transport);
        
        // Store the transport
        let key = format!("{}:{}", if secure { "wss" } else { "ws" }, bind_addr);
        {
            let mut transports = self.transports.lock().await;
            transports.insert(key, transport_arc.clone());
        }
        
        // Process events from this transport
        self.clone().process_transport_events(transport_arc.clone(), rx);
        
        info!("Added {} transport bound to {}", if secure { "WSS" } else { "WS" }, bind_addr);
        
        Ok(transport_arc)
    }
    
    /// Gets the default transport
    pub async fn default_transport(&self) -> Option<Arc<dyn Transport>> {
        self.default_transport.clone()
    }
    
    /// Gets a transport by key
    pub async fn get_transport(&self, key: &str) -> Option<Arc<dyn Transport>> {
        let transports = self.transports.lock().await;
        transports.get(key).cloned()
    }
    
    /// Gets a transport by type and address
    pub async fn get_transport_by_type_and_addr(&self, transport_type: &str, addr: SocketAddr) -> Option<Arc<dyn Transport>> {
        let key = format!("{}:{}", transport_type, addr);
        self.get_transport(&key).await
    }
    
    /// Gets a transport appropriate for the given destination
    pub async fn get_transport_for_destination(&self, destination: SocketAddr) -> Option<Arc<dyn Transport>> {
        // For now, we just return the UDP transport
        // In the future, we'll add URI-based transport selection
        self.udp_transport.clone()
    }
    
    /// Starts processing transport events
    fn start_event_processing(&self) {
        *self.running.try_lock().unwrap() = true;
    }
    
    /// Processes events from a specific transport
    fn process_transport_events(
        self,
        transport: Arc<dyn Transport>,
        mut rx: mpsc::Receiver<TransportEvent>,
    ) {
        tokio::spawn(async move {
            let transport_name = format!("{:?}", transport);
            
            while let Some(event) = rx.recv().await {
                trace!("Received event from {}: {:?}", transport_name, event);
                
                // Forward the event to the main event channel
                if let Err(e) = self.event_tx.send(event.clone()).await {
                    error!("Failed to forward transport event: {}", e);
                    break;
                }
            }
            
            debug!("Transport event processor for {} stopped", transport_name);
        });
    }
    
    /// Sends a message using the appropriate transport
    pub async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        // Get the appropriate transport
        let transport = self.get_transport_for_destination(destination).await
            .ok_or_else(|| Error::Transport("No transport available for destination".into()))?;
        
        // Send the message
        transport.send_message(message, destination).await
            .map_err(|e| Error::Transport(format!("Failed to send message: {}", e)))?;
        
        Ok(())
    }
    
    /// Closes all transports
    pub async fn close(&self) -> Result<()> {
        let transports = self.transports.lock().await;
        
        for (key, transport) in transports.iter() {
            if let Err(e) = transport.close().await {
                error!("Failed to close transport {}: {}", key, e);
            }
        }
        
        *self.running.lock().await = false;
        
        Ok(())
    }
}

/// Information about available transport types and capabilities
#[derive(Debug, Clone)]
pub struct TransportCapabilities {
    /// Whether UDP transport is supported
    pub supports_udp: bool,
    /// Whether TCP transport is supported
    pub supports_tcp: bool,
    /// Whether TLS transport is supported
    pub supports_tls: bool,
    /// Whether WebSocket transport is supported
    pub supports_ws: bool,
    /// Whether Secure WebSocket transport is supported
    pub supports_wss: bool,
    /// Local address used by the transport
    pub local_addr: Option<std::net::SocketAddr>,
    /// Default transport type
    pub default_transport: TransportType,
}

/// Detailed information about a specific transport type
#[derive(Debug, Clone)]
pub struct TransportInfo {
    /// Transport type
    pub transport_type: TransportType,
    /// Whether the transport is currently connected
    pub is_connected: bool,
    /// Local address for this transport
    pub local_addr: Option<std::net::SocketAddr>,
    /// Number of active connections (for connection-oriented transports)
    pub connection_count: usize,
}

/// Network information for SDP generation
#[derive(Debug, Clone)]
pub struct NetworkInfoForSdp {
    /// Local IP address to use in SDP
    pub local_ip: std::net::IpAddr,
    /// Port range for RTP (min, max)
    pub rtp_port_range: (u16, u16),
}

/// WebSocket connection status
#[derive(Debug, Clone)]
pub struct WebSocketStatus {
    /// Number of active insecure WebSocket connections
    pub ws_connections: usize,
    /// Number of active secure WebSocket connections
    pub wss_connections: usize,
    /// Whether there is at least one active WebSocket connection
    pub has_active_connection: bool,
}

/// Extension trait for the transport to provide additional capabilities
pub trait TransportCapabilitiesExt {
    /// Check if UDP transport is supported
    fn supports_udp(&self) -> bool;
    
    /// Check if TCP transport is supported
    fn supports_tcp(&self) -> bool;
    
    /// Check if TLS transport is supported
    fn supports_tls(&self) -> bool;
    
    /// Check if WebSocket transport is supported
    fn supports_ws(&self) -> bool;
    
    /// Check if Secure WebSocket transport is supported
    fn supports_wss(&self) -> bool;
    
    /// Check if a specific transport type is supported
    fn supports_transport(&self, transport_type: TransportType) -> bool;
    
    /// Get the default transport type
    fn default_transport_type(&self) -> TransportType;
    
    /// Check if a specific transport is currently connected
    fn is_transport_connected(&self, transport_type: TransportType) -> bool;
    
    /// Get the local address for a specific transport type
    fn get_transport_local_addr(&self, transport_type: TransportType) -> crate::error::Result<std::net::SocketAddr>;
    
    /// Get the number of active connections for a transport type
    fn get_connection_count(&self, transport_type: TransportType) -> usize;
}

impl<T: rvoip_sip_transport::Transport + ?Sized> TransportCapabilitiesExt for T {
    fn supports_udp(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_udp(self)
    }
    
    fn supports_tcp(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_tcp(self)
    }
    
    fn supports_tls(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_tls(self)
    }
    
    fn supports_ws(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_ws(self)
    }
    
    fn supports_wss(&self) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_wss(self)
    }
    
    fn supports_transport(&self, transport_type: TransportType) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::supports_transport(self, transport_type)
    }
    
    fn default_transport_type(&self) -> TransportType {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::default_transport_type(self)
    }
    
    fn is_transport_connected(&self, transport_type: TransportType) -> bool {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::is_transport_connected(self, transport_type)
    }
    
    fn get_transport_local_addr(&self, _transport_type: TransportType) -> crate::error::Result<std::net::SocketAddr> {
        // Default implementation just returns the main local address
        self.local_addr().map_err(|e| crate::Error::transport_error(e, "Failed to get local address"))
    }
    
    fn get_connection_count(&self, transport_type: TransportType) -> usize {
        // Use the method directly from sip-transport Transport trait
        rvoip_sip_transport::Transport::get_connection_count(self, transport_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    
    #[tokio::test]
    async fn test_transport_manager_creation() {
        let config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec![
                "127.0.0.1:0".parse().unwrap(),
            ],
            ..Default::default()
        };
        
        let (mut manager, _rx) = TransportManager::new(config).await.unwrap();
        
        // Initialize the manager
        let result = manager.initialize().await;
        assert!(result.is_ok(), "Failed to initialize transport manager: {:?}", result);
        
        // Verify UDP transport was created
        let udp_transport = manager.udp_transport.clone();
        assert!(udp_transport.is_some(), "UDP transport should exist");
        
        // Check if default transport is set
        let default_transport = manager.default_transport.clone();
        assert!(default_transport.is_some(), "Default transport should exist");
        
        // Clean up
        let result = manager.close().await;
        assert!(result.is_ok(), "Failed to close transport manager: {:?}", result);
    }
    
    #[tokio::test]
    async fn test_transport_manager_with_defaults() {
        let (mut manager, _rx) = TransportManager::with_defaults().await.unwrap();
        
        // Initialize the manager - will fail without UDP binding config
        let result = manager.initialize().await;
        assert!(result.is_ok(), "Failed to initialize transport manager: {:?}", result);
        
        // Clean up
        let result = manager.close().await;
        assert!(result.is_ok(), "Failed to close transport manager: {:?}", result);
    }
    
    #[tokio::test]
    async fn test_send_message() {
        let config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec![
                "127.0.0.1:0".parse().unwrap(),
            ],
            ..Default::default()
        };
        
        let (mut manager, mut rx) = TransportManager::new(config).await.unwrap();
        
        // Initialize the manager
        let result = manager.initialize().await;
        assert!(result.is_ok(), "Failed to initialize transport manager: {:?}", result);
        
        // Create a test message
        let message = Message::Request(
            rvoip_sip_core::builder::SimpleRequestBuilder::new(
                rvoip_sip_core::Method::Register,
                "sip:example.com",
            )
            .unwrap()
            .from("user", "sip:user@example.com", None)
            .to("user", "sip:user@example.com", None)
            .call_id("test-call-id")
            .cseq(1)
            .build(),
        );
        
        // Get local address to send message to
        let local_addr = manager.udp_transport.as_ref().unwrap().local_addr().unwrap();
        
        // Send message
        let result = manager.send_message(message.clone(), local_addr).await;
        assert!(result.is_ok(), "Failed to send message: {:?}", result);
        
        // Should receive a transport event
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("Timed out waiting for transport event")
            .expect("No transport event received");
        
        // Clean up
        let result = manager.close().await;
        assert!(result.is_ok(), "Failed to close transport manager: {:?}", result);
    }
    
    #[tokio::test]
    async fn test_transport_send_message() {
        let config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec![
                "127.0.0.1:0".parse().unwrap(),
            ],
            ..Default::default()
        };
        
        let (mut manager, mut rx) = TransportManager::new(config).await.unwrap();
        
        // Initialize the manager
        let result = manager.initialize().await;
        assert!(result.is_ok(), "Failed to initialize transport manager: {:?}", result);
        
        // Create a test message
        let message = Message::Request(
            rvoip_sip_core::builder::SimpleRequestBuilder::new(
                rvoip_sip_core::Method::Register,
                "sip:example.com",
            )
            .unwrap()
            .from("user", "sip:user@example.com", None)
            .to("user", "sip:user@example.com", None)
            .call_id("test-call-id")
            .cseq(1)
            .build(),
        );
        
        // Get local address to send message to
        let local_addr = manager.udp_transport.as_ref().unwrap().local_addr().unwrap();
        
        // Send message
        let result = manager.send_message(message.clone(), local_addr).await;
        assert!(result.is_ok(), "Failed to send message: {:?}", result);
        
        // Should receive a transport event
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("Timed out waiting for transport event")
            .expect("No transport event received");
        
        // Clean up
        let result = manager.close().await;
        assert!(result.is_ok(), "Failed to close transport manager: {:?}", result);
    }
    
    #[tokio::test]
    async fn test_integration_with_transaction_manager() {
        use crate::manager::TransactionManager;
        
        // Create a transport manager
        let config = TransportManagerConfig {
            enable_udp: true,
            enable_tcp: false,
            enable_ws: false,
            enable_tls: false,
            bind_addresses: vec![
                "127.0.0.1:0".parse().unwrap(),
            ],
            ..Default::default()
        };
        
        let (mut transport_manager, transport_rx) = TransportManager::new(config).await.unwrap();
        
        // Initialize the transport manager
        let result = transport_manager.initialize().await;
        assert!(result.is_ok(), "Failed to initialize transport manager: {:?}", result);
        
        // Create transaction manager with the transport manager
        let (transaction_manager, mut tx_events_rx) = TransactionManager::with_transport_manager(
            transport_manager.clone(),
            transport_rx,
            Some(100),
        ).await.unwrap();
        
        // Create a test message and send it
        let request = rvoip_sip_core::builder::SimpleRequestBuilder::new(
            rvoip_sip_core::Method::Register,
            "sip:example.com",
        )
        .unwrap()
        .from("user", "sip:user@example.com", None)
        .to("user", "sip:user@example.com", None)
        .call_id("test-call-id")
        .cseq(1)
        .build();
        
        // Get local address
        let local_addr = transport_manager.udp_transport.as_ref().unwrap().local_addr().unwrap();
        
        // Create a client transaction
        let tx_id = transaction_manager.create_client_transaction(request.clone(), local_addr)
            .await
            .expect("Failed to create client transaction");
        
        // Send the request
        let result = transaction_manager.send_request(&tx_id).await;
        assert!(result.is_ok(), "Failed to send request through transaction manager: {:?}", result);
        
        // Verify transaction state change event
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), tx_events_rx.recv())
            .await
            .expect("Timed out waiting for transaction event")
            .expect("No transaction event received");
        
        match event {
            crate::transaction::TransactionEvent::StateChanged { transaction_id, .. } => {
                assert_eq!(transaction_id, tx_id, "Transaction IDs should match");
            },
            _ => panic!("Expected StateChanged event, got {:?}", event),
        }
        
        // Clean up - shutdown transaction manager
        transaction_manager.shutdown().await;
    }
} 