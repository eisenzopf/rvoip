use std::net::SocketAddr;
use tokio::net::{TcpListener as TokioTcpListener, TcpStream};
use tracing::{debug, error, info, trace};

use crate::error::{Error, Result};

/// TCP listener for accepting SIP connections
pub struct TcpListener {
    /// The Tokio TCP listener
    listener: TokioTcpListener,
}

impl TcpListener {
    /// Binds the TCP listener to the specified address
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let listener = TokioTcpListener::bind(addr)
            .await
            .map_err(|e| Error::BindFailed(addr, e))?;
        
        info!("TCP listener bound to {}", listener.local_addr().unwrap());
        
        Ok(Self { listener })
    }
    
    /// Returns the local address this listener is bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener.local_addr().map_err(|e| Error::LocalAddrFailed(e))
    }
    
    /// Accepts a new TCP connection
    pub async fn accept(&self) -> Result<(TcpStream, SocketAddr)> {
        let (stream, peer_addr) = self.listener.accept()
            .await
            .map_err(|e| Error::ReceiveFailed(e))?;
        
        debug!("Accepted TCP connection from {}", peer_addr);
        
        // Configure the socket
        if let Err(e) = stream.set_nodelay(true) {
            error!("Failed to set TCP_NODELAY: {}", e);
        }
        
        Ok((stream, peer_addr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    
    #[tokio::test]
    async fn test_tcp_listener_bind() {
        let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.port() > 0);
    }
    
    #[tokio::test]
    async fn test_tcp_listener_accept() {
        // Bind listener to a random port
        let listener = TcpListener::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        
        // Set up a task to connect to the listener
        let connect_task = tokio::spawn(async move {
            // Give the listener a moment to start
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            
            // Connect to the listener
            let mut stream = TcpStream::connect(addr).await.unwrap();
            
            // Send some data
            stream.write_all(b"Hello").await.unwrap();
            
            // Wait a bit to make sure data is sent
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        });
        
        // Accept a connection
        let (mut stream, peer_addr) = listener.accept().await.unwrap();
        assert_eq!(peer_addr.ip(), "127.0.0.1".parse::<std::net::IpAddr>().unwrap());
        
        // Read the data
        let mut buffer = vec![0u8; 5];
        let bytes_read = stream.read(&mut buffer).await.unwrap();
        assert_eq!(bytes_read, 5);
        assert_eq!(&buffer, b"Hello");
        
        // Wait for the connect task to complete
        connect_task.await.unwrap();
    }
} 