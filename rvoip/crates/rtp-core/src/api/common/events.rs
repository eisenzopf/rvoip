//! Event definitions
//!
//! This module defines common event types used by both client and server APIs.

use std::net::SocketAddr;
use crate::api::common::stats::QualityLevel;

/// Media transport event types for notifications
#[derive(Debug, Clone)]
pub enum MediaTransportEvent {
    /// Transport connected successfully
    Connected,
    /// Transport disconnected
    Disconnected,
    /// Network quality changed
    QualityChanged {
        /// The new quality level
        quality: QualityLevel,
    },
    /// New bandwidth estimate available
    BandwidthEstimate {
        /// Estimated available bandwidth in bits per second
        bps: u32,
    },
    /// Remote address changed (e.g., ICE candidate switch)
    RemoteAddressChanged {
        /// The new remote address
        address: SocketAddr,
    },
    /// Media frame received (only used when not using receive_frame directly)
    FrameReceived(crate::api::common::frame::MediaFrame),
    /// Error occurred
    Error(crate::api::common::error::MediaTransportError),
    /// Transport state changed
    StateChanged(String),
    /// Stream ended
    StreamEnded {
        /// Stream SSRC
        ssrc: u32,
        /// Reason for ending
        reason: String,
    },
    /// New stream detected
    NewStream {
        /// Stream SSRC
        ssrc: u32,
    },
}

/// Callback for receiving transport events
pub type MediaEventCallback = Box<dyn Fn(MediaTransportEvent) + Send + Sync>; 