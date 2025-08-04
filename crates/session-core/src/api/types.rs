//! Core API Types
//!
//! Defines the main types that developers interact with when using the session API.
//! 
//! # Key Types Overview
//! 
//! - **`SessionId`** - Unique identifier for each call session
//! - **`CallSession`** - Represents an active call with state and metadata
//! - **`IncomingCall`** - Data about an incoming call requiring a decision
//! - **`CallState`** - Current state of a call (Ringing, Active, etc.)
//! - **`CallDecision`** - How to handle an incoming call
//! - **`MediaInfo`** - Information about media streams and quality
//! 
//! # Call Lifecycle Example
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::time::Instant;
//! use std::collections::HashMap;
//! 
//! // 1. Incoming call arrives
//! let sdp_offer = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0\r\n";
//! let sdp_answer = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5006 RTP/AVP 0\r\n";
//! let headers = HashMap::new();
//! 
//! let incoming_call = IncomingCall {
//!     id: SessionId::new(),
//!     from: "sip:alice@example.com".to_string(),
//!     to: "sip:bob@ourserver.com".to_string(),
//!     sdp: Some(sdp_offer.to_string()),
//!     headers: headers,
//!     received_at: Instant::now(),
//! };
//! 
//! // 2. Handler makes a decision
//! let decision = CallDecision::Accept(Some(sdp_answer.to_string()));
//! 
//! // 3. Call becomes active
//! let session = CallSession {
//!     id: incoming_call.id.clone(),
//!     from: incoming_call.from,
//!     to: incoming_call.to,
//!     state: CallState::Active,
//!     started_at: Some(Instant::now()),
//! };
//! 
//! // 4. Monitor call state
//! match session.state() {
//!     CallState::Active => println!("Call is connected"),
//!     CallState::OnHold => println!("Call is on hold"),
//!     CallState::Failed(reason) => println!("Call failed: {}", reason),
//!     _ => {}
//! }
//! ```
//! 
//! # Call States
//! 
//! ```text
//! Initiating -> Ringing -> Active -> Terminated
//!                |           |
//!                |           +-----> OnHold -> Active
//!                |           |
//!                |           +-----> Transferring -> Terminated
//!                |
//!                +--------> Failed/Cancelled
//! ```
//! 
//! # SDP Parsing
//! 
//! ```rust
//! use rvoip_session_core::api::parse_sdp_connection;
//! 
//! fn parse_example() -> Result<(), Box<dyn std::error::Error>> {
//!     let sdp = r#"v=0
//! o=- 0 0 IN IP4 127.0.0.1
//! s=-
//! c=IN IP4 192.168.1.100
//! t=0 0
//! m=audio 5004 RTP/AVP 0 8 101
//! a=rtpmap:0 PCMU/8000
//! a=rtpmap:8 PCMA/8000
//! a=rtpmap:101 telephone-event/8000"#;
//!     
//!     let info = parse_sdp_connection(sdp)?;
//!     assert_eq!(info.ip, "192.168.1.100");
//!     assert_eq!(info.port, 5004);
//!     assert!(info.codecs.contains(&"PCMU".to_string()));
//!     Ok(())
//! }
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};
use serde::{Serialize, Deserialize};
use uuid::Uuid;
use crate::errors::Result;
use std::fmt;

// Re-export StatusCode for convenience
pub use rvoip_sip_core::StatusCode;

// Re-export SessionError as Error for compatibility
pub use crate::errors::SessionError as Error;

