//! Event handling for the client-core library
//! 
//! This module contains the event handler that bridges session-core events
//! to client-core events, providing a clean abstraction for applications.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use dashmap::DashMap;
use chrono::Utc;

// Import session-core types
use rvoip_session_core::{
    api::{
        types::{SessionId, CallSession, CallState, IncomingCall, CallDecision},
        handlers::CallHandler,
    },
};

// Import client-core types
use crate::{
    call::{CallId, CallInfo, CallDirection},
    events::{ClientEventHandler, IncomingCallInfo, CallStatusInfo},
};

// All types are re-exported from the main events module

/// Internal call handler that bridges session-core events to client-core events
pub struct ClientCallHandler {
    pub client_event_handler: Arc<RwLock<Option<Arc<dyn ClientEventHandler>>>>,
    pub call_mapping: Arc<DashMap<SessionId, CallId>>,
    pub session_mapping: Arc<DashMap<CallId, SessionId>>,
    pub call_info: Arc<DashMap<CallId, CallInfo>>,
    pub incoming_calls: Arc<DashMap<CallId, IncomingCall>>,
    pub event_tx: Option<tokio::sync::broadcast::Sender<crate::events::ClientEvent>>,
}

impl std::fmt::Debug for ClientCallHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientCallHandler")
            .field("client_event_handler", &"<event handler>")
            .field("call_mapping", &self.call_mapping)
            .field("session_mapping", &self.session_mapping)
            .field("call_info", &self.call_info)
            .field("incoming_calls", &self.incoming_calls)
            .finish()
    }
}

impl ClientCallHandler {
    pub fn new(
        call_mapping: Arc<DashMap<SessionId, CallId>>,
        session_mapping: Arc<DashMap<CallId, SessionId>>,
        call_info: Arc<DashMap<CallId, CallInfo>>,
        incoming_calls: Arc<DashMap<CallId, IncomingCall>>,
    ) -> Self {
        Self {
            client_event_handler: Arc::new(RwLock::new(None)),
            call_mapping,
            session_mapping,
            call_info,
            incoming_calls,
            event_tx: None,
        }
    }
    
    pub fn with_event_tx(mut self, event_tx: tokio::sync::broadcast::Sender<crate::events::ClientEvent>) -> Self {
        self.event_tx = Some(event_tx);
        self
    }
    
    pub async fn set_event_handler(&self, handler: Arc<dyn ClientEventHandler>) {
        *self.client_event_handler.write().await = Some(handler);
    }
    
    /// Store an IncomingCall object for later use
    pub async fn store_incoming_call(&self, call_id: CallId, incoming_call: IncomingCall) {
        self.incoming_calls.insert(call_id, incoming_call);
    }
    
    /// Retrieve a stored IncomingCall object
    pub async fn get_incoming_call(&self, call_id: &CallId) -> Option<IncomingCall> {
        self.incoming_calls.get(call_id).map(|entry| entry.value().clone())
    }
    
    /// Extract display name from SIP URI or headers
    pub fn extract_display_name(&self, uri: &str, headers: &HashMap<String, String>) -> Option<String> {
        // First try to extract from URI (e.g., "Display Name" <sip:user@domain>)
        if let Some(start) = uri.find('"') {
            if let Some(end) = uri[start + 1..].find('"') {
                let display_name = &uri[start + 1..start + 1 + end];
                if !display_name.is_empty() {
                    return Some(display_name.to_string());
                }
            }
        }
        
        // Try display name before < in URI
        if let Some(angle_pos) = uri.find('<') {
            let potential_name = uri[..angle_pos].trim();
            if !potential_name.is_empty() && !potential_name.starts_with("sip:") {
                return Some(potential_name.to_string());
            }
        }
        
        // Try From header display name
        if let Some(from_header) = headers.get("From") {
            return self.extract_display_name_from_header(from_header);
        }
        
        None
    }
    
    pub fn extract_display_name_from_header(&self, header: &str) -> Option<String> {
        if let Some(start) = header.find('"') {
            if let Some(end) = header[start + 1..].find('"') {
                let display_name = &header[start + 1..start + 1 + end];
                if !display_name.is_empty() {
                    return Some(display_name.to_string());
                }
            }
        }
        
        if let Some(angle_pos) = header.find('<') {
            let potential_name = header[..angle_pos].trim();
            if !potential_name.is_empty() && !potential_name.starts_with("sip:") {
                return Some(potential_name.to_string());
            }
        }
        
        None
    }
    
