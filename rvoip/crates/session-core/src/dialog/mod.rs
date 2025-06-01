//! Dialog coordination module
//!
//! This module handles session-level dialog coordination with dialog-core.
//! It focuses on session-specific coordination and delegates SIP protocol
//! handling to dialog-core according to RFC 3261.
//!
//! **ARCHITECTURAL NOTE**: The core dialog implementation has been moved to 
//! dialog-core. This module now focuses purely on session-level coordination
//! and event processing.

use std::net::SocketAddr;
use rvoip_sip_core::Request;

// Session-level dialog coordination modules (minimal set)
pub mod testing;

// Re-export dialog types from dialog-core (authoritative source)
pub use rvoip_dialog_core::{DialogId, Dialog, DialogState, DialogManager, SessionCoordinationEvent};

/// Information about an incoming call that needs session coordination
/// This is passed from DialogManager to SessionManager for proper layer separation
#[derive(Debug, Clone)]
pub struct IncomingCallInfo {
    /// The original INVITE request
    pub request: Request,
    
    /// Source address of the INVITE
    pub source: SocketAddr,
    
    /// Session ID created for this call
    pub session_id: crate::session::SessionId,
} 