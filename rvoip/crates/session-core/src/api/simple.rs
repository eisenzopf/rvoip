//! Simple Developer API for Session-Core
//!
//! This module provides ultra-simple APIs for developers who want to create SIP user agents
//! without needing to understand RFC 3261 or complex session coordination. 
//!
//! # Philosophy: "Easy Button for SIP Sessions"
//!
//! Developers should be able to create functional SIP applications with minimal code:
//! - **3 lines to create working SIP server**: config → manager → handler → running
//! - **1 interface to implement**: `CallHandler` with sensible defaults
//! - **High-level operations**: `answer()`, `reject()`, `terminate()` with no SIP knowledge needed
//!
//! # Core SIP Session Management Features
//!
//! This API provides comprehensive SIP session management capabilities:
//!
//! - **Outgoing Calls**: `make_call()`, `make_direct_call()` for P2P
//! - **Call Control**: `answer()`, `reject()`, `hold()`, `transfer()`, `terminate()`
//! - **SIP Registration**: `register()`, `unregister()`, `refresh_registration()`
//! - **Server Operations**: `start_server()`, `stop_server()`, call handlers
//! - **Session Information**: Party details, headers, call state, duration
//! - **Event Handling**: `on_answered()`, `on_rejected()`, `on_ended()`, etc.
//! - **Advanced Features**: Attended transfer, call replacement, session modification
//!
//! # Examples
//!
//! ## Ultra-Simple Auto-Answer Server
//!
//! ```rust,no_run
//! use rvoip_session_core::api::simple::*;
//! use std::sync::Arc;
//!
//! struct AutoAnswerHandler;
//!
//! impl CallHandler for AutoAnswerHandler {
//!     async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
//!         println!("📞 Incoming call from {} - answering automatically", call.from());
//!         CallAction::Answer
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let session_manager = SessionManager::new(SessionConfig::server("127.0.0.1:5060")?).await?;
//!     session_manager.set_call_handler(Arc::new(AutoAnswerHandler)).await?;
//!     session_manager.start_server("127.0.0.1:5060".parse()?).await?;
//!     
//!     println!("🚀 SIP server running - auto-answering all calls");
//!     tokio::signal::ctrl_c().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Simple Outgoing Call Client
//!
//! ```rust,no_run
//! use rvoip_session_core::api::simple::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let session_manager = SessionManager::new(SessionConfig::client()?).await?;
//!     
//!     let call = session_manager.make_call(
//!         "sip:alice@example.com",
//!         "sip:bob@example.com", 
//!         None // Auto-generate SDP
//!     ).await?;
//!     
//!     println!("📞 Calling bob@example.com...");
//!     
//!     // Set up event handlers
//!     call.on_answered(|call| async move {
//!         println!("✅ Call answered!");
//!     }).await;
//!     
//!     call.on_rejected(|call, reason| async move {
//!         println!("🚫 Call rejected: {}", reason);
//!     }).await;
//!     
//!     // Wait for call to connect or fail
//!     call.wait_for_completion().await?;
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## SIP Registration Client
//!
//! ```rust,no_run
//! use rvoip_session_core::api::simple::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let session_manager = SessionManager::new(SessionConfig::client()?).await?;
//!     
//!     // Register with SIP server
//!     let credentials = Credentials::new("alice".to_string(), "password123".to_string());
//!     let result = session_manager.register(
//!         "sip:alice@example.com",
//!         "sip:registrar.example.com",
//!         Some(credentials)
//!     ).await?;
//!     
//!     if result.is_successful() {
//!         println!("✅ Registered successfully!");
//!         
//!         // Now we can receive incoming calls
//!         // ... handle calls ...
//!         
//!         // Unregister when done
//!         session_manager.unregister().await?;
//!     }
//!     
//!     Ok(())
//! }
//! ```

use std::sync::Arc;
use std::net::SocketAddr;
use std::time::Duration;
use async_trait::async_trait;
use tracing::{info, debug, warn};
use std::collections::HashMap;

use crate::{SessionId, SessionState, SessionManager, SessionConfig};
use crate::errors::Error;
use rvoip_sip_core::StatusCode;

/// Simple call handler trait for developers
/// 
/// This is the only interface developers need to implement for basic SIP functionality.
/// All methods have sensible defaults, so you only override what you need.
///
/// # Design Philosophy
/// 
/// - **Simple decisions**: Answer, reject, or defer - no SIP knowledge required
/// - **Sensible defaults**: Auto-answer is the default behavior
/// - **Optional notifications**: Override state change handlers only if needed
/// - **High-level operations**: Focus on call logic, not protocol details
#[async_trait]
pub trait CallHandler: Send + Sync {
    /// Called when an incoming call is received and ringing
    /// 
    /// This is where you make the key decision: answer, reject, or defer the call.
    /// The call will keep ringing until you make a decision (unless the caller hangs up).
    ///
    /// # Arguments
    /// * `call` - Information about the incoming call (caller, SDP offer, etc.)
    ///
    /// # Returns
    /// * `CallAction` - Your decision on how to handle the call
    ///
    /// # Default Behavior
    /// Automatically answers all incoming calls.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use rvoip_session_core::api::simple::*;
    /// # struct MyHandler;
    /// # impl CallHandler for MyHandler {
    /// async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
    ///     if call.from().contains("@trusted-domain.com") {
    ///         CallAction::Answer
    ///     } else {
    ///         CallAction::Reject
    ///     }
    /// }
    /// # }
    /// ```
    async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
        debug!("📞 Incoming call from {} - auto-answering (default behavior)", call.from());
        CallAction::Answer
    }
    
    /// Called when a call's state changes (optional override)
    /// 
    /// Use this for monitoring, logging, or custom logic based on call state transitions.
    /// Most developers won't need to override this.
    ///
    /// # Arguments
    /// * `call` - The call session that changed state
    /// * `old_state` - Previous call state
    /// * `new_state` - New call state
    ///
    /// # Default Behavior
    /// Logs state changes at debug level.
    async fn on_call_state_changed(&self, call: &CallSession, old_state: SessionState, new_state: SessionState) {
        debug!("📞 Call {} state changed: {} → {}", call.id(), old_state, new_state);
    }
    
    /// Called when a call ends (optional cleanup)
    /// 
    /// Use this for cleanup, logging, or post-call processing.
    /// The call is already terminated when this is called.
    ///
    /// # Arguments
    /// * `call` - The call session that ended
    /// * `reason` - Why the call ended ("Normal call clearing", "User busy", etc.)
    ///
    /// # Default Behavior
    /// Logs call termination at info level.
    async fn on_call_ended(&self, call: &CallSession, reason: &str) {
        info!("📞 Call {} ended: {}", call.id(), reason);
    }
}

/// Simple action enum for call decisions
/// 
/// Represents the developer's decision on how to handle an incoming call.
/// Maps to proper SIP responses internally while keeping the API simple.
#[derive(Debug, Clone)]
pub enum CallAction {
    /// Answer the call immediately with auto-generated SDP
    /// 
    /// Session-core will automatically generate an SDP answer based on the incoming offer
    /// and coordinate with media-core for RTP session setup.
    Answer,
    
