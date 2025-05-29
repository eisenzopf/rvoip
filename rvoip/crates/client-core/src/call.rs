//! Call management for SIP client
//!
//! This module handles individual call lifecycle, state management, and coordination
//! with the underlying rvoip infrastructure (transaction-core, media-core, etc.)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use chrono::{DateTime, Utc};

use rvoip_transaction_core::TransactionManager;
use rvoip_media_core::MediaEngine;

use crate::error::{ClientResult, ClientError};

/// Unique identifier for a call
pub type CallId = Uuid;

/// Current state of a call
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallState {
    /// Call is being initiated (sending INVITE)
    Initiating,
    /// Received 100 Trying or similar provisional response
    Proceeding,
    /// Received 180 Ringing
    Ringing,
    /// Call is connected and media is flowing
    Connected,
    /// Call is being terminated (sending/received BYE)
    Terminating,
    /// Call has ended
    Terminated,
    /// Call failed to establish
    Failed,
    /// Call was cancelled before connection
    Cancelled,
    /// Incoming call waiting for user decision
    IncomingPending,
}

impl CallState {
    /// Check if the call is in an active state (can send/receive media)
    pub fn is_active(&self) -> bool {
        matches!(self, CallState::Connected)
    }

    /// Check if the call is in a terminated state
    pub fn is_terminated(&self) -> bool {
        matches!(
            self,
            CallState::Terminated | CallState::Failed | CallState::Cancelled
        )
    }

    /// Check if the call is still in progress
    pub fn is_in_progress(&self) -> bool {
        !self.is_terminated()
    }
}

/// Direction of a call (from client's perspective)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CallDirection {
    /// Outgoing call (client initiated)
    Outgoing,
    /// Incoming call (received from network)
    Incoming,
}

/// Information about a SIP call
#[derive(Debug, Clone)]
pub struct CallInfo {
    /// Unique call identifier
    pub call_id: CallId,
    /// Current state of the call
    pub state: CallState,
    /// Direction of the call
    pub direction: CallDirection,
    /// Local party URI (our user)
    pub local_uri: String,
    /// Remote party URI (who we're calling/called by)
    pub remote_uri: String,
    /// Display name of remote party (if available)
    pub remote_display_name: Option<String>,
    /// Call subject/reason
    pub subject: Option<String>,
    /// When the call was created
    pub created_at: DateTime<Utc>,
    /// When the call was connected (if applicable)
    pub connected_at: Option<DateTime<Utc>>,
    /// When the call ended (if applicable)
    pub ended_at: Option<DateTime<Utc>>,
    /// Remote network address
    pub remote_addr: Option<SocketAddr>,
    /// Associated media session ID (if any)
    pub media_session_id: Option<String>,
    /// SIP Call-ID header value
    pub sip_call_id: String,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Internal call management structure
#[derive(Debug)]
struct ActiveCall {
    /// Basic call information
    pub info: CallInfo,
    /// Associated transaction ID for SIP messages
    pub transaction_id: Option<String>,
    /// State change history (for debugging)
    pub state_history: Vec<(CallState, DateTime<Utc>)>,
}

impl ActiveCall {
    /// Create a new active call
    fn new(info: CallInfo) -> Self {
        let mut state_history = Vec::new();
        state_history.push((info.state.clone(), info.created_at));

        Self {
            info,
            transaction_id: None,
            state_history,
        }
    }

    /// Update the call state and record the change
    fn update_state(&mut self, new_state: CallState) {
        if self.info.state != new_state {
            self.info.state = new_state.clone();
            self.state_history.push((new_state.clone(), Utc::now()));

            // Update timestamps based on state
            match new_state {
                CallState::Connected => {
                    if self.info.connected_at.is_none() {
                        self.info.connected_at = Some(Utc::now());
                    }
                }
                CallState::Terminated | CallState::Failed | CallState::Cancelled => {
                    if self.info.ended_at.is_none() {
                        self.info.ended_at = Some(Utc::now());
                    }
                }
                _ => {}
            }
        }
    }
}

/// Call manager handles individual call lifecycle and state
pub struct CallManager {
    /// Active calls indexed by call ID
    active_calls: Arc<RwLock<HashMap<CallId, ActiveCall>>>,
    /// Reference to transaction manager (reused from server infrastructure)
    transaction_manager: Arc<TransactionManager>,
    /// Reference to media manager (reused from server infrastructure)
    media_manager: Arc<MediaEngine>,
}

impl CallManager {
    /// Create a new call manager
    pub fn new(
        transaction_manager: Arc<TransactionManager>,
        media_manager: Arc<MediaEngine>,
    ) -> Self {
        Self {
            active_calls: Arc::new(RwLock::new(HashMap::new())),
            transaction_manager,
            media_manager,
        }
    }

