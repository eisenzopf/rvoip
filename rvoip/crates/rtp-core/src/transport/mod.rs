//! Network transport for RTP/RTCP
//!
//! This module provides abstractions for sending and receiving RTP/RTCP packets over the network.

use std::net::SocketAddr;
use std::any::Any;
use async_trait::async_trait;
use bytes::Bytes;
use tokio::sync::broadcast;

use crate::error::Error;
use crate::Result;
use crate::packet::RtpPacket;
use crate::packet::rtcp::RtcpPacket;
use crate::traits::RtpEvent;

/// Trait for RTP transport implementations
#[async_trait]
pub trait RtpTransport: Send + Sync {
    /// Get the local address for RTP
    fn local_rtp_addr(&self) -> Result<SocketAddr>;
    
    /// Get the local address for RTCP
    fn local_rtcp_addr(&self) -> Result<SocketAddr>;
    
    /// Send an RTP packet
    async fn send_rtp(&self, packet: &RtpPacket, dest: SocketAddr) -> Result<()>;
    
    /// Send raw RTP bytes
    async fn send_rtp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()>;
    
    /// Send an RTCP packet
    async fn send_rtcp(&self, packet: &RtcpPacket, dest: SocketAddr) -> Result<()>;
    
    /// Send raw RTCP bytes
    async fn send_rtcp_bytes(&self, bytes: &[u8], dest: SocketAddr) -> Result<()>;
    
    /// Receive a packet into the provided buffer
    /// 
    /// Returns the number of bytes read and the source address
    async fn receive_packet(&self, buffer: &mut [u8]) -> Result<(usize, SocketAddr)>;
    
    /// Subscribe to transport events
    ///
    /// This allows receiving both RTP and RTCP packets as events
    fn subscribe(&self) -> broadcast::Receiver<RtpEvent>;
    
    /// Get a reference to this object as Any
    fn as_any(&self) -> &dyn std::any::Any;
    
    /// Close the transport
    async fn close(&self) -> Result<()>;
}

/// RTP transport configuration
#[derive(Debug, Clone)]
pub struct RtpTransportConfig {
    /// Local address for RTP
    pub local_rtp_addr: SocketAddr,
    
    /// Local address for RTCP
    pub local_rtcp_addr: Option<SocketAddr>,
    
    /// Enable symmetric RTP
    pub symmetric_rtp: bool,
}

impl Default for RtpTransportConfig {
    fn default() -> Self {
        Self {
            local_rtp_addr: "0.0.0.0:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
        }
    }
}

// Re-export submodules
pub mod udp;
// pub mod tcp;

// Re-export transport implementations
pub use udp::UdpRtpTransport; 