//! Session Manager - Core session lifecycle management
//!
//! This module provides the central session management functionality,
//! handling session creation, state transitions, and lifecycle events.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tracing::info;

use crate::types::{
    DialogId, MediaSessionId, SessionEvent,
    CallDirection, MediaState, CallDetailRecord,
};
use crate::state_table::types::{SessionId, CallState};
use crate::session_registry::SessionRegistry;
use crate::state_machine::StateMachine;
use crate::state_table::types::EventType;
use crate::session_store::SessionState;
use crate::errors::{Result, SessionError};

/// Session metadata
#[derive(Debug, Clone)]
pub struct SessionMetadata {
    /// Session ID
    pub session_id: SessionId,
    /// Associated dialog ID
    pub dialog_id: Option<DialogId>,
    /// Associated media session ID
    pub media_id: Option<MediaSessionId>,
    /// Call direction
    pub direction: CallDirection,
    /// Current call state
    pub call_state: CallState,
    /// Media state
    pub media_state: MediaState,
    /// From URI
    pub from: String,
    /// To URI
    pub to: String,
    /// SIP Call-ID
    pub call_id: Option<String>,
    /// Session creation time
    pub created_at: std::time::SystemTime,
    /// Call start time
    pub started_at: Option<std::time::SystemTime>,
    /// Call end time
    pub ended_at: Option<std::time::SystemTime>,
}

/// Session store for managing active sessions
#[derive(Clone)]
struct SessionStore {
    /// Active sessions
    sessions: Arc<RwLock<HashMap<SessionId, SessionMetadata>>>,
    /// Session states for state machine
    states: Arc<RwLock<HashMap<SessionId, SessionState>>>,
}

impl SessionStore {
    fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            states: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn insert(&self, session_id: SessionId, metadata: SessionMetadata, state: SessionState) {
        self.sessions.write().await.insert(session_id.clone(), metadata);
        self.states.write().await.insert(session_id, state);
    }

    async fn get_metadata(&self, session_id: &SessionId) -> Option<SessionMetadata> {
        self.sessions.read().await.get(session_id).cloned()
    }

    async fn get_state(&self, session_id: &SessionId) -> Option<SessionState> {
        self.states.read().await.get(session_id).cloned()
    }

    async fn update_state(&self, session_id: &SessionId, state: SessionState) -> Result<()> {
        let mut states = self.states.write().await;
        if states.contains_key(session_id) {
            states.insert(session_id.clone(), state);
            Ok(())
        } else {
            Err(SessionError::SessionNotFound(session_id.to_string()))
        }
    }

    async fn update_metadata<F>(&self, session_id: &SessionId, f: F) -> Result<()>
    where
        F: FnOnce(&mut SessionMetadata),
    {
        let mut sessions = self.sessions.write().await;
        if let Some(metadata) = sessions.get_mut(session_id) {
            f(metadata);
            Ok(())
        } else {
            Err(SessionError::SessionNotFound(session_id.to_string()))
        }
    }

    async fn remove(&self, session_id: &SessionId) -> Result<()> {
        self.sessions.write().await.remove(session_id);
        self.states.write().await.remove(session_id);
        Ok(())
    }

    async fn list_sessions(&self) -> Vec<SessionMetadata> {
        self.sessions.read().await.values().cloned().collect()
    }

    async fn count(&self) -> usize {
        self.sessions.read().await.len()
    }
}

/// Session Manager for handling session lifecycle
pub struct SessionManager {
    /// Session store
    store: SessionStore,
    /// Session registry for ID mappings
    registry: Arc<SessionRegistry>,
    /// State machine for processing events
    state_machine: Arc<StateMachine>,
    /// Event sender for state machine
    event_tx: mpsc::Sender<(SessionId, EventType)>,
    /// Notification channel for session lifecycle events
    notification_tx: mpsc::Sender<SessionLifecycleEvent>,
}

/// Session lifecycle events
#[derive(Debug, Clone)]
pub enum SessionLifecycleEvent {
    /// Session was created
    Created { session_id: SessionId },
    /// Incoming call received
    IncomingCall {
        session_id: SessionId,
        from: String,
    },
    /// Session state changed
    StateChanged {
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
    },
    /// Session was terminated
    Terminated {
        session_id: SessionId,
        reason: Option<String>,
    },
    /// Session encountered an error
    Error {
        session_id: SessionId,
        error: String,
    },
}

impl SessionManager {
    /// Create a new session manager
    pub fn new(
        registry: Arc<SessionRegistry>,
        state_machine: Arc<StateMachine>,
        event_tx: mpsc::Sender<(SessionId, EventType)>,
        notification_tx: mpsc::Sender<SessionLifecycleEvent>,
    ) -> Self {
        Self {
            store: SessionStore::new(),
            registry,
            state_machine,
            event_tx,
            notification_tx,
        }
    }

