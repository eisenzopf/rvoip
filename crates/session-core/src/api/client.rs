//! SIP Client API
//!
//! This module provides the SipClient trait for non-session SIP operations
//! like REGISTER, OPTIONS, MESSAGE, and SUBSCRIBE.

use async_trait::async_trait;
use std::time::Duration;
use std::collections::HashMap;
use crate::errors::{Result, SessionError};

/// Handle for tracking registration state
#[derive(Debug, Clone)]
pub struct RegistrationHandle {
    /// Transaction ID for the REGISTER
    pub transaction_id: String,
    /// Registration expiration in seconds
    pub expires: u32,
    /// Contact URI that was registered
    pub contact_uri: String,
    /// Registrar URI
    pub registrar_uri: String,
}

/// Response from a SIP request
#[derive(Debug, Clone)]
pub struct SipResponse {
    /// Status code (e.g., 200, 404, 401)
    pub status_code: u16,
    /// Reason phrase
    pub reason_phrase: String,
    /// Response headers
    pub headers: HashMap<String, String>,
    /// Response body
    pub body: Option<String>,
}

/// Handle for managing subscriptions
#[derive(Debug, Clone)]
pub struct SubscriptionHandle {
    /// Subscription dialog ID
    pub dialog_id: String,
    /// Event type
    pub event_type: String,
    /// Expiration time
    pub expires_at: std::time::Instant,
}

