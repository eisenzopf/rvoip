//! Session Registry
//!
//! Manages storage and lookup of active sessions.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::api::types::{SessionId, CallSession, SessionStats};
use crate::errors::Result;

/// Registry for managing active sessions
#[derive(Debug)]
pub struct SessionRegistry {
    sessions: Arc<RwLock<HashMap<SessionId, CallSession>>>,
    stats: Arc<RwLock<SessionRegistryStats>>,
}

#[derive(Debug, Default)]
struct SessionRegistryStats {
    total_created: usize,
    total_terminated: usize,
    failed_sessions: usize,
}

impl SessionRegistry {
    /// Create a new session registry
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(SessionRegistryStats::default())),
        }
    }

    /// Register a new session
    pub async fn register_session(&self, session_id: SessionId, session: CallSession) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        let mut stats = self.stats.write().await;

        sessions.insert(session_id.clone(), session);
        stats.total_created += 1;

        tracing::debug!("Registered session: {}", session_id);
        Ok(())
    }

    /// Unregister a session
    pub async fn unregister_session(&self, session_id: &SessionId) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        let mut stats = self.stats.write().await;

        if sessions.remove(session_id).is_some() {
            stats.total_terminated += 1;
            tracing::debug!("Unregistered session: {}", session_id);
        } else {
            tracing::warn!("Attempted to unregister unknown session: {}", session_id);
        }

        Ok(())
    }

    /// Get a session by ID
    pub async fn get_session(&self, session_id: &SessionId) -> Result<Option<CallSession>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.get(session_id).cloned())
    }

    /// Update a session
    pub async fn update_session(&self, session_id: SessionId, session: CallSession) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id, session);
        Ok(())
    }

    /// List all active session IDs
    pub async fn list_active_sessions(&self) -> Result<Vec<SessionId>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.keys().cloned().collect())
    }

    /// Get the number of active sessions
    pub async fn active_session_count(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.len()
    }

    /// Get registry statistics
    pub async fn get_stats(&self) -> Result<SessionStats> {
        let sessions = self.sessions.read().await;
        let stats = self.stats.read().await;

        Ok(SessionStats {
            total_sessions: stats.total_created,
            active_sessions: sessions.len(),
            failed_sessions: stats.failed_sessions,
            average_duration: None, // TODO: Calculate from session data
        })
    }

    /// Mark a session as failed
    pub async fn mark_session_failed(&self, session_id: &SessionId) -> Result<()> {
        let mut stats = self.stats.write().await;
        stats.failed_sessions += 1;
        
        // Also remove from active sessions
        self.unregister_session(session_id).await?;
        
        tracing::warn!("Marked session as failed: {}", session_id);
        Ok(())
    }

    /// Get all active sessions
    pub async fn get_all_sessions(&self) -> Result<Vec<CallSession>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.values().cloned().collect())
    }

    /// Find sessions by criteria
    pub async fn find_sessions_by_caller(&self, caller: &str) -> Result<Vec<CallSession>> {
        let sessions = self.sessions.read().await;
        Ok(sessions.values()
            .filter(|session| session.from == caller)
            .cloned()
            .collect())
    }

    /// Clear all sessions (for shutdown)
    pub async fn clear_all(&self) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        let count = sessions.len();
        sessions.clear();
        
        tracing::info!("Cleared {} sessions from registry", count);
        Ok(())
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
} 