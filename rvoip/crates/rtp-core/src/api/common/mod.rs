//! Common API types and utilities
//!
//! This module contains shared types and utilities used by both client and server APIs.

// Re-exports of common types from parent modules
pub mod config;
pub mod error;
pub mod events;
pub mod frame;
pub mod buffer;
pub mod stats;
pub mod security;
pub mod extension;
pub mod unified_security;
pub mod security_manager;

// Re-export common types for convenience
pub use self::frame::{MediaFrame, MediaFrameType};
pub use self::error::{MediaTransportError, SecurityError, BufferError, StatsError};
pub use self::events::{MediaTransportEvent, MediaEventCallback};
pub use self::config::{SecurityMode, KeyExchangeMethod, SrtpProfile, SecurityInfo, BaseTransportConfig, NetworkPreset, SecurityConfig, SecurityProfile};
pub use self::buffer::{MediaBuffer, MediaBufferConfig, BufferStats};
pub use self::stats::{MediaStats, StreamStats, QualityLevel, Direction, MediaStatsCollector};
pub use self::extension::ExtensionFormat;
pub use self::unified_security::{UnifiedSecurityContext, SecurityState, SecurityContextFactory, KeyExchangeConfig, MikeyMode};
pub use self::security_manager::{SecurityContextManager, SecurityContextType, NegotiationStrategy, SecurityCapabilities}; 