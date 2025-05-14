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

// This function will be the core of the generic event loop.
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

    debug!(id = %data.as_ref_key().branch, "Generic transaction loop starting. Initial state: {:?}", data.as_ref_state().get());

    while let Some(command) = cmd_rx.recv().await {
        let current_state = data.as_ref_state().get();
        let tx_id_clone = data.as_ref_key().clone();

        match command {
            InternalTransactionCommand::TransitionTo(requested_new_state) => {
                if current_state == requested_new_state {
                    trace!(id=%tx_id_clone, state=?current_state, "Already in requested state, no transition needed.");
                    continue;
                }

                if let Err(e) = AtomicTransactionState::validate_transition(current_state, requested_new_state, logic.kind()) {
                    error!(id=%tx_id_clone, error=%e, "Invalid state transition: {:?} -> {:?}", current_state, requested_new_state);
                    let _ = data.get_tu_event_sender().send(TransactionEvent::Error {
                        transaction_id: Some(tx_id_clone.clone()),
                        error: e.to_string(),
                    }).await;
                    continue;
                }

                debug!(id=%tx_id_clone, "State transition: {:?} -> {:?}", current_state, requested_new_state);
                logic.cancel_all_specific_timers(&mut timer_handles);
                let previous_state = data.as_ref_state().set(requested_new_state);

                let _ = data.get_tu_event_sender().send(TransactionEvent::StateChanged {
                    transaction_id: tx_id_clone.clone(),
                    previous_state,
                    new_state: requested_new_state,
                }).await;

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

    logic.cancel_all_specific_timers(&mut timer_handles);
    debug!(id = %data.as_ref_key().branch, "Generic transaction loop ended.");

    if data.as_ref_state().get() == TransactionState::Terminated {
         let _ = data.get_tu_event_sender().send(TransactionEvent::TransactionTerminated {
            transaction_id: data.as_ref_key().clone(),
        }).await;
    }
}

pub trait AsRefState {
    fn as_ref_state(&self) -> &Arc<AtomicTransactionState>;
}

pub trait AsRefKey {
    fn as_ref_key(&self) -> &TransactionKey;
}

pub trait HasTransactionEvents {
    fn get_tu_event_sender(&self) -> mpsc::Sender<TransactionEvent>;
}

pub trait HasTransport {
    fn get_transport_layer(&self) -> Arc<dyn rvoip_sip_transport::Transport>;
}

pub trait HasCommandSender {
     fn get_self_command_sender(&self) -> mpsc::Sender<InternalTransactionCommand>;
} 