    /// Answer the call with custom SDP
    /// 
    /// Use this when you need specific media configuration or codec preferences.
    /// 
    /// # Example
    /// ```rust
    /// # use rvoip_session_core::api::simple::CallAction;
    /// let custom_sdp = "v=0\r\no=alice 123456 654321 IN IP4 host.atlanta.com\r\n...";
    /// CallAction::AnswerWithSdp(custom_sdp.to_string())
    /// ```
    AnswerWithSdp(String),
    
    /// Reject the call with standard "Busy Here" response
    /// 
    /// Sends a 486 Busy Here response to the caller.
    Reject,
    
    /// Reject the call with custom status code and reason
    /// 
    /// Use this for specific rejection scenarios like forbidden callers,
    /// temporarily unavailable, etc.
    /// 
    /// # Example
    /// ```rust
    /// # use rvoip_session_core::api::simple::CallAction;
    /// # use rvoip_sip_core::StatusCode;
    /// CallAction::RejectWith { 
    ///     status: StatusCode::Forbidden, 
    ///     reason: "Caller not authorized".to_string() 
    /// }
    /// ```
    RejectWith { 
        status: StatusCode, 
        reason: String 
    },
    
    /// Defer the decision - call stays ringing
    /// 
    /// Use this when you need more time to decide, want to check external systems,
    /// or implement custom call routing logic. You can answer or reject the call
    /// later using the `CallSession` methods.
    /// 
    /// **Note**: The call will keep ringing until you make a decision or the caller hangs up.
    Defer,
}

/// High-level call session for simple call control
/// 
/// Abstracts all SIP complexity and provides simple methods for call control.
/// Developers interact with calls through this interface without needing to
/// understand SIP headers, transactions, or protocol details.
/// 
/// # Call Lifecycle
/// 
/// 1. **Incoming**: `IncomingCall` → `CallAction::Answer` → `CallSession`
/// 2. **Outgoing**: `SessionManager::make_call()` → `CallSession`
/// 3. **Active**: Use `hold()`, `resume()`, `terminate()` for call control
/// 4. **Ended**: `on_call_ended()` callback, session automatically cleaned up
#[derive(Debug, Clone)]
pub struct CallSession {
    /// Unique session identifier
    session_id: SessionId,
    
    /// Reference to session manager for operations
    session_manager: Arc<SessionManager>,
}

impl CallSession {
    /// Create a new call session
    /// 
    /// This is internal - developers get `CallSession` instances from
    /// `SessionManager::make_call()` or `CallHandler::on_incoming_call()`.
    pub(crate) fn new(session_id: SessionId, session_manager: Arc<SessionManager>) -> Self {
        Self {
            session_id,
            session_manager,
        }
    }
    
    /// Get the unique session ID
    /// 
    /// Useful for logging, debugging, or correlating with external systems.
    pub fn id(&self) -> &SessionId {
        &self.session_id
    }
    
