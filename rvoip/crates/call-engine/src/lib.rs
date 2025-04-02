//! Call routing engine for the RVOIP stack.
//!
//! This crate provides core call routing, policy enforcement, and business logic
//! for the RVOIP telephony platform. It acts as the central coordinator between
//! SIP signaling, media handling, and session management.

pub mod engine;
pub mod routing;
pub mod policy;
pub mod errors;
pub mod registry;

pub use engine::CallEngine;
pub use errors::Error;
pub use registry::Registry;

/// Re-export types from dependent crates that are used in our public API
pub mod prelude {
    pub use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri, Header, HeaderName};
    pub use rvoip_transaction_core::TransactionManager;
    pub use rvoip_session_core::{
        Session, SessionId, SessionState, SessionManager, 
        Dialog, DialogId, DialogState,
    };
    
    pub use crate::{CallEngine, Error, Registry};
} 