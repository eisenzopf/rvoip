//! Session Control API
//!
//! This module provides the main control interface for managing SIP sessions.
//! 
//! # Overview
//! 
//! The `SessionControl` trait is the primary interface for call control operations.
//! It provides methods for:
//! - Creating and managing outgoing calls
//! - Accepting and rejecting incoming calls  
//! - Media control (mute, hold, transfer)
//! - Session state monitoring
//! - DTMF tone generation
//! 
//! # Usage Patterns
//! 
//! ## Pattern 1: Immediate Decision (CallHandler)
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! 
//! #[derive(Debug)]
//! struct MyHandler;
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for MyHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // Immediate decision in callback
//!         if call.from.contains("trusted") {
//!             // In a real implementation, you would generate proper SDP based on the offer
//!             let simple_sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
//!             CallDecision::Accept(Some(simple_sdp.to_string()))
//!         } else {
//!             CallDecision::Reject("Untrusted caller".to_string())
//!         }
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         println!("Call {} ended: {}", call.id(), reason);
//!     }
//! }
//! ```
//! 
//! ## Pattern 2: Deferred Decision (Programmatic)
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::sync::Arc;
//! 
//! #[derive(Debug)]
//! struct DeferHandler;
//! 
//! #[async_trait::async_trait]
//! impl CallHandler for DeferHandler {
//!     async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
//!         // Defer for async processing
//!         CallDecision::Defer
//!     }
//!     
//!     async fn on_call_ended(&self, call: CallSession, reason: &str) {
//!         // Handle call end
//!     }
//! }
//! 
//! // Later, after async operations:
//! async fn process_deferred_call(
//!     coordinator: &Arc<SessionCoordinator>,
//!     call: IncomingCall
//! ) -> Result<()> {
//!     // Check database, apply business rules, etc.
//!     // In a real implementation, you would check permissions here
//!     let allowed = !call.from.contains("blocked");
//!     
//!     if allowed {
//!         let sdp_answer = MediaControl::generate_sdp_answer(
//!             coordinator, 
//!             &call.id, 
//!             call.sdp.as_ref().unwrap()
//!         ).await?;
//!         
//!         SessionControl::accept_incoming_call(
//!             coordinator, 
//!             &call, 
//!             Some(sdp_answer)
//!         ).await?;
//!     } else {
//!         SessionControl::reject_incoming_call(
//!             coordinator, 
//!             &call, 
//!             "Permission denied"
//!         ).await?;
//!     }
//!     
//!     Ok(())
//! }
//! ```
//! 
//! # Example: Complete Call Flow
//! 
//! ```rust
//! use rvoip_session_core::api::*;
//! use std::time::Duration;
//! use std::sync::Arc;
//! 
//! async fn example_call_flow(coordinator: Arc<SessionCoordinator>) -> Result<()> {
//!     // 1. Make an outgoing call
//!     let prepared = SessionControl::prepare_outgoing_call(
//!         &coordinator,
//!         "sip:alice@example.com",
//!         "sip:bob@myserver.com"
//!     ).await?;
//!     
//!     let session = SessionControl::initiate_prepared_call(
//!         &coordinator,
//!         &prepared
//!     ).await?;
//!     
//!     // 2. Wait for answer
//!     SessionControl::wait_for_answer(
//!         &coordinator,
//!         session.id(),
//!         Duration::from_secs(30)
//!     ).await?;
//!     
//!     // 3. Monitor call quality
//!     let stats = MediaControl::get_media_statistics(
//!         &coordinator,
//!         session.id()
//!     ).await?;
//!     
//!     // 4. Send DTMF tones
//!     SessionControl::send_dtmf(
//!         &coordinator,
//!         session.id(),
//!         "1234#"
//!     ).await?;
//!     
//!     // 5. Mute the microphone (sends silence packets)
//!     SessionControl::set_audio_muted(
//!         &coordinator,
//!         session.id(),
//!         true
//!     ).await?;
//!     
//!     // 6. Hold the call
//!     SessionControl::hold_session(
//!         &coordinator,
//!         session.id()
//!     ).await?;
//!     
//!     // 7. Resume the call
//!     SessionControl::resume_session(
//!         &coordinator,
//!         session.id()
//!     ).await?;
//!     
//!     // 8. Unmute the microphone
//!     SessionControl::set_audio_muted(
//!         &coordinator,
//!         session.id(),
//!         false
//!     ).await?;
//!     
//!     // 9. End the call
//!     SessionControl::terminate_session(
//!         &coordinator,
//!         session.id()
//!     ).await?;
//!     
//!     Ok(())
//! }
//! ```

