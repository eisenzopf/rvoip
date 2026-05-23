mod listener;
mod sender;
mod socket;

pub use listener::UdpListener;
pub use sender::UdpSender;
pub use socket::UdpSocketOptions;

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

use crate::error::{Error, Result};
use crate::transport::{Transport, TransportEvent, TransportType};
use rvoip_sip_core::Message;

// Default channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 1000;

/// RFC 3261 §18.1.1 — outbound SIP requests larger than this MUST be
/// shipped over a congestion-controlled transport (TCP) rather than UDP
/// when path MTU is unknown. This is the safe default; deployments
/// with known path MTU can override via [`UdpTransport::bind_with_mtu`].
pub const UDP_SAFE_MAX_BYTES: usize = 1300;

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
    /// Per-instance MTU threshold for the RFC 3261 §18.1.1 UDP→TCP
    /// failover. Defaults to [`UDP_SAFE_MAX_BYTES`]; configurable so
    /// deployments with a known smaller path MTU (e.g. tunnelled SIP
    /// over IPSec) or a known larger one (e.g. controlled DC fabric)
    /// can tune the threshold.
    safe_max_bytes: usize,
    socket_options: UdpSocketOptions,
}

impl UdpTransport {
    /// Creates a new UDP transport bound to the specified address.
    /// Uses the RFC 3261 §18.1.1 default MTU threshold
    /// ([`UDP_SAFE_MAX_BYTES`]).
    pub async fn bind(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_mtu(addr, channel_capacity, UDP_SAFE_MAX_BYTES).await
    }

    /// Creates a new UDP transport with explicit socket options.
    pub async fn bind_with_socket_options(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
        socket_options: UdpSocketOptions,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_mtu_and_socket_options(
            addr,
            channel_capacity,
            UDP_SAFE_MAX_BYTES,
            socket_options,
        )
        .await
    }

    /// Same as [`Self::bind`] but with a caller-supplied MTU threshold
    /// for the UDP→TCP failover decision (RFC 3261 §18.1.1). Useful
    /// for deployments with a known smaller path MTU (e.g. SIP over
    /// IPSec) or a known-safe larger one (DC fabric).
    pub async fn bind_with_mtu(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
        safe_max_bytes: usize,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_mtu_and_socket_options(
            addr,
            channel_capacity,
            safe_max_bytes,
            UdpSocketOptions::default(),
        )
        .await
    }

    /// Same as [`Self::bind_with_mtu`] but with explicit UDP socket options.
    pub async fn bind_with_mtu_and_socket_options(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
        safe_max_bytes: usize,
        socket_options: UdpSocketOptions,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        // Create the event channel
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);

        // Create the UDP listener
        let listener = UdpListener::bind_with_socket_options(addr, socket_options).await?;
        let local_addr = listener.local_addr()?;
        info!(
            "SIP UDP transport bound to {} (MTU threshold {} bytes, socket options {:?})",
            local_addr, safe_max_bytes, socket_options
        );

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
                safe_max_bytes,
                socket_options,
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
                safe_max_bytes: UDP_SAFE_MAX_BYTES,
                socket_options: UdpSocketOptions::default(),
            }),
        }
    }

    /// Returns the socket options requested at bind time.
    pub fn socket_options(&self) -> UdpSocketOptions {
        self.inner.socket_options
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
                                            transport_type: TransportType::Udp,
                                            // Move `packet` straight in — `Bytes` is
                                            // already Arc-managed internally; the
                                            // previous `Arc::new(packet.clone())`
                                            // double-wrapped it for no reason.
                                            raw_bytes: Some(packet),
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
        info!(
            "Sending {} message to {}",
            if let Message::Request(ref req) = message {
                format!("{}", req.method)
            } else {
                "response".to_string()
            },
            destination
        );

        // Send the message using the sender
        self.inner.sender.send(&bytes, destination).await
    }

    async fn send_message_raw(&self, bytes: bytes::Bytes, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        debug!(
            "UDP: sending {} pre-built bytes to {}",
            bytes.len(),
            destination
        );
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
            match tokio::time::timeout(std::time::Duration::from_secs(2), handle).await {
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

    fn max_safe_message_size(&self) -> usize {
        self.inner.safe_max_bytes
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bind_uses_default_mtu_threshold() {
        let (transport, _rx) = UdpTransport::bind("127.0.0.1:0".parse().unwrap(), None)
            .await
            .expect("bind");
        assert_eq!(transport.max_safe_message_size(), UDP_SAFE_MAX_BYTES);
        transport.close().await.ok();
    }

    #[tokio::test]
    async fn bind_with_mtu_honours_explicit_override() {
        let (transport, _rx) =
            UdpTransport::bind_with_mtu("127.0.0.1:0".parse().unwrap(), None, 900)
                .await
                .expect("bind_with_mtu");
        assert_eq!(transport.max_safe_message_size(), 900);
        transport.close().await.ok();
    }

    #[tokio::test]
    async fn bind_with_socket_options_preserves_requested_values() {
        let options = UdpSocketOptions::new(Some(4096), Some(4096));
        let (transport, _rx) =
            UdpTransport::bind_with_socket_options("127.0.0.1:0".parse().unwrap(), None, options)
                .await
                .expect("bind_with_socket_options");
        assert_eq!(transport.socket_options(), options);
        transport.close().await.ok();
    }
}
