//! Session Creation Functions
//!
//! Simple functions for creating outgoing calls and handling incoming calls.

use std::sync::Arc;
use crate::api::types::{CallSession, SessionId, IncomingCall, CallState};
use crate::manager::SessionManager;
use crate::Result;

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
/// use rvoip_session_core::Result;
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
/// let call = make_call_with_sdp(&session_mgr, "sip:alice@example.com", "sip:bob@example.com", &sdp_offer).await?;
/// # Ok(())
/// # }
/// ```
pub fn generate_sdp_offer(local_ip: &str, local_port: u16) -> Result<String> {
    use crate::media::config::MediaConfigConverter;
    let converter = MediaConfigConverter::new();
    converter.generate_sdp_offer(local_ip, local_port)
        .map_err(|e| crate::SessionError::MediaError(e.to_string()))
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
///             Err(_) => CallDecision::Reject("Incompatible media"),
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
        .map_err(|e| crate::SessionError::MediaError(e.to_string()))
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
        .map_err(|e| crate::SessionError::MediaError(e.to_string()))
}

/// Accept an incoming call
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session_id` - The ID of the incoming call session
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// 
/// #[derive(Debug)]
/// struct MyHandler;
/// 
/// #[async_trait::async_trait]
/// impl CallHandler for MyHandler {
///     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
///         CallDecision::Accept(None)
///     }
///     
///     async fn on_call_ended(&self, _call: CallSession, _reason: &str) {}
/// }
/// ```
pub async fn accept_call(session_manager: &Arc<SessionManager>, session_id: &SessionId) -> Result<CallSession> {
    session_manager.accept_incoming_call(session_id).await
}

/// Reject an incoming call with a specific reason
/// 
/// # Arguments
/// * `session_manager` - The SessionManager instance
/// * `session_id` - The ID of the incoming call session
/// * `reason` - The reason for rejecting the call
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::api::*;
/// use std::sync::Arc;
/// use rvoip_session_core::{SessionManager, Result};
/// 
/// # async fn example(session_manager: &Arc<SessionManager>, call: IncomingCall) -> Result<()> {
/// reject_call(session_manager, &call.id, "Busy").await?;
/// # Ok(())
/// # }
/// ```
pub async fn reject_call(session_manager: &Arc<SessionManager>, session_id: &SessionId, reason: &str) -> Result<()> {
    // Terminate the session to reject the call
    session_manager.terminate_session(session_id).await?;
    tracing::info!("Rejected call {} with reason: {}", session_id, reason);
    Ok(())
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
    _manager: Arc<SessionManager>,
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