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
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, error, trace, warn};

use rvoip_sip_core::Message; // Assuming common Message type
use crate::error::{Error, Result};
use crate::transaction::{
    TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand, AtomicTransactionState,
};
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
       HasTransactionEvents + HasTransport + HasCommandSender + Send + Sync + 'static,
    TH: Default + Send + Sync + 'static,
    L: TransactionLogic<D, TH> + Send + Sync + 'static,
{
    let mut timer_handles = TH::default();
    let tx_id = data.as_ref_key().clone();

    println!("Transaction loop starting for {}", tx_id);
    println!("Initial state: {:?}", data.as_ref_state().get());
    debug!(id = %tx_id, "Generic transaction loop starting. Initial state: {:?}", data.as_ref_state().get());

    while let Some(command) = cmd_rx.recv().await {
        let current_state = data.as_ref_state().get();
        let tx_id_clone = data.as_ref_key().clone();

        println!("Received command: {:?} for transaction {}", command, tx_id_clone);
        debug!(id=%tx_id_clone, ?command, "Transaction received command");
        
        match command {
            InternalTransactionCommand::TransitionTo(requested_new_state) => {
                println!("Processing TransitionTo({:?}) current state: {:?}", requested_new_state, current_state);
                debug!(id=%tx_id_clone, current_state=?current_state, new_state=?requested_new_state, "Processing state transition");
                
                if current_state == requested_new_state {
                    println!("Already in requested state, no transition needed: {:?}", current_state);
                    trace!(id=%tx_id_clone, state=?current_state, "Already in requested state, no transition needed.");
                    continue;
                }

                if let Err(e) = AtomicTransactionState::validate_transition(logic.kind(), current_state, requested_new_state) {
                    println!("Invalid state transition: {:?} -> {:?}, error: {}", current_state, requested_new_state, e);
                    error!(id=%tx_id_clone, error=%e, "Invalid state transition: {:?} -> {:?}", current_state, requested_new_state);
                    let _ = data.get_tu_event_sender().send(TransactionEvent::Error {
                        transaction_id: Some(tx_id_clone.clone()),
                        error: e.to_string(),
                    }).await;
                    continue;
                }

                println!("Valid state transition: {:?} -> {:?}", current_state, requested_new_state);
                debug!(id=%tx_id_clone, "State transition: {:?} -> {:?}", current_state, requested_new_state);
                logic.cancel_all_specific_timers(&mut timer_handles);
                let previous_state = data.as_ref_state().set(requested_new_state);
                println!("State successfully changed to: {:?}", requested_new_state);
                debug!(id=%tx_id_clone, "State changed from {:?} to {:?}", previous_state, requested_new_state);

                let result = data.get_tu_event_sender().send(TransactionEvent::StateChanged {
                    transaction_id: tx_id_clone.clone(),
                    previous_state,
                    new_state: requested_new_state,
                }).await;
                println!("Sent StateChanged event result: {}", if result.is_ok() { "Success" } else { "Failed" });
                if result.is_err() {
                    error!(id=%tx_id_clone, "Failed to send StateChanged event");
                }

                if let Err(e) = logic.on_enter_state(
                    &data,
                    requested_new_state,
                    previous_state,
                    &mut timer_handles,
                    data.get_self_command_sender(),
                ).await {
                    error!(id=%tx_id_clone, error=%e, "Error in on_enter_state for state {:?}", requested_new_state);
                     let _ = data.get_tu_event_sender().send(TransactionEvent::Error {
                        transaction_id: Some(tx_id_clone.clone()),
                        error: format!("Error entering state {:?}: {}", requested_new_state, e),
                    }).await;
                }
            }
            InternalTransactionCommand::ProcessMessage(message) => {
                match logic.process_message(&data, message, current_state).await {
                    Ok(Some(next_state)) => {
                        if let Err(e) = data.get_self_command_sender().send(InternalTransactionCommand::TransitionTo(next_state)).await {
                            error!(id=%tx_id_clone, error=%e, "Failed to send self-command for state transition after ProcessMessage");
                        }
                    }
                    Ok(None) => { /* No state change needed */ }
                    Err(e) => {
                        error!(id=%tx_id_clone, error=%e, "Error processing message in state {:?}", current_state);
                        let _ = data.get_tu_event_sender().send(TransactionEvent::Error {
                            transaction_id: Some(tx_id_clone.clone()),
                            error: e.to_string(),
                        }).await;
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
                         let _ = data.get_tu_event_sender().send(TransactionEvent::Error {
                            transaction_id: Some(tx_id_clone.clone()),
                            error: e.to_string(),
                        }).await;
                    }
                }
            }
            InternalTransactionCommand::TransportError => {
                error!(id=%tx_id_clone, "Transport error occurred, terminating transaction");
                let _ = data.get_tu_event_sender().send(TransactionEvent::TransportError {
                    transaction_id: tx_id_clone.clone(),
                }).await;
                if let Err(e) = data.get_self_command_sender().send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await {
                    error!(id=%tx_id_clone, error=%e, "Failed to send self-command for Terminated state on TransportError");
                }
            }
            InternalTransactionCommand::Terminate => {
                debug!(id=%tx_id_clone, "Received explicit termination command");
                if current_state != TransactionState::Terminated {
                    if let Err(e) = data.get_self_command_sender().send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await {
                        error!(id=%tx_id_clone, error=%e, "Failed to send self-command for Terminated state on explicit Terminate");
                    }
                } else {
                    debug!(id=%tx_id_clone, "Already terminated, stopping event loop.");
                    break;
                }
            }
        }

        if data.as_ref_state().get() == TransactionState::Terminated {
            debug!(id=%tx_id_clone, "Transaction reached Terminated state, stopping event loop.");
            break;
        }
    }

    let final_state = data.as_ref_state().get();
    println!("Transaction loop ending for {}. Final state: {:?}", data.as_ref_key(), final_state);
    logic.cancel_all_specific_timers(&mut timer_handles);
    debug!(id = %data.as_ref_key().branch, final_state=?final_state, "Generic transaction loop ended.");

    if final_state == TransactionState::Terminated {
         let _ = data.get_tu_event_sender().send(TransactionEvent::TransactionTerminated {
            transaction_id: data.as_ref_key().clone(),
        }).await;
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