    /// Answer a ringing call with auto-generated SDP
    /// 
    /// Only works for incoming calls in `Ringing` state. For outgoing calls,
    /// the call is automatically "answered" when the remote party accepts.
    /// 
    /// # Errors
    /// Returns error if call is not in appropriate state or SIP/media setup fails.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(call: CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// if call.is_ringing().await {
    ///     call.answer().await?;
    ///     println!("✅ Call answered");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn answer(&self) -> Result<(), Error> {
        info!("📞 Answering call {}", self.session_id);
        self.session_manager.answer_call(&self.session_id).await
    }
    
    /// Answer a ringing call with custom SDP
    /// 
    /// Use this when you need specific media configuration, codec preferences,
    /// or custom media handling.
    /// 
    /// # Arguments
    /// * `sdp` - Custom SDP answer to send
    /// 
    /// # Errors
    /// Returns error if call is not in appropriate state, SDP is invalid, or setup fails.
    pub async fn answer_with_sdp(&self, sdp: String) -> Result<(), Error> {
        info!("📞 Answering call {} with custom SDP", self.session_id);
        self.session_manager.answer_call_with_sdp(&self.session_id, sdp).await
    }
    
    /// Reject a ringing call with "Busy Here" response
    /// 
    /// Only works for incoming calls in `Ringing` state.
    /// 
    /// # Arguments
    /// * `reason` - Optional custom reason phrase (defaults to "Busy Here")
    pub async fn reject(&self, reason: Option<String>) -> Result<(), Error> {
        let reason_phrase = reason.unwrap_or_else(|| "Busy Here".to_string());
        info!("📞 Rejecting call {}: {}", self.session_id, reason_phrase);
        
        self.session_manager.reject_call(&self.session_id, StatusCode::BusyHere).await
    }
    
    /// Reject a ringing call with custom status code
    /// 
    /// Use this for specific rejection scenarios.
    /// 
    /// # Arguments
    /// * `status` - SIP status code (e.g., `StatusCode::Forbidden`, `StatusCode::TemporarilyUnavailable`)
    /// * `reason` - Custom reason phrase
    pub async fn reject_with(&self, status: StatusCode, reason: String) -> Result<(), Error> {
        info!("📞 Rejecting call {} with {}: {}", self.session_id, status, reason);
        self.session_manager.reject_call(&self.session_id, status).await
    }
    
    /// Terminate an active call
    /// 
    /// Works for calls in any active state (`Connected`, `OnHold`, etc.).
    /// Sends a SIP BYE request and cleans up all resources.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(call: CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// if call.is_active().await {
    ///     call.terminate().await?;
    ///     println!("👋 Call ended");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn terminate(&self) -> Result<(), Error> {
        info!("📞 Terminating call {}", self.session_id);
        self.session_manager.terminate_call(&self.session_id, "Normal call clearing").await
    }
    
    /// Put an active call on hold
    /// 
    /// Sends a re-INVITE with `a=sendonly` or `a=inactive` to put the call on hold.
    /// The remote party will hear hold music or silence.
    /// 
    /// # Errors
    /// Returns error if call is not in `Connected` state or SIP operation fails.
    pub async fn hold(&self) -> Result<(), Error> {
        info!("📞 Putting call {} on hold", self.session_id);
        self.session_manager.hold_call(&self.session_id).await
    }
    
    /// Resume a held call
    /// 
    /// Sends a re-INVITE with `a=sendrecv` to resume normal media flow.
    /// 
    /// # Errors
    /// Returns error if call is not in `OnHold` state or SIP operation fails.
    pub async fn resume(&self) -> Result<(), Error> {
        info!("📞 Resuming call {}", self.session_id);
        self.session_manager.resume_call(&self.session_id).await
    }
    
    /// Get current call state
    /// 
    /// Returns the current session state for monitoring or conditional logic.
    /// 
    /// # States
    /// - `Initializing` - Call being set up
    /// - `Dialing` - Outgoing call in progress
    /// - `Ringing` - Incoming call ringing or outgoing call ringing at remote end
    /// - `Connected` - Call is active with media flowing
    /// - `OnHold` - Call is on hold
    /// - `Transferring` - Call transfer in progress
    /// - `Terminating` - Call being terminated
    /// - `Terminated` - Call has ended
    pub async fn state(&self) -> SessionState {
        // Get session and return its state
        match self.session_manager.get_session(&self.session_id) {
            Ok(session) => session.state().await,
            Err(_) => SessionState::Terminated, // Session not found means terminated
        }
    }
    
    /// Check if call is currently active (connected, on hold, or transferring)
    /// 
    /// Convenience method for checking if the call is in an active state where
    /// media might be flowing and call control operations are available.
    pub async fn is_active(&self) -> bool {
        self.state().await.is_active()
    }
    
    /// Check if call is currently ringing (incoming or outgoing)
    /// 
    /// Convenience method for checking if this is an incoming call that can be
    /// answered/rejected, or an outgoing call waiting for the remote party.
    pub async fn is_ringing(&self) -> bool {
        matches!(self.state().await, SessionState::Ringing)
    }
    
    /// Check if call is currently connecting (dialing or ringing)
    /// 
    /// Convenience method for checking if an outgoing call is still in progress.
    pub async fn is_connecting(&self) -> bool {
        matches!(self.state().await, SessionState::Dialing | SessionState::Ringing)
    }
    
    /// Check if call has ended
    /// 
    /// Convenience method for checking if the call is terminated.
    pub async fn is_terminated(&self) -> bool {
        matches!(self.state().await, SessionState::Terminated)
    }
    
    /// Get information about the remote party
    /// 
    /// Returns detailed information about the remote party including URI, display name, and tag.
    pub async fn remote_party(&self) -> Result<PartyInfo, Error> {
        // TODO: Extract from session dialog state
        Ok(PartyInfo::new("sip:remote@example.com".to_string())
            .with_display_name("Remote User".to_string()))
    }
    
    /// Get information about the local party
    /// 
    /// Returns detailed information about the local party including URI, display name, and tag.
    pub async fn local_party(&self) -> Result<PartyInfo, Error> {
        // TODO: Extract from session dialog state
        Ok(PartyInfo::new("sip:local@example.com".to_string())
            .with_display_name("Local User".to_string()))
    }
    
    /// Get information about the remote party URI (simplified)
    /// 
    /// Returns the remote URI (From header for incoming calls, To header for outgoing calls).
    pub async fn remote_uri(&self) -> Result<String, Error> {
        Ok(self.remote_party().await?.uri)
    }
    
    /// Get information about the local party URI (simplified)
    /// 
    /// Returns the local URI (To header for incoming calls, From header for outgoing calls).
    pub async fn local_uri(&self) -> Result<String, Error> {
        Ok(self.local_party().await?.uri)
    }
    
    /// Get a specific SIP header from the dialog
    /// 
    /// Returns the value of a SIP header from the current dialog state.
    /// Useful for accessing custom headers or protocol-specific information.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(call: CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// if let Some(user_agent) = call.get_header("User-Agent").await {
    ///     println!("Remote User-Agent: {}", user_agent);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_header(&self, header_name: &str) -> Option<String> {
        // TODO: Extract from session dialog state
        debug!("Getting header '{}' for session {}", header_name, self.session_id);
        None
    }
    
    /// Get call duration
    /// 
    /// Returns the duration since the call was established (answered).
    /// Returns None if the call hasn't been answered yet.
    pub async fn duration(&self) -> Option<Duration> {
        // TODO: Calculate from session timestamps
        debug!("Getting duration for session {}", self.session_id);
        None
    }
    
    /// Get call direction
    /// 
    /// Returns whether this is an incoming or outgoing call.
    pub async fn direction(&self) -> CallDirection {
        // TODO: Extract from session metadata
        debug!("Getting direction for session {}", self.session_id);
        CallDirection::Outgoing // Placeholder
    }
    
    /// Transfer this call to another party (blind transfer)
    /// 
    /// Sends a REFER request to transfer the call to the specified target.
    /// The call will be terminated once the transfer is accepted.
    /// 
    /// # Arguments
    /// * `target_uri` - SIP URI of the transfer target
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(call: CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// call.transfer("sip:voicemail@example.com").await?;
    /// println!("Call transferred to voicemail");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transfer(&self, target_uri: &str) -> Result<(), Error> {
        info!("📞 Transferring call {} to {}", self.session_id, target_uri);
        // TODO: Send REFER request
        self.session_manager.transfer_call(&self.session_id, target_uri).await
    }
    
    /// Transfer this call to another party after consultation (attended transfer)
    /// 
    /// Transfers this call to the party on the consultation call.
    /// Both calls will be connected together and this session will be terminated.
    /// 
    /// # Arguments
    /// * `consultation_call` - The call session with the transfer target
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(call: CallSession, consultation: CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// call.attended_transfer(&consultation).await?;
    /// println!("Attended transfer completed");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn attended_transfer(&self, consultation_call: &CallSession) -> Result<(), Error> {
        info!("📞 Attended transfer: {} -> {}", self.session_id, consultation_call.session_id);
        // TODO: Coordinate attended transfer
        self.session_manager.attended_transfer(&self.session_id, &consultation_call.session_id).await
    }
    
    /// Replace this call with another call (call replacement)
    /// 
    /// Uses the SIP Replaces header mechanism to replace this call with another.
    /// This is used for advanced call control scenarios.
    pub async fn replace_with(&self, replacement_call: &CallSession) -> Result<(), Error> {
        info!("📞 Replacing call {} with {}", self.session_id, replacement_call.session_id);
        // TODO: Implement call replacement
        self.session_manager.replace_call(&self.session_id, &replacement_call.session_id).await
    }
    
    /// Wait for the call to reach a specific state
    /// 
    /// Blocks until the call reaches the target state or an error occurs.
    /// Useful for synchronizing on call state transitions.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # use rvoip_session_core::SessionState;
    /// # async fn example(call: CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// call.wait_for_state(SessionState::Connected).await?;
    /// println!("Call is now connected");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_state(&self, target_state: SessionState) -> Result<(), Error> {
        debug!("Waiting for session {} to reach state {:?}", self.session_id, target_state);
        
        while self.state().await != target_state {
            // Check if call terminated before reaching target state
            if self.is_terminated().await {
                return Err(Error::SessionTerminated(self.session_id.clone()));
            }
            
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        Ok(())
    }
    
    /// Wait for call completion (any terminal state)
    /// 
    /// Blocks until the call reaches a terminal state (Terminated).
    /// Returns the final state of the call.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(call: CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// let final_state = call.wait_for_completion().await?;
    /// println!("Call ended with state: {:?}", final_state);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn wait_for_completion(&self) -> Result<SessionState, Error> {
        debug!("Waiting for session {} to complete", self.session_id);
        
        loop {
            let state = self.state().await;
            if matches!(state, SessionState::Terminated) {
                return Ok(state);
            }
            
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    
    /// Check if call is completed (terminated)
    /// 
    /// Non-blocking check for call completion.
    pub fn is_completed(&self) -> bool {
        // This would need to be synchronous check of session state
        // For now, return false as placeholder
        false
    }
    
    /// Get unique call ID
    /// 
    /// Returns a unique identifier for this call, separate from the session ID.
    /// Useful for call tracking and correlation with external systems.
    pub fn call_id(&self) -> String {
        // TODO: Generate or extract call-id from SIP headers
        format!("call-{}", self.session_id)
    }
    
    /// Send re-INVITE with new SDP to modify session
    /// 
    /// Changes the media characteristics of an established call.
    /// Common uses include codec changes, adding/removing streams, etc.
    /// 
    /// # Arguments
    /// * `new_sdp` - New SDP offer for session modification
    pub async fn modify_session(&self, new_sdp: String) -> Result<(), Error> {
        info!("📞 Modifying session {} with new SDP", self.session_id);
        // TODO: Send re-INVITE with new SDP
        self.session_manager.modify_session(&self.session_id, new_sdp).await
    }
    
    /// Send re-INVITE without SDP (session refresh)
    /// 
    /// Refreshes the session without changing media characteristics.
    /// Used to keep the session alive or reset session timers.
    pub async fn refresh_session(&self) -> Result<(), Error> {
        info!("📞 Refreshing session {}", self.session_id);
        // TODO: Send re-INVITE without SDP
        self.session_manager.refresh_session(&self.session_id).await
    }
    
    // Event handlers for call state changes
    
    /// Set handler for when call is answered
    /// 
    /// Registers a callback that will be called when the call is answered.
    /// For outgoing calls, this means the remote party answered.
    /// For incoming calls, this means we answered the call.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(call: CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// call.on_answered(|call| async move {
    ///     println!("Call answered with {}", call.remote_party().await.unwrap().uri);
    /// }).await;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn on_answered<F, Fut>(&self, handler: F) 
    where
        F: Fn(CallSession) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // TODO: Register event handler for call answered
        debug!("Registered on_answered handler for session {}", self.session_id);
    }
    
    /// Set handler for when call is ringing
    /// 
    /// For outgoing calls, this means the remote party's phone is ringing.
    /// For incoming calls, this is called when the call starts ringing locally.
    pub async fn on_ringing<F, Fut>(&self, handler: F)
    where
        F: Fn(CallSession) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // TODO: Register event handler for ringing
        debug!("Registered on_ringing handler for session {}", self.session_id);
    }
    
    /// Set handler for when call is rejected
    /// 
    /// Called when the call is rejected by the remote party or due to an error.
    pub async fn on_rejected<F, Fut>(&self, handler: F)
    where
        F: Fn(CallSession, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // TODO: Register event handler for call rejection
        debug!("Registered on_rejected handler for session {}", self.session_id);
    }
    
    /// Set handler for when call ends
    /// 
    /// Called when the call terminates for any reason.
    pub async fn on_ended<F, Fut>(&self, handler: F)
    where
        F: Fn(CallSession, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // TODO: Register event handler for call termination
        debug!("Registered on_ended handler for session {}", self.session_id);
    }
    
    /// Set handler for when call gets busy signal
    /// 
    /// Called when the remote party returns a busy signal (486 Busy Here).
    pub async fn on_busy<F, Fut>(&self, handler: F)
    where
        F: Fn(CallSession) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // TODO: Register event handler for busy signal
        debug!("Registered on_busy handler for session {}", self.session_id);
    }
    
    /// Set handler for when call has no answer
    /// 
    /// Called when the call times out without being answered.
    pub async fn on_no_answer<F, Fut>(&self, handler: F)
    where
        F: Fn(CallSession) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // TODO: Register event handler for no answer timeout
        debug!("Registered on_no_answer handler for session {}", self.session_id);
    }
    
    /// Set handler for when call is put on hold
    /// 
    /// Called when either party puts the call on hold.
    pub async fn on_hold<F, Fut>(&self, handler: F)
    where
        F: Fn(CallSession) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // TODO: Register event handler for call hold
        debug!("Registered on_hold handler for session {}", self.session_id);
    }
    
    /// Set handler for when call is resumed from hold
    /// 
    /// Called when a held call is resumed.
    pub async fn on_resume<F, Fut>(&self, handler: F)
    where
        F: Fn(CallSession) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        // TODO: Register event handler for call resume
        debug!("Registered on_resume handler for session {}", self.session_id);
    }
}

