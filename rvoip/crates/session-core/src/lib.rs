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

// Re-export important types for convenience
pub use dialog::{Dialog, DialogId, DialogState};
// Session implementation is now complete with enhanced media support
pub use session::{Session, SessionId, SessionState, SessionConfig, SessionDirection, SessionManager};
pub use session::session::SessionMediaState;
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

/// Production-ready client implementation
pub mod client {
    //! Client-specific components and factories

    use crate::{
        session::{SessionManager, SessionConfig, SessionDirection},
        events::{EventBus, SessionEvent},
        Error
    };
    use std::sync::Arc;
    use rvoip_transaction_core::TransactionManager;

    /// Client configuration
    #[derive(Debug, Clone)]
    pub struct ClientConfig {
        /// Display name for outgoing calls
        pub display_name: String,
        
        /// Default SIP URI
        pub uri: String,
        
        /// Default contact address
        pub contact: String,
        
        /// Authentication username
        pub auth_user: Option<String>,
        
        /// Authentication password
        pub auth_password: Option<String>,
        
        /// Registration interval (in seconds)
        pub registration_interval: Option<u32>,
        
        /// Session configuration
        pub session_config: SessionConfig,
    }

    impl Default for ClientConfig {
        fn default() -> Self {
            Self {
                display_name: "RVOIP Client".to_string(),
                uri: "sip:user@example.com".to_string(),
                contact: "sip:user@127.0.0.1:5060".to_string(),
                auth_user: None,
                auth_password: None,
                registration_interval: Some(3600),
                session_config: SessionConfig::default(),
            }
        }
    }

    /// Create a session manager configured for client use
    pub fn create_client_session_manager(
        transaction_manager: Arc<TransactionManager>,
        config: ClientConfig
    ) -> Arc<SessionManager> {
        let event_bus = EventBus::new(100);
        
        Arc::new(SessionManager::new_sync(
            transaction_manager,
            config.session_config,
            event_bus
        ))
    }
}

/// Production-ready server implementation
pub mod server {
    //! Server-specific components and factories

    use crate::{
        session::{SessionManager, SessionConfig, SessionDirection},
        events::{EventBus, SessionEvent},
        Error
    };
    use std::sync::Arc;
    use rvoip_transaction_core::TransactionManager;

    /// Server configuration
    #[derive(Debug, Clone)]
    pub struct ServerConfig {
        /// Server name
        pub server_name: String,
        
        /// Domain name
        pub domain: String,
        
        /// Maximum sessions allowed
        pub max_sessions: usize,
        
        /// Session timeout (in seconds)
        pub session_timeout: u32,
        
        /// Session configuration
        pub session_config: SessionConfig,
    }

    impl Default for ServerConfig {
        fn default() -> Self {
            Self {
                server_name: "RVOIP Server".to_string(),
                domain: "example.com".to_string(),
                max_sessions: 10000,
                session_timeout: 3600,
                session_config: SessionConfig::default(),
            }
        }
    }

    /// Create a session manager configured for server use
    pub fn create_server_session_manager(
        transaction_manager: Arc<TransactionManager>,
        config: ServerConfig
    ) -> Arc<SessionManager> {
        let event_bus = EventBus::new(1000);
        
        Arc::new(SessionManager::new_sync(
            transaction_manager,
            config.session_config,
            event_bus
        ))
    }
}

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
        // Convenience modules
        client, server,
        // Following SDPs are not fully implemented yet or need to be imported differently
        // SessionDescription, MediaDescription, MediaFormat, MediaDirection,
    };
}
