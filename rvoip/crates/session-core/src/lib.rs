//! Session management for the RVOIP stack.
//!
//! This crate provides the core functionality for managing SIP call sessions,
//! including dialog state management, call state transitions, and integration
//! between SIP signaling and media (RTP) handling.

// Core modules
pub mod dialog;
pub mod dialog_state;
pub mod session;
pub mod errors;
pub mod media;
pub mod events;
pub mod sdp;

// Public re-exports of main types
pub use dialog::{Dialog, DialogId, DialogManager};
pub use dialog_state::DialogState;
pub use session::{Session, SessionManager, SessionId, SessionState, SessionConfig};
pub use errors::Error;
pub use events::{SessionEvent, EventHandler, EventBus};
pub use sdp::{SessionDescription, MediaDescription, MediaFormat, MediaDirection, SdpError};

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
    
    // From our own crate
    pub use crate::{
        Dialog, DialogState, DialogId, DialogManager,
        Session, SessionManager, SessionId, SessionState, SessionConfig,
        Error, SessionEvent, EventHandler, EventBus,
        SessionDescription, MediaDescription, MediaFormat, MediaDirection,
    };
} 