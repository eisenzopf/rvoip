//! Common API components shared between client and server implementations
//!
//! This module provides shared configuration, error handling, and security contexts
//! that are used by both client and server APIs.

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

// Phase 3: Advanced Features
pub mod advanced_security {
    //! Advanced security features for production environments
    
    pub mod key_management;
    pub mod error_recovery;
    
    pub use key_management::{
        KeyManager, KeyManagerStatistics,
        KeyRotationPolicy, KeyStore, KeySyndication, KeySyndicationConfig,
        SecurityPolicy, StreamType,
    };
    
    pub use error_recovery::{
        ErrorRecoveryManager, FailureStatistics,
        RecoveryStrategy, FailureType, FallbackConfig,
        RecoveryState, RecoveryAction,
    };
}

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

// Phase 3 re-exports for convenience
pub use advanced_security::key_management::{
    KeyRotationPolicy, StreamType, KeySyndicationConfig, SecurityPolicy,
};

pub use advanced_security::error_recovery::{
    RecoveryStrategy, FallbackConfig, FailureType, RecoveryState,
}; 