    /// Create a new session
    pub async fn create_session(
        &self,
        from: String,
        to: String,
        direction: CallDirection,
    ) -> Result<SessionId> {
        let session_id = SessionId::new();
        
        let metadata = SessionMetadata {
            session_id: session_id.clone(),
            dialog_id: None,
            media_id: None,
            direction: direction.clone(),
            call_state: CallState::Initiating,
            media_state: MediaState::Idle,
            from,
            to,
            call_id: None,
            created_at: std::time::SystemTime::now(),
            started_at: None,
            ended_at: None,
        };

        let role = match direction {
            CallDirection::Outgoing => crate::state_table::types::Role::UAC,
            CallDirection::Incoming => crate::state_table::types::Role::UAS,
        };
        let state = SessionState::new(session_id.clone(), role);
        
        self.store.insert(session_id.clone(), metadata, state).await;
        
        // Send creation notification
        let _ = self.notification_tx.send(SessionLifecycleEvent::Created {
            session_id: session_id.clone(),
        }).await;
        
        info!("Created session {}", session_id);
        Ok(session_id)
    }

    /// Get session metadata
    pub async fn get_session(&self, session_id: &SessionId) -> Option<SessionMetadata> {
        self.store.get_metadata(session_id).await
    }

    /// Update session state
    pub async fn update_session_state(
        &self,
        session_id: &SessionId,
        new_state: CallState,
    ) -> Result<()> {
        let old_state = self.store.get_metadata(session_id).await
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?
            .call_state;

        self.store.update_metadata(session_id, |metadata| {
            metadata.call_state = new_state.clone();
            
            // Update timestamps
            match &new_state {
                CallState::Active => {
                    metadata.started_at = Some(std::time::SystemTime::now());
                }
                CallState::Terminated | CallState::Failed(_) | CallState::Cancelled => {
                    metadata.ended_at = Some(std::time::SystemTime::now());
                }
                _ => {}
            }
        }).await?;

        // Send state change notification
        let _ = self.notification_tx.send(SessionLifecycleEvent::StateChanged {
            session_id: session_id.clone(),
            old_state,
            new_state,
        }).await;

        Ok(())
    }

    /// Map a dialog ID to a session
    pub fn map_dialog(&self, session_id: SessionId, dialog_id: DialogId) {
        self.registry.map_dialog(session_id.clone(), dialog_id.clone());
        
        // Update metadata
        let store = self.store.clone();
        let session_id_clone = session_id.clone();
        tokio::spawn(async move {
            let _ = store.update_metadata(&session_id_clone, |metadata| {
                metadata.dialog_id = Some(dialog_id);
            }).await;
        });
    }

    /// Map a media session ID to a session
    pub fn map_media(&self, session_id: SessionId, media_id: MediaSessionId) {
        self.registry.map_media(session_id.clone(), media_id.clone());
        
        // Update metadata
        let store = self.store.clone();
        let session_id_clone = session_id.clone();
        tokio::spawn(async move {
            let _ = store.update_metadata(&session_id_clone, |metadata| {
                metadata.media_id = Some(media_id);
            }).await;
        });
    }

    /// Process an event through the state machine
    pub async fn process_event(
        &self,
        session_id: &SessionId,
        event: SessionEvent,
    ) -> Result<()> {
        // Get current state
        let state = self.store.get_state(session_id).await
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        // Convert to state machine event
        let state_event = self.convert_to_state_event(event)?;

        // Send to state machine
        self.event_tx.send((session_id.clone(), state_event)).await
            .map_err(|e| SessionError::Other(format!("Failed to send event: {}", e)))?;

        Ok(())
    }

    /// Terminate a session
    pub async fn terminate_session(
        &self,
        session_id: &SessionId,
        reason: Option<String>,
    ) -> Result<()> {
        // Update state
        self.update_session_state(session_id, CallState::Terminated).await?;

        // Clean up registry mappings
        self.registry.remove_session(session_id);

        // Send termination notification
        let _ = self.notification_tx.send(SessionLifecycleEvent::Terminated {
            session_id: session_id.clone(),
            reason,
        }).await;

        // Remove from store
        self.store.remove(session_id).await?;

        info!("Terminated session {}", session_id);
        Ok(())
    }

    /// Get all active sessions
    pub async fn list_sessions(&self) -> Vec<SessionMetadata> {
        self.store.list_sessions().await
    }

    /// Get session count
    pub async fn session_count(&self) -> usize {
        self.store.count().await
    }

