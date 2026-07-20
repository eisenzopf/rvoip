//! Network transport for RTP/RTCP
//!
//! This module provides abstractions for sending and receiving RTP/RTCP packets over the network.

use async_trait::async_trait;
use std::net::SocketAddr;
use tokio::sync::broadcast;

use crate::packet::rtcp::RtcpPacket;
use crate::packet::RtpPacket;
use crate::traits::RtpEvent;
use crate::Result;

/// Default transport event broadcast capacity.
pub const RTP_TRANSPORT_EVENT_CHANNEL_CAPACITY: usize = 32;

/// RTP transport buffer and queue sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RtpTransportBufferConfig {
    /// Broadcast ring capacity for RTP/RTCP transport events.
    pub event_channel_capacity: usize,
    /// UDP RTP receive buffer size in bytes.
    pub recv_buffer_size: usize,
    /// UDP RTCP receive buffer size in bytes when RTCP uses a separate socket.
    pub rtcp_recv_buffer_size: usize,
}

impl Default for RtpTransportBufferConfig {
    fn default() -> Self {
        Self {
            event_channel_capacity: RTP_TRANSPORT_EVENT_CHANNEL_CAPACITY,
            recv_buffer_size: crate::DEFAULT_MAX_PACKET_SIZE,
            rtcp_recv_buffer_size: crate::DEFAULT_MAX_PACKET_SIZE,
        }
    }
}

/// Trait for RTP transport implementations
#[async_trait]
pub trait RtpTransport: Send + Sync {
    /// Get the local address for RTP
    fn local_rtp_addr(&self) -> Result<SocketAddr>;

    /// Get the local RTCP address (if available)
    fn local_rtcp_addr(&self) -> Result<Option<SocketAddr>>;

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

    /// Enable RTCP multiplexing (RFC 5761)
    ///
    /// When enabled, RTCP packets will be sent and received on the same port as RTP packets.
    /// This is recommended for WebRTC and modern VoIP applications.
    pub rtcp_mux: bool,

    /// Session ID for port allocation tracking (optional)
    pub session_id: Option<String>,

    /// Use the global port allocator
    pub use_port_allocator: bool,

    /// Transport buffer and event queue sizing.
    pub buffer_config: RtpTransportBufferConfig,
}

impl Default for RtpTransportConfig {
    fn default() -> Self {
        Self {
            local_rtp_addr: "0.0.0.0:0".parse().unwrap(),
            local_rtcp_addr: None,
            symmetric_rtp: true,
            rtcp_mux: true, // Enable by default as it's the modern approach
            session_id: None,
            // Don't use port allocator by default - let the caller decide
            use_port_allocator: false,
            buffer_config: RtpTransportBufferConfig::default(),
        }
    }
}

/// Port allocation strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortPairingStrategy {
    /// Use adjacent port numbers (even for RTP, odd for RTCP)
    Adjacent,
    /// Use the same port for both RTP and RTCP (requires RTCP-MUX)
    Muxed,
}

// Re-export submodules
mod allocator;
pub mod security_transport;
mod symmetric;
mod tcp;
mod udp;
mod validation;

// Re-export transport implementations
pub use allocator::{
    AllocationStrategy, GlobalPortAllocator, PairingStrategy, PortAllocator, PortAllocatorConfig,
    PortAllocatorDiagnostics, DEFAULT_RTP_PORT_RANGE_END, DEFAULT_RTP_PORT_RANGE_START, MIN_PORT,
};
pub use security_transport::SecurityRtpTransport;
pub use symmetric::{SymmetricRtpDiagnostics, SymmetricRtpPolicy};
pub use tcp::TcpRtpTransport;
pub use udp::{set_diagnostics as set_udp_diagnostics, UdpRtpTransport};
pub use validation::{PlatformSocketStrategy, PlatformType, RtpSocketValidator};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_transport_buffer_config_preserves_buffer_sizes() {
        let config = RtpTransportConfig::default();

        assert_eq!(
            config.buffer_config.event_channel_capacity,
            RTP_TRANSPORT_EVENT_CHANNEL_CAPACITY
        );
        assert_eq!(
            config.buffer_config.recv_buffer_size,
            crate::DEFAULT_MAX_PACKET_SIZE
        );
        assert_eq!(
            config.buffer_config.rtcp_recv_buffer_size,
            crate::DEFAULT_MAX_PACKET_SIZE
        );
    }
}
