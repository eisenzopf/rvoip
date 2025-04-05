//! Session management for the RVOIP stack.
//!
//! This crate provides the core functionality for managing SIP call sessions,
//! including dialog state management, call state transitions, and integration
//! between SIP signaling and media (RTP) handling.

pub mod dialog;
pub mod session;
pub mod errors;
pub mod media;
pub mod events;
pub mod sdp;

pub use dialog::{Dialog, DialogState, DialogId};
pub use session::{Session, SessionManager, SessionId, SessionState, SessionConfig};
pub use errors::Error;
pub use events::{SessionEvent, EventHandler, EventBus};
pub use sdp::{SessionDescription, MediaDescription, MediaFormat, MediaDirection, SdpError};

/// Re-export types from dependent crates that are used in our public API
pub mod prelude {
    pub use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri, Header, HeaderName};
    pub use rvoip_transaction_core::TransactionManager;
    pub use rvoip_rtp_core::{RtpSession, RtpPacket};
    pub use rvoip_media_core::{AudioBuffer, Codec};
    
    pub use crate::{
        Dialog, DialogState, DialogId,
        Session, SessionManager, SessionId, SessionState, SessionConfig,
        Error, SessionEvent, EventHandler, EventBus,
        SessionDescription, MediaDescription, MediaFormat, MediaDirection,
    };
} 