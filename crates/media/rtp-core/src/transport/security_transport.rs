//! Security-aware RTP transport wrapper
//!
//! This module provides a wrapper around the UDP transport that adds SRTP encryption/decryption.

use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, trace};

use crate::error::Error;
use crate::packet::rtcp::RtcpPacket;
use crate::packet::RtpPacket;
use crate::srtp::SrtpContext;
use crate::traits::RtpEvent;
use crate::transport::{RtpTransport, UdpRtpTransport};
use crate::Result;
use tokio::sync::broadcast;

/// Security-aware RTP transport that wraps UDP transport with SRTP
pub struct SecurityRtpTransport {
    /// Underlying UDP transport
    inner: Arc<UdpRtpTransport>,

    /// SRTP context for encryption/decryption
    srtp_context: Arc<RwLock<Option<SrtpContext>>>,

    /// Whether SRTP is enabled
    srtp_enabled: bool,

    /// Our own event broadcaster for decrypted events
    event_tx: broadcast::Sender<RtpEvent>,

    /// Task that intercepts and decrypts raw packets
    raw_packet_task: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl SecurityRtpTransport {
    /// Create a new security-aware transport
    pub async fn new(inner: Arc<UdpRtpTransport>, srtp_enabled: bool) -> Result<Self> {
        // Create our own event broadcast channel
        let (event_tx, _) = broadcast::channel(100);

        // If SRTP is enabled, stop the inner transport's receiver to avoid conflicts
        if srtp_enabled {
            // Latch the inner transport before exposing it through
            // `inner_transport`, so its public raw-byte API cannot bypass this
            // wrapper's SRTP path.
            inner.require_srtp();
            debug!("Stopping inner UDP transport receiver to avoid socket conflicts");
            inner.stop_receiver().await?;
        }

        let transport = Self {
            inner,
            srtp_context: Arc::new(RwLock::new(None)),
            srtp_enabled,
            event_tx,
            raw_packet_task: Arc::new(Mutex::new(None)),
        };

        // Start the raw packet interception task if SRTP is enabled
        if srtp_enabled {
            transport.start_raw_packet_task().await?;
        }

        Ok(transport)
    }

    /// Start the raw packet interception task that processes packets before RTP parsing
    async fn start_raw_packet_task(&self) -> Result<()> {
        let inner_socket = self.inner.get_socket();
        let srtp_context = self.srtp_context.clone();
        let event_tx = self.event_tx.clone();
        let srtp_enabled = self.srtp_enabled;

        let task = tokio::spawn(async move {
            debug!("Starting SRTP raw packet interception task");
            let mut buffer = vec![0u8; 2048]; // Buffer for receiving packets

            loop {
                // Receive raw packet data directly from the socket
                match inner_socket.recv_from(&mut buffer).await {
                    Ok((size, addr)) => {
                        let packet_data = &buffer[0..size];
                        debug!("Intercepted raw packet: {} bytes from {}", size, addr);

                        if srtp_enabled {
                            if super::udp::is_rtcp_packet(packet_data) {
                                trace!(
                                    "Dropping RTCP while SRTP is configured because authenticated SRTCP is unavailable"
                                );
                                continue;
                            }

                            let mut srtp_guard = srtp_context.write().await;
                            if let Some(srtp_ctx) = srtp_guard.as_mut() {
                                debug!("Attempting SRTP decryption on {} bytes", size);

                                match srtp_ctx.unprotect(packet_data) {
                                    Ok(decrypted_packet) => {
                                        debug!(
                                            "SRTP decryption successful: {} -> {} bytes",
                                            size,
                                            decrypted_packet.size()
                                        );

                                        // Create a MediaReceived event with the decrypted packet's payload
                                        let decrypted_event = RtpEvent::MediaReceived {
                                            payload_type: decrypted_packet.header.payload_type,
                                            sequence_number: decrypted_packet
                                                .header
                                                .sequence_number,
                                            timestamp: decrypted_packet.header.timestamp,
                                            marker: decrypted_packet.header.marker,
                                            payload: decrypted_packet.payload.clone(),
                                            padding_size: decrypted_packet.padding_size,
                                            source: addr,
                                            ssrc: decrypted_packet.header.ssrc,
                                        };

                                        debug!("Successfully decrypted and parsed: SSRC={:08x}, PT={}, seq={}, payload={} bytes",
                                               decrypted_packet.header.ssrc, decrypted_packet.header.payload_type,
                                               decrypted_packet.header.sequence_number, decrypted_packet.payload.len());

                                        // Forward the decrypted event
                                        if let Err(e) = event_tx.send(decrypted_event) {
                                            debug!("Failed to forward decrypted event: {}", e);
                                        }
                                    }
                                    Err(e) => trace!(
                                        "Dropping packet that failed SRTP authentication: {}",
                                        e
                                    ),
                                }
                            } else {
                                trace!("Dropping packet because SRTP has no cryptographic context");
                            }
                            drop(srtp_guard);
                            continue;
                        }

                        debug!("Processing as plain RTP packet: {} bytes", size);
                        match RtpPacket::parse(packet_data) {
                            Ok(rtp_packet) => {
                                let rtp_event = RtpEvent::MediaReceived {
                                    payload_type: rtp_packet.header.payload_type,
                                    sequence_number: rtp_packet.header.sequence_number,
                                    timestamp: rtp_packet.header.timestamp,
                                    marker: rtp_packet.header.marker,
                                    payload: rtp_packet.payload.clone(),
                                    padding_size: rtp_packet.padding_size,
                                    source: addr,
                                    ssrc: rtp_packet.header.ssrc,
                                };

                                if let Err(e) = event_tx.send(rtp_event) {
                                    debug!("Failed to forward RTP event: {}", e);
                                }
                            }
                            Err(e) => debug!("Dropping malformed plain RTP packet: {}", e),
                        }
                    }
                    Err(e) => {
                        error!("Error receiving raw packet: {}", e);

                        // Send error event
                        let err_event =
                            RtpEvent::Error(Error::Transport(format!("Socket error: {}", e)));
                        if let Err(e) = event_tx.send(err_event) {
                            debug!("Failed to send error event: {}", e);
                        }

                        // Short delay before retrying
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }
                }
            }
        });

        let mut task_guard = self.raw_packet_task.lock().await;
        *task_guard = Some(task);

        Ok(())
    }

    /// Set the SRTP context for a wrapper that was created in SRTP mode.
    ///
    /// A plaintext wrapper cannot be upgraded after construction because its
    /// receiver and inner-transport latch were configured for plaintext. The
    /// caller must construct a new SRTP-enabled wrapper instead.
    pub async fn set_srtp_context(&self, context: SrtpContext) -> Result<()> {
        if !self.srtp_enabled {
            return Err(Error::InvalidState(
                "cannot install an SRTP context on a plaintext security transport".to_string(),
            ));
        }
        context.validate_for_secure_transport()?;
        let mut srtp_guard = self.srtp_context.write().await;
        *srtp_guard = Some(context);
        debug!("SRTP context set on security transport");
        Ok(())
    }

    /// Get the underlying UDP transport for low-level diagnostics.
    ///
    /// Its raw socket handle is an unauthenticated escape from this wrapper.
    /// Public RTP byte sends remain latched closed in SRTP mode, but callers
    /// using `UdpRtpTransport::get_socket` directly bypass all SRTP policy and
    /// must not treat those reads or writes as protected media operations.
    pub fn inner_transport(&self) -> &Arc<UdpRtpTransport> {
        &self.inner
    }

    /// Check if SRTP is enabled and available
    pub async fn is_srtp_ready(&self) -> bool {
        if !self.srtp_enabled {
            return false;
        }
        let srtp_guard = self.srtp_context.read().await;
        srtp_guard
            .as_ref()
            .is_some_and(|context| context.validate_for_secure_transport().is_ok())
    }
}

#[async_trait]
impl RtpTransport for SecurityRtpTransport {
    fn local_rtp_addr(&self) -> Result<SocketAddr> {
        self.inner.local_rtp_addr()
    }

