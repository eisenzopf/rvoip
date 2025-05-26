use std::time::SystemTime;
use tracing::{debug, info, error, warn};

use crate::events::SessionEvent;
use crate::errors::{Error, ErrorCategory, ErrorContext, ErrorSeverity, RecoveryAction};
use super::core::{Session, SessionMediaState};
use super::super::session_types::SessionState;

impl Session {
    /// Set a new session state
    pub async fn set_state(&self, new_state: SessionState) -> Result<(), Error> {
        let mut state_guard = self.state.lock().await;
        let old_state = state_guard.clone();
        
        // Validate state transition
        if !Self::is_valid_transition(&old_state, &new_state) {
            return Err(Error::InvalidSessionStateTransition {
                from: old_state.to_string(),
                to: new_state.to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::None,
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!("Invalid state transition attempted from {} to {}", old_state, new_state)),
                    ..Default::default()
                }
            });
        }
        
        // Update state and emit event
        *state_guard = new_state.clone();
        
        // Drop lock before emitting event
        drop(state_guard);
        
        // Emit state changed event
        self.event_bus.publish(SessionEvent::StateChanged { 
            session_id: self.id.clone(),
            old_state,
            new_state,
        });
        
        Ok(())
    }
    
    /// Check if a state transition is valid
    fn is_valid_transition(from: &SessionState, to: &SessionState) -> bool {
        use SessionState::*;
        
        match (from, to) {
            // Valid transitions from Initializing
            (Initializing, Dialing) => true,
            (Initializing, Ringing) => true,
            (Initializing, Terminating) => true,
            (Initializing, Terminated) => true,
            
            // Valid transitions from Dialing
            (Dialing, Ringing) => true,
            (Dialing, Connected) => true,
            (Dialing, Terminating) => true,
            (Dialing, Terminated) => true,
            
            // Valid transitions from Ringing
            (Ringing, Connected) => true,
            (Ringing, Terminating) => true,
            (Ringing, Terminated) => true,
            
            // Valid transitions from Connected
            (Connected, OnHold) => true,
            (Connected, Transferring) => true,
            (Connected, Terminating) => true,
            (Connected, Terminated) => true,
            
            // Valid transitions from OnHold
            (OnHold, Connected) => true,
            (OnHold, Transferring) => true,
            (OnHold, Terminating) => true,
            (OnHold, Terminated) => true,
            
            // Valid transitions from Transferring
            (Transferring, Connected) => true,
            (Transferring, OnHold) => true,
            (Transferring, Terminating) => true,
            (Transferring, Terminated) => true,
            
            // Valid transitions from Terminating
            (Terminating, Terminated) => true,
            
            // No transitions from Terminated
            (Terminated, _) => false,
            
            // Any other transition is invalid
            _ => false,
        }
    }
    
    /// Get the current media state
    pub async fn media_state(&self) -> SessionMediaState {
        self.media_state.lock().await.clone()
    }
} 