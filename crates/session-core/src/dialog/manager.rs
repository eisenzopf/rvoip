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
use crate::coordinator::registry::InternalSessionRegistry;
use crate::dialog::{DialogError, DialogResult, SessionDialogHandle};

/// Dialog manager for session-level dialog operations
/// (parallel to MediaManager)
pub struct DialogManager {
    dialog_api: Arc<UnifiedDialogApi>,
    registry: Arc<InternalSessionRegistry>,
    dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
}

impl DialogManager {
    /// Create a new dialog manager
    pub fn new(
        dialog_api: Arc<UnifiedDialogApi>,
        registry: Arc<InternalSessionRegistry>,
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
        // Stop the underlying dialog-core DialogManager
        // It will emit DialogEvent::ShutdownComplete when done
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
        
        println!("📍 DIALOG MANAGER: Mapped dialog {} to session {}", dialog_id, session_id);
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
        
        // Get current SDP from session
        let current_sdp = self.get_current_sdp(session_id).await?;
        
        // Generate hold SDP with sendonly
        let hold_sdp = crate::media::generate_hold_sdp(&current_sdp)
            .map_err(|e| DialogError::SipProcessing {
                message: format!("Failed to generate hold SDP: {}", e),
            })?;
        
        // Send re-INVITE with hold SDP via dialog-core unified API
        let _tx_key = self.dialog_api
            .send_update(&dialog_id, Some(hold_sdp))
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
            
        tracing::info!("Holding session: {} with proper SDP", session_id);
        Ok(())
    }
    
    /// Resume a session from hold
    pub async fn resume_session(&self, session_id: &SessionId) -> DialogResult<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Get current SDP from session
        let current_sdp = self.get_current_sdp(session_id).await?;
        
        // Generate resume SDP with sendrecv
        let resume_sdp = crate::media::generate_resume_sdp(&current_sdp)
            .map_err(|e| DialogError::SipProcessing {
                message: format!("Failed to generate resume SDP: {}", e),
            })?;
        
        // Send re-INVITE with resume SDP via dialog-core unified API
        let _tx_key = self.dialog_api
            .send_update(&dialog_id, Some(resume_sdp))
            .await
            .map_err(|e| DialogError::DialogCore {
                source: Box::new(e),
            })?;
            