    /// Generate call detail record for a session
    pub async fn generate_cdr(&self, session_id: &SessionId) -> Result<CallDetailRecord> {
        let metadata = self.store.get_metadata(session_id).await
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))?;

        let duration = match (metadata.started_at, metadata.ended_at) {
            (Some(start), Some(end)) => {
                end.duration_since(start).ok().map(|d| d.as_secs())
            }
            _ => None
        };

        Ok(CallDetailRecord {
            session_id: session_id.clone(),
            dialog_id: metadata.dialog_id.unwrap_or_else(DialogId::new),
            from: metadata.from,
            to: metadata.to,
            start_time: metadata.created_at,
            end_time: metadata.ended_at,
            duration,
            termination_reason: match metadata.call_state {
                CallState::Failed(reason) => Some(format!("{:?}", reason)),
                CallState::Cancelled => Some("Cancelled".to_string()),
                _ => None,
            },
            call_id: metadata.call_id.unwrap_or_else(|| "unknown".to_string()),
        })
    }

    /// Convert SessionEvent to EventType for state machine
    fn convert_to_state_event(&self, event: SessionEvent) -> Result<crate::state_table::types::EventType> {
        use crate::state_table::types::EventType;
        // This is a simplified conversion - in reality would be more complex
        match event {
            SessionEvent::IncomingCall { from, sdp, .. } => Ok(EventType::IncomingCall { from, sdp }),
            SessionEvent::CallAnswered { .. } => Ok(EventType::AcceptCall),
            SessionEvent::CallTerminated { .. } => Ok(EventType::HangupCall),
            SessionEvent::CallFailed { .. } => Ok(EventType::DialogTerminated),
            _ => Ok(EventType::CheckConditions),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::unified::{UnifiedCoordinator, Config};
    use std::net::IpAddr;

    async fn create_test_coordinator() -> Arc<UnifiedCoordinator> {
        let config = Config {
            sip_port: 15060,
            media_port_start: 16000,
            media_port_end: 17000,
            local_ip: "127.0.0.1".parse().unwrap(),
            bind_addr: "127.0.0.1:15060".parse().unwrap(),
            state_table_path: None,
        };
        UnifiedCoordinator::new(config).await.unwrap()
    }

    #[tokio::test]
    async fn test_create_session() {
        let coordinator = create_test_coordinator().await;
        let session_manager = coordinator.session_manager().await.unwrap();

        let session_id = session_manager.create_session(
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
            CallDirection::Outgoing,
        ).await.unwrap();

        let metadata = session_manager.get_session(&session_id).await.unwrap();
        assert_eq!(metadata.from, "alice@example.com");
        assert_eq!(metadata.to, "bob@example.com");
        assert_eq!(metadata.call_state, CallState::Initiating);
    }

    #[tokio::test]
    async fn test_update_session_state() {
        let config = Config {
            sip_port: 15070,
            media_port_start: 17000,
            media_port_end: 18000,
            local_ip: "127.0.0.1".parse().unwrap(),
            bind_addr: "127.0.0.1:15070".parse().unwrap(),
            state_table_path: None,
        };
        let coordinator = UnifiedCoordinator::new(config).await.unwrap();
        let session_manager = coordinator.session_manager().await.unwrap();

        let session_id = session_manager.create_session(
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
            CallDirection::Outgoing,
        ).await.unwrap();

        session_manager.update_session_state(&session_id, CallState::Ringing).await.unwrap();

        let metadata = session_manager.get_session(&session_id).await.unwrap();
        assert_eq!(metadata.call_state, CallState::Ringing);
    }

    #[tokio::test]
    async fn test_session_lifecycle() {
        let config = Config {
            sip_port: 15071,
            media_port_start: 18000,
            media_port_end: 19000,
            local_ip: "127.0.0.1".parse().unwrap(),
            bind_addr: "127.0.0.1:15071".parse().unwrap(),
            state_table_path: None,
        };
        let coordinator = UnifiedCoordinator::new(config).await.unwrap();
        let session_manager = coordinator.session_manager().await.unwrap();

        // Create session
        let session_id = session_manager.create_session(
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
            CallDirection::Outgoing,
        ).await.unwrap();

        // Progress through states
        session_manager.update_session_state(&session_id, CallState::Ringing).await.unwrap();
        session_manager.update_session_state(&session_id, CallState::Active).await.unwrap();

        // Verify active state
        let metadata = session_manager.get_session(&session_id).await.unwrap();
        assert!(metadata.started_at.is_some());

        // Terminate
        session_manager.terminate_session(&session_id, Some("Normal hangup".to_string())).await.unwrap();

        // Verify session is gone
        assert!(session_manager.get_session(&session_id).await.is_none());
    }
}