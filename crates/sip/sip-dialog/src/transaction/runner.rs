use std::env;
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
use tokio::sync::mpsc::{self, error::TrySendError};
use tracing::{debug, error, trace};

use crate::diagnostics;
use crate::transaction::logic::TransactionLogic;
use crate::transaction::state::TransactionLifecycle;
use crate::transaction::{
    AtomicTransactionState, InternalTransactionCommand, TransactionEvent, TransactionKey,
    TransactionState,
};

/// Fence an unrecoverable transaction-runner fault before publishing its
/// transaction-scoped `Error` event.
///
/// A scoped `Error` is a terminal observation at the dialog/session boundary.
/// The transaction must therefore be unable to accept another protocol command
/// by the time that event becomes visible.  In particular, recording an exact
/// completion failure while leaving the RFC state machine live lets a tracked
/// INFO/REFER/NOTIFY/UPDATE owner be released and reused while the old
/// transaction can still process a response.  Commit the one-way terminal
/// fence first; the common runner epilogue then performs manager removal and
/// publishes `TransactionTerminated` in the usual order.
async fn fence_internal_error<D, TH, L>(
    data: &Arc<D>,
    logic: &L,
    timer_handles: &mut TH,
    transaction_id: &TransactionKey,
    message: String,
) where
    D: AsRefState
        + AsRefKey
        + HasTransactionEvents
        + HasTransport
        + HasCommandSender
        + HasLifecycle
        + Send
        + Sync
        + 'static,
    TH: Default + Send + Sync + 'static,
    L: TransactionLogic<D, TH> + Send + Sync + 'static,
{
    logic.cancel_all_specific_timers(timer_handles);

    // Failure must win ahead of the generic Terminated fallback in the exact
    // completion cell. A final SIP response or a more-specific failure that
    // already won remains authoritative by the cell's first-writer rule.
    data.record_completion_failure(crate::transaction::ClientTransactionFailure::Internal);
    data.as_ref_state().set(TransactionState::Terminated);
    data.record_completion_state(TransactionState::Terminated);
    data.await_protocol_writes().await;

    // Destroyed is the irreversible protocol-processing fence.  The cleanup
    // queue is submitted by the common epilogue after this scoped event has
    // retained its transaction/dialog routing context.
    data.set_lifecycle(TransactionLifecycle::Destroyed);

    let _ = data
        .get_tu_event_sender()
        .send(TransactionEvent::Error {
            transaction_id: Some(transaction_id.clone()),
            error: message,
        })
        .await;
}

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
) where
    D: AsRefState
        + AsRefKey
        + HasTransactionEvents
        + HasTransport
        + HasCommandSender
        + HasLifecycle
        + Send
        + Sync
        + 'static,
    TH: Default + Send + Sync + 'static,
    L: TransactionLogic<D, TH> + Send + Sync + 'static,
{
    // Check if we're running in test mode
    let is_test_mode = env::var("RVOIP_TEST").map(|v| v == "1").unwrap_or(false);

    let mut timer_handles = TH::default();
    let mut compact_retired = false;
    let mut terminal_publication_claim = None;
    let tx_id = data.as_ref_key().clone();
    diagnostics::record_transaction_runner_started();
    struct RunnerExitGuard;
    impl Drop for RunnerExitGuard {
        fn drop(&mut self) {
            diagnostics::record_transaction_runner_exited();
        }
    }
    let _runner_exit = RunnerExitGuard;

    tracing::trace!(
        transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id),
        "Transaction loop starting"
    );
    tracing::trace!("Initial state: {:?}", data.as_ref_state().get());
    debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), test_mode = is_test_mode, "Generic transaction loop starting. Initial state: {:?}", data.as_ref_state().get());

    // Enter the constructor-selected initial state under the runner's own
    // timer-handle set. In particular, INVITE server Timer 100 must be owned
    // and cancelled/joined with this runner; a detached constructor task can
    // otherwise write or publish after terminal cleanup.
    let initial_state = data.as_ref_state().get();
    let initial_entry_error = logic
        .on_enter_state(
            &data,
            initial_state,
            initial_state,
            &mut timer_handles,
            data.get_self_command_sender(),
        )
        .await
        .err();
    if let Some(error) = initial_entry_error {
        error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&error), "Failed to enter initial transaction state");
        fence_internal_error(
            &data,
            logic.as_ref(),
            &mut timer_handles,
            &tx_id,
            format!(
                "Error entering initial state {:?}: {}",
                initial_state, error
            ),
        )
        .await;
    } else {
        let mut locally_owned_command = None;
        let mut locally_owned_response_completion: Option<(
            Arc<crate::transaction::server::SupervisedServerResponse>,
            crate::transaction::server::SupervisedServerResponseExecution,
            crate::transaction::error::Result<()>,
        )> = None;
        loop {
            let command = match locally_owned_command.take() {
                Some(command) => command,
                None => {
                    let Some(command) = cmd_rx.recv().await else {
                        break;
                    };
                    command
                }
            };
            let response_completion = locally_owned_response_completion.take();
            let current_state = data.as_ref_state().get();
            let tx_id_clone = data.as_ref_key().clone();

            tracing::trace!(
                command = ?crate::transaction::safe_diagnostics::SafeTransactionCommand::new(&command),
                transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone),
                "Received transaction command"
            );
            debug!(
                id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone),
                command=?crate::transaction::safe_diagnostics::SafeTransactionCommand::new(&command),
                "Transaction received command"
            );

            match command {
                InternalTransactionCommand::TransitionTo(requested_new_state) => {
                    tracing::trace!(
                        "Processing TransitionTo({:?}) current state: {:?}",
                        requested_new_state,
                        current_state
                    );
                    debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), current_state=?current_state, new_state=?requested_new_state, "Processing state transition");

                    if current_state == requested_new_state {
                        tracing::trace!(
                            "Already in requested state, no transition needed: {:?}",
                            current_state
                        );
                        trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), state=?current_state, "Already in requested state, no transition needed.");
                        if let Some((operation, _execution, result)) = response_completion {
                            operation.complete(result);
                        }
                        continue;
                    }

                    if let Err(e) = AtomicTransactionState::validate_transition(
                        logic.kind(),
                        current_state,
                        requested_new_state,
                    ) {
                        tracing::trace!(
                            ?current_state,
                            ?requested_new_state,
                            error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e),
                            "Invalid state transition"
                        );
                        error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Invalid state transition: {:?} -> {:?}", current_state, requested_new_state);
                        fence_internal_error(
                            &data,
                            logic.as_ref(),
                            &mut timer_handles,
                            &tx_id_clone,
                            e.to_string(),
                        )
                        .await;
                        break;
                    }

                    tracing::trace!(
                        "Valid state transition: {:?} -> {:?}",
                        current_state,
                        requested_new_state
                    );
                    debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "State transition: {:?} -> {:?}", current_state, requested_new_state);
                    logic.cancel_all_specific_timers(&mut timer_handles);
                    let terminal_publication =
                        if requested_new_state == TransactionState::Terminated {
                            data.terminal_event_publication()
                        } else {
                            None
                        };
                    if terminal_publication_claim.is_none() {
                        terminal_publication_claim = terminal_publication
                            .as_ref()
                            .and_then(|publication| publication.try_claim());
                    }
                    let owns_terminal_batch =
                        terminal_publication.is_none() || terminal_publication_claim.is_some();
                    let previous_state = data.as_ref_state().set(requested_new_state);
                    let emit_terminal_prefix = requested_new_state == TransactionState::Terminated
                        && owns_terminal_batch
                        && data.should_emit_events();
                    if emit_terminal_prefix {
                        if let Some(claim) = terminal_publication_claim.as_ref() {
                            claim.publication().record_prefix(previous_state);
                        }
                    }
                    // Exact completion is authoritative and must advance before
                    // the corresponding observational StateChanged event.
                    data.record_completion_state(requested_new_state);
                    if requested_new_state == TransactionState::Terminated {
                        data.await_protocol_writes().await;
                    }
                    tracing::trace!("State successfully changed to: {:?}", requested_new_state);
                    debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "State changed from {:?} to {:?}", previous_state, requested_new_state);

                    // Handle lifecycle transition if entering terminal state
                    if requested_new_state == TransactionState::Terminated {
                        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Entering terminal state - transitioning to Terminating lifecycle");
                        data.set_lifecycle(TransactionLifecycle::Terminating);
                    }

                    // Only send event if transaction should emit events (not in draining states)
                    let should_send_event = if requested_new_state == TransactionState::Terminated {
                        emit_terminal_prefix
                    } else {
                        data.should_emit_events()
                    };
                    if should_send_event {
                        // The primary TU stream is lossless. Optional observers
                        // are fanned out nonblocking after this send is accepted.
                        let sender = data.get_tu_event_sender();
                        let event = TransactionEvent::StateChanged {
                            transaction_id: tx_id_clone.clone(),
                            previous_state,
                            new_state: requested_new_state,
                        };

                        let result = if requested_new_state == TransactionState::Terminated {
                            if let Some(claim) = terminal_publication_claim.as_ref() {
                                sender.send_terminal_prefix(event, claim).await
                            } else {
                                sender.send(event).await
                            }
                        } else {
                            sender.send(event).await
                        };
                        if result.is_err() {
                            if requested_new_state == TransactionState::Terminated {
                                if let Some(claim) = terminal_publication_claim.take() {
                                    claim.mark_failed_closed();
                                }
                            }
                            debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), test_mode=is_test_mode, "StateChanged primary TU channel closed; continuing transaction");
                        } else {
                            tracing::trace!("Sent StateChanged event result: Success");
                        }
                    } else {
                        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Transaction in draining state, not emitting StateChanged event");
                    }

                    // If we've reached Terminated state, register its grace and
                    // drain deadlines with the shared due-driven scheduler. This
                    // preserves the lifecycle fence without spawning a sleeper
                    // task (and two Tokio timer entries) per transaction.
                    if requested_new_state == TransactionState::Terminated
                        && data.get_lifecycle() == TransactionLifecycle::Terminating
                    {
                        debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Scheduling consolidated grace period for terminated transaction");
                        crate::transaction::lifecycle_scheduler::schedule(data.clone()).await;
                    }

                    if let Err(e) = logic
                        .on_enter_state(
                            &data,
                            requested_new_state,
                            previous_state,
                            &mut timer_handles,
                            data.get_self_command_sender(),
                        )
                        .await
                    {
                        error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Error in on_enter_state for state {:?}", requested_new_state);

                        fence_internal_error(
                            &data,
                            logic.as_ref(),
                            &mut timer_handles,
                            &tx_id_clone,
                            format!("Error entering state {:?}: {}", requested_new_state, e),
                        )
                        .await;
                        break;
                    }
                }
                InternalTransactionCommand::ProcessMessage(message) => {
                    debug!(
                        id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone),
                        message=?crate::transaction::safe_diagnostics::SafeSipMessage::new(&message),
                        "Received ProcessMessage command"
                    );
                    match logic
                        .process_message(&data, message, current_state, &mut timer_handles)
                        .await
                    {
                        Ok(Some(next_state)) => {
                            if let Err(e) = data
                                .get_self_command_sender()
                                .send(InternalTransactionCommand::TransitionTo(next_state))
                                .await
                            {
                                error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to send self-command for state transition after ProcessMessage");
                            }
                        }
                        Ok(None) => { /* No state change needed */ }
                        Err(e) => {
                            error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Error processing message in state {:?}", current_state);

                            fence_internal_error(
                                &data,
                                logic.as_ref(),
                                &mut timer_handles,
                                &tx_id_clone,
                                e.to_string(),
                            )
                            .await;
                            break;
                        }
                    }
                }
                InternalTransactionCommand::SupervisedServerResponse(operation) => {
                    let Some(response) = operation.take_response() else {
                        operation.complete(Err(crate::transaction::error::Error::Other(
                            "supervised server response was already claimed".to_string(),
                        )));
                        continue;
                    };
                    let execution = operation.execution_guard();
                    match logic
                        .send_server_response(&data, response, current_state, &mut timer_handles)
                        .await
                    {
                        Ok(disposition) => {
                            if disposition.cancel_timer_100 {
                                if let Err(error) =
                                    logic.handle_cancel_timer_100(&mut timer_handles).await
                                {
                                    if let Some(next_state) = disposition.next_state {
                                        locally_owned_command = Some(
                                            InternalTransactionCommand::TransitionTo(next_state),
                                        );
                                        locally_owned_response_completion =
                                            Some((Arc::clone(&operation), execution, Err(error)));
                                    } else {
                                        operation.complete(Err(error));
                                    }
                                    continue;
                                }
                            }
                            if let Some(next_state) = disposition.next_state {
                                // The sole runner owns this transition locally
                                // before the API waiter can observe success.
                                // It cannot deadlock on or lose a self-send to
                                // the bounded command channel.
                                locally_owned_command =
                                    Some(InternalTransactionCommand::TransitionTo(next_state));
                                locally_owned_response_completion =
                                    Some((Arc::clone(&operation), execution, Ok(())));
                            } else {
                                operation.complete(Ok(()));
                            }
                        }
                        Err(error) => {
                            if let Some(next_state) = operation.wire_unknown_transition() {
                                locally_owned_command =
                                    Some(InternalTransactionCommand::TransitionTo(next_state));
                                locally_owned_response_completion =
                                    Some((Arc::clone(&operation), execution, Err(error)));
                            } else {
                                operation.complete(Err(error));
                            }
                        }
                    }
                }
                InternalTransactionCommand::Timer(timer_name) => {
                    match logic
                        .handle_timer(&data, &timer_name, current_state, &mut timer_handles)
                        .await
                    {
                        Ok(Some(next_state)) => {
                            if let Err(e) = data
                                .get_self_command_sender()
                                .send(InternalTransactionCommand::TransitionTo(next_state))
                                .await
                            {
                                error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to send self-command for state transition after Timer");
                            }
                        }
                        Ok(None) => { /* No state change needed */ }
                        Err(e) => {
                            error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), timer_class=%crate::transaction::safe_diagnostics::SafeTimerName::new(&timer_name), timer_len=timer_name.len(), state=?current_state, "Error handling timer");

                            fence_internal_error(
                                &data,
                                logic.as_ref(),
                                &mut timer_handles,
                                &tx_id_clone,
                                e.to_string(),
                            )
                            .await;
                            break;
                        }
                    }
                }
                InternalTransactionCommand::TransportError => {
                    error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Transport error occurred, terminating transaction");

                    data.record_completion_failure(
                        crate::transaction::ClientTransactionFailure::Transport,
                    );

                    let sender = data.get_tu_event_sender();
                    if sender
                        .send(TransactionEvent::TransportError {
                            transaction_id: tx_id_clone.clone(),
                        })
                        .await
                        .is_err()
                    {
                        if !is_test_mode {
                            debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Cannot send transport error to TU, initiating graceful shutdown");
                            logic.cancel_all_specific_timers(&mut timer_handles);
                            data.as_ref_state().set(TransactionState::Terminated);
                            break;
                        }
                    }

                    if let Err(e) = data
                        .get_self_command_sender()
                        .send(InternalTransactionCommand::TransitionTo(
                            TransactionState::Terminated,
                        ))
                        .await
                    {
                        error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to send self-command for Terminated state on TransportError");
                        // Even if we can't send the command, still terminate
                        if !is_test_mode {
                            data.as_ref_state().set(TransactionState::Terminated);
                            break;
                        }
                    }
                }
                InternalTransactionCommand::Terminate => {
                    debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Received Terminate command, shutting down transaction");
                    logic.cancel_all_specific_timers(&mut timer_handles);
                    data.record_completion_failure(
                        crate::transaction::ClientTransactionFailure::Cancelled,
                    );
                    if current_state != TransactionState::Terminated {
                        let terminal_publication = data.terminal_event_publication();
                        if terminal_publication_claim.is_none() {
                            terminal_publication_claim = terminal_publication
                                .as_ref()
                                .and_then(|publication| publication.try_claim());
                        }
                        let owns_terminal_batch =
                            terminal_publication.is_none() || terminal_publication_claim.is_some();
                        let previous_state = data.as_ref_state().set(TransactionState::Terminated);
                        let emit_terminal_prefix = owns_terminal_batch && data.should_emit_events();
                        if emit_terminal_prefix {
                            if let Some(claim) = terminal_publication_claim.as_ref() {
                                claim.publication().record_prefix(previous_state);
                            }
                        }
                        data.record_completion_state(TransactionState::Terminated);
                        data.await_protocol_writes().await;
                        if emit_terminal_prefix {
                            let sender = data.get_tu_event_sender();
                            let event = TransactionEvent::StateChanged {
                                transaction_id: tx_id_clone.clone(),
                                previous_state,
                                new_state: TransactionState::Terminated,
                            };
                            let result = if let Some(claim) = terminal_publication_claim.as_ref() {
                                sender.send_terminal_prefix(event, claim).await
                            } else {
                                sender.send(event).await
                            };
                            if result.is_err() {
                                if let Some(claim) = terminal_publication_claim.take() {
                                    claim.mark_failed_closed();
                                }
                                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Explicit termination StateChanged primary TU channel closed");
                            }
                        }
                    } else {
                        data.await_protocol_writes().await;
                    }
                    data.set_lifecycle(TransactionLifecycle::Destroyed);
                    break;
                }
                InternalTransactionCommand::CompactRetire => {
                    debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Retiring completed transaction into compact manager tombstone");
                    logic.cancel_all_specific_timers(&mut timer_handles);
                    data.set_lifecycle(TransactionLifecycle::Destroyed);
                    compact_retired = true;
                    break;
                }

                InternalTransactionCommand::CancelTimer100 => {
                    debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Received CancelTimer100 command, canceling automatic 100 Trying timer");
                    // This command is specific to INVITE server transactions
                    // The logic implementation will handle the actual timer cancellation
                    if let Err(e) = logic.handle_cancel_timer_100(&mut timer_handles).await {
                        error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to cancel Timer 100");
                    }
                }
            }

            if let Some((operation, _execution, result)) = response_completion {
                operation.complete(result);
            }

            // Check lifecycle state instead of just RFC state for termination
            let lifecycle_state = data.get_lifecycle();
            if lifecycle_state == TransactionLifecycle::Destroyed {
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Transaction lifecycle is Destroyed, stopping event loop.");
                break;
            }

            // Handle messages in different lifecycle states
            if lifecycle_state != TransactionLifecycle::Active {
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id_clone), "Transaction in {:?} lifecycle - processing commands silently", lifecycle_state);
            }
        }
    }

    let final_state = data.as_ref_state().get();
    tracing::trace!(
        transaction=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(data.as_ref_key()),
        ?final_state,
        "Transaction loop ending"
    );
    logic.cancel_all_specific_timers(&mut timer_handles);
    debug!(
        id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(data.as_ref_key()),
        final_state=?final_state,
        "Generic transaction loop ended."
    );

    if final_state == TransactionState::Terminated || compact_retired {
        data.await_protocol_writes().await;
        // Every terminal exit is authoritative. Exceptional exits can bypass
        // the normal grace scheduler (for example, a closed control channel),
        // so make their lifecycle removable before notifying the manager.
        if data.get_lifecycle() != TransactionLifecycle::Destroyed {
            data.set_lifecycle(TransactionLifecycle::Destroyed);
        }

        // Authoritative cleanup is independent of public observation. Enqueue
        // it before publishing the terminal event so a saturated observer
        // cannot retain the transaction runner or its routes.
        if let Some(cleanup_tx) = data.termination_cleanup_sender() {
            let transaction_id = data.as_ref_key().clone();
            match cleanup_tx.try_send(transaction_id) {
                Ok(()) => diagnostics::record_termination_cleanup_enqueued(),
                Err(TrySendError::Full(transaction_id)) => {
                    // The bounded queue is allowed to apply backpressure only
                    // after protocol completion; public observation below is
                    // nonblocking and cannot delay authoritative cleanup.
                    diagnostics::record_termination_cleanup_queue_full();
                    let _ = cleanup_tx.send(transaction_id).await;
                }
                Err(TrySendError::Closed(_)) => {}
            }
        }

        let publication = data.terminal_event_publication();
        if terminal_publication_claim.is_none() {
            terminal_publication_claim = publication
                .as_ref()
                .and_then(|publication| publication.try_claim());
        }
        let publication_claim = terminal_publication_claim;
        if !compact_retired && (publication.is_none() || publication_claim.is_some()) {
            // The primary TU stream remains lossless and ordered after the
            // final StateChanged event. Optional observers are bounded in the
            // manager fanout and cannot backpressure this protocol path.
            let sender = data.get_tu_event_sender();
            let result = sender
                .send_terminal(
                    TransactionEvent::TransactionTerminated {
                        transaction_id: data.as_ref_key().clone(),
                    },
                    None,
                    data.transaction_admission_owner(),
                )
                .await;
            match (result, publication_claim) {
                (Err(_), Some(claim)) => {
                    claim.mark_failed_closed();
                    debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(data.as_ref_key()), "TransactionTerminated primary TU channel closed");
                }
                (Err(_), None) => {
                    debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(data.as_ref_key()), "TransactionTerminated primary TU channel closed");
                }
                (Ok(()), Some(claim)) => claim.mark_delivered(),
                (Ok(()), None) => {}
            }
        }
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
    fn get_tu_event_sender(&self) -> crate::transaction::event_sender::TransactionEventSender;
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

    /// Whether this transaction was attached to a manager-owned lifecycle
    /// scheduler. Kept separate from `schedule_lifecycle` so a scheduler that
    /// has already shut down can be distinguished from a standalone
    /// transaction that needs the compatibility fallback.
    fn lifecycle_scheduler_installed(&self) -> bool {
        false
    }

    /// Enqueue this exact transaction on its manager-owned lifecycle worker.
    /// The opaque boolean result avoids exposing the scheduler implementation
    /// through this otherwise-public runner trait.
    async fn schedule_lifecycle(self: Arc<Self>) -> bool
    where
        Self: Sized + Send + Sync + 'static,
    {
        false
    }

    /// Optional manager-owned cleanup queue. Standalone transactions omit it
    /// and rely on drop/explicit shutdown; manager-created transactions use it
    /// to remove exact routes immediately after their runner exits.
    fn termination_cleanup_sender(&self) -> Option<mpsc::Sender<TransactionKey>> {
        None
    }

    /// Clone the exact manager admission owner into terminal delivery. The
    /// integrated dialog consumer holds it until all derived routing cleanup
    /// is complete; standalone transactions return `None`.
    fn transaction_admission_owner(
        &self,
    ) -> Option<crate::transaction::manager::TransactionAdmissionOwner> {
        None
    }

    /// Claim the one authoritative terminal-event publication. Standalone
    /// custom data keeps the historical single-runner behavior.
    fn terminal_event_publication(
        &self,
    ) -> Option<Arc<crate::transaction::event_sender::TerminalEventPublication>> {
        None
    }

    /// Wait until any server response wire write that began before a terminal
    /// state transition has finished. Client transactions have no analogous
    /// concurrent response writer.
    async fn await_protocol_writes(&self) {}

    /// Update the exact client-completion authority before publishing the
    /// corresponding public state event. Server transactions use the no-op.
    fn record_completion_state(&self, _state: TransactionState) {}

    /// Record a typed terminal client failure before public error/timeout
    /// observation. Server transactions use the no-op.
    fn record_completion_failure(&self, _failure: crate::transaction::ClientTransactionFailure) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::completion::ClientTransactionCompletion;
    use crate::transaction::error::{Error, Result};
    use crate::transaction::timer::TimerSettings;
    use crate::transaction::{ClientTransactionFailure, ClientTransactionOutcome, TransactionKind};
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::prelude::{Message, Method};
    use rvoip_sip_transport::Transport;
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
    use std::time::Duration;

    #[derive(Debug)]
    struct TestTransport;

    #[async_trait::async_trait]
    impl Transport for TestTransport {
        async fn send_message(
            &self,
            _message: Message,
            _destination: SocketAddr,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok("127.0.0.1:5060".parse().expect("test socket address"))
        }

        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    struct FaultData {
        state: Arc<AtomicTransactionState>,
        key: TransactionKey,
        events: crate::transaction::event_sender::TransactionEventSender,
        transport: Arc<TestTransport>,
        commands: mpsc::Sender<InternalTransactionCommand>,
        lifecycle: AtomicU8,
        completion: Arc<ClientTransactionCompletion>,
        cleanup: mpsc::Sender<TransactionKey>,
        timer_settings: TimerSettings,
    }

    impl AsRefState for FaultData {
        fn as_ref_state(&self) -> &Arc<AtomicTransactionState> {
            &self.state
        }
    }

    impl AsRefKey for FaultData {
        fn as_ref_key(&self) -> &TransactionKey {
            &self.key
        }
    }

    impl HasTransactionEvents for FaultData {
        fn get_tu_event_sender(&self) -> crate::transaction::event_sender::TransactionEventSender {
            self.events.clone()
        }
    }

    impl HasTransport for FaultData {
        fn get_transport_layer(&self) -> Arc<dyn Transport> {
            self.transport.clone()
        }
    }

    impl HasCommandSender for FaultData {
        fn get_self_command_sender(&self) -> mpsc::Sender<InternalTransactionCommand> {
            self.commands.clone()
        }
    }

    impl HasLifecycle for FaultData {
        fn get_lifecycle(&self) -> TransactionLifecycle {
            match self.lifecycle.load(Ordering::Acquire) {
                0 => TransactionLifecycle::Active,
                1 => TransactionLifecycle::Terminating,
                2 => TransactionLifecycle::Draining,
                _ => TransactionLifecycle::Destroyed,
            }
        }

        fn set_lifecycle(&self, lifecycle: TransactionLifecycle) {
            let value = match lifecycle {
                TransactionLifecycle::Active => 0,
                TransactionLifecycle::Terminating => 1,
                TransactionLifecycle::Draining => 2,
                TransactionLifecycle::Destroyed => 3,
            };
            self.lifecycle.store(value, Ordering::Release);
        }

        fn should_emit_events(&self) -> bool {
            self.get_lifecycle() == TransactionLifecycle::Active
        }

        fn termination_cleanup_sender(&self) -> Option<mpsc::Sender<TransactionKey>> {
            Some(self.cleanup.clone())
        }

        fn record_completion_state(&self, state: TransactionState) {
            self.completion.record_state(state);
        }

        fn record_completion_failure(&self, failure: ClientTransactionFailure) {
            self.completion.record_failure(failure);
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FaultPoint {
        None,
        InitialEnter,
        TransitionEnter,
        ProcessMessage,
        Timer,
    }

    #[derive(Debug)]
    struct FaultLogic {
        fault: FaultPoint,
        timer_cancellations: AtomicUsize,
    }

    impl FaultLogic {
        fn new(fault: FaultPoint) -> Self {
            Self {
                fault,
                timer_cancellations: AtomicUsize::new(0),
            }
        }
    }

    #[derive(Default)]
    struct FaultTimerHandles;

    #[async_trait::async_trait]
    impl TransactionLogic<FaultData, FaultTimerHandles> for FaultLogic {
        fn kind(&self) -> TransactionKind {
            TransactionKind::NonInviteClient
        }

        fn initial_state(&self) -> TransactionState {
            TransactionState::Trying
        }

        fn timer_settings<'a>(data: &'a Arc<FaultData>) -> &'a TimerSettings {
            &data.timer_settings
        }

        async fn process_message(
            &self,
            _data: &Arc<FaultData>,
            _message: Message,
            _current_state: TransactionState,
            _timer_handles: &mut FaultTimerHandles,
        ) -> Result<Option<TransactionState>> {
            if self.fault == FaultPoint::ProcessMessage {
                Err(Error::Other("injected process-message failure".into()))
            } else {
                Ok(None)
            }
        }

        async fn handle_timer(
            &self,
            _data: &Arc<FaultData>,
            _timer_name: &str,
            _current_state: TransactionState,
            _timer_handles: &mut FaultTimerHandles,
        ) -> Result<Option<TransactionState>> {
            if self.fault == FaultPoint::Timer {
                Err(Error::Other("injected timer failure".into()))
            } else {
                Ok(None)
            }
        }

        async fn on_enter_state(
            &self,
            _data: &Arc<FaultData>,
            new_state: TransactionState,
            previous_state: TransactionState,
            _timer_handles: &mut FaultTimerHandles,
            _command_tx: mpsc::Sender<InternalTransactionCommand>,
        ) -> Result<()> {
            let initial_entry =
                new_state == TransactionState::Trying && previous_state == TransactionState::Trying;
            if (self.fault == FaultPoint::InitialEnter && initial_entry)
                || (self.fault == FaultPoint::TransitionEnter
                    && new_state == TransactionState::Proceeding)
            {
                Err(Error::Other("injected state-entry failure".into()))
            } else {
                Ok(())
            }
        }

        fn cancel_all_specific_timers(&self, _timer_handles: &mut FaultTimerHandles) {
            self.timer_cancellations.fetch_add(1, Ordering::AcqRel);
        }
    }

    fn process_message_command() -> InternalTransactionCommand {
        let request = SimpleRequestBuilder::new(Method::Options, "sip:bob@example.com")
            .expect("OPTIONS builder")
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", Some("bob-tag"))
            .call_id("runner-terminal-error-test")
            .cseq(1)
            .via(
                "127.0.0.1:5060",
                "UDP",
                Some("z9hG4bK.runner-terminal-error-test"),
            )
            .max_forwards(70)
            .build();
        InternalTransactionCommand::ProcessMessage(Message::Request(request))
    }

    async fn assert_fault_is_terminally_fenced(
        fault: FaultPoint,
        command: Option<InternalTransactionCommand>,
    ) {
        let (events_tx, mut events_rx) = mpsc::channel(8);
        let (commands_tx, commands_rx) = mpsc::channel(8);
        let (cleanup_tx, mut cleanup_rx) = mpsc::channel(2);
        let key = TransactionKey::new(
            format!("z9hG4bK.runner-fault-{fault:?}"),
            Method::Update,
            false,
        );
        let completion = Arc::new(ClientTransactionCompletion::new(TransactionState::Trying));
        let data = Arc::new(FaultData {
            state: Arc::new(AtomicTransactionState::new(TransactionState::Trying)),
            key: key.clone(),
            events: crate::transaction::event_sender::TransactionEventSender::new(events_tx),
            transport: Arc::new(TestTransport),
            commands: commands_tx.clone(),
            lifecycle: AtomicU8::new(0),
            completion: completion.clone(),
            cleanup: cleanup_tx,
            timer_settings: TimerSettings::default(),
        });
        let logic = Arc::new(FaultLogic::new(fault));
        let runner = tokio::spawn(run_transaction_loop(
            data.clone(),
            logic.clone(),
            commands_rx,
        ));

        if let Some(command) = command {
            commands_tx
                .send(command)
                .await
                .expect("fault command accepted before the terminal fence");
        }

        let scoped_error = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let event = events_rx.recv().await.expect("runner event stream");
                if let TransactionEvent::Error {
                    transaction_id: Some(transaction_id),
                    error,
                } = event
                {
                    break (transaction_id, error);
                }
            }
        })
        .await
        .expect("transaction-scoped Error was not published");

        assert_eq!(scoped_error.0, key);
        assert!(scoped_error.1.contains("injected") || scoped_error.1.contains("Invalid"));
        assert_eq!(data.state.get(), TransactionState::Terminated);
        assert_eq!(data.get_lifecycle(), TransactionLifecycle::Destroyed);
        assert!(matches!(
            completion
                .wait_for_outcome(Duration::from_millis(10))
                .await
                .expect("exact completion outcome"),
            Some(ClientTransactionOutcome::Failure(
                ClientTransactionFailure::Internal
            ))
        ));

        tokio::time::timeout(Duration::from_secs(2), runner)
            .await
            .expect("terminally fenced runner did not exit")
            .expect("terminally fenced runner panicked");
        assert_eq!(cleanup_rx.recv().await, Some(key.clone()));
        assert!(commands_tx
            .send(InternalTransactionCommand::Timer("after-error".into()))
            .await
            .is_err());
        assert!(logic.timer_cancellations.load(Ordering::Acquire) > 0);

        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), events_rx.recv()).await,
            Ok(Some(TransactionEvent::TransactionTerminated { transaction_id }))
                if transaction_id == key
        ));
    }

    #[tokio::test]
    async fn invalid_transition_error_is_terminally_fenced() {
        assert_fault_is_terminally_fenced(
            FaultPoint::None,
            Some(InternalTransactionCommand::TransitionTo(
                TransactionState::Confirmed,
            )),
        )
        .await;
    }

    #[tokio::test]
    async fn state_entry_errors_are_terminally_fenced() {
        assert_fault_is_terminally_fenced(FaultPoint::InitialEnter, None).await;
        assert_fault_is_terminally_fenced(
            FaultPoint::TransitionEnter,
            Some(InternalTransactionCommand::TransitionTo(
                TransactionState::Proceeding,
            )),
        )
        .await;
    }

    #[tokio::test]
    async fn process_message_error_is_terminally_fenced() {
        assert_fault_is_terminally_fenced(
            FaultPoint::ProcessMessage,
            Some(process_message_command()),
        )
        .await;
    }

    #[tokio::test]
    async fn timer_error_is_terminally_fenced() {
        assert_fault_is_terminally_fenced(
            FaultPoint::Timer,
            Some(InternalTransactionCommand::Timer("injected".into())),
        )
        .await;
    }
}
