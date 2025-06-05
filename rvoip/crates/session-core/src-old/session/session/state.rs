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
        let old_state = *state_guard;
        
        // Validate state transition using the centralized state machine
        if !old_state.can_transition_to(new_state) {
            return Err(Error::InvalidSessionStateTransition {
                from: old_state.to_string(),
                to: new_state.to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Error,
                    recovery: RecoveryAction::CheckConfiguration("session_state_transition".to_string()),
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!(
                        "Invalid state transition attempted from {} to {}. Valid transitions from {}: {:?}",
                        old_state, new_state, old_state, old_state.valid_next_states()
                    )),
                    ..Default::default()
                }
            });
        }
        
        // Update state and emit event
        *state_guard = new_state;
        
        // Drop lock before emitting event
        drop(state_guard);
        
        // Log state transition for debugging
        debug!("Session {} state transition: {} → {}", 
            self.id, old_state, new_state);
        
        // Emit state changed event
        if let Err(e) = self.event_bus.publish(SessionEvent::StateChanged { 
            session_id: self.id.clone(),
            old_state,
            new_state,
        }).await {
            warn!("Failed to publish session state change event: {}", e);
        }
        
        Ok(())
    }
    
    /// Get the current media state
    pub async fn media_state(&self) -> SessionMediaState {
        self.media_state.lock().await.clone()
    }
    
    /// Validate if a state transition would be valid without changing state
    /// 
    /// This method allows checking state transitions without actually performing them,
    /// useful for UI validation and planning ahead.
    /// 
    /// # Arguments
    /// 
    /// * `target_state` - The state to validate transition to
    /// 
    /// # Returns
    /// 
    /// `Ok(())` if transition is valid, `Err` with details if invalid
    pub async fn validate_state_transition(&self, target_state: SessionState) -> Result<(), Error> {
        let current_state = *self.state.lock().await;
        
        if !current_state.can_transition_to(target_state) {
            return Err(Error::InvalidSessionStateTransition {
                from: current_state.to_string(),
                to: target_state.to_string(),
                context: ErrorContext {
                    category: ErrorCategory::Session,
                    severity: ErrorSeverity::Warning,
                    recovery: RecoveryAction::CheckConfiguration("session_state_transition".to_string()),
                    retryable: false,
                    session_id: Some(self.id.to_string()),
                    timestamp: SystemTime::now(),
                    details: Some(format!(
                        "State transition validation failed from {} to {}. Valid options: {:?}",
                        current_state, target_state, current_state.valid_next_states()
                    )),
                    ..Default::default()
                }
            });
        }
        
        Ok(())
    }
    
    /// Get all valid next states for this session
    /// 
    /// Returns the states this session can transition to from its current state.
    /// Useful for UI state management and API validation.
    /// 
    /// # Returns
    /// 
    /// Vector of valid next states
    pub async fn get_valid_next_states(&self) -> Vec<SessionState> {
        let current_state = *self.state.lock().await;
        current_state.valid_next_states()
    }
} 