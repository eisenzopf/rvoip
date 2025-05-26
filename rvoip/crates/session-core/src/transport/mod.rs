//! Transport Integration Module
//!
//! This module provides transport integration for session-core,
//! bridging to the sip-transport layer with clean abstractions.

pub mod integration;
pub mod factory;

// Re-export main types
pub use integration::{TransportIntegration, SessionTransportEvent};
pub use factory::TransportFactory; 