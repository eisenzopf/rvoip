//! Media Transport implementation for RTP sessions
//!
//! This module provides an implementation of the MediaTransport trait
//! that adapts an RtpSession for use with media-core.

use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;
use async_trait::async_trait;
use bytes::Bytes;

use crate::error::Error;
use crate::Result;
use crate::session::RtpSession;
use crate::traits::MediaTransport;
use crate::transport::UdpRtpTransport;

/// Media Transport implementation that adapts an RtpSession
pub struct RtpMediaTransport {
    /// The underlying RTP session
    session: Arc<Mutex<RtpSession>>,
}

impl RtpMediaTransport {
    /// Create a new RTP media transport
    pub fn new(session: RtpSession) -> Self {
        Self {
            session: Arc::new(Mutex::new(session)),
        }
    }
    
    /// Get a reference to the underlying RTP session
    pub fn session(&self) -> &Arc<Mutex<RtpSession>> {
        &self.session
    }
    
    /// Set the remote address directly
    pub async fn set_remote_addr(&self, addr: SocketAddr) -> Result<()> {
        let mut session = self.session.lock().await;
        
        // Set the address on the session and also directly on the transport
        session.set_remote_addr(addr).await;
        
        // Get the transport and set the address directly if it's a UDP transport
        let transport = session.transport();
        if let Some(udp) = transport.as_any().downcast_ref::<UdpRtpTransport>() {
            udp.set_remote_rtp_addr(addr).await;
        }
        
        Ok(())
    }
}

#[async_trait]
impl MediaTransport for RtpMediaTransport {
    async fn local_addr(&self) -> Result<SocketAddr> {
        let session = self.session.lock().await;
        session.local_addr()
    }
    
    async fn send_media(
        &self,
        payload_type: u8,
        timestamp: u32,
        payload: Bytes,
        marker: bool,
    ) -> Result<()> {
        let mut session = self.session.lock().await;
        
        // Update payload type if it's different
        if session.get_payload_type() != payload_type {
            session.set_payload_type(payload_type);
        }
        
        // Send the packet
        session.send_packet(timestamp, payload, marker).await
    }
    
    async fn close(&self) -> Result<()> {
        let mut session = self.session.lock().await;
        session.close().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::RtpSessionConfig;
    
    #[tokio::test]
    async fn test_rtp_media_transport() {
        // Create a test RTP session
        let config = RtpSessionConfig::default();
        let session = RtpSession::new(config).await.unwrap();
        
        // Create the media transport
        let transport = RtpMediaTransport::new(session);
        
        // Test local address
        let addr = transport.local_addr().await.unwrap();
        assert_ne!(addr.port(), 0);
        
        // Test sending media
        let payload = Bytes::from_static(b"test payload");
        let result = transport.send_media(0, 12345, payload, true).await;
        assert!(result.is_ok());
        
        // Close the transport
        let result = transport.close().await;
        assert!(result.is_ok());
    }
} 