use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, debug};
use crate::state_table::{SessionId, DialogId, MediaSessionId, CallId};

use super::state::SessionState;
use crate::state_table::Role;
use crate::state_table::CallState;

/// Session storage with indexes for fast lookup
pub struct SessionStore {
    /// Primary storage - all active sessions
    pub(crate) sessions: Arc<RwLock<HashMap<SessionId, SessionState>>>,

    /// Index by dialog ID
    pub(crate) by_dialog: Arc<RwLock<HashMap<DialogId, SessionId>>>,

    /// Index by call ID
    pub(crate) by_call_id: Arc<RwLock<HashMap<CallId, SessionId>>>,

    /// Index by media session ID
    pub(crate) by_media_id: Arc<RwLock<HashMap<MediaSessionId, SessionId>>>,
}

impl SessionStore {
    /// Create a new session store
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            by_dialog: Arc::new(RwLock::new(HashMap::new())),
            by_call_id: Arc::new(RwLock::new(HashMap::new())),
            by_media_id: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new session
    pub async fn create_session(
        &self,
        session_id: SessionId,
        role: Role,
        with_history: bool,
    ) -> Result<SessionState, Box<dyn std::error::Error + Send + Sync>> {
        let session = if with_history {
            use crate::session_store_v2::HistoryConfig;
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
    ///
    /// Lock ordering: acquire `sessions` first, compute index diffs, release `sessions`,
    /// then update index maps.
    pub async fn update_session(
        &self,
        session: SessionState,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let session_id = session.session_id.clone();

        let index_changes = {
            let mut sessions = self.sessions.write().await;

            let changes = if let Some(old_session) = sessions.get(&session_id) {
                let dialog_changed = old_session.dialog_id != session.dialog_id;
                let media_changed = old_session.media_session_id != session.media_session_id;
                let call_id_changed = old_session.call_id != session.call_id;

                Some(IndexChanges {
                    dialog_changed,
                    old_dialog_id: if dialog_changed { old_session.dialog_id.clone() } else { None },
                    new_dialog_id: if dialog_changed { session.dialog_id.clone() } else { None },
                    media_changed,
                    old_media_id: if media_changed { old_session.media_session_id.clone() } else { None },
                    new_media_id: if media_changed { session.media_session_id.clone() } else { None },
                    call_id_changed,
                    old_call_id: if call_id_changed { old_session.call_id.clone() } else { None },
                    new_call_id: if call_id_changed { session.call_id.clone() } else { None },
                })
            } else {
                None
            };

            sessions.insert(session_id.clone(), session);
            changes
        };

        if let Some(changes) = index_changes {
            if changes.dialog_changed {
                let mut by_dialog = self.by_dialog.write().await;
                if let Some(old_id) = &changes.old_dialog_id {
                    by_dialog.remove(old_id);
                }
                if let Some(new_id) = &changes.new_dialog_id {
                    by_dialog.insert(new_id.clone(), session_id.clone());
                }
            }

            if changes.media_changed {
                let mut by_media = self.by_media_id.write().await;
                if let Some(old_id) = &changes.old_media_id {
                    by_media.remove(old_id);
                }
                if let Some(new_id) = &changes.new_media_id {
                    by_media.insert(new_id.clone(), session_id.clone());
                }
            }

            if changes.call_id_changed {
                let mut by_call = self.by_call_id.write().await;
                if let Some(old_id) = &changes.old_call_id {
                    by_call.remove(old_id);
                }
                if let Some(new_id) = &changes.new_call_id {
                    by_call.insert(new_id.clone(), session_id.clone());
                }
            }
        }

        debug!("Updated session {}", session_id);

        Ok(())
    }

    /// Remove a session
    pub async fn remove_session(
        &self,
        session_id: &SessionId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let removed = {
            let mut sessions = self.sessions.write().await;
            sessions.remove(session_id)
        };

        if let Some(session) = removed {
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
        let session_id = {
            let by_dialog = self.by_dialog.read().await;
            by_dialog.get(dialog_id).cloned()
        };
        if let Some(session_id) = session_id {
            self.get_session(&session_id).await.ok()
        } else {
            None
        }
    }

    /// Find session by media session ID
    pub async fn find_by_media_id(
        &self,
        media_id: &MediaSessionId,
    ) -> Option<SessionState> {
        let session_id = {
            let by_media = self.by_media_id.read().await;
            by_media.get(media_id).cloned()
        };
        if let Some(session_id) = session_id {
            self.get_session(&session_id).await.ok()
        } else {
            None
        }
    }

    /// Find session by call ID
    pub async fn find_by_call_id(
        &self,
        call_id: &CallId,
    ) -> Option<SessionState> {
        let session_id = {
            let by_call = self.by_call_id.read().await;
            by_call.get(call_id).cloned()
        };
        if let Some(session_id) = session_id {
            self.get_session(&session_id).await.ok()
        } else {
            None
        }
    }

    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Vec<SessionState> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }

    /// Mark two sessions as bridged for conferencing
    pub async fn bridge_sessions(
        &self,
        inbound_id: &SessionId,
        outbound_id: &SessionId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut sessions = self.sessions.write().await;

        if let Some(session1) = sessions.get_mut(inbound_id) {
            session1.bridged_to = Some(outbound_id.clone());
        } else {
            return Err(format!("Session {} not found", inbound_id).into());
        }

        if let Some(session2) = sessions.get_mut(outbound_id) {
            session2.bridged_to = Some(inbound_id.clone());
        } else {
            if let Some(session1) = sessions.get_mut(inbound_id) {
                session1.bridged_to = None;
            }
            return Err(format!("Session {} not found", outbound_id).into());
        }

        info!("Bridged sessions {} <-> {}", inbound_id, outbound_id);
        Ok(())
    }

    /// Unbridge two sessions
    pub async fn unbridge_sessions(
        &self,
        session_id: &SessionId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut sessions = self.sessions.write().await;

        let bridged_id = if let Some(session) = sessions.get(session_id) {
            session.bridged_to.clone()
        } else {
            return Err(format!("Session {} not found", session_id).into());
        };

        if let Some(session) = sessions.get_mut(session_id) {
            session.bridged_to = None;
        }

        if let Some(bridged_id) = bridged_id {
            if let Some(bridged) = sessions.get_mut(&bridged_id) {
                bridged.bridged_to = None;
            }
            info!("Unbridged sessions {} <-> {}", session_id, bridged_id);
        }

        Ok(())
    }

    /// Get the bridged partner session
    pub async fn get_bridged_partner(
        &self,
        session_id: &SessionId,
    ) -> Option<SessionState> {
        let sessions = self.sessions.read().await;
        if let Some(session) = sessions.get(session_id) {
            if let Some(bridged_id) = &session.bridged_to {
                return sessions.get(bridged_id).cloned();
            }
        }
        None
    }

    /// Get all bridged session pairs
    pub async fn get_bridged_pairs(&self) -> Vec<(SessionId, SessionId)> {
        let sessions = self.sessions.read().await;
        let mut pairs = Vec::new();
        let mut processed = std::collections::HashSet::new();

        for (id, session) in sessions.iter() {
            if !processed.contains(id) {
                if let Some(bridged_id) = &session.bridged_to {
                    if !processed.contains(bridged_id) {
                        pairs.push((id.clone(), bridged_id.clone()));
                        processed.insert(id.clone());
                        processed.insert(bridged_id.clone());
                    }
                }
            }
        }

        pairs
    }

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
                CallState::Answering => stats.ringing += 1,
                CallState::EarlyMedia => stats.active += 1,
                CallState::Active => stats.active += 1,
                CallState::OnHold => stats.on_hold += 1,
                CallState::Resuming => stats.active += 1,
                CallState::Bridged => stats.active += 1,
                CallState::Transferring => stats.active += 1,
                CallState::TransferringCall => stats.active += 1,
                CallState::Terminating => stats.terminating += 1,
                CallState::Terminated => stats.terminated += 1,
                CallState::Cancelled => stats.terminated += 1,
                CallState::Failed(_) => stats.failed += 1,
                CallState::Muted => stats.active += 1,
                CallState::ConsultationCall => stats.active += 1,
                CallState::Registering => stats.initiating += 1,
                CallState::Registered => stats.idle += 1,
                CallState::Unregistering => stats.terminating += 1,
                CallState::Subscribing => stats.initiating += 1,
                CallState::Subscribed => stats.idle += 1,
                CallState::Publishing => stats.initiating += 1,
                CallState::Authenticating => stats.initiating += 1,
                CallState::Messaging => stats.active += 1,
            }
        }

        stats
    }
}

/// Temporary holder for index changes
struct IndexChanges {
    dialog_changed: bool,
    old_dialog_id: Option<DialogId>,
    new_dialog_id: Option<DialogId>,
    media_changed: bool,
    old_media_id: Option<MediaSessionId>,
    new_media_id: Option<MediaSessionId>,
    call_id_changed: bool,
    old_call_id: Option<CallId>,
    new_call_id: Option<CallId>,
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
