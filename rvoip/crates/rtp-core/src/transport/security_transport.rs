//! Security-aware RTP transport wrapper
//!
//! This module provides a wrapper around the UDP transport that adds SRTP encryption/decryption.

use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, warn, error};
use bytes::Bytes;

use crate::error::Error;
use crate::Result;
use crate::packet::RtpPacket;
use crate::packet::rtcp::RtcpPacket;
use crate::transport::{RtpTransport, UdpRtpTransport};
use crate::srtp::SrtpContext;
use crate::traits::RtpEvent;
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
                        
                        let mut decryption_success = false;
                        
                        if srtp_enabled {
                            // Try SRTP decryption first - use write lock directly
                            let mut srtp_guard = srtp_context.write().await;
                            if let Some(srtp_ctx) = srtp_guard.as_mut() {
                                debug!("Attempting SRTP decryption on {} bytes", size);
                                
                                match srtp_ctx.unprotect(packet_data) {
                                    Ok(decrypted_packet) => {
                                        debug!("SRTP decryption successful: {} -> {} bytes", 
                                               size, decrypted_packet.size());
                                        
                                        // Create a MediaReceived event with the decrypted packet's payload
                                        let decrypted_event = RtpEvent::MediaReceived {
                                            payload_type: decrypted_packet.header.payload_type,
                                            timestamp: decrypted_packet.header.timestamp,
                                            marker: decrypted_packet.header.marker,
                                            payload: decrypted_packet.payload.clone(),
                                            source: addr,
                                        };
                                        
                                        debug!("Successfully decrypted and parsed: SSRC={:08x}, PT={}, seq={}, payload={} bytes", 
                                               decrypted_packet.header.ssrc, decrypted_packet.header.payload_type, 
                                               decrypted_packet.header.sequence_number, decrypted_packet.payload.len());
                                        
                                        // Forward the decrypted event
                                        if let Err(e) = event_tx.send(decrypted_event) {
                                            debug!("Failed to forward decrypted event: {}", e);
                                        }
                                        
                                        decryption_success = true;
                                    },
                                    Err(e) => {
                                        debug!("SRTP decryption failed, treating as plain RTP: {}", e);
                                        // Will fall through to process as plain RTP
                                    }
                                }
                            } else {
                                debug!("SRTP enabled but no context available");
                            }
                            // Release the write lock by dropping srtp_guard
                            drop(srtp_guard);
                        }
                        
                        // Only process as plain RTP if decryption failed or SRTP is disabled
                        if !decryption_success {
                            debug!("Processing as plain RTP packet: {} bytes", size);
                            
                            // Parse as regular RTP packet
                            match RtpPacket::parse(packet_data) {
                                Ok(rtp_packet) => {
                                    debug!("Parsed plain RTP packet: SSRC={:08x}, PT={}, seq={}, payload={} bytes", 
                                           rtp_packet.header.ssrc, rtp_packet.header.payload_type, 
                                           rtp_packet.header.sequence_number, rtp_packet.payload.len());
                                    
                                    let rtp_event = RtpEvent::MediaReceived {
                                        payload_type: rtp_packet.header.payload_type,
                                        timestamp: rtp_packet.header.timestamp,
                                        marker: rtp_packet.header.marker,
                                        payload: rtp_packet.payload.clone(),
                                        source: addr,
                                    };
                                    
                                    if let Err(e) = event_tx.send(rtp_event) {
                                        debug!("Failed to forward RTP event: {}", e);
                                    }
                                },
                                Err(e) => {
                                    debug!("Failed to parse as RTP packet: {}", e);
                                    
                                    // Create fallback event with raw data
                                    let fallback_event = RtpEvent::MediaReceived {
                                        payload_type: 0,
                                        timestamp: 0,
                                        marker: false,
                                        payload: Bytes::copy_from_slice(packet_data),
                                        source: addr,
                                    };
                                    
                                    if let Err(e) = event_tx.send(fallback_event) {
                                        debug!("Failed to forward fallback event: {}", e);
                                    }
                                }
                            }
                        }
                    },
                    Err(e) => {
                        error!("Error receiving raw packet: {}", e);
                        
                        // Send error event
                        let err_event = RtpEvent::Error(Error::Transport(format!("Socket error: {}", e)));
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
    
    /// Set the SRTP context for this transport
    pub async fn set_srtp_context(&self, context: SrtpContext) {
        let mut srtp_guard = self.srtp_context.write().await;
        *srtp_guard = Some(context);
        debug!("SRTP context set on security transport");
    }
    
    /// Get the underlying UDP transport
    pub fn inner_transport(&self) -> &Arc<UdpRtpTransport> {
        &self.inner
    }
    
    /// Check if SRTP is enabled and available
    pub async fn is_srtp_ready(&self) -> bool {
        if !self.srtp_enabled {
            return false;
        }
        let srtp_guard = self.srtp_context.read().await;
        srtp_guard.is_some()
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
            // Try to encrypt with SRTP
            let mut srtp_guard = self.srtp_context.write().await;
            if let Some(srtp_context) = srtp_guard.as_mut() {
                debug!("Encrypting RTP packet with SRTP: PT={}, SEQ={}, TS={}", 
                       packet.header.payload_type, packet.header.sequence_number, packet.header.timestamp);
                
                match srtp_context.protect(packet) {
                    Ok(protected_packet) => {
                        // Serialize the protected packet
                        match protected_packet.serialize() {
                            Ok(protected_bytes) => {
                                debug!("SRTP encryption successful: {} -> {} bytes", 
                                       packet.serialize()?.len(), protected_bytes.len());
                                
                                // Send the encrypted bytes
                                return self.inner.send_rtp_bytes(&protected_bytes, dest).await;
                            },
                            Err(e) => {
                                error!("Failed to serialize protected RTP packet: {}", e);
                                // Fall through to send unencrypted
                            }
                        }
                    },
                    Err(e) => {
                        error!("SRTP encryption failed: {}", e);
                        // Fall through to send unencrypted
                    }
                }
            } else {
                warn!("SRTP enabled but no context available - sending unencrypted");
            }
        }
        
        // Send unencrypted (either SRTP disabled or encryption failed)
        debug!("Sending unencrypted RTP packet");
        self.inner.send_rtp(packet, dest).await
    }
    
    async fn send_rtp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        // For raw bytes, we can't encrypt them (we need an RTP packet structure)
        // So just pass through to the inner transport
        debug!("Sending raw RTP bytes (cannot encrypt)");
        self.inner.send_rtp_bytes(bytes, dest).await
    }
    
    async fn send_rtcp(&self, packet: &RtcpPacket, dest: SocketAddr) -> Result<()> {
        if self.srtp_enabled {
            // Try to encrypt with SRTCP
            let mut srtp_guard = self.srtp_context.write().await;
            if let Some(srtp_context) = srtp_guard.as_mut() {
                debug!("Encrypting RTCP packet with SRTCP");
                
                // Serialize the RTCP packet first
                match packet.serialize() {
                    Ok(rtcp_bytes) => {
                        match srtp_context.protect_rtcp(&rtcp_bytes) {
                            Ok(protected_bytes) => {
                                debug!("SRTCP encryption successful: {} -> {} bytes", 
                                       rtcp_bytes.len(), protected_bytes.len());
                                
                                // Send the encrypted bytes
                                return self.inner.send_rtcp_bytes(&protected_bytes, dest).await;
                            },
                            Err(e) => {
                                error!("SRTCP encryption failed: {}", e);
                                // Fall through to send unencrypted
                            }
                        }
                    },
                    Err(e) => {
                        error!("Failed to serialize RTCP packet: {}", e);
                        // Fall through to send unencrypted
                    }
                }
            } else {
                warn!("SRTP enabled but no context available - sending unencrypted RTCP");
            }
        }
        
        // Send unencrypted RTCP
        debug!("Sending unencrypted RTCP packet");
        self.inner.send_rtcp(packet, dest).await
    }
    
    async fn send_rtcp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.srtp_enabled {
            // Try to encrypt with SRTCP
            let mut srtp_guard = self.srtp_context.write().await;
            if let Some(srtp_context) = srtp_guard.as_mut() {
                debug!("Encrypting raw RTCP bytes with SRTCP");
                
                match srtp_context.protect_rtcp(bytes) {
                    Ok(protected_bytes) => {
                        debug!("SRTCP encryption successful: {} -> {} bytes", 
                               bytes.len(), protected_bytes.len());
                        
                        // Send the encrypted bytes
                        return self.inner.send_rtcp_bytes(&protected_bytes, dest).await;
                    },
                    Err(e) => {
                        error!("SRTCP encryption failed: {}", e);
                        // Fall through to send unencrypted
                    }
                }
            } else {
                warn!("SRTP enabled but no context available - sending unencrypted RTCP");
            }
        }
        
        // Send unencrypted RTCP
        debug!("Sending unencrypted RTCP bytes");
        self.inner.send_rtcp_bytes(bytes, dest).await
    }
    
    async fn receive_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        // Receive from underlying transport
        let (size, addr) = self.inner.receive_packet(buffer).await?;
        
        if self.srtp_enabled {
            // Try to decrypt with SRTP
            let mut srtp_guard = self.srtp_context.write().await;
            if let Some(srtp_context) = srtp_guard.as_mut() {
                debug!("Decrypting received packet with SRTP: {} bytes from {}", size, addr);
                
                // Attempt SRTP decryption
                match srtp_context.unprotect(&buffer[0..size]) {
                    Ok(decrypted_packet) => {
                        debug!("SRTP decryption successful: {} -> {} bytes", 
                               size, decrypted_packet.size());
                        
                        // Serialize decrypted packet back to buffer
                        match decrypted_packet.serialize() {
                            Ok(decrypted_bytes) => {
                                let copy_len = std::cmp::min(decrypted_bytes.len(), buffer.len());
                                buffer[0..copy_len].copy_from_slice(&decrypted_bytes[0..copy_len]);
                                
                                debug!("Successfully decrypted and copied {} bytes to buffer", copy_len);
                                return Ok((copy_len, addr));
                            },
                            Err(e) => {
                                error!("Failed to serialize decrypted RTP packet: {}", e);
                                // Fall through to return unencrypted data
                            }
                        }
                    },
                    Err(e) => {
                        debug!("SRTP decryption failed, assuming plain RTP: {}", e);
                        // Fall through to return unencrypted data
                    }
                }
            } else {
                debug!("SRTP enabled but no context available - passing through unencrypted");
            }
        }
        
        // Return original data (either SRTP disabled or decryption failed)
        debug!("Returning original packet data: {} bytes from {}", size, addr);
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