//! UDP transport for RTP/RTCP
//!
//! This module provides a UDP-based implementation of the RTP transport.

use std::net::SocketAddr;
use std::sync::Arc;
use std::any::Any;

use async_trait::async_trait;
use tokio::net::UdpSocket;
use tokio::sync::{Mutex, mpsc, broadcast};
use tokio::task::JoinHandle;
use bytes::Bytes;
use tracing::{error, warn, debug, trace, info};

use crate::error::Error;
use crate::Result;
use crate::packet::RtpPacket;
use crate::packet::rtcp::RtcpPacket;
use crate::traits::RtpEvent;
use crate::DEFAULT_MAX_PACKET_SIZE;
use super::{RtpTransport, RtpTransportConfig};
use super::validation::{PlatformSocketStrategy, RtpSocketValidator};
use super::allocator::{GlobalPortAllocator, PairingStrategy};

/// UDP transport for RTP/RTCP
///
/// This implementation supports RTCP multiplexing as defined in RFC 5761,
/// allowing RTP and RTCP packets to be sent and received on the same port.
/// 
/// When RTCP multiplexing is enabled (via the `rtcp_mux` field in `RtpTransportConfig`),
/// both RTP and RTCP packets are sent and received on the RTP socket. The transport
/// automatically distinguishes between RTP and RTCP packets based on the payload type:
/// 
/// * RTCP packets have payload types 200-204 (as defined in RFC 5761)
/// * RTP packets use payload types 0-127
/// 
/// RTCP multiplexing is recommended for WebRTC and modern VoIP applications
/// as it simplifies NAT traversal and reduces the number of ports required.
pub struct UdpRtpTransport {
    /// RTP socket
    rtp_socket: Arc<UdpSocket>,
    
    /// RTCP socket (if separate from RTP)
    rtcp_socket: Option<Arc<UdpSocket>>,
    
    /// Transport configuration
    config: RtpTransportConfig,
    
    /// Remote RTP address
    remote_rtp_addr: Arc<Mutex<Option<SocketAddr>>>,
    
    /// Remote RTCP address
    remote_rtcp_addr: Arc<Mutex<Option<SocketAddr>>>,
    
    /// Event broadcaster
    event_tx: broadcast::Sender<RtpEvent>,
    
    /// Receiver task
    receiver_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    
    /// Whether the transport is active
    active: Arc<Mutex<bool>>,
}