    fn local_rtcp_addr(&self) -> Result<Option<SocketAddr>> {
        self.inner.local_rtcp_addr()
    }

    async fn send_rtp(&self, packet: &RtpPacket, dest: SocketAddr) -> Result<()> {
        if self.srtp_enabled {
            let mut srtp_guard = self.srtp_context.write().await;
            let srtp_context = srtp_guard.as_mut().ok_or_else(|| {
                Error::InvalidState("SRTP is enabled but no context is installed".to_string())
            })?;
            let protected_bytes = srtp_context.protect(packet)?.serialize()?;
            return self
                .inner
                .send_protected_rtp_bytes(&protected_bytes, dest)
                .await;
        }

        self.inner.send_rtp(packet, dest).await
    }

    async fn send_rtp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.srtp_enabled {
            return Err(Error::InvalidState(
                "raw RTP bytes cannot bypass an enabled SRTP context".to_string(),
            ));
        }
        self.inner.send_rtp_bytes(bytes, dest).await
    }

    async fn send_rtcp(&self, packet: &RtcpPacket, dest: SocketAddr) -> Result<()> {
        if self.srtp_enabled {
            return Err(Error::UnsupportedFeature(
                "RTCP is disabled while SRTP is configured until authenticated SRTCP is implemented"
                    .to_string(),
            ));
        }

        self.inner.send_rtcp(packet, dest).await
    }

    async fn send_rtcp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.srtp_enabled {
            return Err(Error::UnsupportedFeature(
                "RTCP is disabled while SRTP is configured until authenticated SRTCP is implemented"
                    .to_string(),
            ));
        }

        self.inner.send_rtcp_bytes(bytes, dest).await
    }

    async fn receive_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        if self.srtp_enabled {
            return Err(Error::UnsupportedFeature(
                "direct receive is unavailable on SecurityRtpTransport in SRTP mode; use the authenticated event subscription path"
                    .to_string(),
            ));
        }

        // Receive from underlying transport
        let (size, addr) = self.inner.receive_packet(buffer).await?;

        Ok((size, addr))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn subscribe(&self) -> broadcast::Receiver<RtpEvent> {
        // Return our own event stream (which contains decrypted events)
        // instead of the inner transport's event stream
        self.event_tx.subscribe()
    }

    async fn close(&self) -> Result<()> {
        // Stop the raw packet interception task
        let mut task_guard = self.raw_packet_task.lock().await;
        if let Some(task) = task_guard.take() {
            debug!("Stopping SRTP raw packet interception task");
            task.abort();
        }

        // Close the inner transport
        self.inner.close().await
    }
}
