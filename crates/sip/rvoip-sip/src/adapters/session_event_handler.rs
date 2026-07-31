//! Session Event Handler - Central hub for ALL cross-crate event handling
//!
//! This is the ONLY place where cross-crate events are handled.
//! - Receives events from dialog-core and media-core
//! - Routes them to the state machine
//! - Publishes events to dialog-core and media-core
//!
//! NO OTHER MODULE should interact with the GlobalEventCoordinator directly.

use crate::adapters::outbound_request_tracker::{
    DeferredTrackedRequestEvent, ExactTransactionLookup, OutboundInDialogRequestTracker,
    TrackedInDialogMethod,
};
use crate::adapters::registration_adapter::{RegistrationAdapter, RegistrationAdapterInstall};
use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::api::lifecycle::{
    ExactTerminalClaim, ExactTerminalCompletion, LifecycleIndex, SessionEventPublisher,
};
use crate::cleanup_diag::{self, CleanupStage};
use crate::errors::{Result as SessionResult, SessionError};
use crate::retained_tasks::RetainedTasks;
use crate::session_lifecycle::{
    OwnedOperation, OwnedOperationCompletion, SessionAdmissionError, SessionOperationKind,
};
use crate::session_registry::{PendingInboundBundle, SessionRegistry, SessionRegistryHandle};
use crate::session_store::SessionStateSnapshot;
use crate::state_machine::executor::{
    AuthRequiredProcessOutcome, AuthRequiredStateInput, InboundResponseStateInput,
    Invite2xxAckStateInput, ReferNotifyInput, ReferNotifyOutcome, SessionRefreshStateInput,
    TransferRequestStateInput,
};
use crate::state_machine::{
    ProcessEventResult, StateMachine as StateMachineExecutor, StateMachineHelpers,
};
use crate::state_table::types::{EventTemplate, EventType, Role, SessionId};
use crate::types::{CallState, DialogId};
use anyhow::Result;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::{CrossCrateEventHandler, GlobalEventCoordinator};
use rvoip_infra_common::events::cross_crate::{
    CrossCrateEvent, DialogToSessionEvent, MediaToSessionEvent, OutboundRequestOutcome,
    RvoipCrossCrateEvent, SipTraceEvent,
};
use rvoip_infra_common::planes::routing::RoutableEvent;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{debug, error, info, warn};

const STATE_MACHINE_DISPATCH_JOIN_FAILURE: &str =
    "SIP state-machine dispatch task failed (class=join)";
const REFER_DEFAULT_ACTION_DELAY: Duration = Duration::from_millis(500);
const REFER_DEFAULT_ACTION_COMPLETION_GRACE: Duration = Duration::from_secs(2);

type StateMachineProcessResult =
    std::result::Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommittedDialogTermination {
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommittedCallFailure {
    Failed,
    Cancelled,
    NonTerminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommittedDialog200 {
    InitialAnswer,
    NonInitial,
}

/// Terminal transport classification for a retained fail-fast SIP response.
/// `WireUnknown` is deliberately terminal: another response attempt could
/// duplicate a final response that the transaction runner already wrote.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExactFinalResponseOutcome {
    Written,
    ZeroWire,
    WireUnknown,
}

fn exact_final_response_outcome(
    disposition: rvoip_sip_dialog::FinalResponseCompletionDisposition,
) -> ExactFinalResponseOutcome {
    match disposition {
        rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal => {
            ExactFinalResponseOutcome::Written
        }
        rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable => {
            ExactFinalResponseOutcome::ZeroWire
        }
        rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal => {
            ExactFinalResponseOutcome::WireUnknown
        }
    }
}

fn exact_final_response_result(
    operation: &'static str,
    outcome: ExactFinalResponseOutcome,
) -> Result<()> {
    match outcome {
        ExactFinalResponseOutcome::Written => Ok(()),
        ExactFinalResponseOutcome::ZeroWire => Err(anyhow::anyhow!(
            "{operation} failed before an exact transport write"
        )),
        ExactFinalResponseOutcome::WireUnknown => {
            warn!(
                operation,
                "Exact final response became wire-unknown; suppressing every duplicate attempt"
            );
            Ok(())
        }
    }
}

fn exact_final_response_retires_routes(outcome: ExactFinalResponseOutcome) -> bool {
    !matches!(outcome, ExactFinalResponseOutcome::ZeroWire)
}

/// Translate a state-machine response failure into the causal ingress ACK.
/// Only transaction-core's terminal dispositions may be acknowledged as
/// handled: `ZeroWireRetryable` and unclassified lifecycle/action failures
/// must reach dialog-core so it can select its one safe local fallback.
fn exact_response_failure_processing_ack(
    operation: &'static str,
    disposition: Option<rvoip_sip_dialog::FinalResponseCompletionDisposition>,
    detail: &str,
) -> Result<()> {
    match disposition {
        Some(
            rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal
            | rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
        ) => Ok(()),
        Some(rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable) | None => {
            Err(anyhow::anyhow!(
                "{operation} did not complete a terminal wire response: {detail}"
            ))
        }
    }
}

/// Admit a cancellation-safe child before returning its completion waiter.
/// Dropping the waiter never aborts the response/cleanup future; handler
/// shutdown joins it through the shared retained-task owner.
fn spawn_retained_exact_response_completion<F>(
    retained_tasks: &Arc<RetainedTasks>,
    future: F,
) -> SessionResult<oneshot::Receiver<ExactFinalResponseOutcome>>
where
    F: std::future::Future<Output = ExactFinalResponseOutcome> + Send + 'static,
{
    let (completion, waiter) = oneshot::channel();
    if !retained_tasks.spawn_or_child(async move {
        let outcome = future.await;
        let _ = completion.send(outcome);
    }) {
        return Err(SessionError::InternalError(
            "exact final-response retained task admission is closed".to_string(),
        ));
    }
    Ok(waiter)
}

/// Keep built-in registrar processing alive after the dialog publisher drops
/// its causal ACK waiter. The shared task registry is drained before dialog
/// transports stop, so a classified final response cannot be orphaned during
/// normal shutdown either.
fn spawn_retained_register_processing<F>(
    retained_tasks: &Arc<RetainedTasks>,
    future: F,
) -> SessionResult<oneshot::Receiver<SessionResult<()>>>
where
    F: std::future::Future<Output = SessionResult<()>> + Send + 'static,
{
    let (completion, waiter) = oneshot::channel();
    if !retained_tasks.spawn_or_child(async move {
        let outcome = future.await;
        let _ = completion.send(outcome);
    }) {
        return Err(SessionError::InternalError(
            "REGISTER response retained-task admission is closed".to_string(),
        ));
    }
    Ok(waiter)
}

/// Retain upper failed-inbound cleanup independently from the causal ACK
/// waiter. The future deliberately excludes lower dialog-core cleanup so the
/// original server INVITE remains available to the caller's classified 503
/// fallback until this waiter resolves with a negative ACK.
fn spawn_retained_failed_inbound_cleanup<F>(
    retained_tasks: &Arc<RetainedTasks>,
    future: F,
) -> SessionResult<oneshot::Receiver<SessionResult<()>>>
where
    F: std::future::Future<Output = SessionResult<()>> + Send + 'static,
{
    let (completion, waiter) = oneshot::channel();
    if !retained_tasks.spawn_or_child(async move {
        let outcome = future.await;
        let _ = completion.send(outcome);
    }) {
        return Err(SessionError::InternalError(
            "failed inbound INVITE cleanup retained-task admission is closed".to_string(),
        ));
    }
    Ok(waiter)
}

fn remove_quiesced_failed_inbound_store_lifetime(
    store: &crate::session_store::SessionStore,
    handle: &SessionRegistryHandle,
) -> SessionResult<()> {
    match store.remove_quiesced_session_exact(handle) {
        Ok(()) => Ok(()),
        Err(_) if store.get_session_retained_snapshot_exact(handle).is_err() => Ok(()),
        Err(error) => Err(SessionError::InternalError(format!(
            "failed inbound INVITE upper-session removal failed (class=lifecycle): {error}"
        ))),
    }
}

/// Quiesce and retire only session-core-owned resources. Dialog-core's dialog
/// and exact server transaction are intentionally absent from this function;
/// the negative processing ACK hands them back to the lower classified 503
/// fallback, which retires them only after Written/WireUnknown.
async fn release_failed_inbound_upper_resources_once(
    store: Arc<crate::session_store::SessionStore>,
    helpers: Arc<StateMachineHelpers>,
    dialog_adapter: Arc<DialogAdapter>,
    media_adapter: Arc<MediaAdapter>,
    handle: SessionRegistryHandle,
) -> SessionResult<()> {
    if store.get_session_retained_snapshot_exact(&handle).is_err() {
        return Ok(());
    }
    if let Err(error) = store.quiesce_session_exact(&handle).await {
        if store.get_session_retained_snapshot_exact(&handle).is_err() {
            return Ok(());
        }
        return Err(SessionError::InternalError(format!(
            "failed inbound INVITE quiesce failed (class=lifecycle): {error}"
        )));
    }

    dialog_adapter
        .cleanup_failed_inbound_session_preserving_lower_route_exact(&handle)
        .await?;
    media_adapter.cleanup_session_exact(&handle).await?;
    helpers.cleanup_session(handle.session_id()).await;
    remove_quiesced_failed_inbound_store_lifetime(store.as_ref(), &handle)
}

async fn release_failed_inbound_upper_resources_with_retry(
    store: Arc<crate::session_store::SessionStore>,
    helpers: Arc<StateMachineHelpers>,
    dialog_adapter: Arc<DialogAdapter>,
    media_adapter: Arc<MediaAdapter>,
    handle: SessionRegistryHandle,
) -> SessionResult<()> {
    let first = release_failed_inbound_upper_resources_once(
        Arc::clone(&store),
        Arc::clone(&helpers),
        Arc::clone(&dialog_adapter),
        Arc::clone(&media_adapter),
        handle.clone(),
    )
    .await;
    if first.is_ok() {
        return first;
    }
    warn!(
        session = %handle.session_id(),
        "failed inbound INVITE upper cleanup failed; retrying the same exact lifetime"
    );
    tokio::task::yield_now().await;
    release_failed_inbound_upper_resources_once(
        store,
        helpers,
        dialog_adapter,
        media_adapter,
        handle,
    )
    .await
}

async fn author_exact_final_response(
    dialog_api: Arc<rvoip_sip_dialog::api::UnifiedDialogApi>,
    transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
    status: rvoip_sip_core::StatusCode,
    extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
) -> ExactFinalResponseOutcome {
    let mut response = match dialog_api
        .build_response(&transaction_id, status, None)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            debug!(
                method = %transaction_id.method(),
                status_code = status.as_u16(),
                %error,
                "Exact final response could not be built before any wire write"
            );
            return ExactFinalResponseOutcome::ZeroWire;
        }
    };
    response.headers.extend(extra_headers);
    let outcome = match dialog_api
        .send_response_classified(&transaction_id, response)
        .await
    {
        Ok(disposition) => exact_final_response_outcome(disposition),
        Err(error) => {
            let outcome = exact_final_response_outcome(error.disposition);
            debug!(
                method = %transaction_id.method(),
                status_code = status.as_u16(),
                ?outcome,
                source = %error.source,
                "Exact final response completed with a classified transport error"
            );
            outcome
        }
    };
    if exact_final_response_retires_routes(outcome) {
        if let Err(error) = dialog_api.retire_terminal_response_pending_index(&transaction_id) {
            debug!(
                method = %transaction_id.method(),
                status_code = status.as_u16(),
                %error,
                "Terminal exact response could not retire its pending dialog pointer"
            );
        }
    }
    outcome
}

fn terminal_template_counts(result: &ProcessEventResult) -> (usize, usize, usize) {
    let failed = result
        .events_published
        .iter()
        .filter(|event| matches!(event, EventTemplate::CallFailed))
        .count();
    let cancelled = result
        .events_published
        .iter()
        .filter(|event| matches!(event, EventTemplate::CallCancelled))
        .count();
    let terminated = result
        .events_published
        .iter()
        .filter(|event| matches!(event, EventTemplate::CallTerminated))
        .count();
    (failed, cancelled, terminated)
}

fn executed_action_count(
    result: &ProcessEventResult,
    expected: &crate::state_table::Action,
) -> usize {
    result
        .actions_executed
        .iter()
        .filter(|action| *action == expected)
        .count()
}

fn committed_bye_termination(
    session_id: &SessionId,
    result: &ProcessEventResult,
) -> SessionResult<()> {
    if result.transition.is_none() {
        return Err(SessionError::InvalidTransition(format!(
            "DialogBYE for session {} in state {:?} had no YAML transition",
            session_id, result.old_state
        )));
    }
    let (failed, cancelled, terminated) = terminal_template_counts(result);
    let dialog_cleanup = executed_action_count(result, &crate::state_table::Action::CleanupDialog);
    let media_cleanup = executed_action_count(result, &crate::state_table::Action::CleanupMedia);
    if (failed, cancelled, terminated, result.next_state) == (0, 0, 1, Some(CallState::Terminated))
        && dialog_cleanup == 1
        && media_cleanup == 1
    {
        return Ok(());
    }
    Err(SessionError::InvalidTransition(format!(
        "DialogBYE for session {} state {:?} did not commit exactly one CallTerminated YAML outcome with exact dialog/media cleanup (next={:?}, failed_events={}, cancelled_events={}, terminated_events={}, dialog_cleanup={}, media_cleanup={})",
        session_id,
        result.old_state,
        result.next_state,
        failed,
        cancelled,
        terminated,
        dialog_cleanup,
        media_cleanup
    )))
}

fn committed_call_failure(
    session_id: &SessionId,
    role: Role,
    result: &ProcessEventResult,
) -> SessionResult<CommittedCallFailure> {
    if result.transition.is_none() {
        return Err(SessionError::InvalidTransition(format!(
            "CallFailed for session {} in state {:?} had no YAML transition",
            session_id, result.old_state
        )));
    }
    let (failed, cancelled, terminated) = terminal_template_counts(result);
    let effective_state = result.next_state.unwrap_or(result.old_state);
    let clear_pending_reinvite =
        executed_action_count(result, &crate::state_table::Action::ClearPendingReinvite);
    let dialog_cleanup = executed_action_count(result, &crate::state_table::Action::CleanupDialog);
    let media_cleanup = executed_action_count(result, &crate::state_table::Action::CleanupMedia);
    match (failed, cancelled, terminated, effective_state) {
        (1, 0, 0, CallState::Failed(_)) if dialog_cleanup == 1 && media_cleanup == 1 => {
            Ok(CommittedCallFailure::Failed)
        }
        (0, 1, 0, CallState::Cancelled)
            if role == Role::UAC
                && result.old_state == CallState::CancelPending
                && dialog_cleanup == 1
                && media_cleanup == 1 =>
        {
            Ok(CommittedCallFailure::Cancelled)
        }
        (0, 0, 0, state)
            if matches!(
                (result.old_state, state),
                (CallState::HoldPending, CallState::Active)
                    | (CallState::Resuming, CallState::OnHold)
                    | (CallState::Active, CallState::Active)
            ) && clear_pending_reinvite == 1
                && dialog_cleanup == 0
                && media_cleanup == 0 =>
        {
            Ok(CommittedCallFailure::NonTerminal)
        }
        _ => Err(SessionError::InvalidTransition(format!(
            "CallFailed for session {} role {:?} state {:?} did not commit one matching terminal YAML outcome with exact cleanup or an exact re-INVITE rollback (next={:?}, failed_events={}, cancelled_events={}, terminated_events={}, clear_pending_reinvite={}, dialog_cleanup={}, media_cleanup={})",
            session_id,
            role,
            result.old_state,
            result.next_state,
            failed,
            cancelled,
            terminated,
            clear_pending_reinvite,
            dialog_cleanup,
            media_cleanup
        ))),
    }
}

fn committed_session_interval_retry(
    session_id: &SessionId,
    role: Role,
    result: &ProcessEventResult,
) -> SessionResult<()> {
    let (failed, cancelled, terminated) = terminal_template_counts(result);
    let retry_actions = executed_action_count(
        result,
        &crate::state_table::Action::SendINVITEWithBumpedSessionExpires,
    );
    if result.transition.is_some()
        && role == Role::UAC
        && result.old_state == CallState::Initiating
        && result.next_state == Some(CallState::Initiating)
        && (failed, cancelled, terminated) == (0, 0, 0)
        && retry_actions == 1
    {
        return Ok(());
    }
    Err(SessionError::InvalidTransition(format!(
        "SessionIntervalTooSmall retry for session {} role {:?} state {:?} did not commit the exact non-terminal YAML retry outcome (next={:?}, failed_events={}, cancelled_events={}, terminated_events={}, retry_actions={})",
        session_id,
        role,
        result.old_state,
        result.next_state,
        failed,
        cancelled,
        terminated,
        retry_actions
    )))
}

fn committed_dialog_200(
    session_id: &SessionId,
    role: Role,
    result: &ProcessEventResult,
) -> SessionResult<CommittedDialog200> {
    if result.transition.is_none() {
        return Err(SessionError::InvalidTransition(format!(
            "Dialog200OK for session {} in state {:?} had no YAML transition",
            session_id, result.old_state
        )));
    }
    let initial_answer_events = result
        .events_published
        .iter()
        .filter(|event| {
            matches!(event, EventTemplate::CallEstablished)
                || matches!(event, EventTemplate::Custom(name) if name == "CallAnswered")
        })
        .count();
    let (failed, cancelled, terminated) = terminal_template_counts(result);
    let initial_state = matches!(
        result.old_state,
        CallState::Initiating | CallState::Ringing | CallState::EarlyMedia
    );
    if role == Role::UAC
        && initial_state
        && result.next_state == Some(CallState::Active)
        && initial_answer_events == 1
        && (failed, cancelled, terminated) == (0, 0, 0)
    {
        return Ok(CommittedDialog200::InitialAnswer);
    }
    if role == Role::UAC && initial_state {
        return Err(SessionError::InvalidTransition(format!(
            "Dialog200OK for initial session {} state {:?} did not commit exactly one initial-answer YAML template (next={:?}, initial_answer_events={})",
            session_id, result.old_state, result.next_state, initial_answer_events
        )));
    }
    let effective_state = result.next_state.unwrap_or(result.old_state);
    if initial_answer_events == 0
        && (failed, cancelled, terminated) == (0, 0, 0)
        && !effective_state.is_final()
    {
        return Ok(CommittedDialog200::NonInitial);
    }
    Err(SessionError::InvalidTransition(format!(
        "Dialog200OK for session {} role {:?} state {:?} did not commit a valid initial-answer or non-initial YAML outcome (next={:?}, initial_answer_events={}, failed_events={}, cancelled_events={}, terminated_events={})",
        session_id,
        role,
        result.old_state,
        result.next_state,
        initial_answer_events,
        failed,
        cancelled,
        terminated
    )))
}

/// Classify only a terminal fact committed by the exact YAML transition.
///
/// A lower dialog termination is not itself authority to publish or release a
/// session. The role/state-qualified transition must both reach the matching
/// terminal state and publish exactly one matching terminal template. This
/// keeps malformed/custom tables and stale duplicate facts fail-closed.
fn committed_dialog_termination(
    session_id: &SessionId,
    role: Role,
    result: &ProcessEventResult,
) -> SessionResult<CommittedDialogTermination> {
    if result.transition.is_none() {
        return Err(SessionError::InvalidTransition(format!(
            "DialogTerminated for session {} in state {:?} had no YAML transition",
            session_id, result.old_state
        )));
    }

    let (failed, cancelled, terminated) = terminal_template_counts(result);

    match (failed, terminated, cancelled, result.next_state) {
        (0, 1, 0, Some(CallState::Terminated)) => Ok(CommittedDialogTermination::Ended),
        (0, 0, 1, Some(CallState::Cancelled))
            if role == Role::UAC && result.old_state == CallState::Cancelling =>
        {
            Ok(CommittedDialogTermination::Cancelled)
        }
        _ => Err(SessionError::InvalidTransition(format!(
            "DialogTerminated for session {} role {:?} state {:?} did not commit exactly one matching terminal YAML outcome (next={:?}, failed_events={}, terminated_events={}, cancelled_events={})",
            session_id,
            role,
            result.old_state,
            result.next_state,
            failed,
            terminated,
            cancelled
        ))),
    }
}

fn registry_has_exact_dialog(
    registry: &SessionRegistry,
    dialog_id: &rvoip_sip_dialog::DialogId,
) -> bool {
    let exact_dialog_id: DialogId = dialog_id.clone().into();
    registry
        .get_handle_by_dialog_exact(&exact_dialog_id)
        .is_some()
}

/// Complete non-indexed state for the first visible revision of an inbound
/// INVITE lifetime. The exact dialog and SIP Call-ID identity is committed
/// together only after the lower dialog has been revalidated.
struct InboundInviteInitialState {
    local_uri: String,
    remote_uri: String,
    received_at: Instant,
    transaction: rvoip_sip_dialog::transaction::TransactionKey,
    remote_sdp: Option<String>,
}

impl InboundInviteInitialState {
    fn apply(self, session: &mut crate::session_store::SessionState) {
        session.local_uri = Some(self.local_uri);
        session.remote_uri = Some(self.remote_uri);
        session.incoming_invite_received_at = Some(self.received_at);
        session.pending_inbound_invite_transaction_id = Some(self.transaction);
        session.remote_sdp = self.remote_sdp;
    }
}

async fn wait_for_owned_operation_cancellation(cancel: &mut tokio::sync::watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

async fn rollback_owned_delayed_signaling<T>(
    operation: OwnedOperation,
    value: T,
) -> OwnedOperationCompletion<T> {
    operation
        .rollback(value)
        .await
        .unwrap_or_else(|_| panic!("delayed signaling exact rollback failed"))
}

/// Commit a retained signaling operation without hiding a lifecycle commit
/// failure from the causal dialog ingress. The returned value becomes the
/// processing ACK observed by dialog-core; a failed commit therefore gives
/// the lower exact transaction owner permission to run its one classified
/// fallback.
async fn commit_owned_delayed_signaling_ack(
    operation: OwnedOperation,
) -> OwnedOperationCompletion<std::result::Result<(), String>> {
    match operation.commit() {
        Ok(committed) => committed.complete(Ok(())),
        Err(failure) => {
            let detail = format!("{failure:?}");
            rollback_owned_delayed_signaling(
                failure.into_operation(),
                Err(format!(
                    "retained REFER default action could not commit exact lifecycle ownership: {detail}"
                )),
            )
            .await
        }
    }
}

#[cfg(test)]
fn refer_default_is_pending_exact(
    store: &crate::session_store::SessionStore,
    handle: &SessionRegistryHandle,
    transaction_id: &str,
) -> bool {
    store
        .get_session_snapshot_exact(handle)
        .is_ok_and(|snapshot| snapshot.refer_transaction_id.as_deref() == Some(transaction_id))
}

/// Owns a state-machine task that performs outbound signaling.
///
/// Dropping Tokio's `JoinHandle` detaches the task. Keep an armed owner around
/// the await instead so dispatcher shutdown or cancellation also cancels the
/// state-machine task and its signaling work.
struct AbortStateMachineTaskOnDrop<T> {
    handle: tokio::task::JoinHandle<T>,
    armed: bool,
}

impl<T> AbortStateMachineTaskOnDrop<T> {
    fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self {
            handle,
            armed: true,
        }
    }

    async fn join(mut self) -> std::result::Result<T, tokio::task::JoinError> {
        let result = (&mut self.handle).await;
        self.armed = false;
        result
    }
}

impl<T> Drop for AbortStateMachineTaskOnDrop<T> {
    fn drop(&mut self) {
        if self.armed {
            self.handle.abort();
        }
    }
}

async fn join_state_machine_task(
    task: AbortStateMachineTaskOnDrop<StateMachineProcessResult>,
) -> StateMachineProcessResult {
    task.join().await.map_err(|_| {
        Box::new(SessionError::InternalError(
            STATE_MACHINE_DISPATCH_JOIN_FAILURE.to_string(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?
}

/// Poll a signaling state-machine future from the root of a fresh Tokio task
/// while retaining strict per-session ordering in the caller.
///
/// Dialog events already enter through a sharded worker, but that worker polls
/// the complete cross-crate handler before it reaches `process_event`. The
/// resulting transport → dialog → session → state-machine → outbound signaling
/// poll chain can exhaust the default stack. Awaiting this owned child task lets
/// the parent poll unwind before the signaling action is polled; it does not
/// detach the action or weaken completion/error semantics.
async fn process_event_exact_on_fresh_task(
    state_machine: Arc<StateMachineExecutor>,
    handle: SessionRegistryHandle,
    event: EventType,
) -> StateMachineProcessResult {
    let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async move {
        state_machine.process_event_exact(&handle, event).await
    }));
    join_state_machine_task(task).await
}

async fn process_inbound_response_event_exact_on_fresh_task(
    state_machine: Arc<StateMachineExecutor>,
    handle: SessionRegistryHandle,
    event: EventType,
    inbound_response: InboundResponseStateInput,
) -> StateMachineProcessResult {
    let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async move {
        state_machine
            .process_inbound_response_event_exact(&handle, event, inbound_response)
            .await
    }));
    join_state_machine_task(task).await
}

async fn process_event_with_remote_sdp_exact_on_fresh_task(
    state_machine: Arc<StateMachineExecutor>,
    handle: SessionRegistryHandle,
    event: EventType,
    remote_sdp: Option<String>,
) -> StateMachineProcessResult {
    let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async move {
        state_machine
            .process_event_with_remote_sdp_exact(&handle, event, remote_sdp)
            .await
    }));
    join_state_machine_task(task).await
}

async fn process_invite_2xx_answer_exact_on_fresh_task(
    state_machine: Arc<StateMachineExecutor>,
    handle: SessionRegistryHandle,
    remote_sdp: Option<String>,
    ack: Invite2xxAckStateInput,
) -> StateMachineProcessResult {
    let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async move {
        state_machine
            .process_invite_2xx_answer_exact(&handle, remote_sdp, ack)
            .await
    }));
    join_state_machine_task(task).await
}

