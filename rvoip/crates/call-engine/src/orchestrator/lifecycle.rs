use std::collections::HashMap;
use tracing::{info, debug, warn};

use rvoip_session_core::SessionId;

use crate::error::{CallCenterError, Result};

/// Call lifecycle manager
/// 
/// Manages call state transitions and lifecycle events
pub struct CallLifecycleManager {
    /// Call state tracking
    call_states: HashMap<SessionId, CallState>,
    
    /// State transition callbacks
    state_callbacks: HashMap<CallState, Vec<StateCallback>>,
}

/// Call state enumeration
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CallState {
    /// Call is being established
    Establishing,
    
    /// Call is ringing
    Ringing,
    
    /// Call is queued waiting for agent
    Queued,
    
    /// Call is being connected to agent
    Connecting,
    
    /// Call is active (agent and customer connected)
    Active,
    
    /// Call is on hold
    OnHold,
    
    /// Call is being transferred
    Transferring,
    
    /// Call is ending
    Ending,
    
    /// Call has ended
    Ended,
}

/// State transition callback type
pub type StateCallback = Box<dyn Fn(&SessionId, &CallState, &CallState) + Send + Sync>;

impl CallLifecycleManager {
    /// Create a new call lifecycle manager
    pub fn new() -> Self {
        Self {
            call_states: HashMap::new(),
            state_callbacks: HashMap::new(),
        }
    }
    
    /// Set the state of a call
    pub fn set_call_state(&mut self, session_id: SessionId, new_state: CallState) -> Result<()> {
        let old_state = self.call_states.get(&session_id).cloned();
        
        info!("🔄 Call {} state: {:?} → {:?}", session_id, old_state, new_state);
        
        // Validate state transition
        if let Some(ref old) = old_state {
            if !self.is_valid_transition(old, &new_state) {
                return Err(CallCenterError::orchestration(
                    format!("Invalid state transition from {:?} to {:?}", old, new_state)
                ));
            }
        }
        
        // Update state
        self.call_states.insert(session_id.clone(), new_state.clone());
        
        // Execute callbacks
        if let Some(callbacks) = self.state_callbacks.get(&new_state) {
            for callback in callbacks {
                if let Some(ref old) = old_state {
                    callback(&session_id, old, &new_state);
                } else {
                    callback(&session_id, &CallState::Establishing, &new_state);
                }
            }
        }
        
        Ok(())
    }
    
    /// Get the current state of a call
    pub fn get_call_state(&self, session_id: &SessionId) -> Option<&CallState> {
        self.call_states.get(session_id)
    }
    
    /// Register a callback for state transitions
    pub fn register_state_callback(&mut self, state: CallState, callback: StateCallback) {
        self.state_callbacks.entry(state).or_insert_with(Vec::new).push(callback);
    }
    
    /// Remove a call from tracking
    pub fn remove_call(&mut self, session_id: &SessionId) {
        if let Some(state) = self.call_states.remove(session_id) {
            debug!("🗑️ Removed call {} from lifecycle tracking (final state: {:?})", session_id, state);
        }
    }
    
    /// Get all calls in a specific state
    pub fn get_calls_in_state(&self, state: &CallState) -> Vec<SessionId> {
        self.call_states.iter()
            .filter(|(_, s)| *s == state)
            .map(|(id, _)| id.clone())
            .collect()
    }
    
    /// Get lifecycle statistics
    pub fn get_statistics(&self) -> LifecycleStats {
        let mut state_counts = HashMap::new();
        
        for state in self.call_states.values() {
            *state_counts.entry(state.clone()).or_insert(0) += 1;
        }
        
        LifecycleStats {
            total_calls: self.call_states.len(),
            state_counts,
        }
    }
    
    /// Check if a state transition is valid
    fn is_valid_transition(&self, from: &CallState, to: &CallState) -> bool {
        use CallState::*;
        
        match (from, to) {
            // From Establishing
            (Establishing, Ringing) => true,
            (Establishing, Queued) => true,
            (Establishing, Ending) => true,
            
            // From Ringing
            (Ringing, Connecting) => true,
            (Ringing, Queued) => true,
            (Ringing, Ending) => true,
            
            // From Queued
            (Queued, Connecting) => true,
            (Queued, Ending) => true,
            
            // From Connecting
            (Connecting, Active) => true,
            (Connecting, Ending) => true,
            
            // From Active
            (Active, OnHold) => true,
            (Active, Transferring) => true,
            (Active, Ending) => true,
            
            // From OnHold
            (OnHold, Active) => true,
            (OnHold, Ending) => true,
            
            // From Transferring
            (Transferring, Active) => true,
            (Transferring, Ending) => true,
            
            // From Ending
            (Ending, Ended) => true,
            
            // No transitions from Ended
            (Ended, _) => false,
            
            // All other transitions are invalid
            _ => false,
        }
    }
}

impl Default for CallLifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Lifecycle statistics
#[derive(Debug, Clone)]
pub struct LifecycleStats {
    pub total_calls: usize,
    pub state_counts: HashMap<CallState, usize>,
} 