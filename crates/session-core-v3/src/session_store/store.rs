use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug};
use crate::state_table::{SessionId, DialogId, MediaSessionId, CallId};

use super::state::SessionState;
use crate::state_table::Role;
use crate::types::CallState;

/// Flexible session storage for session-core-v3
/// 
/// This store supports multiple sessions for legitimate use cases like transfers,
/// while keeping the API simple for single session usage.
pub struct SessionStore {
    /// Multiple session storage
    pub(crate) sessions: Arc<RwLock<std::collections::HashMap<SessionId, SessionState>>>,
    
    /// Index by dialog ID
    pub(crate) by_dialog: Arc<RwLock<std::collections::HashMap<DialogId, SessionId>>>,
    
    /// Index by call ID
    pub(crate) by_call_id: Arc<RwLock<std::collections::HashMap<CallId, SessionId>>>,
    
    /// Index by media session ID
    pub(crate) by_media_id: Arc<RwLock<std::collections::HashMap<MediaSessionId, SessionId>>>,
}

impl SessionStore {
    /// Create a new session store
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            by_dialog: Arc::new(RwLock::new(std::collections::HashMap::new())),
            by_call_id: Arc::new(RwLock::new(std::collections::HashMap::new())),
            by_media_id: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }
    
    /// Create a new session (supports multiple sessions for transfers)
    pub async fn create_session(
        &self,
        session_id: SessionId,
        role: Role,
        with_history: bool,
    ) -> Result<SessionState, Box<dyn std::error::Error + Send + Sync>> {
        let session = if with_history {
            use crate::session_store::HistoryConfig;
            SessionState::with_history(session_id.clone(), role, HistoryConfig::default())
        } else {
            SessionState::new(session_id.clone(), role)
        };
        
        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(&session_id) {
            return Err(format!("Session {} already exists", session_id).into());
        }
        
        sessions.insert(session_id.clone(), session.clone());
        info!("Created new session {} with role {:?}", session_id, role);
        
        Ok(session)
    }
    
    /// Get a session by ID
    pub async fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionState, Box<dyn std::error::Error + Send + Sync>> {
        let sessions = self.sessions.read().await;
        debug!("Looking for session {}, store has {} sessions", session_id, sessions.len());
        for (id, _) in sessions.iter() {
            debug!("  Store contains session: {}", id);
        }
        sessions
            .get(session_id)
            .cloned()
            .ok_or_else(|| format!("Session {} not found", session_id).into())
    }
    
    /// Update a session
    pub async fn update_session(
        &self,
        session: SessionState,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let session_id = session.session_id.clone();
        let mut sessions = self.sessions.write().await;
        
        // Update indexes if IDs have changed
        if let Some(old_session) = sessions.get(&session_id) {
            // Remove old indexes
            if old_session.dialog_id != session.dialog_id {
                if let Some(old_id) = &old_session.dialog_id {
                    self.by_dialog.write().await.remove(old_id);
                }
                if let Some(new_id) = &session.dialog_id {
                    self.by_dialog.write().await.insert(new_id.clone(), session_id.clone());
                }
            }
            
            if old_session.media_session_id != session.media_session_id {
                if let Some(old_id) = &old_session.media_session_id {
                    self.by_media_id.write().await.remove(old_id);
                }
                if let Some(new_id) = &session.media_session_id {
                    self.by_media_id.write().await.insert(new_id.clone(), session_id.clone());
                }
            }
            
            if old_session.call_id != session.call_id {
                if let Some(old_id) = &old_session.call_id {
                    self.by_call_id.write().await.remove(old_id);
                }
                if let Some(new_id) = &session.call_id {
                    self.by_call_id.write().await.insert(new_id.clone(), session_id.clone());
                }
            }
        }
        
        sessions.insert(session_id.clone(), session);
        debug!("Updated session {}", session_id);
        
        Ok(())
    }
    
    /// Remove a session
    pub async fn remove_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut sessions = self.sessions.write().await;
        
        if let Some(session) = sessions.remove(session_id) {
            // Clean up indexes
            if let Some(dialog_id) = &session.dialog_id {
                self.by_dialog.write().await.remove(dialog_id);
            }
            if let Some(media_id) = &session.media_session_id {
                self.by_media_id.write().await.remove(media_id);
            }
            if let Some(call_id) = &session.call_id {
                self.by_call_id.write().await.remove(call_id);
            }
            
            info!("Removed session {}", session_id);
            Ok(())
        } else {
            Err(format!("Session {} not found", session_id).into())
        }
    }
    
    /// Find session by dialog ID
    pub async fn find_by_dialog(
        &self,
        dialog_id: &DialogId,
    ) -> Option<SessionState> {
        let by_dialog = self.by_dialog.read().await;
        if let Some(session_id) = by_dialog.get(dialog_id) {
            self.get_session(session_id).await.ok()
        } else {
            None
        }
    }
    
    /// Find session by media session ID
    pub async fn find_by_media_id(
        &self,
        media_id: &MediaSessionId,
    ) -> Option<SessionState> {
        let by_media = self.by_media_id.read().await;
        if let Some(session_id) = by_media.get(media_id) {
            self.get_session(session_id).await.ok()
        } else {
            None
        }
    }
    
    /// Find session by call ID
    pub async fn find_by_call_id(
        &self,
        call_id: &CallId,
    ) -> Option<SessionState> {
        let by_call = self.by_call_id.read().await;
        if let Some(session_id) = by_call.get(call_id) {
            self.get_session(session_id).await.ok()
        } else {
            None
        }
    }
    
    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Vec<SessionState> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    // ===== Multi-Session Utility Methods =====
    
    /// Check if any sessions exist
    pub async fn has_session(&self) -> bool {
        !self.sessions.read().await.is_empty()
    }
    
    /// Get the most recent session ID (for API compatibility)
    pub async fn get_current_session_id(&self) -> Option<SessionId> {
        let sessions = self.sessions.read().await;
        // Return the most recently created session
        sessions.values()
            .max_by_key(|s| s.created_at)
            .map(|s| s.session_id.clone())
    }
    
    /// Clear all session data (complete reset)
    pub async fn clear(&self) {
        self.sessions.write().await.clear();
        self.by_dialog.write().await.clear();
        self.by_call_id.write().await.clear();
        self.by_media_id.write().await.clear();
        info!("Cleared all session data");
    }
    
    /* Old cleanup - replaced by cleanup.rs
    /// Clean up stale sessions
    pub async fn cleanup_stale_sessions_old(&self, max_age: Duration) {
        let mut sessions = self.sessions.write().await;
        let now = Instant::now();
        let mut to_remove = Vec::new();
        
        for (id, session) in sessions.iter() {
            let should_remove = match session.call_state {
                CallState::Terminated | CallState::Failed(_) => {
                    // Keep terminated sessions for a short time
                    session.time_in_state() > Duration::from_secs(300)
                }
                _ => {
                    // Remove long-idle sessions
                    session.session_duration() > max_age
                }
            };
            
            if should_remove {
                to_remove.push(id.clone());
            }
        }
        
        for id in to_remove {
            if let Some(session) = sessions.remove(&id) {
                // Clean up indexes
                if let Some(dialog_id) = &session.dialog_id {
                    self.by_dialog.write().await.remove(dialog_id);
                }
                if let Some(media_id) = &session.media_session_id {
                    self.by_media_id.write().await.remove(media_id);
                }
                if let Some(call_id) = &session.call_id {
                    self.by_call_id.write().await.remove(call_id);
                }
                
                warn!("Cleaned up stale session {}", id);
            }
        }
    }
    */
    
    /// Get session statistics
    pub async fn get_stats(&self) -> SessionStats {
        let sessions = self.sessions.read().await;
        let mut stats = SessionStats::default();
        
        for session in sessions.values() {
            stats.total += 1;
            match session.call_state {
                CallState::Idle => stats.idle += 1,
                CallState::Initiating => stats.initiating += 1,
                CallState::Ringing => stats.ringing += 1,
                CallState::Answering => stats.ringing += 1,  // Still in setup phase
                CallState::EarlyMedia => stats.active += 1,  // Count early media as active
                CallState::Active => stats.active += 1,
                CallState::OnHold => stats.on_hold += 1,
                CallState::Resuming => stats.active += 1,  // Count resuming as active
                CallState::Bridged => stats.active += 1,  // Count bridged as active
                CallState::Transferring => stats.active += 1,  // Count transferring as active
                CallState::TransferringCall => stats.active += 1,  // Count transfer recipient as active
                CallState::Terminating => stats.terminating += 1,
                CallState::Terminated => stats.terminated += 1,
                CallState::Cancelled => stats.terminated += 1,  // Count cancelled as terminated
                CallState::Failed(_) => stats.failed += 1,
                CallState::Muted => stats.active += 1, // Count as active
                CallState::ConsultationCall => stats.active += 1, // Count as active
                
                // Registration states
                CallState::Registering => stats.initiating += 1, // Count as initiating
                CallState::Registered => stats.idle += 1, // Count as idle (ready for calls)
                CallState::Unregistering => stats.terminating += 1, // Count as terminating
                
                // Subscription/Presence states
                CallState::Subscribing => stats.initiating += 1, // Count as initiating
                CallState::Subscribed => stats.idle += 1, // Count as idle (subscription active)
                CallState::Publishing => stats.initiating += 1, // Count as initiating
                
                // Authentication and routing states
                CallState::Authenticating => stats.initiating += 1, // Count as initiating
                CallState::Messaging => stats.active += 1, // Count as active
            }
        }
        
        stats
    }
}

/// Session statistics
#[derive(Debug, Default, Clone)]
pub struct SessionStats {
    pub total: usize,
    pub idle: usize,
    pub initiating: usize,
    pub ringing: usize,
    pub active: usize,
    pub on_hold: usize,
    pub terminating: usize,
    pub terminated: usize,
    pub failed: usize,
}