    /// Extract subject from headers
    pub fn extract_subject(&self, headers: &HashMap<String, String>) -> Option<String> {
        headers.get("Subject")
            .or_else(|| headers.get("subject"))
            .cloned()
            .filter(|s| !s.is_empty())
    }
    
    /// Extract Call-ID from headers
    pub fn extract_call_id(&self, headers: &HashMap<String, String>) -> Option<String> {
        headers.get("Call-ID")
            .or_else(|| headers.get("call-id"))
            .cloned()
    }
    
    /// Update call info with enhanced session data
    pub async fn update_call_info_from_session(&self, call_id: CallId, session: &CallSession) {
        if let Some(mut call_info_ref) = self.call_info.get_mut(&call_id) {
            // Update state if it changed
            let new_client_state = self.map_session_state_to_client_state(&session.state);
            let old_state = call_info_ref.state.clone();
            
            if new_client_state != old_state {
                // Update timestamps based on state transition
                match new_client_state {
                    crate::call::CallState::Connected => {
                        if call_info_ref.connected_at.is_none() {
                            call_info_ref.connected_at = Some(Utc::now());
                        }
                    }
                    crate::call::CallState::Terminated | 
                    crate::call::CallState::Failed | 
                    crate::call::CallState::Cancelled => {
                        if call_info_ref.ended_at.is_none() {
                            call_info_ref.ended_at = Some(Utc::now());
                        }
                    }
                    _ => {}
                }
                
                call_info_ref.state = new_client_state.clone();
                
                // Emit state change event
                if let Some(handler) = self.client_event_handler.read().await.as_ref() {
                    let status_info = CallStatusInfo {
                        call_id,
                        new_state: new_client_state,
                        previous_state: Some(old_state),
                        reason: None,
                        timestamp: Utc::now(),
                    };
                    handler.on_call_state_changed(status_info).await;
                }
            }
        }
    }
    
    /// Map session-core CallState to client-core CallState with enhanced logic
    pub fn map_session_state_to_client_state(&self, session_state: &CallState) -> crate::call::CallState {
        match session_state {
            CallState::Initiating => crate::call::CallState::Initiating,
            CallState::Ringing => crate::call::CallState::Ringing,
            CallState::Active => crate::call::CallState::Connected,
            CallState::OnHold => crate::call::CallState::Connected, // Still connected, just on hold
            CallState::Transferring => crate::call::CallState::Proceeding,
            CallState::Terminating => crate::call::CallState::Terminating,
            CallState::Terminated => crate::call::CallState::Terminated,
            CallState::Cancelled => crate::call::CallState::Cancelled,
            CallState::Failed(reason) => {
                tracing::debug!("Call failed with reason: {}", reason);
                crate::call::CallState::Failed
            }
        }
    }
}

