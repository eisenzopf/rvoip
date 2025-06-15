//! Session Creation API
//!
//! High-level API for creating new sessions.

use std::sync::Arc;
use crate::api::types::{CallSession, SessionId, CallState, IncomingCall, CallDecision};
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};

/// Create an outgoing call
pub async fn create_call(
    manager: &Arc<SessionCoordinator>,
    to: &str,
    from: Option<&str>,
) -> Result<CallSession> {
    let from_uri = from.unwrap_or("sip:user@localhost");
    manager.create_outgoing_call(from_uri, to, None).await
}

/// Create an outgoing call with custom SDP
pub async fn create_call_with_sdp(
    manager: &Arc<SessionCoordinator>,
    to: &str,
    from: Option<&str>,
    sdp: String,
) -> Result<CallSession> {
    let from_uri = from.unwrap_or("sip:user@localhost");
    manager.create_outgoing_call(from_uri, to, Some(sdp)).await
}

/// Generate an SDP offer for making calls
/// 
/// This function creates a proper SDP offer with the system's supported codecs
/// and capabilities. Use this instead of manually creating SDP.
/// 
/// # Arguments
/// * `local_ip` - Local IP address for media
/// * `local_port` - Local RTP port for media
/// 
/// # Returns
/// SDP offer string ready to be used in `make_call_with_sdp`
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # async fn example() -> rvoip_session_core::Result<()> {
/// let sdp_offer = generate_sdp_offer("127.0.0.1", 10000)?;
/// let session_mgr = SessionManagerBuilder::new().build().await?;
/// let call = create_call_with_sdp(&session_mgr, "sip:bob@example.com", None, sdp_offer).await?;
/// # Ok(())
/// # }
/// ```
pub fn generate_sdp_offer(local_ip: &str, local_port: u16) -> Result<String> {
    use crate::media::config::MediaConfigConverter;
    let converter = MediaConfigConverter::new();
    converter.generate_sdp_offer(local_ip, local_port)
        .map_err(|e| SessionError::MediaError(e.to_string()))
}

/// Generate an SDP answer in response to an offer
/// 
/// This function performs proper codec negotiation according to RFC 3264.
/// It finds compatible codecs between the offer and local capabilities,
/// and generates an appropriate answer.
/// 
/// # Arguments
/// * `offer_sdp` - The incoming SDP offer to respond to
/// * `local_ip` - Local IP address for media
/// * `local_port` - Local RTP port for media
/// 
/// # Returns
/// SDP answer string with negotiated codecs
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// async fn handle_incoming_call(call: IncomingCall) -> CallDecision {
///     if let Some(ref offer) = call.sdp {
///         match generate_sdp_answer(offer, "127.0.0.1", 10001) {
///             Ok(answer) => CallDecision::Accept(Some(answer)),
///             Err(_) => CallDecision::Reject("Incompatible media".to_string()),
///         }
///     } else {
///         CallDecision::Accept(None)
///     }
/// }
/// ```
pub fn generate_sdp_answer(offer_sdp: &str, local_ip: &str, local_port: u16) -> Result<String> {
    use crate::media::config::MediaConfigConverter;
    let converter = MediaConfigConverter::new();
    converter.generate_sdp_answer(offer_sdp, local_ip, local_port)
        .map_err(|e| SessionError::MediaError(e.to_string()))
}

/// Parse an SDP answer to extract negotiated media parameters
/// 
/// This function extracts the negotiated codec, remote IP, and remote port
/// from an SDP answer after a call has been established.
/// 
/// # Arguments
/// * `answer_sdp` - The SDP answer received from the remote party
/// 
/// # Returns
/// Negotiated media configuration
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// # fn example(answer_sdp: &str) -> rvoip_session_core::Result<()> {
/// let negotiated = parse_sdp_answer(answer_sdp)?;
/// println!("Negotiated codec: {}", negotiated.codec.name);
/// println!("Remote endpoint: {}:{}", negotiated.remote_ip, negotiated.remote_port);
/// # Ok(())
/// # }
/// ```
pub fn parse_sdp_answer(answer_sdp: &str) -> Result<crate::media::config::NegotiatedConfig> {
    use crate::media::config::MediaConfigConverter;
    let converter = MediaConfigConverter::new();
    converter.parse_sdp_answer(answer_sdp)
        .map_err(|e| SessionError::MediaError(e.to_string()))
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
    _manager: Arc<SessionCoordinator>,
) -> CallSession {
    CallSession {
        id: incoming.id.clone(),
        from: incoming.from.clone(),
        to: incoming.to.clone(),
        state: CallState::Initiating,
        started_at: Some(std::time::Instant::now()),
    }
}

/// Get statistics about active sessions
pub async fn get_session_stats(session_manager: &Arc<SessionCoordinator>) -> Result<crate::api::types::SessionStats> {
    session_manager.get_stats().await
}

/// List all active sessions
pub async fn list_active_sessions(session_manager: &Arc<SessionCoordinator>) -> Result<Vec<SessionId>> {
    session_manager.list_active_sessions().await
}

/// Find a session by ID
pub async fn find_session(session_manager: &Arc<SessionCoordinator>, session_id: &SessionId) -> Result<Option<CallSession>> {
    session_manager.find_session(session_id).await
}

/// Accept an incoming call
/// 
/// This function accepts a pending incoming call.
/// 
/// # Arguments
/// * `session_manager` - The session coordinator instance
/// * `session_id` - ID of the incoming call session to accept
/// 
/// # Returns
/// The accepted CallSession
/// 
/// # Errors
/// Returns an error if the session doesn't exist or is not in a state that can be accepted
pub async fn accept_call(
    session_manager: &Arc<SessionCoordinator>,
    session_id: &SessionId,
) -> Result<CallSession> {
    // Find the session
    let session = session_manager.find_session(session_id).await?
        .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
    
    // For now, just return the session as accepted
    // The actual accept logic would be handled by the dialog coordinator
    Ok(session)
}

/// Reject an incoming call
/// 
/// This function rejects a pending incoming call.
/// 
/// # Arguments
/// * `session_manager` - The session coordinator instance
/// * `session_id` - ID of the incoming call session to reject
/// * `reason` - Reason for rejection
/// 
/// # Returns
/// Ok(()) if the call was successfully rejected
/// 
/// # Errors
/// Returns an error if the session doesn't exist or is not in a state that can be rejected
pub async fn reject_call(
    session_manager: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    reason: &str,
) -> Result<()> {
    // Check if session exists
    let _session = session_manager.find_session(session_id).await?
        .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
    
    // Terminate the session with the rejection reason
    session_manager.terminate_session(session_id).await?;
    
    Ok(())
} 