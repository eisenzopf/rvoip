//! Integration Bridges
//!
//! This module provides integration bridges between media-core and other crates
//! in the RVOIP system, specifically session-core and rtp-core.

pub mod rtp_bridge;
pub mod session_bridge;
pub mod events;

// Re-export main types
pub use rtp_bridge::{RtpBridge, RtpBridgeConfig, RtpEvent, RtpEventCallback};
pub use session_bridge::{SessionBridge, SessionBridgeConfig};
pub use events::{IntegrationEvent, IntegrationEventType}; 