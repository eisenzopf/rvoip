//! Session Creation Functions
//!
//! Simple functions for creating outgoing calls and handling incoming calls.

use std::sync::Arc;
use crate::api::types::{CallSession, SessionId, IncomingCall, CallState};
use crate::manager::SessionManager;
use crate::errors::Result;

/// Make an outgoing call to the specified destination
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `from` - The caller URI (e.g., "sip:alice@example.com")
/// * `to` - The destination URI (e.g., "sip:bob@example.com")
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example() -> Result<()> {
/// let session_mgr = SessionManagerBuilder::new().build().await?;
/// let call = make_call_with_manager(
///     &session_mgr,
///     "sip:alice@192.168.1.100", 
///     "sip:bob@192.168.1.200"
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub async fn make_call_with_manager(
    session_manager: &Arc<SessionManager>,
    from: &str,
    to: &str,
) -> Result<CallSession> {
    session_manager.create_outgoing_call(from, to, None).await
}

/// Make an outgoing call with custom SDP
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `from` - The caller URI
/// * `to` - The destination URI
/// * `sdp` - Custom SDP offer
pub async fn make_call_with_sdp(
    session_manager: &Arc<SessionManager>,
    from: &str,
    to: &str,
    sdp: &str,
) -> Result<CallSession> {
    session_manager.create_outgoing_call(from, to, Some(sdp.to_string())).await
}

/// Accept an incoming call
/// 
/// # Arguments
/// * `session_id` - The ID of the incoming call session
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// struct MyHandler;
/// 
/// #[async_trait::async_trait]
/// impl CallHandler for MyHandler {
///     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
///         // Accept the call
///         match accept_call(&call.id).await {
///             Ok(_) => CallDecision::Accept,
///             Err(_) => CallDecision::Reject("Failed to accept".to_string()),
///         }
///     }
///     
///     async fn on_call_ended(&self, _call: CallSession, _reason: &str) {}
/// }
/// ```
pub async fn accept_call(session_id: &SessionId) -> Result<CallSession> {
    // TODO: Get the SessionManager from context or registry
    // For now, this is a placeholder implementation
    todo!("accept_call implementation - need SessionManager context")
}

/// Reject an incoming call with a specific reason
/// 
/// # Arguments
/// * `session_id` - The ID of the incoming call session
/// * `reason` - The reason for rejecting the call
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example(call: IncomingCall) -> Result<()> {
/// reject_call(&call.id, "Busy").await?;
/// # Ok(())
/// # }
/// ```
pub async fn reject_call(session_id: &SessionId, reason: &str) -> Result<()> {
    // TODO: Get the SessionManager from context or registry
    // For now, this is a placeholder implementation
    todo!("reject_call implementation - need SessionManager context")
}

/// Create an incoming call object from SIP INVITE request
/// 
/// This is typically called internally when a SIP INVITE is received.
pub fn create_incoming_call(
    from: &str,
    to: &str,
    sdp: Option<String>,
    headers: std::collections::HashMap<String, String>,
) -> IncomingCall {
    IncomingCall {
        id: SessionId::new(),
        from: from.to_string(),
        to: to.to_string(),
        sdp,
        headers,
        received_at: std::time::Instant::now(),
    }
}

/// Helper function to create a CallSession from an accepted IncomingCall
pub fn create_call_session(
    incoming: &IncomingCall,
    manager: Arc<SessionManager>,
) -> CallSession {
    CallSession {
        id: incoming.id.clone(),
        from: incoming.from.clone(),
        to: incoming.to.clone(),
        state: CallState::Initiating,
        started_at: Some(std::time::Instant::now()),
        manager,
    }
}

/// Get statistics about active sessions
pub async fn get_session_stats(session_manager: &Arc<SessionManager>) -> Result<crate::api::types::SessionStats> {
    session_manager.get_stats().await
}

/// List all active sessions
pub async fn list_active_sessions(session_manager: &Arc<SessionManager>) -> Result<Vec<SessionId>> {
    session_manager.list_active_sessions().await
}

/// Find a session by ID
pub async fn find_session(session_manager: &Arc<SessionManager>, session_id: &SessionId) -> Result<Option<CallSession>> {
    session_manager.find_session(session_id).await
} 