use std::time::Duration;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, trace};

use crate::transaction::timer::{TimerSettings, TimerManager, TimerFactory, Timer, TimerType};
use crate::transaction::{TransactionKey, TransactionState, InternalTransactionCommand};

/// Helper module for transaction-specific timer operations using the core timer infrastructure.
/// This provides a simpler interface for transaction implementations to start/stop timers.

/// Starts a timer for a transaction using the timer manager
/// 
/// # Arguments
/// * `timer_manager` - Manager to handle timer execution
/// * `tx_id` - Transaction ID (for logging and identification)
/// * `timer_name` - The name of the timer for events (e.g., "E", "F", "K")
/// * `timer_type` - Type of timer (e.g., TimerType::E, TimerType::F)
/// * `interval` - Duration for the timer
/// * `cmd_tx` - Channel to send commands when the timer fires
/// 
/// # Returns
/// A JoinHandle for the timer task
pub async fn start_transaction_timer(
    timer_manager: &TimerManager,
    tx_id: &TransactionKey,
    timer_name: &str,
    timer_type: TimerType,
    interval: Duration,
    cmd_tx: mpsc::Sender<InternalTransactionCommand>
) -> Result<JoinHandle<()>, crate::transaction::error::Error> {
    // Register the transaction if not already done
    timer_manager.register_transaction(tx_id.clone(), cmd_tx).await;
    
    // Start the timer with the manager
    let handle = timer_manager.start_timer(
        tx_id.clone(),
        timer_type,
        interval,
    ).await?;
    
    trace!(id=%tx_id, timer=%timer_name, interval=?interval, "Started transaction timer");
    Ok(handle)
}

/// Starts a timer with a transition to a specified state when it expires
/// 
/// # Arguments
/// * `timer_manager` - Manager to handle timer execution
/// * `tx_id` - Transaction ID (for logging and identification)
/// * `timer_name` - Name of the timer for events (e.g., "K", "D")
/// * `timer_type` - Type of timer (e.g., TimerType::K, TimerType::D)
/// * `interval` - Duration for the timer
/// * `cmd_tx` - Channel to send commands when the timer fires
/// * `target_state` - State to transition to when the timer fires
/// 
/// # Returns
/// A JoinHandle for the timer task
pub async fn start_timer_with_transition(
    timer_manager: &TimerManager,
    tx_id: &TransactionKey,
    timer_name: &str,
    timer_type: TimerType,
    interval: Duration,
    cmd_tx: mpsc::Sender<InternalTransactionCommand>,
    target_state: TransactionState
) -> Result<JoinHandle<()>, crate::transaction::error::Error> {
    // Clone values for transition - this fixes the 'borrowed data escapes function' error
    let tx_id_clone = tx_id.clone();
    let cmd_tx_clone = cmd_tx.clone();
    let state = target_state;
    let timer_name_clone = timer_name.to_string(); // Clone the timer name to avoid borrowing issues
    
    // Register the transaction if not already done
    timer_manager.register_transaction(tx_id_clone.clone(), cmd_tx_clone.clone()).await;
    
    // We need to create a custom timer handler that will send both Timer and TransitionTo commands
    let handle = tokio::spawn(async move {
        // Sleep for the interval
        tokio::time::sleep(interval).await;
        
        // Then send both commands
        debug!(id=%tx_id_clone, timer=%timer_name_clone, "Timer fired with transition to {:?}", state);
        
        // First send the timer event
        let _ = cmd_tx_clone.send(InternalTransactionCommand::Timer(timer_name_clone.clone())).await;
        
        // Then send the transition command
        let _ = cmd_tx_clone.send(InternalTransactionCommand::TransitionTo(state)).await;
    });
    
    trace!(id=%tx_id, timer=%timer_name, interval=?interval, target_state=?target_state, "Started timer with transition");
    Ok(handle)
}

/// Unregisters a transaction from the timer manager
/// 
/// # Arguments
/// * `timer_manager` - Manager to handle timer unregistration
/// * `tx_id` - Transaction ID to unregister
pub async fn unregister_transaction(
    timer_manager: &TimerManager,
    tx_id: &TransactionKey
) {
    timer_manager.unregister_transaction(tx_id).await;
    trace!(id=%tx_id, "Unregistered transaction from timer manager");
}

/// Helper that creates a proper backoff interval for retransmission timers
/// 
/// # Arguments
/// * `current_interval` - Current timer interval
/// * `settings` - Timer settings containing T1 and T2 values
/// 
/// # Returns
/// The next interval to use for retransmission (doubles until reaching T2)
pub fn calculate_backoff_interval(
    current_interval: Duration,
    settings: &TimerSettings
) -> Duration {
    std::cmp::min(current_interval * 2, settings.t2)
} 