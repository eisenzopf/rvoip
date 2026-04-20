//! UAS-specific types

use std::sync::Arc;
use tokio::sync::RwLock;
use crate::api::types::{SessionId, CallState, IncomingCall, CallSession};
use crate::errors::Result;

/// UAS configuration
#[derive(Debug, Clone)]
pub struct UasConfig {
    /// Local bind address (e.g., "0.0.0.0:5060")
    pub local_addr: String,
    
    /// Server identity (e.g., "sip:server@example.com")
    pub identity: String,
    
    /// User agent string
    pub user_agent: String,
    
    /// Maximum concurrent calls (0 = unlimited)
    pub max_concurrent_calls: usize,
    
    /// Auto-reject when at capacity
    pub auto_reject_on_busy: bool,
    
    /// Enable authentication
    pub require_authentication: bool,
    
    /// Authentication realm
    pub auth_realm: Option<String>,
    
    /// Call timeout in seconds
    pub call_timeout: u32,
    
    /// Enable auto-answer (for testing)
    pub auto_answer: bool,
    
    /// Enable call recording
    pub enable_recording: bool,
}

impl Default for UasConfig {
    fn default() -> Self {
        Self {
            local_addr: "0.0.0.0:5060".to_string(),
            identity: "sip:server@localhost".to_string(),
            user_agent: format!("session-core-uas/{}", env!("CARGO_PKG_VERSION")),
            max_concurrent_calls: 0,
            auto_reject_on_busy: true,
            require_authentication: false,
            auth_realm: None,
            call_timeout: 300, // 5 minutes
            auto_answer: false,
            enable_recording: false,
        }
    }
}

/// Decision on how to handle an incoming call
#[derive(Debug, Clone)]
pub enum UasCallDecision {
    /// Accept the call with optional SDP answer
    Accept(Option<String>),
    /// Reject the call with reason
    Reject(String),
    /// Forward to another URI
    Forward(String),
    /// Queue the call for later processing
    Queue,
    /// Defer decision (handle programmatically)
    Defer,
}

/// Active UAS call handle
#[derive(Clone)]
pub struct UasCall {
    /// Session ID
    pub session_id: SessionId,
    /// Caller URI
    pub from: String,
    /// Called URI
    pub to: String,
    /// Call state
    pub state: CallState,
    /// Start time
    pub started_at: std::time::Instant,
}

impl UasCall {
    pub fn new(session: &CallSession) -> Self {
        Self {
            session_id: session.id.clone(),
            from: session.from.clone(),
            to: session.to.clone(),
            state: session.state.clone(),
            started_at: std::time::Instant::now(),
        }
    }
    
    pub fn duration(&self) -> std::time::Duration {
        self.started_at.elapsed()
    }
}

/// UAS statistics
#[derive(Debug, Clone, Default)]
pub struct UasStats {
    /// Total calls received
    pub calls_received: u64,
    /// Calls accepted
    pub calls_accepted: u64,
    /// Calls rejected
    pub calls_rejected: u64,
    /// Calls forwarded
    pub calls_forwarded: u64,
    /// Calls queued
    pub calls_queued: u64,
    /// Current active calls
    pub active_calls: u32,
    /// Average call duration in seconds
    pub avg_call_duration: u64,
}