impl UdpRtpTransport {
    /// Create a new UDP transport for RTP
    pub async fn new(config: RtpTransportConfig) -> Result<Self> {
        // Use platform-specific socket strategy
        let socket_strategy = PlatformSocketStrategy::for_current_platform();

        // Determine how to create the sockets based on config
        let (socket_rtp, socket_rtcp, local_rtp_addr, local_rtcp_addr) = if config.use_port_allocator {
            // Generate a session ID if not provided
            let session_id = config.session_id.clone().unwrap_or_else(|| {
                use rand::Rng;
                let random_suffix: u32 = rand::thread_rng().gen();
                format!("rtp-session-{}", random_suffix)
            });

            // Get the global port allocator
            let allocator = GlobalPortAllocator::instance().await;
            
            // Configure the pairing strategy based on rtcp_mux
            let pairing_strategy = if config.rtcp_mux {
                PairingStrategy::Muxed
            } else {
                PairingStrategy::Adjacent
            };
            
            // Determine IP from the provided address (keep the same IP, ignore port)
            let ip = config.local_rtp_addr.ip();
            
            // Allocate port(s)
            debug!("Allocating port(s) with strategy: {:?}", pairing_strategy);
            let (rtp_addr, rtcp_addr_opt) = allocator.allocate_port_pair(&session_id, Some(ip)).await?;
            
            debug!("Allocated RTP port: {}", rtp_addr);
            if let Some(rtcp_addr) = rtcp_addr_opt {
                debug!("Allocated RTCP port: {}", rtcp_addr);
            }
            
            // Create sockets with the allocated ports
            let socket_rtp = allocator.create_validated_socket(rtp_addr).await?;
            
            let socket_rtcp = if let Some(rtcp_addr) = rtcp_addr_opt {
                Some(allocator.create_validated_socket(rtcp_addr).await?)
            } else {
                None
            };
            
            (socket_rtp, socket_rtcp, rtp_addr, rtcp_addr_opt)
        } else {
            // Traditional socket binding without the allocator
            // Create RTP socket
            let socket_rtp = UdpSocket::bind(config.local_rtp_addr).await
                .map_err(|e| Error::Transport(format!("Failed to bind RTP socket: {}", e)))?;
            
            // Apply platform-specific settings to the socket
            socket_strategy.apply_to_socket(&socket_rtp).await
                .map_err(|e| Error::Transport(format!("Failed to configure RTP socket: {}", e)))?;
                
            // Get bound address
            let local_rtp_addr = socket_rtp.local_addr()
                .map_err(|e| Error::Transport(format!("Failed to get local RTP address: {}", e)))?;
            
            debug!("Bound RTP socket to {}", local_rtp_addr);
            
            // Create RTCP socket if not using RTCP-MUX
            let (socket_rtcp, local_rtcp_addr) = if !config.rtcp_mux {
                // Use the next port for RTCP (per convention)
                let local_rtcp_addr = match config.local_rtcp_addr {
                    Some(addr) => addr,
                    None => {
                        let rtcp_port = local_rtp_addr.port() + 1;
                        SocketAddr::new(local_rtp_addr.ip(), rtcp_port)
                    }
                };
                
                // Create RTCP socket
                let socket_rtcp = UdpSocket::bind(local_rtcp_addr).await
                    .map_err(|e| Error::Transport(format!("Failed to bind RTCP socket: {}", e)))?;
                    
                // Apply platform-specific settings to the socket
                socket_strategy.apply_to_socket(&socket_rtcp).await
                    .map_err(|e| Error::Transport(format!("Failed to configure RTCP socket: {}", e)))?;
                    
                // Get bound address
                let local_rtcp_addr = socket_rtcp.local_addr()
                    .map_err(|e| Error::Transport(format!("Failed to get local RTCP address: {}", e)))?;
                    
                debug!("Bound RTCP socket to {}", local_rtcp_addr);
                
                (Some(socket_rtcp), Some(local_rtcp_addr))
            } else {
                debug!("Using RTCP-MUX - no separate RTCP socket");
                (None, None)
            };
            
            (socket_rtp, socket_rtcp, local_rtp_addr, local_rtcp_addr)
        };

        // Create broadcaster
        let (event_tx, _) = broadcast::channel(100);
        
        let transport = Self {
            rtp_socket: Arc::new(socket_rtp),
            rtcp_socket: socket_rtcp.map(Arc::new),
            config,
            remote_rtp_addr: Arc::new(Mutex::new(None)),
            remote_rtcp_addr: Arc::new(Mutex::new(None)),
            event_tx,
            receiver_task: Arc::new(Mutex::new(None)),
            active: Arc::new(Mutex::new(false)),
        };
        
        // Start the receiver task
        transport.start_receiver().await?;
        
        Ok(transport)
    }
    