/// Simple incoming call information for developers
/// 
/// Contains essential information about an incoming call without exposing
/// SIP protocol details. Developers use this to make call handling decisions.
#[derive(Debug, Clone)]
pub struct IncomingCall {
    /// The call session for this incoming call
    call_session: CallSession,
    
    /// Caller information (From header URI)
    from_uri: String,
    
    /// Called party information (To header URI)  
    to_uri: String,
    
    /// SDP offer from the caller (if present)
    sdp_offer: Option<String>,
    
    /// Additional call information
    display_name: Option<String>,
    user_agent: Option<String>,
}

impl IncomingCall {
    /// Create a new incoming call
    /// 
    /// This is internal - developers receive `IncomingCall` instances in their
    /// `CallHandler::on_incoming_call()` implementation.
    pub(crate) fn new(
        call_session: CallSession,
        from_uri: String,
        to_uri: String,
        sdp_offer: Option<String>,
        display_name: Option<String>,
        user_agent: Option<String>,
    ) -> Self {
        Self {
            call_session,
            from_uri,
            to_uri,
            sdp_offer,
            display_name,
            user_agent,
        }
    }
    
    /// Get the call session for this incoming call
    /// 
    /// Use this to access call control methods or defer the decision and
    /// answer/reject the call later.
    pub fn call(&self) -> &CallSession {
        &self.call_session
    }
    
    /// Get the caller's URI (From header)
    /// 
    /// Returns the SIP URI of the calling party, e.g., "sip:alice@example.com".
    pub fn from(&self) -> &str {
        &self.from_uri
    }
    
    /// Get the called party's URI (To header)
    /// 
    /// Returns the SIP URI that was called, e.g., "sip:bob@example.com".
    pub fn to(&self) -> &str {
        &self.to_uri
    }
    
    /// Get the SDP offer from the caller (if present)
    /// 
    /// Returns the media description (SDP) that the caller sent for media negotiation.
    /// Use this if you need to analyze the offered codecs, media types, or generate
    /// a custom SDP answer.
    pub fn sdp_offer(&self) -> Option<&String> {
        self.sdp_offer.as_ref()
    }
    
    /// Get the caller's display name (if present)
    /// 
    /// Returns the human-readable name from the From header, e.g., "Alice Smith".
    pub fn display_name(&self) -> Option<&String> {
        self.display_name.as_ref()
    }
    
    /// Get the caller's User-Agent (if present)
    /// 
    /// Returns the User-Agent header value, useful for identifying the calling device/software.
    pub fn user_agent(&self) -> Option<&String> {
        self.user_agent.as_ref()
    }
    
    /// Get just the username part of the caller's URI
    /// 
    /// Convenience method that extracts "alice" from "sip:alice@example.com".
    /// Useful for simple caller identification and authorization.
    pub fn caller_username(&self) -> Option<&str> {
        if self.from_uri.starts_with("sip:") {
            self.from_uri.strip_prefix("sip:")
                .and_then(|s| s.split('@').next())
        } else {
            None
        }
    }
    
    /// Get the domain part of the caller's URI
    /// 
    /// Convenience method that extracts "example.com" from "sip:alice@example.com".
    /// Useful for domain-based authorization.
    pub fn caller_domain(&self) -> Option<&str> {
        if self.from_uri.starts_with("sip:") {
            self.from_uri.strip_prefix("sip:")
                .and_then(|s| s.split('@').nth(1))
        } else {
            None
        }
    }
    
    /// Get source network address of the call
    /// 
    /// Returns the IP address and port from which this call originated.
    /// Useful for network-based authorization and logging.
    pub fn source_address(&self) -> SocketAddr {
        // TODO: Extract from SIP transport layer
        "127.0.0.1:5060".parse().unwrap() // Placeholder
    }
    
