//! UDP transport for RTP/RTCP
//!
//! This module provides a UDP-based implementation of the RTP transport.

use std::net::SocketAddr;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use crate::error::Error;
use crate::Result;
use crate::packet::RtpPacket;
use crate::packet::rtcp::RtcpPacket;
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
        
        Ok(Self {
            rtp_socket: Arc::new(rtp_socket),
            rtcp_socket,
            config,
            remote_rtp_addr: Arc::new(Mutex::new(None)),
            remote_rtcp_addr: Arc::new(Mutex::new(None)),
        })
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
    
    async fn close(&self) -> Result<()> {
        // UDP sockets don't need explicit closing
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use crate::RtpPacket;
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
} 