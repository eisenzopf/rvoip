//! Common API components shared between client and server implementations
//!
//! This module provides shared configuration, error handling, and security contexts
//! that are used by both client and server APIs.

// Re-exports of common types from parent modules
pub mod buffer;
pub mod config;
pub mod error;
pub mod events;
pub mod extension;
pub mod frame;
pub mod security;
pub mod security_manager;
pub mod stats;
pub mod unified_security;

// Phase 3: Advanced Features
pub mod advanced_security {
    //! Advanced security features for production environments

    pub mod error_recovery;
    pub mod key_management;

    pub use key_management::{
        KeyManager, KeyManagerStatistics, KeyRotationPolicy, KeyStore, KeySyndication,
        KeySyndicationConfig, SecurityPolicy, StreamType,
    };

    pub use error_recovery::{
        ErrorRecoveryManager, FailureStatistics, FailureType, FallbackConfig, RecoveryAction,
        RecoveryState, RecoveryStrategy,
    };
}

// Re-export common types for convenience
pub use self::buffer::{BufferStats, MediaBuffer, MediaBufferConfig};
pub use self::config::{
    BaseTransportConfig, KeyExchangeMethod, NetworkPreset, SecurityConfig, SecurityInfo,
    SecurityMode, SecurityProfile, SrtpProfile,
};
pub use self::error::{BufferError, MediaTransportError, SecurityError, StatsError};
pub use self::events::{MediaEventCallback, MediaTransportEvent};
pub use self::extension::ExtensionFormat;
pub use self::frame::{MediaFrame, MediaFrameType};
pub use self::security_manager::{
    NegotiationStrategy, SecurityCapabilities, SecurityContextManager, SecurityContextType,
};
pub use self::stats::{Direction, MediaStats, MediaStatsCollector, QualityLevel, StreamStats};
pub use self::unified_security::{
    KeyExchangeConfig, MikeyMode, SecurityContextFactory, SecurityState, UnifiedSecurityContext,
};

// Phase 3 re-exports for convenience
pub use advanced_security::key_management::{
    KeyRotationPolicy, KeySyndicationConfig, SecurityPolicy, StreamType,
};

pub use advanced_security::error_recovery::{
    FailureType, FallbackConfig, RecoveryState, RecoveryStrategy,
};
