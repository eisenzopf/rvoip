use std::sync::Arc;
use dashmap::DashMap;
use tracing::{info, debug};
use crate::state_table::{SessionId, DialogId, MediaSessionId, CallId};

use super::state::SessionState;
use crate::state_table::Role;
use crate::types::CallState;

/// Flexible session storage for session-core-v3
/// 
/// This store supports multiple sessions for legitimate use cases like transfers,
/// while keeping the API simple for single session usage.
/// 
/// Uses DashMap for lock-free concurrent access to avoid deadlocks.
pub struct SessionStore {
    /// Multiple session storage (lock-free with DashMap)
    pub(crate) sessions: Arc<DashMap<SessionId, SessionState>>,
    
    /// Index by dialog ID (lock-free with DashMap)
    pub(crate) by_dialog: Arc<DashMap<DialogId, SessionId>>,
    
    /// Index by call ID (lock-free with DashMap)
    pub(crate) by_call_id: Arc<DashMap<CallId, SessionId>>,
    
    /// Index by media session ID (lock-free with DashMap)
    pub(crate) by_media_id: Arc<DashMap<MediaSessionId, SessionId>>,
}

impl SessionStore {
    /// Create a new session store
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            by_dialog: Arc::new(DashMap::new()),
            by_call_id: Arc::new(DashMap::new()),
            by_media_id: Arc::new(DashMap::new()),
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
        
        // DashMap: Lock-free insert with check
        if self.sessions.contains_key(&session_id) {
            return Err(format!("Session {} already exists", session_id).into());
        }
        
        self.sessions.insert(session_id.clone(), session.clone());
        info!("Created new session {} with role {:?}", session_id, role);
        
        Ok(session)
    }
    
    /// Get a session by ID
    pub async fn get_session(
        &self,
        session_id: &SessionId,
    ) -> Result<SessionState, Box<dyn std::error::Error + Send + Sync>> {
        // DashMap: Lock-free read
        debug!("Looking for session {}, store has {} sessions", session_id, self.sessions.len());
        self.sessions
            .get(session_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| format!("Session {} not found", session_id).into())
    }
    
    /// Update a session
    pub async fn update_session(
        &self,
        session: SessionState,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let session_id = session.session_id.clone();
        
        // DashMap: Lock-free update with index management
        // Get old session to check for index changes
        if let Some(old_entry) = self.sessions.get(&session_id) {
            let old_session = old_entry.value();
            
            // Update indexes if IDs have changed
            if old_session.dialog_id != session.dialog_id {
                if let Some(old_id) = &old_session.dialog_id {
                    self.by_dialog.remove(old_id);
                }
                if let Some(new_id) = &session.dialog_id {
                    self.by_dialog.insert(new_id.clone(), session_id.clone());
                }
            }
            
            if old_session.media_session_id != session.media_session_id {
                if let Some(old_id) = &old_session.media_session_id {
                    self.by_media_id.remove(old_id);
                }
                if let Some(new_id) = &session.media_session_id {
                    self.by_media_id.insert(new_id.clone(), session_id.clone());
                }
            }
            
            if old_session.call_id != session.call_id {
                if let Some(old_id) = &old_session.call_id {
                    self.by_call_id.remove(old_id);
                }
                if let Some(new_id) = &session.call_id {
                    self.by_call_id.insert(new_id.clone(), session_id.clone());
                }
            }
        }
        
        self.sessions.insert(session_id.clone(), session);
        debug!("Updated session {}", session_id);
        
        Ok(())
    }
    
    /// Remove a session
    pub async fn remove_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // DashMap: Lock-free remove
        if let Some((_, session)) = self.sessions.remove(session_id) {
            // Clean up indexes
            if let Some(dialog_id) = &session.dialog_id {
                self.by_dialog.remove(dialog_id);
            }
            if let Some(media_id) = &session.media_session_id {
                self.by_media_id.remove(media_id);
            }
            if let Some(call_id) = &session.call_id {
                self.by_call_id.remove(call_id);
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
        // DashMap: Lock-free lookup
        self.by_dialog.get(dialog_id)
            .and_then(|entry| {
                let session_id = entry.value();
                self.sessions.get(session_id).map(|s| s.value().clone())
            })
    }
    
    /// Find session by media session ID
    pub async fn find_by_media_id(
        &self,
        media_id: &MediaSessionId,
    ) -> Option<SessionState> {
        // DashMap: Lock-free lookup
        self.by_media_id.get(media_id)
            .and_then(|entry| {
                let session_id = entry.value();
                self.sessions.get(session_id).map(|s| s.value().clone())
            })
    }
    
    /// Find session by call ID
    pub async fn find_by_call_id(
        &self,
        call_id: &CallId,
    ) -> Option<SessionState> {
        // DashMap: Lock-free lookup
        self.by_call_id.get(call_id)
            .and_then(|entry| {
                let session_id = entry.value();
                self.sessions.get(session_id).map(|s| s.value().clone())
            })
    }
    
    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Vec<SessionState> {
        // DashMap: Lock-free iteration
        self.sessions.iter().map(|entry| entry.value().clone()).collect()
    }

    // ===== Multi-Session Utility Methods =====
    
    /// Check if any sessions exist
    pub async fn has_session(&self) -> bool {
        // DashMap: Lock-free check
        !self.sessions.is_empty()
    }
    
    /// Get the most recent session ID (for API compatibility)
    pub async fn get_current_session_id(&self) -> Option<SessionId> {
        // DashMap: Lock-free iteration
        self.sessions.iter()
            .max_by_key(|entry| entry.value().created_at)
            .map(|entry| entry.key().clone())
    }
    
    /// Clear all session data (complete reset)
    pub async fn clear(&self) {
        // DashMap: Lock-free clear
        self.sessions.clear();
        self.by_dialog.clear();
        self.by_call_id.clear();
        self.by_media_id.clear();
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
        // DashMap: Lock-free iteration
        let mut stats = SessionStats::default();
        
        for entry in self.sessions.iter() {
            let session = entry.value();
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