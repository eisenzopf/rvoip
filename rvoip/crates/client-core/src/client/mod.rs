//! Client module containing the main client implementation
//! 
//! This module provides the high-level client API for SIP operations

pub mod config;
pub mod manager;
pub mod types;
pub mod calls;
pub mod events;
pub mod media;
pub mod controls;
pub mod tests;
pub mod builder;

// Re-export the main ClientManager
pub use manager::ClientManager;

// Re-export all types from types.rs
pub use types::{
    ClientStats,
    CallMediaInfo,
    AudioCodecInfo,
    AudioQualityMetrics,
    MediaCapabilities,
    CallCapabilities,
    MediaSessionInfo,
    NegotiatedMediaParams,
    EnhancedMediaCapabilities,
    AudioDirection,
};

// Re-export event types from events.rs
pub use events::{
    ClientCallHandler,
};

// Re-export builder module
pub use builder::ClientBuilder;

// Type alias for convenient use
pub type Client = ClientManager;

// Note: Individual operation methods are implemented as impl blocks in separate files
// and will be automatically available on ClientManager instances