async fn process_auth_required_on_fresh_task(
    state_machine: Arc<StateMachineExecutor>,
    handle: SessionRegistryHandle,
    status_code: u16,
    challenge: String,
    method: String,
    auth_required: AuthRequiredStateInput,
) -> std::result::Result<AuthRequiredProcessOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async move {
        state_machine
            .process_auth_required_exact(&handle, status_code, challenge, method, auth_required)
            .await
    }));
    task.join().await.map_err(|_| {
        Box::new(SessionError::InternalError(
            STATE_MACHINE_DISPATCH_JOIN_FAILURE.to_string(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?
}

fn is_missing_credentials_for_auth_error(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    matches!(
        error.downcast_ref::<SessionError>(),
        Some(SessionError::MissingCredentialsForInviteAuth)
            | Some(SessionError::MissingCredentialsForRequestAuth { .. })
    )
}

fn is_session_lifecycle_capacity_exhaustion(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    matches!(
        error.downcast_ref::<SessionAdmissionError>(),
        Some(
            SessionAdmissionError::CapacityExhausted
                | SessionAdmissionError::RetainedCapacityExhausted
        )
    )
}

fn safe_auth_method_label(method: &str) -> &'static str {
    match method.trim().to_ascii_uppercase().as_str() {
        "INVITE" => "INVITE",
        "REGISTER" => "REGISTER",
        "BYE" => "BYE",
        "REFER" => "REFER",
        "NOTIFY" => "NOTIFY",
        "INFO" => "INFO",
        "UPDATE" => "UPDATE",
        "MESSAGE" => "MESSAGE",
        "OPTIONS" => "OPTIONS",
        "SUBSCRIBE" => "SUBSCRIBE",
        _ => "extension",
    }
}

fn outbound_request_outcome_label(outcome: OutboundRequestOutcome) -> &'static str {
    match outcome {
        OutboundRequestOutcome::FinalResponse { .. } => "final-response",
        OutboundRequestOutcome::Timeout => "timeout",
        OutboundRequestOutcome::TransportFailure => "transport-failure",
    }
}

fn reinvite_completion_failure(
    outcome: OutboundRequestOutcome,
) -> Option<(EventType, &'static str)> {
    match outcome {
        OutboundRequestOutcome::FinalResponse { status_code }
            if (200..300).contains(&status_code) || status_code == 491 =>
        {
            None
        }
        OutboundRequestOutcome::FinalResponse { status_code } if status_code < 500 => Some((
            EventType::Dialog4xxFailure(status_code),
            "re-INVITE was rejected",
        )),
        OutboundRequestOutcome::FinalResponse { status_code } if status_code < 600 => Some((
            EventType::Dialog5xxFailure(status_code),
            "re-INVITE failed with a server error",
        )),
        OutboundRequestOutcome::FinalResponse { status_code } => Some((
            EventType::Dialog6xxFailure(status_code),
            "re-INVITE failed globally",
        )),
        OutboundRequestOutcome::Timeout => {
            Some((EventType::DialogTimeout, "re-INVITE transaction timed out"))
        }
        OutboundRequestOutcome::TransportFailure => {
            Some((EventType::DialogTimeout, "re-INVITE transport failed"))
        }
    }
}

fn update_completion_transition(
    outcome: OutboundRequestOutcome,
) -> (EventType, Option<&'static str>) {
    match outcome {
        OutboundRequestOutcome::FinalResponse { status_code }
            if (200..300).contains(&status_code) =>
        {
            (EventType::Dialog200OK, None)
        }
        OutboundRequestOutcome::FinalResponse { status_code } if status_code < 500 => (
            EventType::Dialog4xxFailure(status_code),
            Some("UPDATE was rejected"),
        ),
        OutboundRequestOutcome::FinalResponse { status_code } if status_code < 600 => (
            EventType::Dialog5xxFailure(status_code),
            Some("UPDATE failed with a server error"),
        ),
        OutboundRequestOutcome::FinalResponse { status_code } => (
            EventType::Dialog6xxFailure(status_code),
            Some("UPDATE failed globally"),
        ),
        OutboundRequestOutcome::Timeout => (
            EventType::DialogTimeout,
            Some("UPDATE transaction timed out"),
        ),
        OutboundRequestOutcome::TransportFailure => {
            (EventType::DialogTimeout, Some("UPDATE transport failed"))
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingOfferTransactionCorrelation {
    NoPendingOffer,
    OtherMethod,
    Exact,
    Mismatched,
}

fn correlate_pending_offer_transaction(
    pending_method: Option<&rvoip_sip_core::Method>,
    pending_transaction: Option<&rvoip_sip_dialog::transaction::TransactionKey>,
    method: &rvoip_sip_core::Method,
    transaction: Option<&rvoip_sip_dialog::transaction::TransactionKey>,
) -> PendingOfferTransactionCorrelation {
    let Some(pending_method) = pending_method else {
        return PendingOfferTransactionCorrelation::NoPendingOffer;
    };
    if pending_method != method {
        return PendingOfferTransactionCorrelation::OtherMethod;
    }
    if pending_transaction == transaction && transaction.is_some() {
        PendingOfferTransactionCorrelation::Exact
    } else {
        PendingOfferTransactionCorrelation::Mismatched
    }
}

fn invite_success_is_retransmission(
    dialog_established: bool,
    correlation: PendingOfferTransactionCorrelation,
) -> bool {
    dialog_established && correlation == PendingOfferTransactionCorrelation::NoPendingOffer
}

fn initial_invite_used_delayed_offer(snapshot: &SessionStateSnapshot) -> bool {
    snapshot.role == Role::UAC
        && snapshot
            .pending_invite_options
            .as_ref()
            .and_then(|options| options.sdp.as_deref())
            .is_some_and(|sdp| sdp.trim().is_empty())
}

fn response_from_bytes(raw_response: Option<&bytes::Bytes>) -> Option<rvoip_sip_core::Response> {
    match rvoip_sip_core::parse_message(raw_response?.as_ref()).ok()? {
        rvoip_sip_core::Message::Response(response) => Some(response),
        rvoip_sip_core::Message::Request(_) => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutboundAuthTerminalClass {
    MissingCredentials,
    ChallengeResponse,
    RetryLimit,
    StateMachine,
}

impl OutboundAuthTerminalClass {
    fn from_error(error: &(dyn std::error::Error + Send + Sync + 'static)) -> Self {
        match error.downcast_ref::<SessionError>() {
            Some(
                SessionError::MissingCredentialsForInviteAuth
                | SessionError::MissingCredentialsForRequestAuth { .. },
            ) => Self::MissingCredentials,
            Some(
                SessionError::InviteAuthConstructionFailed
                | SessionError::RequestAuthConstructionFailed
                | SessionError::RegisterAuthConstructionFailed
                | SessionError::AuthError(_),
            ) => Self::ChallengeResponse,
            Some(
                SessionError::InviteAuthRetryExhausted
                | SessionError::RequestAuthRetryExhausted { .. },
            ) => Self::RetryLimit,
            _ => Self::StateMachine,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::MissingCredentials => "missing-credentials",
            Self::ChallengeResponse => "challenge-response",
            Self::RetryLimit => "retry-limit",
            Self::StateMachine => "state-machine",
        }
    }

    fn invite_reason(self) -> String {
        format!("INVITE authentication failed (class={})", self.label())
    }
}

enum CallFailureReason {
    Protocol(String),
    OutboundInviteAuth(OutboundAuthTerminalClass),
}

impl CallFailureReason {
    fn into_event_reason(self) -> String {
        match self {
            Self::Protocol(reason) => reason,
            Self::OutboundInviteAuth(class) => class.invite_reason(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CallFailureDiagnostics {
    reason_present: bool,
    reason_bytes: usize,
}

impl CallFailureDiagnostics {
    fn new(reason: &str) -> Self {
        Self {
            reason_present: !reason.is_empty(),
            reason_bytes: reason.len(),
        }
    }
}

/// Window within which repeated RFC 5626 flow-failure events for the
/// same AoR collapse to a single re-REGISTER. Matches the guidance in
/// RFC 5626 §4.4.1 (flow recovery should not storm the registrar).
const OUTBOUND_FLOW_REFRESH_DEBOUNCE: Duration = Duration::from_secs(1);

fn transaction_id_diagnostics(value: &str) -> (&'static str, usize) {
    let class = if value
        .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
        .is_ok()
    {
        "valid"
    } else {
        "invalid"
    };
    (class, value.len())
}

fn sip_trace_owner_matches(configured_owner_id: Option<&str>, event_owner_id: &str) -> bool {
    configured_owner_id.is_some_and(|owner_id| owner_id == event_owner_id)
}

fn map_sip_trace_session_id(
    event: &SipTraceEvent,
    store: &crate::session_store::SessionStore,
) -> Option<SessionId> {
    event
        .session_id
        .as_ref()
        .map(|id| SessionId(id.clone()))
        .or_else(|| {
            event.sip_call_id.as_ref().and_then(|sip_call_id| {
                let handle = store
                    .registry()
                    .get_handle_by_sip_call_id_exact(sip_call_id)?;
                store
                    .get_session_snapshot_exact(&handle)
                    .ok()
                    .map(|_| handle.session_id().clone())
            })
        })
}

pub(crate) fn dialog_dispatch_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_mul(4))
        .unwrap_or(16)
        .clamp(8, 64)
}

fn session_dispatch_shard(session_id: &str, shard_count: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    session_id.hash(&mut hasher);
    (hasher.finish() as usize) % shard_count.max(1)
}

fn shutdown_change_requests_stop(
    changed: std::result::Result<(), tokio::sync::watch::error::RecvError>,
    receiver: &tokio::sync::watch::Receiver<bool>,
) -> bool {
    changed.is_err() || *receiver.borrow()
}

/// One immutable projection input admitted only after the exact state-machine
/// commit is visible. The legacy/detailed event order is intentionally part of
/// this compatibility boundary.
enum CommittedResponseObservation {
    Progress {
        status_code: u16,
        reason: String,
        sdp: Option<String>,
        raw_response: Option<bytes::Bytes>,
    },
    Established {
        sdp: Option<String>,
        raw_response: Option<bytes::Bytes>,
    },
    Failed {
        status_code: u16,
        reason: String,
        raw_response: Option<bytes::Bytes>,
    },
}

fn project_committed_response_events(
    committed: &SessionStateSnapshot,
    observation: CommittedResponseObservation,
) -> [crate::api::events::Event; 2] {
    let call_id = committed.session_id.clone();
    match observation {
        CommittedResponseObservation::Progress {
            status_code,
            reason,
            sdp,
            raw_response,
        } => {
            let detailed = build_incoming_response_from_bytes(
                call_id.clone(),
                status_code,
                reason.clone(),
                sdp.clone(),
                raw_response,
            );
            [
                crate::api::events::Event::CallProgress {
                    call_id,
                    status_code,
                    reason,
                    sdp,
                },
                crate::api::events::Event::CallProgressDetailed(detailed),
            ]
        }
        CommittedResponseObservation::Established { sdp, raw_response } => {
            let detailed = build_incoming_response_from_bytes(
                call_id.clone(),
                200,
                "OK".to_string(),
                sdp.clone(),
                raw_response,
            );
            [
                crate::api::events::Event::CallAnswered { call_id, sdp },
                crate::api::events::Event::CallEstablishedDetailed(detailed),
            ]
        }
        CommittedResponseObservation::Failed {
            status_code,
            reason,
            raw_response,
        } => {
            let detailed = build_incoming_response_from_bytes(
                call_id.clone(),
                status_code,
                reason.clone(),
                None,
                raw_response,
            );
            [
                crate::api::events::Event::CallFailedDetailed(detailed),
                crate::api::events::Event::CallFailed {
                    call_id,
                    status_code,
                    reason,
                },
            ]
        }
    }
}

enum QueuedDialogPayload {
    Dialog(DialogToSessionEvent),
    Deferred(DeferredReplayDelivery),
}

struct DeferredReplayDelivery {
    tracker: OutboundInDialogRequestTracker,
    event: Option<DeferredTrackedRequestEvent>,
}

struct StartedDeferredReplay {
    tracker: OutboundInDialogRequestTracker,
    event: DeferredTrackedRequestEvent,
}

impl StartedDeferredReplay {
    fn event(&self) -> &DeferredTrackedRequestEvent {
        &self.event
    }
}

impl Drop for StartedDeferredReplay {
    fn drop(&mut self) {
        self.tracker.abort_started_replay(&self.event);
    }
}

impl DeferredReplayDelivery {
    fn new(tracker: OutboundInDialogRequestTracker, event: DeferredTrackedRequestEvent) -> Self {
        Self {
            tracker,
            event: Some(event),
        }
    }

    fn begin(mut self) -> Option<StartedDeferredReplay> {
        let event = self.event.as_ref()?;
        let accepted = self.tracker.mark_deferred_replay_started(event);
        let event = self.event.take();
        accepted.then(|| StartedDeferredReplay {
            tracker: self.tracker.clone(),
            event: event.expect("accepted delivery retains its event"),
        })
    }
}

impl Drop for DeferredReplayDelivery {
    fn drop(&mut self) {
        if let Some(event) = self.event.take() {
            self.tracker.abort_deferred_replay(&event);
        }
    }
}

struct QueuedDialogToSessionEvent {
    payload: QueuedDialogPayload,
    queued_at: Instant,
    kind: &'static str,
    route_key: Option<String>,
    /// Exact lifetime captured synchronously at causal admission. `None` is
    /// reserved for initial INVITE admission and transaction/flow events that
    /// do not address an existing session.
    exact_handle: Option<SessionRegistryHandle>,
    authoritative_completion: Option<oneshot::Sender<std::result::Result<(), String>>>,
}

struct ServerCallAdmissionGuard {
    pending: Arc<AtomicUsize>,
}

impl Drop for ServerCallAdmissionGuard {
    fn drop(&mut self) {
        self.pending.fetch_sub(1, Ordering::Relaxed);
    }
}

enum ServerCallAdmissionDecision {
    Admit(Option<ServerCallAdmissionGuard>),
    Reject {
        observed_sessions: usize,
        hard_limit: usize,
    },
}

#[derive(Clone)]
struct DialogToSessionDirectRouter {
    shard_senders: Arc<Vec<mpsc::Sender<QueuedDialogToSessionEvent>>>,
    fallback_shard: Arc<AtomicUsize>,
    deferred_tracker: OutboundInDialogRequestTracker,
    registration_adapter: Arc<OnceLock<Arc<RegistrationAdapter>>>,
    registration_response_owner: Option<Arc<DialogAdapter>>,
    store: Arc<crate::session_store::SessionStore>,
}

fn capture_dialog_ingress_handle(
    store: &crate::session_store::SessionStore,
    event: &DialogToSessionEvent,
    route_key: Option<&str>,
) -> SessionResult<Option<SessionRegistryHandle>> {
    if matches!(event, DialogToSessionEvent::IncomingCall { .. }) {
        // The initial INVITE is the one routed event that creates its exact
        // lifetime, so no handle can exist at admission.
        return Ok(None);
    }
    if matches!(
        event,
        DialogToSessionEvent::MessageReceived { session_id, .. } if session_id.is_empty()
    ) {
        // Standalone MESSAGE is transaction-owned and deliberately has no
        // artificial state-machine session lifetime.
        return Ok(None);
    }
    let Some(route_key) = route_key else {
        // DialogCreated, IncomingRegister, OutboundFlowFailed and out-of-
        // dialog OPTIONS are transaction/flow observations, not mutations of
        // an existing session lifetime.
        return Ok(None);
    };
    let session_id = SessionId(route_key.to_string());
    store
        .lifecycle_handle(&session_id)
        .map(Some)
        .ok_or_else(|| {
            SessionError::InvalidTransition(
                "dialog ingress did not resolve an exact live session".to_string(),
            )
        })
}

fn require_dialog_event_handle<'a>(
    exact_handle: Option<&'a SessionRegistryHandle>,
    session_id: &str,
) -> Result<&'a SessionRegistryHandle> {
    exact_handle
        .filter(|handle| handle.session_id().0 == session_id)
        .ok_or_else(|| anyhow::anyhow!("typed dialog event lost exact session authority"))
}

fn queued_dialog_lifetime_is_current(
    store: &crate::session_store::SessionStore,
    exact_handle: Option<&SessionRegistryHandle>,
) -> bool {
    exact_handle.is_none_or(|handle| store.get_session_snapshot_exact(handle).is_ok())
}

fn stale_dialog_dispatch_result(kind: &'static str) -> Result<()> {
    if matches!(
        kind,
        "info_received" | "reinvite_received" | "transfer_requested"
    ) {
        Err(anyhow::anyhow!(
            "{kind} exact session lifetime ended before response ownership was accepted"
        ))
    } else {
        // Late terminal/progress/deferred observations are idempotent no-ops.
        Ok(())
    }
}

fn media_observation_session_id(event: &MediaToSessionEvent) -> &str {
    match event {
        MediaToSessionEvent::MediaStreamStarted { session_id, .. }
        | MediaToSessionEvent::MediaStreamStopped { session_id, .. }
        | MediaToSessionEvent::MediaQualityUpdate { session_id, .. }
        | MediaToSessionEvent::RecordingStarted { session_id, .. }
        | MediaToSessionEvent::RecordingStopped { session_id, .. }
        | MediaToSessionEvent::AudioPlaybackFinished { session_id }
        | MediaToSessionEvent::MediaError { session_id, .. }
        | MediaToSessionEvent::MediaFlowEstablished { session_id }
        | MediaToSessionEvent::MediaQualityDegraded { session_id, .. }
        | MediaToSessionEvent::DtmfDetected { session_id, .. }
        | MediaToSessionEvent::RtpTimeout { session_id, .. }
        | MediaToSessionEvent::PacketLossThresholdExceeded { session_id, .. } => session_id,
    }
}

/// Project media-core reports onto the existing public observation surface.
/// This function deliberately cannot dispatch a state-machine event: media
/// resource actions and exact lifecycle watchdogs are the causal authorities.
fn media_observation_api_event(event: &MediaToSessionEvent) -> Option<crate::api::events::Event> {
    match event {
        MediaToSessionEvent::MediaQualityUpdate {
            session_id,
            quality_metrics,
        }
        | MediaToSessionEvent::MediaQualityDegraded {
            session_id,
            metrics: quality_metrics,
            ..
        } => Some(crate::api::events::Event::MediaQualityChanged {
            call_id: SessionId(session_id.clone()),
            packet_loss_percent: (quality_metrics.packet_loss * 100.0) as u32,
            jitter_ms: quality_metrics.jitter_ms as u32,
        }),
        _ => None,
    }
}

async fn dispatch_queued_dialog_payload(
    handler: &SessionCrossCrateEventHandler,
    payload: QueuedDialogPayload,
    exact_handle: Option<&SessionRegistryHandle>,
) -> Result<()> {
    match payload {
        QueuedDialogPayload::Dialog(event) => {
            handler
                .handle_dialog_to_session_event(&event, exact_handle)
                .await
        }
        QueuedDialogPayload::Deferred(delivery) => match delivery.begin() {
            Some(started) => {
                let event = started.event().clone();
                let result = handler.handle_deferred_tracked_request(event).await;
                drop(started);
                result
            }
            None => Ok(()),
        },
    }
}

/// Stable typed dialog-to-session ingress registered before the SIP transport
/// stack is created.
///
/// Integrated startup installs the existing sharded router into this slot
/// before returning the coordinator. If loopback traffic arrives while the
/// remaining session components are still being assembled, its causal
/// publisher waits here instead of falling through to the observational bus
/// or losing the first event.
#[derive(Clone)]
pub(crate) struct CausalDialogToSessionIngress {
    target: Arc<OnceLock<Arc<dyn CrossCrateEventHandler>>>,
    startup: Arc<StdMutex<()>>,
    closed: Arc<AtomicBool>,
    changed: Arc<tokio::sync::Notify>,
}

impl CausalDialogToSessionIngress {
    pub(crate) fn new() -> Self {
        Self {
            target: Arc::new(OnceLock::new()),
            startup: Arc::new(StdMutex::new(())),
            closed: Arc::new(AtomicBool::new(false)),
            changed: Arc::new(tokio::sync::Notify::new()),
        }
    }

    fn install(&self, router: DialogToSessionDirectRouter) -> SessionResult<()> {
        self.install_target(Arc::new(router))
    }

    fn install_target(&self, target: Arc<dyn CrossCrateEventHandler>) -> SessionResult<()> {
        let _startup = self
            .startup
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.closed.load(Ordering::Acquire) {
            return Err(SessionError::InternalError(
                "causal dialog-to-session ingress closed before router installation".to_string(),
            ));
        }
        self.target.set(target).map_err(|_| {
            SessionError::InternalError(
                "causal dialog-to-session ingress already has a router".to_string(),
            )
        })?;
        self.changed.notify_waiters();
        Ok(())
    }

    /// Release any early publisher if coordinator construction exits before
    /// the real router can be installed.
    pub(crate) fn close_pending(&self) {
        let _startup = self
            .startup
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.target.get().is_none() {
            self.closed.store(true, Ordering::Release);
            self.changed.notify_waiters();
        }
    }

    async fn target(&self) -> Result<Arc<dyn CrossCrateEventHandler>> {
        loop {
            if let Some(target) = self.target.get() {
                return Ok(Arc::clone(target));
            }
            // Register the waiter before examining the slot so an install
            // racing this check cannot lose its wakeup.
            let changed = self.changed.notified();
            if let Some(target) = self.target.get() {
                return Ok(Arc::clone(target));
            }
            if self.closed.load(Ordering::Acquire) {
                return Err(anyhow::anyhow!(
                    "causal dialog-to-session ingress closed before readiness"
                ));
            }
            changed.await;
        }
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for CausalDialogToSessionIngress {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        self.target().await?.handle(event).await
    }
}

impl DialogToSessionDirectRouter {
    fn new(
        handler: SessionCrossCrateEventHandler,
        worker_count: usize,
        queue_capacity: usize,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> SessionResult<Self> {
        let deferred_tracker = handler.dialog_adapter.outbound_request_tracker.clone();
        let registration_adapter = Arc::clone(&handler.registration_adapter);
        let registration_response_owner = Some(Arc::clone(&handler.dialog_adapter));
        let retained_tasks = Arc::clone(&handler.retained_tasks);
        let store = Arc::clone(&handler.state_machine.store);
        let per_shard_capacity = (queue_capacity / worker_count.max(1)).max(1);
        let mut shard_senders = Vec::with_capacity(worker_count);

        for shard in 0..worker_count {
            let (tx, mut rx) = mpsc::channel::<QueuedDialogToSessionEvent>(per_shard_capacity);
            let handler_for_shard = handler.clone();
            let mut shutdown = shutdown_rx.clone();
            if !retained_tasks.spawn(async move {
                let mut draining = false;
                loop {
                    tokio::select! {
                        biased;
                        changed = shutdown.changed(), if !draining => {
                            if shutdown_change_requests_stop(changed, &shutdown) {
                                info!(
                                    shard,
                                    queued = rx.len(),
                                    "🔔 [session_event_handler] Direct dialog-to-session shard draining"
                                );
                                // Reject new enqueue operations, then run every
                                // accepted envelope to completion. In particular,
                                // an authoritative ByeReceived acknowledgment may
                                // never be discarded at the shutdown boundary.
                                rx.close();
                                draining = true;
                            }
                        }
                        queued = rx.recv() => {
                            let Some(queued) = queued else { break };
                            let QueuedDialogToSessionEvent {
                                payload,
                                queued_at,
                                kind,
                                route_key,
                                exact_handle,
                                authoritative_completion,
                            } = queued;
                            let queue_delay = queued_at.elapsed();
                            cleanup_diag::record_queue_depth(
                                CleanupStage::SessionEventDispatch,
                                rx.len(),
                            );
                            rvoip_sip_dialog::diagnostics::record_dialog_to_session_queue_delay(
                                kind,
                                queue_delay,
                            );

                            let label = route_key
                                .as_deref()
                                .unwrap_or(kind);
                            let dispatch_guard =
                                cleanup_diag::stage_guard(CleanupStage::SessionEventDispatch, label);
                            let exact_lifetime_current = queued_dialog_lifetime_is_current(
                                handler_for_shard.state_machine.store.as_ref(),
                                exact_handle.as_ref(),
                            );
                            let result = if !exact_lifetime_current {
                                debug!(
                                    shard,
                                    kind,
                                    route_key = route_key.as_deref().unwrap_or("<none>"),
                                    "Ignoring queued dialog event after exact session retirement"
                                );
                                drop(payload);
                                stale_dialog_dispatch_result(kind)
                            } else {
                                dispatch_queued_dialog_payload(
                                    &handler_for_shard,
                                    payload,
                                    exact_handle.as_ref(),
                                )
                                .await
                            };
                            if let Some(completion) = authoritative_completion {
                                let acknowledgement = result
                                    .as_ref()
                                    .map(|_| ())
                                    .map_err(ToString::to_string);
                                // Cancellation of the publisher after enqueue
                                // does not cancel processing of the accepted
                                // event; it only means nobody awaits this ACK.
                                let _ = completion.send(acknowledgement);
                            }
                            match result {
                                Ok(()) => dispatch_guard.finish_success(),
                                Err(e) => {
                                    error!(
                                        shard,
                                        kind,
                                        "Error handling direct dialog-to-session event: {}",
                                        e
                                    );
                                    dispatch_guard.finish_failure();
                                }
                            }
                        }
                    }
                    if draining && rx.is_empty() {
                        break;
                    }
                }
            }) {
                return Err(SessionError::InternalError(
                    "dialog-to-session retained task admission is closed".to_string(),
                ));
            }
            shard_senders.push(tx);
        }

        info!(
            workers = worker_count,
            per_shard_capacity,
            "🔔 [session_event_handler] Registered direct dialog-to-session dispatcher"
        );

        Ok(Self {
            shard_senders: Arc::new(shard_senders),
            fallback_shard: Arc::new(AtomicUsize::new(0)),
            deferred_tracker,
            registration_adapter,
            registration_response_owner,
            store,
        })
    }

    fn shard_for(&self, route_key: Option<&str>) -> usize {
        match route_key {
            Some(session_id) => session_dispatch_shard(session_id, self.shard_senders.len()),
            None => self.fallback_shard.fetch_add(1, Ordering::Relaxed) % self.shard_senders.len(),
        }
    }

    async fn enqueue(&self, queued: QueuedDialogToSessionEvent) -> Result<()> {
        let shard = self.shard_for(queued.route_key.as_deref());
        match self.shard_senders[shard].try_send(queued) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(queued)) => {
                warn!(
                    shard,
                    kind = queued.kind,
                    route_key = queued.route_key.as_deref().unwrap_or("<none>"),
                    "Direct dialog-to-session shard is full; applying backpressure"
                );
                cleanup_diag::record_queue_depth(
                    CleanupStage::SessionEventDispatch,
                    self.shard_senders[shard].max_capacity(),
                );
                self.shard_senders[shard]
                    .send(queued)
                    .await
                    .map_err(|e| anyhow::anyhow!("dialog-to-session shard closed: {}", e))
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                Err(anyhow::anyhow!("dialog-to-session shard is closed"))
            }
        }
    }

    fn enqueue_authoritative(&self, queued: QueuedDialogToSessionEvent) -> Result<()> {
        let shard = self.shard_for(queued.route_key.as_deref());
        self.shard_senders[shard]
            .try_send(queued)
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    anyhow::anyhow!("authoritative dialog-to-session shard is full")
                }
                mpsc::error::TrySendError::Closed(_) => {
                    anyhow::anyhow!("authoritative dialog-to-session shard is closed")
                }
            })
    }

    async fn handle_deferred(&self, event: DeferredTrackedRequestEvent) -> Result<()> {
        let route_key = Some(event.session_id().0.clone());
        let exact_handle = Some(event.handle().clone());
        let kind = match &event {
            DeferredTrackedRequestEvent::AuthRequired { .. } => "deferred_auth_required",
            DeferredTrackedRequestEvent::Completed { .. } => "deferred_request_completed",
        };
        self.enqueue(QueuedDialogToSessionEvent {
            payload: QueuedDialogPayload::Deferred(DeferredReplayDelivery::new(
                self.deferred_tracker.clone(),
                event,
            )),
            queued_at: Instant::now(),
            kind,
            route_key,
            exact_handle,
            authoritative_completion: None,
        })
        .await
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for DialogToSessionDirectRouter {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        if let Some(install) = event.as_any().downcast_ref::<RegistrationAdapterInstall>() {
            let response_owner = self.registration_response_owner.as_ref().ok_or_else(|| {
                anyhow::anyhow!("authoritative REGISTER handler has no dialog response owner")
            })?;
            let adapter = install.adapter();
            adapter.install_registration_response_owner(Arc::clone(response_owner))?;
            return self.registration_adapter.set(adapter).map_err(|_| {
                anyhow::anyhow!("authoritative REGISTER handler already has a registration adapter")
            });
        }

        let wrapped = event
            .as_any()
            .downcast_ref::<RvoipCrossCrateEvent>()
            .ok_or_else(|| anyhow::anyhow!("typed dialog ingress received a foreign event"))?;
        let RvoipCrossCrateEvent::DialogToSession(typed) = wrapped else {
            return Err(anyhow::anyhow!(
                "typed dialog ingress received a non-dialog event"
            ));
        };
        let kind = dialog_to_session_event_kind(typed);
        let route_key = RoutableEvent::session_id(wrapped).map(ToOwned::to_owned);
        let exact_handle =
            match capture_dialog_ingress_handle(self.store.as_ref(), typed, route_key.as_deref()) {
                Ok(handle) => handle,
                Err(_) => {
                    debug!(
                        kind,
                        route_key = route_key.as_deref().unwrap_or("<none>"),
                        "Ignoring dialog ingress without an exact live session"
                    );
                    if matches!(
                        typed,
                        DialogToSessionEvent::InfoReceived { .. }
                            | DialogToSessionEvent::ReinviteReceived { .. }
                            | DialogToSessionEvent::TransferRequested { .. }
                    ) {
                        // These requests are response-bearing: accepting one
                        // without a live exact owner would strand its server
                        // transaction. Let dialog-core select its one failure
                        // response instead.
                        return Err(anyhow::anyhow!(
                            "response-bearing dialog request has no exact live session owner"
                        ));
                    }
                    // Late terminal/progress duplicates, including an already-
                    // completed BYE cleanup notification, are idempotent no-ops.
                    // Propagating an error would make dialog-core retry cleanup
                    // even though no session lifetime remains to mutate.
                    return Ok(());
                }
            };
        let authoritative = dialog_event_requires_processing_ack(typed);
        let (authoritative_completion, acknowledgement) = if authoritative {
            let (completion, acknowledgement) = oneshot::channel();
            (Some(completion), Some(acknowledgement))
        } else {
            (None, None)
        };
        let queued = QueuedDialogToSessionEvent {
            payload: QueuedDialogPayload::Dialog(typed.clone()),
            queued_at: Instant::now(),
            kind,
            route_key,
            exact_handle,
            authoritative_completion,
        };
        if kind == "info_received" {
            self.enqueue_authoritative(queued)?;
        } else {
            self.enqueue(queued).await?;
        }
        let Some(acknowledgement) = acknowledgement else {
            return Ok(());
        };
        match acknowledgement.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(anyhow::anyhow!(error)),
            Err(_) => Err(anyhow::anyhow!(
                "authoritative dialog-to-session shard closed before acknowledgment"
            )),
        }
    }
}

fn dialog_event_requires_processing_ack(event: &DialogToSessionEvent) -> bool {
    matches!(
        event,
        DialogToSessionEvent::IncomingCall { .. }
            | DialogToSessionEvent::IncomingRegister { .. }
            | DialogToSessionEvent::ByeReceived { .. }
            | DialogToSessionEvent::InfoReceived { .. }
            | DialogToSessionEvent::ReinviteReceived { .. }
            | DialogToSessionEvent::TransferRequested { .. }
    )
}

fn dialog_to_session_event_kind(event: &DialogToSessionEvent) -> &'static str {
    match event {
        DialogToSessionEvent::IncomingCall { .. } => "incoming_call",
        DialogToSessionEvent::AckReceived { .. } => "ack_received",
        DialogToSessionEvent::ByeReceived { .. } => "bye_received",
        DialogToSessionEvent::InfoReceived { .. } => "info_received",
        DialogToSessionEvent::ReinviteReceived { .. } => "reinvite_received",
        DialogToSessionEvent::TransferRequested { .. } => "transfer_requested",
        DialogToSessionEvent::CallTerminated { .. } => "call_terminated",
        DialogToSessionEvent::CallFailed { .. } => "call_failed",
        DialogToSessionEvent::CallCancelled { .. } => "call_cancelled",
        _ => "dialog_to_session_other",
    }
}

/// Handler for processing cross-crate events in rvoip-sip
#[derive(Clone)]
pub struct SessionCrossCrateEventHandler {
    /// State machine executor
    state_machine: Arc<StateMachineExecutor>,

    /// Helper-owned auxiliary state released with the exact session lifetime.
    /// Integrated startup injects the coordinator's canonical helper owner;
    /// standalone construction retains a helper over the same executor.
    helpers: Arc<StateMachineHelpers>,

    /// Coordinator-owned task retention for causal shard parents and terminal
    /// release children. Standalone construction owns a private registry.
    retained_tasks: Arc<RetainedTasks>,

    /// Global event coordinator
    global_coordinator: Arc<GlobalEventCoordinator>,

    /// Dialog adapter for setting up backward compatibility channels
    dialog_adapter: Arc<DialogAdapter>,

    /// Media adapter for setting up backward compatibility channels
    media_adapter: Arc<MediaAdapter>,

    /// Session registry for mappings
    registry: Arc<SessionRegistry>,

    /// Channel to send incoming call notifications
    incoming_call_tx: Option<mpsc::Sender<crate::types::IncomingCallInfo>>,

    /// Immediately accept inbound calls after the state machine records them.
    fast_auto_accept_incoming_calls: bool,

    /// Config-owned cap for server-side inbound call admission.
    server_call_admission_limit: Option<usize>,

    /// Soft threshold where server-side admission starts pacing.
    server_call_admission_soft_limit: Option<usize>,

    /// Delay used while above the soft threshold and below hard overload.
    server_call_admission_pacing_delay_ms: Option<u64>,

    /// Retry-After seconds for SIP overload rejections.
    server_overload_retry_after_secs: Option<u32>,

    /// Hysteresis state: once hard overload is reached, reject until below soft.
    server_call_admission_overloaded: Arc<AtomicBool>,

    /// Inbound INVITEs admitted but not yet inserted into the session store.
    server_call_admission_pending: Arc<AtomicUsize>,

    /// Serializes admission check/reserve so the hard limit is meaningful with
    /// multiple dialog-to-session workers.
    server_call_admission_lock: Arc<Mutex<()>>,

    /// Total capacity for the direct dialog-to-session dispatcher queues.
    dialog_event_dispatch_queue_capacity: usize,

    /// Pre-registered causal ingress used by integrated startup. Lower-level
    /// constructors leave this empty and register the router when `start`
    /// runs, preserving their existing behavior.
    causal_dialog_ingress: Option<CausalDialogToSessionIngress>,

    /// Internal state-machine event stream owned by rvoip-sip.
    state_machine_event_rx:
        Option<Arc<Mutex<mpsc::Receiver<crate::state_machine::executor::SessionEvent>>>>,

    /// Optional built-in registrar installed through the existing
    /// authoritative dialog-to-session handler. The shared once-cell keeps a
    /// strong owner across all sharded handler clones and rejects duplicate
    /// installations before they could produce duplicate REGISTER responses.
    registration_adapter: Arc<OnceLock<Arc<RegistrationAdapter>>>,

    /// Last RFC 5626 `OutboundFlowFailed`-driven refresh per AoR, used
    /// to debounce storms of pong-timeout / connection-closed events
    /// (multiple transport signals can observe the same underlying
    /// failure within a handful of milliseconds). Entries live
    /// indefinitely — this map grows with the number of unique AoRs
    /// the peer has ever registered, which in practice is 1.
    outbound_flow_last_refresh: Arc<DashMap<String, Instant>>,

    /// App-level event publisher. Updates lifecycle before global bus delivery.
    app_event_publisher: SessionEventPublisher,

    /// Optional owner id for SIP trace events emitted by this coordinator's transport stack.
    sip_trace_owner_id: Option<String>,

    /// SIP_API_DESIGN_2 Phase D — weak handle back to the
    /// `UnifiedCoordinator` so the typed `IncomingRegister`
    /// construction can supply `RegisterResponseBuilder` with the
    /// coordinator hook it needs to publish responses back to
    /// dialog-core. `Weak` breaks the circular ownership
    /// (coordinator -> handler -> coordinator). Populated after
    /// construction via [`Self::set_coordinator`]; cloning the handler
    /// shares the underlying once-cell.
    coordinator: Arc<std::sync::OnceLock<std::sync::Weak<crate::api::unified::UnifiedCoordinator>>>,
}

impl SessionCrossCrateEventHandler {
    async fn send_retained_info_failure_response(
        &self,
        transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
    ) -> Result<()> {
        let dialog_api = Arc::clone(&self.dialog_adapter.dialog_api);
        let completion = spawn_retained_exact_response_completion(
            &self.retained_tasks,
            author_exact_final_response(
                dialog_api,
                transaction_id,
                rvoip_sip_core::StatusCode::ServerInternalError,
                Vec::new(),
            ),
        )?;
        let outcome = completion.await.map_err(|_| {
            anyhow::anyhow!("inbound INFO failure-response child ended without completion")
        })?;
        // ZeroWire produces the negative processing ACK that lets dialog-core
        // own one safe classified fallback. WireUnknown is terminal success:
        // retrying here could write a second final response.
        exact_final_response_result("inbound INFO 500 response", outcome)
    }

    /// Reject an INFO that could not be delivered to its application owner.
    /// The response obligation is claimed before the retained classified send
    /// so the coordinator deadline path cannot become a second writer.
    async fn send_retained_info_control_rejection(
        &self,
        transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
        obligation: Arc<crate::api::incoming::ExactResponseObligation>,
    ) -> Result<()> {
        let claim = obligation.claim()?;
        let dialog_api = Arc::clone(&self.dialog_adapter.dialog_api);
        let completion = match spawn_retained_exact_response_completion(
            &self.retained_tasks,
            author_exact_final_response(
                dialog_api,
                transaction_id,
                rvoip_sip_core::StatusCode::ServiceUnavailable,
                Vec::new(),
            ),
        ) {
            Ok(completion) => completion,
            Err(error) => {
                // Application delivery is already closed. Retire its
                // coordinator obligation before the negative ACK transfers
                // response authority back to dialog-core.
                claim.complete();
                return Err(error.into());
            }
        };
        let outcome = match completion.await {
            Ok(outcome) => outcome,
            Err(_) => {
                claim.complete();
                return Err(anyhow::anyhow!(
                    "inbound INFO 503 response child ended without completion"
                ));
            }
        };
        // The application never received this request, so its deadline-owned
        // response obligation must end for every classification. On ZeroWire,
        // the negative causal ACK transfers the sole retry to dialog-core.
        claim.complete();
        // Written and wire-unknown are terminal transaction facts. Only a
        // zero-wire result returns a negative causal ACK to dialog-core.
        exact_final_response_result("inbound INFO control-delivery 503 response", outcome)
    }

    async fn handle_deferred_tracked_request(
        &self,
        event: DeferredTrackedRequestEvent,
    ) -> Result<()> {
        match event {
            DeferredTrackedRequestEvent::AuthRequired {
                handle,
                transaction_id,
                request_uri,
                status,
                challenge,
                method,
                outbound_transport,
            } => {
                let session_id = handle.session_id().clone();
                self.handle_auth_required_parts(
                    session_id,
                    transaction_id,
                    request_uri,
                    status,
                    challenge,
                    method,
                    outbound_transport,
                    handle,
                    true,
                )
                .await
            }
            DeferredTrackedRequestEvent::Completed {
                handle,
                transaction_id,
                method,
                outcome,
                response_sdp,
            } => {
                let session_id = handle.session_id().clone();
                self.handle_outbound_request_completed_parts(
                    session_id,
                    &transaction_id,
                    &method,
                    outcome,
                    response_sdp,
                    handle,
                    true,
                )
                .await
            }
        }
    }

    async fn handle_dialog_to_session_event(
        &self,
        event: &DialogToSessionEvent,
        exact_handle: Option<&SessionRegistryHandle>,
    ) -> Result<()> {
        match event {
            DialogToSessionEvent::DialogCreated { dialog_id, call_id } => {
                self.handle_dialog_created_parts(dialog_id.clone(), call_id.clone())
                    .await
            }
            DialogToSessionEvent::IncomingCall {
                session_id,
                call_id,
                from,
                to,
                sdp_offer,
                headers,
                transaction_id,
                source_addr,
                raw_request,
                transport,
                identity_verification: _,
            } => {
                self.handle_incoming_call_parts(
                    session_id.clone(),
                    call_id.clone(),
                    from.clone(),
                    to.clone(),
                    sdp_offer.clone(),
                    headers,
                    transaction_id,
                    source_addr,
                    raw_request.clone(),
                    transport.clone(),
                )
                .await
            }
            DialogToSessionEvent::CallStateChanged {
                session_id,
                new_state,
                ..
            } => {
                self.handle_call_state_changed_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    new_state,
                )
                .await
            }
            DialogToSessionEvent::CallProgress {
                session_id,
                status_code,
                reason_phrase,
                sdp,
                raw_response,
            } => {
                self.handle_call_progress_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    *status_code,
                    reason_phrase.clone(),
                    sdp.clone(),
                    raw_response.clone(),
                )
                .await
            }
            DialogToSessionEvent::CallEstablished {
                session_id,
                sdp_answer,
                raw_response,
            } => {
                self.handle_call_established_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    sdp_answer.clone(),
                    raw_response.clone(),
                )
                .await
            }
            DialogToSessionEvent::ByeReceived { session_id } => {
                self.handle_bye_received_parts(require_dialog_event_handle(
                    exact_handle,
                    session_id,
                )?)
                .await
            }
            DialogToSessionEvent::CallTerminated { session_id, reason } => {
                self.handle_call_terminated_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    termination_reason_to_string(reason),
                )
                .await
            }
            DialogToSessionEvent::CallFailed {
                session_id,
                status_code,
                reason_phrase,
                raw_response,
            } => {
                self.handle_call_failed_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    *status_code,
                    CallFailureReason::Protocol(reason_phrase.clone()),
                    raw_response.clone(),
                )
                .await
            }
            DialogToSessionEvent::CallCancelled { session_id } => {
                self.handle_call_cancelled_session(require_dialog_event_handle(
                    exact_handle,
                    session_id,
                )?)
                .await
            }
            DialogToSessionEvent::SessionRefreshed {
                session_id,
                expires_secs,
            } => {
                self.handle_session_refreshed_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    *expires_secs,
                )
                .await
            }
            DialogToSessionEvent::SessionRefreshFailed { session_id, reason } => {
                self.handle_session_refresh_failed_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    reason.clone(),
                )
                .await
            }
            DialogToSessionEvent::AuthRequired {
                session_id,
                transaction_id,
                request_uri,
                status_code,
                challenge,
                method,
                outbound_transport,
                ..
            } => {
                self.handle_auth_required_parts(
                    SessionId(session_id.clone()),
                    transaction_id.clone(),
                    request_uri.clone(),
                    *status_code,
                    challenge.clone(),
                    method.clone(),
                    outbound_transport.clone(),
                    exact_handle.cloned().ok_or_else(|| {
                        anyhow::anyhow!("AuthRequired ingress lost exact session authority")
                    })?,
                    false,
                )
                .await
            }
            DialogToSessionEvent::OutboundRequestCompleted {
                session_id,
                transaction_id,
                method,
                outcome,
                response_sdp,
            } => {
                self.handle_outbound_request_completed_parts(
                    SessionId(session_id.clone()),
                    transaction_id,
                    method,
                    *outcome,
                    response_sdp.clone(),
                    exact_handle.cloned().ok_or_else(|| {
                        anyhow::anyhow!(
                            "OutboundRequestCompleted ingress lost exact session authority"
                        )
                    })?,
                    false,
                )
                .await
            }
            DialogToSessionEvent::CallRedirected {
                session_id,
                status_code,
                targets,
                q_values,
            } => {
                self.handle_call_redirected_typed(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    *status_code,
                    targets,
                    q_values,
                )
                .await
            }
            DialogToSessionEvent::ReinviteGlare { session_id } => {
                self.handle_reinvite_glare_session(require_dialog_event_handle(
                    exact_handle,
                    session_id,
                )?)
                .await
            }
            DialogToSessionEvent::SessionIntervalTooSmall {
                session_id,
                min_se_secs,
            } => {
                self.handle_session_interval_too_small_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    *min_se_secs,
                )
                .await
            }
            DialogToSessionEvent::DtmfReceived { session_id, tones } => {
                self.handle_dtmf_received_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    tones.clone(),
                )
                .await
            }
            DialogToSessionEvent::DialogError {
                session_id, error, ..
            } => {
                self.handle_dialog_error_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    error.clone(),
                )
                .await
            }
            DialogToSessionEvent::DialogStateChanged {
                session_id,
                old_state,
                new_state,
            } => {
                self.handle_dialog_state_changed_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    format!("{:?}", old_state),
                    format!("{:?}", new_state),
                )
                .await
            }
            DialogToSessionEvent::ReinviteReceived {
                session_id,
                sdp,
                method,
                raw_request,
                transport,
            } => {
                let sid = SessionId(session_id.clone());
                let handle = require_dialog_event_handle(exact_handle, session_id)?;
                let inbound_response =
                    derive_inbound_response_state_input(method, raw_request.as_ref())?;
                // SIP_API_DESIGN_2 Phase E: surface UPDATE separately
                // via `Event::UpdateReceived` so subscribers can
                // distinguish a re-INVITE from an UPDATE without
                // string-matching on `method`. INVITE keeps the
                // legacy hold/resume state-machine path.
                if method.eq_ignore_ascii_case("UPDATE") {
                    if let Some(incoming) = build_incoming_request_from_bytes(
                        sid.clone(),
                        raw_request.clone(),
                        transport.clone(),
                    ) {
                        self.app_event_publisher.publish_exact(
                            handle,
                            crate::api::events::Event::UpdateReceived {
                                call_id: sid.clone(),
                                request: incoming,
                            },
                        );
                    }
                }
                self.handle_reinvite_received_parts(
                    handle,
                    sdp.clone(),
                    method.clone(),
                    inbound_response,
                )
                .await
            }
            DialogToSessionEvent::TransferRequested {
                session_id,
                refer_to,
                transfer_type,
                transaction_id,
                referred_by,
                replaces,
                raw_request,
                transport,
            } => {
                self.handle_transfer_requested_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    refer_to.clone(),
                    transfer_type_to_string(transfer_type),
                    transaction_id.clone(),
                    referred_by.clone(),
                    replaces.clone(),
                    raw_request.clone(),
                    transport.clone(),
                )
                .await
            }
            DialogToSessionEvent::AckReceived { session_id, sdp } => {
                self.handle_ack_received_session(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    sdp.clone(),
                )
                .await
            }
            DialogToSessionEvent::RegistrationSuccess { session_id } => {
                self.handle_registration_success_parts(require_dialog_event_handle(
                    exact_handle,
                    session_id,
                )?)
                .await
            }
            DialogToSessionEvent::RegistrationFailed {
                session_id,
                status_code,
            } => {
                self.handle_registration_failed_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    *status_code,
                )
                .await
            }
            DialogToSessionEvent::SubscriptionAccepted { session_id } => {
                self.handle_state_event_if_ours(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    EventType::SubscriptionAccepted,
                    "SubscriptionAccepted",
                )
                .await
            }
            DialogToSessionEvent::SubscriptionFailed {
                session_id,
                status_code,
            } => {
                self.handle_state_event_if_ours(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    EventType::SubscriptionFailed(*status_code),
                    "SubscriptionFailed",
                )
                .await
            }
            DialogToSessionEvent::NotifyReceived {
                session_id,
                event_package,
                subscription_state,
                content_type,
                body,
                raw_request,
                transport,
            } => {
                if raw_request.is_none() {
                    tracing::warn!(
                        "NotifyReceived cross-crate bridge: raw_request was None — \
                         upstream publish site did not preserve NOTIFY bytes for \
                         session {}",
                        session_id
                    );
                }
                self.handle_notify_received_parts(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    event_package.clone(),
                    subscription_state.clone(),
                    content_type.clone(),
                    body.clone(),
                    raw_request.clone(),
                    transport.clone(),
                )
                .await
            }
            DialogToSessionEvent::MessageDelivered { session_id } => {
                self.handle_state_event_if_ours(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    EventType::MessageDelivered,
                    "MessageDelivered",
                )
                .await
            }
            DialogToSessionEvent::MessageFailed {
                session_id,
                status_code,
            } => {
                self.handle_state_event_if_ours(
                    require_dialog_event_handle(exact_handle, session_id)?,
                    EventType::MessageFailed(*status_code),
                    "MessageFailed",
                )
                .await
            }
            DialogToSessionEvent::IncomingRegister {
                transaction_id,
                from_uri,
                to_uri,
                contact_uri,
                expires,
                authorization,
                call_id,
                raw_request,
                transport,
            } => {
                // The optional built-in registrar is a direct consumer of the
                // authoritative typed request. Keep its result until after the
                // public projection is emitted so custom registrar
                // applications retain the existing IncomingRegister surface
                // even when the built-in path reports a protocol error.
                let (built_in_owner, registration_result) = match self.registration_adapter.get() {
                    Some(adapter) => {
                        let adapter = Arc::clone(adapter);
                        let retained_event = event.clone();
                        let completion =
                            spawn_retained_register_processing(&self.retained_tasks, async move {
                                adapter
                                    .handle_incoming_register_event(&retained_event)
                                    .await
                            })?;
                        (
                            true,
                            completion.await.map_err(|_| {
                                anyhow::anyhow!(
                                    "built-in REGISTER response task ended without completion"
                                )
                            })?,
                        )
                    }
                    None => (false, Ok(())),
                };

                // SIP_API_DESIGN_2 Phase D — surface inbound REGISTER as a
                // typed `IncomingRegister` so registrar applications can
                // author responses via `accept_builder()` / `challenge_builder()`
                // / `reject_builder()`. When the typed event carries the
                // original wire bytes, re-parse them once into an `Arc<Request>`
                // for typed-header inspection; otherwise fall through to
                // the synthesized compatibility view.
                let coordinator = self.coordinator.get().and_then(|w| w.upgrade());
                let parsed_request: Option<Arc<rvoip_sip_core::Request>> =
                    raw_request.as_ref().and_then(|bytes| {
                        match rvoip_sip_core::parse_message(bytes.as_ref()) {
                            Ok(rvoip_sip_core::Message::Request(req)) => Some(Arc::new(req)),
                            _ => None,
                        }
                    });

                let register = match (parsed_request, coordinator.as_ref()) {
                    (Some(req), Some(coord)) => {
                        crate::api::incoming::IncomingRegister::with_request_and_coordinator(
                            transaction_id.clone(),
                            from_uri.clone(),
                            to_uri.clone(),
                            contact_uri.clone(),
                            *expires,
                            authorization.clone(),
                            call_id.clone(),
                            req,
                            Arc::clone(coord),
                        )
                    }
                    (Some(req), None) => crate::api::incoming::IncomingRegister::with_request(
                        transaction_id.clone(),
                        from_uri.clone(),
                        to_uri.clone(),
                        contact_uri.clone(),
                        *expires,
                        authorization.clone(),
                        call_id.clone(),
                        req,
                    ),
                    (None, _) => crate::api::incoming::IncomingRegister::synthetic(
                        transaction_id.clone(),
                        from_uri.clone(),
                        to_uri.clone(),
                        contact_uri.clone(),
                        *expires,
                        authorization.clone(),
                        call_id.clone(),
                    ),
                };
                let mut register =
                    register.with_transport_context(sip_transport_context(transport));
                if let Some(coordinator) = coordinator {
                    register.set_coordinator(coordinator);
                }
                if built_in_owner {
                    // The built-in adapter has already completed the sole
                    // classified response path. Preserve the public API event
                    // as observation only; a callback must not regain response
                    // authority by attaching its coordinator.
                    register.clear_response_capability();
                    publish_api_event(
                        &self.app_event_publisher,
                        crate::api::events::Event::IncomingRegister { register },
                    );
                    registration_result.map_err(Into::into)
                } else {
                    debug_assert!(registration_result.is_ok());
                    let coordinator = register.coordinator.clone().ok_or_else(|| {
                        anyhow::anyhow!("custom REGISTER response owner has no coordinator")
                    })?;
                    match register.install_response_obligation(coordinator)? {
                        crate::api::unified::ExactResponseRegistration::Registered => {}
                        crate::api::unified::ExactResponseRegistration::Closed => {
                            return Err(anyhow::anyhow!(
                                "custom REGISTER response registry is draining"
                            ));
                        }
                        crate::api::unified::ExactResponseRegistration::Collision => {
                            // A prior delivery/deadline already owns this exact
                            // transaction. Do not create a second app handler or
                            // transfer fallback authority back to dialog-core.
                            return Ok(());
                        }
                    }
                    let response_obligation =
                        register.response_obligation.clone().ok_or_else(|| {
                            anyhow::anyhow!("custom REGISTER lost its exact response obligation")
                        })?;
                    // Optional/custom registrar mode is a causal private
                    // control delivery. Positive ingress ACK means one bounded
                    // response-capable owner accepted the request, while its
                    // exact deadline owns the 503 fallback if the app drops it.
                    // Failed admission retires that deadline before returning a
                    // negative ACK so dialog-core is the only fallback writer.
                    match self
                        .app_event_publisher
                        .publish_control_now(crate::api::events::Event::IncomingRegister {
                            register,
                        })
                        .await
                    {
                        Ok(()) => Ok(()),
                        Err(error) => match response_obligation.claim() {
                            Ok(claim) => {
                                claim.complete();
                                Err(error.into())
                            }
                            Err(_) => {
                                // The managed fallback already claimed or
                                // completed the exact transaction.
                                Ok(())
                            }
                        },
                    }
                }
            }
            DialogToSessionEvent::OutboundFlowFailed { aor, reason, .. } => {
                self.handle_outbound_flow_failed_parts(aor.clone(), reason.clone())
                    .await
            }
            // SIP_API_DESIGN_2 Phase E — inbound mid-dialog INFO / MESSAGE / OPTIONS.
            // Each variant reaches session-core with the original
            // inbound bytes preserved; we re-parse them once via
            // `parse_message` into an `Arc<Request>` and surface a
            // typed `Event::*Received` carrying the `IncomingRequest`
            // view.
            DialogToSessionEvent::InfoReceived {
                session_id,
                transaction_id,
                raw_request,
                transport,
            } => {
                let handle = require_dialog_event_handle(exact_handle, session_id)?;
                if self
                    .state_machine
                    .store
                    .get_session_snapshot_exact(handle)
                    .is_err()
                {
                    return Err(anyhow::anyhow!(
                        "InfoReceived exact session lifetime ended before response ownership was accepted"
                    ));
                }
                if raw_request.is_none() {
                    tracing::warn!(
                        "InfoReceived cross-crate bridge: raw_request was None — \
                         upstream publish site did not preserve INFO bytes for \
                         session {}",
                        session_id
                    );
                }
                let sid = SessionId(session_id.clone());
                let Some(mut incoming) = build_incoming_request_from_bytes(
                    sid.clone(),
                    raw_request.clone(),
                    transport.clone(),
                ) else {
                    // Dialog-core already created this server transaction. If
                    // its preserved request cannot be reconstructed, fail it
                    // explicitly instead of leaving a non-INVITE transaction
                    // in Trying forever. The adapter boundary verifies that
                    // the carried transaction belongs to this session's
                    // dialog before it can write the response.
                    let failure_transaction = validated_inbound_info_event_transaction(
                        transaction_id,
                    )
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "unusable inbound INFO has no valid exact event transaction"
                        )
                    })?;
                    self.send_retained_info_failure_response(failure_transaction)
                        .await?;
                    return Ok(());
                };
                let exact_response_transaction = match incoming.raw_request() {
                    Some(request) => {
                        match correlate_inbound_info_transaction(transaction_id, request) {
                            Ok(transaction) => transaction,
                            Err(failure_transaction) => {
                                tracing::warn!(
                                    session_id = %sid,
                                    "Rejecting InfoReceived whose exact transaction does not match the wire request"
                                );
                                // The request-derived key is authoritative.
                                // Never fall back to the event key here: a
                                // stale same-dialog event could otherwise
                                // author a response on a different INFO.
                                let failure_transaction = failure_transaction.ok_or_else(|| {
                                    anyhow::anyhow!(
                                        "inbound INFO has no safe exact response transaction"
                                    )
                                })?;
                                self.send_retained_info_failure_response(failure_transaction)
                                    .await?;
                                return Ok(());
                            }
                        }
                    }
                    None => {
                        tracing::warn!(
                            session_id = %sid,
                            "Rejecting InfoReceived without an authoritative wire request"
                        );
                        return Err(anyhow::anyhow!(
                            "inbound INFO has no authoritative wire request"
                        ));
                    }
                };
                incoming.set_response_transaction(exact_response_transaction.clone());
                let coordinator = self
                    .coordinator
                    .get()
                    .and_then(std::sync::Weak::upgrade)
                    .ok_or_else(|| {
                        anyhow::anyhow!("inbound INFO response owner has no coordinator")
                    })?;
                match incoming.set_coordinator_captured(coordinator, Some((*handle).clone())) {
                    crate::api::unified::ExactResponseRegistration::Registered => {}
                    crate::api::unified::ExactResponseRegistration::Closed => {
                        tracing::warn!(
                            session_id = %sid,
                            "Rejecting inbound INFO while exact-response registry is draining"
                        );
                        return Err(anyhow::anyhow!(
                            "inbound INFO exact-response registry is draining"
                        ));
                    }
                    crate::api::unified::ExactResponseRegistration::Collision => {
                        tracing::warn!(
                            session_id = %sid,
                            "Discarding duplicate inbound INFO exact-response obligation"
                        );
                        return Ok(());
                    }
                }
                let rejected_delivery_response = incoming.clone();
                let event = crate::api::events::Event::InfoReceived {
                    call_id: sid.clone(),
                    request: incoming,
                };
                if let Err(error) = self
                    .app_event_publisher
                    .publish_control_exact_now(&handle, event)
                    .await
                {
                    tracing::warn!(
                        session_id = %sid,
                        "Inbound INFO application delivery was rejected: {}",
                        error
                    );
                    let obligation = rejected_delivery_response
                        .response_obligation
                        .clone()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "rejected inbound INFO has no exact response obligation"
                            )
                        })?;
                    self.send_retained_info_control_rejection(
                        exact_response_transaction,
                        obligation,
                    )
                    .await?;
                }
                Ok(())
            }
            DialogToSessionEvent::MessageReceived {
                session_id,
                raw_request,
                transport,
            } => {
                let lifecycle_handle = if !session_id.is_empty() {
                    let handle = require_dialog_event_handle(exact_handle, session_id)?;
                    if self
                        .state_machine
                        .store
                        .get_session_snapshot_exact(handle)
                        .is_err()
                    {
                        return Ok(());
                    }
                    Some(handle)
                } else {
                    None
                };
                if raw_request.is_none() {
                    tracing::warn!(
                        "MessageReceived cross-crate bridge: raw_request was None — \
                         upstream publish site did not preserve MESSAGE bytes for \
                         session {}",
                        session_id
                    );
                }
                let sid = SessionId(session_id.clone());
                if let Some(incoming) = build_incoming_request_from_bytes(
                    sid.clone(),
                    raw_request.clone(),
                    transport.clone(),
                ) {
                    let event = crate::api::events::Event::MessageReceived {
                        call_id: sid,
                        request: incoming,
                    };
                    if let Some(handle) = lifecycle_handle {
                        self.app_event_publisher.publish_exact(handle, event);
                    } else {
                        publish_api_event(&self.app_event_publisher, event);
                    }
                }
                Ok(())
            }
            DialogToSessionEvent::OptionsReceived {
                session_id,
                raw_request,
                transport,
            } => {
                let lifecycle_handle = if !session_id.is_empty() {
                    let handle = require_dialog_event_handle(exact_handle, session_id)?;
                    if self
                        .state_machine
                        .store
                        .get_session_snapshot_exact(handle)
                        .is_err()
                    {
                        return Ok(());
                    }
                    Some(handle)
                } else {
                    None
                };
                if raw_request.is_none() {
                    tracing::warn!(
                        "OptionsReceived cross-crate bridge: raw_request was None — \
                         upstream publish site did not preserve OPTIONS bytes for \
                         session {:?}",
                        session_id
                    );
                }
                // Out-of-dialog OPTIONS arrives with an empty
                // session_id; in-dialog OPTIONS rides the session
                // mapping established during INVITE.
                let sid_opt = if session_id.is_empty() {
                    None
                } else {
                    Some(SessionId(session_id.clone()))
                };
                let sid_for_request = sid_opt
                    .clone()
                    .unwrap_or_else(|| SessionId(String::from("options-oob")));
                if let Some(incoming) = build_incoming_request_from_bytes(
                    sid_for_request,
                    raw_request.clone(),
                    transport.clone(),
                ) {
                    let event = crate::api::events::Event::OptionsReceived {
                        call_id: sid_opt,
                        request: incoming,
                    };
                    if let Some(handle) = lifecycle_handle {
                        self.app_event_publisher.publish_exact(handle, event);
                    } else {
                        publish_api_event(&self.app_event_publisher, event);
                    }
                }
                Ok(())
            }
        }
    }

    async fn handle_media_to_session_event(&self, event: &MediaToSessionEvent) -> Result<()> {
        let session_id = SessionId(media_observation_session_id(event).to_string());
        if !self.is_our_session(&session_id) {
            debug!(
                session_id = %session_id,
                "Ignoring media observation for a session outside this coordinator"
            );
            return Ok(());
        }

        if let Some(api_event) = media_observation_api_event(event) {
            publish_api_event(&self.app_event_publisher, api_event);
        }

        match event {
            MediaToSessionEvent::MediaStreamStopped { reason, .. } => warn!(
                session_id = %session_id,
                reason_present = !reason.is_empty(),
                reason_bytes = reason.len(),
                "Observed media stream stop; synchronous media ownership remains authoritative"
            ),
            MediaToSessionEvent::MediaError {
                error, error_code, ..
            } => warn!(
                session_id = %session_id,
                error_present = !error.is_empty(),
                error_bytes = error.len(),
                error_code,
                "Observed media error; exact lifecycle watchdogs remain authoritative"
            ),
            MediaToSessionEvent::MediaQualityDegraded {
                metrics, severity, ..
            } => warn!(
                session_id = %session_id,
                packet_loss = metrics.packet_loss,
                jitter_ms = metrics.jitter_ms,
                ?severity,
                "Observed degraded media quality"
            ),
            MediaToSessionEvent::RtpTimeout {
                last_packet_time, ..
            } => warn!(
                session_id = %session_id,
                last_packet_time,
                "Observed RTP timeout; exact retained watchdog owns lifecycle policy"
            ),
            MediaToSessionEvent::PacketLossThresholdExceeded {
                loss_percentage, ..
            } => warn!(
                session_id = %session_id,
                loss_percentage,
                "Observed packet-loss threshold crossing"
            ),
            _ => debug!(
                session_id = %session_id,
                media_event = ?event,
                "Observed media event without a session lifecycle transition"
            ),
        }
        Ok(())
    }

    async fn handle_transport_to_session_event(&self, event: &SipTraceEvent) -> Result<()> {
        if !sip_trace_owner_matches(self.sip_trace_owner_id.as_deref(), &event.owner_id) {
            return Ok(());
        }

        let session_id = map_sip_trace_session_id(event, self.state_machine.store.as_ref());

        let trace = crate::api::events::SipTrace {
            direction: event.direction.clone(),
            transport: event.transport.clone(),
            local_addr: event.local_addr.clone(),
            remote_addr: event.remote_addr.clone(),
            timestamp_unix_millis: event.timestamp_unix_millis,
            start_line: event.start_line.clone(),
            sip_call_id: event.sip_call_id.clone(),
            session_id,
            raw_message: event.raw_message.clone(),
            original_len: event.original_len,
            truncated: event.truncated,
            redacted: event.redacted,
        };

        publish_api_event(
            &self.app_event_publisher,
            crate::api::events::Event::SipTrace(trace),
        );
        Ok(())
    }

    pub fn new(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
    ) -> Self {
        let helpers = Arc::new(StateMachineHelpers::new(Arc::clone(&state_machine)));
        Self {
            state_machine,
            helpers,
            retained_tasks: RetainedTasks::new(),
            global_coordinator: global_coordinator.clone(),
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx: None,
            fast_auto_accept_incoming_calls: false,
            server_call_admission_limit: None,
            server_call_admission_soft_limit: None,
            server_call_admission_pacing_delay_ms: None,
            server_overload_retry_after_secs: Some(1),
            server_call_admission_overloaded: Arc::new(AtomicBool::new(false)),
            server_call_admission_pending: Arc::new(AtomicUsize::new(0)),
            server_call_admission_lock: Arc::new(Mutex::new(())),
            dialog_event_dispatch_queue_capacity: 1024,
            causal_dialog_ingress: None,
            state_machine_event_rx: None,
            registration_adapter: Arc::new(OnceLock::new()),
            outbound_flow_last_refresh: Arc::new(DashMap::new()),
            app_event_publisher: SessionEventPublisher::new(
                global_coordinator.clone(),
                LifecycleIndex::new(),
            ),
            sip_trace_owner_id: None,
            coordinator: Arc::new(std::sync::OnceLock::new()),
        }
    }

    pub(crate) fn with_incoming_call_channel(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types::IncomingCallInfo>,
        app_event_publisher: SessionEventPublisher,
    ) -> Self {
        let helpers = Arc::new(StateMachineHelpers::new(Arc::clone(&state_machine)));
        Self {
            state_machine,
            helpers,
            retained_tasks: RetainedTasks::new(),
            global_coordinator: global_coordinator.clone(),
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx: Some(incoming_call_tx),
            fast_auto_accept_incoming_calls: false,
            server_call_admission_limit: None,
            server_call_admission_soft_limit: None,
            server_call_admission_pacing_delay_ms: None,
            server_overload_retry_after_secs: Some(1),
            server_call_admission_overloaded: Arc::new(AtomicBool::new(false)),
            server_call_admission_pending: Arc::new(AtomicUsize::new(0)),
            server_call_admission_lock: Arc::new(Mutex::new(())),
            dialog_event_dispatch_queue_capacity: 1024,
            causal_dialog_ingress: None,
            state_machine_event_rx: None,
            registration_adapter: Arc::new(OnceLock::new()),
            outbound_flow_last_refresh: Arc::new(DashMap::new()),
            app_event_publisher,
            sip_trace_owner_id: None,
            coordinator: Arc::new(std::sync::OnceLock::new()),
        }
    }

    /// Preferred constructor for UnifiedCoordinator. In addition to
    /// cross-crate bus subscriptions, this owns the internal state-machine
    /// event stream that publishes app-visible call state events.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn with_event_broadcast_and_state_machine_events(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types::IncomingCallInfo>,
        state_machine_event_rx: mpsc::Receiver<crate::state_machine::executor::SessionEvent>,
        app_event_publisher: SessionEventPublisher,
        sip_trace_owner_id: Option<String>,
    ) -> Self {
        let mut handler = Self::with_incoming_call_channel(
            state_machine,
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx,
            app_event_publisher,
        );
        handler.state_machine_event_rx = Some(Arc::new(Mutex::new(state_machine_event_rx)));
        handler.sip_trace_owner_id = sip_trace_owner_id;
        handler
    }

    /// Use the integrated coordinator's canonical helper state and retained
    /// task registry without changing the standalone/public constructor.
    pub(crate) fn with_runtime_owners(
        mut self,
        helpers: Arc<StateMachineHelpers>,
        retained_tasks: Arc<RetainedTasks>,
    ) -> Self {
        debug_assert!(Arc::ptr_eq(&self.state_machine, &helpers.state_machine));
        self.helpers = helpers;
        self.retained_tasks = retained_tasks;
        self
    }

    pub(crate) fn with_fast_auto_accept_incoming_calls(
        mut self,
        enabled: bool,
        queue_capacity: usize,
    ) -> Self {
        self.fast_auto_accept_incoming_calls = enabled;
        self.dialog_event_dispatch_queue_capacity = queue_capacity.max(1);
        self
    }

    pub(crate) fn with_server_call_admission(
        mut self,
        limit: Option<usize>,
        soft_limit: Option<usize>,
        pacing_delay_ms: Option<u64>,
        retry_after_secs: Option<u32>,
    ) -> Self {
        self.server_call_admission_limit = limit;
        self.server_call_admission_soft_limit = soft_limit;
        self.server_call_admission_pacing_delay_ms = pacing_delay_ms;
        self.server_overload_retry_after_secs = retry_after_secs;
        self
    }

    pub(crate) fn with_causal_dialog_ingress(
        mut self,
        ingress: CausalDialogToSessionIngress,
    ) -> Self {
        self.causal_dialog_ingress = Some(ingress);
        self
    }

    /// SIP_API_DESIGN_2 Phase D — pin the coordinator handle so the
    /// typed `IncomingRegister` branch can build a
    /// `RegisterResponseBuilder` that can publish responses back to
    /// dialog-core. Idempotent; subsequent calls are no-ops.
    pub(crate) fn set_coordinator(
        &self,
        coordinator: &Arc<crate::api::unified::UnifiedCoordinator>,
    ) {
        let _ = self.coordinator.set(Arc::downgrade(coordinator));
    }

    /// Record a terminal app-level observation, then release the session from
    /// the store + registry.
    ///
    /// Terminal events are `CallEnded`, `CallFailed`, `CallCancelled`. The
    /// lifecycle fact is committed and its bounded observational copy is
    /// offered first, but delivery is never awaited and does not retain the
    /// session for a subscriber. Exact release then runs in the same retained
    /// child task. The causal shard returns immediately while coordinator
    /// shutdown still joins the child before stopping dialog or publisher
    /// dependencies. Without this, long-running peers (and especially b2bua,
    /// which multiplies sessions) would leak `SessionStore` entries
    /// indefinitely.
    async fn publish_and_release_session(
        &self,
        api_event: crate::api::events::Event,
        handle: SessionRegistryHandle,
    ) {
        let session_id = handle.session_id().clone();
        let publisher = self.app_event_publisher.clone();
        let store = self.state_machine.store.clone();
        let helpers = Arc::clone(&self.helpers);
        let dialog_adapter = self.dialog_adapter.clone();
        let media_adapter = self.media_adapter.clone();
        let coordinator = self.coordinator.get().and_then(std::sync::Weak::upgrade);
        let pending_exact_responses = coordinator
            .as_ref()
            .map(|coordinator| coordinator.pending_exact_response_registry());
        let setup_teardown_deadlines = coordinator
            .as_ref()
            .map(|coordinator| coordinator.setup_teardown_deadline_cancellation());
        if store.get_session_snapshot_exact(&handle).is_err() {
            debug!(
                session = %session_id,
                "exact terminal lifetime was already released before publication"
            );
            return;
        }
        let claim_owner = match publisher.claim_exact_terminal(&handle) {
            ExactTerminalClaim::Owner(owner) => {
                debug!(session = %session_id, "dialog handler owns exact terminal release");
                owner
            }
            ExactTerminalClaim::Observer(_) => {
                debug!(
                    session = %session_id,
                    "exact terminal release is already owned by another path"
                );
                return;
            }
        };
        let admission_session_id = session_id.clone();
        if !self.retained_tasks.spawn_or_child(async move {
            let release_guard =
                cleanup_diag::stage_guard(CleanupStage::TerminalRelease, &session_id.0);
            if let Err(error) = publisher.publish_terminal_best_effort_exact(&handle, api_event) {
                tracing::warn!(
                    "Failed to publish terminal event to global coordinator: {}",
                    error
                );
            }
            let completion = match crate::api::unified::release_exact_local_resources_with_retry(
                store,
                helpers,
                dialog_adapter,
                media_adapter,
                pending_exact_responses,
                setup_teardown_deadlines,
                handle,
            )
            .await
            {
                Ok(()) => {
                    release_guard.finish_success();
                    ExactTerminalCompletion::Released
                }
                Err(error) => {
                    tracing::debug!(%error, "exact terminal cleanup was incomplete");
                    release_guard.finish_failure();
                    ExactTerminalCompletion::ReleaseFailed
                }
            };
            claim_owner.finish(completion);
        }) {
            tracing::error!(
                session = %admission_session_id,
                "terminal release task admission was closed after event dispatch drained"
            );
        }
    }

    /// Commit the canonical exact registry bundle. The lower inbound dialog
    /// already owns its protocol mapping, so this deliberately does not call
    /// the public unconditional dialog-core mapping API.
    fn commit_exact_inbound_dialog(
        &self,
        handle: &SessionRegistryHandle,
        dialog_id: DialogId,
        incoming_info: crate::types::IncomingCallInfo,
    ) -> SessionResult<()> {
        let receipt = self
            .registry
            .commit_inbound_dialog_exact(
                handle.key(),
                handle.slot_revision(),
                dialog_id,
                incoming_info,
            )
            .map_err(|error| {
                SessionError::InternalError(format!(
                    "exact inbound registry commit failed (class=registry): {error}"
                ))
            })?;

        receipt.finalize().map_err(|error| {
            SessionError::InternalError(format!(
                "exact inbound mapping finalize failed (class=registry): {error}"
            ))
        })
    }

    /// Start event processing loops.
    ///
    /// Background tasks will stop when `shutdown_rx` receives `true`.
    pub async fn start(
        &self,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> SessionResult<()> {
        self.start_global_event_subscriptions(shutdown_rx).await?;
        Ok(())
    }

    /// Start subscriptions to global cross-crate events
    async fn start_global_event_subscriptions(
        &self,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> SessionResult<()> {
        // Session lifecycle correctness must not depend on broadcast delivery.
        // Register a direct handler that only enqueues into bounded sharded
        // workers; the global broadcast remains available for observers.
        let dialog_router = DialogToSessionDirectRouter::new(
            self.clone(),
            dialog_dispatch_worker_count(),
            self.dialog_event_dispatch_queue_capacity,
            shutdown_rx.clone(),
        )?;
        if let Some(ingress) = &self.causal_dialog_ingress {
            ingress.install(dialog_router.clone())?;
        } else {
            self.global_coordinator
                .register_handler("dialog_to_session", dialog_router.clone())
                .await
                .map_err(|e| {
                    SessionError::InternalError(format!(
                        "Failed to register direct dialog event handler: {}",
                        e
                    ))
                })?;
        }

        if let Some(mut deferred_replay_rx) = self
            .dialog_adapter
            .outbound_request_tracker
            .take_deferred_replay_receiver()
        {
            let replay_router = dialog_router;
            let tracker = self.dialog_adapter.outbound_request_tracker.clone();
            let mut replay_shutdown = shutdown_rx.clone();
            if !self.retained_tasks.spawn(async move {
                loop {
                    tokio::select! {
                        changed = replay_shutdown.changed() => {
                            if shutdown_change_requests_stop(changed, &replay_shutdown) {
                                deferred_replay_rx.close();
                                while let Ok(deferred) = deferred_replay_rx.try_recv() {
                                    tracker.abort_deferred_replay(&deferred);
                                }
                                break;
                            }
                        }
                        deferred = deferred_replay_rx.recv() => {
                            let Some(deferred) = deferred else { break };
                            if let Err(error) = replay_router.handle_deferred(deferred).await {
                                error!(
                                    "Failed to replay deferred exact in-dialog event: {}",
                                    error
                                );
                            }
                        }
                    }
                }
            }) {
                return Err(SessionError::InternalError(
                    "deferred replay retained task admission is closed".to_string(),
                ));
            }
        }

        // Subscribe to transport-to-session diagnostics such as SIP trace.
        let mut transport_sub = self
            .global_coordinator
            .subscribe("transport_to_session")
            .await
            .map_err(|e| {
                SessionError::InternalError(format!(
                    "Failed to subscribe to transport diagnostics: {}",
                    e
                ))
            })?;

        let handler = self.clone();
        let mut shutdown = shutdown_rx.clone();
        if !self.retained_tasks.spawn(async move {
            loop {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if shutdown_change_requests_stop(changed, &shutdown) { break; }
                    }
                    event = transport_sub.recv() => {
                        let Some(event) = event else { break };
                        if let Err(e) = handler.handle(event).await {
                            error!("Error handling transport-to-session event: {}", e);
                        }
                    }
                }
            }
        }) {
            return Err(SessionError::InternalError(
                "transport event retained task admission is closed".to_string(),
            ));
        }

        // Subscribe to media-to-session events
        let mut media_sub = self
            .global_coordinator
            .subscribe("media_to_session")
            .await
            .map_err(|e| {
                SessionError::InternalError(format!("Failed to subscribe to media events: {}", e))
            })?;

        let handler = self.clone();
        let mut shutdown = shutdown_rx.clone();
        if !self.retained_tasks.spawn(async move {
            loop {
                tokio::select! {
                    changed = shutdown.changed() => {
                        if shutdown_change_requests_stop(changed, &shutdown) { break; }
                    }
                    event = media_sub.recv() => {
                        let Some(event) = event else { break };
                        if let Err(e) = handler.handle(event).await {
                            error!("Error handling media-to-session event: {}", e);
                        }
                    }
                }
            }
        }) {
            return Err(SessionError::InternalError(
                "media event retained task admission is closed".to_string(),
            ));
        }

        if let Some(state_machine_event_rx) = &self.state_machine_event_rx {
            let state_machine_event_rx = state_machine_event_rx.clone();
            let handler = self.clone();
            let mut shutdown = shutdown_rx;
            if !self.retained_tasks.spawn(async move {
                loop {
                    tokio::select! {
                        changed = shutdown.changed() => {
                            if shutdown_change_requests_stop(changed, &shutdown) { break; }
                        }
                        event = async {
                            let mut rx = state_machine_event_rx.lock().await;
                            rx.recv().await
                        } => {
                            let Some(event) = event else { break };
                            handler.handle_state_machine_event(event).await;
                        }
                    }
                }
            }) {
                return Err(SessionError::InternalError(
                    "state-machine event retained task admission is closed".to_string(),
                ));
            }
        }

        Ok(())
    }

    async fn handle_state_machine_event(
        &self,
        event: crate::state_machine::executor::SessionEvent,
    ) {
        let legacy_observation = match &event {
            crate::state_machine::executor::SessionEvent::StateChanged {
                session_id,
                old_state,
                new_state,
            } => Some((
                session_id.clone(),
                crate::state_machine::helpers::SessionEvent::StateChanged {
                    from: *old_state,
                    to: *new_state,
                },
            )),
            crate::state_machine::executor::SessionEvent::MediaFlowEstablished {
                session_id,
                ..
            } => Some((
                session_id.clone(),
                crate::state_machine::helpers::SessionEvent::MediaReady,
            )),
            crate::state_machine::executor::SessionEvent::CallEstablished { session_id } => Some((
                session_id.clone(),
                crate::state_machine::helpers::SessionEvent::CallEstablished,
            )),
            crate::state_machine::executor::SessionEvent::CallTerminated { session_id } => Some((
                session_id.clone(),
                crate::state_machine::helpers::SessionEvent::CallTerminated {
                    reason: "terminated".to_string(),
                },
            )),
            crate::state_machine::executor::SessionEvent::CallCancelled { session_id } => Some((
                session_id.clone(),
                crate::state_machine::helpers::SessionEvent::CallTerminated {
                    reason: "cancelled".to_string(),
                },
            )),
            crate::state_machine::executor::SessionEvent::CallOnHold { .. }
            | crate::state_machine::executor::SessionEvent::CallResumed { .. }
            | crate::state_machine::executor::SessionEvent::Custom { .. } => None,
        };
        if let Some((session_id, legacy_observation)) = legacy_observation {
            self.helpers
                .notify_subscribers(&session_id, legacy_observation)
                .await;
        }

        let api_event = match event {
            crate::state_machine::executor::SessionEvent::CallCancelled { session_id } => {
                debug!(
                    "Ignoring state-machine CallCancelled for {}; terminal cancellation is published by the dialog event handler after wire teardown",
                    session_id
                );
                return;
            }
            crate::state_machine::executor::SessionEvent::CallOnHold { session_id } => {
                Some(crate::api::events::Event::CallOnHold {
                    call_id: session_id,
                })
            }
            crate::state_machine::executor::SessionEvent::CallResumed { session_id } => {
                Some(crate::api::events::Event::CallResumed {
                    call_id: session_id,
                })
            }
            _ => None,
        };

        if let Some(api_event) = api_event {
            publish_api_event(&self.app_event_publisher, api_event);
        }
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for SessionCrossCrateEventHandler {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event: {}", event.event_type());

        match event.as_any().downcast_ref::<RvoipCrossCrateEvent>() {
            Some(RvoipCrossCrateEvent::DialogToSession(typed)) => {
                let wrapped = RvoipCrossCrateEvent::DialogToSession(typed.clone());
                let route_key = RoutableEvent::session_id(&wrapped);
                let exact_handle = match capture_dialog_ingress_handle(
                    self.state_machine.store.as_ref(),
                    typed,
                    route_key,
                ) {
                    Ok(handle) => handle,
                    Err(_)
                        if matches!(
                            typed,
                            DialogToSessionEvent::InfoReceived { .. }
                                | DialogToSessionEvent::ReinviteReceived { .. }
                                | DialogToSessionEvent::TransferRequested { .. }
                        ) =>
                    {
                        return Err(anyhow::anyhow!(
                            "response-bearing dialog request has no exact live session owner"
                        ));
                    }
                    Err(_) => return Ok(()),
                };
                self.handle_dialog_to_session_event(typed, exact_handle.as_ref())
                    .await?;
            }
            Some(RvoipCrossCrateEvent::MediaToSession(typed)) => {
                self.handle_media_to_session_event(typed).await?;
            }
            Some(RvoipCrossCrateEvent::TransportToSession(typed)) => {
                self.handle_transport_to_session_event(typed).await?;
            }
            Some(other) => {
                debug!(
                    "Ignoring cross-crate event not targeted at session-core: {:?}",
                    other
                );
            }
            None => {
                debug!(
                    "Ignoring non-rvoip cross-crate event on session-core handler: {}",
                    event.event_type()
                );
            }
        }

        Ok(())
    }
}

