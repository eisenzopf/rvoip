// Transaction testing utilities to help with state tracking and validation
//
// This module provides utilities to track transaction state changes,
// accelerate timers, and validate proper SIP transaction state flows
// according to RFC 3261.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;

use rvoip_transaction_core::{
    TransactionEvent, TransactionKey, TransactionManager, TransactionState
};

/// Structure to track all state changes for a transaction
#[derive(Default, Debug)]
pub struct StateTracker {
    states: Mutex<HashMap<String, Vec<TransactionState>>>,
}

impl StateTracker {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            states: Mutex::new(HashMap::new()),
        })
    }

    pub fn record_state(&self, tx_id: &str, state: TransactionState) {
        let mut states = self.states.lock().unwrap();
        states.entry(tx_id.to_string())
            .or_insert_with(Vec::new)
            .push(state);
    }
    
    pub fn get_states(&self, tx_id: &str) -> Vec<TransactionState> {
        let states = self.states.lock().unwrap();
        states.get(tx_id)
            .cloned()
            .unwrap_or_default()
    }
    
    pub fn last_state(&self, tx_id: &str) -> Option<TransactionState> {
        let states = self.states.lock().unwrap();
        states.get(tx_id)
            .and_then(|s| s.last().cloned())
    }
}

/// Async function to wait for and validate a sequence of transaction states
pub async fn assert_state_sequence(
    manager: &TransactionManager,
    tracker: &StateTracker,
    tx_id: &TransactionKey,
    expected_states: Vec<TransactionState>,
    timeout_ms: u64,
) -> bool {
    let start = std::time::Instant::now();
    let tx_id_str = tx_id.to_string();
    
    // Keep checking until we've seen all expected states or timeout
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        // Get current state from manager (this will trigger a state update in our tracker)
        if let Ok(current) = manager.transaction_state(&tx_id_str).await {
            // Record the state
            tracker.record_state(&tx_id_str, current);
        }
        
        // Get all recorded states
        let states = tracker.get_states(&tx_id_str);
        
        // Check if all expected states have been seen in sequence
        if states.len() >= expected_states.len() {
            let mut matches = true;
            for (i, expected) in expected_states.iter().enumerate() {
                if i >= states.len() || &states[i] != expected {
                    matches = false;
                    break;
                }
            }
            
            if matches {
                return true;
            }
        }
        
        sleep(Duration::from_millis(10)).await;
    }
    
    // Report failure
    let states = tracker.get_states(&tx_id_str);
    println!("Expected state sequence: {:?}", expected_states);
    println!("Actual state sequence: {:?}", states);
    false
}

/// Process events from a transaction manager and monitor transaction states
pub async fn process_events(
    event_rx: &mut mpsc::Receiver<TransactionEvent>,
    tracker: Arc<StateTracker>,
    manager: &TransactionManager,
    timeout_ms: u64,
) -> Option<TransactionEvent> {
    let start = std::time::Instant::now();
    
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        if let Ok(Some(event)) = tokio::time::timeout(
            Duration::from_millis(100),
            event_rx.recv()
        ).await {
            // Extract transaction_id from event if present
            let tx_id = match &event {
                TransactionEvent::NewRequest { transaction_id, .. } => Some(transaction_id),
                TransactionEvent::AckReceived { transaction_id, .. } => Some(transaction_id),
                TransactionEvent::CancelReceived { transaction_id, .. } => Some(transaction_id),
                TransactionEvent::ProvisionalResponse { transaction_id, .. } => Some(transaction_id),
                TransactionEvent::SuccessResponse { transaction_id, .. } => Some(transaction_id),
                TransactionEvent::FailureResponse { transaction_id, .. } => Some(transaction_id),
                TransactionEvent::ProvisionalResponseSent { transaction_id, .. } => Some(transaction_id),
                TransactionEvent::FinalResponseSent { transaction_id, .. } => Some(transaction_id),
                TransactionEvent::TransactionTimeout { transaction_id } => Some(transaction_id),
                TransactionEvent::AckTimeout { transaction_id } => Some(transaction_id),
                TransactionEvent::TransportError { transaction_id } => Some(transaction_id),
                TransactionEvent::Error { transaction_id: Some(id), .. } => Some(id),
                _ => None,
            };
            
            // If we have a transaction ID, check and record its current state
            if let Some(id) = tx_id {
                if let Ok(current_state) = manager.transaction_state(&id.to_string()).await {
                    tracker.record_state(&id.to_string(), current_state);
                    println!("Transaction {:?} state: {:?}", id, current_state);
                }
            }
            
            return Some(event);
        }
        
        sleep(Duration::from_millis(10)).await;
    }
    
    None
}

/// Helper function to wait for a transaction to reach a specific state
pub async fn wait_for_state(
    manager: &TransactionManager,
    tx_id: &TransactionKey,
    expected_state: TransactionState,
    timeout_ms: u64,
) -> bool {
    let start = std::time::Instant::now();
    let tx_id_str = tx_id.to_string();
    
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        if let Ok(state) = manager.transaction_state(&tx_id_str).await {
            if state == expected_state {
                return true;
            }
        }
        
        sleep(Duration::from_millis(10)).await;
    }
    
    false
}

/// Utility to manually accelerate transaction timers for testing
/// This is a mock function that would need actual implementation in TransactionManager
pub async fn accelerate_transaction_timers(
    manager: &TransactionManager, 
    tx_id: &TransactionKey,
    timer_name: &str,
) -> Result<(), String> {
    // This is a placeholder - actual implementation would trigger specific timers
    // in the transaction manager
    Err("Timer acceleration not implemented in TransactionManager yet".to_string())
}

/// Helper to print transaction state history for debugging
pub fn print_transaction_history(tracker: &StateTracker, tx_id: &str) {
    let states = tracker.get_states(tx_id);
    println!("Transaction {} state history:", tx_id);
    for (i, state) in states.iter().enumerate() {
        println!("  {}: {:?}", i, state);
    }
} 