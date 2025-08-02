//! Call operations for the client-core library
//! 
//! This module contains all call-related operations including making calls,
//! answering, rejecting, hanging up, and querying call information.
//!
//! # Call Management Overview
//!
//! The call operations provide a comprehensive API for managing SIP calls through
//! the session-core infrastructure. This includes:
//!
//! - **Outgoing Calls**: Initiate calls with `make_call()`
//! - **Incoming Calls**: Handle with `answer_call()` and `reject_call()`
//! - **Call Control**: Terminate calls with `hangup_call()`
//! - **Call Information**: Query call state and history
//! - **Statistics**: Track call metrics and performance
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────┐
//! │   Client Application    │
//! └───────────┬─────────────┘
//!             │
//! ┌───────────▼─────────────┐
//! │    Call Operations      │ ◄── This Module
//! │ ┌─────────────────────┐ │
//! │ │ make_call()         │ │
//! │ │ answer_call()       │ │
//! │ │ hangup_call()       │ │
//! │ │ get_call_*()        │ │
//! │ └─────────────────────┘ │
//! └───────────┬─────────────┘
//!             │
//! ┌───────────▼─────────────┐
//! │    session-core         │
//! │  SessionControl API     │
//! └─────────────────────────┘
//! ```
//!
//! # Usage Examples
//!
//! ```rust
//! use rvoip_client_core::{ClientManager, ClientConfig, CallId, CallState};
//! 
//! async fn call_operations_example() -> Result<(), Box<dyn std::error::Error>> {
//!     // Create and start client
//!     let config = ClientConfig::new()
//!         .with_sip_addr("127.0.0.1:5060".parse()?);
//!     let client = ClientManager::new(config).await?;
//!     client.start().await?;
//!     
//!     // Make an outgoing call
//!     let call_id = client.make_call(
//!         "sip:alice@example.com".to_string(),
//!         "sip:bob@example.com".to_string(),
//!         Some("Business call".to_string()),
//!     ).await?;
//!     
//!     // Check call information
//!     let call_info = client.get_call(&call_id).await?;
//!     println!("Call state: {:?}", call_info.state);
//!     
//!     // List all active calls
//!     let active_calls = client.get_active_calls().await;
//!     
//!     // End the call
//!     client.hangup_call(&call_id).await?;
//!     
//!     Ok(())
//! }
//! ```

use std::collections::HashMap;
use chrono::Utc;

// Import session-core APIs
use rvoip_session_core::api::{
    SessionControl,
    MediaControl,
};

// Import client-core types
use crate::{
    ClientResult, ClientError,
    call::{CallId, CallInfo, CallDirection},
};

use super::types::*;
use super::recovery::{retry_with_backoff, RetryConfig, ErrorContext};

