use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info};

use crate::transport::{Transport, TransportEvent};
use crate::transport::udp::UdpTransport;
use crate::transport::tcp::TcpTransport;
use crate::transport::tls::TlsTransport;
use crate::transport::ws::WebSocketTransport;
use crate::transport::tcp::pool::PoolConfig;
use crate::error::{Error, Result};

/// Transport type enum for creating specific transports
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    /// UDP transport (connectionless)
    Udp,
    /// TCP transport (connection-oriented)
    Tcp,
    /// TLS transport (secure connection-oriented)
    Tls,
    /// WebSocket transport (connection-oriented)
    WebSocket,
    /// Secure WebSocket transport (secure connection-oriented)
    SecureWebSocket,
}

impl TransportType {
    /// Creates a TransportType from a URI scheme
    pub fn from_scheme(scheme: &str) -> Result<Self> {
        match scheme.to_lowercase().as_str() {
            "sip" => Ok(TransportType::Udp), // Default is UDP
            "sips" => Ok(TransportType::Tls), // Default secure is TLS
            "tcp" => Ok(TransportType::Tcp),
            "udp" => Ok(TransportType::Udp),
            "tls" => Ok(TransportType::Tls),
            "ws" => Ok(TransportType::WebSocket),
            "wss" => Ok(TransportType::SecureWebSocket),
            _ => Err(Error::UnsupportedTransport(scheme.to_string())),
        }
    }
    
    /// Returns the string representation of the transport type
    pub fn as_str(&self) -> &'static str {
        match self {
            TransportType::Udp => "udp",
            TransportType::Tcp => "tcp",
            TransportType::Tls => "tls",
            TransportType::WebSocket => "ws",
            TransportType::SecureWebSocket => "wss",
        }
    }
    
    /// Returns if this is a secure transport
    pub fn is_secure(&self) -> bool {
        matches!(self, TransportType::Tls | TransportType::SecureWebSocket)
    }
    
    /// Returns if this is a connection-oriented transport
    pub fn is_connection_oriented(&self) -> bool {
        !matches!(self, TransportType::Udp)
    }
    
    /// Returns the default port for this transport type
    pub fn default_port(&self) -> u16 {
        match self {
            TransportType::Udp => 5060,
            TransportType::Tcp => 5060,
            TransportType::Tls => 5061,
            TransportType::WebSocket => 80,
            TransportType::SecureWebSocket => 443,
        }
    }
}

/// Configuration for the transport factory
#[derive(Clone, Debug)]
pub struct TransportFactoryConfig {
    /// Channel capacity for transport event channels
    pub channel_capacity: usize,
    /// Configuration for TCP/TLS connection pools
    pub pool_config: PoolConfig,
    /// Path to TLS certificate file
    pub tls_cert_path: Option<String>,
    /// Path to TLS key file
    pub tls_key_path: Option<String>,
    /// Path to TLS CA file for client authentication
    pub tls_ca_path: Option<String>,
}

impl Default for TransportFactoryConfig {
    fn default() -> Self {
        Self {
            channel_capacity: 100,
            pool_config: PoolConfig::default(),
            tls_cert_path: None,
            tls_key_path: None,
            tls_ca_path: None,
        }
    }
}

/// Factory for creating SIP transports
pub struct TransportFactory {
    /// Configuration for the factory
    config: TransportFactoryConfig,
}

impl TransportFactory {
    /// Creates a new transport factory with the given configuration
    pub fn new(config: TransportFactoryConfig) -> Self {
        Self { config }
    }
    
    /// Creates a transport factory with default configuration
    pub fn with_defaults() -> Self {
        Self::new(TransportFactoryConfig::default())
    }
    
