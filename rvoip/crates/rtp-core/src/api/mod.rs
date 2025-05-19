//! RTP Core API
//!
//! This module provides a high-level API for integrating with the RTP Core library.
//! It's specifically designed to provide a clean interface for media-core integration
//! while hiding implementation details.

// Original modules (will be gradually phased out)
pub mod transport;
pub mod security;
pub mod buffer;
pub mod stats;

// New structure with client/server separation
pub mod common;
pub mod client;
pub mod server;

// Re-export key types from the old structure for backward compatibility
pub use self::transport::{MediaTransportSession, MediaTransportConfig};
pub use self::security::{SecurityConfig, SecureMediaContext};
pub use self::buffer::{MediaBuffer, MediaBufferConfig};
pub use self::stats::{MediaStatsCollector, QualityLevel};

// Re-export key types from the new structure
pub use self::common::frame::{MediaFrame, MediaFrameType};
pub use self::common::error::{MediaTransportError, SecurityError, BufferError, StatsError};
pub use self::common::events::{MediaTransportEvent, MediaEventCallback};
pub use self::common::config::{SecurityMode, SrtpProfile, SecurityInfo, NetworkPreset, BaseTransportConfig};
pub use self::common::buffer::{MediaBuffer as CommonMediaBuffer, MediaBufferConfig as CommonMediaBufferConfig, BufferStats};
pub use self::common::stats::{MediaStats, StreamStats, QualityLevel as CommonQualityLevel, Direction, MediaStatsCollector as CommonMediaStatsCollector};

// Client re-exports
pub use self::client::transport::{MediaTransportClient, ClientFactory};
pub use self::client::security::{ClientSecurityContext, ClientSecurityConfig, ClientSecurityFactory};
pub use self::client::config::{ClientConfig, ClientConfigBuilder};

// Server re-exports
pub use self::server::transport::{MediaTransportServer, ClientInfo, ServerFactory};
pub use self::server::security::{ServerSecurityContext, ServerSecurityConfig, ClientSecurityContext as ServerClientSecurityContext, ServerSecurityFactory};
pub use self::server::config::{ServerConfig, ServerConfigBuilder}; 