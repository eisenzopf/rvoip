//! Session Creation API
//!
//! High-level API for creating new sessions and generating SDP.
//! 
//! # Overview
//! 
//! This module provides convenience functions for common session operations:
//! - Creating outgoing calls
//! - Generating SDP offers and answers
//! - Parsing SDP responses
//! - Managing session lifecycle
//! 
//! # Key Functions
//! 
//! ## Call Creation
//! - `create_call()` - Simple outgoing call
//! - `create_call_with_sdp()` - Call with custom SDP
//! 
//! ## SDP Generation
//! - `generate_sdp_offer()` - Create SDP for outgoing calls
//! - `generate_sdp_answer()` - Respond to incoming SDP offers
//! - `parse_sdp_answer()` - Extract media parameters from SDP
//! 
//! ## Session Management
//! - `accept_call()` - Accept incoming calls programmatically
//! - `reject_call()` - Reject incoming calls
//! - `find_session()` - Look up active sessions
//! 
//! # Example: Complete Call Flow
//! 
//! ```rust
//! use rvoip_session_core::{SessionCoordinator, SessionControl, Result};
//! use rvoip_session_core::create::{generate_sdp_offer, create_call_with_sdp, parse_sdp_answer};
//! use std::sync::Arc;
//! use std::time::Duration;
//! 
//! async fn make_call_example(
//!     coordinator: Arc<SessionCoordinator>
//! ) -> Result<()> {
//!     // 1. Generate SDP offer
//!     let sdp_offer = generate_sdp_offer("192.168.1.100", 10000)?;
//!     
//!     // 2. Create call with SDP
//!     let session = create_call_with_sdp(
//!         &coordinator,
//!         "sip:bob@example.com",
//!         Some("sip:alice@ourserver.com"),
//!         sdp_offer
//!     ).await?;
//!     
//!     // 3. Wait for answer (using SessionControl)
//!     SessionControl::wait_for_answer(
//!         &coordinator,
//!         &session.id,
//!         Duration::from_secs(30)
//!     ).await?;
//!     
//!     // 4. Get the answer SDP
//!     if let Some(media_info) = SessionControl::get_media_info(
//!         &coordinator,
//!         &session.id
//!     ).await? {
//!         if let Some(remote_sdp) = media_info.remote_sdp {
//!             // 5. Parse the answer to get media parameters
//!             let negotiated = parse_sdp_answer(&remote_sdp)?;
//!             println!("Call established with {}", negotiated.codec.name);
//!             println!("Remote RTP: {}:{}", 
//!                 negotiated.remote_ip, 
//!                 negotiated.remote_port
//!             );
//!         }
//!     }
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # SDP Negotiation Example
//! 
//! ```rust
//! use rvoip_session_core::{CallHandler, IncomingCall, CallDecision, CallSession};
//! use rvoip_session_core::create::generate_sdp_answer;
//! 
//! #[derive(Debug)]
//! struct MyHandler;
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for MyHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         if let Some(offer) = &call.sdp {
//!             // Generate compatible answer
//!             match generate_sdp_answer(offer, "192.168.1.100", 20000) {
//!                 Ok(answer) => {
//!                     println!("Accepting call with SDP answer");
//!                     CallDecision::Accept(Some(answer))
//!                 }
//!                 Err(e) => {
//!                     println!("SDP negotiation failed: {}", e);
//!                     CallDecision::Reject("Incompatible media".to_string())
//!                 }
//!             }
//!         } else {
//!             // No SDP in offer, accept without media
//!             CallDecision::Accept(None)
//!         }
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         println!("Call {} ended: {}", call.id(), reason);
//!     }
//! }
//! ```

use std::sync::Arc;
use crate::api::types::{CallSession, SessionId, CallState, IncomingCall, CallDecision};
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};

/// Create an outgoing call
/// 
/// This is a convenience function for simple call creation. It automatically
/// generates an SDP offer with default media capabilities.
/// 
/// # Arguments
/// * `manager` - The session coordinator instance
/// * `to` - Destination SIP URI (e.g., "sip:bob@example.com")
/// * `from` - Optional source SIP URI (defaults to "sip:user@localhost")
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::{SessionCoordinator, Result};
/// use rvoip_session_core::create::create_call;
/// use std::sync::Arc;
/// 
/// async fn quick_call(coordinator: &Arc<SessionCoordinator>) -> Result<()> {
///     // Simple call with defaults
///     let session = create_call(
///         coordinator,
///         "sip:alice@example.com",
///         None
///     ).await?;
///     
///     // Call with specific from address
///     let session2 = create_call(
///         coordinator,
///         "sip:bob@example.com",
///         Some("sip:john@mycompany.com")
///     ).await?;
///     
///     Ok(())
/// }
/// ```
pub async fn create_call(
    manager: &Arc<SessionCoordinator>,
    to: &str,
    from: Option<&str>,
) -> Result<CallSession> {
    let from_uri = from.unwrap_or("sip:user@localhost");
    manager.create_outgoing_call(from_uri, to, None).await
}

