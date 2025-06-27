//! Dialog Manager (parallel to MediaManager)
//!
//! Main interface for dialog operations, providing session-level abstractions
//! over dialog-core UnifiedDialogApi functionality.

use std::sync::Arc;
use rvoip_dialog_core::{
    api::unified::UnifiedDialogApi,
    DialogId,
};
use crate::api::types::{SessionId, CallSession, CallState, MediaInfo};
use crate::manager::registry::SessionRegistry;
use crate::dialog::{DialogError, DialogResult, SessionDialogHandle};

/// Dialog manager for session-level dialog operations
/// (parallel to MediaManager)
pub struct DialogManager {
    dialog_api: Arc<UnifiedDialogApi>,
    registry: Arc<SessionRegistry>,
    dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
}

impl DialogManager {
    /// Create a new dialog manager
    pub fn new(
        dialog_api: Arc<UnifiedDialogApi>,
        registry: Arc<SessionRegistry>,
        dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
    ) -> Self {
        Self {
            dialog_api,
            registry,
            dialog_to_session,
        }
    }
    
    /// Start the dialog API
    pub async fn start(&self) -> DialogResult<()> {
        self.dialog_api
            .start()
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
            
        Ok(())
    }
    
    /// Stop the dialog API
    pub async fn stop(&self) -> DialogResult<()> {
        self.dialog_api
            .stop()
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
            
        Ok(())
    }
    
    /// Create an outgoing call session
    pub async fn create_outgoing_call(
        &self,
        session_id: SessionId,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> DialogResult<SessionDialogHandle> {
        // Create SIP INVITE and dialog using dialog-core unified API
        let call_handle = self.dialog_api
            .make_call(from, to, sdp)
            .await
            .map_err(|e| DialogError::DialogCreation {
                reason: format!("Failed to create call via dialog-core: {}", e),
            })?;
        
        let dialog_id = call_handle.dialog().id().clone();
        
        // Map dialog to session
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        
        tracing::info!("Created outgoing call: {} -> {} (dialog: {})", from, to, dialog_id);
        
        Ok(SessionDialogHandle::new(session_id, dialog_id).with_call_handle(call_handle))
    }
    
    /// Accept an incoming call
    pub async fn accept_incoming_call(&self, session_id: &SessionId, sdp_answer: Option<String>) -> DialogResult<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Get the call handle and answer the call
        let call_handle = self.dialog_api
            .get_call_handle(&dialog_id)
            .await
            .map_err(|e| DialogError::SessionNotFound {
                session_id: session_id.0.clone(),
            })?;
            
        call_handle
            .answer(sdp_answer)
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
        
        tracing::info!("Accepted incoming call: {} (dialog: {})", session_id, dialog_id);
        Ok(())
    }
    
    /// Hold a session
    pub async fn hold_session(&self, session_id: &SessionId) -> DialogResult<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send re-INVITE with hold SDP via dialog-core unified API
        let _tx_key = self.dialog_api
            .send_update(&dialog_id, Some("SDP with hold attributes".to_string()))
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
            