    /// Start the packet receiver task
    async fn start_receiver(&self) -> Result<()> {
        let mut active = self.active.lock().await;
        if *active {
            return Ok(());
        }
        
        // Set active state
        *active = true;
        
        // Start RTP receiver
        let rtp_socket = self.rtp_socket.clone();
        let event_tx = self.event_tx.clone();
        let active_state = self.active.clone();
        
        let rtp_receiver = tokio::spawn(async move {
            let mut buffer = vec![0u8; DEFAULT_MAX_PACKET_SIZE];
            
            loop {
                // Check if we should continue running
                if !*active_state.lock().await {
                    break;
                }
                
                // Receive packet
                match rtp_socket.recv_from(&mut buffer).await {
                    Ok((size, addr)) => {
                        // Check if it looks like an RTP or RTCP packet
                        if size < 8 {
                            // Too small to be either RTP or RTCP
                            warn!("Received packet too small: {} bytes", size);
                            continue;
                        }
                        
                        // Check if it's RTCP according to RFC 5761
                        if is_rtcp_packet(&buffer[..size]) {
                            // This is an RTCP packet
                            debug!("Received RTCP packet, type: {}", buffer[1] & 0x7F);
                            let rtcp_data = Bytes::copy_from_slice(&buffer[0..size]);
                            let event = RtpEvent::RtcpReceived {
                                data: rtcp_data,
                                source: addr,
                            };
                            
                            // Only log errors if there are receivers
                            if event_tx.receiver_count() > 0 {
                                if let Err(e) = event_tx.send(event) {
                                    warn!("Failed to send RTCP event: {}", e);
                                }
                            } else {
                                // Still send the event but ignore errors if no one is listening
                                let _ = event_tx.send(event);
                            }
                        } else {
                            // Try to parse as RTP
                            match RtpPacket::parse(&buffer[0..size]) {
                                Ok(packet) => {
                                    // Log packet reception at transport level
                                    info!("Transport received packet with SSRC={:08x}, seq={}, ts={}",
                                           packet.header.ssrc, 
                                           packet.header.sequence_number,
                                           packet.header.timestamp);
                                    
                                    // Debug: Log SSRC demultiplexing info
                                    debug!("SSRC demultiplexing: Forwarding packet with SSRC={:08x}, seq={}, payload size={} bytes",
                                           packet.header.ssrc, packet.header.sequence_number, packet.payload.len());
                                    
                                    // Create RTP event
                                    let event = RtpEvent::MediaReceived {
                                        payload_type: packet.header.payload_type,
                                        timestamp: packet.header.timestamp,
                                        marker: packet.header.marker,
                                        payload: packet.payload.clone(), // Use the parsed payload
                                        source: addr,
                                    };
                                    
                                    // Only log errors if there are receivers
                                    if event_tx.receiver_count() > 0 {
                                        if let Err(e) = event_tx.send(event) {
                                            warn!("Failed to send RTP event: {}", e);
                                        }
                                    } else {
                                        // Still send the event but ignore errors if no one is listening
                                        let _ = event_tx.send(event);
                                    }
                                }
                                Err(e) => {
                                    warn!("Failed to parse RTP packet: {}", e);
                                    
                                    // Even though parsing failed, we still need to generate a MediaReceived event
                                    // This allows higher layers to handle non-standard or malformed packets
                                    
                                    // Use default/fallback values for required fields
                                    let fallback_payload_type = if size > 1 { buffer[1] & 0x7F } else { 0 };
                                    let fallback_timestamp = if size >= 8 {
                                        let mut ts = 0u32;
                                        ts |= (buffer[4] as u32) << 24;
                                        ts |= (buffer[5] as u32) << 16;
                                        ts |= (buffer[6] as u32) << 8;
                                        ts |= buffer[7] as u32;
                                        ts
                                    } else {
                                        0
                                    };
                                    
                                    let fallback_marker = if size > 1 { (buffer[1] & 0x80) != 0 } else { false };
                                    
                                    // Create the payload from the entire packet
                                    // This allows the application layer to implement its own parsing if needed
                                    let raw_payload = Bytes::copy_from_slice(&buffer[0..size]);
                                    
                                    debug!("Generating fallback MediaReceived event for non-RTP packet ({})", e);
                                    
                                    // Create a MediaReceived event with the data we have
                                    let event = RtpEvent::MediaReceived {
                                        payload_type: fallback_payload_type,
                                        timestamp: fallback_timestamp,
                                        marker: fallback_marker,
                                        payload: raw_payload,
                                        source: addr,
                                    };
                                    
                                    // Send the event
                                    if event_tx.receiver_count() > 0 {
                                        if let Err(e) = event_tx.send(event) {
                                            warn!("Failed to send fallback MediaReceived event: {}", e);
                                        }
                                    } else {
                                        let _ = event_tx.send(event);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Error receiving packet: {}", e);
                        
                        // Send error event
                        let err_event = RtpEvent::Error(Error::Transport(format!("Socket error: {}", e)));
                        if event_tx.receiver_count() > 0 {
                            let _ = event_tx.send(err_event);
                        }
                        
                        // Short delay before retrying
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }
                }
            }
        });
        
        // Store task handle
        let mut receiver_task = self.receiver_task.lock().await;
        *receiver_task = Some(rtp_receiver);
        
        // If we have a separate RTCP socket, start that receiver too
        if let Some(rtcp_socket) = &self.rtcp_socket {
            let rtcp_socket = rtcp_socket.clone();
            let event_tx = self.event_tx.clone();
            let active_state = self.active.clone();
            
            let rtcp_receiver = tokio::spawn(async move {
                let mut buffer = vec![0u8; DEFAULT_MAX_PACKET_SIZE];
                
                loop {
                    // Check if we should continue running
                    if !*active_state.lock().await {
                        break;
                    }
                    
                    // Receive packet
                    match rtcp_socket.recv_from(&mut buffer).await {
                        Ok((size, addr)) => {
                            // Create RTCP event
                            let rtcp_data = Bytes::copy_from_slice(&buffer[0..size]);
                            let event = RtpEvent::RtcpReceived {
                                data: rtcp_data,
                                source: addr,
                            };
                            
                            // Only log errors if there are receivers
                            if event_tx.receiver_count() > 0 {
                                if let Err(e) = event_tx.send(event) {
                                    warn!("Failed to send RTCP event: {}", e);
                                }
                            } else {
                                // Still send the event but ignore errors if no one is listening
                                let _ = event_tx.send(event);
                            }
                        }
                        Err(e) => {
                            error!("Error receiving RTCP packet: {}", e);
                            
                            // Send error event
                            let err_event = RtpEvent::Error(Error::Transport(format!("RTCP socket error: {}", e)));
                            if event_tx.receiver_count() > 0 {
                                let _ = event_tx.send(err_event);
                            }
                            
                            // Short delay before retrying
                            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                        }
                    }
                }
            });
            
            // Store in the same place - we only care about tracking any active tasks
            *receiver_task = Some(rtcp_receiver);
        }
        
        info!("Started UDP transport receiver tasks");
        Ok(())
    }
    
    /// Stop the receiver task
    pub async fn stop_receiver(&self) -> Result<()> {
        // Set inactive state
        let mut active = self.active.lock().await;
        *active = false;
        
        // Wait for receiver task to complete
        let mut receiver_task = self.receiver_task.lock().await;
        if let Some(task) = receiver_task.take() {
            task.abort();
        }
        
        Ok(())
    }
    
    /// Set the remote RTP address
    pub async fn set_remote_rtp_addr(&self, addr: SocketAddr) {
        let mut remote_addr = self.remote_rtp_addr.lock().await;
        *remote_addr = Some(addr);
    }
    
    /// Set the remote RTCP address
    pub async fn set_remote_rtcp_addr(&self, addr: SocketAddr) {
        let mut remote_addr = self.remote_rtcp_addr.lock().await;
        *remote_addr = Some(addr);
    }
    
    /// Get the remote RTP address
    pub async fn remote_rtp_addr(&self) -> Option<SocketAddr> {
        let remote_addr = self.remote_rtp_addr.lock().await;
        *remote_addr
    }
    
    /// Get the remote RTCP address
    pub async fn remote_rtcp_addr(&self) -> Option<SocketAddr> {
        let remote_addr = self.remote_rtcp_addr.lock().await;
        *remote_addr
    }
    
    /// Subscribe to transport events
    pub fn subscribe(&self) -> broadcast::Receiver<RtpEvent> {
        self.event_tx.subscribe()
    }
    
    /// Get a clone of the RTP socket
    /// This is used when sharing the same socket with other protocols (e.g., DTLS)
    pub fn get_socket(&self) -> Arc<UdpSocket> {
        self.rtp_socket.clone()
    }
}

#[async_trait]
impl RtpTransport for UdpRtpTransport {
    fn local_rtp_addr(&self) -> Result<SocketAddr> {
        self.rtp_socket.local_addr()
            .map_err(|e| Error::Transport(format!("Failed to get local RTP address: {}", e)))
    }
    
    /// Get the local RTCP address
    fn local_rtcp_addr(&self) -> Result<Option<SocketAddr>> {
        Ok(self.config.local_rtcp_addr)
    }
    
    async fn send_rtp(&self, packet: &RtpPacket, dest: SocketAddr) -> Result<()> {
        // Serialize the packet
        let data = packet.serialize()?;
        
        // Send the bytes
        self.send_rtp_bytes(&data, dest).await
    }
    
    async fn send_rtp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.config.symmetric_rtp {
            // Update remote address if using symmetric RTP
            let mut remote_addr = self.remote_rtp_addr.lock().await;
            *remote_addr = Some(dest);
        }
        
        // Send the data
        self.rtp_socket.send_to(bytes, dest).await
            .map_err(|e| Error::Transport(format!("Failed to send RTP packet: {}", e)))?;
            
        Ok(())
    }
    
    async fn send_rtcp(&self, packet: &RtcpPacket, dest: SocketAddr) -> Result<()> {
        // Serialize the packet
        let data = packet.serialize()?;
        
        // Send the serialized bytes
        self.send_rtcp_bytes(&data, dest).await
    }
    
    async fn send_rtcp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.config.symmetric_rtp {
            // Update remote RTCP address if using symmetric RTP
            let mut remote_addr = self.remote_rtcp_addr.lock().await;
            *remote_addr = Some(dest);
        }
        
        // Use the appropriate socket for sending RTCP
        let socket = if self.config.rtcp_mux {
            // If RTCP-MUX is enabled, use the RTP socket for RTCP packets
            &self.rtp_socket
        } else if let Some(rtcp_socket) = &self.rtcp_socket {
            // If a separate RTCP socket exists, use it
            rtcp_socket
        } else {
            // Fallback to RTP socket if no RTCP socket is available
            &self.rtp_socket
        };
        
        // Send the data
        socket.send_to(bytes, dest).await
            .map_err(|e| Error::Transport(format!("Failed to send RTCP packet: {}", e)))?;
            
        Ok(())
    }
    
    async fn receive_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        // Receive data from the RTP socket
        self.rtp_socket.recv_from(buffer).await
            .map_err(|e| Error::Transport(format!("Failed to receive packet: {}", e)))
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn subscribe(&self) -> broadcast::Receiver<RtpEvent> {
        self.event_tx.subscribe()
    }
    
    async fn close(&self) -> Result<()> {
        // Stop the receiver task
        self.stop_receiver().await?;
        
        // If we used the port allocator, release the ports
        if self.config.use_port_allocator {
            if let Some(session_id) = &self.config.session_id {
                // Get the global allocator
                let allocator = GlobalPortAllocator::instance().await;
                
                // Release all ports associated with this session
                if let Err(e) = allocator.release_session(session_id).await {
                    warn!("Failed to release ports for session {}: {}", session_id, e);
                } else {
                    debug!("Released all ports for session {}", session_id);
                }
            }
        }
        
        // UDP sockets don't need explicit closing
        Ok(())
    }
}

/// Determine if a packet is RTCP according to RFC 5761
///
/// RFC 5761 specifies that RTP/RTCP multiplexing uses the following rules to 
/// distinguish RTCP from RTP packets:
///
/// 1. Packets with payload types in the range 64-95 could be either RTP or RTCP.
/// 2. For these ambiguous payload types, a packet is RTCP if:
///    - The payload type is in the range 64-95 AND
///    - The value corresponds to a known RTCP packet type (SR=200, RR=201, SDES=202, BYE=203, APP=204)
/// 3. All other packets in the range 64-95 are RTP.
/// 4. All packets with payload types in the range 0-63 and 96-127 are RTP.
///
/// See RFC 5761 section 4 for more details.
fn is_rtcp_packet(buffer: &[u8]) -> bool {
    if buffer.len() < 2 {
        return false;
    }
    
    let first_byte = buffer[0];
    let second_byte = buffer[1];
    
    let version = (first_byte >> 6) & 0x03;
    // For RTP, payload type is in the lower 7 bits of the second byte
    // For RTCP, packet type is the full second byte value
    
    // First check: If the payload type is between 200-204, it's definitely RTCP
    if version == 2 && (second_byte >= 200 && second_byte <= 204) {
        debug!("Identified RTCP packet: version={}, PT={}", version, second_byte);
        return true;
    }
    
    // Second check: For ambiguous range (64-95), we need to do additional checks
    let rtp_payload_type = second_byte & 0x7F;  // Strip marker bit
    if version == 2 && (rtp_payload_type >= 64 && rtp_payload_type <= 95) {
        // Additional checks could be implemented here
        // For example, checking packet structure specific to RTCP
        
        // For now, we'll conservatively treat this as RTP
        debug!("Ambiguous packet in PT range 64-95: {}, treating as RTP", rtp_payload_type);
        return false;
    }
    
    // If neither condition is met, it's an RTP packet
    debug!("Identified as RTP packet: version={}, PT={}", version, rtp_payload_type);
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::packet::RtpHeader;
    
    #[tokio::test]
    async fn test_udp_transport_creation() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()),
            symmetric_rtp: true,
            rtcp_mux: false, // Disable RTCP-MUX for this test
            session_id: Some("test_creation".to_string()),
            use_port_allocator: false,
        };
        
