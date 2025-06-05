//! RVOIP Session Core API
//!
//! This module provides **session coordination infrastructure** for building SIP applications.
//! session-core provides the essential SessionManager and coordination primitives that 
//! call-engine and client-core use for their business logic.
//!
//! ## Architectural Principle
//!
//! **✅ session-core provides**: SessionManager, Session primitives, Bridge infrastructure
//! **✅ call-engine uses**: SessionManager for business logic (call routing, policies)  
//! **✅ client-core uses**: SessionManager for client-specific behavior patterns
//!
//! # SessionManager Infrastructure API
//!
//! The core API provides SessionManager that call-engine and client-core orchestrate:
//!
//! ```rust
//! use rvoip_session_core::api::*;
//!
//! // Create SessionManager infrastructure
//! let session_manager = create_session_manager(dialog_api, media_manager, config).await?;
//! 
//! // call-engine orchestrates business logic using SessionManager
//! let bridge_id = session_manager.create_bridge(BridgeConfig::default()).await?;
//! ```
//!
//! # Infrastructure Components Available
//!
//! Session-core provides these coordination components:
//!
//! - **SessionManager**: Central session coordination infrastructure
//! - **Session Primitives**: SessionId, Session, SessionState, etc.
//! - **Bridge Infrastructure**: Multi-session bridging mechanics  
//! - **Basic Groups**: Session grouping data structures
//! - **Basic Resources**: Resource tracking primitives
//! - **Basic Priorities**: Priority classification primitives
//! - **Basic Events**: Simple pub/sub event communication

// Session coordination infrastructure
pub mod factory;
pub mod handler;

// NEW: Simple developer-focused API module
pub mod simple;

// ✅ **INFRASTRUCTURE EXPORTS**: Core session coordination infrastructure
pub use crate::{
    // SessionManager - the central infrastructure component
    SessionManager, SessionConfig,
    
    // Session primitives
    SessionId, Session, SessionState, SessionDirection,
    
    // Bridge infrastructure for call-engine orchestration
    session::bridge::{
        BridgeId, BridgeState, BridgeConfig, BridgeInfo, BridgeEvent, BridgeEventType, 
        BridgeStats, BridgeError, SessionBridge,
    },
    
    // Event infrastructure
    EventBus, SessionEvent,
    
    // Media coordination infrastructure
    MediaManager, MediaSessionId, MediaConfig,
};

// ✅ **BASIC PRIMITIVES**: All basic coordination primitives for call-engine composition
pub use crate::{
    // Basic session grouping primitives
    BasicSessionGroup, BasicGroupType, BasicGroupState, BasicGroupConfig,
    BasicSessionMembership, BasicGroupEvent,
    
    // Basic resource tracking primitives
    BasicResourceType, BasicResourceAllocation, BasicResourceUsage, BasicResourceLimits,
    BasicResourceRequest, BasicResourceStats,
    
    // Basic priority classification primitives
    BasicSessionPriority, BasicPriorityClass, BasicQoSLevel, BasicPriorityInfo,
    BasicPriorityConfig,
    
    // Basic event communication primitives
    BasicSessionEvent, BasicEventBus, BasicEventBusConfig, BasicEventFilter,
    FilteredEventSubscriber,
};

// ✅ **FACTORY EXPORTS**: Clean SessionManager creation API
pub use factory::{
    // NEW CLEAN API
    SessionManagerConfig, SessionMode,
    
    // Legacy APIs for backward compatibility (deprecated)
    SessionInfrastructure, SessionInfrastructureConfig,
    create_session_manager_for_sip_server, create_session_manager_for_sip_endpoint,
};

/// API version information
pub const API_VERSION: &str = "1.0.0";

/// Supported SIP protocol versions
pub const SUPPORTED_SIP_VERSIONS: &[&str] = &["2.0"];

/// Default user agent string for the API
pub const DEFAULT_USER_AGENT: &str = "RVOIP-SessionCore/1.0";

/// Session-core infrastructure capabilities
#[derive(Debug, Clone)]
pub struct SessionCoreCapabilities {
    /// Supports SessionManager infrastructure
    pub session_manager: bool,
    
    /// Supports session coordination
    pub session_coordination: bool,
    
    /// Supports media coordination infrastructure
    pub media_coordination: bool,
    
    /// Supports session bridging infrastructure
    pub session_bridging: bool,
    
    /// Supports basic grouping primitives
    pub basic_groups: bool,
    
    /// Supports basic resource tracking
    pub basic_resources: bool,
    
    /// Supports basic priority classification
    pub basic_priorities: bool,
    
    /// Supports basic event communication
    pub basic_events: bool,
    
    /// Maximum concurrent sessions (infrastructure limit)
    pub max_sessions: usize,
}

impl Default for SessionCoreCapabilities {
    fn default() -> Self {
        Self {
            session_manager: true,
            session_coordination: true,
            media_coordination: true,
            session_bridging: true,
            basic_groups: true,
            basic_resources: true,
            basic_priorities: true,
            basic_events: true,
            max_sessions: 10000,
        }
    }
}

/// Get session-core infrastructure capabilities
pub fn get_session_core_capabilities() -> SessionCoreCapabilities {
    SessionCoreCapabilities::default()
}

/// Check if a session-core feature is supported
pub fn is_session_feature_supported(feature: &str) -> bool {
    let capabilities = get_session_core_capabilities();
    
    match feature {
        "session_manager" => capabilities.session_manager,
        "session_coordination" => capabilities.session_coordination,
        "media_coordination" => capabilities.media_coordination,
        "session_bridging" => capabilities.session_bridging,
        "basic_groups" => capabilities.basic_groups,
        "basic_resources" => capabilities.basic_resources,
        "basic_priorities" => capabilities.basic_priorities,
        "basic_events" => capabilities.basic_events,
        _ => false,
    }
}

/// Session infrastructure configuration
#[derive(Debug, Clone)]
pub struct SessionApiConfig {
    /// API version to use
    pub version: String,
    
    /// Enable debug logging
    pub debug_logging: bool,
    
    /// Enable metrics collection
    pub enable_metrics: bool,
    
    /// Event buffer size
    pub event_buffer_size: usize,
    
    /// Maximum sessions
    pub max_sessions: usize,
}

impl Default for SessionApiConfig {
    fn default() -> Self {
        Self {
            version: API_VERSION.to_string(),
            debug_logging: false,
            enable_metrics: true,
            event_buffer_size: 1000,
            max_sessions: 10000,
        }
    }
} 