#[async_trait::async_trait]
impl CallHandler for ClientCallHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Map session to call
        let call_id = CallId::new_v4();
        self.call_mapping.insert(call.id.clone(), call_id);
        self.session_mapping.insert(call_id, call.id.clone());
        
        // Store the IncomingCall for later use in answer/reject
        self.incoming_calls.insert(call_id, call.clone());
        
        // Enhanced call info extraction
        let caller_display_name = self.extract_display_name(&call.from, &call.headers);
        let subject = self.extract_subject(&call.headers);
        let sip_call_id = self.extract_call_id(&call.headers)
            .unwrap_or_else(|| call.id.0.clone());
        
        // Create comprehensive call info
        let call_info = CallInfo {
            call_id,
            state: crate::call::CallState::IncomingPending,
            direction: CallDirection::Incoming,
            local_uri: call.to.clone(),
            remote_uri: call.from.clone(),
            remote_display_name: caller_display_name.clone(),
            subject: subject.clone(),
            created_at: Utc::now(),
            connected_at: None,
            ended_at: None,
            remote_addr: None, // TODO: Extract from session if available
            media_session_id: None,
            sip_call_id,
            metadata: call.headers.clone(),
        };
        
        // Store call info
        self.call_info.insert(call_id, call_info.clone());
        
        // Create incoming call info for event
        let incoming_call_info = IncomingCallInfo {
            call_id,
            caller_uri: call.from.clone(),
            callee_uri: call.to.clone(),
            caller_display_name,
            subject,
            created_at: Utc::now(),
        };
        
        // Broadcast event
        if let Some(event_tx) = &self.event_tx {
            let _ = event_tx.send(crate::events::ClientEvent::IncomingCall { 
                info: incoming_call_info.clone(),
                priority: crate::events::EventPriority::High,
            });
        }
        
        // Forward to client event handler
        if let Some(handler) = self.client_event_handler.read().await.as_ref() {
            let action = handler.on_incoming_call(incoming_call_info).await;
            match action {
                crate::events::CallAction::Accept => CallDecision::Accept(None),
                crate::events::CallAction::Reject => CallDecision::Reject("Call rejected by user".to_string()),
                crate::events::CallAction::Ignore => CallDecision::Defer,
            }
        } else {
            CallDecision::Reject("No event handler configured".to_string())
        }
    }
    
    async fn on_call_ended(&self, session: CallSession, reason: &str) {
        // Map session to client call and emit event
        if let Some(call_id) = self.call_mapping.get(&session.id).map(|entry| *entry.value()) {
            // Update call info with final state
            if let Some(mut call_info_ref) = self.call_info.get_mut(&call_id) {
                call_info_ref.state = self.map_session_state_to_client_state(&session.state);
                call_info_ref.ended_at = Some(Utc::now());
                
                // Add termination reason to metadata
                call_info_ref.metadata.insert("termination_reason".to_string(), reason.to_string());
            }
            
            let status_info = CallStatusInfo {
                call_id,
                new_state: self.map_session_state_to_client_state(&session.state),
                previous_state: None, // TODO: Track previous state
                reason: Some(reason.to_string()),
                timestamp: Utc::now(),
            };
            
            // Broadcast event
            if let Some(event_tx) = &self.event_tx {
                let _ = event_tx.send(crate::events::ClientEvent::CallStateChanged { 
                    info: status_info.clone(),
                    priority: crate::events::EventPriority::Normal,
                });
            }
            
            // Forward to client event handler
            if let Some(handler) = self.client_event_handler.read().await.as_ref() {
                handler.on_call_state_changed(status_info).await;
            }
            
            // Clean up mappings but keep call_info for history
            self.call_mapping.remove(&session.id);
            self.session_mapping.remove(&call_id);
        }
    }
    
    async fn on_call_established(&self, session: CallSession, local_sdp: Option<String>, remote_sdp: Option<String>) {
        // Map session to client call
        if let Some(call_id) = self.call_mapping.get(&session.id).map(|entry| *entry.value()) {
            // Update call info with establishment
            if let Some(mut call_info_ref) = self.call_info.get_mut(&call_id) {
                call_info_ref.state = crate::call::CallState::Connected;
                if call_info_ref.connected_at.is_none() {
                    call_info_ref.connected_at = Some(Utc::now());
                }
                
                // Store SDP information
                if let Some(local_sdp) = &local_sdp {
                    call_info_ref.metadata.insert("local_sdp".to_string(), local_sdp.clone());
                }
                if let Some(remote_sdp) = &remote_sdp {
                    call_info_ref.metadata.insert("remote_sdp".to_string(), remote_sdp.clone());
                }
            }
            
            let status_info = CallStatusInfo {
                call_id,
                new_state: crate::call::CallState::Connected,
                previous_state: Some(crate::call::CallState::Proceeding),
                reason: Some("Call established".to_string()),
                timestamp: Utc::now(),
            };
            
            // Broadcast event
            if let Some(event_tx) = &self.event_tx {
                let _ = event_tx.send(crate::events::ClientEvent::CallStateChanged { 
                    info: status_info.clone(),
                    priority: crate::events::EventPriority::High,
                });
            }
            
            // Forward to client event handler
            if let Some(handler) = self.client_event_handler.read().await.as_ref() {
                handler.on_call_state_changed(status_info).await;
            }
            
            tracing::info!("Call {} established with SDP exchange", call_id);
        }
    }
}