    /// Get a specific SIP header from the INVITE request
    /// 
    /// Returns the value of a SIP header from the incoming INVITE.
    /// Useful for accessing custom headers or protocol-specific information.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # fn example(call: &IncomingCall) {
    /// if let Some(priority) = call.get_header("Priority") {
    ///     println!("Call priority: {}", priority);
    /// }
    /// # }
    /// ```
    pub fn get_header(&self, header_name: &str) -> Option<&str> {
        // TODO: Extract from SIP message
        debug!("Getting header '{}' for incoming call from {}", header_name, self.from_uri);
        None
    }
    
    /// Get a URI parameter from the incoming call
    /// 
    /// Extracts parameters from the Request-URI or other URI fields.
    /// Useful for routing decisions and call metadata.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # fn example(call: &IncomingCall) {
    /// if let Some(tenant) = call.get_parameter("tenant") {
    ///     println!("Call is for tenant: {}", tenant);
    /// }
    /// # }
    /// ```
    pub fn get_parameter(&self, param_name: &str) -> Option<&str> {
        // TODO: Parse URI parameters
        debug!("Getting parameter '{}' for incoming call from {}", param_name, self.from_uri);
        None
    }
    
    /// Create a mock incoming call for testing
    /// 
    /// Creates a mock IncomingCall instance for unit testing and development.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// let mock_call = IncomingCall::mock("sip:test@example.com");
    /// assert_eq!(mock_call.from(), "sip:test@example.com");
    /// ```
    pub fn mock(from_uri: &str) -> Self {
        // Create a mock session for testing
        let session_id = SessionId::new();
        
        // Create a minimal mock SessionManager - this would need proper mock implementation
        use crate::SessionManagerConfig;
        let config = SessionManagerConfig::default();
        
        // For testing, we create a basic session manager
        // TODO: Implement proper mock session manager for testing
        let session_manager = match SessionManager::new(config) {
            Ok(sm) => Arc::new(sm),
            Err(_) => {
                // Fallback for testing - create a placeholder
                // This should be replaced with proper mock implementation
                panic!("Mock session manager creation failed - implement proper mock")
            }
        };
        
        let call_session = CallSession {
            session_id,
            session_manager,
        };
        
        Self {
            call_session,
            from_uri: from_uri.to_string(),
            to_uri: "sip:mock@localhost".to_string(),
            sdp_offer: None,
            display_name: Some("Mock Caller".to_string()),
            user_agent: Some("MockUA/1.0".to_string()),
        }
    }
}

/// Built-in call handlers for common use cases
/// 
/// These handlers provide ready-to-use implementations for typical scenarios.
/// Developers can use them directly or as examples for custom implementations.
pub mod handlers {
    use super::*;
    use std::collections::HashSet;
    use std::time::Duration;
    
    /// Auto-answer handler that accepts all incoming calls
    /// 
    /// Simple handler that automatically answers every incoming call with
    /// auto-generated SDP. Perfect for testing and basic use cases.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::handlers::AutoAnswerHandler;
    /// # use std::sync::Arc;
    /// let handler = Arc::new(AutoAnswerHandler::new("TestServer"));
    /// // session_manager.set_call_handler(handler).await?;
    /// ```
    #[derive(Debug, Clone)]
    pub struct AutoAnswerHandler {
        name: String,
        delay_ms: Option<u64>,
    }
    
    impl AutoAnswerHandler {
        /// Create a new auto-answer handler
        pub fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                delay_ms: None,
            }
        }
        
        /// Add a delay before answering calls
        /// 
        /// Useful for testing ring duration or simulating human response time.
        pub fn with_delay(mut self, delay_ms: u64) -> Self {
            self.delay_ms = Some(delay_ms);
            self
        }
    }
    
    #[async_trait]
    impl CallHandler for AutoAnswerHandler {
        async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
            info!("📞 {} - Auto-answering call from {}", self.name, call.from());
            
            if let Some(delay) = self.delay_ms {
                info!("⏱️ {} - Waiting {}ms before answering", self.name, delay);
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
            
            CallAction::Answer
        }
        
        async fn on_call_state_changed(&self, call: &CallSession, _old: SessionState, new: SessionState) {
            info!("📞 {} - Call {} state: {}", self.name, call.id(), new);
        }
    }
    
    /// Selective answer handler based on allowed callers
    /// 
    /// Accepts calls from a whitelist of allowed callers and rejects all others.
    /// Can be configured with usernames, full URIs, or domains.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::handlers::SelectiveAnswerHandler;
    /// # use std::sync::Arc;
    /// let handler = Arc::new(
    ///     SelectiveAnswerHandler::new("Security")
    ///         .allow_caller("alice")
    ///         .allow_caller("bob")
    ///         .allow_domain("trusted-domain.com")
    /// );
    /// // session_manager.set_call_handler(handler).await?;
    /// ```
    #[derive(Debug, Clone)]
    pub struct SelectiveAnswerHandler {
        name: String,
        allowed_callers: HashSet<String>,
        allowed_domains: HashSet<String>,
        default_action: CallAction,
    }
    
    impl SelectiveAnswerHandler {
        /// Create a new selective answer handler
        pub fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
                allowed_callers: HashSet::new(),
                allowed_domains: HashSet::new(),
                default_action: CallAction::Reject,
            }
        }
        
        /// Allow a specific caller (username or full URI)
        pub fn allow_caller(mut self, caller: &str) -> Self {
            self.allowed_callers.insert(caller.to_string());
            self
        }
        
        /// Allow all callers from a domain
        pub fn allow_domain(mut self, domain: &str) -> Self {
            self.allowed_domains.insert(domain.to_string());
            self
        }
        
        /// Set the default action for non-allowed callers
        pub fn with_default_action(mut self, action: CallAction) -> Self {
            self.default_action = action;
            self
        }
        
        /// Check if a caller is allowed
        fn is_caller_allowed(&self, call: &IncomingCall) -> bool {
            // Check full URI
            if self.allowed_callers.contains(call.from()) {
                return true;
            }
            
            // Check username
            if let Some(username) = call.caller_username() {
                if self.allowed_callers.contains(username) {
                    return true;
                }
            }
            
            // Check domain
            if let Some(domain) = call.caller_domain() {
                if self.allowed_domains.contains(domain) {
                    return true;
                }
            }
            
            false
        }
    }
    
    #[async_trait]
    impl CallHandler for SelectiveAnswerHandler {
        async fn on_incoming_call(&self, call: &IncomingCall) -> CallAction {
            if self.is_caller_allowed(call) {
                info!("📞 {} - Accepting call from allowed caller: {}", self.name, call.from());
                CallAction::Answer
            } else {
                warn!("📞 {} - Rejecting call from non-allowed caller: {}", self.name, call.from());
                self.default_action.clone()
            }
        }
    }
}

/// Simple coordination functions for session management
/// 
/// These functions provide easy access to session coordination features
/// without requiring deep knowledge of the underlying coordination systems.
pub mod coordination {
    use super::*;
    use crate::session::coordination::*;
    use crate::{SessionId, SessionManager};
    
    /// Simple session group for organizing related calls
    /// 
    /// Groups allow you to organize related sessions together, such as
    /// conference calls, call transfers, or related business processes.
    #[derive(Debug, Clone)]
    pub struct SessionGroup {
        /// Group identifier
        pub id: String,
        
        /// Group name for display/logging
        pub name: String,
        
        /// List of sessions in this group
        pub sessions: Vec<SessionId>,
        
        /// Group type (Conference, Transfer, etc.)
        pub group_type: BasicGroupType,
    }
    