/// Trait for non-session SIP operations
/// 
/// This trait provides methods for SIP operations that don't create or manage
/// sessions/dialogs, such as registration, instant messaging, and presence.
/// 
/// # Example
/// 
/// ```rust,no_run
/// use rvoip_session_core::api::*;
/// 
/// # async fn example() -> Result<()> {
/// let coordinator = SessionManagerBuilder::new()
///     .with_sip_port(5060)
///     .with_local_address("sip:alice@192.168.1.100")
///     .enable_sip_client()
///     .build()
///     .await?;
/// 
/// // Register with a SIP server
/// let registration = coordinator.register(
///     "sip:registrar.example.com",
///     "sip:alice@example.com",
///     "sip:alice@192.168.1.100:5060",
///     3600
/// ).await?;
/// 
/// println!("Registered successfully: {}", registration.transaction_id);
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait SipClient: Send + Sync {
    /// Send a REGISTER request
    /// 
    /// Registers a SIP endpoint with a registrar server. This is used to
    /// tell the server where to route incoming calls for a particular AOR.
    /// 
    /// # Arguments
    /// * `registrar_uri` - The registrar server URI (e.g., "sip:registrar.example.com")
    /// * `from_uri` - The AOR being registered (e.g., "sip:alice@example.com")
    /// * `contact_uri` - Where to reach this endpoint (e.g., "sip:alice@192.168.1.100:5060")
    /// * `expires` - Registration duration in seconds (0 to unregister)
    /// 
    /// # Returns
    /// A handle tracking the registration or an error
    /// 
    /// # Example
    /// 
    /// ```rust,no_run
    /// # use rvoip_session_core::api::*;
    /// # use std::sync::Arc;
    /// # async fn example(coordinator: Arc<SessionCoordinator>) -> Result<()> {
    /// // Register for 1 hour
    /// let reg = coordinator.register(
    ///     "sip:registrar.example.com",
    ///     "sip:alice@example.com",
    ///     "sip:alice@192.168.1.100:5060",
    ///     3600
    /// ).await?;
    /// 
    /// // Later, unregister
    /// coordinator.register(
    ///     "sip:registrar.example.com",
    ///     "sip:alice@example.com",
    ///     "sip:alice@192.168.1.100:5060",
    ///     0  // expires=0 means unregister
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn register(
        &self,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<RegistrationHandle>;
    
    /// Send an OPTIONS request (keepalive/capability query)
    /// 
    /// OPTIONS is used to query the capabilities of a SIP endpoint or
    /// to keep NAT mappings alive.
    /// 
    /// # Arguments
    /// * `target_uri` - The target to query
    /// 
    /// # Returns
    /// The OPTIONS response containing supported methods and capabilities
    async fn send_options(&self, target_uri: &str) -> Result<SipResponse>;
    
    /// Send a MESSAGE request (instant message)
    /// 
    /// Sends a SIP instant message to another endpoint without establishing
    /// a session.
    /// 
    /// # Arguments
    /// * `to_uri` - Message recipient
    /// * `message` - Message content
    /// * `content_type` - MIME type (defaults to "text/plain")
    /// 
    /// # Returns
    /// The MESSAGE response indicating delivery status
    async fn send_message(
        &self,
        to_uri: &str,
        message: &str,
        content_type: Option<&str>,
    ) -> Result<SipResponse>;
    
    /// Send a SUBSCRIBE request
    /// 
    /// Subscribes to events from another endpoint, such as presence updates
    /// or dialog state changes.
    /// 
    /// # Arguments
    /// * `target_uri` - What to subscribe to
    /// * `event_type` - Event package (e.g., "presence", "dialog", "message-summary")
    /// * `expires` - Subscription duration in seconds
    /// 
    /// # Returns
    /// Subscription handle for managing the subscription
    async fn subscribe(
        &self,
        target_uri: &str,
        event_type: &str,
        expires: u32,
    ) -> Result<SubscriptionHandle>;
    
    /// Send a raw SIP request (advanced use)
    /// 
    /// For advanced users who need complete control over the SIP request.
    /// 
    /// # Arguments
    /// * `request` - Complete SIP request to send
    /// * `timeout` - Response timeout
    /// 
    /// # Returns
    /// The SIP response
    async fn send_raw_request(
        &self,
        request: rvoip_sip_core::Request,
        timeout: Duration,
    ) -> Result<SipResponse>;
}

/// Unified Client API
/// 
/// This module provides a comprehensive set of functions that combine all the
/// functionality from SipClient, SessionControl, and MediaControl into a single
/// unified interface. This allows client-core to use all session-core functionality
/// through a single, consistent API.
/// 
/// # Architecture
/// 
/// All functions in this module are thin wrappers that delegate to the actual
/// implementations in SessionControl, MediaControl, and SipClient traits. This
/// maintains the existing architecture while providing a convenient unified interface.
/// 
/// # Example Usage
/// 
/// ```rust,no_run
/// use rvoip_session_core::api::client::unified;
/// use rvoip_session_core::api::SessionManagerBuilder;
/// use std::sync::Arc;
/// 
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create a session coordinator
/// let coordinator = SessionManagerBuilder::new()
///     .with_sip_port(5060)
///     .build()
///     .await?;
/// 
/// // Use unified API for all operations
/// unified::start(&coordinator).await?;
/// 
/// // Make a call
/// let prepared = unified::prepare_outgoing_call(
///     &coordinator,
///     "sip:alice@example.com",
///     "sip:bob@example.com"
/// ).await?;
/// 
/// let session = unified::initiate_prepared_call(&coordinator, &prepared).await?;
/// 
/// // Register with a SIP server
/// let registration = unified::register(
///     &coordinator,
///     "sip:registrar.example.com",
///     "sip:alice@example.com",
///     "sip:alice@192.168.1.100:5060",
///     3600
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub mod unified {
    use super::*;
    use crate::SessionCoordinator;
    use crate::api::{SessionControl, MediaControl};
    use crate::api::types::*;
    use crate::media::stats::{RtpSessionStats, MediaStatistics, CallStatistics};
    use std::sync::Arc;
    use std::net::SocketAddr;
    
    // ==================== SipClient Functions ====================
    
    /// Send a REGISTER request to a SIP registrar
    /// 
    /// See [`SipClient::register`] for details.
    pub async fn register(
        coordinator: &Arc<SessionCoordinator>,
        registrar_uri: &str,
        from_uri: &str,
        contact_uri: &str,
        expires: u32,
    ) -> Result<RegistrationHandle> {
        <Arc<SessionCoordinator> as SipClient>::register(
            coordinator,
            registrar_uri,
            from_uri,
            contact_uri,
            expires
        ).await
    }
    
    /// Send an OPTIONS request for keepalive or capability query
    /// 
    /// See [`SipClient::send_options`] for details.
    pub async fn send_options(
        coordinator: &Arc<SessionCoordinator>,
        target_uri: &str,
    ) -> Result<SipResponse> {
        <Arc<SessionCoordinator> as SipClient>::send_options(coordinator, target_uri).await
    }
    
    /// Send a MESSAGE request (instant message)
    /// 
    /// See [`SipClient::send_message`] for details.
    pub async fn send_message(
        coordinator: &Arc<SessionCoordinator>,
        to_uri: &str,
        message: &str,
        content_type: Option<&str>,
    ) -> Result<SipResponse> {
        <Arc<SessionCoordinator> as SipClient>::send_message(
            coordinator,
            to_uri,
            message,
            content_type
        ).await
    }
    
    /// Send a SUBSCRIBE request for event notifications
    /// 
    /// See [`SipClient::subscribe`] for details.
    pub async fn subscribe(
        coordinator: &Arc<SessionCoordinator>,
        target_uri: &str,
        event_type: &str,
        expires: u32,
    ) -> Result<SubscriptionHandle> {
        <Arc<SessionCoordinator> as SipClient>::subscribe(
            coordinator,
            target_uri,
            event_type,
            expires
        ).await
    }
    
    /// Send a raw SIP request (advanced use)
    /// 
    /// See [`SipClient::send_raw_request`] for details.
    pub async fn send_raw_request(
        coordinator: &Arc<SessionCoordinator>,
        request: rvoip_sip_core::Request,
        timeout: Duration,
    ) -> Result<SipResponse> {
        <Arc<SessionCoordinator> as SipClient>::send_raw_request(
            coordinator,
            request,
            timeout
        ).await
    }
    
    // ==================== SessionControl Functions ====================
    
    /// Prepare an outgoing call without initiating it
    /// 
    /// This allocates resources and generates SDP but doesn't send INVITE yet.
    /// See [`SessionControl::prepare_outgoing_call`] for details.
    pub async fn prepare_outgoing_call(
        coordinator: &Arc<SessionCoordinator>,
        from: &str,
        to: &str,
    ) -> Result<PreparedCall> {
        SessionControl::prepare_outgoing_call(coordinator, from, to).await
    }
    
    /// Initiate a previously prepared call
    /// 
    /// Sends the INVITE and starts the call setup process.
    /// See [`SessionControl::initiate_prepared_call`] for details.
    pub async fn initiate_prepared_call(
        coordinator: &Arc<SessionCoordinator>,
        prepared_call: &PreparedCall,
    ) -> Result<CallSession> {
        SessionControl::initiate_prepared_call(coordinator, prepared_call).await
    }
    
    /// Accept an incoming call
    /// 
    /// See [`SessionControl::accept_incoming_call`] for details.
    pub async fn accept_incoming_call(
        coordinator: &Arc<SessionCoordinator>,
        call: &IncomingCall,
        sdp_answer: Option<String>,
    ) -> Result<CallSession> {
        SessionControl::accept_incoming_call(coordinator, call, sdp_answer).await
    }
    
    /// Reject an incoming call
    /// 
    /// See [`SessionControl::reject_incoming_call`] for details.
    pub async fn reject_incoming_call(
        coordinator: &Arc<SessionCoordinator>,
        call: &IncomingCall,
        reason: &str,
    ) -> Result<()> {
        SessionControl::reject_incoming_call(coordinator, call, reason).await
    }
    
    /// Terminate an active session
    /// 
    /// See [`SessionControl::terminate_session`] for details.
    pub async fn terminate_session(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        SessionControl::terminate_session(coordinator, session_id).await
    }
    
    /// Start the session coordinator
    /// 
    /// See [`SessionControl::start`] for details.
    pub async fn start(coordinator: &Arc<SessionCoordinator>) -> Result<()> {
        SessionControl::start(coordinator).await
    }
    
    /// Stop the session coordinator
    /// 
    /// See [`SessionControl::stop`] for details.
    pub async fn stop(coordinator: &Arc<SessionCoordinator>) -> Result<()> {
        SessionControl::stop(coordinator).await
    }
    
    /// Get the bound SIP address
    /// 
    /// See [`SessionControl::get_bound_address`] for details.
    pub fn get_bound_address(coordinator: &Arc<SessionCoordinator>) -> SocketAddr {
        SessionControl::get_bound_address(coordinator)
    }
    
    /// Put a call on hold
    /// 
    /// See [`SessionControl::hold_session`] for details.
    pub async fn hold_session(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        SessionControl::hold_session(coordinator, session_id).await
    }
    
    /// Resume a held call
    /// 
    /// See [`SessionControl::resume_session`] for details.
    pub async fn resume_session(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        SessionControl::resume_session(coordinator, session_id).await
    }
    
    /// Send DTMF tones
    /// 
    /// See [`SessionControl::send_dtmf`] for details.
    pub async fn send_dtmf(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        digits: &str,
    ) -> Result<()> {
        SessionControl::send_dtmf(coordinator, session_id, digits).await
    }
    
    /// Transfer a call to another party
    /// 
    /// See [`SessionControl::transfer_session`] for details.
    pub async fn transfer_session(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        target: &str,
    ) -> Result<()> {
        SessionControl::transfer_session(coordinator, session_id, target).await
    }
    
    /// Set audio mute state
    /// 
    /// See [`SessionControl::set_audio_muted`] for details.
    pub async fn set_audio_muted(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        muted: bool,
    ) -> Result<()> {
        SessionControl::set_audio_muted(coordinator, session_id, muted).await
    }
    
    /// Update media parameters with new SDP
    /// 
    /// See [`SessionControl::update_media`] for details.
    pub async fn update_media(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        new_sdp: &str,
    ) -> Result<()> {
        SessionControl::update_media(coordinator, session_id, new_sdp).await
    }
    
    // ==================== MediaControl Functions ====================
    
    /// Generate an SDP answer for an incoming offer
    /// 
    /// See [`MediaControl::generate_sdp_answer`] for details.
    pub async fn generate_sdp_answer(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        offer: &str,
    ) -> Result<String> {
        MediaControl::generate_sdp_answer(coordinator, session_id, offer).await
    }
    
    /// Generate an SDP offer for outgoing calls
    /// 
    /// See [`MediaControl::generate_sdp_offer`] for details.
    pub async fn generate_sdp_offer(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<String> {
        MediaControl::generate_sdp_offer(coordinator, session_id).await
    }
    
    /// Subscribe to receive audio frames from a session
    /// 
    /// See [`MediaControl::subscribe_to_audio_frames`] for details.
    pub async fn subscribe_to_audio_frames(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<AudioFrameSubscriber> {
        MediaControl::subscribe_to_audio_frames(coordinator, session_id).await
    }
    
    /// Get media information for a session
    /// 
    /// See [`MediaControl::get_media_info`] for details.
    pub async fn get_media_info(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<Option<MediaInfo>> {
        MediaControl::get_media_info(coordinator, session_id).await
    }
    
    /// Start audio transmission for a session
    /// 
    /// See [`MediaControl::start_audio_transmission`] for details.
    pub async fn start_audio_transmission(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        MediaControl::start_audio_transmission(coordinator, session_id).await
    }
    
    /// Start audio transmission with a test tone
    /// 
    /// See [`MediaControl::start_audio_transmission_with_tone`] for details.
    pub async fn start_audio_transmission_with_tone(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        MediaControl::start_audio_transmission_with_tone(coordinator, session_id).await
    }
    
    /// Start audio transmission with custom audio samples
    /// 
    /// See [`MediaControl::start_audio_transmission_with_custom_audio`] for details.
    pub async fn start_audio_transmission_with_custom_audio(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        samples: Vec<u8>,
        repeat: bool,
    ) -> Result<()> {
        MediaControl::start_audio_transmission_with_custom_audio(
            coordinator,
            session_id,
            samples,
            repeat
        ).await
    }
    
    /// Set custom audio samples for transmission
    /// 
    /// See [`MediaControl::set_custom_audio`] for details.
    pub async fn set_custom_audio(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        samples: Vec<u8>,
        repeat: bool,
    ) -> Result<()> {
        MediaControl::set_custom_audio(coordinator, session_id, samples, repeat).await
    }
    
    /// Configure tone generation parameters
    /// 
    /// See [`MediaControl::set_tone_generation`] for details.
    pub async fn set_tone_generation(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        frequency: f64,
        amplitude: f64,
    ) -> Result<()> {
        MediaControl::set_tone_generation(coordinator, session_id, frequency, amplitude).await
    }
    
    /// Set pass-through mode for audio
    /// 
    /// See [`MediaControl::set_pass_through_mode`] for details.
    pub async fn set_pass_through_mode(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        MediaControl::set_pass_through_mode(coordinator, session_id).await
    }
    
    /// Stop audio transmission
    /// 
    /// See [`MediaControl::stop_audio_transmission`] for details.
    pub async fn stop_audio_transmission(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        MediaControl::stop_audio_transmission(coordinator, session_id).await
    }
    
    /// Update remote SDP for a session
    /// 
    /// See [`MediaControl::update_remote_sdp`] for details.
    pub async fn update_remote_sdp(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        sdp: &str,
    ) -> Result<()> {
        MediaControl::update_remote_sdp(coordinator, session_id, sdp).await
    }
    
    /// Create a media session
    /// 
    /// See [`MediaControl::create_media_session`] for details.
    pub async fn create_media_session(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        MediaControl::create_media_session(coordinator, session_id).await
    }
    
    /// Establish media flow to a remote endpoint
    /// 
    /// See [`MediaControl::establish_media_flow`] for details.
    pub async fn establish_media_flow(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        remote_addr: &str,
    ) -> Result<()> {
        MediaControl::establish_media_flow(coordinator, session_id, remote_addr).await
    }
    
    /// Get RTP statistics for a session
    /// 
    /// See [`MediaControl::get_rtp_statistics`] for details.
    pub async fn get_rtp_statistics(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<Option<RtpSessionStats>> {
        MediaControl::get_rtp_statistics(coordinator, session_id).await
    }
    
    /// Get media statistics for a session
    /// 
    /// See [`MediaControl::get_media_statistics`] for details.
    pub async fn get_media_statistics(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<Option<MediaStatistics>> {
        MediaControl::get_media_statistics(coordinator, session_id).await
    }
    
    /// Get call statistics for a session
    /// 
    /// See [`MediaControl::get_call_statistics`] for details.
    pub async fn get_call_statistics(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<Option<CallStatistics>> {
        MediaControl::get_call_statistics(coordinator, session_id).await
    }
    
    /// Send an audio frame to a session
    /// 
    /// See [`MediaControl::send_audio_frame`] for details.
    pub async fn send_audio_frame(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        frame: AudioFrame,
    ) -> Result<()> {
        MediaControl::send_audio_frame(coordinator, session_id, frame).await
    }
    
    /// Get audio stream configuration
    /// 
    /// See [`MediaControl::get_audio_stream_config`] for details.
    pub async fn get_audio_stream_config(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<Option<AudioStreamConfig>> {
        MediaControl::get_audio_stream_config(coordinator, session_id).await
    }
    
    /// Set audio stream configuration
    /// 
    /// See [`MediaControl::set_audio_stream_config`] for details.
    pub async fn set_audio_stream_config(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
        config: AudioStreamConfig,
    ) -> Result<()> {
        MediaControl::set_audio_stream_config(coordinator, session_id, config).await
    }
    
    /// Start audio streaming
    /// 
    /// See [`MediaControl::start_audio_stream`] for details.
    pub async fn start_audio_stream(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        MediaControl::start_audio_stream(coordinator, session_id).await
    }
    
    /// Stop audio streaming
    /// 
    /// See [`MediaControl::stop_audio_stream`] for details.
    pub async fn stop_audio_stream(
        coordinator: &Arc<SessionCoordinator>,
        session_id: &SessionId,
    ) -> Result<()> {
        MediaControl::stop_audio_stream(coordinator, session_id).await
    }
} 