        tracing::info!("Holding session: {}", session_id);
        Ok(())
    }
    
    /// Resume a session from hold
    pub async fn resume_session(&self, session_id: &SessionId) -> DialogResult<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send re-INVITE with active SDP via dialog-core unified API
        let _tx_key = self.dialog_api
            .send_update(&dialog_id, Some("SDP with active media".to_string()))
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
            
        tracing::info!("Resuming session: {}", session_id);
        Ok(())
    }
    
    /// Transfer a session to another destination
    pub async fn transfer_session(&self, session_id: &SessionId, target: &str) -> DialogResult<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send REFER request via dialog-core unified API
        let _tx_key = self.dialog_api
            .send_refer(&dialog_id, target.to_string(), None)
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
            
        tracing::info!("Transferring session {} to {}", session_id, target);
        Ok(())
    }
    
    /// Terminate a session
    /// 
    /// This method is state-aware:
    /// - For sessions in Early state (no final response to INVITE), sends CANCEL
    /// - For sessions in Active/Established state, sends BYE
    pub async fn terminate_session(&self, session_id: &SessionId) -> DialogResult<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Get the session to check its state
        let session = self.registry
            .get_session(session_id)
            .await
            .map_err(|_| DialogError::SessionNotFound {
                session_id: session_id.0.clone(),
            })?
            .ok_or_else(|| DialogError::SessionNotFound {
                session_id: session_id.0.clone(),
            })?;
        
        // Check the session state to determine the appropriate termination method
        match session.state() {
            CallState::Initiating => {
                // Early dialog - send CANCEL
                tracing::info!("Canceling early dialog for session {} in state {:?}", session_id, session.state());
                
                let _tx_key = self.dialog_api
                    .send_cancel(&dialog_id)
                    .await
                    .map_err(|e| DialogError::DialogCore {
                        source: Box::new(e),
                    })?;
                
                tracing::info!("Sent CANCEL for session: {}", session_id);
            },
            CallState::Ringing | CallState::Active | CallState::OnHold | CallState::Transferring => {
                // Established dialog - send BYE
                tracing::info!("Terminating established dialog for session {} in state {:?}", session_id, session.state());
                
                let _tx_key = self.dialog_api
                    .send_bye(&dialog_id)
                    .await
                    .map_err(|e| DialogError::DialogCore {
                        source: Box::new(e),
                    })?;
                
                tracing::info!("Sent BYE for session: {}", session_id);
            },
            CallState::Terminating | CallState::Terminated | CallState::Cancelled | CallState::Failed(_) => {
                // Already terminated - just clean up
                tracing::warn!("Session {} is already in state {:?}, just cleaning up", session_id, session.state());
            }
        }
        
        // Remove the dialog-to-session mapping
        self.dialog_to_session.remove(&dialog_id);
        
        tracing::info!("Terminated session: {}", session_id);
        Ok(())
    }
    
    /// Send DTMF tones
    pub async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> DialogResult<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send INFO request with DTMF payload via dialog-core unified API
        let _tx_key = self.dialog_api
            .send_info(&dialog_id, format!("DTMF: {}", digits))
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
            
        tracing::info!("Sending DTMF {} to session {}", digits, session_id);
        Ok(())
    }
    
    /// Update media for a session (send re-INVITE with new SDP)
    pub async fn update_media(&self, session_id: &SessionId, sdp: &str) -> DialogResult<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send re-INVITE with new SDP via dialog-core unified API
        let _tx_key = self.dialog_api
            .send_update(&dialog_id, Some(sdp.to_string()))
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
            
        tracing::info!("Updating media for session {}", session_id);
        Ok(())
    }
    
    /// Get dialog ID for a session ID
    pub fn get_dialog_id_for_session(&self, session_id: &SessionId) -> DialogResult<DialogId> {
        self.dialog_to_session
            .iter()
            .find_map(|entry| {
                if entry.value() == session_id {
                    Some(entry.key().clone())
                } else {
                    None
                }
            })
            .ok_or_else(|| DialogError::SessionNotFound {
                session_id: session_id.0.clone(),
            })
    }
    
    /// Get session ID for a dialog ID
    pub fn get_session_id_for_dialog(&self, dialog_id: &DialogId) -> Option<SessionId> {
        self.dialog_to_session
            .get(dialog_id)
            .map(|entry| entry.value().clone())
    }
    
    /// Map dialog to session
    pub fn map_dialog_to_session(&self, dialog_id: DialogId, session_id: SessionId) {
        self.dialog_to_session.insert(dialog_id, session_id);
    }
    
    /// Unmap dialog from session
    pub fn unmap_dialog(&self, dialog_id: &DialogId) -> Option<SessionId> {
        self.dialog_to_session
            .remove(dialog_id)
            .map(|(_, session_id)| session_id)
    }
    
    /// Get the actual bound address
    pub fn get_bound_address(&self) -> std::net::SocketAddr {
        self.dialog_api.config().local_address()
    }
    
    /// Get dialog statistics
    pub fn get_dialog_stats(&self) -> DialogManagerStats {
        DialogManagerStats {
            active_dialogs: self.dialog_to_session.len(),
            mapped_sessions: self.dialog_to_session.len(),
        }
    }
    
    /// List all active dialog IDs
    pub fn list_active_dialogs(&self) -> Vec<DialogId> {
        self.dialog_to_session
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }
    
    /// Check if dialog exists for session
    pub fn has_dialog_for_session(&self, session_id: &SessionId) -> bool {
        self.dialog_to_session
            .iter()
            .any(|entry| entry.value() == session_id)
    }
    
    /// Get the dialog API reference (for advanced usage)
    pub fn dialog_api(&self) -> &Arc<UnifiedDialogApi> {
        &self.dialog_api
    }
}

/// Dialog manager statistics
#[derive(Debug, Clone)]
pub struct DialogManagerStats {
    pub active_dialogs: usize,
    pub mapped_sessions: usize,
}

impl std::fmt::Debug for DialogManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialogManager")
            .field("active_dialogs", &self.dialog_to_session.len())
            .field("bound_address", &self.get_bound_address())
            .finish()
    }
} 