    /// Creates a transport of the specified type bound to the given address
    pub async fn create_transport(
        &self,
        transport_type: TransportType,
        bind_addr: SocketAddr,
    ) -> Result<(Arc<dyn Transport>, mpsc::Receiver<TransportEvent>)> {
        match transport_type {
            TransportType::Udp => {
                let (transport, rx) = UdpTransport::bind(bind_addr, Some(self.config.channel_capacity)).await?;
                Ok((Arc::new(transport), rx))
            },
            TransportType::Tcp => {
                let (transport, rx) = TcpTransport::bind(
                    bind_addr,
                    Some(self.config.channel_capacity),
                    Some(self.config.pool_config.clone()),
                ).await?;
                Ok((Arc::new(transport), rx))
            },
            TransportType::Tls => {
                // Check if TLS certificates are provided
                let cert_path = self.config.tls_cert_path.as_ref()
                    .ok_or_else(|| Error::InvalidState("TLS certificate path not provided".to_string()))?;
                let key_path = self.config.tls_key_path.as_ref()
                    .ok_or_else(|| Error::InvalidState("TLS key path not provided".to_string()))?;
                
                let (transport, rx) = TlsTransport::bind(
                    bind_addr,
                    cert_path,
                    key_path,
                    self.config.tls_ca_path.as_deref(),
                    Some(self.config.channel_capacity),
                    Some(self.config.pool_config.clone()),
                ).await?;
                Ok((Arc::new(transport), rx))
            },
            TransportType::WebSocket => {
                let (transport, rx) = WebSocketTransport::bind(
                    bind_addr,
                    false, // not secure
                    None,  // no cert
                    None,  // no key
                    Some(self.config.channel_capacity),
                ).await?;
                Ok((Arc::new(transport), rx))
            },
            TransportType::SecureWebSocket => {
                // Check if TLS certificates are provided
                let cert_path = self.config.tls_cert_path.as_ref()
                    .ok_or_else(|| Error::InvalidState("TLS certificate path not provided".to_string()))?;
                let key_path = self.config.tls_key_path.as_ref()
                    .ok_or_else(|| Error::InvalidState("TLS key path not provided".to_string()))?;
                
                let (transport, rx) = WebSocketTransport::bind(
                    bind_addr,
                    true, // secure
                    Some(cert_path),
                    Some(key_path),
                    Some(self.config.channel_capacity),
                ).await?;
                Ok((Arc::new(transport), rx))
            },
        }
    }
    
    /// Creates a transport based on a SIP URI scheme
    pub async fn create_transport_for_scheme(
        &self,
        scheme: &str,
        bind_addr: SocketAddr,
    ) -> Result<(Arc<dyn Transport>, mpsc::Receiver<TransportEvent>)> {
        let transport_type = TransportType::from_scheme(scheme)?;
        self.create_transport(transport_type, bind_addr).await
    }
    
    /// Returns the configuration of this factory
    pub fn config(&self) -> &TransportFactoryConfig {
        &self.config
    }
}

// Unit tests for the transport factory
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_transport_type_from_scheme() {
        assert_eq!(TransportType::from_scheme("sip").unwrap(), TransportType::Udp);
        assert_eq!(TransportType::from_scheme("sips").unwrap(), TransportType::Tls);
        assert_eq!(TransportType::from_scheme("tcp").unwrap(), TransportType::Tcp);
        assert_eq!(TransportType::from_scheme("udp").unwrap(), TransportType::Udp);
        assert_eq!(TransportType::from_scheme("tls").unwrap(), TransportType::Tls);
        assert_eq!(TransportType::from_scheme("ws").unwrap(), TransportType::WebSocket);
        assert_eq!(TransportType::from_scheme("wss").unwrap(), TransportType::SecureWebSocket);
        
        // Case insensitivity
        assert_eq!(TransportType::from_scheme("SIP").unwrap(), TransportType::Udp);
        assert_eq!(TransportType::from_scheme("SIPS").unwrap(), TransportType::Tls);
        
        // Unsupported schemes
        assert!(TransportType::from_scheme("http").is_err());
        assert!(TransportType::from_scheme("ftp").is_err());
    }
    
    #[test]
    fn test_transport_type_properties() {
        // Test is_secure
        assert!(!TransportType::Udp.is_secure());
        assert!(!TransportType::Tcp.is_secure());
        assert!(TransportType::Tls.is_secure());
        assert!(!TransportType::WebSocket.is_secure());
        assert!(TransportType::SecureWebSocket.is_secure());
        
        // Test is_connection_oriented
        assert!(!TransportType::Udp.is_connection_oriented());
        assert!(TransportType::Tcp.is_connection_oriented());
        assert!(TransportType::Tls.is_connection_oriented());
        assert!(TransportType::WebSocket.is_connection_oriented());
        assert!(TransportType::SecureWebSocket.is_connection_oriented());
        
        // Test default_port
        assert_eq!(TransportType::Udp.default_port(), 5060);
        assert_eq!(TransportType::Tcp.default_port(), 5060);
        assert_eq!(TransportType::Tls.default_port(), 5061);
        assert_eq!(TransportType::WebSocket.default_port(), 80);
        assert_eq!(TransportType::SecureWebSocket.default_port(), 443);
    }
    
    #[tokio::test]
    async fn test_factory_create_udp_transport() {
        let factory = TransportFactory::with_defaults();
        
        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (transport, _rx) = factory.create_transport(TransportType::Udp, bind_addr).await.unwrap();
        
        let bound_addr = transport.local_addr().unwrap();
        assert_ne!(bound_addr.port(), 0);
        
        transport.close().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_factory_create_tcp_transport() {
        let factory = TransportFactory::with_defaults();
        
        let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let (transport, _rx) = factory.create_transport(TransportType::Tcp, bind_addr).await.unwrap();
        
        let bound_addr = transport.local_addr().unwrap();
        assert_ne!(bound_addr.port(), 0);
        
        transport.close().await.unwrap();
    }
} 