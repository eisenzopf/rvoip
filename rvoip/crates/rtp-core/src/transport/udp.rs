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

/// UDP transport for RTP/RTCP
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
    /// Create a new UDP transport
    pub async fn new(config: RtpTransportConfig) -> Result<Self> {
        // Create RTP socket
        let rtp_socket = UdpSocket::bind(config.local_rtp_addr).await
            .map_err(|e| Error::Transport(format!("Failed to bind RTP socket: {}", e)))?;
            
        // Create RTCP socket if configured
        let rtcp_socket = if let Some(rtcp_addr) = config.local_rtcp_addr {
            let socket = UdpSocket::bind(rtcp_addr).await
                .map_err(|e| Error::Transport(format!("Failed to bind RTCP socket: {}", e)))?;
            Some(Arc::new(socket))
        } else {
            None
        };
        
        // Create broadcaster
        let (event_tx, _) = broadcast::channel(100);
        
        let transport = Self {
            rtp_socket: Arc::new(rtp_socket),
            rtcp_socket,
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
                        
                        // Check if it's RTCP (common for RTP and RTCP to use the same socket)
                        let version = (buffer[0] >> 6) & 0x03;
                        let payload_type = buffer[1];  // Use the full byte, don't mask
                        
                        if version == 2 && payload_type >= 200 && payload_type <= 204 {
                            // This is an RTCP packet
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
                                    
                                    // Create RTP event
                                    let event = RtpEvent::MediaReceived {
                                        payload_type: packet.header.payload_type,
                                        timestamp: packet.header.timestamp,
                                        marker: packet.header.marker,
                                        payload: packet.payload.clone(),
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
                                    // This could be an RTCP packet
                                    if buffer[1] >= 200 && buffer[1] <= 204 {
                                        debug!("Received RTCP packet type {}", buffer[1]);
                                        // TODO: Implement RTCP packet handling
                                    } else {
                                        warn!("Failed to parse RTP packet: {}", e);
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
    async fn stop_receiver(&self) -> Result<()> {
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
}

#[async_trait]
impl RtpTransport for UdpRtpTransport {
    fn local_rtp_addr(&self) -> Result<SocketAddr> {
        self.rtp_socket.local_addr()
            .map_err(|e| Error::Transport(format!("Failed to get local RTP address: {}", e)))
    }
    
    fn local_rtcp_addr(&self) -> Result<SocketAddr> {
        if let Some(rtcp_socket) = &self.rtcp_socket {
            rtcp_socket.local_addr()
                .map_err(|e| Error::Transport(format!("Failed to get local RTCP address: {}", e)))
        } else {
            // If no separate RTCP socket, use the RTP socket
            self.local_rtp_addr()
        }
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
        // This is a placeholder - we'd need to implement proper serialization first
        // Let's return an error for now
        Err(Error::Transport("RTCP serialization not implemented yet".to_string()))
    }
    
    async fn send_rtcp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()> {
        if self.config.symmetric_rtp {
            // Update remote RTCP address if using symmetric RTP
            let mut remote_addr = self.remote_rtcp_addr.lock().await;
            *remote_addr = Some(dest);
        }
        
        // Use the RTCP socket if available, otherwise use the RTP socket
        let socket = if let Some(rtcp_socket) = &self.rtcp_socket {
            rtcp_socket
        } else {
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
        
        // UDP sockets don't need explicit closing
        Ok(())
    }
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
        };
        
        let transport = UdpRtpTransport::new(config).await;
        assert!(transport.is_ok());
        
        let transport = transport.unwrap();
        let rtp_addr = transport.local_rtp_addr().unwrap();
        let rtcp_addr = transport.local_rtcp_addr().unwrap();
        
        assert_ne!(rtp_addr.port(), 0);
        assert_ne!(rtcp_addr.port(), 0);
        assert_ne!(rtp_addr.port(), rtcp_addr.port());
    }
    
    #[tokio::test]
    async fn test_udp_transport_packet_send() {
        // Create two transport instances
        let config1 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
        };
        
        let config2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
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
        };
        
        let config2 = RtpTransportConfig {
            local_rtp_addr: "127.0.0.1:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
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
} 