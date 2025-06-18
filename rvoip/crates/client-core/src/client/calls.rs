//! Call operations for the client-core library
//! 
//! This module contains all call-related operations including making calls,
//! answering, rejecting, hanging up, and querying call information.

use std::collections::HashMap;
use chrono::Utc;

// Import session-core APIs
use rvoip_session_core::api::{
    SessionControl,
};

// Import client-core types
use crate::{
    ClientResult, ClientError,
    call::{CallId, CallInfo, CallDirection},
};

use super::types::*;

/// Call operations implementation for ClientManager
impl super::manager::ClientManager {
    /// Make an outgoing call with enhanced information tracking
    pub async fn make_call(
        &self,
        from: String,
        to: String,
        subject: Option<String>,
    ) -> ClientResult<CallId> {
        // Create call via session-core using SessionControl trait
        let session = SessionControl::create_outgoing_call(
            &self.coordinator,
            &from,
            &to,
            None  // Let session-core generate SDP
        )
        .await
        .map_err(|e| ClientError::CallSetupFailed { 
            reason: format!("Session creation failed: {}", e) 
        })?;
            
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
        
        // Use SessionControl to accept the call
        SessionControl::accept_incoming_call(
            &self.coordinator,
            &incoming_call,
            None  // Let session-core generate SDP answer
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
            
        // Terminate the session using SessionControl trait
        SessionControl::terminate_session(&self.coordinator, &session_id)
            .await
            .map_err(|e| ClientError::CallTerminated { 
                reason: format!("Failed to hangup call: {}", e) 
            })?;
            
        // Update call info
        if let Some(mut call_info) = self.call_info.get_mut(call_id) {
            call_info.state = crate::call::CallState::Terminated;
            call_info.ended_at = Some(Utc::now());
            call_info.metadata.insert("hangup_at".to_string(), Utc::now().to_rfc3339());
            call_info.metadata.insert("hangup_reason".to_string(), "user_hangup".to_string());
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
