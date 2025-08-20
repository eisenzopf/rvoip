/// # Transaction Runner
///
/// This module provides the core event loop implementation that drives SIP transaction
/// state machines according to RFC 3261 Section 17. It's the "engine" that powers all
/// transaction types by translating events into state transitions.
///
/// ## RFC 3261 Context
///
/// RFC 3261 defines four distinct transaction state machines:
/// - INVITE client transactions (Section 17.1.1)
/// - Non-INVITE client transactions (Section 17.1.2)
/// - INVITE server transactions (Section 17.2.1)
/// - Non-INVITE server transactions (Section 17.2.2)
///
/// While each transaction type has its own specific states and transitions, they all
/// share a common execution pattern:
/// 1. Receive messages or timer events
/// 2. Process these events based on the current state
/// 3. Potentially transition to a new state
/// 4. Start/stop timers as needed for the new state
///
/// ## Implementation Architecture
///
/// This module implements a generic "runner" that can power any of the four transaction
/// types by delegating the transaction-specific logic to implementations of the
/// `TransactionLogic` trait. This separation allows:
///
/// 1. **Code Reuse**: The common event loop logic is implemented once
/// 2. **Type Safety**: Each transaction type can have its own specific data structures
/// 3. **Maintainability**: The state machine implementations are separate from the event loop
///
/// The architecture follows a dependency inversion principle - the runner depends on
/// abstract traits rather than concrete implementations, allowing new transaction types
/// to be added without modifying the runner itself.

use std::sync::Arc;
use std::env;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, trace, warn};

use rvoip_sip_core::Message; // Assuming common Message type
use rvoip_sip_core::types::method::Method; // Import Method for method comparison
use crate::transaction::error::{Error, Result};
use crate::transaction::{
    TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand, AtomicTransactionState,
};
use crate::transaction::state::TransactionLifecycle;
use crate::transaction::logic::TransactionLogic; // The new trait

