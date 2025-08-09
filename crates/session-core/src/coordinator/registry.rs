//! Internal Session Registry
//!
//! This module provides the internal registry for storing full Session objects
//! while maintaining clean separation from the public API.

use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use crate::api::types::{SessionId, CallSession, CallState, SessionStats};
use crate::session::Session;
use crate::errors::{Result, SessionError};

/// Internal registry that stores full Session objects
/// 
/// This registry is used internally by the SessionCoordinator and related
/// components to store complete session data including SDP information.
/// It provides conversion methods to expose only CallSession through the public API.
#[derive(Debug)]
pub struct InternalSessionRegistry {
    /// Storage for full Session objects with all internal data
    sessions: Arc<RwLock<HashMap<SessionId, Session>>>,
    /// Statistics tracking
    stats: Arc<RwLock<SessionRegistryStats>>,
}

#[derive(Debug, Default)]
struct SessionRegistryStats {
    total_created: usize,
    total_terminated: usize,
    failed_sessions: usize,
}

impl InternalSessionRegistry {
    /// Create a new internal session registry
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(SessionRegistryStats::default())),
        }
    }

    /// Register a new session (internal use)
    pub async fn register_session(&self, session: Session) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        let mut stats = self.stats.write().await;

        let session_id = session.session_id.clone();
        sessions.insert(session_id.clone(), session);
        stats.total_created += 1;

        tracing::debug!("Registered internal session: {}", session_id);
        Ok(())
    }

    /// Get a full Session object (internal use)
    pub async fn get_session(&self, session_id: &SessionId) -> Result<Option<Session>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(session_id).cloned())
    }

    /// Get a public CallSession (for API use)
    pub async fn get_public_session(&self, session_id: &SessionId) -> Result<Option<CallSession>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(session_id).map(|s| s.as_call_session().clone()))
    }

    /// Update session state
    pub async fn update_session_state(&self, session_id: &SessionId, state: CallState) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        
        if let Some(session) = sessions.get_mut(session_id) {
            session.update_call_state(state)?;
            Ok(())
        } else {
            Err(SessionError::session_not_found(&session_id.0))
        }
    }

    /// Update session SDP data
    pub async fn update_session_sdp(
        &self,
        session_id: &SessionId,
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
    ) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        
        if let Some(session) = sessions.get_mut(session_id) {
            if local_sdp.is_some() {
                session.local_sdp = local_sdp;
            }
            if remote_sdp.is_some() {
                session.remote_sdp = remote_sdp;
            }
            session.updated_at = std::time::Instant::now();
            Ok(())
        } else {
            Err(SessionError::session_not_found(&session_id.0))
        }
    }

    /// Unregister a session
    pub async fn unregister_session(&self, session_id: &SessionId) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        let mut stats = self.stats.write().await;

        if sessions.remove(session_id).is_some() {
            stats.total_terminated += 1;
            tracing::debug!("Unregistered session: {}", session_id);
            Ok(())
        } else {
            Err(SessionError::session_not_found(&session_id.0))
        }
    }

    /// List all active session IDs
    pub async fn list_active_sessions(&self) -> Result<Vec<SessionId>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.keys().cloned().collect())
    }

    /// Get statistics about sessions
    pub async fn get_stats(&self) -> Result<SessionStats> {
        let sessions = self.sessions.read().await;
        let stats = self.stats.read().await;

        let mut active_calls = 0;
        let mut calls_on_hold = 0;

        for session in sessions.values() {
            match session.state() {
                CallState::Active => active_calls += 1,
                CallState::OnHold => calls_on_hold += 1,
                _ => {}
            }
        }

        Ok(SessionStats {
            total_sessions: sessions.len(),
            active_sessions: active_calls,
            failed_sessions: stats.failed_sessions,
            average_duration: None, // TODO: Calculate average duration
        })
    }

    /// Find sessions by state
    pub async fn find_sessions_by_state(&self, state: CallState) -> Vec<Session> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|s| *s.state() == state)
            .cloned()
            .collect()
    }

    /// Update failed session count
    pub async fn increment_failed_sessions(&self) {
        let mut stats = self.stats.write().await;
        stats.failed_sessions += 1;
    }

    /// Check if a session exists
    pub async fn session_exists(&self, session_id: &SessionId) -> bool {
        let sessions = self.sessions.read().await;
        sessions.contains_key(session_id)
    }

    /// Get mutable access to a session (use with caution)
    pub async fn with_session_mut<F, R>(&self, session_id: &SessionId, f: F) -> Result<R>
    where
        F: FnOnce(&mut Session) -> Result<R>,
    {
        let mut sessions = self.sessions.write().await;
        
        if let Some(session) = sessions.get_mut(session_id) {
            f(session)
        } else {
            Err(SessionError::session_not_found(&session_id.0))
        }
    }
}

impl Clone for InternalSessionRegistry {
    fn clone(&self) -> Self {
        Self {
            sessions: Arc::clone(&self.sessions),
            stats: Arc::clone(&self.stats),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::CallState;

    #[tokio::test]
    async fn test_session_registration() {
        let registry = InternalSessionRegistry::new();
        let session_id = SessionId::new();
        let session = Session::new(session_id.clone());

        // Register session
        registry.register_session(session).await.unwrap();

        // Verify it exists
        assert!(registry.session_exists(&session_id).await);

        // Get as internal session
        let internal_session = registry.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(internal_session.session_id, session_id);

        // Get as public session
        let public_session = registry.get_public_session(&session_id).await.unwrap().unwrap();
        assert_eq!(public_session.id, session_id);
    }

    #[tokio::test]
    async fn test_sdp_updates() {
        let registry = InternalSessionRegistry::new();
        let session_id = SessionId::new();
        let session = Session::new(session_id.clone());

        registry.register_session(session).await.unwrap();

        // Update SDP
        let local_sdp = "v=0\r\no=test 123 456 IN IP4 0.0.0.0\r\n";
        let remote_sdp = "v=0\r\no=remote 789 012 IN IP4 0.0.0.0\r\n";
        
        registry.update_session_sdp(
            &session_id,
            Some(local_sdp.to_string()),
            Some(remote_sdp.to_string()),
        ).await.unwrap();

        // Verify SDP was updated
        let session = registry.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(session.local_sdp, Some(local_sdp.to_string()));
        assert_eq!(session.remote_sdp, Some(remote_sdp.to_string()));
    }

    #[tokio::test]
    async fn test_state_updates() {
        let registry = InternalSessionRegistry::new();
        let session_id = SessionId::new();
        let session = Session::new(session_id.clone());

        registry.register_session(session).await.unwrap();

        // Update state
        registry.update_session_state(&session_id, CallState::Active).await.unwrap();

        // Verify state was updated
        let session = registry.get_session(&session_id).await.unwrap().unwrap();
        assert_eq!(*session.state(), CallState::Active);

        // Public API should also see the update
        let public_session = registry.get_public_session(&session_id).await.unwrap().unwrap();
        assert_eq!(*public_session.state(), CallState::Active);
    }
}