/// Unique identifier for a session
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    /// Create a new random session ID
    pub fn new() -> Self {
        Self(format!("sess_{}", Uuid::new_v4()))
    }
    
    /// Create a session ID from a string
    pub fn from_string(id: String) -> Self {
        Self(id)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

/// Alias for CallSession for compatibility
pub type Session = CallSession;

/// Represents a prepared outgoing call with allocated resources
/// This is created before initiating the actual SIP INVITE
#[derive(Debug, Clone)]
pub struct PreparedCall {
    /// The session ID for this call
    pub session_id: SessionId,
    /// Local SIP URI
    pub from: String,
    /// Remote SIP URI
    pub to: String,
    /// Generated SDP offer with allocated media ports
    pub sdp_offer: String,
    /// Local RTP port that was allocated
    pub local_rtp_port: u16,
}

/// Represents an active call session
#[derive(Debug, Clone)]
pub struct CallSession {
    pub id: SessionId,
    pub from: String,
    pub to: String,
    pub state: CallState,
    pub started_at: Option<Instant>,
}

impl CallSession {
    /// Get the session ID
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// Get the current call state
    pub fn state(&self) -> &CallState {
        &self.state
    }

    /// Check if the call is active (connected)
    pub fn is_active(&self) -> bool {
        matches!(self.state, CallState::Active)
    }

    /// Wait for the call to be answered
    /// Note: Use SessionManager::wait_for_answer() method instead
    pub async fn wait_for_answer(&self) -> Result<()> {
        // This method requires access to the event system
        Err(crate::errors::SessionError::Other(
            "Use SessionManager::wait_for_answer() method instead".to_string()
        ))
    }

    /// Hold the call
    /// Note: Use SessionManager::hold_session() method instead
    pub async fn hold(&self) -> Result<()> {
        // This method now requires the caller to use SessionManager directly
        Err(crate::errors::SessionError::Other(
            "Use SessionManager::hold_session() method instead".to_string()
        ))
    }

    /// Resume the call from hold
    /// Note: Use SessionManager::resume_session() method instead
    pub async fn resume(&self) -> Result<()> {
        // This method now requires the caller to use SessionManager directly
        Err(crate::errors::SessionError::Other(
            "Use SessionManager::resume_session() method instead".to_string()
        ))
    }

    /// Transfer the call to another destination
    /// Note: Use SessionManager::transfer_session() method instead
    pub async fn transfer(&self, target: &str) -> Result<()> {
        // This method now requires the caller to use SessionManager directly
        Err(crate::errors::SessionError::Other(
            "Use SessionManager::transfer_session() method instead".to_string()
        ))
    }

    /// Terminate the call
    /// Note: Use SessionManager::terminate_session() method instead
    pub async fn terminate(&self) -> Result<()> {
        // This method now requires the caller to use SessionManager directly
        Err(crate::errors::SessionError::Other(
            "Use SessionManager::terminate_session() method instead".to_string()
        ))
    }
}

/// Represents an incoming call that needs to be handled
#[derive(Debug, Clone)]
pub struct IncomingCall {
    pub id: SessionId,
    pub from: String,
    pub to: String,
    pub sdp: Option<String>,
    pub headers: std::collections::HashMap<String, String>,
    pub received_at: Instant,
}

impl IncomingCall {
    /// Accept the incoming call
    /// Note: Use accept_call() function with SessionManager parameter instead
    pub async fn accept(&self) -> Result<CallSession> {
        Err(crate::errors::SessionError::Other(
            "Use accept_call(session_manager, &session_id) function instead".to_string()
        ))
    }

    /// Reject the incoming call with a reason
    /// Note: Use reject_call() function with SessionManager parameter instead
    pub async fn reject(&self, reason: &str) -> Result<()> {
        Err(crate::errors::SessionError::Other(
            "Use reject_call(session_manager, &session_id, reason) function instead".to_string()
        ))
    }

    /// Get caller information
    pub fn caller(&self) -> &str {
        &self.from
    }

    /// Get called party information
    pub fn called(&self) -> &str {
        &self.to
    }
}

/// Current state of a call session
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    /// Call is being initiated
    Initiating,
    /// Call is ringing (180 Ringing received)
    Ringing,
    /// Call is active and media is flowing
    Active,
    /// Call is on hold
    OnHold,
    /// Call is being transferred
    Transferring,
    /// Call is being terminated
    Terminating,
    /// Call has ended
    Terminated,
    /// Call was cancelled (487 Request Terminated)
    Cancelled,
    /// Call failed or was rejected
    Failed(String),
}

impl CallState {
    /// Check if this is a final state (call is over)
    pub fn is_final(&self) -> bool {
        matches!(self, CallState::Terminated | CallState::Cancelled | CallState::Failed(_))
    }

    /// Check if the call is in progress
    pub fn is_in_progress(&self) -> bool {
        matches!(self, CallState::Initiating | CallState::Ringing | CallState::Active | CallState::OnHold)
    }
}

impl fmt::Display for CallState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallState::Initiating => write!(f, "Initiating"),
            CallState::Ringing => write!(f, "Ringing"),
            CallState::Active => write!(f, "Active"),
            CallState::OnHold => write!(f, "OnHold"),
            CallState::Transferring => write!(f, "Transferring"),
            CallState::Terminating => write!(f, "Terminating"),
            CallState::Terminated => write!(f, "Terminated"),
            CallState::Cancelled => write!(f, "Cancelled"),
            CallState::Failed(reason) => write!(f, "Failed: {}", reason),
        }
    }
}