/// Run the main event loop for a SIP transaction.
///
/// This function implements the core event processing and state machine logic for
/// all SIP transaction types. It receives commands through a channel, processes them
/// according to the transaction's current state, and triggers appropriate state transitions
/// and timer operations.
///
/// ## RFC 3261 Context
///
/// This function implements the runtime machinery required by the transaction layer
/// as defined in RFC 3261 Section 17. It handles:
///
/// - Processing incoming SIP messages (Section 17.1.1.2, 17.1.2.2, 17.2.1, 17.2.2)
/// - Managing transaction state transitions
/// - Handling timer events for retransmissions and timeouts
/// - Reporting events to the Transaction User (TU)
///
/// ## Implementation Details
///
/// The event loop receives commands through `cmd_rx` and uses the provided `logic`
/// implementation to determine how to process them based on the transaction's current state.
/// It manages timer activation/cancellation during state transitions and reports significant
/// events to the TU via the event sender in `data`.
///
/// This generic implementation can run any transaction type, with the type-specific
/// behavior delegated to the `logic` parameter that implements `TransactionLogic`.
///
/// ## Type Parameters
///
/// - `D`: The transaction data type, which must implement various traits for accessing state and channels
/// - `TH`: The timer handles type, which stores JoinHandles for active timers
/// - `L`: The logic implementation type, which must implement TransactionLogic
///
/// ## Arguments
///
/// * `data`: Shared data for the transaction, including state and communication channels
/// * `logic`: Implementation of transaction-specific logic (INVITE client, Non-INVITE server, etc.)
/// * `cmd_rx`: Channel for receiving commands to process
#[allow(clippy::too_many_arguments)] // May have many args initially
pub async fn run_transaction_loop<D, TH, L>(
    data: Arc<D>,
    logic: Arc<L>,
    mut cmd_rx: mpsc::Receiver<InternalTransactionCommand>,
)
where
    D: AsRefState + AsRefKey +
       HasTransactionEvents + HasTransport + HasCommandSender + HasLifecycle + Send + Sync + 'static,
    TH: Default + Send + Sync + 'static,
    L: TransactionLogic<D, TH> + Send + Sync + 'static,
{
    // Check if we're running in test mode
    let is_test_mode = env::var("RVOIP_TEST").map(|v| v == "1").unwrap_or(false);
    
    let mut timer_handles = TH::default();
    let tx_id = data.as_ref_key().clone();

    tracing::trace!("Transaction loop starting for {}", tx_id);
    tracing::trace!("Initial state: {:?}", data.as_ref_state().get());
    debug!(id = %tx_id, test_mode = is_test_mode, "Generic transaction loop starting. Initial state: {:?}", data.as_ref_state().get());

    while let Some(command) = cmd_rx.recv().await {
        let current_state = data.as_ref_state().get();
        let tx_id_clone = data.as_ref_key().clone();

        tracing::trace!("Received command: {:?} for transaction {}", command, tx_id_clone);
        debug!(id=%tx_id_clone, ?command, "Transaction received command");
        
        match command {
            InternalTransactionCommand::TransitionTo(requested_new_state) => {
                tracing::trace!("Processing TransitionTo({:?}) current state: {:?}", requested_new_state, current_state);
                debug!(id=%tx_id_clone, current_state=?current_state, new_state=?requested_new_state, "Processing state transition");
                
                if current_state == requested_new_state {
                    tracing::trace!("Already in requested state, no transition needed: {:?}", current_state);
                    trace!(id=%tx_id_clone, state=?current_state, "Already in requested state, no transition needed.");
                    continue;
                }

                if let Err(e) = AtomicTransactionState::validate_transition(logic.kind(), current_state, requested_new_state) {
                    tracing::trace!("Invalid state transition: {:?} -> {:?}, error: {}", current_state, requested_new_state, e);
                    error!(id=%tx_id_clone, error=%e, "Invalid state transition: {:?} -> {:?}", current_state, requested_new_state);
                    let _ = data.get_tu_event_sender().send(TransactionEvent::Error {
                        transaction_id: Some(tx_id_clone.clone()),
                        error: e.to_string(),
                    }).await;
                    continue;
                }

                tracing::trace!("Valid state transition: {:?} -> {:?}", current_state, requested_new_state);
                debug!(id=%tx_id_clone, "State transition: {:?} -> {:?}", current_state, requested_new_state);
                logic.cancel_all_specific_timers(&mut timer_handles);
                let previous_state = data.as_ref_state().set(requested_new_state);
                tracing::trace!("State successfully changed to: {:?}", requested_new_state);
                debug!(id=%tx_id_clone, "State changed from {:?} to {:?}", previous_state, requested_new_state);

                // Handle lifecycle transition if entering terminal state
                if requested_new_state == TransactionState::Terminated {
                    debug!(id=%tx_id_clone, "Entering terminal state - transitioning to Terminating lifecycle");
                    data.set_lifecycle(TransactionLifecycle::Terminating);
                }
                
                // Only send event if transaction should emit events (not in draining states)
                let should_send_event = data.should_emit_events();
                if should_send_event {
                    // Try to send event with timeout to prevent blocking on full channel
                    let sender = data.get_tu_event_sender();
                    let send_future = sender.send(TransactionEvent::StateChanged {
                        transaction_id: tx_id_clone.clone(),
                        previous_state,
                        new_state: requested_new_state,
                    });
                    
                    let result = tokio::time::timeout(
                        std::time::Duration::from_millis(100),
                        send_future
                    ).await;
                
                    match result {
                        Ok(Ok(())) => {
                            tracing::trace!("Sent StateChanged event result: Success");
                        }
                        Ok(Err(e)) => {
                            // Channel is closed
                            error!(id=%tx_id_clone, "Failed to send StateChanged event: channel closed");
                            
                            // In test mode, don't terminate transactions when event channels close
                            if is_test_mode {
                                debug!(id=%tx_id_clone, "Test mode detected, continuing despite closed event channel");
                            } else if requested_new_state == TransactionState::Terminated {
                                // If we're already terminating, this is expected
                                debug!(id=%tx_id_clone, "Event channel closed during termination - expected");
                            } else {
                                // In production, only terminate if we're certain the channel is closed
                                warn!(id=%tx_id_clone, "Event channel appears closed, initiating graceful shutdown");
                                logic.cancel_all_specific_timers(&mut timer_handles);
                                data.as_ref_state().set(TransactionState::Terminated);
                                break;
                            }
                        }
                        Err(_) => {
                            // Timeout - channel is likely full, log but continue
                            warn!(id=%tx_id_clone, "Timeout sending StateChanged event - channel may be congested");
                            tracing::trace!("Sent StateChanged event result: Timeout (channel congested)");
                            // Don't terminate on timeout - just continue processing
                        }
                    }
                } else {
                    debug!(id=%tx_id_clone, "Transaction in draining state, not emitting StateChanged event");
                }
                
                // If we've reached Terminated state, start grace period timer
                if requested_new_state == TransactionState::Terminated && data.get_lifecycle() == TransactionLifecycle::Terminating {
                    debug!(id=%tx_id_clone, "Starting grace period for terminated transaction");
                    let data_clone = data.clone();
                    let tx_id_for_timer = tx_id_clone.clone();
                    
                    tokio::spawn(async move {
                        // Wait for grace period
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        debug!(id=%tx_id_for_timer, "Grace period expired, transitioning to Draining");
                        data_clone.set_lifecycle(TransactionLifecycle::Draining);
                        
                        // After additional time, transition to Destroyed
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        debug!(id=%tx_id_for_timer, "Draining period complete, ready for cleanup");
                        data_clone.set_lifecycle(TransactionLifecycle::Destroyed);
                    });
                }

                if let Err(e) = logic.on_enter_state(
                    &data,
                    requested_new_state,
                    previous_state,
                    &mut timer_handles,
                    data.get_self_command_sender(),
                ).await {
                    error!(id=%tx_id_clone, error=%e, "Error in on_enter_state for state {:?}", requested_new_state);
                    
                    // Try to send error event with timeout
                    let sender = data.get_tu_event_sender();
                    let send_future = sender.send(TransactionEvent::Error {
                        transaction_id: Some(tx_id_clone.clone()),
                        error: format!("Error entering state {:?}: {}", requested_new_state, e),
                    });
                    
                    let result = tokio::time::timeout(
                        std::time::Duration::from_millis(100),
                        send_future
                    ).await;
                    
                    // Only terminate on actual channel closure, not on timeout
                    if let Ok(Err(_)) = result {
                        if !is_test_mode {
                            debug!(id=%tx_id_clone, "Cannot send errors to TU, initiating graceful shutdown");
                            logic.cancel_all_specific_timers(&mut timer_handles);
                            data.as_ref_state().set(TransactionState::Terminated);
                            break;
                        }
                    }
                }
            }
            InternalTransactionCommand::ProcessMessage(message) => {
                debug!(id=%tx_id_clone, "Received ProcessMessage command with {:?}", message);
                match logic.process_message(&data, message, current_state, &mut timer_handles).await {
                    Ok(Some(next_state)) => {
                        if let Err(e) = data.get_self_command_sender().send(InternalTransactionCommand::TransitionTo(next_state)).await {
                             error!(id=%tx_id_clone, error=%e, "Failed to send self-command for state transition after ProcessMessage");
                        }
                    }
                    Ok(None) => { /* No state change needed */ }
                    Err(e) => {
                        error!(id=%tx_id_clone, error=%e, "Error processing message in state {:?}", current_state);
                        
                        // Try to send error event with timeout
                        let sender = data.get_tu_event_sender();
                        let send_future = sender.send(TransactionEvent::Error {
                            transaction_id: Some(tx_id_clone.clone()),
                            error: e.to_string(),
                        });
                        
                        let result = tokio::time::timeout(
                            std::time::Duration::from_millis(100),
                            send_future
                        ).await;
                        
                        // Only terminate on actual channel closure, not on timeout
                        if let Ok(Err(_)) = result {
                            if !is_test_mode {
                                debug!(id=%tx_id_clone, "Cannot send errors to TU, initiating graceful shutdown");
                                logic.cancel_all_specific_timers(&mut timer_handles);
                                data.as_ref_state().set(TransactionState::Terminated);
                                break;
                            }
                        }
                    }
                }
            }
            InternalTransactionCommand::Timer(timer_name) => {
                match logic.handle_timer(&data, &timer_name, current_state, &mut timer_handles).await {
                    Ok(Some(next_state)) => {
                        if let Err(e) = data.get_self_command_sender().send(InternalTransactionCommand::TransitionTo(next_state)).await {
                             error!(id=%tx_id_clone, error=%e, "Failed to send self-command for state transition after Timer");
                        }
                    }
                    Ok(None) => { /* No state change needed */ }
                    Err(e) => {
                        error!(id=%tx_id_clone, error=%e, "Error handling timer '{}' in state {:?}", timer_name, current_state);
                        
                        // Try to send error event with timeout
                        let sender = data.get_tu_event_sender();
                        let send_future = sender.send(TransactionEvent::Error {
                            transaction_id: Some(tx_id_clone.clone()),
                            error: e.to_string(),
                        });
                        
                        let result = tokio::time::timeout(
                            std::time::Duration::from_millis(100),
                            send_future
                        ).await;
                        
                        // Only terminate on actual channel closure, not on timeout
                        if let Ok(Err(_)) = result {
                            if !is_test_mode {
                                debug!(id=%tx_id_clone, "Cannot send errors to TU, initiating graceful shutdown");
                                logic.cancel_all_specific_timers(&mut timer_handles);
                                data.as_ref_state().set(TransactionState::Terminated);
                                break;
                            }
                        }
                    }
                }
            }
            InternalTransactionCommand::TransportError => {
                error!(id=%tx_id_clone, "Transport error occurred, terminating transaction");
                
                // Try to send transport error event with timeout
                let sender = data.get_tu_event_sender();
                let send_future = sender.send(TransactionEvent::TransportError {
                    transaction_id: tx_id_clone.clone(),
                });
                
                let result = tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    send_future
                ).await;
                
                // Only skip shutdown on actual channel closure in test mode
                if let Ok(Err(_)) = result {
                    if !is_test_mode {
                        debug!(id=%tx_id_clone, "Cannot send transport error to TU, initiating graceful shutdown");
                        logic.cancel_all_specific_timers(&mut timer_handles);
                        data.as_ref_state().set(TransactionState::Terminated);
                        break;
                    }
                }
                
                if let Err(e) = data.get_self_command_sender().send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await {
                    error!(id=%tx_id_clone, error=%e, "Failed to send self-command for Terminated state on TransportError");
                    // Even if we can't send the command, still terminate
                    if !is_test_mode {
                        data.as_ref_state().set(TransactionState::Terminated);
                        break;
                    }
                }
            }
            InternalTransactionCommand::Terminate => {
                debug!(id=%tx_id_clone, "Received Terminate command, shutting down transaction");
                logic.cancel_all_specific_timers(&mut timer_handles);
                data.as_ref_state().set(TransactionState::Terminated);
                break;
            }
            
            InternalTransactionCommand::CancelTimer100 => {
                debug!(id=%tx_id_clone, "Received CancelTimer100 command, canceling automatic 100 Trying timer");
                // This command is specific to INVITE server transactions
                // The logic implementation will handle the actual timer cancellation
                if let Err(e) = logic.handle_cancel_timer_100(&mut timer_handles).await {
                    error!(id=%tx_id_clone, error=%e, "Failed to cancel Timer 100");
                }
            }
        }

        // Check lifecycle state instead of just RFC state for termination
        let lifecycle_state = data.get_lifecycle();
        if lifecycle_state == TransactionLifecycle::Destroyed {
            debug!(id=%tx_id_clone, "Transaction lifecycle is Destroyed, stopping event loop.");
            break;
        }
        
        // Handle messages in different lifecycle states
        if lifecycle_state != TransactionLifecycle::Active {
            debug!(id=%tx_id_clone, "Transaction in {:?} lifecycle - processing commands silently", lifecycle_state);
        }
    }

    let final_state = data.as_ref_state().get();
    tracing::trace!("Transaction loop ending for {}. Final state: {:?}", data.as_ref_key(), final_state);
    logic.cancel_all_specific_timers(&mut timer_handles);
    debug!(id = %data.as_ref_key().branch, final_state=?final_state, "Generic transaction loop ended.");

    if final_state == TransactionState::Terminated {
        // Try to send termination event with timeout (best effort)
        let sender = data.get_tu_event_sender();
        let send_future = sender.send(TransactionEvent::TransactionTerminated {
            transaction_id: data.as_ref_key().clone(),
        });
        
        let _ = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            send_future
        ).await;
        // Don't log errors here as it's expected during shutdown
    }
}

