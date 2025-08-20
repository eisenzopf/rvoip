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
    receive_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
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
        
        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        
        // Create the transport
        let transport = UdpTransport {
            inner: Arc::new(UdpTransportInner {
                sender,
                listener: Arc::new(listener),
                closed: AtomicBool::new(false),
                events_tx: events_tx.clone(),
                receive_task: tokio::sync::Mutex::new(None),
                shutdown_tx,
                shutdown_rx,
            }),
        };

        // Start the receive loop
        transport.spawn_receive_loop().await;

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
        
        // Create shutdown channel
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
        
        // Create and return the transport with closed=true so it won't be used
        UdpTransport {
            inner: Arc::new(UdpTransportInner {
                sender,
                listener: Arc::new(listener),
                closed: AtomicBool::new(true), // Mark as closed
                events_tx,
                receive_task: tokio::sync::Mutex::new(None),
                shutdown_tx,
                shutdown_rx,
            }),
        }
    }

    // Spawns a task to receive packets from the UDP socket
    async fn spawn_receive_loop(&self) {
        let transport = self.clone();
        let mut shutdown_rx = self.inner.shutdown_rx.clone();
        
        let handle = tokio::spawn(async move {
            let inner = &transport.inner;
            let listener_clone = inner.listener.clone();
            
            loop {
                // Use select to listen for both packets and shutdown signal
                tokio::select! {
                    // Watch for shutdown signal
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            debug!("UDP receive loop received shutdown signal");
                            break;
                        }
                    }
                    
                    // Receive packets
                    result = listener_clone.receive() => {
                
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
                                error!("Error receiving UDP packet: {}", e);
                                let _ = inner.events_tx.send(TransportEvent::Error {
                                    error: format!("Error receiving packet: {}", e),
                                }).await;
                            }
                        }
                    }
                }
            }
            
            // Send closed event when the loop exits
            let _ = inner.events_tx.send(TransportEvent::Closed).await;
            info!("UDP receive loop terminated");
        });
        
        // Store the task handle
        let mut task_guard = self.inner.receive_task.lock().await;
        *task_guard = Some(handle);
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
        debug!("UDP transport closing...");
        
        // Step 1: Signal shutdown to receive loop via watch channel
        let _ = self.inner.shutdown_tx.send(true);
        self.inner.closed.store(true, Ordering::Relaxed);
        
        // Step 2: Take the receive task handle and wait for it to finish
        let mut task_guard = self.inner.receive_task.lock().await;
        if let Some(handle) = task_guard.take() {
            debug!("Waiting for UDP receive loop to terminate...");
            // Wait for the task to finish (with timeout to prevent hanging)
            match tokio::time::timeout(
                std::time::Duration::from_secs(2),
                handle
            ).await {
                Ok(Ok(())) => {
                    debug!("UDP receive loop terminated cleanly");
                }
                Ok(Err(e)) => {
                    debug!("UDP receive loop task error: {}", e);
                }
                Err(_) => {
                    warn!("UDP receive loop termination timed out after 2 seconds");
                }
            }
        }
        drop(task_guard);
        
        // Step 3: Send a final closed event to notify upper layers
        // But check if the channel is still open
        let _ = self.inner.events_tx.try_send(TransportEvent::Closed);
        
        info!("UDP transport closed successfully");
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