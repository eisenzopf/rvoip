//! Helper methods for common state machine operations
//! 
//! These methods provide convenience functions that can't be done through
//! simple message passing. They handle:
//! - Session creation and initialization
//! - State queries and session info
//! - Subscription management
//! - Complex operations that need multiple coordinated steps

use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use crate::{
    types::{SessionId, SessionInfo, CallState},
    state_table::types::{Role, EventType},
    errors::{Result, SessionError},
};
use super::StateMachine;

/// Extended state machine with helper methods
pub struct StateMachineHelpers {
    /// Core state machine
    pub state_machine: Arc<StateMachine>,
    
    /// Active session tracking (for queries)
    active_sessions: Arc<RwLock<HashMap<SessionId, SessionInfo>>>,
    
    /// Event subscribers
    subscribers: Arc<RwLock<HashMap<SessionId, Vec<Box<dyn Fn(SessionEvent) + Send + Sync>>>>>,
}

/// Events for subscribers
#[derive(Debug, Clone)]
pub enum SessionEvent {
    StateChanged { from: CallState, to: CallState },
    CallEstablished,
    CallTerminated { reason: String },
    MediaReady,
    IncomingCall { from: String },
}

impl StateMachineHelpers {
    pub fn new(state_machine: Arc<StateMachine>) -> Self {
        Self {
            state_machine,
            active_sessions: Arc::new(RwLock::new(HashMap::new())),
            subscribers: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    // ========== Session Creation ==========
    // These can't be done through message passing alone
    
    /// Create and initialize a new session
    pub async fn create_session(
        &self,
        session_id: SessionId,
        from: String,
        to: String,
        role: Role,
    ) -> Result<()> {
        // Create session in the store
        let session = self.state_machine.store.create_session(
            session_id.clone(),
            role,
            true, // with history
        ).await?;
        
        // Set initial data
        let mut session = session;
        session.local_uri = Some(from.clone());
        session.remote_uri = Some(to.clone());
        
        // Store it
        self.state_machine.store.update_session(session).await?;
        
        // Track in active sessions
        let info = SessionInfo {
            session_id: session_id.clone(),
            from,
            to,
            state: CallState::Idle,
            start_time: std::time::SystemTime::now(),
            media_active: false,
        };
        self.active_sessions.write().await.insert(session_id, info);
        
        Ok(())
    }
    
    // ========== Convenience Methods ==========
    // High-level operations that coordinate multiple events
    
    /// Make an outgoing call (creates session + sends MakeCall event)
    pub async fn make_call(&self, from: &str, to: &str) -> Result<SessionId> {
        let session_id = SessionId::new();
        
        // Create session
        self.create_session(
            session_id.clone(),
            from.to_string(),
            to.to_string(),
            Role::UAC,
        ).await?;
        
        // Send MakeCall event
        self.state_machine.process_event(
            &session_id,
            EventType::MakeCall { target: to.to_string() },
        ).await?;
        
        Ok(session_id)
    }
    
    /// Accept an incoming call
    pub async fn accept_call(&self, session_id: &SessionId) -> Result<()> {
        self.state_machine.process_event(
            session_id,
            EventType::AcceptCall,
        ).await?;
        Ok(())
    }
    
    /// Reject an incoming call
    pub async fn reject_call(&self, session_id: &SessionId, reason: &str) -> Result<()> {
        self.state_machine.process_event(
            session_id,
            EventType::RejectCall { reason: reason.to_string() },
        ).await?;
        Ok(())
    }
    
    /// Hangup a call
    pub async fn hangup(&self, session_id: &SessionId) -> Result<()> {
        self.state_machine.process_event(
            session_id,
            EventType::HangupCall,
        ).await?;
        Ok(())
    }
    
    /// Create a conference from an active call
    pub async fn create_conference(&self, session_id: &SessionId, name: &str) -> Result<()> {
        self.state_machine.process_event(
            session_id,
            EventType::CreateConference { name: name.to_string() },
        ).await?;
        Ok(())
    }
    
    /// Add a participant to a conference
    pub async fn add_to_conference(
        &self,
        host_session_id: &SessionId,
        participant_session_id: &SessionId,
    ) -> Result<()> {
        self.state_machine.process_event(
            host_session_id,
            EventType::AddParticipant { 
                session_id: participant_session_id.to_string() 
            },
        ).await?;
        Ok(())
    }
    
    // ========== Query Methods ==========
    // These need access to internal state
    
    /// Get session information
    pub async fn get_session_info(&self, session_id: &SessionId) -> Result<SessionInfo> {
        self.active_sessions.read().await
            .get(session_id)
            .cloned()
            .ok_or_else(|| SessionError::SessionNotFound(session_id.to_string()))
    }
    
    /// List all active sessions
    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        self.active_sessions.read().await
            .values()
            .cloned()
            .collect()
    }
    
    /// Get current state of a session
    pub async fn get_state(&self, session_id: &SessionId) -> Result<CallState> {
        let session = self.state_machine.store.get_session(session_id).await?;
        Ok(session.call_state)
    }
    
    /// Check if a session is in conference
    pub async fn is_in_conference(&self, session_id: &SessionId) -> Result<bool> {
        let state = self.get_state(session_id).await?;
        Ok(matches!(state, CallState::InConference | CallState::ConferenceHost))
    }
    
    // ========== Subscription Management ==========
    // Can't be done through message passing
    
    /// Subscribe to events for a session
    pub async fn subscribe<F>(&self, session_id: SessionId, callback: F)
    where
        F: Fn(SessionEvent) + Send + Sync + 'static,
    {
        self.subscribers.write().await
            .entry(session_id)
            .or_insert_with(Vec::new)
            .push(Box::new(callback));
    }
    
    /// Unsubscribe from a session
    pub async fn unsubscribe(&self, session_id: &SessionId) {
        self.subscribers.write().await.remove(session_id);
    }
    
    // ========== Internal Helpers ==========
    
    /// Notify subscribers of an event
    pub(crate) async fn notify_subscribers(&self, session_id: &SessionId, event: SessionEvent) {
        if let Some(callbacks) = self.subscribers.read().await.get(session_id) {
            for callback in callbacks {
                callback(event.clone());
            }
        }
    }
    
    /// Clean up terminated session
    pub(crate) async fn cleanup_session(&self, session_id: &SessionId) {
        self.active_sessions.write().await.remove(session_id);
        self.subscribers.write().await.remove(session_id);
    }
}

// ========== Things that CAN'T be done through message passing ==========
// 
// 1. Session Creation - Need to allocate storage, set initial state
// 2. State Queries - Need direct access to session store
// 3. Listing Sessions - Need to enumerate all active sessions
// 4. Subscriptions - Need to maintain callback registry
// 5. Complex Coordinated Operations - Like creating a conference which needs
//    to track multiple sessions together
// 6. Resource Cleanup - Need to clean up multiple data structures
// 7. Session History - Need to access and query transition history
// 8. Performance Metrics - Need to collect timing data across components
//
// Everything else (call control, media operations, etc.) is done through
// the state machine by sending events and executing actions.