/// Decision on how to handle an incoming call
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallDecision {
    /// Accept the call immediately, optionally with SDP answer
    Accept(Option<String>),
    /// Reject the call with a reason
    Reject(String),
    /// Defer the decision (e.g., add to queue)
    Defer,
    /// Forward the call to another destination
    Forward(String),
}

impl CallDecision {
    /// Create a reject decision with status code
    pub fn reject_with_code(status_code: StatusCode, reason: Option<String>) -> Self {
        CallDecision::Reject(reason.unwrap_or_else(|| status_code.to_string()))
    }
    
    /// Create an accept decision with optional SDP
    pub fn accept(sdp: Option<String>) -> Self {
        CallDecision::Accept(sdp)
    }
    
    /// Create a defer decision
    pub fn defer() -> Self {
        CallDecision::Defer
    }
    
    /// Create a forward decision
    pub fn forward(destination: impl Into<String>) -> Self {
        CallDecision::Forward(destination.into())
    }
}

/// Statistics about active sessions
#[derive(Debug, Clone)]
pub struct SessionStats {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub failed_sessions: usize,
    pub average_duration: Option<std::time::Duration>,
}

/// Media information for a session
#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub local_rtp_port: Option<u16>,
    pub remote_rtp_port: Option<u16>,
    pub codec: Option<String>,
    pub rtp_stats: Option<crate::media::stats::RtpSessionStats>,
    pub quality_metrics: Option<crate::media::stats::QualityMetrics>,
}

/// Call direction
#[derive(Debug, Clone, PartialEq)]
pub enum CallDirection {
    /// Outgoing call (UAC)
    Outgoing,
    /// Incoming call (UAS)
    Incoming,
}

/// Call termination reason
#[derive(Debug, Clone)]
pub enum TerminationReason {
    /// Normal hangup by local party
    LocalHangup,
    /// Normal hangup by remote party
    RemoteHangup,
    /// Call rejected
    Rejected(String),
    /// Call failed due to error
    Error(String),
    /// Call timed out
    Timeout,
}

impl fmt::Display for TerminationReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TerminationReason::LocalHangup => write!(f, "Local hangup"),
            TerminationReason::RemoteHangup => write!(f, "Remote hangup"),
            TerminationReason::Rejected(reason) => write!(f, "Rejected: {}", reason),
            TerminationReason::Error(error) => write!(f, "Error: {}", error),
            TerminationReason::Timeout => write!(f, "Timeout"),
        }
    }
}

/// Parsed SDP information for easier handling
#[derive(Debug, Clone)]
pub struct SdpInfo {
    /// Connection IP address
    pub ip: String,
    /// Media port (typically RTP port)
    pub port: u16,
    /// List of supported codecs
    pub codecs: Vec<String>,
}

/// Parse SDP connection information
/// 
/// # Example
/// ```no_run
/// use rvoip_session_core::api::parse_sdp_connection;
/// 
/// let sdp = "v=0\r\nc=IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0 8\r\n";
/// if let Ok(info) = parse_sdp_connection(sdp) {
///     println!("Remote endpoint: {}:{}", info.ip, info.port);
/// }
/// ```
pub fn parse_sdp_connection(sdp: &str) -> Result<SdpInfo> {
    let mut ip = None;
    let mut port = None;
    let mut codecs = Vec::new();
    
    for line in sdp.lines() {
        if line.starts_with("c=IN IP4 ") {
            ip = line.strip_prefix("c=IN IP4 ").map(|s| s.to_string());
        } else if line.starts_with("m=audio ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() > 1 {
                port = parts[1].parse().ok();
            }
            // Extract codec numbers
            if parts.len() > 3 {
                for codec in &parts[3..] {
                    codecs.push(codec.to_string());
                }
            }
        } else if line.starts_with("a=rtpmap:") {
            // Parse codec names
            if let Some(codec_info) = line.strip_prefix("a=rtpmap:") {
                let parts: Vec<&str> = codec_info.split_whitespace().collect();
                if parts.len() >= 2 {
                    // Format: "0 PCMU/8000" -> add "PCMU" to codecs
                    if let Some(codec_name) = parts[1].split('/').next() {
                        codecs.push(codec_name.to_string());
                    }
                }
            }
        }
    }
    
    match (ip, port) {
        (Some(ip), Some(port)) => Ok(SdpInfo { ip, port, codecs }),
        _ => Err(crate::errors::SessionError::MediaIntegration {
            message: "Failed to parse SDP connection information".to_string()
        }),
    }
}

// =============================================================================
// AUDIO STREAMING TYPES
// =============================================================================

