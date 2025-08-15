//! Dialog Correlation Module
//! 
//! This module provides a workaround for the dialog-core bug where dialog IDs
//! change when transitioning from Early to Confirmed state. 
//! 
//! This is a TEMPORARY workaround until dialog-core is fixed to maintain
//! consistent dialog IDs throughout the dialog lifecycle per RFC 3261.

use std::sync::Arc;
use dashmap::DashMap;
use rvoip_dialog_core::DialogId;
use rvoip_sip_core::{Response, CallId};
use crate::api::types::SessionId;

/// Correlation data for matching responses to sessions when dialog IDs change
#[derive(Debug, Clone)]
pub struct CorrelationData {
    pub session_id: SessionId,
    pub from_uri: String,
    pub to_uri: String,
    pub call_id: String,
    pub from_tag: Option<String>,
}

/// Dialog Correlator - Workaround for dialog-core dialog ID bug
/// 
/// This correlator helps map responses to sessions when dialog-core
/// creates new dialog IDs during state transitions.
pub struct DialogCorrelator {
    /// Track sessions awaiting responses (UAC sessions)
    pending_uac_sessions: Arc<DashMap<String, CorrelationData>>,  // key: from_tag or call_id
    
    /// Track confirmed dialogs
    confirmed_dialogs: Arc<DashMap<DialogId, SessionId>>,
}

impl DialogCorrelator {
    pub fn new() -> Self {
        Self {
            pending_uac_sessions: Arc::new(DashMap::new()),
            confirmed_dialogs: Arc::new(DashMap::new()),
        }
    }
    
    /// Register a UAC session that's awaiting responses
    pub fn register_uac_session(
        &self,
        session_id: SessionId,
        from_uri: String,
        to_uri: String,
        call_id: String,
        from_tag: Option<String>,
    ) {
        let correlation = CorrelationData {
            session_id: session_id.clone(),
            from_uri,
            to_uri,
            call_id: call_id.clone(),
            from_tag: from_tag.clone(),
        };
        
        // Store by Call-ID as primary key
        self.pending_uac_sessions.insert(call_id.clone(), correlation.clone());
        
        // Also store by From tag if available
        if let Some(tag) = from_tag {
            self.pending_uac_sessions.insert(tag, correlation);
        }
        
        tracing::debug!(
            "Registered UAC session {} with Call-ID {} for correlation",
            session_id, call_id
        );
    }
    
    /// Try to correlate a response to a session
    pub fn correlate_response(
        &self,
        response: &Response,
        dialog_id: &DialogId,
    ) -> Option<SessionId> {
        // Extract correlation keys from response
        let call_id = response.call_id().map(|h| h.0.clone())?;
        
        // Try to find by Call-ID first
        if let Some(entry) = self.pending_uac_sessions.get(&call_id) {
            let session_id = entry.session_id.clone();
            
            // Move to confirmed dialogs
            self.confirmed_dialogs.insert(dialog_id.clone(), session_id.clone());
            
            tracing::info!(
                "Correlated response for dialog {} to session {} via Call-ID {}",
                dialog_id, session_id, call_id
            );
            
            return Some(session_id);
        }
        
        // Try From tag as fallback
        if let Some(from_header) = response.from() {
            if let Some(tag) = from_header.tag() {
                if let Some(entry) = self.pending_uac_sessions.get(tag) {
                    let session_id = entry.session_id.clone();
                    
                    // Move to confirmed dialogs
                    self.confirmed_dialogs.insert(dialog_id.clone(), session_id.clone());
                    
                    tracing::info!(
                        "Correlated response for dialog {} to session {} via From tag {}",
                        dialog_id, session_id, tag
                    );
                    
                    return Some(session_id);
                }
            }
        }
        
        // Check if this is an already confirmed dialog
        if let Some(entry) = self.confirmed_dialogs.get(dialog_id) {
            return Some(entry.value().clone());
        }
        
        tracing::debug!(
            "Could not correlate response for dialog {} with Call-ID {}",
            dialog_id, call_id
        );
        
        None
    }
    
    /// Clean up correlation data for a completed session
    pub fn cleanup_session(&self, session_id: &SessionId) {
        // Remove from pending sessions
        self.pending_uac_sessions.retain(|_k, v| &v.session_id != session_id);
        
        // Remove from confirmed dialogs
        self.confirmed_dialogs.retain(|_k, v| v != session_id);
        
        tracing::debug!("Cleaned up correlation data for session {}", session_id);
    }
    
    /// Check if we have correlation data for a Call-ID
    pub fn has_correlation_for_call_id(&self, call_id: &str) -> bool {
        self.pending_uac_sessions.contains_key(call_id)
    }
    
    /// Get statistics
    pub fn stats(&self) -> CorrelatorStats {
        CorrelatorStats {
            pending_sessions: self.pending_uac_sessions.len(),
            confirmed_dialogs: self.confirmed_dialogs.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CorrelatorStats {
    pub pending_sessions: usize,
    pub confirmed_dialogs: usize,
}

impl Default for DialogCorrelator {
    fn default() -> Self {
        Self::new()
    }
}