use std::net::SocketAddr;
use std::sync::Arc;
use bytes::BytesMut;
use tokio::net::UdpSocket;
use tracing::{debug, error, trace};

use crate::error::{Error, Result};

// Maximum UDP packet size
const MAX_UDP_PACKET_SIZE: usize = 65_507;
// Buffer size for receiving packets
const UDP_BUFFER_SIZE: usize = 8192;

/// UDP listener for receiving SIP messages
pub struct UdpListener {
    socket: Arc<UdpSocket>,
}

impl UdpListener {
    /// Binds the UDP listener to the specified address
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        // Create the UDP socket
        let socket = UdpSocket::bind(addr).await.map_err(|e| Error::BindFailed(addr, e))?;
        
        Ok(Self {
            socket: Arc::new(socket),
        })
    }
    
    /// Returns a reference to the underlying socket
    pub fn socket(&self) -> &UdpSocket {
        &self.socket
    }
    
    /// Returns a cloned Arc to the underlying socket
    pub fn clone_socket(&self) -> Arc<UdpSocket> {
        self.socket.clone()
    }
    
    /// Returns the local address this listener is bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket.local_addr().map_err(Error::from)
    }
    
    /// Receives a packet from the UDP socket
    pub async fn receive(&self) -> Result<(bytes::Bytes, SocketAddr, SocketAddr)> {
        let mut buffer = BytesMut::with_capacity(UDP_BUFFER_SIZE);
        buffer.resize(UDP_BUFFER_SIZE, 0);
        
        // Receive a packet
        let (len, src) = self.socket.recv_from(&mut buffer).await
            .map_err(|e| Error::ReceiveFailed(e))?;
        
        // Get the local address
        let local_addr = self.socket.local_addr()
            .map_err(|e| Error::LocalAddrFailed(e))?;
        
        // Truncate buffer to the actual received length
        buffer.truncate(len);
        
        // Convert to Bytes
        let packet = buffer.freeze();
        trace!("Received {} bytes from {}", len, src);
        
        Ok((packet, src, local_addr))
    }
    
    /// Creates a default dummy listener (used for testing)
    pub fn default() -> Self {
        let socket = match std::net::UdpSocket::bind("127.0.0.1:0") {
            Ok(std_socket) => {
                if let Err(e) = std_socket.set_nonblocking(true) {
                    error!("Failed to set socket to non-blocking mode: {}", e);
                }
                
                match UdpSocket::from_std(std_socket) {
                    Ok(socket) => socket,
                    Err(e) => {
                        error!("Failed to create tokio socket: {}", e);
                        // Create a dummy socket (this will likely fail in real use)
                        let std_socket = std::net::UdpSocket::bind("127.0.0.1:0")
                            .expect("Failed to create dummy socket");
                        UdpSocket::from_std(std_socket)
                            .expect("Failed to create tokio socket")
                    }
                }
            },
            Err(e) => {
                error!("Failed to bind socket: {}", e);
                // Create a dummy socket (this will likely fail in real use)
                let std_socket = std::net::UdpSocket::bind("127.0.0.1:0")
                    .expect("Failed to create dummy socket");
                UdpSocket::from_std(std_socket)
                    .expect("Failed to create tokio socket")
            }
        };
        
        Self {
            socket: Arc::new(socket),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    
    #[tokio::test]
    async fn test_udp_listener_bind() {
        let listener = UdpListener::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        assert!(addr.port() > 0);
    }
    
    #[tokio::test]
    async fn test_udp_listener_receive() {
        // Bind listener to a random port
        let listener = UdpListener::bind("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let addr = listener.local_addr().unwrap();
        
        // Create a separate socket to send data to the listener
        let sender_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        
        // Send a test message
        let test_data = b"TEST SIP MESSAGE";
        sender_socket.send_to(test_data, addr).await.unwrap();
        
        // Receive the message
        let (packet, src, dest) = listener.receive().await.unwrap();
        
        assert_eq!(&packet[..], test_data);
        assert_eq!(dest, addr);
        assert_eq!(src.ip(), sender_socket.local_addr().unwrap().ip());
    }
} 