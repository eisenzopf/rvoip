//! Public traits for integration with other crates
//!
//! This module provides trait definitions that allow rtp-core to be integrated
//! with other components of the rVOIP stack, such as media-core.

use std::net::SocketAddr;
use async_trait::async_trait;
use bytes::Bytes;

use crate::error::Error;
use crate::Result;

// Export the media_transport module
pub mod media_transport;
pub use media_transport::RtpMediaTransport;

/// Media Transport trait for transporting media data
///
/// This trait is used by media-core to send media samples over RTP.
#[async_trait]
pub trait MediaTransport: Send + Sync {
    /// Get the local address for media transport
    async fn local_addr(&self) -> Result<SocketAddr>;
    
    /// Send media data with the given payload type, timestamp, and marker bit
    async fn send_media(
        &self,
        payload_type: u8,
        timestamp: u32,
        payload: Bytes,
        marker: bool,
    ) -> Result<()>;
    
    /// Close the transport
    async fn close(&self) -> Result<()>;
}

/// RTP Events that can be received from the transport
#[derive(Debug, Clone)]
pub enum RtpEvent {
    /// Media data received
    MediaReceived {
        /// Payload type
        payload_type: u8,
        
        /// RTP timestamp
        timestamp: u32,
        
        /// Marker bit
        marker: bool,
        
        /// Payload data
        payload: Bytes,
        
        /// Source address
        source: SocketAddr,
        
        /// SSRC (Synchronization Source)
        ssrc: u32,
    },
    
    /// RTCP packet received (raw bytes for now)
    RtcpReceived {
        /// RTCP data
        data: Bytes,
        
        /// Source address
        source: SocketAddr,
    },
    
    /// Transport error occurred
    Error(Error),
}

/// RTP Event Consumer trait
///
/// This trait is implemented by components that want to receive RTP events.
#[async_trait]
pub trait RtpEventConsumer: Send + Sync {
    /// Process an RTP event
    async fn process_event(&self, event: RtpEvent) -> Result<()>;
} 