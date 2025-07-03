use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tracing::{debug, error, trace};

use crate::error::{Error, Result};

/// UDP sender for sending SIP messages
#[derive(Clone)]
pub struct UdpSender {
    socket: Arc<UdpSocket>,
}

impl UdpSender {
    /// Creates a new UDP sender with the provided socket
    pub fn new(socket: Arc<UdpSocket>) -> Result<Self> {
        Ok(Self { socket })
    }
    
    /// Binds a new UDP socket and creates a sender
    pub async fn bind(addr: SocketAddr) -> Result<Self> {
        let socket = UdpSocket::bind(addr).await.map_err(|e| Error::BindFailed(addr, e))?;
        Ok(Self { socket: Arc::new(socket) })
    }
    
    /// Returns the local address of the socket
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.socket.local_addr().map_err(Error::from)
    }
    
    /// Sends data to the specified destination
    pub async fn send(&self, data: &[u8], destination: SocketAddr) -> Result<()> {
        trace!("Sending {} bytes to {}", data.len(), destination);
        
        match self.socket.send_to(data, destination).await {
            Ok(bytes_sent) => {
                if bytes_sent < data.len() {
                    error!("Sent only {} of {} bytes to {}", bytes_sent, data.len(), destination);
                    return Err(Error::PartialSend(bytes_sent, data.len()));
                }
                debug!("Sent {} bytes to {}", bytes_sent, destination);
                Ok(())
            },
            Err(e) => {
                error!("Failed to send to {}: {}", destination, e);
                Err(Error::SendFailed(destination, e))
            }
        }
    }
    
    /// Creates a default dummy sender (used for testing)
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
    use tokio::sync::mpsc;
    use tokio::io::AsyncReadExt;
    
    #[tokio::test]
    async fn test_udp_sender_send() {
        // Set up a receiver socket
        let receiver_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let receiver_addr = receiver_socket.local_addr().unwrap();
        
        // Create a sender
        let sender = UdpSender::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await.unwrap();
        
        // Send test data
        let test_data = b"TEST SIP MESSAGE";
        sender.send(test_data, receiver_addr).await.unwrap();
        
        // Receive the data
        let mut buffer = vec![0u8; 1024];
        let (len, _) = receiver_socket.recv_from(&mut buffer).await.unwrap();
        buffer.truncate(len);
        
        assert_eq!(&buffer, test_data);
    }
    
    #[tokio::test]
    async fn test_shared_socket() {
        // Create a shared socket
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let socket_addr = socket.local_addr().unwrap();
        let shared_socket = Arc::new(socket);
        
        // Create a sender with the shared socket
        let sender = UdpSender::new(shared_socket.clone()).unwrap();
        
        // Set up a receiver socket
        let receiver_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let receiver_addr = receiver_socket.local_addr().unwrap();
        
        // Send test data using the sender
        let test_data = b"SHARED SOCKET TEST";
        sender.send(test_data, receiver_addr).await.unwrap();
        
        // Receive the data
        let mut buffer = vec![0u8; 1024];
        let (len, src) = receiver_socket.recv_from(&mut buffer).await.unwrap();
        buffer.truncate(len);
        
        assert_eq!(&buffer, test_data);
        assert_eq!(src.ip(), socket_addr.ip());
    }
} 