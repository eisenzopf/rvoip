// RVOIP Session Core Library
//
// This crate provides the core functionality for SIP session management, 
// including dialog handling, media setup, and call flow management.
//
// # Architecture
//
// The library follows a layered architecture:
//
// - **Session Layer**: Manages SIP sessions (calls) with state transitions and media integration
// - **Dialog Layer**: Implements SIP dialogs according to RFC 3261
// - **Transaction Layer**: Handles SIP transactions via the transaction-core crate
// - **Transport Layer**: Abstracts the underlying transport via the sip-transport crate
//
// For production use, the recommended usage pattern is to create a SessionManager instance,
// which will manage dialog creation and transaction handling internally.

pub mod dialog;
pub mod session;
pub mod events;
pub mod errors;
pub mod media;
pub mod sdp;
pub mod helpers;
pub mod metrics;
pub mod api;

// Re-export important types for convenience
pub use dialog::{Dialog, DialogId, DialogState};
// Session implementation is now complete with enhanced media support
pub use session::{Session, SessionId, SessionState, SessionConfig, SessionDirection, SessionManager};
pub use session::session::SessionMediaState;
pub use session::session_types::{TransferId, TransferState, TransferType, TransferContext};
pub use events::{EventBus, SessionEvent};
pub use errors::{
    Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction
};
pub use metrics::MetricsCollector;

// Re-export media types
pub use media::{
    MediaManager, MediaSessionId, RelayId, MediaStatus, MediaConfig, MediaType, 
    AudioCodecType, MediaStream, QualityMetrics, RtpStreamInfo, MediaEvent
};

// Re-export helper functions for internal use
pub(crate) use helpers::{dialog_not_found_error, network_unreachable_error, transaction_creation_error, transaction_send_error};

// Re-export dialog helper functions
pub use helpers::{
    // Basic dialog operations
    create_dialog,
    create_dialog_from_invite,
    send_dialog_request,
    terminate_dialog,
    
    // Dialog management and updates
    update_dialog_media,
    refresh_dialog,
    accept_refresh_request,
    
    // Recovery
    attempt_dialog_recovery,
    
    // UPDATE method support
    send_update_request,
    accept_update_request,
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
    
    // From transaction-core
    pub use rvoip_transaction_core::{
        TransactionManager, 
        TransactionEvent, 
        TransactionState, 
        TransactionKey,
        TransactionKind
    };
    
    // From media libraries
    pub use rvoip_rtp_core::{RtpSession, RtpPacket};
    pub use rvoip_media_core::{AudioBuffer, Codec};
    
    // From our own crate - enhanced session-core with media integration
    pub use crate::{
        Dialog, DialogState, DialogId,
        Session, SessionManager, SessionMediaState, // Now fully implemented with media support
        SessionId, SessionState, SessionConfig, SessionDirection,
        Error, ErrorCategory, ErrorSeverity, RecoveryAction, ErrorContext,
        SessionEvent, EventBus,
        MetricsCollector,
        // Media types
        MediaManager, MediaSessionId, RelayId, MediaStatus, MediaConfig, MediaType,
        AudioCodecType, MediaStream, QualityMetrics, RtpStreamInfo, MediaEvent,
        // Transfer types
        TransferId, TransferState, TransferType, TransferContext,
        // API modules
        api, client, server,
        // API types
        ApiCapabilities, ApiConfig, get_api_capabilities, is_feature_supported,
        // Following SDPs are not fully implemented yet or need to be imported differently
        // SessionDescription, MediaDescription, MediaFormat, MediaDirection,
    };
}