    /// Create a new outgoing call
    pub async fn create_outgoing_call(
        &self,
        local_uri: String,
        remote_uri: String,
        subject: Option<String>,
    ) -> ClientResult<CallId> {
        let call_id = Uuid::new_v4();
        let sip_call_id = format!("{}@client", call_id);

        let call_info = CallInfo {
            call_id,
            state: CallState::Initiating,
            direction: CallDirection::Outgoing,
            local_uri,
            remote_uri,
            remote_display_name: None,
            subject,
            created_at: Utc::now(),
            connected_at: None,
            ended_at: None,
            remote_addr: None,
            media_session_id: None,
            sip_call_id,
            metadata: HashMap::new(),
        };

        let active_call = ActiveCall::new(call_info);
        
        {
            let mut calls = self.active_calls.write().await;
            calls.insert(call_id, active_call);
        }

        Ok(call_id)
    }

    /// Create a new incoming call
    pub async fn create_incoming_call(
        &self,
        local_uri: String,
        remote_uri: String,
        remote_display_name: Option<String>,
        subject: Option<String>,
        remote_addr: SocketAddr,
        sip_call_id: String,
    ) -> ClientResult<CallId> {
        let call_id = Uuid::new_v4();

        let call_info = CallInfo {
            call_id,
            state: CallState::IncomingPending,
            direction: CallDirection::Incoming,
            local_uri,
            remote_uri,
            remote_display_name,
            subject,
            created_at: Utc::now(),
            connected_at: None,
            ended_at: None,
            remote_addr: Some(remote_addr),
            media_session_id: None,
            sip_call_id,
            metadata: HashMap::new(),
        };

        let active_call = ActiveCall::new(call_info);
        
        {
            let mut calls = self.active_calls.write().await;
            calls.insert(call_id, active_call);
        }

        Ok(call_id)
    }

    /// Get call information
    pub async fn get_call(&self, call_id: &CallId) -> ClientResult<CallInfo> {
        let calls = self.active_calls.read().await;
        calls
            .get(call_id)
            .map(|call| call.info.clone())
            .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })
    }

    /// Update call state
    pub async fn update_call_state(
        &self,
        call_id: &CallId,
        new_state: CallState,
    ) -> ClientResult<()> {
        let mut calls = self.active_calls.write().await;
        
        if let Some(call) = calls.get_mut(call_id) {
            call.update_state(new_state);
            Ok(())
        } else {
            Err(ClientError::CallNotFound { call_id: *call_id })
        }
    }

    /// Answer an incoming call
    pub async fn answer_call(&self, call_id: &CallId) -> ClientResult<()> {
        // TODO: Implement call answering logic
        // 1. Verify call exists and is in IncomingPending state
        // 2. Create media session via media_manager
        // 3. Send 200 OK via transaction_manager
        // 4. Update call state to Connected

        self.update_call_state(call_id, CallState::Connected).await?;
        Ok(())
    }

    /// Reject an incoming call
    pub async fn reject_call(&self, call_id: &CallId) -> ClientResult<()> {
        // TODO: Implement call rejection logic
        // 1. Verify call exists and is in appropriate state
        // 2. Send 4xx response via transaction_manager
        // 3. Update call state to Terminated

        self.update_call_state(call_id, CallState::Terminated).await?;
        Ok(())
    }

    /// Hangup an active call
    pub async fn hangup_call(&self, call_id: &CallId) -> ClientResult<()> {
        // TODO: Implement call hangup logic
        // 1. Verify call exists
        // 2. Send BYE via transaction_manager
        // 3. Clean up media session via media_manager
        // 4. Update call state to Terminating

        self.update_call_state(call_id, CallState::Terminating).await?;
        Ok(())
    }

    /// List all active calls
    pub async fn list_calls(&self) -> Vec<CallInfo> {
        let calls = self.active_calls.read().await;
        calls.values().map(|call| call.info.clone()).collect()
    }

    /// Get calls by state
    pub async fn get_calls_by_state(&self, state: CallState) -> Vec<CallInfo> {
        let calls = self.active_calls.read().await;
        calls
            .values()
            .filter(|call| call.info.state == state)
            .map(|call| call.info.clone())
            .collect()
    }

    /// Remove a terminated call from active list
    pub async fn remove_call(&self, call_id: &CallId) -> ClientResult<CallInfo> {
        let mut calls = self.active_calls.write().await;
        
        if let Some(call) = calls.remove(call_id) {
            Ok(call.info)
        } else {
            Err(ClientError::CallNotFound { call_id: *call_id })
        }
    }

    /// Get call statistics
    pub async fn get_call_stats(&self) -> CallStats {
        let calls = self.active_calls.read().await;
        let total_calls = calls.len();
        let active_calls = calls.values().filter(|c| c.info.state.is_active()).count();
        let incoming_pending = calls
            .values()
            .filter(|c| c.info.state == CallState::IncomingPending)
            .count();

        CallStats {
            total_active_calls: total_calls,
            connected_calls: active_calls,
            incoming_pending_calls: incoming_pending,
        }
    }
}

/// Statistics about current calls
#[derive(Debug, Clone)]
pub struct CallStats {
    pub total_active_calls: usize,
    pub connected_calls: usize,
    pub incoming_pending_calls: usize,
} 