/// Trait for accessing a transaction's state.
///
/// This trait allows the runner to access the transaction's state without knowing
/// the concrete data type. The state is wrapped in an `Arc<AtomicTransactionState>`
/// for thread-safe access from multiple tasks.
pub trait AsRefState {
    /// Returns a reference to the transaction's state storage.
    fn as_ref_state(&self) -> &Arc<AtomicTransactionState>;
}

/// Trait for accessing a transaction's key.
///
/// This trait allows the runner to access the transaction's key without knowing
/// the concrete data type. The key uniquely identifies the transaction within
/// the transaction layer.
pub trait AsRefKey {
    /// Returns a reference to the transaction's key.
    fn as_ref_key(&self) -> &TransactionKey;
}

/// Trait for accessing a transaction's event sender.
///
/// This trait allows the runner to send events to the Transaction User (TU)
/// without knowing the concrete data type. These events inform the TU about
/// significant transaction events like state changes, responses, and errors.
pub trait HasTransactionEvents {
    /// Returns the channel sender for communicating with the TU.
    fn get_tu_event_sender(&self) -> mpsc::Sender<TransactionEvent>;
}

/// Trait for accessing the transport layer.
///
/// This trait allows the runner to access the SIP transport layer for sending
/// messages without knowing the concrete data type. The transport layer is
/// responsible for actually sending SIP messages over the network.
pub trait HasTransport {
    /// Returns a reference to the transport layer implementation.
    fn get_transport_layer(&self) -> Arc<dyn rvoip_sip_transport::Transport>;
}

/// Trait for accessing a transaction's command sender.
///
/// This trait allows the runner to send commands to itself (typically as a result
/// of timer expirations or message processing) without knowing the concrete data type.
/// This is used for things like scheduling state transitions.
pub trait HasCommandSender {
    /// Returns the channel sender for sending commands to this transaction.
    fn get_self_command_sender(&self) -> mpsc::Sender<InternalTransactionCommand>;
}

/// Trait for managing transaction lifecycle state.
/// Required by the transaction runner to coordinate robust shutdown.
pub trait HasLifecycle {
    /// Gets the current lifecycle state
    fn get_lifecycle(&self) -> TransactionLifecycle;
    
    /// Sets the lifecycle state
    fn set_lifecycle(&self, new_lifecycle: TransactionLifecycle);
    
    /// Checks if the transaction should emit events to the Transaction User
    fn should_emit_events(&self) -> bool;
} 