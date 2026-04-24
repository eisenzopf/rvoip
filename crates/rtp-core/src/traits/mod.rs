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

    /// RFC 4733 telephone-event received (DTMF / fax / modem tone).
    ///
    /// Emitted by the UDP receive loop whenever a packet arrives with
    /// the negotiated telephone-event payload type (PT 101 by default).
    /// Payload is pre-decoded into typed fields so the media layer
    /// doesn't re-parse; dedup across redundant retransmits (RFC 4733
    /// §2.5.1.3) is left to the consumer — the recommended shape is to
    /// key on `(ssrc, timestamp)` since retransmits of the same tone
    /// share both while distinct tones get distinct timestamps.
    DtmfEvent {
        /// Event code (0-15 for DTMF: 0-9 / `*` / `#` / A-D).
        event: u8,
        /// End-of-event bit (RFC 4733 §2.3 `E`). The last three frames
        /// of a tone all have this set.
        end_of_event: bool,
        /// Volume in -dBm0 (0 = loudest, 63 = quietest). RFC 4733 6-bit.
        volume: u8,
        /// Duration in RTP timestamp units since event start.
        duration: u16,
        /// RTP packet timestamp. Distinct per tone; shared across the
        /// three retransmissions of a single tone per RFC 4733 §2.5.1.3.
        /// This is the stable dedup key on the consumer side.
        timestamp: u32,
        /// Source address.
        source: SocketAddr,
        /// SSRC — lets the consumer disambiguate simultaneous DTMF
        /// streams on the same socket (rare, but possible for b2bua).
        ssrc: u32,
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