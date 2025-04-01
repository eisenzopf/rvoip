use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::BytesMut;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

use rvoip_sip_core::{Message, parse_message};

use crate::error::{Error, Result};
use crate::transport::{Transport, TransportEvent};

// Maximum UDP packet size
const MAX_UDP_PACKET_SIZE: usize = 65_507;
// Buffer size for receiving packets
const UDP_BUFFER_SIZE: usize = 8192;
// Default channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 100;

/// UDP transport for SIP messages
#[derive(Clone)]
pub struct UdpTransport {
    inner: Arc<UdpTransportInner>,
}

struct UdpTransportInner {
    socket: UdpSocket,
    closed: AtomicBool,
    events_tx: mpsc::Sender<TransportEvent>,
}

impl UdpTransport {
    /// Creates a new UDP transport bound to the specified address
    pub async fn bind(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        // Create the UDP socket
        let socket = UdpSocket::bind(addr).await.map_err(|e| Error::BindFailed(addr, e))?;
        
        // Get the actual bound address
        let local_addr = socket.local_addr()?;
        info!("SIP UDP transport bound to {}", local_addr);

        // Create the event channel
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);

        // Create the transport
        let transport = UdpTransport {
            inner: Arc::new(UdpTransportInner {
                socket,
                closed: AtomicBool::new(false),
                events_tx,
            }),
        };

        // Start the receive loop
        transport.spawn_receive_loop();

        Ok((transport, events_rx))
    }

    // Spawns a task to receive packets from the UDP socket
    fn spawn_receive_loop(&self) {
        let transport = self.clone();
        tokio::spawn(async move {
            let inner = &transport.inner;
            let mut buffer = vec![0u8; UDP_BUFFER_SIZE];
            
            while !inner.closed.load(Ordering::Relaxed) {
                // Receive a packet
                let (len, src) = match inner.socket.recv_from(&mut buffer).await {
                    Ok((len, src)) => (len, src),
                    Err(e) => {
                        // Ignore would-block errors or errors after closing
                        if inner.closed.load(Ordering::Relaxed) {
                            break;
                        }
                        
                        error!("Error receiving UDP packet: {}", e);
                        let _ = inner.events_tx.send(TransportEvent::Error {
                            error: format!("Error receiving packet: {}", e),
                        }).await;
                        continue;
                    }
                };
                
                let local_addr = match inner.socket.local_addr() {
                    Ok(addr) => addr,
                    Err(e) => {
                        error!("Error getting local address: {}", e);
                        continue;
                    }
                };
                
                // Create a Bytes object from the received data
                let packet_data = bytes::Bytes::copy_from_slice(&buffer[..len]);
                trace!("Received packet from {}: {:?}", src, packet_data);
                
                // Parse the SIP message
                let packet_str = String::from_utf8_lossy(&packet_data);
                debug!("Received SIP message from {}: {}", src, packet_str);
                
                match parse_message(&packet_data) {
                    Ok(message) => {
                        let event = TransportEvent::MessageReceived {
                            message,
                            source: src,
                            destination: local_addr,
                        };
                        
                        if let Err(e) = inner.events_tx.send(event).await {
                            error!("Error sending event: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        warn!("Error parsing SIP message: {}", e);
                        let _ = inner.events_tx.send(TransportEvent::Error {
                            error: format!("Error parsing SIP message: {}", e),
                        }).await;
                    }
                }
            }
            
            // Send closed event when the loop exits
            let _ = inner.events_tx.send(TransportEvent::Closed).await;
        });
    }
}

#[async_trait::async_trait]
impl Transport for UdpTransport {
    fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.socket.local_addr().map_err(Error::from)
    }
    
    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        
        // Convert the message to a string
        let message_str = message.to_string();
        let bytes = message_str.as_bytes();
        
        // Check if the message is too large
        if bytes.len() > MAX_UDP_PACKET_SIZE {
            return Err(Error::PacketTooLarge(bytes.len(), MAX_UDP_PACKET_SIZE));
        }
        
        // Send the message
        self.inner.socket.send_to(bytes, destination).await
            .map_err(|e| Error::SendFailed(destination, e))?;
            
        trace!("Sent SIP message to {}: {}", destination, message_str);
        
        Ok(())
    }
    
    async fn close(&self) -> Result<()> {
        self.inner.closed.store(true, Ordering::Relaxed);
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Relaxed)
    }
}

impl fmt::Debug for UdpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Ok(addr) = self.inner.socket.local_addr() {
            write!(f, "UdpTransport({})", addr)
        } else {
            write!(f, "UdpTransport(<error>)")
        }
    }
} 