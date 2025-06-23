//! Call operations for the client-core library
//! 
//! This module contains all call-related operations including making calls,
//! answering, rejecting, hanging up, and querying call information.

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
    /// ```rust,no_run
    /// # use rvoip_client_core::{Client, CallId};
    /// # use std::sync::Arc;
    /// # async fn example(client: Arc<Client>) -> Result<CallId, Box<dyn std::error::Error>> {
    /// let call_id = client.make_call(
    ///     "sip:alice@ourcompany.com".to_string(),
    ///     "sip:bob@example.com".to_string(),
    ///     None,
    /// ).await?;
    /// 
    /// println!("Outgoing call started: {}", call_id);
    /// # Ok(call_id)
    /// # }
    /// ```
    /// 
    /// Call with subject:
    /// ```rust,no_run
    /// # use rvoip_client_core::{Client, CallId};
    /// # use std::sync::Arc;
    /// # async fn example(client: Arc<Client>) -> Result<CallId, Box<dyn std::error::Error>> {
    /// let call_id = client.make_call(
    ///     "sip:support@ourcompany.com".to_string(),
    ///     "sip:customer@example.com".to_string(),
    ///     Some("Technical Support Call".to_string()),
    /// ).await?;
    /// # Ok(call_id)
    /// # }
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
        let session = retry_with_backoff(
            "create_outgoing_call",
            RetryConfig::quick(),
            || async {
                SessionControl::create_outgoing_call(
                    &self.coordinator,
                    &from,
                    &to,
                    None  // Let session-core generate SDP
                )
                .await
                .map_err(|e| ClientError::CallSetupFailed { 
                    reason: format!("Session creation failed: {}", e) 
                })
            }
        )
        .await
        .with_context(|| format!("Failed to create call from {} to {}", from, to))?;
            
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
    
    /// Answer an incoming call
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
        
        // Update stats
        let mut stats = self.stats.lock().await;
        stats.connected_calls += 1;
        
        tracing::info!("Answered call {}", call_id);
        Ok(())
    }
    
    /// Reject an incoming call
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
    
    /// Hang up a call
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
        
        // Update stats
        let mut stats = self.stats.lock().await;
        if stats.connected_calls > 0 {
            stats.connected_calls -= 1;
        }
        
        tracing::info!("Hung up call {}", call_id);
        Ok(())
    }
    
    /// Get information about a call
    pub async fn get_call(&self, call_id: &CallId) -> ClientResult<CallInfo> {
        self.call_info.get(call_id)
            .map(|entry| entry.value().clone())
            .ok_or(ClientError::CallNotFound { call_id: *call_id })
    }
    
    /// Get detailed call information with enhanced metadata
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
    pub async fn list_calls(&self) -> Vec<CallInfo> {
        self.call_info.iter()
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get calls by state
    pub async fn get_calls_by_state(&self, state: crate::call::CallState) -> Vec<CallInfo> {
        self.call_info.iter()
            .filter(|entry| entry.value().state == state)
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get calls by direction
    pub async fn get_calls_by_direction(&self, direction: CallDirection) -> Vec<CallInfo> {
        self.call_info.iter()
            .filter(|entry| entry.value().direction == direction)
            .map(|entry| entry.value().clone())
            .collect()
    }
    
    /// Get call history (ended calls)
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
    
    /// Get active calls (not terminated)
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
    
    /// Get client statistics
    pub async fn get_client_stats(&self) -> ClientStats {
        let mut stats = self.stats.lock().await.clone();
        
        // Update with current call counts
        let _active_calls = self.get_active_calls().await;
        let connected_calls = self.get_calls_by_state(crate::call::CallState::Connected).await;
        
        stats.connected_calls = connected_calls.len();
        stats.total_calls = self.call_info.len();
        
        stats
    }
}