use crate::api::types::*;
use crate::api::handlers::CallHandler;
use crate::api::media::MediaControl;
use crate::coordinator::SessionCoordinator;
use crate::errors::{Result, SessionError};
use crate::manager::events::SessionEvent;
use crate::session::Session;
use std::sync::Arc;

/// Main session control trait for managing SIP sessions
/// 
/// This trait provides the primary interface for call control operations.
/// All methods are async and return proper error types for robust error handling.
pub trait SessionControl {
    /// Prepare an outgoing call by allocating resources and generating SDP
    /// 
    /// This method allocates media resources (RTP port) and generates an SDP offer,
    /// but does NOT send the INVITE yet. This allows you to prepare multiple calls
    /// or add custom headers before initiating.
    /// 
    /// # Arguments
    /// * `from` - Local SIP URI (e.g., "sip:alice@example.com")
    /// * `to` - Remote SIP URI (e.g., "sip:bob@example.com")
    /// 
    /// # Returns
    /// * `PreparedCall` containing session ID, SDP offer, and allocated RTP port
    /// 
    /// # Example
    /// ```rust,no_run
    /// # use rvoip_session_core::{SessionControl, SessionCoordinator};
    /// # use std::sync::Arc;
    /// # async fn example(coordinator: Arc<SessionCoordinator>) -> Result<(), Box<dyn std::error::Error>> {
    /// let prepared = SessionControl::prepare_outgoing_call(
    ///     &coordinator,
    ///     "sip:alice@ourserver.com", 
    ///     "sip:bob@example.com"
    /// ).await?;
    /// 
    /// println!("Allocated RTP port: {}", prepared.local_rtp_port);
    /// 
    /// // Now initiate the call
    /// let session = SessionControl::initiate_prepared_call(
    ///     &coordinator,
    ///     &prepared
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    async fn prepare_outgoing_call(
        &self,
        from: &str,
        to: &str,
    ) -> Result<PreparedCall>;
    
    /// Initiate a prepared call by sending the INVITE
    async fn initiate_prepared_call(
        &self,
        prepared_call: &PreparedCall,
    ) -> Result<CallSession>;
    
    /// Create an outgoing call (legacy method - prepares and initiates in one step)
    async fn create_outgoing_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<CallSession>;
    
    /// Terminate an active session
    async fn terminate_session(&self, session_id: &SessionId) -> Result<()>;
    
    /// Get session information
    async fn get_session(&self, session_id: &SessionId) -> Result<Option<CallSession>>;
    
    /// List all active sessions
    async fn list_active_sessions(&self) -> Result<Vec<SessionId>>;
    
    /// Get session statistics
    async fn get_stats(&self) -> Result<SessionStats>;
    
    /// Get the configured call handler
    fn get_handler(&self) -> Option<Arc<dyn CallHandler>>;
    
    /// Get the bound SIP address
    fn get_bound_address(&self) -> std::net::SocketAddr;
    
    /// Start the session manager
    async fn start(&self) -> Result<()>;
    
    /// Stop the session manager
    async fn stop(&self) -> Result<()>;
    
    /// Put a session on hold
    async fn hold_session(&self, session_id: &SessionId) -> Result<()>;
    
    /// Resume a held session
    async fn resume_session(&self, session_id: &SessionId) -> Result<()>;
    
    /// Transfer a session to another party
    async fn transfer_session(&self, session_id: &SessionId, target: &str) -> Result<()>;
    
    /// Update session media (e.g., for codec changes)
    async fn update_media(&self, session_id: &SessionId, sdp: &str) -> Result<()>;
    
    /// Get media information for a session
    async fn get_media_info(&self, session_id: &SessionId) -> Result<Option<MediaInfo>>;
    