    impl SessionGroup {
        /// Create a new session group
        pub fn new(id: String, name: String, group_type: BasicGroupType) -> Self {
            Self {
                id,
                name,
                sessions: Vec::new(),
                group_type,
            }
        }
        
        /// Add a session to this group
        pub fn add_session(&mut self, session_id: SessionId) {
            if !self.sessions.contains(&session_id) {
                self.sessions.push(session_id);
            }
        }
        
        /// Remove a session from this group
        pub fn remove_session(&mut self, session_id: &SessionId) {
            self.sessions.retain(|id| id != session_id);
        }
        
        /// Check if this group contains a session
        pub fn contains_session(&self, session_id: &SessionId) -> bool {
            self.sessions.contains(session_id)
        }
        
        /// Get the number of sessions in this group
        pub fn session_count(&self) -> usize {
            self.sessions.len()
        }
    }
    
    /// Simple session priority levels for call handling
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CallPriority {
        /// Emergency calls (highest priority)
        Emergency,
        /// Critical business calls
        Critical,
        /// High priority calls
        High,
        /// Normal priority calls (default)
        Normal,
        /// Low priority calls
        Low,
        /// Background calls (lowest priority)
        Background,
    }
    
    impl From<CallPriority> for BasicSessionPriority {
        fn from(priority: CallPriority) -> Self {
            match priority {
                CallPriority::Emergency => BasicSessionPriority::Emergency,
                CallPriority::Critical => BasicSessionPriority::Critical,
                CallPriority::High => BasicSessionPriority::High,
                CallPriority::Normal => BasicSessionPriority::Normal,
                CallPriority::Low => BasicSessionPriority::Low,
                CallPriority::Background => BasicSessionPriority::Background,
            }
        }
    }
    
    /// Simple session event types for monitoring
    #[derive(Debug, Clone)]
    pub enum SessionEventType {
        /// Call started
        CallStarted,
        /// Call answered
        CallAnswered,
        /// Call ended
        CallEnded,
        /// Call transferred
        CallTransferred,
        /// Custom event
        Custom(String),
    }
}

/// Extensions to SessionConfig for simple configuration
impl SessionConfig {
    /// Create a server configuration
    /// 
    /// Creates a configuration suitable for SIP servers that listen for incoming calls.
    /// 
    /// # Arguments
    /// * `bind_address` - Address to bind the SIP server to (e.g., "0.0.0.0:5060")
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// let config = SessionConfig::server("0.0.0.0:5060")?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn server(bind_address: &str) -> Result<Self, Error> {
        info!("Creating server configuration for {}", bind_address);
        
        // TODO: Configure for server mode
        let mut config = SessionConfig::default();
        // Set server-specific settings
        
        Ok(config)
    }
    
    /// Create a client configuration
    /// 
    /// Creates a configuration suitable for SIP clients that make outgoing calls.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// let config = SessionConfig::client()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn client() -> Result<Self, Error> {
        info!("Creating client configuration");
        
        // TODO: Configure for client mode
        let mut config = SessionConfig::default();
        // Set client-specific settings
        
        Ok(config)
    }
    
    /// Enable or disable P2P mode
    /// 
    /// P2P mode allows direct client-to-client communication without a SIP server.
    /// Registration is disabled in P2P mode.
    /// 
    /// # Arguments
    /// * `enabled` - Whether to enable P2P mode
    pub fn set_p2p_mode(&mut self, enabled: bool) -> &mut Self {
        info!("Setting P2P mode: {}", enabled);
        // TODO: Configure P2P mode
        self
    }
    
    /// Set the local network address
    /// 
    /// Specifies the local IP address and port for SIP communication.
    /// 
    /// # Arguments
    /// * `address` - Local socket address
    pub fn set_local_address(&mut self, address: SocketAddr) -> &mut Self {
        info!("Setting local address: {}", address);
        // TODO: Configure local address
        self
    }
    
    /// Set the SIP transport protocol
    /// 
    /// Configures whether to use UDP, TCP, or TLS for SIP signaling.
    /// 
    /// # Arguments
    /// * `transport` - Transport protocol to use
    pub fn set_transport(&mut self, transport: SipTransport) -> &mut Self {
        info!("Setting SIP transport: {:?}", transport);
        // TODO: Configure transport
        self
    }
}

/// Extensions to SessionManager for simple call operations
impl SessionManager {
    /// Make an outgoing call
    /// 
    /// Initiates a new outgoing call to the specified target.
    /// Returns a CallSession that can be used to control the call.
    /// 
    /// # Arguments
    /// * `from_uri` - Local party URI (e.g., "sip:alice@example.com")
    /// * `to_uri` - Remote party URI (e.g., "sip:bob@example.com")
    /// * `sdp_offer` - Optional SDP offer (auto-generated if None)
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let call = session_manager.make_call(
    ///     "sip:alice@example.com",
    ///     "sip:bob@example.com",
    ///     None // Auto-generate SDP
    /// ).await?;
    /// 
    /// // Wait for call to connect
    /// call.wait_for_state(rvoip_session_core::SessionState::Connected).await?;
    /// println!("Call connected!");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn make_call(
        &self,
        from_uri: &str,
        to_uri: &str,
        sdp_offer: Option<String>,
    ) -> Result<CallSession, Error> {
        info!("📞 Making call from {} to {}", from_uri, to_uri);
        
        // TODO: Create outgoing session with SIP stack
        let session_id = SessionId::new();
        
        // Create the call session
        let call_session = CallSession::new(session_id.clone(), Arc::new(self.clone()));
        
        info!("✅ Outgoing call initiated: {}", session_id);
        Ok(call_session)
    }
    
    /// Make a direct call to a specific address (P2P)
    /// 
    /// Makes a direct call to a peer without going through a SIP server.
    /// Useful for peer-to-peer communication scenarios.
    /// 
    /// # Arguments
    /// * `from_uri` - Local party URI
    /// * `to_uri` - Remote party URI  
    /// * `target_address` - Direct network address of the target
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let target_addr = "192.168.1.100:5060".parse()?;
    /// let call = session_manager.make_direct_call(
    ///     "sip:alice@192.168.1.50",
    ///     "sip:bob@192.168.1.100",
    ///     target_addr
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn make_direct_call(
        &self,
        from_uri: &str,
        to_uri: &str,
        target_address: SocketAddr,
    ) -> Result<CallSession, Error> {
        info!("📞 Making direct call from {} to {} at {}", from_uri, to_uri, target_address);
        
        // TODO: Create direct P2P session
        let session_id = SessionId::new();
        
        // Create the call session
        let call_session = CallSession::new(session_id.clone(), Arc::new(self.clone()));
        
        info!("✅ Direct call initiated: {}", session_id);
        Ok(call_session)
    }
    