        let transport = UdpRtpTransport::new(config).await;
        assert!(transport.is_ok());
        
        let transport = transport.unwrap();
        let rtp_addr = transport.local_rtp_addr().unwrap();
        
        // For non-muxed connections, we should get assigned a real RTCP socket
        assert_ne!(rtp_addr.port(), 0);
        assert!(transport.rtcp_socket.is_some(), "RTCP socket should exist when rtcp_mux is false");
        
        // Check the actual RTCP socket address, not just the config value
        if let Some(rtcp_socket) = &transport.rtcp_socket {
            let rtcp_addr = rtcp_socket.local_addr().unwrap();
            assert_ne!(rtcp_addr.port(), 0);
            assert_ne!(rtp_addr.port(), rtcp_addr.port());
        }
    }
    
    #[tokio::test]
    async fn test_udp_transport_with_rtcp_mux() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()), // This should be ignored
            symmetric_rtp: true,
            rtcp_mux: true, // Enable RTCP-MUX
            session_id: Some("test_rtcp_mux".to_string()),
            use_port_allocator: false,
        };
        
        let transport = UdpRtpTransport::new(config).await;
        assert!(transport.is_ok());
        
        let transport = transport.unwrap();
        let rtp_addr = transport.local_rtp_addr().unwrap();
        
        assert_ne!(rtp_addr.port(), 0, "RTP port should not be 0");
        
        // With RTCP-MUX, no separate RTCP socket should be created
        assert!(transport.rtcp_socket.is_none(), "RTCP socket should be None with rtcp_mux enabled");
        
        // The config should retain the original RTCP address - it doesn't matter
        // what this is with RTCP-MUX as it's not used
        let rtcp_addr_option = transport.local_rtcp_addr().unwrap();
        assert!(rtcp_addr_option.is_some(), "RTCP address should be available in the config");
    }
    
    #[tokio::test]
    async fn test_rtcp_packet_detection() {
        // Test RTCP detection with SR packet (PT=200)
        let mut sr_packet = vec![0x80, 200, 0, 0]; // Version=2, PT=200 (SR)
        sr_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&sr_packet));
        
        // Test RTCP detection with RR packet (PT=201)
        let mut rr_packet = vec![0x80, 201, 0, 0]; // Version=2, PT=201 (RR)
        rr_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&rr_packet));
        
        // Test RTCP detection with SDES packet (PT=202)
        let mut sdes_packet = vec![0x80, 202, 0, 0]; // Version=2, PT=202 (SDES)
        sdes_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&sdes_packet));
        
        // Test RTCP detection with BYE packet (PT=203)
        let mut bye_packet = vec![0x80, 203, 0, 0]; // Version=2, PT=203 (BYE)
        bye_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&bye_packet));
        
        // Test RTCP detection with APP packet (PT=204)
        let mut app_packet = vec![0x80, 204, 0, 0]; // Version=2, PT=204 (APP)
        app_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(is_rtcp_packet(&app_packet));
        
        // Test regular RTP packet (PT=0)
        let mut rtp_packet = vec![0x80, 0, 0, 0]; // Version=2, PT=0 (PCMU)
        rtp_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(!is_rtcp_packet(&rtp_packet));
        
        // Test regular RTP packet with marker bit (PT=0, M=1)
        let mut rtp_packet_with_marker = vec![0x80, 0x80, 0, 0]; // Version=2, PT=0, M=1
        rtp_packet_with_marker.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(!is_rtcp_packet(&rtp_packet_with_marker));
        
        // Test regular RTP packet (PT=96, common for dynamic codecs)
        let mut rtp_dynamic_packet = vec![0x80, 96, 0, 0]; // Version=2, PT=96
        rtp_dynamic_packet.extend_from_slice(&[0; 24]); // Add some dummy data
        assert!(!is_rtcp_packet(&rtp_dynamic_packet));
    }
    
    #[tokio::test]
    async fn test_udp_transport_packet_send() {
        // Create two transport instances
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_send1".to_string()),
            use_port_allocator: false,
        };
        
        let config2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_send2".to_string()),
            use_port_allocator: false,
        };
        
        let transport1 = UdpRtpTransport::new(config1).await.unwrap();
        let transport2 = UdpRtpTransport::new(config2).await.unwrap();
        
        // Create a test packet
        let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new(header, payload);
        
        // Send from transport1 to transport2
        let addr2 = transport2.local_rtp_addr().unwrap();
        let result = transport1.send_rtp(&packet, addr2).await;
        assert!(result.is_ok());
        
        // Check if remote address was updated in transport1
        let remote_addr = transport1.remote_rtp_addr().await;
        assert_eq!(remote_addr, Some(addr2));
    }
    
    #[tokio::test]
    async fn test_udp_transport_event_subscription() {
        // Create two transport instances
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_event1".to_string()),
            use_port_allocator: false,
        };
        
        let config2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_event2".to_string()),
            use_port_allocator: false,
        };
        
        let transport1 = UdpRtpTransport::new(config1).await.unwrap();
        let transport2 = UdpRtpTransport::new(config2).await.unwrap();
        
        // Subscribe to events on transport2
        let mut events = transport2.subscribe();
        
        // Create a test packet
        let header = RtpHeader::new(96, 1000, 12345, 0xabcdef01);
        let payload = Bytes::from_static(b"test payload");
        let packet = RtpPacket::new(header, payload.clone());
        
        // Send from transport1 to transport2
        let addr2 = transport2.local_rtp_addr().unwrap();
        transport1.send_rtp(&packet, addr2).await.unwrap();
        
        // Give some time for the packet to be processed
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        
        // Try to receive the event
        match tokio::time::timeout(tokio::time::Duration::from_millis(500), events.recv()).await {
            Ok(Ok(event)) => {
                match event {
                    RtpEvent::MediaReceived { payload_type, timestamp, marker, payload: received_payload, source } => {
                        assert_eq!(payload_type, 96);
                        assert_eq!(timestamp, 12345);
                        assert_eq!(marker, false);
                        assert_eq!(&received_payload[..], &payload[..]);
                        assert_eq!(source, transport1.local_rtp_addr().unwrap());
                    },
                    _ => panic!("Unexpected event type: {:?}", event),
                }
            },
            Ok(Err(e)) => panic!("Failed to receive event: {}", e),
            Err(_) => panic!("Timeout waiting for event"),
        }
    }
    
    #[tokio::test]
    async fn test_separate_rtcp_socket_creation() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()), 
            symmetric_rtp: true,
            rtcp_mux: false,
            session_id: Some("test1".to_string()),
            use_port_allocator: false,
        };
        
        let transport = UdpRtpTransport::new(config).await.unwrap();
        
        let rtp_addr = transport.local_rtp_addr().unwrap();
        assert_ne!(rtp_addr.port(), 0, "RTP port should not be 0");
        
        // Check that a separate RTCP socket was created
        assert!(transport.rtcp_socket.is_some(), "RTCP socket should be created");
        
        // Check the actual RTCP socket address, not just the config value
        if let Some(rtcp_socket) = &transport.rtcp_socket {
            let rtcp_addr = rtcp_socket.local_addr().unwrap();
            assert_ne!(rtcp_addr.port(), 0, "RTCP port should not be 0");
            assert_ne!(rtp_addr.port(), rtcp_addr.port(), "RTP and RTCP ports should be different");
        }
    }
    
    #[tokio::test]
    async fn test_rtcp_mux_socket_creation() {
        let config = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None, // Should be ignored with mux
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test2".to_string()),
            use_port_allocator: false,
        };
        
        let transport = UdpRtpTransport::new(config).await.unwrap();
        
        let rtp_addr = transport.local_rtp_addr().unwrap();
        assert_ne!(rtp_addr.port(), 0, "RTP port should not be 0");
        
        // With RTCP mux, no separate RTCP socket should be created
        assert!(transport.rtcp_socket.is_none(), "No RTCP socket should be created with rtcp_mux");
        
        // For RTCP mux, the config does not need to have an RTCP address since it uses the RTP address
        // As long as this doesn't panic, this is sufficient
        let _rtcp_addr_option = transport.local_rtcp_addr();
    }

    #[tokio::test]
    async fn test_separate_socket_bind_conflicts() {
        // First transport
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: Some("127.0.0.1:0".parse().unwrap()),
            symmetric_rtp: true,
            rtcp_mux: false,
            session_id: Some("test_conflict1".to_string()),
            use_port_allocator: false,
        };
        
        let transport1 = UdpRtpTransport::new(config1).await.unwrap();
        let rtp_addr1 = transport1.local_rtp_addr().unwrap();
        let rtcp_addr1 = transport1.local_rtcp_addr().unwrap().expect("RTCP address should be available");
        
        // Second transport with specific ports
        let config2 = RtpTransportConfig {
            // Try to bind to the same ports as the first transport
            local_rtp_addr: SocketAddr::new(rtp_addr1.ip(), rtp_addr1.port()),
            local_rtcp_addr: Some(SocketAddr::new(rtcp_addr1.ip(), rtcp_addr1.port())),
            symmetric_rtp: true,
            rtcp_mux: false,
            session_id: Some("test_conflict2".to_string()),
            use_port_allocator: false,
        };
        
        // This should fail because the ports are already in use
        let result = UdpRtpTransport::new(config2).await;
        assert!(result.is_err());
    }
    
    #[tokio::test]
    async fn test_muxed_socket_bind_conflicts() {
        // First transport with RTCP mux
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_mux_conflict1".to_string()),
            use_port_allocator: false,
        };
        
        let transport1 = UdpRtpTransport::new(config1).await.unwrap();
        let rtp_addr1 = transport1.local_rtp_addr().unwrap();
        
        // Second transport trying to use the same port
        let config2 = RtpTransportConfig {
            local_rtp_addr: rtp_addr1,
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true,
            session_id: Some("test_mux_conflict2".to_string()),
            use_port_allocator: false,
        };
        
        // This should fail because the port is already in use
        let result = UdpRtpTransport::new(config2).await;
        assert!(result.is_err());
    }
} 