/// Re-export AudioFrame from media-core to unify the type across the codebase
/// This eliminates unnecessary conversions and potential issues
pub use rvoip_media_core::types::AudioFrame;

/// Configuration for audio streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioStreamConfig {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Number of channels (1 for mono, 2 for stereo)
    pub channels: u8,
    /// Preferred codec (e.g., "PCMU", "PCMA", "Opus")
    pub codec: String,
    /// Frame size in milliseconds
    pub frame_size_ms: u32,
    /// Enable echo cancellation
    pub enable_aec: bool,
    /// Enable automatic gain control
    pub enable_agc: bool,
    /// Enable voice activity detection
    pub enable_vad: bool,
}

impl Default for AudioStreamConfig {
    fn default() -> Self {
        Self {
            sample_rate: 8000,     // Standard telephony
            channels: 1,           // Mono
            codec: "PCMU".to_string(), // G.711 μ-law
            frame_size_ms: 20,     // 20ms frames
            enable_aec: true,
            enable_agc: true,
            enable_vad: true,
        }
    }
}

impl AudioStreamConfig {
    /// Create a new audio stream configuration
    pub fn new(sample_rate: u32, channels: u8, codec: impl Into<String>) -> Self {
        Self {
            sample_rate,
            channels,
            codec: codec.into(),
            ..Default::default()
        }
    }
    
    /// Get the expected frame size in samples
    pub fn frame_size_samples(&self) -> usize {
        (self.sample_rate as usize * self.frame_size_ms as usize) / 1000
    }
    
    /// Get the expected frame size in bytes (for PCM)
    pub fn frame_size_bytes(&self) -> usize {
        self.frame_size_samples() * self.channels as usize * 2 // 16-bit samples
    }
    
    /// Create a telephony configuration (mono, 8kHz, G.711)
    pub fn telephony() -> Self {
        Self::default()
    }
    
    /// Create a wideband configuration (mono, 16kHz, Opus)
    pub fn wideband() -> Self {
        Self {
            sample_rate: 16000,
            codec: "Opus".to_string(),
            ..Default::default()
        }
    }
    
    /// Create a high-quality configuration (stereo, 48kHz, Opus)
    pub fn high_quality() -> Self {
        Self {
            sample_rate: 48000,
            channels: 2,
            codec: "Opus".to_string(),
            ..Default::default()
        }
    }
}

/// Subscriber for receiving audio frames from a session
/// 
/// This is a handle that allows receiving decoded audio frames from a specific session.
/// Use this to get audio data that should be played on speakers.
#[derive(Debug)]
pub struct AudioFrameSubscriber {
    /// The session ID this subscriber is associated with
    session_id: SessionId,
    /// Receiver for audio frames (async tokio channel for non-blocking operation)
    receiver: tokio::sync::mpsc::Receiver<AudioFrame>,
}

impl AudioFrameSubscriber {
    /// Create a new audio frame subscriber
    pub fn new(session_id: SessionId, receiver: tokio::sync::mpsc::Receiver<AudioFrame>) -> Self {
        Self {
            session_id,
            receiver,
        }
    }
    
    /// Get the session ID this subscriber is associated with
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }
    
    /// Receive the next audio frame (async)
    /// 
    /// # Returns
    /// - `Some(audio_frame)` - Audio frame ready for playback
    /// - `None` - Channel is closed or session ended
    pub async fn recv(&mut self) -> Option<AudioFrame> {
        self.receiver.recv().await
    }
    
    /// Try to receive an audio frame (non-blocking)
    /// 
    /// # Returns
    /// - `Ok(audio_frame)` - Audio frame ready for playback
    /// - `Err(TryRecvError::Empty)` - No frame available right now
    /// - `Err(TryRecvError::Disconnected)` - Channel is closed or session ended
    pub fn try_recv(&mut self) -> std::result::Result<AudioFrame, tokio::sync::mpsc::error::TryRecvError> {
        self.receiver.try_recv()
    }
    
    /// Check if the subscriber is still connected to the session
    pub fn is_connected(&self) -> bool {
        !self.receiver.is_closed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_session_id_creation() {
        let id1 = SessionId::new();
        let id2 = SessionId::new();
        assert_ne!(id1, id2);
        assert!(id1.0.starts_with("sess_"));
    }
    
    #[test]
    fn test_call_state_display() {
        assert_eq!(CallState::Active.to_string(), "Active");
        assert_eq!(CallState::Failed("timeout".to_string()).to_string(), "Failed: timeout");
    }
}

 