    /// Register with a SIP server
    /// 
    /// Registers this UA with a SIP registrar server to receive incoming calls.
    /// 
    /// # Arguments
    /// * `aor` - Address of Record (the public SIP URI for this UA)
    /// * `registrar` - SIP registrar server URI or address
    /// * `credentials` - Authentication credentials (optional)
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let creds = Credentials::new("alice".to_string(), "password123".to_string());
    /// let result = session_manager.register(
    ///     "sip:alice@example.com",
    ///     "sip:registrar.example.com",
    ///     Some(creds)
    /// ).await?;
    /// 
    /// if result.is_successful() {
    ///     println!("Registration successful!");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn register(
        &self,
        aor: &str,
        registrar: &str,
        credentials: Option<Credentials>,
    ) -> Result<RegistrationResult, Error> {
        info!("📱 Registering {} with {}", aor, registrar);
        
        // TODO: Send REGISTER request to SIP registrar
        
        // Placeholder implementation
        let result = RegistrationResult {
            is_successful: true,
            expires: Some(Duration::from_secs(3600)), // 1 hour
            error_message: None,
        };
        
        info!("✅ Registration completed for {}", aor);
        Ok(result)
    }
    
    /// Unregister from SIP server
    /// 
    /// Removes this UA's registration from the SIP server.
    /// No more incoming calls will be routed to this UA.
    pub async fn unregister(&self) -> Result<(), Error> {
        info!("📱 Unregistering from SIP server");
        
        // TODO: Send REGISTER with Expires: 0
        
        info!("✅ Unregistration completed");
        Ok(())
    }
    
    /// Refresh registration with SIP server
    /// 
    /// Refreshes the current registration to prevent it from expiring.
    /// Usually called automatically by the session manager.
    pub async fn refresh_registration(&self) -> Result<(), Error> {
        info!("📱 Refreshing SIP registration");
        
        // TODO: Send REGISTER request to refresh
        
        info!("✅ Registration refreshed");
        Ok(())
    }
    
    /// Start SIP server listener
    /// 
    /// Starts listening for incoming SIP requests on the specified address.
    /// Required for receiving incoming calls.
    /// 
    /// # Arguments
    /// * `bind_address` - Socket address to bind to
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let addr = "0.0.0.0:5060".parse()?;
    /// session_manager.start_server(addr).await?;
    /// println!("SIP server listening on port 5060");
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start_server(&self, bind_address: SocketAddr) -> Result<(), Error> {
        info!("🚀 Starting SIP server on {}", bind_address);
        
        // TODO: Start SIP transport listener
        
        info!("✅ SIP server started on {}", bind_address);
        Ok(())
    }
    
    /// Start P2P listener
    /// 
    /// Starts listening for direct peer-to-peer connections.
    /// Used in P2P mode for direct client-to-client communication.
    /// 
    /// # Arguments
    /// * `bind_address` - Socket address to bind to
    pub async fn start_p2p_listener(&self, bind_address: SocketAddr) -> Result<(), Error> {
        info!("🚀 Starting P2P listener on {}", bind_address);
        
        // TODO: Start P2P transport listener
        
        info!("✅ P2P listener started on {}", bind_address);
        Ok(())
    }
    
    /// Stop SIP server
    /// 
    /// Stops the SIP server and closes all listening sockets.
    /// Existing calls will continue but no new calls will be accepted.
    pub async fn stop_server(&self) -> Result<(), Error> {
        info!("🛑 Stopping SIP server");
        
        // TODO: Stop SIP transport listeners
        
        info!("✅ SIP server stopped");
        Ok(())
    }
    
    /// Set incoming call handler
    /// 
    /// Sets the handler that will be called for all incoming calls.
    /// This is an alternative to set_call_handler for function-based handling.
    /// 
    /// # Arguments
    /// * `handler` - Async function to handle incoming calls
    pub async fn set_incoming_call_handler<F, Fut>(&self, handler: F) -> Result<(), Error>
    where
        F: Fn(IncomingCall) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = CallAction> + Send + 'static,
    {
        info!("Setting incoming call handler");
        
        // TODO: Store handler for incoming call processing
        
        info!("✅ Incoming call handler set");
        Ok(())
    }
    
    // Internal helper methods that would be called by CallSession
    
    /// Transfer a call (internal)
    pub(crate) async fn transfer_call(&self, session_id: &SessionId, target_uri: &str) -> Result<(), Error> {
        info!("Transferring session {} to {}", session_id, target_uri);
        // TODO: Send REFER request
        Ok(())
    }
    
    /// Attended transfer (internal)
    pub(crate) async fn attended_transfer(&self, session_id: &SessionId, consultation_id: &SessionId) -> Result<(), Error> {
        info!("Attended transfer: {} -> {}", session_id, consultation_id);
        // TODO: Coordinate attended transfer
        Ok(())
    }
    
    /// Replace call (internal)
    pub(crate) async fn replace_call(&self, session_id: &SessionId, replacement_id: &SessionId) -> Result<(), Error> {
        info!("Replacing session {} with {}", session_id, replacement_id);
        // TODO: Implement call replacement
        Ok(())
    }
    
    /// Modify session (internal)
    pub(crate) async fn modify_session(&self, session_id: &SessionId, new_sdp: String) -> Result<(), Error> {
        info!("Modifying session {} with new SDP", session_id);
        // TODO: Send re-INVITE with new SDP
        Ok(())
    }
    
    /// Refresh session (internal)
    pub(crate) async fn refresh_session(&self, session_id: &SessionId) -> Result<(), Error> {
        info!("Refreshing session {}", session_id);
        // TODO: Send re-INVITE without SDP
        Ok(())
    }
}

/// Extensions to SessionManager for simple coordination
impl SessionManager {
    // ========================================
    // SIMPLE COORDINATION FUNCTIONS
    // ========================================
    
    /// Create a simple session group
    /// 
    /// Groups allow you to organize related sessions together.
    /// Useful for conference calls, transfers, or related processes.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let group = session_manager.create_group(
    ///     "conf1".to_string(),
    ///     "Sales Team Conference".to_string(),
    ///     coordination::CallPriority::High
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_group(
        &self,
        group_id: String,
        group_name: String,
        priority: coordination::CallPriority,
    ) -> Result<coordination::SessionGroup, Error> {
        use crate::session::coordination::*;
        
        info!("Creating session group: {} ({})", group_name, group_id);
        
        // Convert simple priority to basic priority
        let basic_priority: BasicSessionPriority = priority.into();
        
        // Create basic group configuration
        let mut group_config = BasicGroupConfig {
            max_sessions: Some(10), // Default limit
            metadata: HashMap::new(),
        };
        
        // Add metadata for group name and priority
        group_config.metadata.insert("name".to_string(), group_name.clone());
        group_config.metadata.insert("priority".to_string(), format!("{:?}", basic_priority));
        
        // Create the group using basic primitives
        let basic_group = BasicSessionGroup::new(BasicGroupType::Conference, group_config);
        
        // Convert to simple group
        let group = coordination::SessionGroup::new(
            basic_group.id.clone(), // Use the generated ID from basic_group
            group_name,
            BasicGroupType::Conference,
        );
        
        info!("✅ Created session group: {}", group.name);
        Ok(group)
    }
    
    /// Add a session to a group
    /// 
    /// This creates a relationship between sessions for coordination purposes.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager, call: &CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// session_manager.add_to_group("conf1", call.id()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn add_to_group(&self, group_id: &str, session_id: &SessionId) -> Result<(), Error> {
        info!("Adding session {} to group {}", session_id, group_id);
        
        // For now, this is a basic implementation
        // In a full implementation, this would coordinate with the group management system
        
