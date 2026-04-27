//! Integration Bridges
//!
//! This module provides integration bridges between media-core and other crates
//! in the RVOIP system, specifically session-core and rtp-core.

pub mod events;
pub mod rtp_bridge;

// Re-export main types
pub use events::{IntegrationEvent, IntegrationEventType};
pub use rtp_bridge::{RtpBridge, RtpBridgeConfig, RtpEvent, RtpEventCallback};
