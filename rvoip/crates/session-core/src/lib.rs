// RVOIP Session Core Library
//
// This crate provides the core functionality for SIP session management, 
// including session handling, media setup, and call flow management.
//
// # Architecture
//
// The library follows a layered architecture:
//
// - **Session Layer**: Manages SIP sessions (calls) with state transitions and media integration
// - **Dialog Layer**: Delegates to dialog-core for SIP dialog management according to RFC 3261
// - **Media Layer**: Coordinates with media-core for RTP session management
//
// For production use, the recommended usage pattern is to create a SessionManager instance,
// which will coordinate sessions and delegate dialog handling to dialog-core.

pub mod dialog;
pub mod session;
pub mod events;
pub mod errors;
pub mod media;
pub mod sdp;
pub mod metrics;
pub mod api;

// Re-export dialog types from dialog-core (the authoritative source)
pub use rvoip_dialog_core::{DialogId, DialogManager, SessionCoordinationEvent, Dialog, DialogState};

// Session implementation is now complete with enhanced media support
pub use session::{Session, SessionId, SessionState, SessionConfig, SessionDirection, SessionManager};
pub use session::session::SessionMediaState;
pub use session::session_types::{TransferId, TransferState, TransferType, TransferContext};

// ✅ BASIC PRIMITIVES (Phase 12.1): Export basic session coordination primitives
pub use session::{
    BasicSessionGroup, BasicGroupType, BasicGroupState, BasicGroupConfig,
    BasicSessionMembership, BasicGroupEvent,
    SessionDependencyTracker, SessionDependency, DependencyType, DependencyState,
};

// ✅ BASIC PRIMITIVES (Phase 12.2): Export basic resource tracking primitives
pub use session::{
    BasicResourceType, BasicResourceAllocation, BasicResourceUsage, BasicResourceLimits,
    BasicResourceRequest, BasicResourceStats,
};

// ✅ BASIC PRIMITIVES (Phase 12.3): Export basic priority classification primitives
pub use session::{
    BasicSessionPriority, BasicPriorityClass, BasicQoSLevel, BasicPriorityInfo,
    BasicPriorityConfig,
};

pub use events::{EventBus, SessionEvent};
pub use errors::{
    Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction
};
pub use metrics::MetricsCollector;

// Re-export media types
pub use media::{
    MediaManager, MediaSessionId, RelayId, MediaStatus, MediaConfig, 
    SessionMediaType as MediaType, SessionMediaDirection as MediaDirection,
    AudioCodecType, QualityMetrics, RtpStreamInfo, MediaEvent,
    // New coordination components
    SessionMediaCoordinator, MediaConfigConverter,
    // Re-exported media-core types (using proper paths)
    SampleRate, MediaSessionParams, MediaSessionHandle, MediaEngine, MediaEngineConfig
};

// Re-export API modules for convenience
pub use api::{
    client, server,
    ApiCapabilities, ApiConfig, get_api_capabilities, is_feature_supported,
    API_VERSION, SUPPORTED_SIP_VERSIONS, DEFAULT_USER_AGENT,
};

/// Re-export types from dependent crates that are used in our public API
pub mod prelude {
    // From sip-core
    pub use rvoip_sip_core::prelude::*;
    
    // From media libraries
    pub use rvoip_rtp_core::{RtpSession, RtpPacket};
    pub use rvoip_media_core::{AudioBuffer, Codec};
    
    // From our own crate - enhanced session-core with media integration
    pub use crate::{
        Session, SessionManager, SessionMediaState, // Now fully implemented with media support
        SessionId, SessionState, SessionConfig, SessionDirection,
        Error, ErrorCategory, ErrorSeverity, RecoveryAction, ErrorContext,
        SessionEvent, EventBus,
        MetricsCollector,
        // Media types
        MediaManager, MediaSessionId, RelayId, MediaStatus, MediaConfig, MediaType,
        AudioCodecType, QualityMetrics, RtpStreamInfo, MediaEvent,
        // Transfer types
        TransferId, TransferState, TransferType, TransferContext,
        // API modules
        api, client, server,
        // API types
        ApiCapabilities, ApiConfig, get_api_capabilities, is_feature_supported,
        // Dialog types from dialog-core (already imported above, no need to duplicate)
        DialogId, DialogManager, SessionCoordinationEvent, Dialog, DialogState,
    };
}
