mod listener;
mod sender;

pub use listener::UdpListener;
pub use sender::UdpSender;

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

use rvoip_sip_core::Message;
use crate::error::{Error, Result};
use crate::transport::{Transport, TransportEvent};

// Default channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 100;

/// UDP transport for SIP messages
#[derive(Clone)]
pub struct UdpTransport {
    inner: Arc<UdpTransportInner>,
}

struct UdpTransportInner {
    sender: UdpSender,
    listener: Arc<UdpListener>,
    closed: AtomicBool,
    events_tx: mpsc::Sender<TransportEvent>,
}

impl UdpTransport {
    /// Creates a new UDP transport bound to the specified address
    pub async fn bind(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        // Create the event channel
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);
        
        // Create the UDP listener
        let listener = UdpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        info!("SIP UDP transport bound to {}", local_addr);
        
        // Create the UDP sender (shares same socket)
        let sender = UdpSender::new(listener.clone_socket())?;
        
        // Create the transport
        let transport = UdpTransport {
            inner: Arc::new(UdpTransportInner {
                sender,
                listener: Arc::new(listener),
                closed: AtomicBool::new(false),
                events_tx: events_tx.clone(),
            }),
        };

        // Start the receive loop
        transport.spawn_receive_loop();

        Ok((transport, events_rx))
    }

    /// Create a default dummy UDP transport (used only for creating dummy transaction managers)
    /// This transport doesn't work for real communication
    #[cfg(test)]
    pub fn default() -> Self {
        // Create a dummy event channel
        let (events_tx, _) = mpsc::channel(1);
        
        // Create a dummy listener and sender
        let listener = UdpListener::default();
        let sender = UdpSender::default();
        
        // Create and return the transport with closed=true so it won't be used
        UdpTransport {
            inner: Arc::new(UdpTransportInner {
                sender,
                listener: Arc::new(listener),
                closed: AtomicBool::new(true), // Mark as closed
                events_tx,
            }),
        }
    }

    // Spawns a task to receive packets from the UDP socket
    fn spawn_receive_loop(&self) {
        let transport = self.clone();
        
        tokio::spawn(async move {
            let inner = &transport.inner;
            let listener_clone = inner.listener.clone();
            
            while !inner.closed.load(Ordering::Relaxed) {
                // Receive a packet from the listener
                let result = listener_clone.receive().await;
                
                match result {
                    Ok((packet, src, local_addr)) => {
                        debug!("Received SIP message from {}", src);
                        
                        match rvoip_sip_core::parse_message(&packet) {
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
                    },
                    Err(e) => {
                        if inner.closed.load(Ordering::Relaxed) {
                            break;
                        }
                        
                        error!("Error receiving UDP packet: {}", e);
                        let _ = inner.events_tx.send(TransportEvent::Error {
                            error: format!("Error receiving packet: {}", e),
                        }).await;
                    }
                }
            }
            
            // Send closed event when the loop exits
            let _ = inner.events_tx.send(TransportEvent::Closed).await;
            info!("UDP receive loop terminated");
        });
    }
}

#[async_trait::async_trait]
impl Transport for UdpTransport {
    fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.listener.local_addr()
    }
    
    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        
        // Convert message to bytes
        let bytes = message.to_bytes();
        
        debug!("Sending {} byte message to {}", bytes.len(), destination);
        info!("Sending {} message to {}", 
            if let Message::Request(ref req) = message { 
                format!("{}", req.method) 
            } else { 
                "response".to_string() 
            }, 
            destination);
        
        // Send the message using the sender
        self.inner.sender.send(&bytes, destination).await
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
        if let Ok(addr) = self.inner.listener.local_addr() {
            write!(f, "UdpTransport({})", addr)
        } else {
            write!(f, "UdpTransport(<e>)")
        }
    }
} 