/// Call operations implementation for ClientManager
impl super::manager::ClientManager {
    /// Make an outgoing call with enhanced information tracking
    /// 
    /// This method initiates a new outgoing call using the session-core infrastructure.
    /// It handles SDP generation, session creation, and proper event notification.
    /// 
    /// # Arguments
    /// 
    /// * `from` - The caller's SIP URI (e.g., "sip:alice@example.com")
    /// * `to` - The callee's SIP URI (e.g., "sip:bob@example.com")  
    /// * `subject` - Optional call subject/reason
    /// 
    /// # Returns
    /// 
    /// Returns a unique `CallId` that can be used to track and control the call.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::InvalidConfiguration` - If the URIs are malformed
    /// * `ClientError::NetworkError` - If there's a network connectivity issue
    /// * `ClientError::CallSetupFailed` - If the call cannot be initiated
    /// 
    /// # Examples
    /// 
    /// Basic call:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn make_basic_call() -> Result<CallId, Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5060".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = client.make_call(
    ///         "sip:alice@ourcompany.com".to_string(),
    ///         "sip:bob@example.com".to_string(),
    ///         None,
    ///     ).await?;
    /// 
    ///     println!("Outgoing call started: {}", call_id);
    ///     Ok(call_id)
    /// }
    /// ```
    /// 
    /// Call with subject:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn make_call_with_subject() -> Result<CallId, Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5061".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_id = client.make_call(
    ///         "sip:support@ourcompany.com".to_string(),
    ///         "sip:customer@example.com".to_string(),
    ///         Some("Technical Support Call".to_string()),
    ///     ).await?;
    ///     
    ///     Ok(call_id)
    /// }
    /// ```
    /// 
    /// # Call Flow
    /// 
    /// 1. Validates the SIP URIs
    /// 2. Creates a new session via session-core
    /// 3. Generates and stores call metadata
    /// 4. Emits appropriate events
    /// 5. Returns the CallId for tracking
    pub async fn make_call(
        &self,
        from: String,
        to: String,
        subject: Option<String>,
    ) -> ClientResult<CallId> {
        // Check if client is running
        if !*self.is_running.read().await {
            return Err(ClientError::InternalError {
                message: "Client is not started. Call start() before making calls.".to_string()
            });
        }
        
        // Create call via session-core with retry logic for network errors
        let prepared_call = retry_with_backoff(
            "prepare_outgoing_call",
            RetryConfig::quick(),
            || async {
                SessionControl::prepare_outgoing_call(
                    &self.coordinator,
                    &from,
                    &to
                )
                .await
                .map_err(|e| ClientError::CallSetupFailed { 
                    reason: format!("Failed to prepare call: {}", e) 
                })
            }
        )
        .await
        .with_context(|| format!("Failed to prepare call from {} to {}", from, to))?;
        
        // Log the allocated RTP port
        tracing::info!("Prepared call with allocated RTP port: {}", prepared_call.local_rtp_port);
        
        // Now initiate the prepared call
        let session = retry_with_backoff(
            "initiate_prepared_call", 
            RetryConfig::quick(),
            || async {
                SessionControl::initiate_prepared_call(
                    &self.coordinator,
                    &prepared_call
                )
                .await
                .map_err(|e| ClientError::CallSetupFailed { 
                    reason: format!("Failed to initiate call: {}", e) 
                })
            }
        )
        .await
        .with_context(|| format!("Failed to initiate call from {} to {}", from, to))?;
            
        // Create call ID and mapping
        let call_id = CallId::new_v4();
        self.call_handler.call_mapping.insert(session.id.clone(), call_id);
        self.session_mapping.insert(call_id, session.id.clone());
        
        // Create enhanced call info
        let mut metadata = HashMap::new();
        metadata.insert("created_via".to_string(), "make_call".to_string());
        if let Some(ref subj) = subject {
            metadata.insert("subject".to_string(), subj.clone());
        }
        
        let call_info = CallInfo {
            call_id,
            state: crate::call::CallState::Initiating,
            direction: CallDirection::Outgoing,
            local_uri: from,
            remote_uri: to,
            remote_display_name: None,
            subject,
            created_at: Utc::now(),
            connected_at: None,
            ended_at: None,
            remote_addr: None,
            media_session_id: None,
            sip_call_id: session.id.0.clone(),
            metadata,
        };
        
        self.call_info.insert(call_id, call_info.clone());
        
        // Emit call created event
        let _ = self.event_tx.send(crate::events::ClientEvent::CallStateChanged {
            info: crate::events::CallStatusInfo {
                call_id,
                new_state: crate::call::CallState::Initiating,
                previous_state: None, // No previous state for new calls
                reason: Some("Call created".to_string()),
                timestamp: Utc::now(),
            },
            priority: crate::events::EventPriority::Normal,
        });
        
        // Update stats
        let mut stats = self.stats.lock().await;
        stats.total_calls += 1;
        
        tracing::info!("Created outgoing call {} -> {} (call_id: {})", 
                      call_info.local_uri, call_info.remote_uri, call_id);
        
        Ok(call_id)
    }
    