    /// Mute/unmute audio for a session
    /// 
    /// This implements silence-based muting where RTP packets continue to flow
    /// but audio is replaced with silence. This approach maintains NAT bindings
    /// and is compatible with all SIP endpoints.
    /// 
    /// # Arguments
    /// * `session_id` - The session to mute/unmute
    /// * `muted` - `true` to send silence, `false` to send normal audio
    /// 
    /// # Behavior
    /// - RTP packets continue flowing to prevent NAT timeouts
    /// - Audio frames are replaced with silence when muted
    /// - No SIP renegotiation required
    /// - Instant mute/unmute effect
    /// 
    /// # Returns
    /// Returns `Ok(())` on success, or an error if the session doesn't exist
    /// or is in an invalid state
    async fn set_audio_muted(&self, session_id: &SessionId, muted: bool) -> Result<()>;
    
    /// Enable/disable video
    async fn set_video_enabled(&self, session_id: &SessionId, enabled: bool) -> Result<()>;
    
    /// Send DTMF tones on an active session
    /// 
    /// # Arguments
    /// * `session_id` - The ID of the session to send DTMF on
    /// * `digits` - The DTMF digits to send (0-9, *, #, A-D)
    /// 
    /// # Returns
    /// * `Ok(())` if the DTMF was sent successfully
    /// * `Err(SessionError)` if the session doesn't exist or is not in an active state
    async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()>;
    
    /// Wait for an outgoing call to be answered
    /// 
    /// This method blocks until the call transitions to Active state (answered)
    /// or fails/times out.
    /// 
    /// # Arguments
    /// * `session_id` - The ID of the session to wait for
    /// * `timeout` - Maximum time to wait for answer
    /// 
    /// # Returns
    /// * `Ok(())` if the call was answered
    /// * `Err(SessionError)` if the call failed, was cancelled, or timed out
    async fn wait_for_answer(&self, session_id: &SessionId, timeout: std::time::Duration) -> Result<()>;
    
    /// Accept an incoming call programmatically outside of CallHandler
    /// 
    /// This is useful for more complex decision logic that can't be handled
    /// in the synchronous on_incoming_call callback.
    /// 
    /// # Arguments
    /// * `call` - The incoming call to accept
    /// * `sdp_answer` - Optional SDP answer to send with the acceptance
    /// 
    /// # Returns
    /// * `Ok(CallSession)` if the call was accepted successfully
    /// * `Err(SessionError)` if the call cannot be accepted
    async fn accept_incoming_call(&self, call: &IncomingCall, sdp_answer: Option<String>) -> Result<CallSession>;
    
    /// Reject an incoming call programmatically outside of CallHandler
    /// 
    /// This complements accept_incoming_call for cases where the decision
    /// needs to be made outside the callback.
    /// 
    /// # Arguments
    /// * `call` - The incoming call to reject
    /// * `reason` - The rejection reason
    /// 
    /// # Returns
    /// * `Ok(())` if the call was rejected successfully
    /// * `Err(SessionError)` if the call cannot be rejected
    async fn reject_incoming_call(&self, call: &IncomingCall, reason: &str) -> Result<()>;
}

/// Implementation of SessionControl for SessionCoordinator
impl SessionControl for Arc<SessionCoordinator> {
    async fn prepare_outgoing_call(
        &self,
        from: &str,
        to: &str,
    ) -> Result<PreparedCall> {
        // Create a session ID
        let session_id = SessionId::new();
        
        // Create the call session in preparing state
        let call = CallSession {
            id: session_id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            state: CallState::Initiating,
            started_at: Some(std::time::Instant::now()),
            sip_call_id: None, // Will be populated when we get the response
        };
        
        // Create and register internal session
        let session = Session::from_call_session(call.clone());
        self.registry.register_session(session).await?;
        
        // Create media session to allocate port
        self.media_manager.create_media_session(&session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to create media session: {}", e) 
            })?;
        