        tracing::info!("Resuming session: {} with proper SDP", session_id);
        Ok(())
    }
    
    /// Transfer a session to another destination
    pub async fn transfer_session(&self, session_id: &SessionId, target: &str) -> DialogResult<()> {
        println!("🔍 DIALOG_MANAGER: transfer_session called for {} to {}", session_id, target);
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        println!("🔍 DIALOG_MANAGER: Found dialog_id {} for session {}", dialog_id, session_id);
        
        // Send REFER request via dialog-core unified API
        println!("🔍 DIALOG_MANAGER: About to call dialog_api.send_refer");
        let _tx_key = self.dialog_api
            .send_refer(&dialog_id, target.to_string(), None)
            .await
            .map_err(|e| {
                println!("❌ DIALOG_MANAGER: send_refer failed: {}", e);
                DialogError::DialogCore {
                    source: Box::new(e),
                }
            })?;
        
        println!("✅ DIALOG_MANAGER: send_refer completed successfully");    
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
                // Early dialog - attempt CANCEL but handle race conditions
                tracing::info!("Attempting to cancel early dialog for session {} in state {:?}", session_id, session.state());
                
                // 🚨 CRITICAL RACE CONDITION FIX: Always use atomic dialog state check
                // This prevents the "Cannot send CANCEL for dialog in state Confirmed" error
                match self.dialog_api.get_dialog_state(&dialog_id).await {
                    Ok(dialog_state) => {
                        match dialog_state {
                            rvoip_dialog_core::dialog::DialogState::Initial | 
                            rvoip_dialog_core::dialog::DialogState::Early => {
                                // Safe to send CANCEL
                                match self.dialog_api.send_cancel(&dialog_id).await {
                                    Ok(_tx_key) => {
                                        tracing::info!("✅ Sent CANCEL for early dialog session: {}", session_id);
                                    }
                                    Err(e) => {
                                        // CANCEL failed, likely due to race condition - try BYE instead
                                        tracing::warn!("⚠️ CANCEL failed for dialog {} (race condition), attempting BYE: {}", dialog_id, e);
                                        
                                        match self.dialog_api.send_bye(&dialog_id).await {
                                            Ok(_tx_key) => {
                                                tracing::info!("✅ Sent BYE after CANCEL failure for session: {}", session_id);
                                            }
                                            Err(e2) => {
                                                tracing::warn!("⚠️ Both CANCEL and BYE failed for session {}: CANCEL={}, BYE={}", session_id, e, e2);
                                            }
                                        }
                                    }
                                }
                            },
                            _ => {
                                // Dialog has already progressed beyond Early state - send BYE instead
                                tracing::info!("Dialog {} has progressed beyond Early state ({}), sending BYE instead of CANCEL", 
                                    dialog_id, format!("{:?}", dialog_state));
                                
                                match self.dialog_api.send_bye(&dialog_id).await {
                                    Ok(_tx_key) => {
                                        tracing::info!("✅ Sent BYE for confirmed dialog session: {}", session_id);
                                    }
                                    Err(e) => {
                                        tracing::warn!("⚠️ BYE failed for confirmed dialog session {}: {}", session_id, e);
                                    }
                                }
                            }
                        }
                    },
                    Err(e) => {
                        // Dialog not found or error getting state - it may have already terminated
                        tracing::warn!("Could not get dialog state for {}: {}. Dialog may have already terminated.", dialog_id, e);
                    }
                }
            },
            CallState::Ringing | CallState::Active | CallState::OnHold | CallState::Transferring => {
                // Established dialog - send BYE with improved error handling
                tracing::info!("Terminating established dialog for session {} in state {:?}", session_id, session.state());
                
                // 🚨 IMPROVED BYE ERROR HANDLING: Handle unreachable endpoints gracefully
                // This prevents excessive Timer E retransmissions when agents are unreachable
                tracing::info!("📞 BYE-SEND: Attempting to send BYE for established session: {} (dialog: {})", session_id, dialog_id);
                
                match self.dialog_api.send_bye(&dialog_id).await {
                    Ok(tx_key) => {
                        tracing::info!("✅ BYE-SEND: Successfully sent BYE for session: {} (tx: {}, dialog: {})", 
                                     session_id, tx_key, dialog_id);
                        
                        // PHASE 0.24: Set a reasonable timeout for BYE response (increased from 5s to 15s)
                        // If no response in 15 seconds, consider the call terminated locally
                        let dialog_api = self.dialog_api.clone();
                        let session_id_timeout = session_id.clone();
                        let dialog_id_timeout = dialog_id.clone();
                        
                        tokio::spawn(async move {
                            // PHASE 0.24: Increased timeout from 5s to 15s for better reliability
                            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                            
                            // Check if dialog still exists and force cleanup if needed
                            match dialog_api.get_dialog_state(&dialog_id_timeout).await {
                                Ok(state) => {
                                    if !matches!(state, rvoip_dialog_core::dialog::DialogState::Terminated) {
                                        tracing::warn!("⏰ BYE timeout for session {} after 15s - forcing dialog termination (state: {:?})", 
                                                     session_id_timeout, state);
                                        
                                        // PHASE 0.24: Add detailed timeout logging
                                        tracing::info!("📞 BYE-TIMEOUT: Session {} did not receive 200 OK within 15 seconds", session_id_timeout);
                                        
                                        // Force terminate the dialog to stop retransmissions
                                        match dialog_api.terminate_dialog(&dialog_id_timeout).await {
                                            Ok(()) => {
                                                tracing::info!("✅ BYE-TIMEOUT: Successfully forced termination of dialog {}", dialog_id_timeout);
                                            }
                                            Err(e) => {
                                                tracing::warn!("⚠️ BYE-TIMEOUT: Failed to force terminate dialog {}: {}", dialog_id_timeout, e);
                                            }
                                        }
                                    } else {
                                        tracing::debug!("✅ BYE-SUCCESS: Session {} dialog already terminated within 15s", session_id_timeout);
                                    }
                                }
                                Err(e) => {
                                    // Dialog already cleaned up, which is normal for successful BYE
                                    tracing::debug!("✅ BYE-SUCCESS: Session {} dialog already cleaned up: {}", session_id_timeout, e);
                                }
                            }
                        });
                    },
                    Err(e) => {
                        let error_msg = e.to_string();
                        
                        tracing::warn!("❌ BYE-SEND: Failed to send BYE for session {} (dialog: {}): {}", 
                                     session_id, dialog_id, e);
                        
                        // PHASE 0.24: Enhanced error categorization for better handling
                        if error_msg.contains("unreachable") || error_msg.contains("timeout") || 
                           error_msg.contains("connection") || error_msg.contains("network") ||
                           error_msg.contains("Connection refused") || error_msg.contains("No route to host") {
                            tracing::warn!("🌐 BYE-ERROR-NETWORK: Network/connectivity issue for session {}: {}. Terminating locally.", session_id, e);
                            
                            // For network issues, terminate the dialog immediately to prevent retransmissions
                            match self.dialog_api.terminate_dialog(&dialog_id).await {
                                Ok(()) => {
                                    tracing::info!("✅ BYE-ERROR-NETWORK: Forcibly terminated dialog {} due to network issue", dialog_id);
                                }
                                Err(e2) => {
                                    tracing::warn!("⚠️ BYE-ERROR-NETWORK: Failed to forcibly terminate dialog {}: {}", dialog_id, e2);
                                }
                            }
                        } else if error_msg.contains("already terminated") || error_msg.contains("not exist") ||
                                  error_msg.contains("dialog not found") || error_msg.contains("No dialog found") {
                            tracing::info!("ℹ️ BYE-ERROR-ALREADY-GONE: BYE not needed for session {} - dialog already terminated: {}", session_id, e);
                            // This is fine, the dialog is already gone - no further action needed
                        } else if error_msg.contains("invalid state") || error_msg.contains("wrong state") ||
                                  error_msg.contains("Cannot send") {
                            tracing::warn!("🔄 BYE-ERROR-STATE: State issue for session {} - may need retry: {}", session_id, e);
                            
                            // For state issues, try to force terminate after a brief delay
                            let dialog_api = self.dialog_api.clone();
                            let dialog_id_retry = dialog_id.clone();
                            let session_id_retry = session_id.clone();
                            
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                tracing::info!("🔄 BYE-RETRY: Attempting force termination for session {} after state error", session_id_retry);
                                
                                match dialog_api.terminate_dialog(&dialog_id_retry).await {
                                    Ok(()) => {
                                        tracing::info!("✅ BYE-RETRY: Successfully force terminated dialog {} after state error", dialog_id_retry);
                                    }
                                    Err(e2) => {
                                        tracing::warn!("⚠️ BYE-RETRY: Failed to force terminate dialog {} after state error: {}", dialog_id_retry, e2);
                                    }
                                }
                            });
                        } else {
                            tracing::warn!("⚠️ BYE-ERROR-UNKNOWN: Unknown error for session {}: {}. Attempting force termination.", session_id, e);
                            
                            // For unknown errors, attempt immediate force termination
                            match self.dialog_api.terminate_dialog(&dialog_id).await {
                                Ok(()) => {
                                    tracing::info!("✅ BYE-ERROR-UNKNOWN: Force terminated dialog {} for unknown error", dialog_id);
                                }
                                Err(e2) => {
                                    tracing::warn!("⚠️ BYE-ERROR-UNKNOWN: Failed to force terminate dialog {} for unknown error: {}", dialog_id, e2);
                                }
                            }
                        }
                    }
                }
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
    
    /// Get access to the underlying dialog API for shutdown coordination
    pub fn dialog_api(&self) -> &Arc<UnifiedDialogApi> {
        &self.dialog_api
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
    
    
    /// Get current SDP for a session
    async fn get_current_sdp(&self, session_id: &SessionId) -> DialogResult<String> {
        // Get internal session from registry
        let session = self.registry
            .get_session(session_id)
            .await
            .map_err(|_| DialogError::SessionNotFound {
                session_id: session_id.0.clone(),
            })?
            .ok_or_else(|| DialogError::SessionNotFound {
                session_id: session_id.0.clone(),
            })?;
        
        // Get local SDP from internal session
        session.local_sdp
            .ok_or_else(|| DialogError::SipProcessing {
                message: "No local SDP found for session".to_string(),
            })
    }
    
    /// Update session SDP after offer/answer exchange
    pub async fn update_session_sdp(
        &self, 
        session_id: &SessionId, 
        local_sdp: Option<String>, 
        remote_sdp: Option<String>
    ) -> DialogResult<()> {
        // Update session SDP using internal registry
        self.registry.update_session_sdp(session_id, local_sdp, remote_sdp).await
            .map_err(|e| DialogError::SipProcessing {
                message: format!("Failed to update session SDP: {}", e),
            })?;
        
        Ok(())
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