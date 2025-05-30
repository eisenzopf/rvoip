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
use tracing::{info, debug, warn, error};

use rvoip_transaction_core::TransactionManager;
use rvoip_media_core::MediaEngine;
use rvoip_sip_core::{Request, Response, StatusCode, HeaderName};
use rvoip_sip_core::types::headers::HeaderAccess;

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
    /// Manager state
    is_running: Arc<RwLock<bool>>,
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
            is_running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the call manager
    pub async fn start(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if *running {
            return Ok(());
        }

        info!("▶️ Starting CallManager");
        *running = true;
        info!("✅ CallManager started");
        Ok(())
    }

    /// Stop the call manager
    pub async fn stop(&self) -> ClientResult<()> {
        let mut running = self.is_running.write().await;
        if !*running {
            return Ok(());
        }

        info!("🛑 Stopping CallManager");

        // Hangup all active calls
        let call_ids: Vec<CallId> = {
            let calls = self.active_calls.read().await;
            calls.keys().cloned().collect()
        };

        for call_id in call_ids {
            if let Err(e) = self.hangup_call(&call_id).await {
                warn!("Failed to hangup call {}: {}", call_id, e);
            }
        }

        *running = false;
        info!("✅ CallManager stopped");
        Ok(())
    }

    /// Handle incoming INVITE request
    pub async fn handle_incoming_invite(&self, request: Request) -> ClientResult<()> {
        debug!("📞 Handling incoming INVITE from {}", 
               request.raw_header_value(&HeaderName::From).unwrap_or_else(|| "unknown".to_string()));

        // Extract call information from INVITE
        let call_id_header = request.raw_header_value(&HeaderName::CallId)
            .ok_or_else(|| ClientError::protocol_error("INVITE missing Call-ID header"))?;
        
        let from_header = request.raw_header_value(&HeaderName::From)
            .ok_or_else(|| ClientError::protocol_error("INVITE missing From header"))?;
        
        let to_header = request.raw_header_value(&HeaderName::To)
            .ok_or_else(|| ClientError::protocol_error("INVITE missing To header"))?;

        // Create incoming call
        let call_id = self.create_incoming_call(
            to_header,
            from_header,
            None, // TODO: Parse display name from From header
            None, // TODO: Parse subject from headers
            "127.0.0.1:5060".parse().unwrap(), // TODO: Get actual remote address
            call_id_header,
        ).await?;

        info!("📞 Created incoming call {} from {}", call_id, request.raw_header_value(&HeaderName::From).unwrap_or_default());

        // Send 100 Trying immediately
        self.send_provisional_response(&request, StatusCode::Trying, "Trying").await?;

        // Send 180 Ringing
        self.send_provisional_response(&request, StatusCode::Ringing, "Ringing").await?;
        self.update_call_state(&call_id, CallState::Ringing).await?;

        // TODO: Emit incoming call event to UI
        // TODO: Set up timer for no-answer scenarios

        info!("✅ Incoming INVITE handled, call in ringing state");
        Ok(())
    }

    /// Handle incoming BYE request
    pub async fn handle_incoming_bye(&self, request: Request) -> ClientResult<()> {
        debug!("📴 Handling incoming BYE");

        let call_id_header = request.raw_header_value(&HeaderName::CallId)
            .ok_or_else(|| ClientError::protocol_error("BYE missing Call-ID header"))?;

        // Find the call by SIP Call-ID
        let call_id = self.find_call_by_sip_call_id(&call_id_header).await;

        if let Some(call_id) = call_id {
            info!("📴 Processing BYE for call {}", call_id);
            
            // Update call state
            self.update_call_state(&call_id, CallState::Terminated).await?;
            
            // Send 200 OK response
            self.send_final_response(&request, StatusCode::Ok, "OK").await?;
            
            // Clean up media session
            // TODO: Stop media session via media_manager
            
            // Remove from active calls
            self.remove_call(&call_id).await?;
            
            info!("✅ Call {} terminated by remote party", call_id);
        } else {
            warn!("⚠️ Received BYE for unknown call: {}", call_id_header);
            // Still send 200 OK to be polite
            self.send_final_response(&request, StatusCode::Ok, "OK").await?;
        }

        Ok(())
    }

    /// Handle incoming ACK request
    pub async fn handle_incoming_ack(&self, request: Request) -> ClientResult<()> {
        debug!("✅ Handling incoming ACK");

        let call_id_header = request.raw_header_value(&HeaderName::CallId)
            .ok_or_else(|| ClientError::protocol_error("ACK missing Call-ID header"))?;

        // Find the call by SIP Call-ID
        let call_id = self.find_call_by_sip_call_id(&call_id_header).await;

        if let Some(call_id) = call_id {
            info!("✅ ACK received for call {}, establishing media", call_id);
            
            // Update call state to connected
            self.update_call_state(&call_id, CallState::Connected).await?;
            
            // TODO: Start media session via media_manager
            
            info!("🟢 Call {} is now connected", call_id);
        } else {
            debug!("ACK received for unknown call: {}", call_id_header);
        }

        Ok(())
    }

    /// Handle incoming CANCEL request
    pub async fn handle_incoming_cancel(&self, request: Request) -> ClientResult<()> {
        debug!("🚫 Handling incoming CANCEL");

        let call_id_header = request.raw_header_value(&HeaderName::CallId)
            .ok_or_else(|| ClientError::protocol_error("CANCEL missing Call-ID header"))?;

        // Find the call by SIP Call-ID
        let call_id = self.find_call_by_sip_call_id(&call_id_header).await;

        if let Some(call_id) = call_id {
            info!("🚫 Cancelling call {}", call_id);
            
            // Update call state
            self.update_call_state(&call_id, CallState::Cancelled).await?;
            
            // Send 200 OK to CANCEL
            self.send_final_response(&request, StatusCode::Ok, "OK").await?;
            
            // TODO: Send 487 Request Terminated to original INVITE
            
            // Remove from active calls
            self.remove_call(&call_id).await?;
            
            info!("✅ Call {} cancelled", call_id);
        } else {
            warn!("⚠️ Received CANCEL for unknown call: {}", call_id_header);
            // Still send 200 OK
            self.send_final_response(&request, StatusCode::Ok, "OK").await?;
        }

        Ok(())
    }

    /// Handle INVITE response
    pub async fn handle_invite_response(&self, response: Response) -> ClientResult<()> {
        debug!("📨 Handling INVITE response: {} {}", response.status_code(), response.reason_phrase());

        let call_id_header = response.raw_header_value(&HeaderName::CallId)
            .ok_or_else(|| ClientError::protocol_error("Response missing Call-ID header"))?;

        let call_id = self.find_call_by_sip_call_id(&call_id_header).await;

        if let Some(call_id) = call_id {
            match response.status_code() {
                100 => { // StatusCode::Trying
                    info!("⏳ Call {} proceeding", call_id);
                    self.update_call_state(&call_id, CallState::Proceeding).await?;
                },
                180 => { // StatusCode::Ringing
                    info!("📳 Call {} ringing", call_id);
                    self.update_call_state(&call_id, CallState::Ringing).await?;
                },
                200 => { // StatusCode::Ok
                    info!("✅ Call {} answered, sending ACK", call_id);
                    self.update_call_state(&call_id, CallState::Connected).await?;
                    
                    // TODO: Send ACK
                    // TODO: Start media session
                },
                code if code >= 400 => {
                    warn!("❌ Call {} failed with {}", call_id, code);
                    self.update_call_state(&call_id, CallState::Failed).await?;
                    
                    // TODO: Clean up and remove call
                },
                _ => {
                    debug!("📨 Unhandled response {} for call {}", response.status_code(), call_id);
                }
            }
        } else {
            debug!("Received response for unknown call: {}", call_id_header);
        }

        Ok(())
    }

    /// Handle BYE response
    pub async fn handle_bye_response(&self, response: Response) -> ClientResult<()> {
        debug!("📨 Handling BYE response: {} {}", response.status_code(), response.reason_phrase());

        let call_id_header = response.raw_header_value(&HeaderName::CallId)
            .ok_or_else(|| ClientError::protocol_error("Response missing Call-ID header"))?;

        let call_id = self.find_call_by_sip_call_id(&call_id_header).await;

        if let Some(call_id) = call_id {
            info!("📴 BYE response received for call {}, terminating", call_id);
            self.update_call_state(&call_id, CallState::Terminated).await?;
            
            // TODO: Clean up media session
            
            // Remove from active calls
            self.remove_call(&call_id).await?;
        }

        Ok(())
    }

    /// Handle transaction timeout
    pub async fn handle_transaction_timeout(&self, transaction_key: &str) -> ClientResult<()> {
        debug!("⏰ Handling call transaction timeout: {}", transaction_key);

        // Find call by transaction key
        // TODO: Implement proper transaction key tracking
        
        warn!("⏰ Call transaction timeout: {}", transaction_key);
        Ok(())
    }

    /// Find call by SIP Call-ID header
    async fn find_call_by_sip_call_id(&self, sip_call_id: &str) -> Option<CallId> {
        let calls = self.active_calls.read().await;
        for (call_id, call) in calls.iter() {
            if call.info.sip_call_id == sip_call_id {
                return Some(*call_id);
            }
        }
        None
    }

    /// Send provisional response (1xx)
    async fn send_provisional_response(&self, _request: &Request, status_code: StatusCode, reason: &str) -> ClientResult<()> {
        debug!("📤 Sending {} {} response", status_code.as_u16(), reason);
        
        // TODO: Build proper SIP response
        // TODO: Send via transaction_manager
        
        Ok(())
    }

    /// Send final response (2xx, 4xx, 5xx, 6xx)
    async fn send_final_response(&self, _request: &Request, status_code: StatusCode, reason: &str) -> ClientResult<()> {
        debug!("📤 Sending {} {} response", status_code.as_u16(), reason);
        
        // TODO: Build proper SIP response
        // TODO: Send via transaction_manager
        
        Ok(())
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
        info!("✅ Answering call {}", call_id);
        
        // Verify call exists and is in appropriate state
        {
            let calls = self.active_calls.read().await;
            let call = calls.get(call_id)
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?;
            
            if call.info.state != CallState::IncomingPending && call.info.state != CallState::Ringing {
                return Err(ClientError::InvalidCallState { 
                    call_id: *call_id, 
                    current_state: call.info.state.clone() 
                });
            }
        }

        // TODO: Create media session via media_manager
        // TODO: Send 200 OK response via transaction_manager
        
        self.update_call_state(call_id, CallState::Connected).await?;
        
        info!("✅ Call {} answered successfully", call_id);
        Ok(())
    }

    /// Reject an incoming call
    pub async fn reject_call(&self, call_id: &CallId) -> ClientResult<()> {
        info!("❌ Rejecting call {}", call_id);
        
        // Verify call exists and is in appropriate state
        {
            let calls = self.active_calls.read().await;
            let call = calls.get(call_id)
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?;
            
            if call.info.direction != CallDirection::Incoming {
                return Err(ClientError::InvalidCallState { 
                    call_id: *call_id, 
                    current_state: call.info.state.clone() 
                });
            }
        }

        // TODO: Send 4xx response via transaction_manager
        
        self.update_call_state(call_id, CallState::Terminated).await?;
        
        // Remove from active calls
        self.remove_call(call_id).await?;
        
        info!("✅ Call {} rejected", call_id);
        Ok(())
    }

    /// Hangup an active call
    pub async fn hangup_call(&self, call_id: &CallId) -> ClientResult<()> {
        info!("📴 Hanging up call {}", call_id);
        
        // Verify call exists
        {
            let calls = self.active_calls.read().await;
            let _call = calls.get(call_id)
                .ok_or_else(|| ClientError::CallNotFound { call_id: *call_id })?;
        }

        // TODO: Send BYE via transaction_manager
        // TODO: Clean up media session via media_manager
        
        self.update_call_state(call_id, CallState::Terminating).await?;
        
        info!("✅ Call {} hangup initiated", call_id);
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