impl SessionCrossCrateEventHandler {
    /// Check if a session belongs to this handler's store.
    /// Returns false (and logs at debug) if the session was created by a different peer.
    fn is_our_session(&self, session_id: &SessionId) -> bool {
        self.state_machine
            .store
            .lifecycle_handle(session_id)
            .is_some()
    }

    async fn handle_state_event_if_ours(
        &self,
        handle: &SessionRegistryHandle,
        event_type: EventType,
        label: &str,
    ) -> Result<()> {
        let session_id = handle.session_id();
        if self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)
            .is_err()
        {
            debug!(
                "Ignoring {} for session {} - not in our store",
                label, session_id
            );
            return Ok(());
        };
        if let Err(e) = self
            .state_machine
            .process_event_exact(handle, event_type)
            .await
        {
            error!(
                "Failed to process {} for session {}: {}",
                label, session_id, e
            );
        }
        Ok(())
    }

    async fn handle_dialog_created_parts(&self, dialog_id: String, call_id: String) -> Result<()> {
        // Dialog creation is an observational lower-layer fact. Exact dialog
        // identity is committed by the initiating/inbound lifecycle path; a
        // route-less observation must never rediscover a raw SessionId and
        // mutate whichever generation happens to own that string later.
        debug!(dialog_id, call_id, "Observed dialog creation");
        Ok(())
    }

    async fn acquire_server_call_admission(&self) -> ServerCallAdmissionDecision {
        #[cfg(feature = "perf-tests")]
        crate::admission_diag::record_attempt();

        let Some(hard_limit) = self.server_call_admission_limit else {
            #[cfg(feature = "perf-tests")]
            crate::admission_diag::record_no_limit_admit();
            return ServerCallAdmissionDecision::Admit(None);
        };
        let soft_limit = self
            .server_call_admission_soft_limit
            .unwrap_or(hard_limit)
            .min(hard_limit);
        #[cfg(feature = "perf-tests")]
        crate::admission_diag::record_limits(hard_limit, soft_limit);
        let pacing_delay = self
            .server_call_admission_pacing_delay_ms
            .map(Duration::from_millis);
        let mut paced_once = false;

        loop {
            #[cfg(feature = "perf-tests")]
            let admission_lock_wait_started = Instant::now();
            let _lock = self.server_call_admission_lock.lock().await;
            #[cfg(feature = "perf-tests")]
            crate::admission_diag::record_lock_wait(admission_lock_wait_started.elapsed());
            let pending = self.server_call_admission_pending.load(Ordering::Relaxed);
            let observed_sessions = self
                .state_machine
                .store
                .sessions
                .len()
                .saturating_add(pending);
            #[cfg(feature = "perf-tests")]
            crate::admission_diag::record_observed(observed_sessions, pending);

            if self
                .server_call_admission_overloaded
                .load(Ordering::Relaxed)
            {
                if observed_sessions < soft_limit {
                    self.server_call_admission_overloaded
                        .store(false, Ordering::Relaxed);
                    #[cfg(feature = "perf-tests")]
                    crate::admission_diag::record_overload_cleared();
                } else {
                    #[cfg(feature = "perf-tests")]
                    crate::admission_diag::record_reject_overloaded(observed_sessions);
                    return ServerCallAdmissionDecision::Reject {
                        observed_sessions,
                        hard_limit,
                    };
                }
            }

            if observed_sessions >= hard_limit {
                self.server_call_admission_overloaded
                    .store(true, Ordering::Relaxed);
                #[cfg(feature = "perf-tests")]
                {
                    crate::admission_diag::record_overload_entered();
                    crate::admission_diag::record_reject_hard_limit(observed_sessions);
                }
                return ServerCallAdmissionDecision::Reject {
                    observed_sessions,
                    hard_limit,
                };
            }

            if !paced_once {
                if let (Some(delay), Some(configured_soft_limit)) =
                    (pacing_delay, self.server_call_admission_soft_limit)
                {
                    if observed_sessions >= configured_soft_limit {
                        #[cfg(feature = "perf-tests")]
                        crate::admission_diag::record_pacing_decision();
                        drop(_lock);
                        #[cfg(feature = "perf-tests")]
                        let admission_pacing_started = Instant::now();
                        tokio::time::sleep(delay).await;
                        #[cfg(feature = "perf-tests")]
                        crate::admission_diag::record_pacing_sleep(
                            admission_pacing_started.elapsed(),
                        );
                        paced_once = true;
                        continue;
                    }
                }
            }

            #[cfg(feature = "perf-tests")]
            {
                let pending_after = self
                    .server_call_admission_pending
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                crate::admission_diag::record_admit(observed_sessions, pending_after);
            }
            #[cfg(not(feature = "perf-tests"))]
            self.server_call_admission_pending
                .fetch_add(1, Ordering::Relaxed);
            return ServerCallAdmissionDecision::Admit(Some(ServerCallAdmissionGuard {
                pending: self.server_call_admission_pending.clone(),
            }));
        }
    }

    async fn cleanup_rejected_incoming_call_routes(
        &self,
        incoming_dialog_id: Option<&rvoip_sip_dialog::DialogId>,
        dialog_id_str: &str,
        transaction_id: &rvoip_sip_dialog::transaction::TransactionKey,
    ) {
        if let Some(dialog_id) = incoming_dialog_id {
            let removed = self
                .dialog_adapter
                .dialog_api
                .dialog_manager()
                .core()
                .cleanup_dialog_storage_and_transactions(dialog_id)
                .await;
            debug!(
                dialog_id = %dialog_id,
                removed,
                "Cleaned up rejected inbound INVITE dialog after admission overload response"
            );
        } else {
            warn!(
                dialog_id = %dialog_id_str,
                "Rejected inbound INVITE for overload without a parseable dialog id; dialog-core cleanup skipped"
            );
        }
        self.dialog_adapter
            .dialog_api
            .dialog_manager()
            .core()
            .cleanup_transaction_receiver(transaction_id);
    }

    async fn reject_and_cleanup_incoming_call_for_overload(
        &self,
        transaction_id: &str,
        incoming_dialog_id: Option<&rvoip_sip_dialog::DialogId>,
        dialog_id_str: &str,
        observed_sessions: usize,
        configured_admission_limit: Option<usize>,
    ) -> Result<()> {
        let transaction_id = transaction_id
            .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
            .map_err(|_| anyhow::anyhow!("server admission transaction id is invalid"))?;
        let extra_headers = self
            .server_overload_retry_after_secs
            .map(|seconds| {
                vec![rvoip_sip_core::types::TypedHeader::RetryAfter(
                    rvoip_sip_core::types::retry_after::RetryAfter::new(seconds),
                )]
            })
            .unwrap_or_default();
        let dialog_api = Arc::clone(&self.dialog_adapter.dialog_api);
        let cleanup_owner = self.clone();
        let cleanup_dialog_id = incoming_dialog_id.cloned();
        let cleanup_dialog_label = dialog_id_str.to_string();
        let cleanup_transaction = transaction_id.clone();
        let completion =
            spawn_retained_exact_response_completion(&self.retained_tasks, async move {
                let outcome = author_exact_final_response(
                    dialog_api,
                    transaction_id,
                    rvoip_sip_core::StatusCode::ServiceUnavailable,
                    extra_headers,
                )
                .await;
                // ZeroWire is the only retryable disposition. Preserve its
                // exact server transaction and dialog route so the negative
                // processing ACK can authorize one safe lower-layer retry.
                // Written and WireUnknown are terminal and must retire their
                // routes without ever authoring a duplicate final response.
                if exact_final_response_retires_routes(outcome) {
                    cleanup_owner
                        .cleanup_rejected_incoming_call_routes(
                            cleanup_dialog_id.as_ref(),
                            &cleanup_dialog_label,
                            &cleanup_transaction,
                        )
                        .await;
                }
                outcome
            })?;
        let outcome = completion.await.map_err(|_| {
            anyhow::anyhow!("server overload response child ended without completion")
        })?;
        exact_final_response_result("server overload 503 response", outcome)?;
        warn!(
            observed_sessions,
            configured_admission_limit,
            soft_limit = ?self.server_call_admission_soft_limit,
            retry_after_secs = ?self.server_overload_retry_after_secs,
            ?outcome,
            "Rejected inbound INVITE with 503 because server admission capacity was reached"
        );
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_incoming_call_parts(
        &self,
        session_id_str: String,
        call_id: String,
        from: String,
        to: String,
        sdp: Option<String>,
        headers: &std::collections::HashMap<String, String>,
        transaction_id: &str,
        _source_addr: &str,
        raw_request: Option<bytes::Bytes>,
        transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
    ) -> Result<()> {
        let dialog_id_str = headers
            .get("X-Dialog-Id")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let p_asserted_identity = headers.get("P-Asserted-Identity").cloned();

        let incoming_dialog_id = uuid::Uuid::parse_str(&dialog_id_str)
            .ok()
            .map(rvoip_sip_dialog::DialogId);
        if let Some(rvoip_dialog_id) = incoming_dialog_id.as_ref() {
            if registry_has_exact_dialog(self.registry.as_ref(), rvoip_dialog_id) {
                debug!(
                    "Ignoring IncomingCall for dialog {} - already handled by another peer",
                    dialog_id_str
                );
                return Ok(());
            }

            if !self
                .dialog_adapter
                .dialog_api
                .dialog_manager()
                .core()
                .has_dialog(rvoip_dialog_id)
            {
                debug!(
                    "Ignoring IncomingCall for dialog {} - not in our dialog-core",
                    dialog_id_str
                );
                return Ok(());
            }
        }

        let session_id = SessionId(session_id_str);
        let pending_transaction = transaction_id
            .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
            .map_err(|_| {
                let (transaction_class, transaction_bytes) =
                    transaction_id_diagnostics(transaction_id);
                debug!(
                    session = %session_id,
                    transaction_class,
                    transaction_bytes,
                    "IncomingCall carried an unparsable transaction id"
                );
                SessionError::InvalidTransition(
                    "IncomingCall has no exact inbound INVITE transaction".to_string(),
                )
            })?;
        let inbound_response = InboundResponseStateInput::from_initial_invite_transaction(
            pending_transaction.clone(),
        )?;
        let authenticated_principal = self
            .dialog_adapter
            .dialog_api
            .dialog_manager()
            .core()
            .transaction_manager()
            .peek_inbound_principal(&pending_transaction);
        let setup_guard = cleanup_diag::stage_guard(CleanupStage::IncomingCallSetup, &session_id.0);

        let admission_guard = match self.acquire_server_call_admission().await {
            ServerCallAdmissionDecision::Admit(guard) => guard,
            ServerCallAdmissionDecision::Reject {
                observed_sessions,
                hard_limit,
            } => {
                self.reject_and_cleanup_incoming_call_for_overload(
                    transaction_id,
                    incoming_dialog_id.as_ref(),
                    &dialog_id_str,
                    observed_sessions,
                    Some(hard_limit),
                )
                .await?;
                setup_guard.finish_success();
                return Ok(());
            }
        };

        let initial_state = InboundInviteInitialState {
            local_uri: to.clone(),
            remote_uri: from.clone(),
            received_at: Instant::now(),
            transaction: pending_transaction,
            remote_sdp: sdp.clone(),
        };
        let create_result = self
            .state_machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAS, true, move |session| {
                initial_state.apply(session);
            })
            .await;
        drop(admission_guard);
        let created_session = match create_result {
            Ok(session) => session,
            Err(error) => {
                if is_session_lifecycle_capacity_exhaustion(error.as_ref()) {
                    let observed_sessions = self.state_machine.store.sessions.len();
                    warn!(
                        observed_sessions,
                        configured_admission_limit = ?self.server_call_admission_limit,
                        "Session lifecycle capacity rejected inbound INVITE; returning SIP overload"
                    );
                    self.reject_and_cleanup_incoming_call_for_overload(
                        transaction_id,
                        incoming_dialog_id.as_ref(),
                        &dialog_id_str,
                        observed_sessions,
                        self.server_call_admission_limit,
                    )
                    .await?;
                    setup_guard.finish_success();
                    return Ok(());
                }
                return Err(SessionError::InternalError(format!(
                    "Failed to create session: {error}"
                ))
                .into());
            }
        };
        let session_remote_sdp = created_session.remote_sdp.clone();

        let dialog_uuid =
            uuid::Uuid::parse_str(&dialog_id_str).unwrap_or_else(|_| uuid::Uuid::new_v4());
        let our_dialog_id = DialogId(dialog_uuid);
        let lifecycle = self
            .state_machine
            .store
            .lifecycle_token(&session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?;
        let incoming_info = crate::types::IncomingCallInfo {
            session_id: session_id.clone(),
            from: from.clone(),
            to: to.clone(),
            call_id: call_id.clone(),
            dialog_id: our_dialog_id,
            p_asserted_identity: p_asserted_identity.clone(),
        };
        self.commit_exact_inbound_dialog(&lifecycle, our_dialog_id, incoming_info.clone())?;

        let event_type = if self.fast_auto_accept_incoming_calls {
            EventType::IncomingCallAutoAccept {
                from: from.clone(),
                sdp,
            }
        } else {
            EventType::IncomingCall {
                from: from.clone(),
                sdp,
            }
        };

        let process_result = process_inbound_response_event_exact_on_fresh_task(
            Arc::clone(&self.state_machine),
            lifecycle.clone(),
            event_type,
            inbound_response,
        )
        .await;
        if let Err(error) = process_result {
            let disposition = crate::state_machine::actions::exact_sip_response_failure_disposition(
                error.as_ref(),
            );
            let detail = error.to_string();
            if let Err(processing_error) =
                exact_response_failure_processing_ack("IncomingCall", disposition, &detail)
            {
                let cleanup = release_failed_inbound_upper_resources_with_retry(
                    Arc::clone(&self.state_machine.store),
                    Arc::clone(&self.helpers),
                    Arc::clone(&self.dialog_adapter),
                    Arc::clone(&self.media_adapter),
                    lifecycle.clone(),
                );
                let completion = match spawn_retained_failed_inbound_cleanup(
                    &self.retained_tasks,
                    cleanup,
                ) {
                    Ok(completion) => completion,
                    Err(cleanup_error) => {
                        setup_guard.finish_failure();
                        return Err(anyhow::anyhow!(
                            "{processing_error}; failed to retain exact upper cleanup: {cleanup_error}"
                        ));
                    }
                };
                let cleanup_result = completion.await.map_err(|_| {
                    anyhow::anyhow!("failed inbound INVITE upper cleanup ended without completion")
                });
                setup_guard.finish_failure();
                match cleanup_result {
                    Ok(Ok(())) => return Err(processing_error),
                    Ok(Err(cleanup_error)) => {
                        return Err(anyhow::anyhow!(
                            "{processing_error}; exact upper cleanup remained incomplete: {cleanup_error}"
                        ));
                    }
                    Err(cleanup_error) => {
                        return Err(anyhow::anyhow!("{processing_error}; {cleanup_error}"));
                    }
                }
            }
            warn!(
                session = %session_id,
                ?disposition,
                "Inbound INVITE response reached a terminal wire disposition"
            );
        }
        {
            if self.fast_auto_accept_incoming_calls {
                debug!("Fast auto-accepted inbound call {}", session_id);
            }
            if let Some(coordinator) = self.coordinator.get().and_then(|w| w.upgrade()) {
                coordinator
                    .schedule_inbound_setup_timeout(&session_id)
                    .await;
            }

            // SIP_API_DESIGN_2 Phase A: re-parse the inbound INVITE bytes
            // after the fast 200 OK path has completed, but before app
            // observation events are published. Failure to parse is never
            // fatal — we fall back to the legacy headers-only path.
            let mut observed_request = None;
            if let Some(bytes) = raw_request.as_ref() {
                match rvoip_sip_core::parse_message(bytes.as_ref()) {
                    Ok(rvoip_sip_core::Message::Request(req)) => {
                        let req = Arc::new(req);
                        observed_request = Some(req);
                    }
                    Ok(_) => {
                        tracing::warn!(
                            session_id = %session_id,
                            "IncomingCall raw_request was not a SIP request"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %e,
                            "Failed to re-parse inbound INVITE bytes; \
                             IncomingCall.raw_request() will be None"
                        );
                    }
                }
            }
            if let Some(coordinator) = self.coordinator.get().and_then(|weak| weak.upgrade()) {
                coordinator.notify_inbound_invite_observers(
                    crate::api::unified::InboundInviteObservation {
                        session_id: session_id.clone(),
                        request: observed_request.clone(),
                        principal: authenticated_principal.clone(),
                    },
                );
            }
            let mut pending = self
                .registry
                .pending_bundle_exact(lifecycle.key(), lifecycle.slot_revision())
                .map_err(|error| {
                    SessionError::InternalError(format!(
                        "exact inbound bundle read failed (class=registry): {error}"
                    ))
                })?;
            pending.request = observed_request;
            pending.transport = Some(Arc::new(sip_transport_context(&transport)));
            pending.principal = authenticated_principal.clone();
            self.registry
                .store_pending_bundle_exact(
                    lifecycle.key(),
                    lifecycle.slot_revision(),
                    PendingInboundBundle {
                        info: pending.info,
                        request: pending.request,
                        transport: pending.transport,
                        principal: pending.principal,
                    },
                )
                .map_err(|error| {
                    SessionError::InternalError(format!(
                        "exact inbound bundle commit failed (class=registry): {error}"
                    ))
                })?;

            self.app_event_publisher.publish_exact(
                &lifecycle,
                crate::api::events::Event::IncomingCall {
                    call_id: session_id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    sdp: session_remote_sdp,
                },
            );
            if let Some(principal) = authenticated_principal {
                publish_api_event(
                    &self.app_event_publisher,
                    crate::api::events::Event::IncomingCallAuthenticated {
                        call_id: session_id.clone(),
                        principal,
                    },
                );
            }

            if let Some(ref tx) = self.incoming_call_tx {
                // Project the same immutable bundle committed above. Queue
                // pressure remains observational and cannot change admission,
                // state-machine progress, or wire handling.
                if let Err(e) = tx.try_send(incoming_info) {
                    debug!(
                        "Legacy incoming_call_tx not ready — caller is using app_event_publisher path: {}",
                        e
                    );
                }
            }
        }

        setup_guard.finish_success();
        Ok(())
    }

    async fn handle_call_established_parts(
        &self,
        handle: &SessionRegistryHandle,
        sdp_answer: Option<String>,
        raw_response: Option<bytes::Bytes>,
    ) -> Result<()> {
        let session_id = handle.session_id().clone();
        let initial_snapshot = match self.state_machine.store.get_session_snapshot_exact(handle) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                debug!(
                    "Ignoring CallEstablished for session {} - not in our store",
                    session_id
                );
                return Ok(());
            }
        };

        let parsed_response = response_from_bytes(raw_response.as_ref());
        let response_transaction = parsed_response
            .as_ref()
            .and_then(rvoip_sip_dialog::transaction::TransactionKey::from_response);
        let pending_offer = initial_snapshot.pending_offer_answer.as_ref();
        let pending_correlation = correlate_pending_offer_transaction(
            pending_offer.map(|pending| &pending.method),
            pending_offer.and_then(|pending| pending.transaction_id.as_ref()),
            &rvoip_sip_core::Method::Invite,
            response_transaction.as_ref(),
        );
        if invite_success_is_retransmission(
            initial_snapshot.dialog_established,
            pending_correlation,
        ) {
            if let (Some(response), Some(transaction_id)) =
                (parsed_response.as_ref(), response_transaction.as_ref())
            {
                self.dialog_adapter
                    .send_invite_2xx_ack_exact(handle, transaction_id, response)
                    .await?;
            }
            debug!(
                session_id = %session_id,
                "ACKed and ignored retransmitted INVITE 2xx after offer/answer commit"
            );
            return Ok(());
        }
        match pending_correlation {
            PendingOfferTransactionCorrelation::OtherMethod => {
                if let (Some(response), Some(transaction_id)) =
                    (parsed_response.as_ref(), response_transaction.as_ref())
                {
                    self.dialog_adapter
                        .send_invite_2xx_ack_exact(handle, transaction_id, response)
                        .await?;
                }
                warn!(
                    session_id = %session_id,
                    "Ignoring an INVITE success while another method owns the pending offer"
                );
                return Ok(());
            }
            PendingOfferTransactionCorrelation::Mismatched => {
                if let (Some(response), Some(transaction_id)) =
                    (parsed_response.as_ref(), response_transaction.as_ref())
                {
                    self.dialog_adapter
                        .send_invite_2xx_ack_exact(handle, transaction_id, response)
                        .await?;
                }
                warn!(
                    session_id = %session_id,
                    "Ignoring a re-INVITE answer that does not own the pending offer transaction"
                );
                return Ok(());
            }
            PendingOfferTransactionCorrelation::Exact => {
                if let Some(transaction) = response_transaction.as_ref() {
                    if self
                        .dialog_adapter
                        .outbound_request_tracker
                        .complete_if_matches(handle, TrackedInDialogMethod::Reinvite, transaction)
                    {
                        self.clear_tracked_request_auth_state(
                            handle,
                            TrackedInDialogMethod::Reinvite,
                            transaction,
                        )
                        .await;
                    }
                }
            }
            PendingOfferTransactionCorrelation::NoPendingOffer => {}
        }

        if initial_snapshot.session_refresh_phase
            == crate::session_store::state::SessionRefreshPhase::ReinviteInFlight
        {
            if let Err(error) = self
                .state_machine
                .process_session_refresh_exact(handle, SessionRefreshStateInput::ReinviteSucceeded)
                .await
            {
                debug!(
                    session_id = %session_id,
                    %error,
                    "stale RFC 4028 re-INVITE success was suppressed"
                );
            }
            return Ok(());
        }

        let mid_dialog_offer = pending_correlation == PendingOfferTransactionCorrelation::Exact;
        if mid_dialog_offer && sdp_answer.as_deref().is_none_or(str::is_empty) {
            if let (Some(response), Some(transaction_id)) =
                (parsed_response.as_ref(), response_transaction.as_ref())
            {
                self.dialog_adapter
                    .send_invite_2xx_ack_exact(handle, transaction_id, response)
                    .await?;
            }
            self.app_event_publisher.publish_exact(
                handle,
                crate::api::events::Event::RenegotiationFailed {
                    call_id: session_id.clone(),
                    method: "INVITE".to_string(),
                    reason: "successful response contained no SDP answer".to_string(),
                },
            );
            self.terminate_confirmed_negotiation(
                handle,
                "acknowledged re-INVITE contained no SDP answer",
            )
            .await?;
            return Ok(());
        }

        let delayed_offer = initial_invite_used_delayed_offer(&initial_snapshot);

        let ack_input = parsed_response
            .clone()
            .zip(response_transaction.clone())
            .filter(|(response, _)| !response.body().is_empty())
            .map(|(response, transaction_id)| {
                Invite2xxAckStateInput::new(transaction_id, response)
            });
        if delayed_offer
            && (sdp_answer
                .as_deref()
                .is_none_or(|sdp| sdp.trim().is_empty())
                || ack_input.is_none())
        {
            if let (Some(response), Some(transaction_id)) =
                (parsed_response.as_ref(), response_transaction.as_ref())
            {
                self.dialog_adapter
                    .send_invite_2xx_ack_exact(handle, transaction_id, response)
                    .await?;
            }
            self.app_event_publisher.publish_exact(
                handle,
                crate::api::events::Event::RenegotiationFailed {
                    call_id: session_id.clone(),
                    method: "INVITE".to_string(),
                    reason: "offerless INVITE received no usable 200 OK SDP offer".to_string(),
                },
            );
            self.fail_initial_invite_negotiation(
                handle,
                "offerless INVITE received no usable 200 OK SDP offer",
            )
            .await?;
            return Ok(());
        }
        let process_result = if let Some(ack) = ack_input {
            process_invite_2xx_answer_exact_on_fresh_task(
                Arc::clone(&self.state_machine),
                handle.clone(),
                sdp_answer.clone(),
                ack,
            )
            .await
        } else {
            process_event_with_remote_sdp_exact_on_fresh_task(
                Arc::clone(&self.state_machine),
                handle.clone(),
                EventType::Dialog200OK,
                sdp_answer.clone(),
            )
            .await
        };
        let result = match process_result {
            Ok(result) => result,
            Err(e) => {
                error!("Failed to process CallEstablished as Dialog200OK: {}", e);
                if matches!(
                    e.downcast_ref::<SessionError>(),
                    Some(SessionError::DialogError(_))
                ) {
                    // Leave the exact response pending so its retransmission
                    // retries the same bodyless or cached answer-bearing ACK.
                    return Err(SessionError::InvalidTransition(format!(
                        "INVITE 2xx ACK write failed for session {}: {e}",
                        session_id
                    ))
                    .into());
                }
                if let (Some(response), Some(transaction_id)) =
                    (parsed_response.as_ref(), response_transaction.as_ref())
                {
                    self.dialog_adapter
                        .send_invite_2xx_ack_exact(handle, transaction_id, response)
                        .await?;
                }
                if let Some(crate::errors::SessionError::SdesNegotiationFailed(diagnostic)) =
                    e.downcast_ref::<SessionError>()
                {
                    let response = parsed_response.as_ref().map_or_else(
                        || {
                            crate::api::incoming::IncomingResponse::synthetic(
                                session_id.clone(),
                                200,
                                "OK".to_string(),
                                sdp_answer.clone(),
                            )
                        },
                        |response| {
                            crate::api::incoming::IncomingResponse::with_response(
                                session_id.clone(),
                                response.status.as_u16(),
                                response.reason_phrase().to_string(),
                                sdp_answer.clone(),
                                Arc::new(response.clone()),
                            )
                        },
                    );
                    self.app_event_publisher.publish_exact(
                        handle,
                        crate::api::events::Event::SdesNegotiationFailed {
                            call_id: session_id.clone(),
                            response,
                            diagnostic: diagnostic.clone(),
                        },
                    );
                }
                if mid_dialog_offer || delayed_offer {
                    self.app_event_publisher.publish_exact(
                        handle,
                        crate::api::events::Event::RenegotiationFailed {
                            call_id: session_id.clone(),
                            method: "INVITE".to_string(),
                            reason: if delayed_offer {
                                "invalid SDP offer or failed answer-bearing ACK".to_string()
                            } else {
                                "invalid or unacceptable SDP answer".to_string()
                            },
                        },
                    );
                    if delayed_offer {
                        self.fail_initial_invite_negotiation(
                            handle,
                            "delayed-offer SDP negotiation or media commit failed",
                        )
                        .await?;
                    } else {
                        self.terminate_confirmed_negotiation(
                            handle,
                            "acknowledged re-INVITE answer could not be applied",
                        )
                        .await?;
                    }
                    return Ok(());
                }
                self.fail_initial_invite_negotiation(
                    handle,
                    "successful initial INVITE response had missing, invalid, or unusable SDP",
                )
                .await?;
                return Ok(());
            }
        };
        match committed_dialog_200(&session_id, initial_snapshot.role, &result)? {
            CommittedDialog200::InitialAnswer => {
                let committed = match self.state_machine.store.get_session_snapshot_exact(handle) {
                    Ok(snapshot) => snapshot,
                    Err(_) => return Ok(()),
                };
                let events = project_committed_response_events(
                    committed.as_ref(),
                    CommittedResponseObservation::Established {
                        sdp: sdp_answer,
                        raw_response,
                    },
                );
                publish_committed_api_projection(&self.app_event_publisher, handle, events);
            }
            CommittedDialog200::NonInitial => {
                info!(
                    "Suppressing CallAnswered for {} because the committed Dialog200OK was not an initial-answer YAML outcome",
                    session_id
                );
            }
        }

        Ok(())
    }

    async fn fail_initial_invite_negotiation(
        &self,
        handle: &SessionRegistryHandle,
        reason: &str,
    ) -> Result<()> {
        self.app_event_publisher.publish_exact(
            handle,
            crate::api::events::Event::CallFailed {
                call_id: handle.session_id().clone(),
                status_code: 488,
                reason: reason.to_string(),
            },
        );
        self.terminate_confirmed_negotiation(handle, reason).await
    }

    async fn terminate_confirmed_negotiation(
        &self,
        handle: &SessionRegistryHandle,
        reason: &str,
    ) -> Result<()> {
        self.state_machine
            .process_event_exact(handle, EventType::ConfirmedNegotiationFailure)
            .await
            .map_err(|error| {
                SessionError::InvalidTransition(format!(
                    "failed to terminate confirmed dialog after negotiation failure ({reason}): {error}"
                ))
            })?;
        Ok(())
    }

    async fn handle_auth_required_parts(
        &self,
        session_id: SessionId,
        transaction_id: String,
        request_uri: String,
        status: u16,
        challenge: String,
        method: String,
        outbound_transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
        handle: SessionRegistryHandle,
        exact_replay: bool,
    ) -> Result<()> {
        if handle.session_id() != &session_id
            || self
                .state_machine
                .store
                .get_session_snapshot_exact(&handle)
                .is_err()
        {
            debug!(
                "Ignoring AuthRequired for session {} - exact lifetime ended",
                session_id
            );
            return Ok(());
        }

        let tracked_method = TrackedInDialogMethod::from_label(&method).filter(|tracked_method| {
            *tracked_method != TrackedInDialogMethod::Reinvite
                || self
                    .dialog_adapter
                    .outbound_request_tracker
                    .has_request(&handle, TrackedInDialogMethod::Reinvite)
        });
        let exact_transaction = if tracked_method.is_some() || method.eq_ignore_ascii_case("BYE") {
            if transaction_id.is_empty() || request_uri.is_empty() {
                warn!(
                    session_id = %session_id,
                    method = safe_auth_method_label(&method),
                    "Ignoring in-dialog authentication challenge without exact correlation"
                );
                return Ok(());
            }
            let Ok(transaction) =
                transaction_id.parse::<rvoip_sip_dialog::transaction::TransactionKey>()
            else {
                warn!(
                    session_id = %session_id,
                    method = safe_auth_method_label(&method),
                    "Ignoring in-dialog authentication challenge with invalid correlation"
                );
                return Ok(());
            };
            Some(transaction)
        } else {
            None
        };

        if let (Some(tracked_method), Some(transaction)) =
            (tracked_method, exact_transaction.as_ref())
        {
            if !exact_replay {
                let deferred_event = DeferredTrackedRequestEvent::AuthRequired {
                    handle: handle.clone(),
                    transaction_id: transaction_id.clone(),
                    request_uri: request_uri.clone(),
                    status,
                    challenge: challenge.clone(),
                    method: method.clone(),
                    outbound_transport: outbound_transport.clone(),
                };
                match self
                    .dialog_adapter
                    .outbound_request_tracker
                    .correlate_or_defer(&handle, tracked_method, transaction, deferred_event)
                {
                    ExactTransactionLookup::Matched => {}
                    ExactTransactionLookup::Prepared => return Ok(()),
                    ExactTransactionLookup::Mismatched => {
                        debug!(
                            session_id = %session_id,
                            method = safe_auth_method_label(&method),
                            "Ignoring stale or foreign in-dialog authentication challenge"
                        );
                        return Ok(());
                    }
                    ExactTransactionLookup::Rejected => {
                        return Err(anyhow::anyhow!(
                            "in-dialog authentication replay admission rejected"
                        ));
                    }
                }
            }
        } else if method.eq_ignore_ascii_case("BYE")
            && !exact_transaction.as_ref().is_some_and(|transaction| {
                self.dialog_adapter
                    .outgoing_bye_transaction_matches_exact(&handle, transaction)
            })
        {
            debug!(
                session_id = %session_id,
                "Ignoring stale or foreign BYE authentication challenge"
            );
            return Ok(());
        }

        let is_session_refresh_request = match (tracked_method, exact_transaction.as_ref()) {
            (Some(TrackedInDialogMethod::Update), Some(transaction)) => self
                .dialog_adapter
                .outbound_request_tracker
                .is_session_timer_update(&handle, transaction),
            (Some(TrackedInDialogMethod::Reinvite), Some(transaction)) => self
                .dialog_adapter
                .outbound_request_tracker
                .is_session_timer_reinvite(&handle, transaction),
            _ => false,
        };
        let exact_transaction_id = exact_transaction.as_ref().map(ToString::to_string);
        let exact_request_uri = (!request_uri.is_empty()).then_some(request_uri);
        let outcome = process_auth_required_on_fresh_task(
            Arc::clone(&self.state_machine),
            handle.clone(),
            status,
            challenge,
            method,
            AuthRequiredStateInput::new(
                outbound_transport,
                exact_transaction_id,
                exact_request_uri,
            ),
        )
        .await
        .map_err(|_| anyhow::anyhow!("session lifetime ended before AuthRequired dispatch"))?;

        if let Err(e) = outcome.result {
            let failure_class = OutboundAuthTerminalClass::from_error(e.as_ref());
            if is_missing_credentials_for_auth_error(e.as_ref()) {
                debug!(
                    "Failed to process AuthRequired({}) for session {} (class={})",
                    status,
                    session_id,
                    failure_class.label()
                );
            } else {
                error!(
                    "Failed to process AuthRequired({}) for session {} (class={})",
                    status,
                    session_id,
                    failure_class.label()
                );
            }
            if matches!(
                outcome.state_before_auth,
                crate::types::CallState::Initiating
            ) {
                self.handle_call_failed_parts(
                    &handle,
                    status,
                    CallFailureReason::OutboundInviteAuth(failure_class),
                    None,
                )
                .await?;
            } else if let (Some(tracked_method), Some(transaction)) =
                (tracked_method, exact_transaction.as_ref())
            {
                self.dialog_adapter.outbound_request_tracker.abort_matching(
                    &handle,
                    tracked_method,
                    transaction,
                );
                self.clear_tracked_request_auth_state(&handle, tracked_method, transaction)
                    .await;
                if is_session_refresh_request {
                    let input = match tracked_method {
                        TrackedInDialogMethod::Update => SessionRefreshStateInput::UpdateFailed,
                        TrackedInDialogMethod::Reinvite => SessionRefreshStateInput::ReinviteFailed,
                        TrackedInDialogMethod::Refer
                        | TrackedInDialogMethod::Notify
                        | TrackedInDialogMethod::Info => {
                            unreachable!("only RFC 4028 tracker methods set the refresh marker")
                        }
                    };
                    if let Err(error) = self
                        .state_machine
                        .process_session_refresh_exact(&handle, input)
                        .await
                    {
                        debug!(
                            session_id = %session_id,
                            %error,
                            "stale RFC 4028 authentication failure was suppressed"
                        );
                    }
                } else if matches!(
                    tracked_method,
                    TrackedInDialogMethod::Update | TrackedInDialogMethod::Reinvite
                ) {
                    let pending_matches = self
                        .state_machine
                        .store
                        .get_session_snapshot_exact(&handle)
                        .ok()
                        .is_some_and(|snapshot| {
                            let pending = snapshot.pending_offer_answer.as_ref();
                            correlate_pending_offer_transaction(
                                pending.map(|offer| &offer.method),
                                pending.and_then(|offer| offer.transaction_id.as_ref()),
                                &tracked_method.as_sip_method(),
                                Some(transaction),
                            ) == PendingOfferTransactionCorrelation::Exact
                        });
                    if pending_matches {
                        self.state_machine
                            .process_event_exact(&handle, EventType::Dialog4xxFailure(status))
                            .await
                            .map_err(|error| {
                                SessionError::InvalidTransition(format!(
                                    "failed to roll back an authenticated session modification: {error}"
                                ))
                            })?;
                        self.app_event_publisher.publish_exact(
                            &handle,
                            crate::api::events::Event::RenegotiationFailed {
                                call_id: session_id.clone(),
                                method: tracked_method.as_sip_method().to_string(),
                                reason: format!(
                                    "authentication failed (class={})",
                                    failure_class.label()
                                ),
                            },
                        );
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_outbound_request_completed_parts(
        &self,
        session_id: SessionId,
        transaction_id: &str,
        method: &str,
        outcome: OutboundRequestOutcome,
        response_sdp: Option<String>,
        handle: SessionRegistryHandle,
        exact_replay: bool,
    ) -> Result<()> {
        if handle.session_id() != &session_id
            || self
                .state_machine
                .store
                .get_session_snapshot_exact(&handle)
                .is_err()
        {
            debug!(
                session_id = %session_id,
                method = safe_auth_method_label(method),
                "Ignoring outbound request completion after exact session removal"
            );
            return Ok(());
        }
        if method.eq_ignore_ascii_case("BYE") {
            let transaction = transaction_id
                .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
                .map_err(|_| anyhow::anyhow!("outbound BYE completion has invalid correlation"))?;
            return self
                .handle_outbound_bye_completed_parts(handle, &transaction, outcome)
                .await;
        }
        let Some(tracked_method) = TrackedInDialogMethod::from_label(method) else {
            return Ok(());
        };
        let Ok(transaction) =
            transaction_id.parse::<rvoip_sip_dialog::transaction::TransactionKey>()
        else {
            warn!(
                session_id = %session_id,
                method = safe_auth_method_label(method),
                "Ignoring outbound request completion with invalid correlation"
            );
            return Ok(());
        };
        if !exact_replay {
            let deferred_event = DeferredTrackedRequestEvent::Completed {
                handle: handle.clone(),
                transaction_id: transaction_id.to_string(),
                method: method.to_string(),
                outcome,
                response_sdp: response_sdp.clone(),
            };
            match self
                .dialog_adapter
                .outbound_request_tracker
                .correlate_or_defer(&handle, tracked_method, &transaction, deferred_event)
            {
                ExactTransactionLookup::Prepared => return Ok(()),
                ExactTransactionLookup::Mismatched => {
                    debug!(
                        session_id = %session_id,
                        method = safe_auth_method_label(method),
                        "Ignoring stale or foreign outbound request completion"
                    );
                    return Ok(());
                }
                ExactTransactionLookup::Rejected => {
                    return Err(anyhow::anyhow!(
                        "in-dialog completion replay admission rejected"
                    ));
                }
                ExactTransactionLookup::Matched => {}
            }
        }
        let is_session_timer_update = tracked_method == TrackedInDialogMethod::Update
            && self
                .dialog_adapter
                .outbound_request_tracker
                .is_session_timer_update(&handle, &transaction);
        let is_session_timer_reinvite = tracked_method == TrackedInDialogMethod::Reinvite
            && self
                .dialog_adapter
                .outbound_request_tracker
                .is_session_timer_reinvite(&handle, &transaction);
        let pending_offer_correlation = self
            .state_machine
            .store
            .get_session_snapshot_exact(&handle)
            .ok()
            .map(|snapshot| {
                let pending = snapshot.pending_offer_answer.as_ref();
                correlate_pending_offer_transaction(
                    pending.map(|offer| &offer.method),
                    pending.and_then(|offer| offer.transaction_id.as_ref()),
                    &tracked_method.as_sip_method(),
                    Some(&transaction),
                )
            })
            .unwrap_or(PendingOfferTransactionCorrelation::NoPendingOffer);
        if pending_offer_correlation == PendingOfferTransactionCorrelation::Mismatched {
            if self
                .dialog_adapter
                .outbound_request_tracker
                .complete_if_matches(&handle, tracked_method, &transaction)
            {
                self.clear_tracked_request_auth_state(&handle, tracked_method, &transaction)
                    .await;
            }
            warn!(
                session_id = %session_id,
                method = safe_auth_method_label(method),
                "Ignoring an answer that does not own the pending offer transaction"
            );
            return Ok(());
        }
        if !self
            .dialog_adapter
            .outbound_request_tracker
            .complete_if_matches(&handle, tracked_method, &transaction)
        {
            debug!(
                session_id = %session_id,
                method = safe_auth_method_label(method),
                "Exact outbound request changed before completion cleanup"
            );
            return Ok(());
        };

        self.clear_tracked_request_auth_state(&handle, tracked_method, &transaction)
            .await;
        debug!(
            session_id = %session_id,
            method = safe_auth_method_label(method),
            outcome = outbound_request_outcome_label(outcome),
            "Released exact in-dialog request snapshot"
        );
        let refresh_input = if is_session_timer_update {
            Some(match outcome {
                OutboundRequestOutcome::FinalResponse { status_code }
                    if (200..300).contains(&status_code) =>
                {
                    SessionRefreshStateInput::UpdateSucceeded
                }
                OutboundRequestOutcome::FinalResponse { .. }
                | OutboundRequestOutcome::Timeout
                | OutboundRequestOutcome::TransportFailure => {
                    SessionRefreshStateInput::UpdateFailed
                }
            })
        } else if is_session_timer_reinvite {
            Some(match outcome {
                OutboundRequestOutcome::FinalResponse { status_code }
                    if (200..300).contains(&status_code) =>
                {
                    SessionRefreshStateInput::ReinviteSucceeded
                }
                OutboundRequestOutcome::FinalResponse { .. }
                | OutboundRequestOutcome::Timeout
                | OutboundRequestOutcome::TransportFailure => {
                    SessionRefreshStateInput::ReinviteFailed
                }
            })
        } else {
            None
        };
        if let Some(input) = refresh_input {
            let phase_matches =
                self.state_machine
                    .store
                    .get_session_snapshot_exact(&handle)
                    .ok()
                    .is_some_and(|snapshot| match input {
                        SessionRefreshStateInput::UpdateSucceeded
                        | SessionRefreshStateInput::UpdateFailed => {
                            snapshot.session_refresh_phase
                                == crate::session_store::state::SessionRefreshPhase::UpdateInFlight
                        }
                        SessionRefreshStateInput::ReinviteSucceeded
                        | SessionRefreshStateInput::ReinviteFailed => snapshot
                            .session_refresh_phase
                            == crate::session_store::state::SessionRefreshPhase::ReinviteInFlight,
                        SessionRefreshStateInput::UpdateDue { .. }
                        | SessionRefreshStateInput::ReinviteDue { .. }
                        | SessionRefreshStateInput::PeerExpired { .. } => false,
                    });
            if phase_matches {
                if let Err(error) = self
                    .state_machine
                    .process_session_refresh_exact(&handle, input)
                    .await
                {
                    debug!(
                        session_id = %session_id,
                        %error,
                        "stale RFC 4028 completion was suppressed"
                    );
                }
            }
        } else if tracked_method == TrackedInDialogMethod::Update
            && pending_offer_correlation == PendingOfferTransactionCorrelation::Exact
        {
            let (mut completion_event, mut failure_reason) = update_completion_transition(outcome);
            let successful_response = matches!(completion_event, EventType::Dialog200OK);
            let remote_answer = if successful_response {
                match response_sdp.filter(|sdp| !sdp.trim().is_empty()) {
                    Some(answer) => Some(answer),
                    None => {
                        completion_event = EventType::Dialog4xxFailure(488);
                        failure_reason = Some("successful UPDATE response contained no SDP answer");
                        None
                    }
                }
            } else {
                None
            };
            if let Some(answer) = remote_answer {
                self.state_machine
                    .process_event_with_remote_sdp_exact(&handle, completion_event, Some(answer))
                    .await
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            } else {
                self.state_machine
                    .process_event_exact(&handle, completion_event)
                    .await
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            }
            if let Some(reason) = failure_reason {
                self.app_event_publisher.publish_exact(
                    &handle,
                    crate::api::events::Event::RenegotiationFailed {
                        call_id: session_id.clone(),
                        method: "UPDATE".to_string(),
                        reason: reason.to_string(),
                    },
                );
            }
        } else if tracked_method == TrackedInDialogMethod::Reinvite
            && pending_offer_correlation == PendingOfferTransactionCorrelation::Exact
        {
            if let Some((failure_event, reason)) = reinvite_completion_failure(outcome) {
                self.state_machine
                    .process_event_exact(&handle, failure_event)
                    .await
                    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                self.app_event_publisher.publish_exact(
                    &handle,
                    crate::api::events::Event::RenegotiationFailed {
                        call_id: session_id,
                        method: "INVITE".to_string(),
                        reason: reason.to_string(),
                    },
                );
            }
        }
        Ok(())
    }

    async fn handle_outbound_bye_completed_parts(
        &self,
        handle: SessionRegistryHandle,
        transaction: &rvoip_sip_dialog::transaction::TransactionKey,
        outcome: OutboundRequestOutcome,
    ) -> Result<()> {
        if transaction.is_server() || transaction.method() != &rvoip_sip_core::Method::Bye {
            return Err(anyhow::anyhow!(
                "outbound BYE completion does not identify a client BYE transaction"
            ));
        }
        if !self
            .dialog_adapter
            .outgoing_bye_completion_is_current_or_unretained_exact(&handle, transaction)
        {
            debug!(
                session = %handle.session_id(),
                "Ignoring stale outbound BYE completion after a newer retry"
            );
            return Ok(());
        }
        debug!(
            session = %handle.session_id(),
            outcome = outbound_request_outcome_label(outcome),
            "Committing exact outbound BYE lifecycle completion"
        );

        let state_before_completion = self
            .state_machine
            .store
            .get_session_snapshot_exact(&handle)
            .ok()
            .map(|snapshot| snapshot.call_state);
        let lifecycle = crate::api::unified::commit_local_bye_lifecycle_exact(
            Arc::clone(&self.state_machine),
            &handle,
        )
        .await
        .map_err(anyhow::Error::new)?;
        let api_event = match lifecycle {
            crate::api::unified::LocalByeLifecycleCommit::AlreadyReleased => return Ok(()),
            crate::api::unified::LocalByeLifecycleCommit::Cancelled => {
                crate::api::events::Event::CallCancelled {
                    call_id: handle.session_id().clone(),
                }
            }
            crate::api::unified::LocalByeLifecycleCommit::Ended => {
                let reason = if matches!(state_before_completion, Some(CallState::Terminating)) {
                    "Local hangup"
                } else {
                    "Local BYE"
                };
                crate::api::events::Event::CallEnded {
                    call_id: handle.session_id().clone(),
                    reason: reason.to_string(),
                }
            }
        };
        self.publish_and_release_session(api_event, handle).await;
        Ok(())
    }

    async fn clear_tracked_request_auth_state(
        &self,
        handle: &SessionRegistryHandle,
        method: TrackedInDialogMethod,
        transaction: &rvoip_sip_dialog::transaction::TransactionKey,
    ) {
        let transaction_id = transaction.to_string();
        match self
            .state_machine
            .clear_tracked_request_auth_state_exact(handle, &transaction_id)
            .await
        {
            Ok(true) => {}
            Ok(false) => {
                debug!(
                    session_id = %handle.session_id(),
                    method = ?method,
                    "Exact in-dialog auth owner changed before completion cleanup"
                );
            }
            Err(_) => {
                debug!(
                    session_id = %handle.session_id(),
                    method = ?method,
                    "Exact in-dialog request completed after session removal"
                );
            }
        }
    }

    // Dialog event handlers

    async fn handle_call_failed_parts(
        &self,
        handle: &SessionRegistryHandle,
        status: u16,
        reason: CallFailureReason,
        raw_response: Option<bytes::Bytes>,
    ) -> Result<()> {
        let session_id = handle.session_id().clone();
        let Ok(initial_snapshot) = self.state_machine.store.get_session_snapshot_exact(handle)
        else {
            debug!(
                "Ignoring CallFailed for session {} - not in our store",
                session_id
            );
            return Ok(());
        };

        let response_transaction = response_from_bytes(raw_response.as_ref())
            .as_ref()
            .and_then(rvoip_sip_dialog::transaction::TransactionKey::from_response);
        let pending_offer = initial_snapshot.pending_offer_answer.as_ref();
        let pending_correlation = correlate_pending_offer_transaction(
            pending_offer.map(|pending| &pending.method),
            pending_offer.and_then(|pending| pending.transaction_id.as_ref()),
            &rvoip_sip_core::Method::Invite,
            response_transaction.as_ref(),
        );
        if matches!(
            pending_correlation,
            PendingOfferTransactionCorrelation::OtherMethod
                | PendingOfferTransactionCorrelation::Mismatched
        ) {
            warn!(
                session_id = %session_id,
                status,
                "Ignoring an INVITE failure that does not own the pending offer transaction"
            );
            return Ok(());
        }

        if initial_snapshot.session_refresh_phase
            == crate::session_store::state::SessionRefreshPhase::ReinviteInFlight
        {
            if let Err(error) = self
                .state_machine
                .process_session_refresh_exact(handle, SessionRefreshStateInput::ReinviteFailed)
                .await
            {
                debug!(
                    session_id = %session_id,
                    status,
                    %error,
                    "stale RFC 4028 re-INVITE failure was suppressed"
                );
            }
            return Ok(());
        }

        let reason = reason.into_event_reason();

        info!(
            "[handle_call_failed] session={} status={} {:?}",
            session_id,
            status,
            CallFailureDiagnostics::new(&reason)
        );

        // Drive the existing Dialog{4,5,6}xxFailure state transitions. 3xx
        // currently maps onto the 4xx path because the default state table
        // has no dedicated redirect transition; proper 3xx/redirect handling
        // is a separate feature.
        let event_type = match status {
            300..=499 => EventType::Dialog4xxFailure(status),
            500..=599 => EventType::Dialog5xxFailure(status),
            600..=699 => EventType::Dialog6xxFailure(status),
            _ => EventType::DialogError(format!("unexpected CallFailed status {}", status)),
        };

        let result = match self
            .state_machine
            .process_event_exact(handle, event_type)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                error!(
                    "Failed to process CallFailed({}) for session {}: {}",
                    status, session_id, e
                );
                return Err(SessionError::InvalidTransition(format!(
                    "CallFailed lifecycle transition failed for session {} role {:?} state {:?}: {}",
                    session_id, initial_snapshot.role, initial_snapshot.call_state, e
                ))
                .into());
            }
        };
        match committed_call_failure(&session_id, initial_snapshot.role, &result)? {
            CommittedCallFailure::NonTerminal => {
                debug!(
                    "session {} in-dialog request failed with {}; committed a non-terminal YAML rollback and retained the call",
                    session_id, status
                );
                if pending_correlation == PendingOfferTransactionCorrelation::Exact {
                    self.app_event_publisher.publish_exact(
                        handle,
                        crate::api::events::Event::RenegotiationFailed {
                            call_id: session_id.clone(),
                            method: "INVITE".to_string(),
                            reason: format!("re-INVITE failed with SIP status {status}"),
                        },
                    );
                }
                return Ok(());
            }
            CommittedCallFailure::Cancelled => {
                let api_event = crate::api::events::Event::CallCancelled {
                    call_id: session_id.clone(),
                };
                self.publish_and_release_session(api_event, handle.clone())
                    .await;
                return Ok(());
            }
            CommittedCallFailure::Failed => {}
        }

        let committed = match self.state_machine.store.get_session_snapshot_exact(handle) {
            Ok(snapshot) => snapshot,
            Err(_) => return Ok(()),
        };

        // Transfer-leg failure signaling is owned by the committed YAML row's
        // `SendTransferNotifyFailure` action. Keeping it out of this adapter
        // projection path avoids a second raw-ID NOTIFY writer and duplicate
        // `TransferFailed` observations.

        // Publish app-level CallFailed for any StreamPeer/CallbackPeer subscribers,
        // then release the session from the store + registry. Publish runs first
        // so subscribers receive the terminal event before the session vanishes.
        let [detailed, terminal] = project_committed_response_events(
            committed.as_ref(),
            CommittedResponseObservation::Failed {
                status_code: status,
                reason,
                raw_response,
            },
        );
        publish_api_event(&self.app_event_publisher, detailed);

        self.publish_and_release_session(terminal, handle.clone())
            .await;

        Ok(())
    }

    /// Handle a 3xx redirect response (RFC 3261 §8.1.3.4) with the
    /// typed cross-crate event payload. Bypasses the legacy debug-
    /// string parser: `status_code` and `targets` arrive as already-
    /// structured fields from `DialogToSessionEvent::CallRedirected`,
    /// which dialog-core's event hub builds straight from typed
    /// Contact headers (with q-values per RFC 3261 §20.10).
    async fn handle_call_redirected_typed(
        &self,
        handle: &SessionRegistryHandle,
        status_code: u16,
        targets: &[String],
        _q_values: &[f32],
    ) -> Result<()> {
        let session_id = handle.session_id().clone();

        if self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)
            .is_err()
        {
            debug!(
                "Ignoring CallRedirected for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        info!(
            session_id = %session_id,
            status_code,
            target_count = targets.len(),
            "Handling typed SIP redirect"
        );

        if targets.is_empty() {
            // No usable Contact URIs in the 3xx — fall back to the
            // generic failure path so the state machine tears the call
            // down cleanly instead of hanging waiting for a retry.
            warn!("3xx response with no Contact URIs — treating as failure");
            let _ = process_event_exact_on_fresh_task(
                Arc::clone(&self.state_machine),
                handle.clone(),
                EventType::Dialog4xxFailure(status_code),
            )
            .await;
            return Ok(());
        }

        if let Err(e) = process_event_exact_on_fresh_task(
            Arc::clone(&self.state_machine),
            handle.clone(),
            EventType::Dialog3xxRedirect {
                status: status_code,
                targets: targets.to_vec(),
            },
        )
        .await
        {
            error!(
                "Failed to process CallRedirected for session {}: {}",
                session_id, e
            );
        }

        Ok(())
    }

    async fn handle_session_interval_too_small_parts(
        &self,
        handle: &SessionRegistryHandle,
        min_se_secs: u32,
    ) -> Result<()> {
        let session_id = handle.session_id().clone();
        let Ok(snapshot) = self.state_machine.store.get_session_snapshot_exact(handle) else {
            debug!(
                "Ignoring SessionIntervalTooSmall for session {} - not in our store",
                session_id
            );
            return Ok(());
        };

        const CAP: u8 = 2;
        let current_retries = snapshot.session_timer_retry_count;
        let can_retry = min_se_secs > 0 && current_retries < CAP;

        if can_retry {
            match process_event_exact_on_fresh_task(
                Arc::clone(&self.state_machine),
                handle.clone(),
                EventType::SessionIntervalTooSmall { min_se_secs },
            )
            .await
            {
                Ok(result) => {
                    match committed_session_interval_retry(&session_id, snapshot.role, &result) {
                        Ok(()) => return Ok(()),
                        Err(error) => warn!(
                            session = %session_id,
                            %error,
                            "SessionIntervalTooSmall retry did not commit; applying the YAML failure path"
                        ),
                    }
                }
                Err(e) => {
                    error!(
                        "Failed to dispatch SessionIntervalTooSmall retry for session {}: {}",
                        session_id, e
                    );
                }
            }
        }

        let result = process_event_exact_on_fresh_task(
            Arc::clone(&self.state_machine),
            handle.clone(),
            EventType::Dialog4xxFailure(422),
        )
        .await
        .map_err(|error| {
            SessionError::InvalidTransition(format!(
                "SessionIntervalTooSmall fallback failed for session {} role {:?} state {:?}: {}",
                session_id, snapshot.role, snapshot.call_state, error
            ))
        })?;
        let api_event = match committed_call_failure(&session_id, snapshot.role, &result)? {
            CommittedCallFailure::NonTerminal => {
                debug!(
                    session = %session_id,
                    "422 fallback committed a non-terminal in-dialog rollback; retaining session"
                );
                return Ok(());
            }
            CommittedCallFailure::Cancelled => crate::api::events::Event::CallCancelled {
                call_id: session_id.clone(),
            },
            CommittedCallFailure::Failed => crate::api::events::Event::CallFailed {
                call_id: session_id.clone(),
                status_code: 422,
                reason: format!(
                    "Session Interval Too Small (required Min-SE: {}s)",
                    min_se_secs
                ),
            },
        };
        self.publish_and_release_session(api_event, handle.clone())
            .await;

        Ok(())
    }

    async fn handle_reinvite_glare_session(&self, handle: &SessionRegistryHandle) -> Result<()> {
        let session_id = handle.session_id();
        let snapshot = match self.state_machine.store.get_session_snapshot_exact(handle) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                debug!(
                    "Ignoring ReinviteGlare for session {} - not in our store",
                    session_id
                );
                return Ok(());
            }
        };

        if snapshot.session_refresh_phase
            == crate::session_store::state::SessionRefreshPhase::ReinviteInFlight
        {
            if let Err(error) = self
                .state_machine
                .process_session_refresh_exact(handle, SessionRefreshStateInput::ReinviteFailed)
                .await
            {
                debug!(
                    session_id = %session_id,
                    %error,
                    "stale RFC 4028 glare failure was suppressed"
                );
            }
            return Ok(());
        }

        if let Err(e) = self
            .state_machine
            .process_event_exact(handle, EventType::ReinviteGlare)
            .await
        {
            error!(
                "Failed to process ReinviteGlare for session {}: {}",
                session_id, e
            );
        }
        Ok(())
    }

    async fn handle_session_refreshed_parts(
        &self,
        handle: &SessionRegistryHandle,
        expires_secs: u32,
    ) -> Result<()> {
        debug!(
            session_id = %handle.session_id(),
            expires_secs,
            "Ignoring retired dialog-owned session refresh observation"
        );
        Ok(())
    }

    async fn handle_session_refresh_failed_parts(
        &self,
        handle: &SessionRegistryHandle,
        reason: String,
    ) -> Result<()> {
        debug!(
            session_id = %handle.session_id(),
            reason_present = !reason.is_empty(),
            "Ignoring retired dialog-owned session refresh failure observation"
        );
        Ok(())
    }

    async fn handle_outbound_flow_failed_parts(&self, aor: String, reason: String) -> Result<()> {
        let now = Instant::now();
        if let Some(prev) = self
            .outbound_flow_last_refresh
            .get(&aor)
            .map(|e| *e.value())
        {
            if now.duration_since(prev) < OUTBOUND_FLOW_REFRESH_DEBOUNCE {
                debug!(
                    aor_present = !aor.is_empty(),
                    aor_bytes = aor.len(),
                    reason_present = !reason.is_empty(),
                    reason_bytes = reason.len(),
                    elapsed_ms = now.duration_since(prev).as_millis(),
                    "OutboundFlowFailed debounced"
                );
                return Ok(());
            }
        }
        self.outbound_flow_last_refresh.insert(aor.clone(), now);

        let matching_handle = self.state_machine.store.sessions.iter().find_map(|entry| {
            let state = entry.value().snapshot();
            match state.local_uri.as_deref() {
                Some(uri) if uri == aor.as_str() => state.lifecycle_handle.clone(),
                _ => None,
            }
        });

        let Some(handle) = matching_handle else {
            warn!(
                aor_present = !aor.is_empty(),
                aor_bytes = aor.len(),
                "OutboundFlowFailed had no matching registration session; dropping"
            );
            return Ok(());
        };
        let session_id = handle.session_id().clone();

        if let Err(e) = self
            .state_machine
            .process_event_exact(&handle, EventType::RefreshRegistration)
            .await
        {
            warn!(
                "Failed to dispatch RefreshRegistration for session {} after flow failure: {}",
                session_id, e
            );
        }
        Ok(())
    }

    async fn handle_call_cancelled_session(&self, handle: &SessionRegistryHandle) -> Result<()> {
        let session_id = handle.session_id().clone();
        let (initial_role, initial_state) =
            match self.state_machine.store.get_session_snapshot_exact(handle) {
                Ok(initial) => (initial.role, initial.call_state),
                Err(_) => {
                    debug!(
                        "Ignoring CallCancelled for session {} - not in our store",
                        session_id
                    );
                    return Ok(());
                }
            };

        info!("🎯 [handle_call_cancelled] session={}", session_id);

        // The transaction/dialog owner already completed the wire mechanics.
        // Admit exactly one role-correct lifecycle fact into YAML: a UAC sees
        // its INVITE's 487; a UAS sees the matched inbound CANCEL. There is no
        // direct-publication fallback when the table rejects the event.
        let lifecycle_event = match initial_role {
            Role::UAC => EventType::Dialog487RequestTerminated,
            Role::UAS => EventType::DialogCANCEL,
            Role::Both => {
                return Err(SessionError::InvalidTransition(format!(
                    "CANCEL lifecycle for session {} has no concrete SIP role in state {:?}",
                    session_id, initial_state
                ))
                .into());
            }
        };
        let result = self
            .state_machine
            .process_event_exact(handle, lifecycle_event)
            .await
            .map_err(|error| {
                SessionError::InvalidTransition(format!(
                    "CANCEL lifecycle transition failed for session {} in state {:?}: {}",
                    session_id, initial_state, error
                ))
            })?;
        if result.transition.is_none()
            || !result
                .events_published
                .iter()
                .any(|event| matches!(event, EventTemplate::CallCancelled))
        {
            return Err(SessionError::InvalidTransition(format!(
                "CANCEL lifecycle for session {} in state {:?} did not commit CallCancelled",
                session_id, initial_state
            ))
            .into());
        }

        // Terminal publication and exact release follow only the committed
        // YAML outcome and remain independent of observational bus health.
        let api_event = crate::api::events::Event::CallCancelled {
            call_id: session_id.clone(),
        };
        self.publish_and_release_session(api_event, handle.clone())
            .await;

        Ok(())
    }

    async fn handle_call_progress_parts(
        &self,
        handle: &SessionRegistryHandle,
        status_code: u16,
        reason: String,
        sdp: Option<String>,
        raw_response: Option<bytes::Bytes>,
    ) -> Result<()> {
        let sid = handle.session_id().clone();
        let mut committed = match self.state_machine.store.get_session_snapshot_exact(handle) {
            Ok(snapshot) => snapshot,
            Err(_) => {
                debug!(
                    "Ignoring CallProgress for session {} - not in our store",
                    sid
                );
                return Ok(());
            }
        };

        let state_event = match status_code {
            183 if sdp.is_some() => Some(EventType::Dialog183SessionProgress),
            101..=199 => Some(EventType::Dialog180Ringing),
            _ => None,
        };

        if let Some(event_type) = state_event {
            let result = match self
                .state_machine
                .process_event_with_remote_sdp_exact(handle, event_type, sdp.clone())
                .await
            {
                Ok(result) => result,
                Err(e) => {
                    error!("Failed to process CallProgress for {}: {}", sid, e);
                    return Ok(());
                }
            };
            if result.transition.is_none() {
                debug!(
                    session = %sid,
                    status_code,
                    "Suppressing duplicate CallProgress projection without a committed transition"
                );
                return Ok(());
            }
            committed = match self.state_machine.store.get_session_snapshot_exact(handle) {
                Ok(snapshot) => snapshot,
                Err(_) => return Ok(()),
            };
        }

        let events = project_committed_response_events(
            committed.as_ref(),
            CommittedResponseObservation::Progress {
                status_code,
                reason,
                sdp,
                raw_response,
            },
        );
        publish_committed_api_projection(&self.app_event_publisher, handle, events);

        Ok(())
    }

    async fn handle_call_state_changed_parts(
        &self,
        handle: &SessionRegistryHandle,
        new_state: &rvoip_infra_common::events::cross_crate::CallState,
    ) -> Result<()> {
        let sid = handle.session_id();
        if self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)
            .is_err()
        {
            debug!(
                "Ignoring CallStateChanged for session {} - not in our store",
                sid
            );
            return Ok(());
        }

        match new_state {
            rvoip_infra_common::events::cross_crate::CallState::Ringing => {
                return self
                    .handle_call_progress_parts(handle, 180, "Ringing".to_string(), None, None)
                    .await;
            }
            rvoip_infra_common::events::cross_crate::CallState::Terminated => {
                // Dialog StateChanged is observational. DialogEvent::Terminated
                // follows as the typed causal terminal fact and is the only
                // path allowed to commit DialogTerminated, publish, and
                // release. Pre-committing here would make that authoritative
                // event look like a duplicate and strand the exact lifetime.
                debug!(
                    session = %sid,
                    "Ignoring observational terminated state; awaiting typed CallTerminated"
                );
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_call_terminated_parts(
        &self,
        handle: &SessionRegistryHandle,
        reason: String,
    ) -> Result<()> {
        let session_id = handle.session_id().clone();
        let initial = match self.state_machine.store.get_session_snapshot_exact(handle) {
            Ok(initial) => initial,
            Err(_) => {
                debug!(
                    "Ignoring CallTerminated for session {} - not in our store",
                    session_id
                );
                return Ok(());
            }
        };

        info!(
            "🎯 [handle_call_terminated] Processing DialogTerminated for session {} with reason: {}",
            session_id, reason
        );

        // Process DialogTerminated to complete Terminating → Terminated, or
        // Cancelling → Cancelled for the late-200/ACK/BYE cleanup path. The
        // exact YAML result is the only authority for terminal publication
        // and release; a rejected or missing transition remains retained.
        let result = self
            .state_machine
            .process_event_exact(handle, EventType::DialogTerminated)
            .await
            .map_err(|error| {
                SessionError::InvalidTransition(format!(
                    "DialogTerminated lifecycle transition failed for session {} role {:?} state {:?}: {}",
                    session_id, initial.role, initial.call_state, error
                ))
            })?;
        let outcome = committed_dialog_termination(&session_id, initial.role, &result)?;
        let api_event = match outcome {
            CommittedDialogTermination::Ended => crate::api::events::Event::CallEnded {
                call_id: session_id.clone(),
                reason,
            },
            CommittedDialogTermination::Cancelled => crate::api::events::Event::CallCancelled {
                call_id: session_id.clone(),
            },
        };
        info!(
            "✅ [handle_call_terminated] committed {:?} for {}",
            outcome, session_id
        );
        self.publish_and_release_session(api_event, handle.clone())
            .await;

        Ok(())
    }

    async fn handle_bye_received_parts(&self, handle: &SessionRegistryHandle) -> Result<()> {
        let session_id = handle.session_id().clone();
        let initial_state = match self.state_machine.store.get_session_snapshot_exact(handle) {
            Ok(snapshot) => snapshot.call_state,
            Err(_) => {
                rvoip_sip_dialog::diagnostics::record_bye_cleanup_session_missing();
                debug!(
                    "Ignoring ByeReceived for session {} - not in our store",
                    session_id
                );
                return Ok(());
            }
        };

        rvoip_sip_dialog::diagnostics::record_bye_cleanup_delivered();
        let bye_guard = cleanup_diag::stage_guard(CleanupStage::ByeReceivedHandling, &session_id.0);
        let result = process_event_exact_on_fresh_task(
            Arc::clone(&self.state_machine),
            handle.clone(),
            EventType::DialogBYE,
        )
        .await
        .map_err(|error| {
            SessionError::InvalidTransition(format!(
                "DialogBYE lifecycle transition failed for session {} state {:?}: {}",
                session_id, initial_state, error
            ))
        })?;
        committed_bye_termination(&session_id, &result)?;
        let api_event = crate::api::events::Event::CallEnded {
            call_id: session_id.clone(),
            reason: "BYE received".to_string(),
        };
        self.publish_and_release_session(api_event, handle.clone())
            .await;
        bye_guard.finish_success();

        Ok(())
    }

    async fn handle_dialog_error_parts(
        &self,
        handle: &SessionRegistryHandle,
        error: String,
    ) -> Result<()> {
        let sid = handle.session_id();
        if self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)
            .is_err()
        {
            debug!(
                "Ignoring DialogError for session {} - not in our store",
                sid
            );
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event_exact(handle, EventType::DialogError(error))
            .await
        {
            error!("Failed to process dialog error: {}", e);
        }
        Ok(())
    }

    async fn handle_dtmf_received_parts(
        &self,
        handle: &SessionRegistryHandle,
        tones: String,
    ) -> Result<()> {
        if self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)
            .is_err()
        {
            return Ok(());
        }
        let sid = handle.session_id();
        for digit in tones.chars() {
            self.app_event_publisher.publish_exact(
                handle,
                crate::api::events::Event::DtmfReceived {
                    call_id: sid.clone(),
                    digit,
                },
            );
        }
        Ok(())
    }

    async fn handle_dialog_state_changed_parts(
        &self,
        handle: &SessionRegistryHandle,
        old_state: String,
        new_state: String,
    ) -> Result<()> {
        if self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)
            .is_err()
        {
            return Ok(());
        }
        if let Err(e) = self
            .state_machine
            .process_event_exact(
                handle,
                EventType::DialogStateChanged {
                    old_state,
                    new_state,
                },
            )
            .await
        {
            error!("Failed to process DialogStateChanged: {}", e);
        }
        Ok(())
    }

    async fn handle_reinvite_received_parts(
        &self,
        handle: &SessionRegistryHandle,
        sdp: Option<String>,
        method: String,
        inbound_response: InboundResponseStateInput,
    ) -> Result<()> {
        let snapshot = self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)
            .map_err(|_| {
                anyhow::anyhow!(
                    "ReinviteReceived/UpdateReceived exact session lifetime ended before response ownership was accepted"
                )
            })?;
        let previous_remote_direction = Some(snapshot.remote_media_direction);
        let has_sdp = sdp.is_some();
        if let Some(offer) = sdp.as_deref() {
            if let Err(error) = self.media_adapter.validate_inbound_sdp_offer(offer) {
                let mut response_input = Some(inbound_response);
                let terminal = crate::state_machine::actions::send_exact_inbound_final_response(
                    handle.session_id(),
                    response_input.as_mut(),
                    &self.dialog_adapter,
                    488,
                    None,
                    None,
                )
                .await
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                warn!(
                    session_id = %handle.session_id(),
                    method = %method,
                    error_class = %error,
                    "Rejected invalid inbound session-modification offer"
                );
                if let Some(error) = terminal.terminal_error {
                    return Err(anyhow::anyhow!(error.to_string()));
                }
                return Ok(());
            }
        }
        let event = if method.eq_ignore_ascii_case("UPDATE") {
            EventType::UpdateReceived { sdp }
        } else {
            EventType::ReinviteReceived { sdp }
        };
        let process_result = self
            .state_machine
            .process_inbound_response_event_exact(handle, event, inbound_response)
            .await;
        if let Err(error) = process_result {
            let disposition = crate::state_machine::actions::exact_sip_response_failure_disposition(
                error.as_ref(),
            );
            let detail = error.to_string();
            exact_response_failure_processing_ack(
                "ReinviteReceived/UpdateReceived",
                disposition,
                &detail,
            )?;
            // Written and wire-unknown are both terminal transaction facts.
            // The executor committed their YAML transition before surfacing
            // the diagnostic error, so acknowledge them without authoring a
            // competing response.
            warn!(
                method = %method,
                ?disposition,
                "Inbound re-INVITE/UPDATE response reached a terminal wire disposition"
            );
        }
        if has_sdp {
            self.apply_inbound_reinvite_media_direction(handle, previous_remote_direction)
                .await;
        }
        Ok(())
    }

    async fn apply_inbound_reinvite_media_direction(
        &self,
        handle: &SessionRegistryHandle,
        previous_remote_direction: Option<crate::types::MediaDirection>,
    ) {
        let Ok(snapshot) = self.state_machine.store.get_session_snapshot_exact(handle) else {
            return;
        };
        let sid = handle.session_id();
        let media_session_id = snapshot.media_session_id.clone();
        let local_media_direction = snapshot.local_media_direction;

        if let Some(media_id) = media_session_id {
            if let Err(e) = self
                .media_adapter
                .set_media_direction(media_id, local_media_direction)
                .await
            {
                error!(
                    "Failed to apply inbound re-INVITE media direction for session {}: {}",
                    sid, e
                );
            }
        }

        let Some(previous_remote_direction) = previous_remote_direction else {
            return;
        };
        let Ok(current) = self.state_machine.store.get_session_snapshot_exact(handle) else {
            return;
        };
        let remote_media_direction = current.remote_media_direction;
        let was_remote_held = remote_direction_is_hold(previous_remote_direction);
        let is_remote_held = remote_direction_is_hold(remote_media_direction);

        let api_event = match (was_remote_held, is_remote_held) {
            (false, true) => Some(crate::api::events::Event::RemoteCallOnHold {
                call_id: sid.clone(),
            }),
            (true, false) => Some(crate::api::events::Event::RemoteCallResumed {
                call_id: sid.clone(),
            }),
            _ => None,
        };

        if let Some(api_event) = api_event {
            self.app_event_publisher.publish_exact(handle, api_event);
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_transfer_requested_parts(
        &self,
        lifecycle_handle: &SessionRegistryHandle,
        refer_to: String,
        transfer_type: String,
        transaction_id: String,
        referred_by: Option<String>,
        replaces: Option<String>,
        raw_request: Option<bytes::Bytes>,
        transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
    ) -> Result<()> {
        let session_id = lifecycle_handle.session_id().clone();
        if self
            .state_machine
            .store
            .get_session_snapshot_exact(lifecycle_handle)
            .is_err()
        {
            return Err(anyhow::anyhow!(
                "TransferRequested exact session lifetime ended before response ownership was accepted"
            ));
        }

        self.state_machine
            .stage_transfer_request_exact(
                lifecycle_handle,
                TransferRequestStateInput::new(
                    refer_to.clone(),
                    transaction_id.clone(),
                    referred_by.clone(),
                    replaces.clone(),
                ),
            )
            .await
            .map_err(|error| anyhow::anyhow!("failed to record exact transfer request: {error}"))?;

        // SIP_API_DESIGN_2 Phase E: re-parse the inbound REFER bytes
        // into a typed `IncomingRequest`. The coordinator hook stays
        // `None` on the bus path; the surface consumer rehydrates it
        // before dispatching to application code.
        let request = build_incoming_request_from_bytes(session_id.clone(), raw_request, transport);

        // Publish ReferReceived event to the global coordinator's "session_to_app" channel.
        debug!("🔍 [DEBUG] Publishing ReferReceived event to global coordinator");
        self.app_event_publisher.publish_exact(
            lifecycle_handle,
            crate::api::events::Event::ReferReceived {
                call_id: session_id.clone(),
                refer_to: refer_to.clone(),
                referred_by: referred_by.clone(),
                replaces: replaces.clone(),
                transaction_id: transaction_id.clone(),
                transfer_type: transfer_type.clone(),
                request,
            },
        );

        let state_machine = Arc::clone(&self.state_machine);
        let authority = Arc::clone(state_machine.store.authority());
        let operation_key = lifecycle_handle.key().clone();
        let lifecycle_handle = lifecycle_handle.clone();
        let refer_to_for_default = refer_to.clone();
        let transfer_type_for_default = transfer_type.clone();
        let transaction_for_default = transaction_id.clone();
        let hard_timeout = REFER_DEFAULT_ACTION_DELAY
            .saturating_add(self.dialog_adapter.non_invite_transaction_timeout())
            .saturating_add(REFER_DEFAULT_ACTION_COMPLETION_GRACE);
        let scheduled = authority
            .spawn_owned_exact(
                &operation_key,
                SessionOperationKind::Signaling,
                hard_timeout,
                move |operation| async move {
                    let Some(mut cancellation) = operation.cancellation() else {
                        return rollback_owned_delayed_signaling(
                            operation,
                            Err(
                                "retained REFER default action has no exact cancellation authority"
                                    .to_string(),
                            ),
                        )
                        .await;
                    };
                    tokio::select! {
                        _ = tokio::time::sleep(REFER_DEFAULT_ACTION_DELAY) => {}
                        () = wait_for_owned_operation_cancellation(&mut cancellation) => {
                            return rollback_owned_delayed_signaling(
                                operation,
                                Err("retained REFER default action was cancelled before a classified final response".to_string()),
                            ).await;
                        }
                    }

                    let current = match state_machine
                        .store
                        .get_session_snapshot_exact(&lifecycle_handle)
                    {
                        Ok(current) => current,
                        Err(error) => {
                            return rollback_owned_delayed_signaling(
                                operation,
                                Err(format!(
                                    "retained REFER exact lifetime ended before a classified final response: {error}"
                                )),
                            )
                            .await;
                        }
                    };

                    if current.refer_transaction_id.as_deref()
                        != Some(transaction_for_default.as_str())
                    {
                        // The only writers that retire this staged exact
                        // transaction do so after Written/WireUnknown. An app
                        // response won the race, so the causal delivery is
                        // already terminal and the delayed default is a no-op.
                        return rollback_owned_delayed_signaling(operation, Ok(())).await;
                    }

                    let dispatch = state_machine
                        .process_event_exact(
                            &lifecycle_handle,
                            EventType::TransferRequested {
                                refer_to: refer_to_for_default,
                                transfer_type: transfer_type_for_default,
                                transaction_id: transaction_for_default.clone(),
                            },
                        )
                        .await;
                    if let Err(error) = dispatch {
                        let disposition =
                            crate::state_machine::actions::exact_sip_response_failure_disposition(
                                error.as_ref(),
                            );
                        let detail = error.to_string();
                        tracing::error!(
                            session_id = %lifecycle_handle.session_id(),
                            ?disposition,
                            "Failed to auto-accept pending TransferRequested: {detail}"
                        );
                        if exact_response_failure_processing_ack(
                            "TransferRequested",
                            disposition,
                            &detail,
                        )
                        .is_ok()
                        {
                            return commit_owned_delayed_signaling_ack(operation).await;
                        }

                        // An application response can commit while this
                        // delayed dispatch waits for the state-machine lane.
                        // Recheck the exact staged key before asking the lower
                        // owner for a fallback.
                        if state_machine
                            .store
                            .get_session_snapshot_exact(&lifecycle_handle)
                            .is_ok_and(|current| {
                                current.refer_transaction_id.as_deref()
                                    != Some(transaction_for_default.as_str())
                            })
                        {
                            return rollback_owned_delayed_signaling(operation, Ok(())).await;
                        }

                        return rollback_owned_delayed_signaling(
                            operation,
                            Err(format!(
                                "retained REFER default action failed before a classified final response: {detail}"
                            )),
                        )
                        .await;
                    }
                    // SendReferAccepted clears the matching transaction on the
                    // lane-owned working state; the transition's canonical commit
                    // is the only writer. No post-transition repair write is
                    // needed here.
                    commit_owned_delayed_signaling_ack(operation).await
                },
            )
            .map_err(|error| {
                anyhow::anyhow!(
                    "REFER default action was not admitted for exact session {session_id}: {error}"
                )
            })?;

        let processing_ack = scheduled.await.map_err(|error| {
            anyhow::anyhow!(
                "retained REFER default action ended before a classified final response: {error}"
            )
        })?;
        processing_ack.map_err(anyhow::Error::msg)
    }

    async fn handle_ack_received_session(
        &self,
        handle: &SessionRegistryHandle,
        sdp_answer: Option<String>,
    ) -> Result<()> {
        let session_id = handle.session_id();
        if self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)
            .is_err()
        {
            debug!(
                "Ignoring AckReceived for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        rvoip_sip_dialog::diagnostics::record_ack_event_delivered();
        let delayed_offer_answer = !self
            .state_machine
            .store
            .get_session_snapshot_exact(handle)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?
            .sdp_negotiated;
        let process_result = if delayed_offer_answer {
            self.state_machine
                .process_event_with_remote_sdp_exact(handle, EventType::DialogACK, sdp_answer)
                .await
        } else {
            self.state_machine
                .process_event_exact(handle, EventType::DialogACK)
                .await
        };
        if let Err(error) = process_result {
            error!(
                "Failed to process DialogACK event after AckReceived: {}",
                error
            );
            if delayed_offer_answer {
                self.app_event_publisher.publish_exact(
                    handle,
                    crate::api::events::Event::RenegotiationFailed {
                        call_id: session_id.clone(),
                        method: "ACK".to_string(),
                        reason: "missing, invalid, or unacceptable ACK SDP answer".to_string(),
                    },
                );
            }
            self.terminate_confirmed_negotiation(
                handle,
                if delayed_offer_answer {
                    "ACK carried a missing, invalid, or unusable SDP answer"
                } else {
                    "confirmed UAS dialog could not activate negotiated media"
                },
            )
            .await?;
        } else if let Some(coordinator) = self.coordinator.get().and_then(|w| w.upgrade()) {
            coordinator
                .schedule_active_call_media_timeout_if_current(session_id)
                .await;
        }
        Ok(())
    }

    async fn handle_registration_success_parts(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<()> {
        self.handle_state_event_if_ours(handle, EventType::Registration200OK, "RegistrationSuccess")
            .await
    }

    async fn handle_registration_failed_parts(
        &self,
        handle: &SessionRegistryHandle,
        status_code: u16,
    ) -> Result<()> {
        self.handle_state_event_if_ours(
            handle,
            EventType::RegistrationFailed(status_code),
            "RegistrationFailed",
        )
        .await
    }

    /// Handle `DialogToSessionEvent::NotifyReceived` (RFC 6665) — the
    /// cross-crate event dialog-core publishes after validating and
    /// 200-OK'ing an inbound NOTIFY.
    ///
    /// Always emits `Event::NotifyReceived` on the public event stream.
    /// For `event_package == "refer"` with a `message/sipfrag` body
    /// (RFC 3515 §2.4.5) additionally parses the sipfrag status line and
    /// emits `Event::ReferNotify` plus derived `ReferProgress`,
    /// `ReferCompleted`, or `TransferFailed` events so transferor apps
    /// (including b2bua wrappers) can observe the transferee's progress.
    #[allow(clippy::too_many_arguments)]
    async fn handle_notify_received_parts(
        &self,
        lifecycle_handle: &SessionRegistryHandle,
        event_package: String,
        subscription_state: Option<String>,
        content_type: Option<String>,
        body: Option<String>,
        raw_request: Option<bytes::Bytes>,
        transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
    ) -> Result<()> {
        let session_id = lifecycle_handle.session_id().clone();
        if self
            .state_machine
            .store
            .get_session_snapshot_exact(lifecycle_handle)
            .is_err()
        {
            debug!(
                "Ignoring NotifyReceived for session {} — not in our store",
                session_id
            );
            return Ok(());
        }

        // SIP_API_DESIGN_2 Phase E: re-parse the inbound NOTIFY bytes
        // into a typed `IncomingRequest`. The coordinator hook stays
        // `None`; the surface consumer rehydrates it on dispatch.
        let request = build_incoming_request_from_bytes(session_id.clone(), raw_request, transport);

        // Always surface the raw NOTIFY as a public event.
        let api_event = crate::api::events::Event::NotifyReceived {
            call_id: session_id.clone(),
            event_package: event_package.clone(),
            subscription_state: subscription_state.clone(),
            content_type: content_type.clone(),
            body: body.clone(),
            request,
        };
        self.app_event_publisher
            .publish_exact(lifecycle_handle, api_event);

        if event_package.eq_ignore_ascii_case("dialog") {
            let is_dialog_info = content_type
                .as_deref()
                .map(|ct| {
                    ct.to_ascii_lowercase()
                        .contains("application/dialog-info+xml")
                })
                .unwrap_or(false);
            if is_dialog_info {
                if let Some(body) = body.as_deref() {
                    match crate::api::dialog_package::parse_dialog_info_xml(body) {
                        Ok(document) => {
                            let dialogs = document.dialogs.clone();
                            self.app_event_publisher.publish_exact(
                                lifecycle_handle,
                                crate::api::events::Event::DialogPackageNotify {
                                    subscription_id: session_id.clone(),
                                    entity: document.entity.clone(),
                                    version: document.version,
                                    dialogs: dialogs.clone(),
                                    document,
                                },
                            );
                            for dialog in dialogs {
                                self.app_event_publisher.publish_exact(
                                    lifecycle_handle,
                                    crate::api::events::Event::DialogStateChanged {
                                        subscription_id: session_id.clone(),
                                        dialog: dialog.clone(),
                                    },
                                );
                            }
                        }
                        Err(e) => {
                            debug!(
                                "dialog NOTIFY body for session {} was not parseable dialog-info XML: {}",
                                session_id, e
                            );
                        }
                    }
                }
            }
        }

        // RFC 3515 §2.4.5 progress NOTIFYs carry a `message/sipfrag` body
        // containing the final-response status line of the transferee's
        // INVITE. Parse it so the transferor sees progress events
        // symmetric to what a transferee emits on the send side.
        if event_package.eq_ignore_ascii_case("refer") {
            let is_sipfrag = content_type
                .as_deref()
                .map(|ct| ct.to_ascii_lowercase().contains("message/sipfrag"))
                .unwrap_or(false);
            if is_sipfrag {
                if let Some(body) = body {
                    if let Some((status_code, reason)) = parse_sipfrag_status_line(&body) {
                        let outcome = match self
                            .state_machine
                            .process_refer_notify_exact(
                                lifecycle_handle,
                                ReferNotifyInput::new(status_code, reason.clone()),
                            )
                            .await
                        {
                            Ok(outcome) => outcome,
                            Err(error) => {
                                debug!(
                                    session = %session_id,
                                    %error,
                                    "Skipping REFER-derived observations because exact ReceiveNOTIFY processing did not commit"
                                );
                                return Ok(());
                            }
                        };
                        self.app_event_publisher.publish_exact(
                            lifecycle_handle,
                            crate::api::events::Event::ReferNotify {
                                call_id: session_id.clone(),
                                status_code,
                                reason: reason.clone(),
                                subscription_state: subscription_state
                                    .clone()
                                    .map(crate::api::events::SubscriptionState::parse),
                                body: Some(body.clone()),
                            },
                        );
                        match outcome {
                            ReferNotifyOutcome::Progress => self.app_event_publisher.publish_exact(
                                lifecycle_handle,
                                crate::api::events::Event::ReferProgress {
                                    call_id: session_id.clone(),
                                    status_code,
                                    reason,
                                },
                            ),
                            ReferNotifyOutcome::Completed {
                                transfer_target,
                                progress_evidence,
                            } => {
                                if let Some((progress_status_code, progress_reason)) =
                                    progress_evidence
                                {
                                    self.app_event_publisher.publish_exact(
                                        lifecycle_handle,
                                        crate::api::events::Event::TransferTargetAnswered {
                                            transfer_call_id: session_id.clone(),
                                            target_uri: transfer_target.clone(),
                                            evidence: crate::api::events::TransferTargetEvidence::ReferProgressThenFinal {
                                                progress_status_code,
                                                progress_reason,
                                                final_status_code: status_code,
                                                final_reason: reason.clone(),
                                            },
                                        },
                                    );
                                }
                                self.app_event_publisher.publish_exact(
                                    lifecycle_handle,
                                    crate::api::events::Event::ReferCompleted {
                                        call_id: session_id.clone(),
                                        target: transfer_target,
                                        status_code,
                                        reason,
                                    },
                                );
                            }
                            ReferNotifyOutcome::Failed => self.app_event_publisher.publish_exact(
                                lifecycle_handle,
                                crate::api::events::Event::TransferFailed {
                                    call_id: session_id.clone(),
                                    reason,
                                    status_code,
                                },
                            ),
                            ReferNotifyOutcome::Ignored => {}
                        }
                    } else {
                        debug!(
                            "NOTIFY sipfrag body for session {} was not a parseable status line; skipping REFER-derived emission",
                            session_id
                        );
                    }
                }
            }
        }

        Ok(())
    }
}

/// Publish a non-terminal app-level event to the global coordinator's
/// `session_to_app` channel. Terminal events (`CallEnded` / `CallFailed` /
/// `CallCancelled`) go through `publish_and_release_session` instead,
/// which also frees the session-store entry after publish.
fn publish_api_event(publisher: &SessionEventPublisher, api_event: crate::api::events::Event) {
    publisher.publish(api_event);
}

fn publish_committed_api_projection<const N: usize>(
    publisher: &SessionEventPublisher,
    lifecycle_handle: &SessionRegistryHandle,
    events: [crate::api::events::Event; N],
) {
    for event in events {
        publisher.publish_exact(lifecycle_handle, event);
    }
}

/// Derive response authority from the preserved request itself. A missing
/// request, method mismatch, or missing top-Via branch is a causal-ingress
/// error; no mutable per-dialog pending transaction is consulted.
fn derive_inbound_response_state_input(
    claimed_method: &str,
    raw_request: Option<&bytes::Bytes>,
) -> SessionResult<InboundResponseStateInput> {
    let raw_request = raw_request.ok_or_else(|| {
        SessionError::InvalidTransition(
            "inbound re-INVITE/UPDATE event has no preserved SIP request".to_string(),
        )
    })?;
    let request = match rvoip_sip_core::parse_message(raw_request.as_ref()) {
        Ok(rvoip_sip_core::Message::Request(request)) => request,
        _ => {
            return Err(SessionError::InvalidTransition(
                "inbound re-INVITE/UPDATE preserved bytes are not a SIP request".to_string(),
            ));
        }
    };
    InboundResponseStateInput::from_request(claimed_method, &request)
}

/// SIP_API_DESIGN_2 Phase E — re-parse the inbound bytes carried on
/// the cross-crate variant into an `IncomingRequest`. Returns `None`
/// when the bytes are missing or unparseable; callers treat that as
/// "skip the typed event surface" rather than failing the bus
/// delivery.
fn build_incoming_request_from_bytes(
    call_id: SessionId,
    raw_request: Option<bytes::Bytes>,
    transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
) -> Option<crate::api::incoming::IncomingRequest> {
    let bytes = raw_request?;
    match rvoip_sip_core::parse_message(bytes.as_ref()) {
        Ok(rvoip_sip_core::Message::Request(req)) => {
            let from = req.from().map(|f| f.to_string()).unwrap_or_default();
            let to = req.to().map(|t| t.to_string()).unwrap_or_default();
            let method = req.method();
            Some(
                crate::api::incoming::IncomingRequest::from_bus_request(
                    call_id,
                    from,
                    to,
                    method,
                    std::sync::Arc::new(req),
                )
                .with_transport_context(sip_transport_context(&transport)),
            )
        }
        _ => None,
    }
}

/// Correlate an inbound INFO control event with the transaction encoded by
/// the preserved SIP request. The wire request is authoritative: a malformed,
/// stale, or wrong-branch event identifier is rejected and the caller may use
/// only the returned request-derived key for a fail-closed response.
fn correlate_inbound_info_transaction(
    event_transaction_id: &str,
    request: &rvoip_sip_core::Request,
) -> std::result::Result<
    rvoip_sip_dialog::transaction::TransactionKey,
    Option<rvoip_sip_dialog::transaction::TransactionKey>,
> {
    let wire_transaction = rvoip_sip_dialog::transaction::TransactionKey::from_request(request)
        .filter(|transaction| {
            transaction.is_server() && transaction.method() == &rvoip_sip_core::Method::Info
        });
    let event_transaction = event_transaction_id
        .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
        .ok();

    match (event_transaction, wire_transaction) {
        (Some(event), Some(wire)) if event == wire => Ok(wire),
        (_, wire) => Err(wire),
    }
}

fn validated_inbound_info_event_transaction(
    event_transaction_id: &str,
) -> Option<rvoip_sip_dialog::transaction::TransactionKey> {
    event_transaction_id
        .parse::<rvoip_sip_dialog::transaction::TransactionKey>()
        .ok()
        .filter(|transaction| {
            transaction.is_server() && transaction.method() == &rvoip_sip_core::Method::Info
        })
}

fn sip_transport_context(
    transport: &Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
) -> crate::auth::SipTransportSecurityContext {
    transport
        .as_ref()
        .map(crate::auth::SipTransportSecurityContext::from_transport_context)
        .unwrap_or_default()
}

/// SIP_API_DESIGN_2 Phase A — construct an `IncomingResponse` from
/// the optional inbound bytes carried on the cross-crate variant.
/// When `raw_response` is `Some`, re-parse the bytes via
/// `rvoip_sip_core::parse_message` so applications can access typed
/// headers (Allow / Supported / Retry-After / Warning / …); when
/// `None`, fall back to a synthesized view that only carries the
/// status / reason / sdp fields.
fn build_incoming_response_from_bytes(
    call_id: SessionId,
    status_code: u16,
    reason_phrase: String,
    sdp: Option<String>,
    raw_response: Option<bytes::Bytes>,
) -> crate::api::incoming::IncomingResponse {
    use crate::api::handle::CallId;
    let call_id_view: CallId = call_id;
    match raw_response.as_ref() {
        Some(bytes) => {
            // Re-parse the inbound bytes back into a typed `Response`.
            // On parse failure (shouldn't happen — these are the
            // bytes we already accepted) fall back to the synthesized
            // view.
            match rvoip_sip_core::parse_message(bytes.as_ref()) {
                Ok(rvoip_sip_core::Message::Response(resp)) => {
                    crate::api::incoming::IncomingResponse::with_response(
                        call_id_view,
                        status_code,
                        reason_phrase,
                        sdp,
                        std::sync::Arc::new(resp),
                    )
                }
                _ => crate::api::incoming::IncomingResponse::synthetic(
                    call_id_view,
                    status_code,
                    reason_phrase,
                    sdp,
                ),
            }
        }
        None => crate::api::incoming::IncomingResponse::synthetic(
            call_id_view,
            status_code,
            reason_phrase,
            sdp,
        ),
    }
}

fn remote_direction_is_hold(direction: crate::types::MediaDirection) -> bool {
    matches!(
        direction,
        crate::types::MediaDirection::SendOnly | crate::types::MediaDirection::Inactive
    )
}

fn termination_reason_to_string(
    reason: &rvoip_infra_common::events::cross_crate::TerminationReason,
) -> String {
    match reason {
        rvoip_infra_common::events::cross_crate::TerminationReason::LocalHangup => {
            "LocalHangup".to_string()
        }
        rvoip_infra_common::events::cross_crate::TerminationReason::RemoteHangup => {
            "RemoteHangup".to_string()
        }
        rvoip_infra_common::events::cross_crate::TerminationReason::Rejected(reason) => {
            format!("Rejected: {}", reason)
        }
        rvoip_infra_common::events::cross_crate::TerminationReason::Error(error) => {
            format!("Error: {}", error)
        }
        rvoip_infra_common::events::cross_crate::TerminationReason::Timeout => {
            "Timeout".to_string()
        }
    }
}

fn transfer_type_to_string(
    transfer_type: &rvoip_infra_common::events::cross_crate::TransferType,
) -> String {
    match transfer_type {
        rvoip_infra_common::events::cross_crate::TransferType::Blind => "blind".to_string(),
        rvoip_infra_common::events::cross_crate::TransferType::Attended => "attended".to_string(),
    }
}

/// Parse an RFC 3515 §2.4.5 sipfrag status line of the form
/// `SIP/2.0 NNN Reason\r\n...` into `(status_code, reason)`. Returns
/// `None` on any deviation (missing version, non-numeric status, empty
/// reason phrase).
fn parse_sipfrag_status_line(body: &str) -> Option<(u16, String)> {
    let first_line = body.lines().next()?.trim();
    let rest = first_line.strip_prefix("SIP/2.0")?.trim_start();
    let mut parts = rest.splitn(2, char::is_whitespace);
    let code_part = parts.next()?;
    let reason = parts.next().unwrap_or("").trim().to_string();
    let status_code: u16 = code_part.parse().ok()?;
    if !(100..=699).contains(&status_code) {
        return None;
    }
    Some((status_code, reason))
}

#[cfg(test)]
mod tests {
    use super::{
        build_incoming_request_from_bytes, build_incoming_response_from_bytes,
        capture_dialog_ingress_handle, committed_bye_termination, committed_call_failure,
        committed_dialog_200, committed_dialog_termination, committed_session_interval_retry,
        correlate_inbound_info_transaction, derive_inbound_response_state_input,
        dialog_event_requires_processing_ack, exact_final_response_outcome,
        exact_final_response_result, exact_final_response_retires_routes,
        exact_response_failure_processing_ack, is_session_lifecycle_capacity_exhaustion,
        join_state_machine_task, map_sip_trace_session_id, media_observation_api_event,
        media_observation_session_id, parse_sipfrag_status_line, queued_dialog_lifetime_is_current,
        refer_default_is_pending_exact, registry_has_exact_dialog,
        remove_quiesced_failed_inbound_store_lifetime, safe_auth_method_label,
        shutdown_change_requests_stop, sip_trace_owner_matches,
        spawn_retained_exact_response_completion, spawn_retained_failed_inbound_cleanup,
        spawn_retained_register_processing, stale_dialog_dispatch_result,
        validated_inbound_info_event_transaction, AbortStateMachineTaskOnDrop,
        CallFailureDiagnostics, CallFailureReason, CausalDialogToSessionIngress,
        CommittedCallFailure, CommittedDialog200, CommittedDialogTermination,
        DeferredReplayDelivery, DialogToSessionDirectRouter, ExactFinalResponseOutcome,
        InboundInviteInitialState, OutboundAuthTerminalClass, QueuedDialogPayload,
        QueuedDialogToSessionEvent, SessionRegistryHandle, StateMachineProcessResult,
        STATE_MACHINE_DISPATCH_JOIN_FAILURE,
    };
    use crate::adapters::outbound_request_tracker::{
        DeferredTrackedRequestEvent, ExactTransactionLookup, OutboundInDialogRequestTracker,
        TrackedInDialogMethod, TrackedInDialogOptions,
    };
    use crate::errors::SessionError;
    use crate::retained_tasks::RetainedTasks;
    use crate::session_lifecycle::SessionAdmissionError;
    use crate::state_machine::ProcessEventResult;
    use crate::state_table::types::{Role, SessionId};
    use crate::state_table::{ConditionUpdates, Transition};
    use crate::types::{CallState, FailureReason};
    use rvoip_infra_common::events::coordinator::CrossCrateEventHandler;
    use rvoip_infra_common::events::cross_crate::{
        CrossCrateEvent, DialogToSessionEvent, MediaQualityMetrics, MediaToSessionEvent,
        QualitySeverity, RvoipCrossCrateEvent, SipTraceDirection, SipTraceEvent,
    };
    use rvoip_infra_common::events::{EventCoordinatorConfig, GlobalEventCoordinator};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::sync::{mpsc, oneshot, watch, Notify};

    fn committed_result(
        old_state: CallState,
        next_state: CallState,
        events: Vec<crate::state_table::EventTemplate>,
    ) -> ProcessEventResult {
        committed_result_with_actions(old_state, next_state, events, vec![])
    }

    fn committed_result_with_actions(
        old_state: CallState,
        next_state: CallState,
        events: Vec<crate::state_table::EventTemplate>,
        actions: Vec<crate::state_table::Action>,
    ) -> ProcessEventResult {
        ProcessEventResult {
            old_state,
            next_state: Some(next_state),
            transition: Some(Transition {
                guards: vec![],
                actions: actions.clone(),
                next_state: Some(next_state),
                condition_updates: ConditionUpdates::none(),
                publish_events: events.clone(),
            }),
            actions_executed: actions,
            events_published: events,
        }
    }

    fn missing_result(old_state: CallState) -> ProcessEventResult {
        ProcessEventResult {
            old_state,
            next_state: None,
            transition: None,
            actions_executed: vec![],
            events_published: vec![],
        }
    }

    async fn tracker_test_handle(session_id: &SessionId) -> SessionRegistryHandle {
        let store = crate::session_store::SessionStore::new();
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact tracker test lifetime");
        store
            .lifecycle_handle(session_id)
            .expect("capture exact tracker test handle")
    }

    fn test_media_quality() -> MediaQualityMetrics {
        MediaQualityMetrics {
            mos_score: 3.8,
            packet_loss: 0.125,
            jitter_ms: 17.9,
            delay_ms: 42,
        }
    }

    #[test]
    fn media_bus_projects_quality_without_duplicating_direct_dtmf_callback() {
        for event in [
            MediaToSessionEvent::MediaQualityUpdate {
                session_id: "media-reporting".to_string(),
                quality_metrics: test_media_quality(),
            },
            MediaToSessionEvent::MediaQualityDegraded {
                session_id: "media-reporting".to_string(),
                metrics: test_media_quality(),
                severity: QualitySeverity::High,
            },
        ] {
            assert_eq!(media_observation_session_id(&event), "media-reporting");
            match media_observation_api_event(&event) {
                Some(crate::api::events::Event::MediaQualityChanged {
                    call_id,
                    packet_loss_percent,
                    jitter_ms,
                }) => {
                    assert_eq!(call_id, SessionId("media-reporting".to_string()));
                    assert_eq!(packet_loss_percent, 12);
                    assert_eq!(jitter_ms, 17);
                }
                other => panic!("unexpected media quality projection: {other:?}"),
            }
        }

        let dtmf = MediaToSessionEvent::DtmfDetected {
            session_id: "media-reporting".to_string(),
            digit: '5',
            duration_ms: 160,
        };
        assert!(media_observation_api_event(&dtmf).is_none());
    }

    #[test]
    fn media_bus_lifecycle_reports_have_no_application_lifecycle_projection() {
        for event in [
            MediaToSessionEvent::MediaStreamStarted {
                session_id: "media-reporting".to_string(),
                local_port: 40_000,
                codec: "PCMU".to_string(),
            },
            MediaToSessionEvent::MediaError {
                session_id: "media-reporting".to_string(),
                error: "synthetic".to_string(),
                error_code: Some(1),
            },
            MediaToSessionEvent::RtpTimeout {
                session_id: "media-reporting".to_string(),
                last_packet_time: 123,
            },
        ] {
            assert_eq!(media_observation_session_id(&event), "media-reporting");
            assert!(media_observation_api_event(&event).is_none());
        }
    }

    #[derive(Clone)]
    struct TestCausalHandler {
        delivered: Arc<AtomicUsize>,
        entered: Option<Arc<Notify>>,
        release: Option<Arc<Notify>>,
    }

    #[async_trait::async_trait]
    impl CrossCrateEventHandler for TestCausalHandler {
        async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> anyhow::Result<()> {
            let counted = matches!(
                event.as_any().downcast_ref::<RvoipCrossCrateEvent>(),
                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::CallCancelled { session_id }
                )) if session_id.starts_with("causal-startup-")
            );
            if !counted {
                return Ok(());
            }
            if let Some(entered) = &self.entered {
                entered.notify_waiters();
            }
            if let Some(release) = &self.release {
                release.notified().await;
            }
            self.delivered.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn dialog_test_event(prefix: &str, id: usize) -> Arc<dyn CrossCrateEvent> {
        Arc::new(RvoipCrossCrateEvent::DialogToSession(
            DialogToSessionEvent::CallCancelled {
                session_id: format!("{prefix}-{id}"),
            },
        ))
    }

    fn causal_test_event(id: usize) -> Arc<dyn CrossCrateEvent> {
        dialog_test_event("causal-startup", id)
    }

    fn incoming_call_test_event(session_id: &str) -> DialogToSessionEvent {
        DialogToSessionEvent::IncomingCall {
            session_id: session_id.to_string(),
            call_id: format!("{session_id}@example.test"),
            from: "sip:caller@example.test".to_string(),
            to: "sip:callee@example.test".to_string(),
            sdp_offer: None,
            headers: std::collections::HashMap::new(),
            transaction_id: format!("z9hG4bK-{session_id}:INVITE:server"),
            source_addr: "127.0.0.1:5060".to_string(),
            raw_request: None,
            transport: None,
            identity_verification: None,
        }
    }

    fn incoming_register_test_event() -> DialogToSessionEvent {
        DialogToSessionEvent::IncomingRegister {
            transaction_id: "z9hG4bK-processing-ack-register:REGISTER:server".to_string(),
            from_uri: "sip:alice@example.test".to_string(),
            to_uri: "sip:alice@example.test".to_string(),
            contact_uri: "sip:alice@127.0.0.1:5060".to_string(),
            expires: 300,
            authorization: None,
            call_id: "processing-ack-register@example.test".to_string(),
            raw_request: None,
            transport: None,
        }
    }

    #[test]
    fn response_bearing_causal_events_wait_for_shard_processing() {
        assert!(!dialog_event_requires_processing_ack(
            &DialogToSessionEvent::CallEstablished {
                session_id: "fast-200".to_string(),
                sdp_answer: None,
                raw_response: None,
            }
        ));
        assert!(dialog_event_requires_processing_ack(
            &incoming_call_test_event("inbound-invite")
        ));
        assert!(dialog_event_requires_processing_ack(
            &incoming_register_test_event()
        ));
        assert!(dialog_event_requires_processing_ack(
            &DialogToSessionEvent::ByeReceived {
                session_id: "inbound-bye".to_string(),
            }
        ));
        assert!(dialog_event_requires_processing_ack(
            &DialogToSessionEvent::InfoReceived {
                session_id: "inbound-info".to_string(),
                transaction_id: "transaction".to_string(),
                raw_request: None,
                transport: None,
            }
        ));
        assert!(dialog_event_requires_processing_ack(
            &DialogToSessionEvent::ReinviteReceived {
                session_id: "inbound-reinvite".to_string(),
                sdp: None,
                method: "INVITE".to_string(),
                raw_request: None,
                transport: None,
            }
        ));
        assert!(dialog_event_requires_processing_ack(
            &DialogToSessionEvent::TransferRequested {
                session_id: "inbound-refer".to_string(),
                refer_to: "sip:target@example.test".to_string(),
                transfer_type: rvoip_infra_common::events::cross_crate::TransferType::Blind,
                transaction_id: "z9hG4bK-inbound-refer:REFER:server".to_string(),
                referred_by: None,
                replaces: None,
                raw_request: None,
                transport: None,
            }
        ));
    }

    #[test]
    fn incoming_register_has_one_causal_response_owner() {
        let source = include_str!("session_event_handler.rs");
        let branch = source
            .split("DialogToSessionEvent::IncomingRegister {")
            .nth(2)
            .and_then(|tail| {
                tail.split("DialogToSessionEvent::OutboundFlowFailed")
                    .next()
            })
            .expect("IncomingRegister handler branch");
        assert!(branch.contains("if built_in_owner"));
        assert!(branch.contains("register.clear_response_capability()"));
        assert!(branch.contains(".publish_control_now("));
        let install = branch
            .find("register.install_response_obligation(coordinator)")
            .expect("standalone REGISTER obligation install");
        let publish = branch
            .find(".publish_control_now(")
            .expect("causal REGISTER control delivery");
        assert!(install < publish);
        assert!(branch.contains("ExactResponseRegistration::Collision"));
        assert!(branch.contains("return Ok(())"));
        assert!(branch.contains("response_obligation.claim()"));
        assert!(branch.contains("claim.complete()"));
        assert!(!branch.contains("None => Ok(())"));
    }

    #[tokio::test]
    async fn response_bearing_ingress_waits_for_error_while_observation_is_enqueue_only() {
        let store = Arc::new(crate::session_store::SessionStore::new());
        let (shard_tx, mut shard_rx) = mpsc::channel(4);
        let router = DialogToSessionDirectRouter {
            shard_senders: Arc::new(vec![shard_tx]),
            fallback_shard: Arc::new(AtomicUsize::new(0)),
            deferred_tracker: OutboundInDialogRequestTracker::default(),
            registration_adapter: Arc::new(std::sync::OnceLock::new()),
            registration_response_owner: None,
            store: Arc::clone(&store),
        };

        let incoming_router = router.clone();
        let incoming_dispatch = tokio::spawn(async move {
            incoming_router
                .handle(Arc::new(RvoipCrossCrateEvent::DialogToSession(
                    incoming_call_test_event("processing-ack-invite"),
                )))
                .await
        });
        let queued_incoming =
            tokio::time::timeout(std::time::Duration::from_secs(1), shard_rx.recv())
                .await
                .expect("incoming call was not enqueued")
                .expect("incoming call shard closed");
        assert!(
            !incoming_dispatch.is_finished(),
            "IncomingCall returned before causal processing completed"
        );
        queued_incoming
            .authoritative_completion
            .expect("IncomingCall lost its processing ACK")
            .send(Err("synthetic zero-wire overload".to_string()))
            .expect("return processing failure");
        assert!(incoming_dispatch
            .await
            .expect("IncomingCall dispatch task panicked")
            .is_err());

        let register_router = router.clone();
        let register_dispatch = tokio::spawn(async move {
            register_router
                .handle(Arc::new(RvoipCrossCrateEvent::DialogToSession(
                    incoming_register_test_event(),
                )))
                .await
        });
        let queued_register =
            tokio::time::timeout(std::time::Duration::from_secs(1), shard_rx.recv())
                .await
                .expect("incoming REGISTER was not enqueued")
                .expect("incoming REGISTER shard closed");
        assert!(
            !register_dispatch.is_finished(),
            "IncomingRegister returned before causal processing completed"
        );
        queued_register
            .authoritative_completion
            .expect("IncomingRegister lost its processing ACK")
            .send(Err("synthetic zero-wire REGISTER".to_string()))
            .expect("return REGISTER processing failure");
        assert!(register_dispatch
            .await
            .expect("IncomingRegister dispatch task panicked")
            .is_err());

        let observation_session = SessionId("enqueue-only-observation".to_string());
        store
            .create_session(observation_session.clone(), Role::UAC, false)
            .await
            .expect("create observation session");
        router
            .handle(Arc::new(RvoipCrossCrateEvent::DialogToSession(
                DialogToSessionEvent::CallEstablished {
                    session_id: observation_session.0,
                    sdp_answer: None,
                    raw_response: None,
                },
            )))
            .await
            .expect("ordinary observation enqueue");
        let queued_observation = shard_rx.recv().await.expect("queued observation");
        assert!(
            queued_observation.authoritative_completion.is_none(),
            "ordinary observation unexpectedly waited for processing"
        );
    }

    #[test]
    fn exact_final_response_policy_preserves_only_zero_wire_retry_authority() {
        use rvoip_sip_dialog::FinalResponseCompletionDisposition as Disposition;

        assert_eq!(
            exact_final_response_outcome(Disposition::WrittenSuccessTerminal),
            ExactFinalResponseOutcome::Written
        );
        assert_eq!(
            exact_final_response_outcome(Disposition::ZeroWireRetryable),
            ExactFinalResponseOutcome::ZeroWire
        );
        assert_eq!(
            exact_final_response_outcome(Disposition::WireUnknownErrorTerminal),
            ExactFinalResponseOutcome::WireUnknown
        );

        assert!(exact_final_response_result(
            "written response",
            ExactFinalResponseOutcome::Written
        )
        .is_ok());
        assert!(exact_final_response_result(
            "zero-wire response",
            ExactFinalResponseOutcome::ZeroWire
        )
        .is_err());
        assert!(exact_final_response_result(
            "wire-unknown response",
            ExactFinalResponseOutcome::WireUnknown
        )
        .is_ok());

        assert!(exact_final_response_retires_routes(
            ExactFinalResponseOutcome::Written
        ));
        assert!(!exact_final_response_retires_routes(
            ExactFinalResponseOutcome::ZeroWire
        ));
        assert!(exact_final_response_retires_routes(
            ExactFinalResponseOutcome::WireUnknown
        ));

        assert!(exact_response_failure_processing_ack(
            "re-INVITE",
            Some(Disposition::WrittenSuccessTerminal),
            "late written diagnostic",
        )
        .is_ok());
        assert!(exact_response_failure_processing_ack(
            "re-INVITE",
            Some(Disposition::WireUnknownErrorTerminal),
            "wire completion unknown",
        )
        .is_ok());
        assert!(exact_response_failure_processing_ack(
            "re-INVITE",
            Some(Disposition::ZeroWireRetryable),
            "pre-wire failure",
        )
        .is_err());
        assert!(exact_response_failure_processing_ack(
            "re-INVITE",
            None,
            "unclassified lifecycle failure",
        )
        .is_err());

        for kind in ["info_received", "reinvite_received", "transfer_requested"] {
            assert!(
                stale_dialog_dispatch_result(kind).is_err(),
                "stale {kind} returned a successful processing ACK without a response owner"
            );
        }
        assert!(stale_dialog_dispatch_result("bye_received").is_ok());
    }

    #[test]
    fn committed_invite_2xx_retransmission_always_sends_ack() {
        let source = include_str!("session_event_handler.rs");
        let retransmission = source
            .split("if invite_success_is_retransmission(")
            .nth(1)
            .and_then(|tail| tail.split("match pending_correlation").next())
            .expect("committed INVITE 2xx retransmission branch");

        assert!(retransmission.contains("send_invite_2xx_ack_exact"));
        assert!(
            !retransmission.contains("body().is_empty()"),
            "a bodyless INVITE 2xx retransmission still requires ACK"
        );
    }

    #[test]
    fn failed_sdes_2xx_is_acked_observable_and_terminal() {
        let source = include_str!("session_event_handler.rs");
        let handler = source
            .split("async fn handle_call_established_parts")
            .nth(1)
            .and_then(|tail| {
                tail.split("async fn fail_initial_invite_negotiation")
                    .next()
            })
            .expect("INVITE 2xx handler source");
        let failure = handler
            .split("Err(e) =>")
            .nth(1)
            .expect("failed INVITE 2xx branch");

        let ack = failure
            .find("send_invite_2xx_ack_exact")
            .expect("failed INVITE 2xx ACK");
        let observation = failure
            .find("Event::SdesNegotiationFailed")
            .expect("application-visible SDES failure");
        let terminal = failure
            .find("fail_initial_invite_negotiation")
            .expect("terminal failed-call transition");
        assert!(ack < observation && observation < terminal);
        assert!(
            !failure[..observation].contains("body().is_empty()"),
            "every INVITE 2xx requires ACK, including bodyless failures"
        );
    }

    #[test]
    fn reinvite_handler_propagates_every_nonterminal_response_failure() {
        let source = include_str!("session_event_handler.rs");
        let handler = source
            .split("    async fn handle_reinvite_received_parts(")
            .nth(1)
            .and_then(|tail| {
                tail.split("    async fn apply_inbound_reinvite_media_direction(")
                    .next()
            })
            .expect("re-INVITE/UPDATE handler source");

        assert!(handler.contains("exact_sip_response_failure_disposition"));
        assert!(handler.contains("exact_response_failure_processing_ack("));
        assert!(
            handler.contains("&detail,\n            )?;"),
            "re-INVITE/UPDATE failure classification was not propagated through its processing ACK"
        );
        assert!(handler.contains("get_session_snapshot_exact(handle)\n            .map_err"));
    }

    #[test]
    fn fast_autoaccept_keeps_terminal_wire_state_and_propagates_zero_wire() {
        use rvoip_sip_dialog::FinalResponseCompletionDisposition as Disposition;

        assert!(exact_response_failure_processing_ack(
            "IncomingCall",
            Some(Disposition::WireUnknownErrorTerminal),
            "fast 200 write boundary became unknown",
        )
        .is_ok());
        assert!(exact_response_failure_processing_ack(
            "IncomingCall",
            Some(Disposition::ZeroWireRetryable),
            "fast 200 stopped before its first write",
        )
        .is_err());

        let source = include_str!("session_event_handler.rs");
        let handler = source
            .split("    async fn handle_incoming_call_parts(")
            .nth(1)
            .and_then(|tail| {
                tail.split("    async fn handle_call_established_parts(")
                    .next()
            })
            .expect("incoming INVITE handler source");
        assert!(handler.contains("exact_sip_response_failure_disposition"));
        assert!(handler.contains("release_failed_inbound_upper_resources_with_retry"));
        assert!(handler.contains("spawn_retained_failed_inbound_cleanup"));
        assert!(!handler.contains("release_exact_local_resources("));
        assert!(handler.contains("return Err(processing_error)"));
        assert!(handler.contains("Fast auto-accepted inbound call"));
    }

    #[tokio::test]
    async fn failed_inbound_upper_removal_leaves_lower_route_and_no_session_leak() {
        let store = crate::session_store::SessionStore::new();
        let session_id = SessionId("failed-inbound-preserve-route".to_string());
        store
            .create_session(session_id.clone(), Role::UAS, false)
            .await
            .expect("create failed-inbound fixture");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("capture failed-inbound fixture");
        let dialog_id = crate::types::DialogId(uuid::Uuid::new_v4());
        let receipt = store
            .registry()
            .commit_inbound_dialog_exact(
                handle.key(),
                handle.slot_revision(),
                dialog_id,
                crate::types::IncomingCallInfo {
                    session_id: session_id.clone(),
                    from: "sip:caller@example.test".to_string(),
                    to: "sip:callee@example.test".to_string(),
                    call_id: "failed-inbound@example.test".to_string(),
                    dialog_id,
                    p_asserted_identity: None,
                },
            )
            .expect("stage upper inbound dialog copy");
        receipt
            .finalize()
            .expect("commit upper inbound dialog copy");

        // This tuple models dialog-core's independently retained fallback
        // authority. Upper cleanup has no reference to it and therefore
        // cannot terminate or rewrite it before the negative causal ACK.
        let lower_fallback_route = (
            dialog_id,
            "z9hG4bK-failed-inbound:INVITE:server".to_string(),
        );
        store
            .quiesce_session_exact(&handle)
            .await
            .expect("quiesce failed-inbound upper lifetime");
        assert!(store
            .registry()
            .clear_dialog_handle_retained(&handle, dialog_id)
            .expect("clear upper dialog copy"));
        remove_quiesced_failed_inbound_store_lifetime(&store, &handle)
            .expect("remove failed-inbound upper lifetime");

        assert!(store.get_session_retained_snapshot_exact(&handle).is_err());
        assert_eq!(store.sessions.len(), 0);
        assert_eq!(lower_fallback_route.0, dialog_id);
        assert_eq!(
            lower_fallback_route.1,
            "z9hG4bK-failed-inbound:INVITE:server"
        );
    }

    #[tokio::test]
    async fn retained_exact_response_child_survives_cancelled_waiter_through_cleanup() {
        let retained_tasks = RetainedTasks::new();
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let route_cleanup_count = Arc::new(AtomicUsize::new(0));
        let child_cleanup_count = Arc::clone(&route_cleanup_count);
        let waiter_tasks = Arc::clone(&retained_tasks);
        let waiter = tokio::spawn(async move {
            let completion = spawn_retained_exact_response_completion(&waiter_tasks, async move {
                let _ = started_tx.send(());
                let _ = release_rx.await;
                child_cleanup_count.fetch_add(1, Ordering::SeqCst);
                ExactFinalResponseOutcome::WireUnknown
            })
            .expect("retain exact response child");
            completion.await
        });

        started_rx.await.expect("retained response child started");
        waiter.abort();
        assert!(waiter
            .await
            .expect_err("waiter cancellation")
            .is_cancelled());
        release_tx
            .send(())
            .expect("release retained response child");
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            retained_tasks.wait_idle(),
        )
        .await
        .expect("retained response child drained");
        assert_eq!(
            route_cleanup_count.load(Ordering::SeqCst),
            1,
            "caller cancellation aborted exact response route cleanup"
        );
        assert_eq!(retained_tasks.count(), 0);
        assert!(!retained_tasks.panicked());
    }

    #[tokio::test]
    async fn failed_inbound_cleanup_survives_cancelled_processing_ack_waiter() {
        let retained_tasks = RetainedTasks::new();
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let completions = Arc::new(AtomicUsize::new(0));
        let child_completions = Arc::clone(&completions);
        let waiter_tasks = Arc::clone(&retained_tasks);
        let waiter = tokio::spawn(async move {
            let completion = spawn_retained_failed_inbound_cleanup(&waiter_tasks, async move {
                let _ = started_tx.send(());
                let _ = release_rx.await;
                child_completions.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .expect("retain failed-inbound cleanup child");
            completion.await
        });

        started_rx
            .await
            .expect("retained failed-inbound cleanup started");
        waiter.abort();
        assert!(waiter
            .await
            .expect_err("processing ACK waiter cancellation")
            .is_cancelled());
        release_tx
            .send(())
            .expect("release failed-inbound cleanup child");
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            retained_tasks.wait_idle(),
        )
        .await
        .expect("failed-inbound cleanup child drained");
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        assert_eq!(retained_tasks.count(), 0);
        assert!(!retained_tasks.panicked());
    }

    #[tokio::test]
    async fn retained_register_response_survives_cancelled_causal_ack_waiter() {
        let retained_tasks = RetainedTasks::new();
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let completions = Arc::new(AtomicUsize::new(0));
        let child_completions = Arc::clone(&completions);
        let waiter_tasks = Arc::clone(&retained_tasks);
        let waiter = tokio::spawn(async move {
            let completion = spawn_retained_register_processing(&waiter_tasks, async move {
                let _ = started_tx.send(());
                let _ = release_rx.await;
                child_completions.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .expect("retain REGISTER response child");
            completion.await
        });

        started_rx.await.expect("retained REGISTER child started");
        waiter.abort();
        assert!(waiter
            .await
            .expect_err("waiter cancellation")
            .is_cancelled());
        release_tx
            .send(())
            .expect("release REGISTER response child");
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            retained_tasks.wait_idle(),
        )
        .await
        .expect("retained REGISTER response child drained");
        assert_eq!(completions.load(Ordering::SeqCst), 1);
        assert_eq!(retained_tasks.count(), 0);
        assert!(!retained_tasks.panicked());
    }

    #[test]
    fn compatibility_response_pairs_have_one_committed_projection_owner() {
        let source = include_str!("session_event_handler.rs");
        let projector = source
            .split("fn project_committed_response_events")
            .nth(1)
            .and_then(|tail| tail.split("enum QueuedDialogPayload").next())
            .expect("committed response projector source");
        for variant in [
            "CallProgressDetailed",
            "CallEstablishedDetailed",
            "CallFailedDetailed",
        ] {
            assert_eq!(projector.matches(variant).count(), 1);
        }
        assert!(
            projector.find("CallProgress {").unwrap()
                < projector.find("CallProgressDetailed").unwrap()
        );
        assert!(
            projector.find("CallAnswered {").unwrap()
                < projector.find("CallEstablishedDetailed").unwrap()
        );
        assert!(
            projector.find("CallFailedDetailed").unwrap() < projector.find("CallFailed {").unwrap()
        );

        for handler in [
            "handle_call_established_parts",
            "handle_call_failed_parts",
            "handle_call_progress_parts",
        ] {
            let body = source
                .split(&format!("async fn {handler}"))
                .nth(1)
                .and_then(|tail| tail.split("\n    async fn ").next())
                .unwrap_or_else(|| panic!("missing {handler} source"));
            assert!(body.contains("project_committed_response_events"));
            assert!(body.contains("get_session_snapshot_exact"));
        }

        let inbound = source
            .split("async fn handle_incoming_call_parts")
            .nth(1)
            .and_then(|tail| tail.split("\n    async fn ").next())
            .expect("inbound admission handler source");
        assert!(inbound.contains("incoming_info.clone()"));
        assert!(inbound.contains("try_send(incoming_info)"));
    }

    #[test]
    fn causal_shards_and_terminal_release_use_retained_ownership() {
        let source = include_str!("session_event_handler.rs");
        let router = source
            .split("impl DialogToSessionDirectRouter")
            .nth(1)
            .and_then(|tail| tail.split("fn shard_for").next())
            .expect("direct router constructor source");
        assert!(router.contains("retained_tasks.spawn(async move"));
        assert!(!router.contains("tokio::spawn(async move"));

        let terminal = source
            .split("async fn publish_and_release_session")
            .nth(1)
            .and_then(|tail| tail.split("fn commit_exact_inbound_dialog").next())
            .expect("terminal release helper source");
        assert!(terminal.contains("retained_tasks.spawn_or_child(async move"));
        assert!(terminal.contains("publish_terminal_best_effort"));
        assert!(
            terminal.find("publish_terminal_best_effort").unwrap()
                < terminal
                    .find("release_exact_local_resources_with_retry")
                    .unwrap()
        );
        assert!(!terminal.contains(concat!("publish_terminal_then_", "release")));
        assert!(!terminal.contains("tokio::spawn"));
        assert!(terminal.contains("pending_exact_response_registry"));
        assert!(!terminal.contains("dispatch_authoritative_handler"));

        let subscriptions = source
            .split("async fn start_global_event_subscriptions")
            .nth(1)
            .and_then(|tail| tail.split("async fn handle_state_machine_event").next())
            .expect("event subscription startup source");
        assert_eq!(
            subscriptions
                .matches("self.retained_tasks.spawn(async move")
                .count(),
            4
        );
        assert!(!subscriptions.contains("tokio::spawn"));
    }

    #[tokio::test]
    async fn first_causal_event_waits_for_router_and_never_enters_observational_bus() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(1),
            )
            .await
            .unwrap(),
        );
        let ingress = CausalDialogToSessionIngress::new();
        coordinator
            .register_handler("dialog_to_session", ingress.clone())
            .await
            .unwrap();
        let mut observer = coordinator.subscribe("dialog_to_session").await.unwrap();

        let delivered = Arc::new(AtomicUsize::new(0));
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let entered_wait = entered.notified();
        let publish = tokio::spawn({
            let coordinator = Arc::clone(&coordinator);
            async move {
                coordinator
                    .dispatch_authoritative_handler(causal_test_event(1))
                    .await
            }
        });
        tokio::task::yield_now().await;
        assert!(
            !publish.is_finished(),
            "early event bypassed ingress readiness"
        );
        assert_eq!(delivered.load(Ordering::SeqCst), 0);

        ingress
            .install_target(Arc::new(TestCausalHandler {
                delivered: Arc::clone(&delivered),
                entered: Some(Arc::clone(&entered)),
                release: Some(Arc::clone(&release)),
            }))
            .unwrap();
        entered_wait.await;
        assert_eq!(delivered.load(Ordering::SeqCst), 0);
        assert!(matches!(
            observer.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        release.notify_waiters();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), publish)
                .await
                .expect("causal startup publish stalled")
                .expect("causal startup task panicked")
                .expect("causal startup publish failed")
        );
        assert_eq!(delivered.load(Ordering::SeqCst), 1);
        tokio::time::timeout(std::time::Duration::from_millis(20), observer.recv())
            .await
            .expect_err("capability-bearing dialog event entered observational bus");
    }

    #[tokio::test]
    async fn observer_absence_saturation_and_closure_do_not_affect_causal_delivery() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::new(
                EventCoordinatorConfig::monolithic().with_channel_capacity(1),
            )
            .await
            .unwrap(),
        );
        let ingress = CausalDialogToSessionIngress::new();
        coordinator
            .register_handler("dialog_to_session", ingress.clone())
            .await
            .unwrap();
        let delivered = Arc::new(AtomicUsize::new(0));
        ingress
            .install_target(Arc::new(TestCausalHandler {
                delivered: Arc::clone(&delivered),
                entered: None,
                release: None,
            }))
            .unwrap();

        assert!(coordinator
            .dispatch_authoritative_handler(causal_test_event(1))
            .await
            .unwrap());

        let saturated_observer = coordinator.subscribe("dialog_to_session").await.unwrap();
        coordinator
            .publish_observational(dialog_test_event("observer-filler", 1))
            .await
            .unwrap();
        coordinator
            .publish_observational(dialog_test_event("observer-filler", 2))
            .await
            .unwrap();
        assert!(coordinator
            .dispatch_authoritative_handler(causal_test_event(2))
            .await
            .unwrap());
        assert!(tokio::time::timeout(
            std::time::Duration::from_secs(1),
            coordinator.dispatch_authoritative_handler(causal_test_event(3)),
        )
        .await
        .expect("a full observer backpressured causal delivery")
        .expect("causal delivery failed with a full observer"));

        drop(saturated_observer);
        assert!(coordinator
            .dispatch_authoritative_handler(causal_test_event(4))
            .await
            .unwrap());
        assert_eq!(delivered.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn queued_generation_a_event_is_rejected_after_raw_id_reuse() {
        let store = Arc::new(crate::session_store::SessionStore::new());
        let session_id = SessionId("queued-dialog-generation-reuse".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create generation A");
        let generation_a = store
            .lifecycle_handle(&session_id)
            .expect("capture generation A");
        let event = DialogToSessionEvent::CallCancelled {
            session_id: session_id.0.clone(),
        };
        let captured =
            capture_dialog_ingress_handle(store.as_ref(), &event, Some(session_id.0.as_str()))
                .expect("capture queued ingress lifetime");
        assert_eq!(captured.as_ref(), Some(&generation_a));

        let queued = QueuedDialogToSessionEvent {
            payload: QueuedDialogPayload::Dialog(event),
            queued_at: std::time::Instant::now(),
            kind: "call_cancelled",
            route_key: Some(session_id.0.clone()),
            exact_handle: captured,
            authoritative_completion: None,
        };
        let (queue_tx, mut queue_rx) = mpsc::channel(1);
        queue_tx
            .send(queued)
            .await
            .expect("admit generation A event");
        let (release_tx, release_rx) = oneshot::channel();
        let delivered = Arc::new(AtomicUsize::new(0));
        let queued_store = Arc::clone(&store);
        let queued_delivered = Arc::clone(&delivered);
        let worker = tokio::spawn(async move {
            let queued = queue_rx.recv().await.expect("queued generation A event");
            release_rx.await.expect("release queued event");
            if queued_dialog_lifetime_is_current(
                queued_store.as_ref(),
                queued.exact_handle.as_ref(),
            ) {
                queued_delivered.fetch_add(1, Ordering::SeqCst);
            }
            drop(queued);
        });

        store
            .remove_session_exact(&generation_a)
            .await
            .expect("retire generation A");
        assert!(store.authority().elapse_reuse_horizon_for_test(&session_id));
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create generation B");
        let generation_b = store
            .lifecycle_handle(&session_id)
            .expect("capture generation B");
        assert_ne!(generation_a.key(), generation_b.key());
        let replacement_revision = store
            .get_session_snapshot_exact(&generation_b)
            .expect("generation B snapshot")
            .revision();

        release_tx.send(()).expect("release generation A queue");
        worker.await.expect("queue worker");
        assert_eq!(delivered.load(Ordering::SeqCst), 0);
        assert_eq!(
            store
                .get_session_snapshot_exact(&generation_b)
                .expect("generation B remains current")
                .revision(),
            replacement_revision,
            "stale queued work must not mutate the replacement lifetime"
        );
    }

    #[tokio::test]
    async fn missing_exact_lifetime_rejects_every_unanswered_mid_dialog_request() {
        let (shard_tx, mut shard_rx) = mpsc::channel(4);
        let router = DialogToSessionDirectRouter {
            shard_senders: Arc::new(vec![shard_tx]),
            fallback_shard: Arc::new(AtomicUsize::new(0)),
            deferred_tracker: OutboundInDialogRequestTracker::default(),
            registration_adapter: Arc::new(std::sync::OnceLock::new()),
            registration_response_owner: None,
            store: Arc::new(crate::session_store::SessionStore::new()),
        };
        let late_id = "already-retired".to_string();

        for event in [
            DialogToSessionEvent::CallCancelled {
                session_id: late_id.clone(),
            },
            DialogToSessionEvent::ByeReceived {
                session_id: late_id.clone(),
            },
        ] {
            router
                .handle(Arc::new(RvoipCrossCrateEvent::DialogToSession(event)))
                .await
                .expect("late terminal ingress is idempotent");
        }
        assert!(matches!(
            shard_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        for request in [
            DialogToSessionEvent::InfoReceived {
                session_id: late_id.clone(),
                transaction_id: "missing-info-owner".to_string(),
                raw_request: None,
                transport: None,
            },
            DialogToSessionEvent::ReinviteReceived {
                session_id: late_id.clone(),
                sdp: None,
                method: "INVITE".to_string(),
                raw_request: None,
                transport: None,
            },
            DialogToSessionEvent::TransferRequested {
                session_id: late_id,
                refer_to: "sip:target@example.test".to_string(),
                transfer_type: rvoip_infra_common::events::cross_crate::TransferType::Blind,
                transaction_id: "missing-refer-owner".to_string(),
                referred_by: None,
                replaces: None,
                raw_request: None,
                transport: None,
            },
        ] {
            assert!(
                router
                    .handle(Arc::new(RvoipCrossCrateEvent::DialogToSession(request)))
                    .await
                    .is_err(),
                "response-bearing request without an owner returned a successful processing ACK"
            );
        }
        assert!(matches!(
            shard_rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    fn parsed_info_request(branch: &str) -> rvoip_sip_core::Request {
        let raw = format!(
            "INFO sip:bob@example.invalid SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5060;branch={branch}\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:alice@example.invalid>;tag=alice-tag\r\n\
             To: <sip:bob@example.invalid>;tag=bob-tag\r\n\
             Call-ID: info-correlation@example.invalid\r\n\
             CSeq: 2 INFO\r\n\
             Content-Length: 0\r\n\r\n"
        );
        match rvoip_sip_core::parse_message(raw.as_bytes()).expect("parse INFO request") {
            rvoip_sip_core::Message::Request(request) => request,
            rvoip_sip_core::Message::Response(_) => panic!("parsed INFO as a response"),
        }
    }

    fn preserved_in_dialog_request(method: &str, branch: &str) -> bytes::Bytes {
        bytes::Bytes::from(format!(
            "{method} sip:bob@example.invalid SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5060;branch={branch}\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:alice@example.invalid>;tag=alice-tag\r\n\
             To: <sip:bob@example.invalid>;tag=bob-tag\r\n\
             Call-ID: exact-in-dialog@example.invalid\r\n\
             CSeq: 2 {method}\r\n\
             Content-Length: 0\r\n\r\n"
        ))
    }

    #[test]
    fn inbound_response_authority_requires_matching_preserved_request_method() {
        let invite = preserved_in_dialog_request("INVITE", "z9hG4bK-preserved-invite");
        let input = derive_inbound_response_state_input("INVITE", Some(&invite))
            .expect("derive exact INVITE response authority");
        assert_eq!(
            input.transaction_id().expect("unconsumed authority"),
            &rvoip_sip_dialog::transaction::TransactionKey::new(
                "z9hG4bK-preserved-invite".to_string(),
                rvoip_sip_core::Method::Invite,
                true,
            )
        );
        assert!(derive_inbound_response_state_input("UPDATE", Some(&invite)).is_err());
        assert!(derive_inbound_response_state_input("INVITE", None).is_err());

        let no_branch = bytes::Bytes::from_static(
            b"UPDATE sip:bob@example.invalid SIP/2.0\r\n\
              Via: SIP/2.0/UDP 127.0.0.1:5060\r\n\
              From: <sip:alice@example.invalid>;tag=a\r\n\
              To: <sip:bob@example.invalid>;tag=b\r\n\
              Call-ID: no-branch@example.invalid\r\n\
              CSeq: 3 UPDATE\r\n\
              Content-Length: 0\r\n\r\n",
        );
        assert!(derive_inbound_response_state_input("UPDATE", Some(&no_branch)).is_err());
    }

    #[test]
    fn inbound_info_correlation_accepts_only_the_wire_transaction() {
        let request = parsed_info_request("z9hG4bK-info-wire");
        let wire = rvoip_sip_dialog::transaction::TransactionKey::from_request(&request)
            .expect("wire transaction");

        assert_eq!(
            correlate_inbound_info_transaction(&wire.to_string(), &request)
                .expect("matching transaction"),
            wire
        );
    }

    #[test]
    fn inbound_info_correlation_rejects_wrong_branch_with_wire_failure_target() {
        let request = parsed_info_request("z9hG4bK-info-wire");
        let wire = rvoip_sip_dialog::transaction::TransactionKey::from_request(&request)
            .expect("wire transaction");
        let wrong = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-info-stale".to_string(),
            rvoip_sip_core::Method::Info,
            true,
        );

        assert_eq!(
            correlate_inbound_info_transaction(&wrong.to_string(), &request)
                .expect_err("wrong branch was accepted"),
            Some(wire)
        );
    }

    #[test]
    fn inbound_info_correlation_rejects_malformed_event_with_wire_failure_target() {
        let request = parsed_info_request("z9hG4bK-info-wire");
        let wire = rvoip_sip_dialog::transaction::TransactionKey::from_request(&request)
            .expect("wire transaction");

        assert_eq!(
            correlate_inbound_info_transaction("not-a-transaction", &request)
                .expect_err("malformed event transaction was accepted"),
            Some(wire)
        );
    }

    #[test]
    fn missing_or_unparseable_info_request_uses_only_validated_event_failure_target() {
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-info-unusable-request".to_string(),
            rvoip_sip_core::Method::Info,
            true,
        );
        let transaction_id = transaction.to_string();
        let call_id = SessionId("unusable-info-request".to_string());

        assert!(build_incoming_request_from_bytes(call_id.clone(), None, None).is_none());
        assert!(build_incoming_request_from_bytes(
            call_id,
            Some(bytes::Bytes::from_static(b"not a SIP request")),
            None,
        )
        .is_none());
        assert_eq!(
            validated_inbound_info_event_transaction(&transaction_id),
            Some(transaction)
        );
        assert!(validated_inbound_info_event_transaction("not-a-transaction").is_none());
    }

    #[tokio::test]
    async fn closed_shutdown_watch_is_terminal() {
        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        drop(shutdown_tx);
        let changed = shutdown_rx.changed().await;
        assert!(shutdown_change_requests_stop(changed, &shutdown_rx));
    }

    #[tokio::test]
    async fn inbound_invite_first_visible_revision_is_fully_initialized() {
        let store = Arc::new(crate::session_store::SessionStore::new());
        let session_id = SessionId("atomic-inbound-invite".to_string());
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-atomic-inbound".to_string(),
            rvoip_sip_core::Method::Invite,
            true,
        );
        let received_at = std::time::Instant::now();
        let observing_store = Arc::clone(&store);
        let observing_session_id = session_id.clone();
        let transaction_for_initialization = transaction.clone();

        let created = store
            .create_session_initialized(session_id.clone(), Role::UAS, true, move |session| {
                assert!(
                    observing_store
                        .with_session(&observing_session_id, |_| ())
                        .is_err(),
                    "the exact lifetime must remain invisible until initialization completes"
                );
                InboundInviteInitialState {
                    local_uri: "sip:bob@example.test".to_string(),
                    remote_uri: "sip:alice@example.test".to_string(),
                    received_at,
                    transaction: transaction_for_initialization,
                    remote_sdp: Some("v=0\r\na=x-first-revision\r\n".to_string()),
                }
                .apply(session);
            })
            .await
            .expect("publish initialized inbound INVITE lifetime");

        let snapshot = store
            .get_session_snapshot(&session_id)
            .await
            .expect("read first visible inbound INVITE revision");
        assert_eq!(snapshot.revision(), 1);
        assert_eq!(snapshot.local_uri.as_deref(), Some("sip:bob@example.test"));
        assert_eq!(
            snapshot.remote_uri.as_deref(),
            Some("sip:alice@example.test")
        );
        assert_eq!(snapshot.incoming_invite_received_at, Some(received_at));
        assert_eq!(
            snapshot.pending_inbound_invite_transaction_id.as_ref(),
            Some(&transaction)
        );
        assert_eq!(
            snapshot.remote_sdp.as_deref(),
            Some("v=0\r\na=x-first-revision\r\n")
        );
        assert_eq!(
            created.lifecycle_handle.as_ref(),
            snapshot.lifecycle_handle.as_ref()
        );
    }

    #[tokio::test]
    async fn inbound_duplicate_guard_uses_the_canonical_exact_registry_mapping() {
        let store = crate::session_store::SessionStore::new();
        let session_id = SessionId("duplicate-inbound-dialog".to_string());
        let created = store
            .create_session(session_id, Role::UAS, true)
            .await
            .expect("create inbound duplicate-guard session");
        let handle = created
            .lifecycle_handle
            .clone()
            .expect("inbound duplicate-guard exact handle");
        let dialog_id = rvoip_sip_dialog::DialogId::new();
        assert!(!registry_has_exact_dialog(store.registry(), &dialog_id));
        store
            .registry()
            .map_dialog_handle(&handle, dialog_id.clone().into())
            .expect("install canonical inbound dialog mapping");
        assert!(registry_has_exact_dialog(store.registry(), &dialog_id));
        store
            .registry()
            .clear_dialog_handle_retained(&handle, dialog_id.clone().into())
            .expect("clear canonical inbound dialog mapping");
        assert!(!registry_has_exact_dialog(store.registry(), &dialog_id));
    }

    #[test]
    fn inbound_invite_handler_uses_one_initialized_publication() {
        let source = include_str!("session_event_handler.rs");
        let handler = source
            .split("async fn handle_incoming_call_parts")
            .nth(1)
            .and_then(|tail| tail.split("async fn handle_call_established_parts").next())
            .expect("typed inbound INVITE handler source");
        let parse = handler
            .find("let pending_transaction")
            .expect("transaction is parsed before admission and creation");
        let create = handler
            .find(".create_session_initialized(")
            .expect("atomic initialized session creation");
        let dialog_commit = handler
            .find(".commit_exact_inbound_dialog(")
            .expect("exact inbound dialog commit");
        let event = handler
            .find("EventType::IncomingCall")
            .expect("state-machine inbound event");

        assert!(parse < create);
        assert!(create < dialog_commit);
        assert!(dialog_commit < event);
        assert!(!handler.contains(".update_session_with("));
        assert!(handler.contains("remote_sdp: sdp.clone()"));
        assert!(handler.contains("created_session.remote_sdp.clone()"));
        assert!(handler.contains("sdp: session_remote_sdp"));
    }

    #[test]
    fn refer_notify_observations_follow_exact_yaml_commit() {
        let source = include_str!("session_event_handler.rs");
        let handler = source
            .split("async fn handle_notify_received_parts")
            .nth(1)
            .and_then(|tail| {
                tail.split("/// Publish a non-terminal app-level event")
                    .next()
            })
            .expect("typed inbound NOTIFY handler source");
        let exact_handle = handler
            .find("lifecycle_handle: &SessionRegistryHandle")
            .expect("generation-qualified NOTIFY handle parameter");
        let first_wait = handler.find(".await").expect("exact YAML dispatch wait");
        let raw_observation = handler
            .find("Event::NotifyReceived")
            .expect("raw NOTIFY observation");
        let dialog_observation = handler
            .find("Event::DialogPackageNotify")
            .expect("dialog-package observation");
        let exact_dispatch = handler
            .find(".process_refer_notify_exact(")
            .expect("exact ReceiveNOTIFY YAML dispatch");
        let refer = handler
            .split("if event_package.eq_ignore_ascii_case(\"refer\")")
            .nth(1)
            .expect("REFER sipfrag branch");
        let refer_dispatch = refer
            .find(".process_refer_notify_exact(")
            .expect("REFER branch exact dispatch");

        assert!(exact_handle < first_wait);
        assert!(raw_observation < exact_dispatch);
        assert!(dialog_observation < exact_dispatch);
        for observation in [
            "Event::ReferNotify",
            "Event::ReferProgress",
            "Event::TransferTargetAnswered",
            "Event::ReferCompleted",
            "Event::TransferFailed",
        ] {
            assert!(
                refer_dispatch < refer.find(observation).expect("derived REFER observation"),
                "{observation} must follow exact YAML commit"
            );
        }
        assert!(!handler.contains(".update_session_with("));
        assert!(
            !handler.contains(".lifecycle_handle("),
            "NOTIFY handling must not rediscover authority from a raw SessionId"
        );
    }

    #[test]
    fn inbound_refer_stages_before_app_event_and_defers_yaml_acceptance() {
        let source = include_str!("session_event_handler.rs");
        let handler = source
            .split("async fn handle_transfer_requested_parts")
            .nth(1)
            .and_then(|tail| tail.split("async fn handle_ack_received_session").next())
            .expect("typed inbound REFER handler source");
        let stage = handler
            .find(".stage_transfer_request_exact(")
            .expect("exact lane-owned REFER stage");
        let app_event = handler
            .find("Event::ReferReceived")
            .expect("public ReferReceived event");
        let delayed_default = handler
            .find(".spawn_owned_exact(")
            .expect("retained delayed REFER default");
        let yaml_dispatch = handler
            .find(".process_event_exact(")
            .expect("delayed YAML TransferRequested dispatch");

        assert!(stage < app_event);
        assert!(app_event < delayed_default);
        assert!(delayed_default < yaml_dispatch);
        assert!(handler.contains("EventType::TransferRequested"));
        assert!(!handler.contains("record_transfer_request_exact"));
    }

    #[test]
    fn inbound_refer_processing_ack_waits_for_retained_terminal_classification() {
        let source = include_str!("session_event_handler.rs");
        let handler = source
            .split("async fn handle_transfer_requested_parts")
            .nth(1)
            .and_then(|tail| tail.split("async fn handle_ack_received_session").next())
            .expect("typed inbound REFER handler source");
        let admission = handler
            .find(".spawn_owned_exact(")
            .expect("exact retained REFER admission");
        let waiter = handler
            .find("let processing_ack = scheduled.await")
            .expect("causal ACK waits for retained REFER completion");

        assert!(admission < waiter);
        assert!(handler.contains("exact_response_failure_processing_ack("));
        assert!(handler.contains("cancelled before a classified final response"));
        assert!(handler.contains("ended before a classified final response"));
        assert!(handler.contains(".map_err(|error|"));
        assert!(handler.contains("processing_ack.map_err(anyhow::Error::msg)"));
        assert!(!handler.contains("if let Err(error) = scheduled"));
    }

    #[test]
    fn rejected_info_control_delivery_uses_retained_classified_exact_response() {
        let source = include_str!("session_event_handler.rs");
        let event_handler = source
            .split("DialogToSessionEvent::InfoReceived {\n                session_id,")
            .nth(1)
            .and_then(|tail| tail.split("DialogToSessionEvent::MessageReceived {").next())
            .expect("typed inbound INFO event handler source");
        let rejection = source
            .split("async fn send_retained_info_control_rejection")
            .nth(1)
            .and_then(|tail| {
                tail.split("async fn handle_deferred_tracked_request")
                    .next()
            })
            .expect("retained INFO control rejection helper source");
        let exact_author = source
            .split("async fn author_exact_final_response")
            .nth(1)
            .and_then(|tail| tail.split("fn terminal_template_counts").next())
            .expect("shared exact final response author source");

        assert!(event_handler.contains("send_retained_info_control_rejection("));
        assert!(!event_handler.contains("respond(503)?.send()"));
        assert!(rejection.contains("author_exact_final_response("));
        assert!(rejection.contains("StatusCode::ServiceUnavailable"));
        assert!(rejection.contains("claim.complete()"));
        assert!(!rejection.contains("claim.release_after_failure()"));
        assert!(rejection.contains("exact_final_response_result("));
        assert!(exact_author.contains("exact_final_response_retires_routes(outcome)"));
        assert!(exact_author.contains("retire_terminal_response_pending_index"));
    }

    #[tokio::test]
    async fn refer_default_pending_check_rejects_a_stale_registry_revision() {
        let store = crate::session_store::SessionStore::new();
        let session_id = SessionId("refer-default-stale-revision".to_string());
        store
            .create_session(session_id.clone(), Role::UAS, false)
            .await
            .expect("create exact REFER session");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("current exact REFER handle");
        store
            .update_session_exact_with(&handle, None, |session| {
                session.refer_transaction_id = Some("refer-transaction".to_string());
            })
            .expect("stage exact REFER transaction");

        assert!(refer_default_is_pending_exact(
            &store,
            &handle,
            "refer-transaction"
        ));
        assert!(
            !refer_default_is_pending_exact(
                &store,
                &handle.with_next_slot_revision_for_test(),
                "refer-transaction",
            ),
            "a delayed REFER callback must not resolve through a stale registry revision"
        );
    }

    #[tokio::test]
    async fn queued_deferred_delivery_aborts_exact_owner_when_shard_drops_it() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("queued-deferred-shutdown".to_string());
        let handle = tracker_test_handle(&session).await;
        let lease = tracker
            .prepare(
                &handle,
                TrackedInDialogOptions::Info(Arc::new(Default::default())),
            )
            .unwrap();
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-queued-deferred-shutdown".to_string(),
            rvoip_sip_core::Method::Info,
            false,
        );
        let event = DeferredTrackedRequestEvent::Completed {
            handle: handle.clone(),
            transaction_id: transaction.to_string(),
            method: "INFO".to_string(),
            outcome: rvoip_infra_common::events::cross_crate::OutboundRequestOutcome::Timeout,
            response_sdp: None,
        };
        let mut replay = tracker.take_deferred_replay_receiver().unwrap();
        assert_eq!(
            tracker.correlate_or_defer(&handle, TrackedInDialogMethod::Info, &transaction, event,),
            ExactTransactionLookup::Prepared
        );
        tracker.activate(lease, transaction).unwrap();
        let queued = replay.recv().await.expect("deferred replay missing");
        drop(DeferredReplayDelivery::new(tracker.clone(), queued));
        assert_eq!(tracker.deferred_event_count(), 0);
        assert!(!tracker.has_request(&handle, TrackedInDialogMethod::Info));
    }

    #[tokio::test]
    async fn started_deferred_delivery_aborts_owner_if_handler_is_cancelled() {
        let tracker = OutboundInDialogRequestTracker::default();
        let session = SessionId("started-deferred-cancel".to_string());
        let handle = tracker_test_handle(&session).await;
        let lease = tracker
            .prepare(
                &handle,
                TrackedInDialogOptions::Info(Arc::new(Default::default())),
            )
            .unwrap();
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-started-deferred-cancel".to_string(),
            rvoip_sip_core::Method::Info,
            false,
        );
        let event = DeferredTrackedRequestEvent::Completed {
            handle: handle.clone(),
            transaction_id: transaction.to_string(),
            method: "INFO".to_string(),
            outcome: rvoip_infra_common::events::cross_crate::OutboundRequestOutcome::Timeout,
            response_sdp: None,
        };
        let mut replay = tracker.take_deferred_replay_receiver().unwrap();
        assert_eq!(
            tracker.correlate_or_defer(&handle, TrackedInDialogMethod::Info, &transaction, event,),
            ExactTransactionLookup::Prepared
        );
        tracker.activate(lease, transaction).unwrap();
        let queued = replay.recv().await.expect("deferred replay missing");
        let started = DeferredReplayDelivery::new(tracker.clone(), queued)
            .begin()
            .expect("exact replay should start");
        assert_eq!(tracker.deferred_event_count(), 0);
        drop(started);
        assert!(!tracker.has_request(&handle, TrackedInDialogMethod::Info));
    }

    #[test]
    fn exact_auth_cleanup_preserves_newly_staged_same_method_request() {
        let mut session = crate::session_store::SessionState::new(
            SessionId("auth-cleanup-staging-race".to_string()),
            Role::UAC,
        );
        session.pending_auth_transaction_id = Some("old-tx".to_string());
        session.pending_auth_request_uri = Some("sip:target@example.invalid".to_string());
        session.pending_auth_method = Some("INFO".to_string());
        session.pending_auth = Some((401, "challenge".to_string()));
        session.request_auth_retry_count = 7;
        session.pending_info_options = Some(Arc::new(Default::default()));

        assert!(session.clear_tracked_auth_if_transaction("old-tx"));

        assert!(session.pending_info_options.is_some());
        assert!(session.pending_auth_transaction_id.is_none());
        assert!(session.pending_auth_request_uri.is_none());
        assert!(session.pending_auth_method.is_none());
        assert!(session.pending_auth.is_none());
        assert_eq!(session.request_auth_retry_count, 7);
    }

    #[test]
    fn auth_required_dispatch_has_no_raw_id_store_prewrite_or_reread() {
        let source = include_str!("session_event_handler.rs");
        let handler = source
            .split("async fn handle_auth_required_parts")
            .nth(1)
            .and_then(|tail| {
                tail.split("async fn handle_outbound_request_completed_parts")
                    .next()
            })
            .expect("AuthRequired handler source");

        assert!(handler.contains("AuthRequiredStateInput::new"));
        assert!(handler.contains("process_auth_required_on_fresh_task"));
        assert!(!handler.contains("update_session_with"));
        assert!(!handler.contains("with_session"));
    }

    #[test]
    fn outbound_bye_completion_commits_yaml_before_exact_release() {
        let source = include_str!("session_event_handler.rs");
        let dispatch = source
            .split("async fn handle_outbound_request_completed_parts")
            .nth(1)
            .and_then(|tail| {
                tail.split("async fn handle_outbound_bye_completed_parts")
                    .next()
            })
            .expect("outbound completion dispatcher source");
        assert!(dispatch.contains("method.eq_ignore_ascii_case(\"BYE\")"));
        assert!(dispatch.contains("handle_outbound_bye_completed_parts"));

        let bye = source
            .split("async fn handle_outbound_bye_completed_parts")
            .nth(1)
            .and_then(|tail| {
                tail.split("async fn clear_tracked_request_auth_state")
                    .next()
            })
            .expect("outbound BYE completion source");
        let transition = bye
            .find("commit_local_bye_lifecycle_exact")
            .expect("canonical local-BYE YAML commit");
        let release = bye
            .find("publish_and_release_session")
            .expect("exact local-BYE terminal release");
        assert!(transition < release);
        assert!(!bye.contains("CallTerminating"));
    }

    #[test]
    fn stale_completion_cannot_clear_newer_auth_owner() {
        let mut session = crate::session_store::SessionState::new(
            SessionId("auth-cleanup-exact-owner".to_string()),
            Role::UAC,
        );
        session.pending_auth_transaction_id = Some("new-tx".to_string());
        session.pending_auth_request_uri = Some("sip:new@example.invalid".to_string());
        session.pending_auth_method = Some("NOTIFY".to_string());

        assert!(!session.clear_tracked_auth_if_transaction("old-tx"));

        assert_eq!(
            session.pending_auth_transaction_id.as_deref(),
            Some("new-tx")
        );
        assert_eq!(session.pending_auth_method.as_deref(), Some("NOTIFY"));
    }

    #[test]
    fn lifecycle_capacity_errors_are_the_only_admission_errors_mapped_to_overload() {
        for error in [
            SessionAdmissionError::CapacityExhausted,
            SessionAdmissionError::RetainedCapacityExhausted,
        ] {
            assert!(is_session_lifecycle_capacity_exhaustion(&error));
        }

        assert!(!is_session_lifecycle_capacity_exhaustion(
            &SessionAdmissionError::AlreadyActive
        ));
        assert!(!is_session_lifecycle_capacity_exhaustion(
            &SessionError::InternalError("not-capacity".to_string())
        ));
    }

    struct DropSignal(Option<oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(signal) = self.0.take() {
                let _ = signal.send(());
            }
        }
    }

    #[tokio::test]
    async fn dropping_state_machine_join_aborts_the_owned_task() {
        let (started_tx, started_rx) = oneshot::channel();
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async move {
            let _drop_signal = DropSignal(Some(dropped_tx));
            let _ = started_tx.send(());
            std::future::pending::<StateMachineProcessResult>().await
        }));

        started_rx.await.expect("auth-retry task started");
        let join = Box::pin(task.join());
        drop(join);

        tokio::time::timeout(std::time::Duration::from_secs(1), dropped_rx)
            .await
            .expect("cancelled state-machine task did not stop")
            .expect("state-machine task drop signal closed");
    }

    #[tokio::test]
    async fn state_machine_task_panics_map_to_a_fixed_internal_error_class() {
        let task = AbortStateMachineTaskOnDrop::new(tokio::spawn(async {
            panic!("synthetic state-machine dispatch panic");
            #[allow(unreachable_code)]
            std::future::pending::<StateMachineProcessResult>().await
        }));

        let error = join_state_machine_task(task)
            .await
            .expect_err("panicked state-machine task must fail");
        match error.downcast_ref::<SessionError>() {
            Some(SessionError::InternalError(detail)) => {
                assert_eq!(detail, STATE_MACHINE_DISPATCH_JOIN_FAILURE);
            }
            other => panic!("unexpected state-machine join error: {other:?}"),
        }
    }

    #[test]
    fn outbound_auth_terminal_reasons_and_log_metadata_are_value_free() {
        const SECRET: &str = "terminal-auth-provider-secret-canary";
        let lower = SessionError::AuthError(format!("AKA provider failed: {SECRET}"));
        let class = OutboundAuthTerminalClass::from_error(&lower);
        assert_eq!(class, OutboundAuthTerminalClass::ChallengeResponse);

        let reason = CallFailureReason::OutboundInviteAuth(class).into_event_reason();
        assert_eq!(
            reason,
            "INVITE authentication failed (class=challenge-response)"
        );
        assert!(!reason.contains(SECRET));

        let legacy_event = crate::api::events::Event::CallFailed {
            call_id: SessionId("safe-call".to_string()),
            status_code: 401,
            reason: reason.clone(),
        };
        let detailed = build_incoming_response_from_bytes(
            SessionId("safe-call".to_string()),
            401,
            reason.clone(),
            None,
            None,
        );
        assert!(!format!("{legacy_event:?}").contains(SECRET));
        assert!(!detailed.reason_phrase.contains(SECRET));
        assert_eq!(detailed.reason_phrase, reason);

        let protocol_reason = format!("peer-controlled reason {SECRET}");
        let log_metadata = format!("{:?}", CallFailureDiagnostics::new(&protocol_reason));
        assert!(!log_metadata.contains(SECRET));
        assert!(log_metadata.contains(&format!("reason_bytes: {}", protocol_reason.len())));
        assert_eq!(safe_auth_method_label(SECRET), "extension");
    }

    #[test]
    fn auth_failure_source_has_no_lower_error_or_reason_log_relay() {
        let handler_source = include_str!("session_event_handler.rs");
        for forbidden in [
            ["Failed to process AuthRequired({}) for session {}", ": {}"].concat(),
            ["INVITE authentication failed", ": {}"].concat(),
            ["[handle_call_failed] session={} status={} ", "reason={}"].concat(),
            ["Failed to extract session_id from event", ": {}"].concat(),
        ] {
            assert!(
                !handler_source.contains(&forbidden),
                "auth diagnostic relay returned: {forbidden}"
            );
        }

        let actions_source = include_str!("../state_machine/actions.rs");
        assert_eq!(
            actions_source
                .matches("OutboundAuthOperation::Invite")
                .count(),
            1
        );
        assert_eq!(
            actions_source
                .matches("OutboundAuthOperation::Request")
                .count(),
            1
        );
        assert!(!actions_source.contains("realm={}, nonce={}"));

        let executor_source = include_str!("../state_machine/executor.rs");
        assert!(!executor_source.contains(&["Processing event ", "{:?}"].concat()));
        assert!(!executor_source.contains(&["Executing transition for {:?} + ", "{:?}"].concat()));
        assert!(!executor_source
            .contains(&["No transition defined for ", "{:?}", "\"", ", key"].concat()));

        let dialog_source = include_str!("dialog_adapter.rs");
        assert_eq!(
            dialog_source
                .matches("OutboundAuthOperation::Register")
                .count(),
            1
        );
        let unified_source = include_str!("../api/unified.rs");
        assert_eq!(
            unified_source
                .matches("OutboundAuthOperation::Request")
                .count(),
            1
        );
    }

    #[test]
    fn sipfrag_parses_progress_and_final() {
        assert_eq!(
            parse_sipfrag_status_line("SIP/2.0 180 Ringing\r\n"),
            Some((180, "Ringing".into()))
        );
        assert_eq!(
            parse_sipfrag_status_line("SIP/2.0 200 OK"),
            Some((200, "OK".into()))
        );
        assert_eq!(
            parse_sipfrag_status_line("SIP/2.0 486 Busy Here\r\n"),
            Some((486, "Busy Here".into()))
        );
    }

    #[test]
    fn sipfrag_rejects_malformed_input() {
        assert!(parse_sipfrag_status_line("HTTP/1.1 200 OK").is_none());
        assert!(parse_sipfrag_status_line("SIP/2.0 notanumber Ringing").is_none());
        assert!(parse_sipfrag_status_line("").is_none());
    }

    #[test]
    fn sip_trace_owner_filter_accepts_only_matching_owner() {
        assert!(sip_trace_owner_matches(Some("owner-a"), "owner-a"));
        assert!(!sip_trace_owner_matches(Some("owner-a"), "owner-b"));
        assert!(!sip_trace_owner_matches(None, "owner-a"));
    }

    #[tokio::test]
    async fn sip_trace_maps_unique_exact_sip_call_id_to_session_id() {
        let store = crate::session_store::SessionStore::new();
        let session_id = SessionId("session-1".into());
        let created = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create trace-correlated session");
        let handle = created
            .lifecycle_handle
            .expect("trace-correlated exact handle");
        store
            .registry()
            .map_dialog_identity_handle(&handle, crate::types::DialogId::new(), "wire-call".into())
            .expect("bind exact SIP Call-ID");
        let event = trace_event(None, Some("wire-call"));

        assert_eq!(map_sip_trace_session_id(&event, &store), Some(session_id));
    }

    #[test]
    fn sip_trace_direct_session_id_wins_over_call_id_mapping() {
        let store = crate::session_store::SessionStore::new();
        let event = trace_event(Some("direct-session"), Some("wire-call"));

        assert_eq!(
            map_sip_trace_session_id(&event, &store),
            Some(SessionId("direct-session".into()))
        );
    }

    #[test]
    fn sip_trace_unknown_call_id_remains_uncorrelated() {
        let store = crate::session_store::SessionStore::new();
        let event = trace_event(None, Some("unknown-wire-call"));

        assert_eq!(map_sip_trace_session_id(&event, &store), None);
    }

    #[test]
    fn dialog_bye_requires_committed_terminated_template() {
        let session_id = SessionId::from("strict-bye-terminal");
        assert!(
            committed_bye_termination(&session_id, &missing_result(CallState::Active)).is_err()
        );

        let malformed = committed_result(CallState::Active, CallState::Terminated, vec![]);
        assert!(committed_bye_termination(&session_id, &malformed).is_err());

        let cleanup = vec![
            crate::state_table::Action::CleanupDialog,
            crate::state_table::Action::CleanupMedia,
        ];
        let valid = committed_result_with_actions(
            CallState::Active,
            CallState::Terminated,
            vec![crate::state_table::EventTemplate::CallTerminated],
            cleanup.clone(),
        );
        assert!(committed_bye_termination(&session_id, &valid).is_ok());

        let duplicate = committed_result_with_actions(
            CallState::Active,
            CallState::Terminated,
            vec![
                crate::state_table::EventTemplate::CallTerminated,
                crate::state_table::EventTemplate::CallTerminated,
            ],
            cleanup,
        );
        assert!(committed_bye_termination(&session_id, &duplicate).is_err());
    }

    #[test]
    fn generic_dialog_termination_requires_a_committed_terminal_yaml_event() {
        let session_id = SessionId::from("uncommitted-generic-terminal");
        let missing = ProcessEventResult {
            old_state: CallState::Idle,
            next_state: None,
            transition: None,
            actions_executed: vec![],
            events_published: vec![],
        };
        assert!(committed_dialog_termination(&session_id, Role::UAC, &missing).is_err());

        let malformed = ProcessEventResult {
            old_state: CallState::Active,
            next_state: Some(CallState::Terminated),
            transition: Some(Transition {
                guards: vec![],
                actions: vec![],
                next_state: Some(CallState::Terminated),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![],
            }),
            actions_executed: vec![],
            events_published: vec![],
        };
        assert!(committed_dialog_termination(&session_id, Role::UAC, &malformed).is_err());
    }

    #[test]
    fn generic_dialog_termination_accepts_exactly_one_terminated_outcome() {
        let session_id = SessionId::from("committed-generic-terminal");
        let result = ProcessEventResult {
            old_state: CallState::Active,
            next_state: Some(CallState::Terminated),
            transition: Some(Transition {
                guards: vec![],
                actions: vec![],
                next_state: Some(CallState::Terminated),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![crate::state_table::EventTemplate::CallTerminated],
            }),
            actions_executed: vec![],
            events_published: vec![crate::state_table::EventTemplate::CallTerminated],
        };

        assert!(matches!(
            committed_dialog_termination(&session_id, Role::UAS, &result),
            Ok(CommittedDialogTermination::Ended)
        ));
    }

    #[test]
    fn generic_dialog_cancellation_requires_uac_cancelling_yaml_outcome() {
        let session_id = SessionId::from("committed-cancel-terminal");
        let result = ProcessEventResult {
            old_state: CallState::Cancelling,
            next_state: Some(CallState::Cancelled),
            transition: Some(Transition {
                guards: vec![],
                actions: vec![],
                next_state: Some(CallState::Cancelled),
                condition_updates: ConditionUpdates::none(),
                publish_events: vec![crate::state_table::EventTemplate::CallCancelled],
            }),
            actions_executed: vec![],
            events_published: vec![crate::state_table::EventTemplate::CallCancelled],
        };

        assert!(matches!(
            committed_dialog_termination(&session_id, Role::UAC, &result),
            Ok(CommittedDialogTermination::Cancelled)
        ));
        assert!(committed_dialog_termination(&session_id, Role::UAS, &result).is_err());

        let duplicate = ProcessEventResult {
            events_published: vec![
                crate::state_table::EventTemplate::CallCancelled,
                crate::state_table::EventTemplate::CallCancelled,
            ],
            ..result
        };
        assert!(committed_dialog_termination(&session_id, Role::UAC, &duplicate).is_err());
    }

    #[test]
    fn call_failure_requires_matching_terminal_template_or_nonterminal_rollback() {
        let session_id = SessionId::from("strict-call-failure");
        assert!(committed_call_failure(
            &session_id,
            Role::UAC,
            &missing_result(CallState::Initiating)
        )
        .is_err());

        let malformed = committed_result(
            CallState::Initiating,
            CallState::Failed(FailureReason::Other),
            vec![],
        );
        assert!(committed_call_failure(&session_id, Role::UAC, &malformed).is_err());

        let terminal_without_cleanup = committed_result(
            CallState::Initiating,
            CallState::Failed(FailureReason::Other),
            vec![crate::state_table::EventTemplate::CallFailed],
        );
        assert!(committed_call_failure(&session_id, Role::UAC, &terminal_without_cleanup).is_err());

        let cleanup = vec![
            crate::state_table::Action::CleanupDialog,
            crate::state_table::Action::CleanupMedia,
        ];
        let failed = committed_result_with_actions(
            CallState::Initiating,
            CallState::Failed(FailureReason::Other),
            vec![crate::state_table::EventTemplate::CallFailed],
            cleanup.clone(),
        );
        assert_eq!(
            committed_call_failure(&session_id, Role::UAC, &failed).unwrap(),
            CommittedCallFailure::Failed
        );

        let cancelled = committed_result_with_actions(
            CallState::CancelPending,
            CallState::Cancelled,
            vec![crate::state_table::EventTemplate::CallCancelled],
            cleanup,
        );
        assert_eq!(
            committed_call_failure(&session_id, Role::UAC, &cancelled).unwrap(),
            CommittedCallFailure::Cancelled
        );
        assert!(committed_call_failure(&session_id, Role::UAS, &cancelled).is_err());

        let rollback = committed_result_with_actions(
            CallState::HoldPending,
            CallState::Active,
            vec![],
            vec![crate::state_table::Action::ClearPendingReinvite],
        );
        assert_eq!(
            committed_call_failure(&session_id, Role::UAC, &rollback).unwrap(),
            CommittedCallFailure::NonTerminal
        );

        let arbitrary_nonterminal = committed_result(CallState::Active, CallState::OnHold, vec![]);
        assert!(committed_call_failure(&session_id, Role::UAC, &arbitrary_nonterminal).is_err());
    }

    #[test]
    fn session_interval_retry_requires_exact_uac_initiating_self_loop() {
        let session_id = SessionId::from("strict-session-interval-retry");
        assert!(committed_session_interval_retry(
            &session_id,
            Role::UAC,
            &missing_result(CallState::Initiating)
        )
        .is_err());

        let malformed_self_loop =
            committed_result(CallState::Initiating, CallState::Initiating, vec![]);
        assert!(
            committed_session_interval_retry(&session_id, Role::UAC, &malformed_self_loop).is_err()
        );

        let retry = committed_result_with_actions(
            CallState::Initiating,
            CallState::Initiating,
            vec![],
            vec![crate::state_table::Action::SendINVITEWithBumpedSessionExpires],
        );
        assert!(committed_session_interval_retry(&session_id, Role::UAC, &retry).is_ok());
        assert!(committed_session_interval_retry(&session_id, Role::UAS, &retry).is_err());

        let terminal = committed_result(
            CallState::Initiating,
            CallState::Failed(FailureReason::Other),
            vec![crate::state_table::EventTemplate::CallFailed],
        );
        assert!(committed_session_interval_retry(&session_id, Role::UAC, &terminal).is_err());
    }

    #[test]
    fn dialog_200_projects_initial_answer_only_from_exact_yaml_template() {
        let session_id = SessionId::from("strict-dialog-200");
        assert!(committed_dialog_200(
            &session_id,
            Role::UAC,
            &missing_result(CallState::Initiating)
        )
        .is_err());

        let malformed_initial = committed_result(CallState::Initiating, CallState::Active, vec![]);
        assert!(committed_dialog_200(&session_id, Role::UAC, &malformed_initial).is_err());

        let fast_answer = committed_result(
            CallState::Initiating,
            CallState::Active,
            vec![crate::state_table::EventTemplate::Custom(
                "CallAnswered".to_string(),
            )],
        );
        assert_eq!(
            committed_dialog_200(&session_id, Role::UAC, &fast_answer).unwrap(),
            CommittedDialog200::InitialAnswer
        );

        let ringing_answer = committed_result(
            CallState::Ringing,
            CallState::Active,
            vec![crate::state_table::EventTemplate::CallEstablished],
        );
        assert_eq!(
            committed_dialog_200(&session_id, Role::UAC, &ringing_answer).unwrap(),
            CommittedDialog200::InitialAnswer
        );

        for reinvite in [
            committed_result(
                CallState::HoldPending,
                CallState::OnHold,
                vec![crate::state_table::EventTemplate::CallOnHold],
            ),
            committed_result(
                CallState::Resuming,
                CallState::Active,
                vec![crate::state_table::EventTemplate::CallResumed],
            ),
        ] {
            assert_eq!(
                committed_dialog_200(&session_id, Role::UAC, &reinvite).unwrap(),
                CommittedDialog200::NonInitial
            );
        }

        let malformed_reinvite = committed_result(
            CallState::HoldPending,
            CallState::OnHold,
            vec![crate::state_table::EventTemplate::CallEstablished],
        );
        assert!(committed_dialog_200(&session_id, Role::UAC, &malformed_reinvite).is_err());
    }

    fn trace_event(session_id: Option<&str>, sip_call_id: Option<&str>) -> SipTraceEvent {
        SipTraceEvent {
            owner_id: "owner-a".into(),
            direction: SipTraceDirection::Inbound,
            transport: "UDP".into(),
            local_addr: "127.0.0.1:5060".into(),
            remote_addr: "127.0.0.1:5080".into(),
            timestamp_unix_millis: 1,
            start_line: "INVITE sip:bob@example.com SIP/2.0".into(),
            sip_call_id: sip_call_id.map(str::to_string),
            session_id: session_id.map(str::to_string),
            raw_message: "INVITE sip:bob@example.com SIP/2.0\n\n".into(),
            original_len: 40,
            truncated: false,
            redacted: true,
        }
    }
}
