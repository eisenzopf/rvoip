//! Session Registry for mapping between SessionId, DialogId, and MediaSessionId
//! 
//! This module provides bidirectional mappings between the different identifiers
//! used across the session-core subsystems. It ensures that we can efficiently
//! look up sessions by any of their associated IDs.

use dashmap::DashMap;
use std::sync::Arc;

use crate::types::{SessionId, DialogId, MediaSessionId, IncomingCallInfo};

/// Registry for mapping between different session identifiers
#[derive(Clone)]
pub struct SessionRegistry {
    /// Map from SessionId to DialogId
    session_to_dialog: Arc<DashMap<SessionId, DialogId>>,
    /// Map from DialogId to SessionId
    dialog_to_session: Arc<DashMap<DialogId, SessionId>>,
    /// Map from SessionId to MediaSessionId
    session_to_media: Arc<DashMap<SessionId, MediaSessionId>>,
    /// Map from MediaSessionId to SessionId
    media_to_session: Arc<DashMap<MediaSessionId, SessionId>>,
    /// Temporary storage for pending incoming calls
    pending_incoming_calls: Arc<DashMap<SessionId, IncomingCallInfo>>,
}

impl SessionRegistry {
    /// Create a new session registry
    pub fn new() -> Self {
        Self {
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            session_to_media: Arc::new(DashMap::new()),
            media_to_session: Arc::new(DashMap::new()),
            pending_incoming_calls: Arc::new(DashMap::new()),
        }
    }

    /// Map a dialog ID to a session ID
    pub fn map_dialog(&self, session_id: SessionId, dialog_id: DialogId) {
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id, session_id);
    }

    /// Map a media session ID to a session ID
    pub fn map_media(&self, session_id: SessionId, media_id: MediaSessionId) {
        self.session_to_media.insert(session_id.clone(), media_id.clone());
        self.media_to_session.insert(media_id, session_id);
    }

    /// Get session ID by dialog ID
    pub fn get_session_by_dialog(&self, dialog_id: &DialogId) -> Option<SessionId> {
        self.dialog_to_session.get(dialog_id).map(|entry| entry.clone())
    }

    /// Get session ID by media session ID
    pub fn get_session_by_media(&self, media_id: &MediaSessionId) -> Option<SessionId> {
        self.media_to_session.get(media_id).map(|entry| entry.clone())
    }

    /// Get dialog ID by session ID
    pub fn get_dialog_by_session(&self, session_id: &SessionId) -> Option<DialogId> {
        self.session_to_dialog.get(session_id).map(|entry| entry.clone())
    }

    /// Get media session ID by session ID
    pub fn get_media_by_session(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        self.session_to_media.get(session_id).map(|entry| entry.clone())
    }

    /// Remove all mappings for a session
    pub fn remove_session(&self, session_id: &SessionId) {
        // Remove dialog mappings
        if let Some(dialog_id) = self.session_to_dialog.remove(session_id) {
            self.dialog_to_session.remove(&dialog_id.1);
        }
        
        // Remove media mappings
        if let Some(media_id) = self.session_to_media.remove(session_id) {
            self.media_to_session.remove(&media_id.1);
        }
    }

    /// Check if a session exists in the registry
    pub fn contains_session(&self, session_id: &SessionId) -> bool {
        self.session_to_dialog.contains_key(session_id) || 
        self.session_to_media.contains_key(session_id)
    }

    /// Get the total number of sessions in the registry
    pub fn session_count(&self) -> usize {
        self.session_to_dialog.len().max(self.session_to_media.len())
    }

    /// Clear all mappings
    pub fn clear(&self) {
        self.session_to_dialog.clear();
        self.dialog_to_session.clear();
        self.session_to_media.clear();
        self.media_to_session.clear();
        self.pending_incoming_calls.clear();
    }
    
    /// Store pending incoming call info
    pub fn store_pending_incoming_call(&self, session_id: SessionId, info: IncomingCallInfo) {
        self.pending_incoming_calls.insert(session_id, info);
    }
    
    /// Get and remove pending incoming call info
    pub fn take_pending_incoming_call(&self, session_id: &SessionId) -> Option<IncomingCallInfo> {
        self.pending_incoming_calls.remove(session_id).map(|(_, info)| info)
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dialog_mapping() {
        let registry = SessionRegistry::new();
        let session_id = SessionId::new();
        let dialog_id = DialogId::new();

        registry.map_dialog(session_id.clone(), dialog_id.clone());

        assert_eq!(registry.get_session_by_dialog(&dialog_id), Some(session_id.clone()));
        assert_eq!(registry.get_dialog_by_session(&session_id), Some(dialog_id));
    }

    #[test]
    fn test_media_mapping() {
        let registry = SessionRegistry::new();
        let session_id = SessionId::new();
        let media_id = MediaSessionId::new();

        registry.map_media(session_id.clone(), media_id.clone());

        assert_eq!(registry.get_session_by_media(&media_id), Some(session_id.clone()));
        assert_eq!(registry.get_media_by_session(&session_id), Some(media_id));
    }

    #[test]
    fn test_remove_session() {
        let registry = SessionRegistry::new();
        let session_id = SessionId::new();
        let dialog_id = DialogId::new();
        let media_id = MediaSessionId::new();

        registry.map_dialog(session_id.clone(), dialog_id.clone());
        registry.map_media(session_id.clone(), media_id.clone());

        assert!(registry.contains_session(&session_id));

        registry.remove_session(&session_id);

        assert!(!registry.contains_session(&session_id));
        assert_eq!(registry.get_session_by_dialog(&dialog_id), None);
        assert_eq!(registry.get_session_by_media(&media_id), None);
    }

    #[test]
    fn test_session_count() {
        let registry = SessionRegistry::new();
        
        assert_eq!(registry.session_count(), 0);

        let session1 = SessionId::new();
        let session2 = SessionId::new();
        
        registry.map_dialog(session1.clone(), DialogId::new());
        registry.map_dialog(session2.clone(), DialogId::new());

        assert_eq!(registry.session_count(), 2);
    }

    #[test]
    fn test_clear() {
        let registry = SessionRegistry::new();
        let session_id = SessionId::new();
        
        registry.map_dialog(session_id.clone(), DialogId::new());
        registry.map_media(session_id.clone(), MediaSessionId(uuid::Uuid::new_v4().to_string()));

        registry.clear();

        assert_eq!(registry.session_count(), 0);
        assert!(!registry.contains_session(&session_id));
    }
}