        // Generate SDP offer with allocated port
        let sdp_offer = self.media_manager.generate_sdp_offer(&session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to generate SDP offer: {}", e) 
            })?;
        
        // Get the allocated port from media info
        let media_info = self.media_manager.get_media_info(&session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })?;
        
        let local_rtp_port = media_info
            .and_then(|info| info.local_rtp_port)
            .unwrap_or(0);
        
        // Store the generated SDP in the registry for hold/resume
        self.registry.update_session_sdp(&session_id, Some(sdp_offer.clone()), None).await?;
        
        Ok(PreparedCall {
            session_id,
            from: from.to_string(),
            to: to.to_string(),
            sdp_offer,
            local_rtp_port,
        })
    }
    
    async fn initiate_prepared_call(
        &self,
        prepared_call: &PreparedCall,
    ) -> Result<CallSession> {
        // CRITICAL: Track From URI BEFORE creating dialog
        // This ensures the mapping exists when the 200 OK arrives
        // Use the configured local_address which dialog-core will actually use
        self.dialog_coordinator().track_from_uri(prepared_call.session_id.clone(), &self.config().local_address);
        
        // Send the INVITE with the prepared SDP
        let dialog_handle = self.dialog_manager
            .create_outgoing_call(
                prepared_call.session_id.clone(),
                &prepared_call.from,
                &prepared_call.to,
                Some(prepared_call.sdp_offer.clone()),
            )
            .await
            .map_err(|e| SessionError::internal(&format!("Failed to initiate call: {}", e)))?;
        
        // Set session-to-dialog mapping in the coordinator
        self.dialog_coordinator().map_session_to_dialog(
            prepared_call.session_id.clone(), 
            dialog_handle.dialog_id.clone(),
            None  // Call-ID will be tracked when we see the first SIP message
        );
        
        // Return the session
        self.get_session(&prepared_call.session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&prepared_call.session_id.0))
    }
    
    async fn create_outgoing_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<CallSession> {
        SessionCoordinator::create_outgoing_call(self, from, to, sdp).await
    }
    
    async fn terminate_session(&self, session_id: &SessionId) -> Result<()> {
        SessionCoordinator::terminate_session(self, session_id).await
    }
    
    async fn get_session(&self, session_id: &SessionId) -> Result<Option<CallSession>> {
        SessionCoordinator::find_session(self, session_id).await
    }
    
    async fn list_active_sessions(&self) -> Result<Vec<SessionId>> {
        SessionCoordinator::list_active_sessions(self).await
    }
    
    async fn get_stats(&self) -> Result<SessionStats> {
        SessionCoordinator::get_stats(self).await
    }
    
    fn get_handler(&self) -> Option<Arc<dyn CallHandler>> {
        self.handler.clone()
    }
    
    fn get_bound_address(&self) -> std::net::SocketAddr {
        SessionCoordinator::get_bound_address(self)
    }
    
    async fn start(&self) -> Result<()> {
        SessionCoordinator::start(self).await
    }
    
    async fn stop(&self) -> Result<()> {
        SessionCoordinator::stop(self).await
    }
    
    async fn hold_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only hold if session is active
        if !matches!(session.state(), CallState::Active) {
            return Err(SessionError::invalid_state(
                &format!("Cannot hold session in state {:?}", session.state())
            ));
        }
        
        // Start music-on-hold or mute audio
        if let Err(e) = self.start_music_on_hold(session_id).await {
            println!("🎵 Failed to start music-on-hold for {}, falling back to mute: {}", session_id, e);
            // Fallback to muting if MoH fails
            println!("🔇 Attempting to mute audio for session: {}", session_id);
            self.media_manager.set_audio_muted(session_id, true).await
                .map_err(|e| {
                    println!("❌ MUTE FAILED for {}: {}", session_id, e);
                    SessionError::MediaIntegration { 
                        message: format!("Failed to mute audio for hold: {}", e) 
                    }
                })?;
            println!("✅ Audio muted successfully for {}", session_id);
        }
        
        // Use dialog manager to send hold request with proper SDP
        self.dialog_manager.hold_session(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to hold session: {}", e)))?;
        
        // Update session state using internal registry
        self.registry.update_session_state(session_id, CallState::OnHold).await?;
        
        // Emit state change event
        let _ = self.publish_event(SessionEvent::StateChanged {
            session_id: session_id.clone(),
            old_state: CallState::Active,
            new_state: CallState::OnHold,
        }).await;
        
        Ok(())
    }
    
    async fn resume_session(&self, session_id: &SessionId) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only resume if session is on hold
        if !matches!(session.state(), CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot resume session in state {:?}", session.state())
            ));
        }
        
        // Use dialog manager to send resume request with proper SDP
        self.dialog_manager.resume_session(session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to resume session: {}", e)))?;
        
        // Stop music-on-hold and resume microphone audio
        self.stop_music_on_hold(session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to resume audio: {}", e) 
            })?;
        
        // Unmute the audio (it was muted during hold)
        self.media_manager.set_audio_muted(session_id, false).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to unmute audio: {}", e) 
            })?;
        
        // Update session state using internal registry
        self.registry.update_session_state(session_id, CallState::Active).await?;
        
        // Emit state change event
        let _ = self.publish_event(SessionEvent::StateChanged {
            session_id: session_id.clone(),
            old_state: CallState::OnHold,
            new_state: CallState::Active,
        }).await;
        
        Ok(())
    }
    
    async fn transfer_session(&self, session_id: &SessionId, target: &str) -> Result<()> {
        println!("🎯 API: transfer_session called for {} to {}", session_id, target);
        
        // Check if session exists
        println!("🎯 API: About to call get_session");
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        println!("🎯 API: get_session returned");
        
        println!("🎯 API: Session found, state: {:?}", session.state());
        
        // Only transfer if session is active or on hold
        if !matches!(session.state(), CallState::Active | CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot transfer session in state {:?}", session.state())
            ));
        }
        
        println!("🎯 API: Calling dialog_manager.transfer_session");
        
        // Use dialog manager to send transfer request
        self.dialog_manager.transfer_session(session_id, target).await
            .map_err(|e| {
                println!("❌ API: dialog_manager.transfer_session failed: {}", e);
                SessionError::internal(&format!("Failed to transfer session: {}", e))
            })?;
        
        println!("✅ API: dialog_manager.transfer_session succeeded");
        
        // Update session state
        self.registry.update_session_state(session_id, CallState::Transferring).await?;
        
        // Emit state change event
        let _ = self.publish_event(SessionEvent::StateChanged {
            session_id: session_id.clone(),
            old_state: CallState::Active, // Assume it was active before transfer
            new_state: CallState::Transferring,
        }).await;
        
        Ok(())
    }
    
    async fn update_media(&self, session_id: &SessionId, sdp: &str) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only update media if session is active or on hold
        if !matches!(session.state(), CallState::Active | CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot update media in state {:?}", session.state())
            ));
        }
        
        // Use dialog manager to send UPDATE/re-INVITE with new SDP
        self.dialog_manager.update_media(session_id, sdp).await
            .map_err(|e| SessionError::internal(&format!("Failed to update media: {}", e)))?;
        
        // Create media session if it doesn't exist
        if self.media_manager.get_media_info(session_id).await.ok().flatten().is_none() {
            self.media_manager.create_media_session(session_id).await
                .map_err(|e| SessionError::MediaIntegration { 
                    message: format!("Failed to create media session: {}", e) 
                })?;
        }
        
        // Also update media manager with new SDP
        self.media_manager.update_media_session(session_id, sdp).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to update media session: {}", e) 
            })?;
        
        // Send SDP event
        let _ = self.publish_event(SessionEvent::SdpEvent {
            session_id: session_id.clone(),
            event_type: "media_update".to_string(),
            sdp: sdp.to_string(),
        }).await;
        
        Ok(())
    }
    
    async fn get_media_info(&self, session_id: &SessionId) -> Result<Option<MediaInfo>> {
        // Check if session exists
        let _ = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Get media info from media manager
        let media_session_info = self.media_manager.get_media_info(session_id).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to get media info: {}", e) 
            })?;
        
        if let Some(info) = media_session_info {
            // Get stored SDP
            let (local_sdp, remote_sdp) = {
                let sdp_storage = self.media_manager.sdp_storage.read().await;
                sdp_storage.get(session_id).cloned().unwrap_or((None, None))
            };
            
            // Get RTP statistics
            let rtp_stats = self.media_manager.get_rtp_statistics(session_id).await
                .ok()
                .flatten()
                .map(|stats| crate::media::stats::RtpSessionStats {
                    packets_sent: stats.packets_sent,
                    packets_received: stats.packets_received,
                    bytes_sent: stats.bytes_sent,
                    bytes_received: stats.bytes_received,
                    packets_lost: stats.packets_lost,
                    packets_out_of_order: 0, // Not available in rtp-core
                    jitter_buffer_ms: 0.0,   // Not available in rtp-core
                    current_bitrate_kbps: 0, // Would need to calculate
                });
            
            // Get quality metrics from media statistics
            let quality_metrics = self.media_manager.get_media_statistics(session_id).await
                .ok()
                .flatten()
                .and_then(|stats| stats.quality_metrics.clone())
                .map(|qm| crate::media::stats::QualityMetrics {
                    mos_score: qm.mos_score.unwrap_or(5.0),
                    packet_loss_rate: qm.packet_loss_percent,
                    jitter_ms: qm.jitter_ms as f32,
                    round_trip_ms: qm.rtt_ms.unwrap_or(0.0) as f32,
                    network_effectiveness: 1.0 - (qm.packet_loss_percent / 100.0),
                    is_acceptable: qm.mos_score.unwrap_or(5.0) >= 3.0,
                });
            
            Ok(Some(MediaInfo {
                local_sdp,
                remote_sdp,
                local_rtp_port: info.local_rtp_port,
                remote_rtp_port: info.remote_rtp_port,
                codec: info.codec,
                rtp_stats,
                quality_metrics,
            }))
        } else {
            Ok(None)
        }
    }
    
    async fn set_audio_muted(&self, session_id: &SessionId, muted: bool) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only mute/unmute if session is active or on hold
        if !matches!(session.state(), CallState::Active | CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot change audio mute in state {:?}", session.state())
            ));
        }
        
        // Use the media manager to set mute state (sends silence when muted)
        self.media_manager.set_audio_muted(session_id, muted).await
            .map_err(|e| SessionError::MediaIntegration { 
                message: format!("Failed to set audio mute state: {}", e) 
            })?;
        
        // Send media event
        let _ = self.publish_event(SessionEvent::MediaEvent {
            session_id: session_id.clone(),
            event: format!("audio_muted={}", muted),
        }).await;
        
        Ok(())
    }
    
    async fn set_video_enabled(&self, session_id: &SessionId, enabled: bool) -> Result<()> {
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Only enable/disable video if session is active or on hold
        if !matches!(session.state(), CallState::Active | CallState::OnHold) {
            return Err(SessionError::invalid_state(
                &format!("Cannot change video in state {:?}", session.state())
            ));
        }
        
        // Send media event (actual video implementation would require SDP renegotiation)
        let _ = self.publish_event(SessionEvent::MediaEvent {
            session_id: session_id.clone(),
            event: format!("video_enabled={}", enabled),
        }).await;
        
        tracing::info!("Video {} for session {} (requires SDP renegotiation)", 
                      if enabled { "enabled" } else { "disabled" }, session_id);
        
        Ok(())
    }
    
    async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()> {
        SessionCoordinator::send_dtmf(self, session_id, digits).await
    }
    
    async fn wait_for_answer(&self, session_id: &SessionId, timeout: std::time::Duration) -> Result<()> {
        use tokio::time::timeout as tokio_timeout;
        
        // Check if session exists
        let session = self.get_session(session_id).await?
            .ok_or_else(|| SessionError::session_not_found(&session_id.0))?;
        
        // Check current state
        match session.state() {
            CallState::Active => {
                // Already answered
                return Ok(());
            }
            CallState::Failed(_) | CallState::Terminated | CallState::Cancelled => {
                // Already in a terminal state
                return Err(SessionError::invalid_state(
                    &format!("Call already ended with state: {:?}", session.state())
                ));
            }
            CallState::Initiating | CallState::Ringing => {
                // Expected states - continue waiting
            }
            _ => {
                // Unexpected state (OnHold, Transferring, etc.)
                return Err(SessionError::invalid_state(
                    &format!("Cannot wait for answer in state: {:?}", session.state())
                ));
            }
        }
        
        // Subscribe to events for this session
        let mut event_subscriber = self.event_processor.subscribe().await
            .map_err(|_| SessionError::internal("Failed to subscribe to events"))?;
        
        // Wait for state change with timeout
        let wait_future = async {
            loop {
                match event_subscriber.receive().await {
                    Ok(event) => {
                        if let SessionEvent::StateChanged { 
                            session_id: event_session_id, 
                            new_state, 
                            .. 
                        } = event {
                            if event_session_id == *session_id {
                                match new_state {
                                    CallState::Active => {
                                        // Call answered!
                                        return Ok(());
                                    }
                                    CallState::Failed(reason) => {
                                        return Err(SessionError::Other(
                                            format!("Call failed: {}", reason)
                                        ));
                                    }
                                    CallState::Terminated => {
                                        return Err(SessionError::Other(
                                            "Call was terminated".to_string()
                                        ));
                                    }
                                    CallState::Cancelled => {
                                        return Err(SessionError::Other(
                                            "Call was cancelled".to_string()
                                        ));
                                    }
                                    _ => {
                                        // Continue waiting for other states
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        return Err(SessionError::internal(&format!("Event error: {}", e)));
                    }
                }
            }
        };
        
        // Apply timeout
        match tokio_timeout(timeout, wait_future).await {
            Ok(result) => result,
            Err(_) => Err(SessionError::Timeout(
                format!("Timeout waiting for call {} to be answered", session_id)
            )),
        }
    }
    
    async fn accept_incoming_call(&self, call: &IncomingCall, sdp_answer: Option<String>) -> Result<CallSession> {
        tracing::info!("accept_incoming_call: call {} has SDP offer: {}", call.id, call.sdp.is_some());
        
        // Check if session already exists
        if let Ok(Some(session)) = self.get_session(&call.id).await {
            // Check if already accepted
            match session.state() {
                CallState::Active => {
                    return Err(SessionError::invalid_state(
                        "Call already accepted"
                    ));
                }
                CallState::Failed(_) | CallState::Terminated | CallState::Cancelled => {
                    return Err(SessionError::invalid_state(
                        "Call already ended"
                    ));
                }
                CallState::Ringing => {
                    // This is OK - we can accept a ringing call
                }
                _ => {}
            }
        }
        
        // Create session if it doesn't exist
        let mut session = if let Ok(Some(existing)) = self.get_session(&call.id).await {
            // Use existing session
            existing
        } else {
            // Create new session
            let new_session = CallSession {
                id: call.id.clone(),
                from: call.from.clone(),
                to: call.to.clone(),
                state: CallState::Ringing,
                started_at: Some(std::time::Instant::now()),
                sip_call_id: call.sip_call_id.clone(),
            };
            
            // Create and register internal session
            let internal_session = Session::from_call_session(new_session.clone());
            self.registry.register_session(internal_session).await?;
            new_session
        };
        
        // If we have SDP in the offer and no answer provided, generate one
        let final_sdp_answer = if call.sdp.is_some() && sdp_answer.is_none() {
            tracing::info!("Generating SDP answer for call {} because no answer was provided", call.id);
            // Use the public generate_sdp_answer function which calls negotiate_sdp_as_uas
            // This ensures the MediaNegotiated event is published
            match generate_sdp_answer(self, &call.id, call.sdp.as_ref().unwrap()).await {
                Ok(answer) => {
                    tracing::info!("Generated SDP answer for call {}", call.id);
                    Some(answer)
                },
                Err(e) => {
                    tracing::warn!("Failed to generate SDP answer: {}", e);
                    None
                }
            }
        } else {
            tracing::info!("Using provided SDP answer for call {} (provided: {})", call.id, sdp_answer.is_some());
            sdp_answer
        };
        
        // Accept the call at the dialog level with the SDP answer
        self.dialog_manager.accept_incoming_call(&call.id, final_sdp_answer.clone()).await
            .map_err(|e| SessionError::internal(&format!("Failed to accept call: {}", e)))?;
        
        // Update session state to Active
        self.registry.update_session_state(&call.id, CallState::Active).await?;
        
        // Emit state change event
        let _ = self.publish_event(SessionEvent::StateChanged {
            session_id: call.id.clone(),
            old_state: CallState::Ringing,
            new_state: CallState::Active,
        }).await;
        
        // Get the updated session
        // Note: on_call_established will be called by the coordinator when all conditions are met:
        // 1. Dialog is established (Active state)
        // 2. Media session is ready
        // 3. SDP negotiation is complete
        if let Ok(Some(session)) = self.registry.get_public_session(&call.id).await {
            Ok(session)
        } else {
            Err(SessionError::session_not_found(&call.id.0))
        }
    }
    
    async fn reject_incoming_call(&self, call: &IncomingCall, reason: &str) -> Result<()> {
        // Check if session exists and is in a state where it can be rejected
        if let Ok(Some(session)) = self.get_session(&call.id).await {
            match session.state() {
                CallState::Failed(_) | CallState::Terminated | CallState::Cancelled => {
                    return Err(SessionError::invalid_state(
                        "Call already ended"
                    ));
                }
                CallState::Active => {
                    return Err(SessionError::invalid_state(
                        "Cannot reject an active call - use terminate_session instead"
                    ));
                }
                _ => {}
            }
        }
        
        // Since dialog manager doesn't have a specific reject method,
        // we'll use terminate_session which handles the state appropriately
        self.dialog_manager.terminate_session(&call.id).await
            .map_err(|e| SessionError::internal(&format!("Failed to reject call: {}", e)))?;
        
        // Update session state if it exists
        if let Ok(Some(session)) = self.registry.get_session(&call.id).await {
            let old_state = session.state().clone();
            self.registry.update_session_state(&call.id, CallState::Failed(reason.to_string())).await?;
            
            // Emit state change event
            let _ = self.publish_event(SessionEvent::StateChanged {
                session_id: call.id.clone(),
                old_state,
                new_state: CallState::Failed(reason.to_string()),
            }).await;
        }
        
        Ok(())
    }
}

/// Generate an SDP answer for an incoming call
/// 
/// This function is used when you need to generate an SDP answer based on
/// your media preferences for an incoming call that was deferred.
/// 
/// # Arguments
/// * `coordinator` - The session coordinator
/// * `session_id` - The session to generate answer for  
/// * `their_offer` - The SDP offer from the remote party
/// 
/// # Returns
/// The generated SDP answer string
/// 
/// # Example
/// ```rust,no_run
/// # use rvoip_session_core::{SessionControl, SessionCoordinator, IncomingCall, CallDecision};
/// # use rvoip_session_core::control::generate_sdp_answer;
/// # use std::sync::Arc;
/// # async fn example(coordinator: Arc<SessionCoordinator>, call: IncomingCall) -> Result<(), Box<dyn std::error::Error>> {
/// // In deferred call handling
/// let answer = generate_sdp_answer(
///     &coordinator,
///     &call.id,
///     call.sdp.as_ref().unwrap()
/// ).await?;
/// 
/// SessionControl::accept_incoming_call(&coordinator, &call, Some(answer)).await?;
/// # Ok(())
/// # }
/// ```
pub async fn generate_sdp_answer(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
    their_offer: &str,
) -> Result<String> {
    let (answer, _negotiated) = coordinator.negotiate_sdp_as_uas(session_id, their_offer).await?;
    Ok(answer)
}

/// Get the negotiated media configuration for a session
/// 
/// Returns information about the negotiated codec, addresses, and other
/// media parameters after SDP negotiation is complete.
/// 
/// # Arguments
/// * `coordinator` - The session coordinator
/// * `session_id` - The session to query
/// 
/// # Returns
/// The negotiated configuration if available
/// 
/// # Example
/// ```rust,no_run
/// # use rvoip_session_core::{SessionCoordinator, SessionId};
/// # use rvoip_session_core::control::get_negotiated_media_config;
/// # use std::sync::Arc;
/// # async fn example(coordinator: Arc<SessionCoordinator>, session_id: SessionId) -> Result<(), Box<dyn std::error::Error>> {
/// if let Some(config) = get_negotiated_media_config(
///     &coordinator,
///     &session_id
/// ).await? {
///     println!("Using codec: {}", config.codec);
///     println!("Local RTP: {}", config.local_addr);
///     println!("Remote RTP: {}", config.remote_addr);
/// }
/// # Ok(())
/// # }
/// ```
pub async fn get_negotiated_media_config(
    coordinator: &Arc<SessionCoordinator>,
    session_id: &SessionId,
) -> Result<Option<crate::sdp::NegotiatedMediaConfig>> {
    Ok(coordinator.get_negotiated_config(session_id).await)
} 