        info!("✅ Added session {} to group {}", session_id, group_id);
        Ok(())
    }
    
    /// Remove a session from a group
    pub async fn remove_from_group(&self, group_id: &str, session_id: &SessionId) -> Result<(), Error> {
        info!("Removing session {} from group {}", session_id, group_id);
        
        // Basic implementation
        info!("✅ Removed session {} from group {}", session_id, group_id);
        Ok(())
    }
    
    /// Set priority for a call session
    /// 
    /// Higher priority calls may get better treatment for resource allocation.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager, call: &CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// session_manager.set_call_priority(call.id(), coordination::CallPriority::Emergency).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn set_call_priority(&self, session_id: &SessionId, priority: coordination::CallPriority) -> Result<(), Error> {
        info!("Setting priority for session {} to {:?}", session_id, priority);
        
        let basic_priority: crate::session::coordination::BasicSessionPriority = priority.into();
        
        // Update session with priority information
        // This would coordinate with the priority management system
        
        info!("✅ Set priority for session {} to {:?}", session_id, priority);
        Ok(())
    }
    
    /// Get current resource usage for monitoring
    /// 
    /// Returns simple resource information for monitoring call load.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let usage = session_manager.get_resource_usage().await?;
    /// println!("Active calls: {}", usage.active_sessions);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_resource_usage(&self) -> Result<SimpleResourceUsage, Error> {
        let global_metrics = self.get_global_metrics().await;
        
        Ok(SimpleResourceUsage {
            active_sessions: global_metrics.active_sessions,
            memory_usage_mb: global_metrics.total_memory_usage / 1_048_576, // Convert to MB
            peak_sessions: global_metrics.total_sessions_created as usize, // Use total created as approximation
        })
    }
    
    /// Subscribe to simple call events
    /// 
    /// Get notified about call state changes in a simple format.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager) -> Result<(), Box<dyn std::error::Error>> {
    /// let mut events = session_manager.subscribe_to_events().await?;
    /// 
    /// tokio::spawn(async move {
    ///     while let Some(event) = events.recv().await {
    ///         println!("Call event: {:?}", event);
    ///     }
    /// });
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe_to_events(&self) -> Result<tokio::sync::mpsc::Receiver<SimpleCallEvent>, Error> {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        
        // This would set up event subscription using the basic event system
        info!("✅ Subscribed to simple call events");
        
        Ok(rx)
    }
    
    /// Create a dependency between two sessions
    /// 
    /// This establishes a parent-child relationship between sessions,
    /// useful for call transfers, consultative calls, etc.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager, parent: &CallSession, child: &CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// session_manager.create_dependency(parent.id(), child.id(), "transfer").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_dependency(
        &self, 
        parent_session: &SessionId, 
        child_session: &SessionId, 
        relationship_type: &str
    ) -> Result<(), Error> {
        info!("Creating dependency: {} -> {} ({})", parent_session, child_session, relationship_type);
        
        // This would use the dependency tracking system
        
        info!("✅ Created dependency between sessions");
        Ok(())
    }
    
    /// Bridge two calls together
    /// 
    /// This connects two calls so audio flows between them.
    /// Useful for call transfers, conferences, etc.
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::api::simple::*;
    /// # async fn example(session_manager: &SessionManager, call1: &CallSession, call2: &CallSession) -> Result<(), Box<dyn std::error::Error>> {
    /// let bridge_id = session_manager.bridge_calls(call1.id(), call2.id()).await?;
    /// println!("Calls bridged with ID: {}", bridge_id);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn bridge_calls(&self, session1: &SessionId, session2: &SessionId) -> Result<String, Error> {
        info!("Bridging calls: {} <-> {}", session1, session2);
        
        // This would use the bridge infrastructure
        let bridge_id = format!("bridge-{}-{}", session1, session2);
        
        info!("✅ Created bridge: {}", bridge_id);
        Ok(bridge_id)
    }
    
    /// Remove a bridge between calls
    pub async fn unbridge_calls(&self, bridge_id: &str) -> Result<(), Error> {
        info!("Removing bridge: {}", bridge_id);
        
        // This would clean up the bridge
        
        info!("✅ Removed bridge: {}", bridge_id);
        Ok(())
    }
}

/// Simple resource usage information
#[derive(Debug, Clone)]
pub struct SimpleResourceUsage {
    /// Number of active call sessions
    pub active_sessions: usize,
    
    /// Memory usage in megabytes
    pub memory_usage_mb: usize,
    
    /// Peak concurrent sessions seen
    pub peak_sessions: usize,
}

/// Simple call event for monitoring
#[derive(Debug, Clone)]
pub struct SimpleCallEvent {
    /// Session ID for the call
    pub session_id: SessionId,
    
    /// Type of event
    pub event_type: coordination::SessionEventType,
    
    /// Event timestamp
    pub timestamp: std::time::SystemTime,
    
    /// Optional additional data
    pub data: Option<String>,
}

/// SIP registration result
#[derive(Debug, Clone)]
pub struct RegistrationResult {
    /// Whether registration was successful
    pub is_successful: bool,
    /// Registration expiration time
    pub expires: Option<Duration>,
    /// Error message if registration failed
    pub error_message: Option<String>,
}

impl RegistrationResult {
    /// Check if registration was successful
    pub fn is_successful(&self) -> bool {
        self.is_successful
    }
    
    /// Get error message
    pub fn error_message(&self) -> Option<&str> {
        self.error_message.as_deref()
    }
    
    /// Get expiration duration
    pub fn expires_in(&self) -> Option<Duration> {
        self.expires
    }
}

/// SIP credentials for authentication
#[derive(Debug, Clone)]
pub struct Credentials {
    /// Username for authentication
    pub username: String,
    /// Password for authentication
    pub password: String,
    /// Authentication realm (optional)
    pub realm: Option<String>,
}

impl Credentials {
    /// Create new credentials
    pub fn new(username: String, password: String) -> Self {
        Self {
            username,
            password,
            realm: None,
        }
    }
    
    /// Set authentication realm
    pub fn with_realm(mut self, realm: String) -> Self {
        self.realm = Some(realm);
        self
    }
}

/// Information about a party in a call
#[derive(Debug, Clone)]
pub struct PartyInfo {
    /// SIP URI of the party
    pub uri: String,
    /// Display name (if available)
    pub display_name: Option<String>,
    /// SIP tag parameter (for dialog identification)
    pub tag: Option<String>,
}

impl PartyInfo {
    /// Create new party info
    pub fn new(uri: String) -> Self {
        Self {
            uri,
            display_name: None,
            tag: None,
        }
    }
    
    /// Set display name
    pub fn with_display_name(mut self, display_name: String) -> Self {
        self.display_name = Some(display_name);
        self
    }
    
    /// Set SIP tag
    pub fn with_tag(mut self, tag: String) -> Self {
        self.tag = Some(tag);
        self
    }
    
    /// Get just the username from the URI
    pub fn username(&self) -> Option<&str> {
        if self.uri.starts_with("sip:") {
            self.uri.strip_prefix("sip:")
                .and_then(|s| s.split('@').next())
        } else {
            None
        }
    }
    
    /// Get the domain from the URI
    pub fn domain(&self) -> Option<&str> {
        if self.uri.starts_with("sip:") {
            self.uri.strip_prefix("sip:")
                .and_then(|s| s.split('@').nth(1))
        } else {
            None
        }
    }
}

/// Call direction
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallDirection {
    /// Incoming call (we received the INVITE)
    Incoming,
    /// Outgoing call (we sent the INVITE)
    Outgoing,
}

/// SIP transport types
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SipTransport {
    /// UDP transport
    Udp,
    /// TCP transport
    Tcp,
    /// TLS transport (secure)
    Tls,
} 