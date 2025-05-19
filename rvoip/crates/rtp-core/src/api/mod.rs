//! RTP Core API
//!
//! This module provides a high-level API for integrating with the RTP Core library.
//! It's specifically designed to provide a clean interface for media-core integration
//! while hiding implementation details.

pub mod transport;
pub mod security;
pub mod buffer;
pub mod stats;

// Re-export key types for convenience
pub use self::transport::{MediaTransportSession, MediaTransportConfig};
pub use self::security::{SecurityConfig, SecureMediaContext};
pub use self::buffer::{MediaBuffer, MediaBufferConfig};
pub use self::stats::{MediaStatsCollector, QualityLevel}; 