    /// Answer an incoming call with SDP negotiation
    /// 
    /// This method accepts an incoming call that was previously stored by the event handler.
    /// It performs SDP offer/answer negotiation and establishes the media session.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the incoming call to answer
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the call was successfully answered and connected.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no incoming call exists with the given ID
    /// * `ClientError::CallSetupFailed` - If SDP negotiation or call setup fails
    /// * `ClientError::InvalidCallState` - If the call is not in an answerable state
    /// 
    /// # Examples
    /// 
    /// Basic call answering:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId, ClientEventHandler, CallAction, IncomingCallInfo};
    /// use std::sync::Arc;
    /// 
    /// struct MyEventHandler;
    /// 
    /// #[async_trait::async_trait]
    /// impl ClientEventHandler for MyEventHandler {
    ///     async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
    ///         // Store call_id for later use
    ///         println!("Incoming call from: {}", call_info.caller_uri);
    ///         CallAction::Ignore // Let application handle it
    ///     }
    ///     
    ///     async fn on_call_state_changed(&self, _info: rvoip_client_core::CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _info: rvoip_client_core::RegistrationStatusInfo) {}
    ///     async fn on_media_event(&self, _info: rvoip_client_core::MediaEventInfo) {}
    ///     async fn on_client_error(&self, _error: rvoip_client_core::ClientError, _call_id: Option<CallId>) {}
    ///     async fn on_network_event(&self, _connected: bool, _reason: Option<String>) {}
    /// }
    /// 
    /// async fn answer_incoming_call(call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5062".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.set_event_handler(Arc::new(MyEventHandler)).await;
    ///     client.start().await?;
    ///     
    ///     // Answer the call (assuming call_id was obtained from event handler)
    ///     client.answer_call(&call_id).await?;
    ///     println!("Successfully answered call: {}", call_id);
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # SDP Negotiation Process
    /// 
    /// 1. Retrieves the stored incoming call information
    /// 2. If an SDP offer was provided, generates an appropriate SDP answer
    /// 3. If no offer was provided, generates an SDP offer (rare case)
    /// 4. Calls session-core to accept the call with the negotiated SDP
    /// 5. Updates call state to Connected and emits events
    /// 
    /// # Thread Safety
    /// 
    /// This method is async and thread-safe. Multiple calls can be answered concurrently.
    pub async fn answer_call(&self, call_id: &CallId) -> ClientResult<()> {
        // Get the stored IncomingCall object
        let incoming_call = self.call_handler.get_incoming_call(call_id)
            .await
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?;
        
        // Generate SDP answer based on the offer
        let sdp_answer = if let Some(offer) = &incoming_call.sdp {
            // Use MediaControl to generate SDP answer
            MediaControl::generate_sdp_answer(
                &self.coordinator,
                &incoming_call.id,
                offer
            )
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to generate SDP answer: {}", e) 
            })?
        } else {
            // No offer provided, generate our own SDP
            MediaControl::generate_sdp_offer(
                &self.coordinator,
                &incoming_call.id
            )
            .await
            .map_err(|e| ClientError::CallSetupFailed { 
                reason: format!("Failed to generate SDP: {}", e) 
            })?
        };
        
        // Use SessionControl to accept the call with SDP answer
        SessionControl::accept_incoming_call(
            &self.coordinator,
            &incoming_call,
            Some(sdp_answer)  // Provide the generated SDP answer
        )
        .await
        .map_err(|e| ClientError::CallSetupFailed { 
            reason: format!("Failed to answer call: {}", e) 
        })?;
        
        // Update call info
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.state = crate::call::CallState::Connected;
            call_info.connected_at = Some(Utc::now());
            call_info.metadata.insert("answered_at".to_string(), Utc::now().to_rfc3339());
        }
        
        // Set up automatic audio frame subscription for the call
        if let Err(e) = self.setup_call_audio(call_id).await {
            // Log the error but don't fail the call - audio might still work
            // through other means or this might be a non-audio call
            tracing::warn!("Failed to set up audio for call {}: {}", call_id, e);
        }
        
        // Update stats
        let mut stats = self.stats.lock().await;
        stats.connected_calls += 1;
        
        tracing::info!("Answered call {}", call_id);
        Ok(())
    }
    
    /// Reject an incoming call with optional reason
    /// 
    /// This method rejects an incoming call that was previously stored by the event handler.
    /// The call will be terminated with a SIP rejection response.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the incoming call to reject
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the call was successfully rejected.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no incoming call exists with the given ID
    /// * `ClientError::CallTerminated` - If the rejection fails to send properly
    /// 
    /// # Examples
    /// 
    /// Basic call rejection:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId, ClientEventHandler, CallAction, IncomingCallInfo};
    /// use std::sync::Arc;
    /// 
    /// struct RejectingEventHandler;
    /// 
    /// #[async_trait::async_trait]
    /// impl ClientEventHandler for RejectingEventHandler {
    ///     async fn on_incoming_call(&self, call_info: IncomingCallInfo) -> CallAction {
    ///         // Automatically reject calls from unknown numbers
    ///         if !call_info.caller_uri.contains("@trusted-domain.com") {
    ///             CallAction::Reject
    ///         } else {
    ///             CallAction::Ignore
    ///         }
    ///     }
    ///     
    ///     async fn on_call_state_changed(&self, _info: rvoip_client_core::CallStatusInfo) {}
    ///     async fn on_registration_status_changed(&self, _info: rvoip_client_core::RegistrationStatusInfo) {}
    ///     async fn on_media_event(&self, _info: rvoip_client_core::MediaEventInfo) {}
    ///     async fn on_client_error(&self, _error: rvoip_client_core::ClientError, _call_id: Option<CallId>) {}
    ///     async fn on_network_event(&self, _connected: bool, _reason: Option<String>) {}
    /// }
    /// 
    /// async fn reject_unwanted_call(call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5063".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.set_event_handler(Arc::new(RejectingEventHandler)).await;
    ///     client.start().await?;
    ///     
    ///     // Reject the call
    ///     client.reject_call(&call_id).await?;
    ///     println!("Successfully rejected call: {}", call_id);
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Call Rejection Process
    /// 
    /// 1. Retrieves the stored incoming call information
    /// 2. Sends a SIP rejection response (typically 603 Decline)
    /// 3. Updates call state to Terminated
    /// 4. Records rejection reason in metadata
    /// 5. Emits appropriate events
    /// 
    /// # SIP Response Codes
    /// 
    /// The rejection will typically result in a SIP 603 "Decline" response being sent
    /// to the caller, indicating that the call was explicitly rejected by the user.
    pub async fn reject_call(&self, call_id: &CallId) -> ClientResult<()> {
        // Get the stored IncomingCall object
        let incoming_call = self.call_handler.get_incoming_call(call_id)
            .await
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?;
        
        // Use SessionControl to reject the call
        SessionControl::reject_incoming_call(
            &self.coordinator,
            &incoming_call,
            "User rejected"
        )
        .await
        .map_err(|e| ClientError::CallTerminated { 
            reason: format!("Failed to reject call: {}", e) 
        })?;
        
        // Update call info
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.state = crate::call::CallState::Terminated;
            call_info.ended_at = Some(Utc::now());
            call_info.metadata.insert("rejected_at".to_string(), Utc::now().to_rfc3339());
            call_info.metadata.insert("rejection_reason".to_string(), "user_rejected".to_string());
        }
        
        tracing::info!("Rejected call {}", call_id);
        Ok(())
    }
    
    /// Terminate an active call (hang up)
    /// 
    /// This method terminates any call regardless of its current state. It handles
    /// proper session cleanup and state management.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to terminate
    /// 
    /// # Returns
    /// 
    /// Returns `Ok(())` if the call was successfully terminated or was already terminated.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no call exists with the given ID
    /// * `ClientError::CallTerminated` - If the termination process fails
    /// 
    /// # Examples
    /// 
    /// Basic call hangup:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn hangup_active_call(call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5064".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     // Terminate the call
    ///     client.hangup_call(&call_id).await?;
    ///     println!("Successfully hung up call: {}", call_id);
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// Hangup with error handling:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId, ClientError};
    /// 
    /// async fn safe_hangup(call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5065".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     match client.hangup_call(&call_id).await {
    ///         Ok(()) => {
    ///             println!("Call terminated successfully");
    ///         }
    ///         Err(ClientError::CallNotFound { .. }) => {
    ///             println!("Call was already terminated or doesn't exist");
    ///         }
    ///         Err(e) => {
    ///             eprintln!("Failed to hangup call: {}", e);
    ///             return Err(e.into());
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Termination Process
    /// 
    /// 1. Locates the session associated with the call
    /// 2. Checks current call state (skips if already terminated)
    /// 3. Calls session-core to terminate the SIP session
    /// 4. Updates call state to Terminated
    /// 5. Updates statistics and emits events
    /// 
    /// # Idempotent Operation
    /// 
    /// This method is idempotent - calling it multiple times on the same call
    /// will not cause errors. If the call is already terminated, it will return
    /// success immediately.
    pub async fn hangup_call(&self, call_id: &CallId) -> ClientResult<()> {
        let session_id = self.session_mapping.get(call_id)
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?
            .clone();
            
        // Check the current call state first
        if let Some(call_info) = self.call_info.get(call_id) {
            match call_info.state {
                crate::call::CallState::Terminated |
                crate::call::CallState::Failed |
                crate::call::CallState::Cancelled => {
                    tracing::info!("Call {} is already terminated (state: {:?}), skipping hangup", 
                                 call_id, call_info.state);
                    return Ok(());
                }
                _ => {
                    // Proceed with termination for other states
                }
            }
        }
            
        // Terminate the session using SessionControl trait
        match SessionControl::terminate_session(&self.coordinator, &session_id).await {
            Ok(()) => {
                tracing::info!("Successfully terminated session for call {}", call_id);
            }
            Err(e) => {
                // Check if the error is because the session is already terminated
                let error_msg = e.to_string();
                if error_msg.contains("No INVITE transaction found") || 
                   error_msg.contains("already terminated") ||
                   error_msg.contains("already in state") {
                    tracing::warn!("Session already terminated for call {}: {}", call_id, error_msg);
                    // Continue to update our internal state even if session is already gone
                } else {
                    return Err(ClientError::CallTerminated { 
                        reason: format!("Failed to hangup call: {}", e) 
                    });
                }
            }
        }
        
        // Update call info
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            let old_state = call_info.state.clone();
            call_info.state = crate::call::CallState::Terminated;
            call_info.ended_at = Some(Utc::now());
            call_info.metadata.insert("hangup_at".to_string(), Utc::now().to_rfc3339());
            call_info.metadata.insert("hangup_reason".to_string(), "user_hangup".to_string());
            
            // Emit state change event
            let _ = self.event_tx.send(crate::events::ClientEvent::CallStateChanged {
                info: crate::events::CallStatusInfo {
                    call_id: *call_id,
                    new_state: crate::call::CallState::Terminated,
                    previous_state: Some(old_state),
                    reason: Some("User hangup".to_string()),
                    timestamp: Utc::now(),
                },
                priority: crate::events::EventPriority::Normal,
            });
        }
        
        // Clean up audio setup state if it exists
        self.audio_setup_calls.remove(call_id);
        
        // Update stats - use saturating_sub to prevent integer underflow
        let mut stats = self.stats.lock().await;
        stats.connected_calls = stats.connected_calls.saturating_sub(1);
        
        tracing::info!("Hung up call {}", call_id);
        Ok(())
    }
    
    /// Get basic information about a specific call
    /// 
    /// Retrieves the current state and metadata for a call by its ID.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to query
    /// 
    /// # Returns
    /// 
    /// Returns a `CallInfo` struct containing all information about the call.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no call exists with the given ID
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId, CallState};
    /// 
    /// async fn check_call_status(call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5066".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let call_info = client.get_call(&call_id).await?;
    ///     
    ///     println!("Call ID: {}", call_info.call_id);
    ///     println!("State: {:?}", call_info.state);
    ///     println!("From: {}", call_info.local_uri);
    ///     println!("To: {}", call_info.remote_uri);
    ///     
    ///     if let Some(connected_at) = call_info.connected_at {
    ///         println!("Connected at: {}", connected_at);
    ///     }
    ///     
    ///     match call_info.state {
    ///         CallState::Connected => println!("Call is active"),
    ///         CallState::Terminated => println!("Call has ended"),
    ///         _ => println!("Call is in progress"),
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_call(&self, call_id: &CallId) -> ClientResult<CallInfo> {
        self.call_info.get(call_id)
            .map(|entry| entry.value().clone())
            .ok_or(ClientError::CallNotFound { call_id: *call_id })
    }
    
    /// Get detailed call information with enhanced metadata
    /// 
    /// Retrieves comprehensive information about a call including session metadata
    /// and real-time statistics.
    /// 
    /// # Arguments
    /// 
    /// * `call_id` - The unique identifier of the call to query
    /// 
    /// # Returns
    /// 
    /// Returns a `CallInfo` struct with additional metadata fields populated.
    /// 
    /// # Errors
    /// 
    /// * `ClientError::CallNotFound` - If no call exists with the given ID
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallId};
    /// 
    /// async fn get_detailed_call_info(call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5067".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let detailed_info = client.get_call_detailed(&call_id).await?;
    ///     
    ///     println!("Call Details:");
    ///     println!("  ID: {}", detailed_info.call_id);
    ///     println!("  SIP Call-ID: {}", detailed_info.sip_call_id);
    ///     
    ///     // Check enhanced metadata
    ///     for (key, value) in &detailed_info.metadata {
    ///         println!("  {}: {}", key, value);
    ///     }
    ///     
    ///     if let Some(session_id) = detailed_info.metadata.get("session_id") {
    ///         println!("  Session tracking: {}", session_id);
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Enhanced Metadata
    /// 
    /// The detailed call information includes additional metadata fields:
    /// 
    /// - `session_id` - The internal session-core session identifier
    /// - `last_updated` - ISO 8601 timestamp of the last metadata update
    /// - Plus any existing metadata from the basic call info
    pub async fn get_call_detailed(&self, call_id: &CallId) -> ClientResult<CallInfo> {
        // Get base call info
        let mut call_info = self.call_info.get(call_id)
            .map(|entry| entry.value().clone())
            .ok_or(ClientError::CallNotFound { call_id: *call_id })?;
            
        // Add session metadata if available
        if let Some(session_id) = self.session_mapping.get(call_id) {
            call_info.metadata.insert("session_id".to_string(), session_id.0.clone());
            call_info.metadata.insert("last_updated".to_string(), Utc::now().to_rfc3339());
        }
        
        Ok(call_info)
    }
    
    /// List all calls (active and historical)
    /// 
    /// Returns a vector of all calls known to the client, regardless of their state.
    /// This includes active calls, completed calls, and failed calls.
    /// 
    /// # Returns
    /// 
    /// Returns a `Vec<CallInfo>` containing all calls. The list is not sorted.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallState};
    /// 
    /// async fn review_all_calls() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5068".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let all_calls = client.list_calls().await;
    ///     
    ///     println!("Total calls: {}", all_calls.len());
    ///     
    ///     for call in all_calls {
    ///         println!("Call {}: {} -> {} ({:?})", 
    ///                  call.call_id, 
    ///                  call.local_uri, 
    ///                  call.remote_uri, 
    ///                  call.state);
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Performance Note
    /// 
    /// This method iterates through all stored calls. For applications with
    /// many historical calls, consider using filtered methods like
    /// `get_active_calls()` or `get_calls_by_state()` instead.
    pub async fn list_calls(&self) -> Vec<CallInfo> {
        self.call_info.iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get calls filtered by state
    /// 
    /// Returns all calls that are currently in the specified state.
    /// 
    /// # Arguments
    /// 
    /// * `state` - The call state to filter by
    /// 
    /// # Returns
    /// 
    /// Returns a `Vec<CallInfo>` containing all calls in the specified state.
    /// 
    /// # Examples
    /// 
    /// Get all connected calls:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallState};
    /// 
    /// async fn list_connected_calls() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5069".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let connected_calls = client.get_calls_by_state(CallState::Connected).await;
    ///     
    ///     println!("Currently connected calls: {}", connected_calls.len());
    ///     for call in connected_calls {
    ///         if let Some(connected_at) = call.connected_at {
    ///             let duration = chrono::Utc::now().signed_duration_since(connected_at);
    ///             println!("Call {}: {} minutes active", 
    ///                      call.call_id, 
    ///                      duration.num_minutes());
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// Get all failed calls:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallState};
    /// 
    /// async fn review_failed_calls() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5070".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let failed_calls = client.get_calls_by_state(CallState::Failed).await;
    ///     
    ///     for call in failed_calls {
    ///         println!("Failed call: {} -> {}", call.local_uri, call.remote_uri);
    ///         if let Some(reason) = call.metadata.get("failure_reason") {
    ///             println!("  Reason: {}", reason);
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_calls_by_state(&self, state: crate::call::CallState) -> Vec<CallInfo> {
        self.call_info.iter()
            .filter(|entry| entry.value().state == state)
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get calls filtered by direction (incoming or outgoing)
    /// 
    /// Returns all calls that match the specified direction.
    /// 
    /// # Arguments
    /// 
    /// * `direction` - The call direction to filter by (`Incoming` or `Outgoing`)
    /// 
    /// # Returns
    /// 
    /// Returns a `Vec<CallInfo>` containing all calls with the specified direction.
    /// 
    /// # Examples
    /// 
    /// Get all outgoing calls:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallDirection};
    /// 
    /// async fn review_outgoing_calls() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5071".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let outgoing_calls = client.get_calls_by_direction(CallDirection::Outgoing).await;
    ///     
    ///     println!("Outgoing calls made: {}", outgoing_calls.len());
    ///     for call in outgoing_calls {
    ///         println!("Called: {} at {}", call.remote_uri, call.created_at);
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// Get all incoming calls:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallDirection};
    /// 
    /// async fn review_incoming_calls() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5072".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let incoming_calls = client.get_calls_by_direction(CallDirection::Incoming).await;
    ///     
    ///     println!("Calls received: {}", incoming_calls.len());
    ///     for call in incoming_calls {
    ///         println!("From: {} ({})", 
    ///                  call.remote_display_name.as_deref().unwrap_or("Unknown"),
    ///                  call.remote_uri);
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    pub async fn get_calls_by_direction(&self, direction: CallDirection) -> Vec<CallInfo> {
        self.call_info.iter()
            .filter(|entry| entry.value().direction == direction)
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get call history (completed and terminated calls)
    /// 
    /// Returns all calls that have finished, regardless of how they ended.
    /// This includes successfully completed calls, failed calls, and cancelled calls.
    /// 
    /// # Returns
    /// 
    /// Returns a `Vec<CallInfo>` containing all terminated calls.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallState};
    /// 
    /// async fn generate_call_report() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5073".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let history = client.get_call_history().await;
    ///     
    ///     let mut completed = 0;
    ///     let mut failed = 0;
    ///     let mut cancelled = 0;
    ///     let mut total_duration = chrono::Duration::zero();
    ///     
    ///     for call in history {
    ///         match call.state {
    ///             CallState::Terminated => {
    ///                 completed += 1;
    ///                 if let (Some(connected), Some(ended)) = (call.connected_at, call.ended_at) {
    ///                     total_duration = total_duration + ended.signed_duration_since(connected);
    ///                 }
    ///             }
    ///             CallState::Failed => failed += 1,
    ///             CallState::Cancelled => cancelled += 1,
    ///             _ => {} // Should not happen in history
    ///         }
    ///     }
    ///     
    ///     println!("Call History Summary:");
    ///     println!("  Completed: {}", completed);
    ///     println!("  Failed: {}", failed);
    ///     println!("  Cancelled: {}", cancelled);
    ///     println!("  Total talk time: {} minutes", total_duration.num_minutes());
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Use Cases
    /// 
    /// - Call reporting and analytics
    /// - Billing and usage tracking
    /// - Debugging call quality issues
    /// - User activity monitoring
    pub async fn get_call_history(&self) -> Vec<CallInfo> {
        self.call_info.iter()
            .filter(|entry| {
                matches!(entry.value().state, 
                    crate::call::CallState::Terminated |
                    crate::call::CallState::Failed |
                    crate::call::CallState::Cancelled
                )
            })
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get all currently active calls
    /// 
    /// Returns all calls that are not in a terminated state. This includes
    /// calls that are connecting, ringing, connected, or in any other non-final state.
    /// 
    /// # Returns
    /// 
    /// Returns a `Vec<CallInfo>` containing all active calls.
    /// 
    /// # Examples
    /// 
    /// Monitor active calls:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig, CallState};
    /// 
    /// async fn monitor_active_calls() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5074".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let active_calls = client.get_active_calls().await;
    ///     
    ///     if active_calls.is_empty() {
    ///         println!("No active calls");
    ///     } else {
    ///         println!("Active calls: {}", active_calls.len());
    ///         
    ///         for call in active_calls {
    ///             match call.state {
    ///                 CallState::Initiating => {
    ///                     println!("📞 Dialing {} -> {}", call.local_uri, call.remote_uri);
    ///                 }
    ///                 CallState::Ringing => {
    ///                     println!("📳 Ringing {} -> {}", call.local_uri, call.remote_uri);
    ///                 }
    ///                 CallState::Connected => {
    ///                     if let Some(connected_at) = call.connected_at {
    ///                         let duration = chrono::Utc::now().signed_duration_since(connected_at);
    ///                         println!("☎️  Connected {} -> {} ({}:{})", 
    ///                                  call.local_uri, call.remote_uri,
    ///                                  duration.num_minutes(), 
    ///                                  duration.num_seconds() % 60);
    ///                     }
    ///                 }
    ///                 _ => {
    ///                     println!("📱 {} -> {} ({:?})", call.local_uri, call.remote_uri, call.state);
    ///                 }
    ///             }
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Real-time Monitoring
    /// 
    /// This method is useful for:
    /// - Dashboard displays showing current call status
    /// - Resource management (checking call limits)
    /// - User interface updates
    /// - Load balancing decisions
    pub async fn get_active_calls(&self) -> Vec<CallInfo> {
        self.call_info.iter()
            .filter(|entry| {
                !matches!(entry.value().state, 
                    crate::call::CallState::Terminated |
                    crate::call::CallState::Failed |
                    crate::call::CallState::Cancelled
                )
            })
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get comprehensive client statistics
    /// 
    /// Returns detailed statistics about the client's call activity and performance.
    /// The statistics are recalculated on each call to ensure accuracy.
    /// 
    /// # Returns
    /// 
    /// Returns a `ClientStats` struct containing:
    /// - Total number of calls ever made/received
    /// - Currently connected calls count
    /// - Other performance metrics
    /// 
    /// # Examples
    /// 
    /// Basic statistics display:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// 
    /// async fn display_client_stats() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5075".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let stats = client.get_client_stats().await;
    ///     
    ///     println!("Client Statistics:");
    ///     println!("  Total calls: {}", stats.total_calls);
    ///     println!("  Connected calls: {}", stats.connected_calls);
    ///     println!("  Utilization: {:.1}%", 
    ///              if stats.total_calls > 0 {
    ///                  (stats.connected_calls as f64 / stats.total_calls as f64) * 100.0
    ///              } else {
    ///                  0.0
    ///              });
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// Monitoring loop:
    /// ```rust
    /// use rvoip_client_core::{ClientManager, ClientConfig};
    /// use tokio::time::{interval, Duration};
    /// 
    /// async fn monitor_client_performance() -> Result<(), Box<dyn std::error::Error>> {
    ///     let config = ClientConfig::new()
    ///         .with_sip_addr("127.0.0.1:5076".parse()?);
    ///     let client = ClientManager::new(config).await?;
    ///     client.start().await?;
    ///     
    ///     let mut interval = interval(Duration::from_secs(30));
    ///     
    ///     // Monitor for a limited time in doc test
    ///     for _ in 0..3 {
    ///         interval.tick().await;
    ///         let stats = client.get_client_stats().await;
    ///         
    ///         println!("📊 Stats: {} total, {} active", 
    ///                  stats.total_calls, stats.connected_calls);
    ///         
    ///         if stats.connected_calls > 10 {
    ///             println!("⚠️  High call volume detected");
    ///         }
    ///     }
    ///     
    ///     Ok(())
    /// }
    /// ```
    /// 
    /// # Accuracy Guarantee
    /// 
    /// This method recalculates statistics from the actual call states rather than
    /// relying on potentially inconsistent counters. This prevents issues with:
    /// - Race conditions in concurrent call handling
    /// - Integer overflow/underflow
    /// - Inconsistent state after error recovery
    /// 
    /// # Performance Note
    /// 
    /// The recalculation involves iterating through all calls, so for applications
    /// with very large call histories, consider calling this method judiciously.
    pub async fn get_client_stats(&self) -> ClientStats {
        let mut stats = self.stats.lock().await.clone();
        
        // Always recalculate call counts from actual call states to avoid counter bugs
        // This prevents integer overflow/underflow issues from race conditions
        let _active_calls = self.get_active_calls().await;
        let connected_calls = self.get_calls_by_state(crate::call::CallState::Connected).await;
        
        // Use actual counts instead of potentially corrupted stored counters
        stats.connected_calls = connected_calls.len();
        stats.total_calls = self.call_info.len();
        
        // Ensure connected_calls never exceeds total_calls (defensive programming)
        if stats.connected_calls > stats.total_calls {
            tracing::warn!("Connected calls ({}) exceeded total calls ({}), correcting to total", 
                         stats.connected_calls, stats.total_calls);
            stats.connected_calls = stats.total_calls;
        }
        
        stats
    }
}
