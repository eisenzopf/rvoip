//! TCP transport for RTP/RTCP
//!
//! This module provides a TCP transport for RTP and RTCP.
//! Currently, this is a placeholder for future implementation.

use std::any::Any;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::error::Error;
use crate::Result;
use crate::traits::RtpEvent;
use super::{RtpTransport, RtpTransportConfig};

/// TCP transport for RTP (placeholder)
pub struct TcpRtpTransport {
    /// Configuration
    config: RtpTransportConfig,
    
    /// Event channel
    event_tx: broadcast::Sender<RtpEvent>,
}

impl TcpRtpTransport {
    /// Create a new TCP transport (placeholder)
    pub async fn new(_config: RtpTransportConfig) -> Result<Self> {
        let (event_tx, _) = broadcast::channel(100);
        
        Ok(Self {
            config: _config,
            event_tx,
        })
    }
}

#[async_trait::async_trait]
impl RtpTransport for TcpRtpTransport {
    /// Get the local RTP address
    fn local_rtp_addr(&self) -> Result<SocketAddr> {
        Err(Error::NotImplemented("TCP transport not yet implemented".to_string()))
    }
    
    /// Get the local RTCP address
    fn local_rtcp_addr(&self) -> Result<Option<SocketAddr>> {
        Err(Error::NotImplemented("TCP transport not yet implemented".to_string()))
    }
    
    /// Send an RTP packet
    async fn send_rtp(&self, _packet: &crate::packet::RtpPacket, _dest: SocketAddr) -> Result<()> {
        Err(Error::NotImplemented("TCP transport not yet implemented".to_string()))
    }
    
    /// Send RTP bytes
    async fn send_rtp_bytes(&self, _bytes: &[u8], _dest: SocketAddr) -> Result<()> {
        Err(Error::NotImplemented("TCP transport not yet implemented".to_string()))
    }
    
    /// Send RTCP data
    async fn send_rtcp_bytes(&self, _data: &[u8], _dest: SocketAddr) -> Result<()> {
        Err(Error::NotImplemented("TCP transport not yet implemented".to_string()))
    }
    
    /// Send an RTCP packet
    async fn send_rtcp(&self, _packet: &crate::packet::rtcp::RtcpPacket, _dest: SocketAddr) -> Result<()> {
        Err(Error::NotImplemented("TCP transport not yet implemented".to_string()))
    }
    
    /// Receive a packet
    async fn receive_packet(&self, _buffer: &mut [u8]) -> Result<(usize, SocketAddr)> {
        Err(Error::NotImplemented("TCP transport not yet implemented".to_string()))
    }
    
    /// Close the transport
    async fn close(&self) -> Result<()> {
        Ok(())
    }
    
    /// Subscribe to transport events
    fn subscribe(&self) -> broadcast::Receiver<RtpEvent> {
        self.event_tx.subscribe()
    }
    
    /// Get the transport as Any (for downcasting)
    fn as_any(&self) -> &dyn Any {
        self
    }
} 