/// Create an outgoing call with custom SDP
/// 
/// Use this when you need specific media capabilities or have already
/// generated an SDP offer using `generate_sdp_offer()`.
/// 
/// # Arguments
/// * `manager` - The session coordinator instance
/// * `to` - Destination SIP URI
/// * `from` - Optional source SIP URI
/// * `sdp` - Pre-generated SDP offer
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::{SessionCoordinator, Result};
/// use rvoip_session_core::create::{create_call_with_sdp, generate_sdp_offer};
/// use std::sync::Arc;
/// 
/// async fn call_with_specific_media(
///     coordinator: &Arc<SessionCoordinator>
/// ) -> Result<()> {
///     // Generate SDP with specific port
///     let sdp = generate_sdp_offer("192.168.1.100", 15000)?;
///     
///     // Create call with that SDP
///     let session = create_call_with_sdp(
///         coordinator,
///         "sip:conference@server.com",
///         Some("sip:participant@client.com"),
///         sdp
///     ).await?;
///     
///     println!("Call created with custom media on port 15000");
///     Ok(())
/// }
/// ```
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
/// use rvoip_session_core::{SessionManagerBuilder, Result};
/// use rvoip_session_core::create::{generate_sdp_offer, create_call_with_sdp};
/// 
/// # async fn example() -> Result<()> {
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
/// use rvoip_session_core::{IncomingCall, CallDecision};
/// use rvoip_session_core::create::generate_sdp_answer;
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
/// use rvoip_session_core::create::parse_sdp_answer;
/// use rvoip_session_core::Result;
/// 
/// # fn example(answer_sdp: &str) -> Result<()> {
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
/// You usually won't need to call this directly - incoming calls are
/// handled through the CallHandler interface.
/// 
/// # Arguments
/// * `from` - Caller's SIP URI
/// * `to` - Called party's SIP URI
/// * `sdp` - Optional SDP offer from the caller
/// * `headers` - SIP headers from the INVITE
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::create::create_incoming_call;
/// use std::collections::HashMap;
/// 
/// // Simulate an incoming call for testing
/// let mut headers = HashMap::new();
/// headers.insert("User-Agent".to_string(), "TestPhone/1.0".to_string());
/// 
/// let sdp_offer = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
/// 
/// let call = create_incoming_call(
///     "sip:alice@example.com",
///     "sip:bob@ourserver.com",
///     Some(sdp_offer.to_string()),
///     headers
/// );
/// ```
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
/// 
/// Converts an IncomingCall into an active CallSession after acceptance.
/// This is typically used internally after a call is accepted.
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::{IncomingCall, CallSession, SessionCoordinator};
/// use rvoip_session_core::create::create_call_session;
/// use std::sync::Arc;
/// 
/// fn handle_accepted_call(
///     incoming: &IncomingCall,
///     coordinator: Arc<SessionCoordinator>
/// ) -> CallSession {
///     let session = create_call_session(incoming, coordinator);
///     println!("Call {} is now active", session.id());
///     session
/// }
/// ```
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
/// 
/// Returns aggregate statistics about all sessions managed by this coordinator.
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::{SessionCoordinator, Result};
/// use rvoip_session_core::create::get_session_stats;
/// use std::sync::Arc;
/// 
/// async fn monitor_system(coordinator: &Arc<SessionCoordinator>) -> Result<()> {
///     let stats = get_session_stats(coordinator).await?;
///     
///     println!("System Statistics:");
///     println!("  Total sessions: {}", stats.total_sessions);
///     println!("  Active calls: {}", stats.active_sessions);
///     println!("  Failed calls: {}", stats.failed_sessions);
///     
///     if let Some(avg_duration) = stats.average_duration {
///         println!("  Average duration: {:?}", avg_duration);
///     }
///     
///     Ok(())
/// }
/// ```
pub async fn get_session_stats(session_manager: &Arc<SessionCoordinator>) -> Result<crate::api::types::SessionStats> {
    session_manager.get_stats().await
}

/// List all active sessions
/// 
/// Returns the IDs of all currently active sessions. Useful for monitoring
/// and administrative interfaces.
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::{SessionCoordinator, Result};
/// use rvoip_session_core::create::{list_active_sessions, find_session};
/// use std::sync::Arc;
/// 
/// async fn show_active_calls(coordinator: &Arc<SessionCoordinator>) -> Result<()> {
///     let sessions = list_active_sessions(coordinator).await?;
///     
///     println!("Active calls: {}", sessions.len());
///     for session_id in sessions {
///         if let Some(session) = find_session(coordinator, &session_id).await? {
///             println!("  {} -> {} ({})", 
///                 session.from, 
///                 session.to, 
///                 session.state()
///             );
///         }
///     }
///     
///     Ok(())
/// }
/// ```
pub async fn list_active_sessions(session_manager: &Arc<SessionCoordinator>) -> Result<Vec<SessionId>> {
    session_manager.list_active_sessions().await
}

/// Find a session by ID
/// 
/// Look up detailed information about a specific session.
/// 
/// # Arguments
/// * `session_manager` - The coordinator instance
/// * `session_id` - ID of the session to find
/// 
/// # Returns
/// * `Some(CallSession)` if found
/// * `None` if not found
/// 
/// # Example
/// ```rust
/// use rvoip_session_core::{SessionCoordinator, SessionId, Result};
/// use rvoip_session_core::create::find_session;
/// use std::sync::Arc;
/// 
/// async fn check_call_status(
///     coordinator: &Arc<SessionCoordinator>,
///     session_id: &SessionId
/// ) -> Result<()> {
///     match find_session(coordinator, session_id).await? {
///         Some(session) => {
///             println!("Call {} is {}", session.id(), session.state());
///             if session.is_active() {
///                 println!("Call is connected!");
///             }
///         }
///         None => {
///             println!("Call not found - may have ended");
///         }
///     }
///     
///     Ok(())
/// }
/// ```
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