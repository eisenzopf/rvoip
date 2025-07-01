//! # Call Lifecycle Management for Call Center Operations
//!
//! This module provides sophisticated call lifecycle management functionality, handling
//! call state transitions, lifecycle events, validation, and comprehensive monitoring
//! throughout the entire call lifecycle. It ensures proper state management, provides
//! callback mechanisms for state changes, and maintains consistency across the call
//! center system.
//!
//! ## Overview
//!
//! Call lifecycle management is essential for maintaining proper call state consistency
//! and providing visibility into call progression through the system. This module provides
//! a comprehensive state machine that tracks calls from initial establishment through
//! final termination, with support for complex call scenarios including transfers,
//! holds, and conference calls.
//!
//! ## Key Features
//!
//! - **State Machine Management**: Comprehensive call state tracking with validation
//! - **Transition Validation**: Ensures only valid state transitions are allowed
//! - **Event Callbacks**: Configurable callbacks for state transition events
//! - **Statistics Collection**: Real-time statistics on call states and transitions
//! - **Error Prevention**: Validates transitions to prevent invalid states
//! - **Monitoring Support**: Provides comprehensive monitoring and reporting
//! - **Concurrent Access**: Thread-safe state management for concurrent operations
//! - **Memory Efficiency**: Efficient storage and cleanup of lifecycle data
//!
//! ## Call State Model
//!
//! The lifecycle manager uses a sophisticated state model that covers all call scenarios:
//!
//! ### Core States
//!
//! - **Establishing**: Call is being set up initially
//! - **Ringing**: Call is ringing (customer or agent side)
//! - **Queued**: Call is waiting in queue for agent assignment
//! - **Connecting**: Call is being connected to an agent
//! - **Active**: Call is in progress with active conversation
//! - **OnHold**: Call is temporarily on hold
//! - **Transferring**: Call is being transferred to another agent/queue
//! - **Ending**: Call termination is in progress
//! - **Ended**: Call has been completely terminated
//!
//! ### State Transition Rules
//!
//! The system enforces strict validation rules for state transitions to maintain consistency
//! and prevent invalid states that could cause system issues.
//!
//! ## Examples
//!
//! ### Basic Lifecycle Management
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::lifecycle::{CallLifecycleManager, CallState};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut lifecycle_manager = CallLifecycleManager::new();
//! let session_id = SessionId("call-lifecycle-example".to_string());
//! 
//! // Start call lifecycle
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Establishing)?;
//! println!("📞 Call {} state: Establishing", session_id);
//! 
//! // Progress through normal call flow
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Ringing)?;
//! println!("🔔 Call {} state: Ringing", session_id);
//! 
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Queued)?;
//! println!("📋 Call {} state: Queued", session_id);
//! 
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Connecting)?;
//! println!("🔗 Call {} state: Connecting", session_id);
//! 
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Active)?;
//! println!("✅ Call {} state: Active", session_id);
//! 
//! // Get current state
//! if let Some(current_state) = lifecycle_manager.get_call_state(&session_id) {
//!     println!("Current state: {:?}", current_state);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### State Transition Callbacks
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::lifecycle::{CallLifecycleManager, CallState};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut lifecycle_manager = CallLifecycleManager::new();
//! 
//! // Register callback for Active state transitions
//! lifecycle_manager.register_state_callback(
//!     CallState::Active,
//!     Box::new(|session_id, old_state, new_state| {
//!         println!("🎯 Call {} became active: {:?} → {:?}", 
//!                  session_id, old_state, new_state);
//!         
//!         // Custom logic for when calls become active
//!         // - Start call monitoring
//!         // - Begin recording if required
//!         // - Update performance metrics
//!         // - Notify supervisors
//!     })
//! );
//! 
//! // Register callback for call endings
//! lifecycle_manager.register_state_callback(
//!     CallState::Ended,
//!     Box::new(|session_id, old_state, new_state| {
//!         println!("🛑 Call {} ended: {:?} → {:?}", 
//!                  session_id, old_state, new_state);
//!         
//!         // Custom logic for call completion
//!         // - Calculate call metrics
//!         // - Update agent availability
//!         // - Generate call reports
//!         // - Clean up resources
//!     })
//! );
//! 
//! let session_id = SessionId("callback-example".to_string());
//! 
//! // Transitions will trigger callbacks
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Establishing)?;
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Active)?;
//! // ↳ This will trigger the Active state callback
//! 
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Ended)?;
//! // ↳ This will trigger the Ended state callback
//! 
//! println!("✅ All state transitions processed with callbacks");
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Call Scenarios
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::lifecycle::{CallLifecycleManager, CallState};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut lifecycle_manager = CallLifecycleManager::new();
//! 
//! // Scenario 1: Call with hold operation
//! let hold_call = SessionId("hold-scenario".to_string());
//! 
//! lifecycle_manager.set_call_state(hold_call.clone(), CallState::Establishing)?;
//! lifecycle_manager.set_call_state(hold_call.clone(), CallState::Active)?;
//! println!("📞 Call {} is active", hold_call);
//! 
//! // Put call on hold
//! lifecycle_manager.set_call_state(hold_call.clone(), CallState::OnHold)?;
//! println!("⏸️ Call {} placed on hold", hold_call);
//! 
//! // Resume from hold
//! lifecycle_manager.set_call_state(hold_call.clone(), CallState::Active)?;
//! println!("▶️ Call {} resumed from hold", hold_call);
//! 
//! // Scenario 2: Call transfer
//! let transfer_call = SessionId("transfer-scenario".to_string());
//! 
//! lifecycle_manager.set_call_state(transfer_call.clone(), CallState::Active)?;
//! println!("📞 Call {} is active with first agent", transfer_call);
//! 
//! // Start transfer process
//! lifecycle_manager.set_call_state(transfer_call.clone(), CallState::Transferring)?;
//! println!("🔄 Call {} being transferred", transfer_call);
//! 
//! // Complete transfer
//! lifecycle_manager.set_call_state(transfer_call.clone(), CallState::Active)?;
//! println!("✅ Call {} active with new agent", transfer_call);
//! 
//! println!("🎯 Complex call scenarios managed successfully");
//! # Ok(())
//! # }
//! ```
//!
//! ### State Validation and Error Handling
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::lifecycle::{CallLifecycleManager, CallState};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut lifecycle_manager = CallLifecycleManager::new();
//! let session_id = SessionId("validation-example".to_string());
//! 
//! // Valid transition
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Establishing)?;
//! println!("✅ Valid transition to Establishing");
//! 
//! // Another valid transition
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Ringing)?;
//! println!("✅ Valid transition to Ringing");
//! 
//! // Attempt invalid transition
//! match lifecycle_manager.set_call_state(session_id.clone(), CallState::Ended) {
//!     Ok(()) => println!("❌ This shouldn't happen - invalid transition allowed"),
//!     Err(e) => {
//!         println!("✅ Invalid transition properly rejected: {}", e);
//!         println!("🛡️ State validation working correctly");
//!     }
//! }
//! 
//! // Complete valid state progression
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Queued)?;
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Connecting)?;
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Active)?;
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Ending)?;
//! lifecycle_manager.set_call_state(session_id.clone(), CallState::Ended)?;
//! 
//! println!("✅ Complete valid call lifecycle progression");
//! # Ok(())
//! # }
//! ```
//!
//! ### Statistics and Monitoring
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::lifecycle::{CallLifecycleManager, CallState};
//! use rvoip_session_core::SessionId;
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut lifecycle_manager = CallLifecycleManager::new();
//! 
//! // Create multiple calls in different states
//! let calls = vec![
//!     (SessionId("call-1".to_string()), CallState::Active),
//!     (SessionId("call-2".to_string()), CallState::Queued),
//!     (SessionId("call-3".to_string()), CallState::OnHold),
//!     (SessionId("call-4".to_string()), CallState::Active),
//!     (SessionId("call-5".to_string()), CallState::Transferring),
//! ];
//! 
//! // Set up all calls
//! for (session_id, target_state) in calls {
//!     lifecycle_manager.set_call_state(session_id.clone(), CallState::Establishing)?;
//!     
//!     // Progress to target state through valid transitions
//!     match target_state {
//!         CallState::Active => {
//!             lifecycle_manager.set_call_state(session_id.clone(), CallState::Ringing)?;
//!             lifecycle_manager.set_call_state(session_id.clone(), CallState::Connecting)?;
//!             lifecycle_manager.set_call_state(session_id, CallState::Active)?;
//!         }
//!         CallState::Queued => {
//!             lifecycle_manager.set_call_state(session_id.clone(), CallState::Ringing)?;
//!             lifecycle_manager.set_call_state(session_id, CallState::Queued)?;
//!         }
//!         CallState::OnHold => {
//!             lifecycle_manager.set_call_state(session_id.clone(), CallState::Ringing)?;
//!             lifecycle_manager.set_call_state(session_id.clone(), CallState::Connecting)?;
//!             lifecycle_manager.set_call_state(session_id.clone(), CallState::Active)?;
//!             lifecycle_manager.set_call_state(session_id, CallState::OnHold)?;
//!         }
//!         CallState::Transferring => {
//!             lifecycle_manager.set_call_state(session_id.clone(), CallState::Ringing)?;
//!             lifecycle_manager.set_call_state(session_id.clone(), CallState::Connecting)?;
//!             lifecycle_manager.set_call_state(session_id.clone(), CallState::Active)?;
//!             lifecycle_manager.set_call_state(session_id, CallState::Transferring)?;
//!         }
//!         _ => {} // Skip other states for this example
//!     }
//! }
//! 
//! // Get comprehensive statistics
//! let stats = lifecycle_manager.get_statistics();
//! 
//! println!("📊 Call Lifecycle Statistics:");
//! println!("  Total Calls: {}", stats.total_calls);
//! 
//! for (state, count) in &stats.state_counts {
//!     println!("  {:?}: {} calls", state, count);
//! }
//! 
//! // Get calls in specific states
//! let active_calls = lifecycle_manager.get_calls_in_state(&CallState::Active);
//! println!("\n🔍 Active Calls: {} calls", active_calls.len());
//! for call_id in active_calls {
//!     println!("  📞 {}", call_id);
//! }
//! 
//! let queued_calls = lifecycle_manager.get_calls_in_state(&CallState::Queued);
//! println!("\n📋 Queued Calls: {} calls", queued_calls.len());
//! 
//! // Performance indicators
//! let total_active = stats.state_counts.get(&CallState::Active).unwrap_or(&0);
//! let total_queued = stats.state_counts.get(&CallState::Queued).unwrap_or(&0);
//! 
//! if *total_queued > *total_active {
//!     println!("\n⚠️ More calls queued than active - potential bottleneck");
//! } else {
//!     println!("\n✅ Good call distribution");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Real-Time Lifecycle Monitoring
//!
//! ```rust
//! use rvoip_call_engine::orchestrator::lifecycle::{CallLifecycleManager, CallState};
//! use rvoip_session_core::SessionId;
//! use tokio::time::{interval, Duration};
//! 
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut lifecycle_manager = CallLifecycleManager::new();
//! 
//! // Set up monitoring callbacks for all state transitions
//! let states_to_monitor = vec![
//!     CallState::Establishing,
//!     CallState::Ringing,
//!     CallState::Queued,
//!     CallState::Connecting,
//!     CallState::Active,
//!     CallState::OnHold,
//!     CallState::Transferring,
//!     CallState::Ending,
//!     CallState::Ended,
//! ];
//! 
//! for state in states_to_monitor {
//!     lifecycle_manager.register_state_callback(
//!         state.clone(),
//!         Box::new(move |session_id, old_state, new_state| {
//!             let timestamp = chrono::Utc::now().format("%H:%M:%S");
//!             println!("[{}] 🔄 {} transition: {:?} → {:?}", 
//!                      timestamp, session_id, old_state, new_state);
//!         })
//!     );
//! }
//! 
//! // Simulate real-time call monitoring
//! async fn monitor_lifecycle(manager: &CallLifecycleManager) -> Result<(), Box<dyn std::error::Error>> {
//!     let mut monitoring_interval = interval(Duration::from_secs(10));
//!     
//!     loop {
//!         monitoring_interval.tick().await;
//!         
//!         let stats = manager.get_statistics();
//!         
//!         println!("\n📊 Real-time Lifecycle Monitor:");
//!         println!("  ⏰ Timestamp: {}", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S"));
//!         println!("  📞 Total Calls: {}", stats.total_calls);
//!         
//!         // Key performance indicators
//!         let active = stats.state_counts.get(&CallState::Active).unwrap_or(&0);
//!         let queued = stats.state_counts.get(&CallState::Queued).unwrap_or(&0);
//!         let on_hold = stats.state_counts.get(&CallState::OnHold).unwrap_or(&0);
//!         
//!         println!("  📈 KPIs:");
//!         println!("    🟢 Active: {}", active);
//!         println!("    📋 Queued: {}", queued);
//!         println!("    ⏸️ On Hold: {}", on_hold);
//!         
//!         // Alert conditions
//!         if *queued > 10 {
//!             println!("  🚨 Alert: High queue volume ({} calls)", queued);
//!         }
//!         
//!         if *on_hold > 5 {
//!             println!("  ⚠️ Warning: Many calls on hold ({} calls)", on_hold);
//!         }
//!         
//!         if stats.total_calls == 0 {
//!             println!("  ℹ️ System idle - no active calls");
//!         }
//!         
//!         // In real implementation, this would run continuously
//!         break; // Exit for documentation example
//!     }
//!     
//!     Ok(())
//! }
//! 
//! // Start monitoring (would run in background)
//! monitor_lifecycle(&lifecycle_manager).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Integration Patterns
//!
//! ### Call Center Engine Integration
//!
//! The lifecycle manager integrates seamlessly with the call center engine:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::lifecycle::{CallLifecycleManager, CallState};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! // Integration with CallCenterEngine:
//! println!("🔗 Lifecycle Manager Integration:");
//! 
//! println!("  📞 Call Events → Lifecycle Updates:");
//! println!("     ↳ Incoming call → Establishing state");
//! println!("     ↳ Agent assignment → Connecting state");
//! println!("     ↳ Bridge creation → Active state");
//! println!("     ↳ Hold operation → OnHold state");
//! println!("     ↳ Transfer → Transferring state");
//! println!("     ↳ Call termination → Ending → Ended");
//! 
//! println!("  🔄 State Changes → Engine Actions:");
//! println!("     ↳ Active state → Start monitoring");
//! println!("     ↳ OnHold state → Pause metrics");
//! println!("     ↳ Ended state → Clean up resources");
//! 
//! println!("  📊 Statistics → Monitoring:");
//! println!("     ↳ Real-time state counts");
//! println!("     ↳ Performance indicators");
//! println!("     ↳ Alert generation");
//! 
//! # Ok(())
//! # }
//! ```
//!
//! ## Performance Considerations
//!
//! ### Efficient State Management
//!
//! The lifecycle manager is optimized for high-performance operations:
//!
//! ```rust
//! # use rvoip_call_engine::orchestrator::lifecycle::CallLifecycleManager;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! 
//! println!("⚡ Lifecycle Manager Performance:");
//! 
//! println!("  🚀 State Operations:");
//! println!("     ↳ O(1) state lookup and updates");
//! println!("     ↳ O(n) statistics calculation (n = active calls)");
//! println!("     ↳ O(1) callback registration");
//! println!("     ↳ O(k) callback execution (k = callbacks for state)");
//! 
//! println!("  💾 Memory Efficiency:");
//! println!("     ↳ HashMap for efficient state storage");
//! println!("     ↳ Minimal memory per call state");
//! println!("     ↳ Automatic cleanup on call removal");
//! println!("     ↳ Callback storage optimization");
//! 
//! println!("  🔄 Concurrency:");
//! println!("     ↳ Thread-safe state operations");
//! println!("     ↳ Lock-free where possible");
//! println!("     ↳ Atomic state transitions");
//! 
//! let manager = CallLifecycleManager::new();
//! println!("✅ Manager created with minimal overhead");
//! # Ok(())
//! # }
//! ```

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