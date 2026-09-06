use crate::session_lifecycle::{OwnedOperation, OwnedOperationCompletion, SessionOperationKind};
use crate::state_table::SessionId;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tracing::{debug, error, info};

use crate::{
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
    cleanup_diag::{self, CleanupStage},
    session_registry::SessionRegistryHandle,
    session_store::{SessionState, SessionStateSnapshot, SessionStore},
    state_table::{Action, EventTemplate, EventType, StateKey, Transition, MASTER_TABLE},
    types::CallState,
    // Event import removed - events handled by SessionCrossCrateEventHandler
};

use super::{actions, guards};

const REINVITE_RETRY_COMPLETION_GRACE: Duration = Duration::from_secs(30);
const REFER_NOTIFY_COMPLETION_GRACE: Duration = Duration::from_secs(2);
const SESSION_REFRESH_COMPLETION_GRACE: Duration = Duration::from_secs(35);

/// Reserved state-table event tags for the crate-private RFC 4028 driver.
/// Public callers can construct `MediaEvent(String)`, so every tag is also
/// guarded by a typed [`SessionRefreshStateInput`] capability under the exact
/// session lane. A string alone is never sufficient to enter these rows.
pub(crate) const SESSION_REFRESH_EVENT_PREFIX: &str = "__rvoip_internal.session_refresh.";
pub(crate) const SESSION_REFRESH_DUE_EVENT: &str = "__rvoip_internal.session_refresh.update_due";
pub(crate) const SESSION_REFRESH_REINVITE_DUE_EVENT: &str =
    "__rvoip_internal.session_refresh.reinvite_due";
pub(crate) const SESSION_REFRESH_UPDATE_OK_EVENT: &str =
    "__rvoip_internal.session_refresh.update_ok";
pub(crate) const SESSION_REFRESH_UPDATE_FAILED_EVENT: &str =
    "__rvoip_internal.session_refresh.update_failed";
pub(crate) const SESSION_REFRESH_REINVITE_OK_EVENT: &str =
    "__rvoip_internal.session_refresh.reinvite_ok";
pub(crate) const SESSION_REFRESH_REINVITE_FAILED_EVENT: &str =
    "__rvoip_internal.session_refresh.reinvite_failed";
pub(crate) const SESSION_REFRESH_PEER_EXPIRED_EVENT: &str =
    "__rvoip_internal.session_refresh.peer_expired";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeferredReinviteRetryResult {
    Dispatched,
    Cancelled,
    Stale,
    DispatchFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeferredTransferNotifyResult {
    Dispatched,
    DispatchFailed,
    Cancelled,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DeferredSessionRefreshResult {
    Dispatched,
    Cancelled,
    Stale,
    DispatchFailed,
}

async fn rollback_owned_session_refresh(
    operation: OwnedOperation,
    value: DeferredSessionRefreshResult,
) -> OwnedOperationCompletion<DeferredSessionRefreshResult> {
    operation
        .rollback(value)
        .await
        .unwrap_or_else(|_| panic!("RFC 4028 exact timer rollback failed"))
}

async fn run_deferred_session_refresh(
    operation: OwnedOperation,
    state_machine: Arc<StateMachine>,
    handle: SessionRegistryHandle,
    effect: actions::SessionRefreshTimerEffect,
) -> OwnedOperationCompletion<DeferredSessionRefreshResult> {
    let Some(mut cancellation) = operation.cancellation() else {
        return rollback_owned_session_refresh(operation, DeferredSessionRefreshResult::Cancelled)
            .await;
    };
    if !effect.delay.is_zero() {
        tokio::select! {
            _ = tokio::time::sleep(effect.delay) => {}
            () = wait_for_owned_reinvite_retry_cancellation(&mut cancellation) => {
                return rollback_owned_session_refresh(
                    operation,
                    DeferredSessionRefreshResult::Cancelled,
                ).await;
            }
        }
    }

    let input = match effect.kind {
        actions::SessionRefreshDeadlineKind::UpdateDue => SessionRefreshStateInput::UpdateDue {
            timer_generation: effect.generation,
        },
        actions::SessionRefreshDeadlineKind::ReinviteDue => SessionRefreshStateInput::ReinviteDue {
            timer_generation: effect.generation,
        },
        actions::SessionRefreshDeadlineKind::PeerExpired => SessionRefreshStateInput::PeerExpired {
            timer_generation: effect.generation,
        },
        actions::SessionRefreshDeadlineKind::UpdateFailed => SessionRefreshStateInput::UpdateFailed,
        actions::SessionRefreshDeadlineKind::ReinviteFailed => {
            SessionRefreshStateInput::ReinviteFailed
        }
    };
    let current = match state_machine.store.get_session_snapshot_exact(&handle) {
        Ok(current) => current,
        Err(_) => {
            return rollback_owned_session_refresh(operation, DeferredSessionRefreshResult::Stale)
                .await;
        }
    };
    if input.validate(current.state()).is_err() || *cancellation.borrow() {
        return rollback_owned_session_refresh(operation, DeferredSessionRefreshResult::Stale)
            .await;
    }

    let committed = match operation.commit() {
        Ok(committed) => committed,
        Err(failure) => {
            return rollback_owned_session_refresh(
                failure.into_operation(),
                DeferredSessionRefreshResult::Cancelled,
            )
            .await;
        }
    };
    let result = match state_machine
        .process_session_refresh_exact(&handle, input)
        .await
    {
        Ok(result) if result.transition.is_some() => DeferredSessionRefreshResult::Dispatched,
        Ok(_) => DeferredSessionRefreshResult::Stale,
        Err(error) => {
            debug!(
                session_id = %handle.session_id(),
                %error,
                "RFC 4028 exact timer dispatch failed"
            );
            DeferredSessionRefreshResult::DispatchFailed
        }
    };
    committed.complete(result)
}

fn reinvite_retry_matches(
    session: &SessionState,
    kind: &crate::session_store::state::PendingReinvite,
    attempt: u8,
) -> bool {
    session.pending_reinvite.as_ref() == Some(kind) && session.reinvite_retry_attempts == attempt
}

async fn wait_for_owned_reinvite_retry_cancellation(
    cancellation: &mut tokio::sync::watch::Receiver<bool>,
) {
    loop {
        if *cancellation.borrow() {
            return;
        }
        if cancellation.changed().await.is_err() {
            return;
        }
    }
}

async fn rollback_owned_reinvite_retry(
    operation: OwnedOperation,
    value: DeferredReinviteRetryResult,
) -> OwnedOperationCompletion<DeferredReinviteRetryResult> {
    operation
        .rollback(value)
        .await
        .unwrap_or_else(|_| panic!("re-INVITE retry exact rollback failed"))
}

/// Wait outside the state-machine lane, then acquire and validate the retained
/// exact lifetime before granting one retry dispatch permission.
async fn run_deferred_reinvite_retry<F, Fut, E>(
    operation: OwnedOperation,
    store: Arc<SessionStore>,
    handle: SessionRegistryHandle,
    effect: actions::ReinviteRetryEffect,
    dispatch: F,
) -> OwnedOperationCompletion<DeferredReinviteRetryResult>
where
    F: FnOnce(crate::session_store::state::PendingReinvite) -> Fut + Send,
    Fut: std::future::Future<Output = Result<(), E>> + Send,
    E: std::fmt::Display + Send,
{
    let Some(mut cancellation) = operation.cancellation() else {
        return rollback_owned_reinvite_retry(operation, DeferredReinviteRetryResult::Cancelled)
            .await;
    };

    tokio::select! {
        _ = tokio::time::sleep(effect.backoff) => {}
        () = wait_for_owned_reinvite_retry_cancellation(&mut cancellation) => {
            return rollback_owned_reinvite_retry(
                operation,
                DeferredReinviteRetryResult::Cancelled,
            ).await;
        }
    }

    let Some(lane) = store.state_machine_lane_exact(&handle) else {
        return rollback_owned_reinvite_retry(operation, DeferredReinviteRetryResult::Stale).await;
    };
    let _lane = tokio::select! {
        lane = lane.lock_owned() => lane,
        () = wait_for_owned_reinvite_retry_cancellation(&mut cancellation) => {
            return rollback_owned_reinvite_retry(
                operation,
                DeferredReinviteRetryResult::Cancelled,
            ).await;
        }
    };

    let current = match store.get_session_snapshot_exact(&handle) {
        Ok(current) => current,
        Err(_) => {
            return rollback_owned_reinvite_retry(operation, DeferredReinviteRetryResult::Stale)
                .await;
        }
    };
    if !reinvite_retry_matches(current.state(), &effect.kind, effect.attempt) {
        return rollback_owned_reinvite_retry(operation, DeferredReinviteRetryResult::Stale).await;
    }
    if *cancellation.borrow() {
        return rollback_owned_reinvite_retry(operation, DeferredReinviteRetryResult::Cancelled)
            .await;
    }

    // Commit the exact operation immediately before the first retry side
    // effect. A teardown that won the race prevents the wire attempt; after
    // this point teardown drains the retained supervisor instead of treating
    // a successful dispatch as abandoned work.
    let committed = match operation.commit() {
        Ok(committed) => committed,
        Err(failure) => {
            return rollback_owned_reinvite_retry(
                failure.into_operation(),
                DeferredReinviteRetryResult::Cancelled,
            )
            .await;
        }
    };

    let result = match dispatch(effect.kind).await {
        Ok(()) => DeferredReinviteRetryResult::Dispatched,
        Err(error) => {
            debug!(
                session_id = %handle.session_id(),
                %error,
                "deferred re-INVITE retry dispatch failed"
            );
            DeferredReinviteRetryResult::DispatchFailed
        }
    };
    committed.complete(result)
}

async fn rollback_owned_transfer_notify(
    operation: OwnedOperation,
    value: DeferredTransferNotifyResult,
) -> OwnedOperationCompletion<DeferredTransferNotifyResult> {
    operation
        .rollback(value)
        .await
        .unwrap_or_else(|_| panic!("REFER NOTIFY exact rollback failed"))
}

/// Dispatch transfer progress only after the target-leg transition commits,
/// on the exact transferor lifetime that owns the REFER subscription.
///
/// The target and transferor use different state-machine lanes. Deferring this
/// operation prevents either lane from awaiting the other and makes a reused
/// raw session identifier incapable of receiving a stale NOTIFY.
async fn run_deferred_transfer_notify(
    operation: OwnedOperation,
    store: Arc<SessionStore>,
    dialog_adapter: Arc<DialogAdapter>,
    effect: actions::TransferNotifyEffect,
) -> OwnedOperationCompletion<DeferredTransferNotifyResult> {
    let Some(mut cancellation) = operation.cancellation() else {
        return rollback_owned_transfer_notify(operation, DeferredTransferNotifyResult::Cancelled)
            .await;
    };

    // RFC 6665 §4.1.2 requires NOTIFY requests for one subscription to be
    // sent in order. Keep this exact-subscription owner through transaction
    // completion, but never keep the session state-machine lane while waiting
    // for the response: a 401/407 retry and teardown both need that lane.
    let notify_dispatch_lane = dialog_adapter.transfer_notify_dispatch_lane(&effect.transferor);
    let mut notify_sequence = tokio::select! {
        sequence = notify_dispatch_lane.lock_owned() => sequence,
        () = wait_for_owned_reinvite_retry_cancellation(&mut cancellation) => {
            return rollback_owned_transfer_notify(
                operation,
                DeferredTransferNotifyResult::Cancelled,
            ).await;
        }
    };
    if notify_sequence.terminal_issued
        || notify_sequence
            .last_status_code
            .is_some_and(|status| effect.status_code <= status)
    {
        return rollback_owned_transfer_notify(operation, DeferredTransferNotifyResult::Stale)
            .await;
    }

    let Some(lane) = store.state_machine_lane_exact(&effect.transferor) else {
        return rollback_owned_transfer_notify(operation, DeferredTransferNotifyResult::Stale)
            .await;
    };
    let lane_guard = tokio::select! {
        lane = lane.lock_owned() => lane,
        () = wait_for_owned_reinvite_retry_cancellation(&mut cancellation) => {
            return rollback_owned_transfer_notify(
                operation,
                DeferredTransferNotifyResult::Cancelled,
            ).await;
        }
    };

    let current = match store.get_session_snapshot_exact(&effect.transferor) {
        Ok(current) => current,
        Err(_) => {
            return rollback_owned_transfer_notify(operation, DeferredTransferNotifyResult::Stale)
                .await;
        }
    };
    if current.lifecycle_handle.as_ref() != Some(&effect.transferor) || *cancellation.borrow() {
        return rollback_owned_transfer_notify(operation, DeferredTransferNotifyResult::Cancelled)
            .await;
    }

    // Commit immediately before the first wire side effect. If teardown won
    // the race, no NOTIFY and no corresponding public observation are emitted.
    let committed = match operation.commit() {
        Ok(committed) => committed,
        Err(failure) => {
            return rollback_owned_transfer_notify(
                failure.into_operation(),
                DeferredTransferNotifyResult::Cancelled,
            )
            .await;
        }
    };

    let transaction = match dialog_adapter
        .send_refer_notify_lane_owned(
            &effect.transferor,
            current.state(),
            effect.status_code,
            &effect.reason,
        )
        .await
    {
        Ok(transaction) => transaction,
        Err(error) => {
            debug!(
                transferor = %effect.transferor.session_id(),
                status_code = effect.status_code,
                %error,
                "deferred REFER NOTIFY dispatch failed"
            );
            drop(lane_guard);
            return committed.complete(DeferredTransferNotifyResult::DispatchFailed);
        }
    };
    notify_sequence.last_status_code = Some(effect.status_code);
    if effect.status_code >= 200 {
        notify_sequence.terminal_issued = true;
    }
    drop(lane_guard);

    // These are observational projections of the already committed target-leg
    // result. Publish them after the wire attempt, without awaiting observers.
    let observation_handle = effect.transferor.clone();
    for event in effect.observations {
        admit_committed_transfer_observation(event, |event| {
            dialog_adapter.publish_api_event_exact(&observation_handle, event);
        });
    }

    // The lower transaction event removes this exact tracker entry on any
    // final response, timeout, or transport failure. Holding only the NOTIFY
    // sequence owner here makes a later progress/terminal task wait without
    // blocking authentication, state transitions, or cleanup.
    let result = tokio::select! {
        () = dialog_adapter.outbound_request_tracker.wait_until_vacant(
            &effect.transferor,
            crate::adapters::outbound_request_tracker::TrackedInDialogMethod::Notify,
        ) => DeferredTransferNotifyResult::Dispatched,
        () = wait_for_owned_reinvite_retry_cancellation(&mut cancellation) => {
            DeferredTransferNotifyResult::Cancelled
        }
    };
    debug!(
        transferor = %effect.transferor.session_id(),
        status_code = effect.status_code,
        transaction = %transaction,
        ?result,
        "deferred REFER NOTIFY sequence completed"
    );
    drop(notify_sequence);
    committed.complete(result)
}

/// Result of processing an event through the state machine
#[derive(Debug, Clone)]
pub struct ProcessEventResult {
    /// The old state before processing
    pub old_state: CallState,
    /// The new state after processing
    pub next_state: Option<CallState>,
    /// The transition that was executed (if any)
    pub transition: Option<Transition>,
    /// Actions that were executed
    pub actions_executed: Vec<Action>,
    /// Events that were published
    pub events_published: Vec<EventTemplate>,
}

/// Result of one exact-lifetime authentication dispatch. The pre-event state
/// is captured while holding the same lane that applies wire correlation and
/// executes the transition, so error classification never needs a racy raw-ID
/// reread.
pub(crate) struct AuthRequiredProcessOutcome {
    pub(crate) state_before_auth: CallState,
    pub(crate) result: Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>>,
}

/// Application-authored SIP response data admitted with one lifecycle event.
///
/// The input is deliberately separate from `EventType`: the public event enum
/// remains frozen, while response builders can still carry headers, a custom
/// provisional status, and caller-supplied SDP through the exact-session lane.
#[derive(Default)]
pub(crate) struct ResponseStateInput {
    local_sdp: Option<String>,
    sdp_negotiated: Option<bool>,
    extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    status_override: Option<u16>,
}

impl ResponseStateInput {
    pub(crate) fn accept(
        local_sdp: Option<String>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Self {
        let sdp_negotiated = local_sdp.as_ref().map(|_| true);
        Self {
            local_sdp,
            sdp_negotiated,
            extra_headers,
            status_override: None,
        }
    }

    pub(crate) fn provisional(
        status: u16,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Self {
        Self {
            extra_headers,
            status_override: Some(status),
            ..Default::default()
        }
    }

    pub(crate) fn headers(extra_headers: Vec<rvoip_sip_core::types::TypedHeader>) -> Self {
        Self {
            extra_headers,
            ..Default::default()
        }
    }

    fn apply(self, session: &mut SessionState) {
        if let Some(local_sdp) = self.local_sdp {
            session.local_sdp = Some(local_sdp);
        }
        if let Some(sdp_negotiated) = self.sdp_negotiated {
            session.sdp_negotiated = sdp_negotiated;
        }
        // Always replace both slots, including with an empty/None value. One
        // response event must never inherit another event's envelope.
        session.reject_response_extras = Some(self.extra_headers);
        session.pending_response_status_override = self.status_override;
    }
}

/// Capability-bearing input for reserved RFC 4028 state-table events.
///
/// The payload never crosses a public API or the observational event bus. It
/// is created only by an exact retained deadline or an exact transaction
/// completion and is validated again after acquiring the session lane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionRefreshStateInput {
    UpdateDue { timer_generation: u64 },
    ReinviteDue { timer_generation: u64 },
    UpdateSucceeded,
    UpdateFailed,
    ReinviteSucceeded,
    ReinviteFailed,
    PeerExpired { timer_generation: u64 },
}

impl SessionRefreshStateInput {
    fn event(&self) -> EventType {
        let tag = match self {
            Self::UpdateDue { .. } => SESSION_REFRESH_DUE_EVENT,
            Self::ReinviteDue { .. } => SESSION_REFRESH_REINVITE_DUE_EVENT,
            Self::UpdateSucceeded => SESSION_REFRESH_UPDATE_OK_EVENT,
            Self::UpdateFailed => SESSION_REFRESH_UPDATE_FAILED_EVENT,
            Self::ReinviteSucceeded => SESSION_REFRESH_REINVITE_OK_EVENT,
            Self::ReinviteFailed => SESSION_REFRESH_REINVITE_FAILED_EVENT,
            Self::PeerExpired { .. } => SESSION_REFRESH_PEER_EXPIRED_EVENT,
        };
        EventType::MediaEvent(tag.to_string())
    }

    fn validate(&self, session: &SessionState) -> crate::errors::Result<()> {
        use crate::session_store::state::SessionRefreshPhase;

        let valid = match self {
            Self::UpdateDue { timer_generation } => {
                *timer_generation == session.session_refresh_timer_generation
                    && session.session_refresh_local_refresher
                    && session.session_refresh_interval_secs.is_some()
                    && session.session_refresh_phase == SessionRefreshPhase::Idle
            }
            Self::ReinviteDue { timer_generation } => {
                *timer_generation == session.session_refresh_timer_generation
                    && session.session_refresh_local_refresher
                    && session.session_refresh_interval_secs.is_some()
                    && session.session_refresh_phase == SessionRefreshPhase::Idle
            }
            Self::UpdateSucceeded | Self::UpdateFailed => {
                session.session_refresh_phase == SessionRefreshPhase::UpdateInFlight
            }
            Self::ReinviteSucceeded | Self::ReinviteFailed => {
                session.session_refresh_phase == SessionRefreshPhase::ReinviteInFlight
            }
            Self::PeerExpired { timer_generation } => {
                *timer_generation == session.session_refresh_timer_generation
                    && !session.session_refresh_local_refresher
                    && session.session_refresh_interval_secs.is_some()
                    && session.session_refresh_phase == SessionRefreshPhase::Idle
            }
        };
        valid.then_some(()).ok_or_else(|| {
            crate::errors::SessionError::InvalidTransition(
                "stale or mismatched RFC 4028 exact-session input".to_string(),
            )
        })
    }

    fn apply(self, session: &mut SessionState) {
        use crate::session_store::state::SessionRefreshPhase;

        match self {
            Self::UpdateSucceeded | Self::UpdateFailed => {
                session.session_refresh_phase = SessionRefreshPhase::Idle;
            }
            Self::ReinviteSucceeded | Self::ReinviteFailed => {
                session.session_refresh_phase = SessionRefreshPhase::Idle;
            }
            Self::UpdateDue { .. } | Self::ReinviteDue { .. } | Self::PeerExpired { .. } => {}
        }
    }
}

#[derive(Default)]
struct EventStateInput {
    remote_sdp: Option<String>,
    /// Whether this event authoritatively supplied the remote SDP field.
    /// `None` with this bit set means the response/request carried no SDP;
    /// without the bit, the stable remote description is left untouched.
    remote_sdp_supplied: bool,
    /// A final response after committed 183 early media confirms the existing
    /// offer/answer exchange; it is not a second answer. Preserve the stable
    /// provisional description instead of replacing it with an optional copy
    /// from the final response.
    preserve_committed_provisional_sdp: bool,
    local_sdp: Option<String>,
    sdp_negotiated: Option<bool>,
    response: Option<ResponseStateInput>,
    outbound_session: Option<OutboundSessionStateInput>,
    registration_start: Option<RegistrationStartInput>,
    transfer_request: Option<TransferRequestStateInput>,
    refer_notify: Option<ReferNotifyInput>,
    auth_required: Option<AuthRequiredStateInput>,
    session_refresh: Option<SessionRefreshStateInput>,
    confirmed_negotiation_failure: bool,
    inbound_response: Option<InboundResponseStateInput>,
    invite_2xx_ack: Option<Invite2xxAckStateInput>,
}

/// Exact successful INVITE response retained until the ordered `SendACK`
/// action has completed SDP work and written the ACK.
pub(crate) struct Invite2xxAckStateInput {
    pub(crate) transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
    pub(crate) response: rvoip_sip_core::Response,
}

impl Invite2xxAckStateInput {
    pub(crate) fn new(
        transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
        response: rvoip_sip_core::Response,
    ) -> Self {
        Self {
            transaction_id,
            response,
        }
    }

    fn validate_event(&self, event: &EventType) -> crate::errors::Result<()> {
        if !matches!(event, EventType::Dialog200OK)
            || self.transaction_id.is_server()
            || self.transaction_id.method() != &rvoip_sip_core::Method::Invite
            || !self.response.status().is_success()
            || rvoip_sip_dialog::transaction::TransactionKey::from_response(&self.response).as_ref()
                != Some(&self.transaction_id)
        {
            return Err(crate::errors::SessionError::InvalidTransition(
                "deferred INVITE 2xx ACK requires the exact successful response".to_string(),
            ));
        }
        Ok(())
    }
}

/// Event-local authority for a response to one inbound INVITE or UPDATE.
///
/// In-dialog keys are derived from the preserved wire request. The initial
/// INVITE key is captured directly from causal ingress and is also retained in
/// `SessionState` for a later application Accept/Reject event. In either case,
/// a later request on the same dialog cannot overwrite the transaction chosen
/// by an already-admitted event.
pub(crate) struct InboundResponseStateInput {
    transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
    method: rvoip_sip_core::Method,
    terminal: bool,
}

impl InboundResponseStateInput {
    pub(crate) fn from_initial_invite_transaction(
        transaction_id: rvoip_sip_dialog::transaction::TransactionKey,
    ) -> crate::errors::Result<Self> {
        if !transaction_id.is_server() || transaction_id.method() != &rvoip_sip_core::Method::Invite
        {
            return Err(crate::errors::SessionError::InvalidTransition(
                "inbound initial INVITE has no exact server INVITE transaction".to_string(),
            ));
        }
        Ok(Self {
            transaction_id,
            method: rvoip_sip_core::Method::Invite,
            terminal: false,
        })
    }

    pub(crate) fn from_request(
        claimed_method: &str,
        request: &rvoip_sip_core::Request,
    ) -> crate::errors::Result<Self> {
        let claimed_method = if claimed_method.eq_ignore_ascii_case("INVITE") {
            rvoip_sip_core::Method::Invite
        } else if claimed_method.eq_ignore_ascii_case("UPDATE") {
            rvoip_sip_core::Method::Update
        } else {
            return Err(crate::errors::SessionError::InvalidTransition(
                "inbound response event claimed an unsupported SIP method".to_string(),
            ));
        };
        if request.method() != claimed_method {
            return Err(crate::errors::SessionError::InvalidTransition(
                "inbound response event method does not match the preserved request".to_string(),
            ));
        }
        let transaction_id = rvoip_sip_dialog::transaction::TransactionKey::from_request(request)
            .filter(|transaction| {
                transaction.is_server() && transaction.method() == &claimed_method
            })
            .ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(
                    "inbound request has no exact server response transaction".to_string(),
                )
            })?;
        Ok(Self {
            transaction_id,
            method: claimed_method,
            terminal: false,
        })
    }

    fn validate_event(&self, event: &EventType) -> crate::errors::Result<()> {
        let matches = matches!(
            (event, &self.method),
            (
                EventType::ReinviteReceived { .. },
                rvoip_sip_core::Method::Invite
            ) | (
                EventType::UpdateReceived { .. },
                rvoip_sip_core::Method::Update
            ) | (
                EventType::IncomingCall { .. } | EventType::IncomingCallAutoAccept { .. },
                rvoip_sip_core::Method::Invite
            )
        );
        matches.then_some(()).ok_or_else(|| {
            crate::errors::SessionError::InvalidTransition(
                "inbound response authority does not match the state-machine event".to_string(),
            )
        })
    }

    pub(crate) fn transaction_id(
        &self,
    ) -> crate::errors::Result<&rvoip_sip_dialog::transaction::TransactionKey> {
        (!self.terminal)
            .then_some(&self.transaction_id)
            .ok_or_else(|| {
                crate::errors::SessionError::InvalidTransition(
                    "inbound response authority was already consumed".to_string(),
                )
            })
    }

    pub(crate) fn consume_terminal(&mut self) {
        self.terminal = true;
    }
}

/// Session metadata needed by the lifecycle actions that precede the first
/// outbound INVITE. It is derived from the immutable builder snapshot and
/// applied only after the exact-session lane has been acquired.
pub(crate) struct OutboundSessionStateInput {
    credentials: Option<crate::types::Credentials>,
    auth: Option<crate::auth::SipClientAuth>,
    pai_uri: Option<String>,
    transferor_session_id: Option<SessionId>,
    transferor_lifecycle_handle: Option<SessionRegistryHandle>,
    extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
}

impl OutboundSessionStateInput {
    pub(crate) fn new(
        credentials: Option<crate::types::Credentials>,
        auth: Option<crate::auth::SipClientAuth>,
        pai_uri: Option<String>,
        transferor_session_id: Option<SessionId>,
        transferor_lifecycle_handle: Option<SessionRegistryHandle>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    ) -> Self {
        Self {
            credentials,
            auth,
            pai_uri,
            transferor_session_id,
            transferor_lifecycle_handle,
            extra_headers,
        }
    }

    pub(crate) fn from_snapshot(
        snapshot: &crate::api::send::outbound_call::OutboundCallOptionsSnapshot,
        pai_uri: Option<String>,
        transferor_lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        Self {
            credentials: snapshot.credentials.clone(),
            auth: snapshot.auth.clone(),
            pai_uri,
            transferor_session_id: snapshot.transfer_leg.clone(),
            transferor_lifecycle_handle,
            extra_headers: snapshot.extra_headers.clone(),
        }
    }
}

/// Initial REGISTER data applied immediately before `StartRegistration` under
/// the exact-session lane.
pub(crate) struct RegistrationStartInput {
    credentials: crate::types::Credentials,
    registrar_uri: String,
    contact_uri: String,
    expires: u32,
    pending_options: Option<Arc<rvoip_sip_dialog::api::unified::RegisterRequestOptions>>,
}

impl RegistrationStartInput {
    pub(crate) fn new(
        credentials: crate::types::Credentials,
        registrar_uri: String,
        contact_uri: String,
        expires: u32,
        pending_options: Option<Arc<rvoip_sip_dialog::api::unified::RegisterRequestOptions>>,
    ) -> Self {
        Self {
            credentials,
            registrar_uri,
            contact_uri,
            expires,
            pending_options,
        }
    }
}

/// Inbound REFER fields that must be committed before the application can
/// accept or reject the request. The YAML `TransferRequested` transition is
/// deliberately not dispatched by this staging input; acceptance remains an
/// explicit application/default decision.
pub(crate) struct TransferRequestStateInput {
    refer_to: String,
    transaction_id: String,
    referred_by: Option<String>,
    replaces: Option<String>,
}

impl TransferRequestStateInput {
    pub(crate) fn new(
        refer_to: String,
        transaction_id: String,
        referred_by: Option<String>,
        replaces: Option<String>,
    ) -> Self {
        Self {
            refer_to,
            transaction_id,
            referred_by,
            replaces,
        }
    }
}

/// Parsed RFC 3515 sipfrag status applied only after the exact session lane
/// has been acquired. This remains crate-private because the public surface
/// continues to expose `Event::ReferNotify` and the derived transfer events.
pub(crate) struct ReferNotifyInput {
    status_code: u16,
    reason: String,
}

impl ReferNotifyInput {
    pub(crate) fn new(status_code: u16, reason: String) -> Self {
        Self {
            status_code,
            reason,
        }
    }

    fn outcome(&self, session: &SessionState) -> ReferNotifyOutcome {
        match self.status_code {
            100..=199 => ReferNotifyOutcome::Progress,
            200..=299 => ReferNotifyOutcome::Completed {
                transfer_target: session.transfer_target.clone().unwrap_or_default(),
                progress_evidence: session
                    .transfer_target_progress_seen
                    .then(|| session.transfer_target_last_progress.clone())
                    .flatten(),
            },
            300..=699 => ReferNotifyOutcome::Failed,
            _ => ReferNotifyOutcome::Ignored,
        }
    }

    fn apply(self, session: &mut SessionState) {
        match self.status_code {
            100..=199 => {
                session.transfer_target_progress_seen = true;
                session.transfer_target_last_progress = Some((self.status_code, self.reason));
            }
            200..=299 => {
                session.transfer_state =
                    crate::session_store::state::TransferState::TransferCompleted;
            }
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReferNotifyOutcome {
    Progress,
    Completed {
        transfer_target: String,
        progress_evidence: Option<(u16, String)>,
    },
    Failed,
    Ignored,
}

/// Exact wire correlation applied with an `AuthRequired` event after the
/// session's generation-qualified execution lane has been acquired.
pub(crate) struct AuthRequiredStateInput {
    outbound_transport: Option<crate::auth::SipTransportSecurityContext>,
    transaction_id: Option<String>,
    request_uri: Option<String>,
}

impl AuthRequiredStateInput {
    pub(crate) fn new(
        outbound_transport: Option<rvoip_infra_common::events::cross_crate::SipTransportContext>,
        transaction_id: Option<String>,
        request_uri: Option<String>,
    ) -> Self {
        Self {
            outbound_transport: outbound_transport
                .as_ref()
                .map(crate::auth::SipTransportSecurityContext::from_transport_context),
            transaction_id,
            request_uri,
        }
    }
}

impl EventStateInput {
    fn apply(self, session: &mut SessionState) {
        let committed_provisional_answer = self.preserve_committed_provisional_sdp
            && session.call_state == CallState::EarlyMedia
            && session.sdp_negotiated
            && session.media_session_ready
            && session.pending_offer_answer.is_none();
        if self.remote_sdp_supplied && !committed_provisional_answer {
            session.remote_sdp = self.remote_sdp;
        }
        if let Some(local_sdp) = self.local_sdp {
            session.local_sdp = Some(local_sdp);
        }
        if let Some(sdp_negotiated) = self.sdp_negotiated {
            session.sdp_negotiated = sdp_negotiated;
        }
        if let Some(response) = self.response {
            response.apply(session);
        }
        if let Some(input) = self.outbound_session {
            session.credentials = input.credentials;
            session.auth = input.auth;
            session.pai_uri = input.pai_uri;
            session.is_transfer_call = input.transferor_session_id.is_some();
            session.transferor_session_id = input.transferor_session_id;
            session.transferor_lifecycle_handle = input.transferor_lifecycle_handle;
            session.extra_headers = input.extra_headers;
        }
        if let Some(input) = self.registration_start {
            session.credentials = Some(input.credentials);
            session.registrar_uri = Some(input.registrar_uri);
            session.registration_contact = Some(input.contact_uri);
            session.registration_expires = Some(input.expires);
            session.pending_register_options = input.pending_options;
        }
        if let Some(input) = self.transfer_request {
            session.transfer_target = Some(input.refer_to);
            session.transfer_notify_dialog = session.dialog_id;
            session.refer_transaction_id = Some(input.transaction_id);
            session.referred_by = input.referred_by;
            session.replaces_header = input.replaces;
        }
        if let Some(input) = self.auth_required {
            session.pending_auth_transport = input.outbound_transport;
            session.pending_auth_transaction_id = input.transaction_id;
            session.pending_auth_request_uri = input.request_uri;
        }
        if let Some(input) = self.session_refresh {
            input.apply(session);
        }
    }
}

/// The state machine executor that processes events through the state table
pub struct StateMachine {
    /// The master state table (static rules)
    table: Arc<crate::state_table::MasterStateTable>,

    /// Session state storage
    pub(crate) store: Arc<SessionStore>,

    /// Adapter to dialog-core
    dialog_adapter: Arc<DialogAdapter>,

    /// Adapter to media-core
    media_adapter: Arc<MediaAdapter>,

    /// Event publisher (optional - for legacy compatibility)
    event_tx: Option<tokio::sync::mpsc::Sender<SessionEvent>>,
    /// Optional production bridge for state-machine lifecycle observations
    /// that must retain the exact session generation. Public constructors keep
    /// the legacy observational channel contract unchanged.
    exact_api_event_publisher: OnceLock<crate::api::lifecycle::SessionEventPublisher>,
    /// Whether the default inbound INVITE path sends automatic 180 Ringing.
    auto_180_ringing: bool,
    // SimplePeer events now handled by SessionCrossCrateEventHandler
}

/// SIP_API_DESIGN_2 §7.3 — typed wrapper that carries one of the twelve
/// outbound option snapshots from a builder's `.send()` into
/// `StateMachine::stage_outbound_options`. The wrapper matches the
/// shape of the `pending_<method>_options` slot on `SessionState`; the
/// helper unwraps it to write the exact slot and reports
/// `SessionError::Conflict { method }` if the slot is already
/// occupied. Carrying the typed Arc (not a `Box<dyn Any>`) keeps the
/// builder → stash path monomorphic and statically checked.
#[derive(Debug, Clone)]
pub enum PendingOptionsSlot {
    Invite(Arc<crate::api::send::outbound_call::OutboundCallOptionsSnapshot>),
    ReInvite(Arc<rvoip_sip_dialog::api::unified::ReInviteRequestOptions>),
    Register(Arc<rvoip_sip_dialog::api::unified::RegisterRequestOptions>),
    Refer(Arc<rvoip_sip_dialog::api::unified::ReferRequestOptions>),
    Bye(Arc<rvoip_sip_dialog::api::unified::ByeRequestOptions>),
    Cancel(Arc<rvoip_sip_dialog::api::unified::CancelRequestOptions>),
    Notify(Arc<rvoip_sip_dialog::api::unified::NotifyRequestOptions>),
    Subscribe(Arc<rvoip_sip_dialog::api::unified::SubscribeRequestOptions>),
    Info(Arc<rvoip_sip_dialog::api::unified::InfoRequestOptions>),
    Update(Arc<rvoip_sip_dialog::api::unified::UpdateRequestOptions>),
    Message(Arc<rvoip_sip_dialog::api::unified::MessageRequestOptions>),
    Options(Arc<rvoip_sip_dialog::api::unified::OptionsRequestOptions>),
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum PendingOptionsSlotKind {
    Invite,
    ReInvite,
    Register,
    Refer,
    Bye,
    Cancel,
    Notify,
    Subscribe,
    Info,
    Update,
    Message,
    Options,
}

impl PendingOptionsSlot {
    /// Returns the SIP method this slot represents — used by the
    /// conflict-guard error path.
    pub fn method(&self) -> rvoip_sip_core::Method {
        use rvoip_sip_core::Method;
        match self {
            Self::Invite(_) | Self::ReInvite(_) => Method::Invite,
            Self::Register(_) => Method::Register,
            Self::Refer(_) => Method::Refer,
            Self::Bye(_) => Method::Bye,
            Self::Cancel(_) => Method::Cancel,
            Self::Notify(_) => Method::Notify,
            Self::Subscribe(_) => Method::Subscribe,
            Self::Info(_) => Method::Info,
            Self::Update(_) => Method::Update,
            Self::Message(_) => Method::Message,
            Self::Options(_) => Method::Options,
        }
    }

    pub(crate) fn kind(&self) -> PendingOptionsSlotKind {
        match self {
            Self::Invite(_) => PendingOptionsSlotKind::Invite,
            Self::ReInvite(_) => PendingOptionsSlotKind::ReInvite,
            Self::Register(_) => PendingOptionsSlotKind::Register,
            Self::Refer(_) => PendingOptionsSlotKind::Refer,
            Self::Bye(_) => PendingOptionsSlotKind::Bye,
            Self::Cancel(_) => PendingOptionsSlotKind::Cancel,
            Self::Notify(_) => PendingOptionsSlotKind::Notify,
            Self::Subscribe(_) => PendingOptionsSlotKind::Subscribe,
            Self::Info(_) => PendingOptionsSlotKind::Info,
            Self::Update(_) => PendingOptionsSlotKind::Update,
            Self::Message(_) => PendingOptionsSlotKind::Message,
            Self::Options(_) => PendingOptionsSlotKind::Options,
        }
    }

    pub(crate) fn is_exact_staged_on(&self, session: &SessionState) -> bool {
        match self {
            Self::Invite(options) => session
                .pending_invite_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::ReInvite(options) => session
                .pending_reinvite_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Register(options) => session
                .pending_register_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Refer(options) => session
                .pending_refer_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Bye(options) => session
                .pending_bye_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Cancel(options) => session
                .pending_cancel_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Notify(options) => session
                .pending_notify_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Subscribe(options) => session
                .pending_subscribe_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Info(options) => session
                .pending_info_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Update(options) => session
                .pending_update_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Message(options) => session
                .pending_message_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
            Self::Options(options) => session
                .pending_options_options
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, options)),
        }
    }

    pub(crate) fn clear_if_exact(&self, session: &mut SessionState) -> bool {
        if !self.is_exact_staged_on(session) {
            return false;
        }
        match self {
            Self::Invite(_) => session.pending_invite_options = None,
            Self::ReInvite(_) => session.pending_reinvite_options = None,
            Self::Register(_) => session.pending_register_options = None,
            Self::Refer(_) => session.pending_refer_options = None,
            Self::Bye(_) => session.pending_bye_options = None,
            Self::Cancel(_) => session.pending_cancel_options = None,
            Self::Notify(_) => session.pending_notify_options = None,
            Self::Subscribe(_) => session.pending_subscribe_options = None,
            Self::Info(_) => session.pending_info_options = None,
            Self::Update(_) => session.pending_update_options = None,
            Self::Message(_) => session.pending_message_options = None,
            Self::Options(_) => session.pending_options_options = None,
        }
        true
    }

    fn stage_if_vacant(self, session: &mut SessionState) -> crate::errors::Result<()> {
        let method = self.method();
        let occupied = match &self {
            Self::Invite(_) => session.pending_invite_options.is_some(),
            Self::ReInvite(_) => session.pending_reinvite_options.is_some(),
            Self::Register(_) => session.pending_register_options.is_some(),
            Self::Refer(_) => session.pending_refer_options.is_some(),
            Self::Bye(_) => session.pending_bye_options.is_some(),
            Self::Cancel(_) => session.pending_cancel_options.is_some(),
            Self::Notify(_) => session.pending_notify_options.is_some(),
            Self::Subscribe(_) => session.pending_subscribe_options.is_some(),
            Self::Info(_) => session.pending_info_options.is_some(),
            Self::Update(_) => session.pending_update_options.is_some(),
            Self::Message(_) => session.pending_message_options.is_some(),
            Self::Options(_) => session.pending_options_options.is_some(),
        };
        if occupied {
            return Err(crate::errors::SessionError::Conflict { method });
        }

        match self {
            Self::Invite(options) => session.pending_invite_options = Some(options),
            Self::ReInvite(options) => session.pending_reinvite_options = Some(options),
            Self::Register(options) => session.pending_register_options = Some(options),
            Self::Refer(options) => session.pending_refer_options = Some(options),
            Self::Bye(options) => session.pending_bye_options = Some(options),
            Self::Cancel(options) => session.pending_cancel_options = Some(options),
            Self::Notify(options) => session.pending_notify_options = Some(options),
            Self::Subscribe(options) => session.pending_subscribe_options = Some(options),
            Self::Info(options) => session.pending_info_options = Some(options),
            Self::Update(options) => session.pending_update_options = Some(options),
            Self::Message(options) => session.pending_message_options = Some(options),
            Self::Options(options) => session.pending_options_options = Some(options),
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum StageDispatchClaimStatus {
    Unclaimed,
    Claimed,
    Cancelled,
}

struct StageDispatchClaimState {
    status: StageDispatchClaimStatus,
    slot: Option<PendingOptionsSlot>,
}

/// Coordinates cancellation with the exact transfer of a builder's staged
/// request options into the authoritative outbound-request tracker.
///
/// The state mutex is deliberately acquired while the executor owns the exact
/// session's complete-event lane and mutates its one working `SessionState`.
/// This makes cancellation-before-claim and claim-before-cancellation mutually
/// exclusive: before claim the dispatch task is aborted and no request reaches
/// the wire; after claim the task is detached on caller cancellation so it can
/// finish transaction activation and preserve the exact completion owner.
pub(crate) struct StageDispatchClaim {
    state: Mutex<StageDispatchClaimState>,
    method: rvoip_sip_core::Method,
    kind: PendingOptionsSlotKind,
}

impl StageDispatchClaim {
    pub(crate) fn new(slot: PendingOptionsSlot) -> Self {
        let method = slot.method();
        let kind = slot.kind();
        Self {
            state: Mutex::new(StageDispatchClaimState {
                status: StageDispatchClaimStatus::Unclaimed,
                slot: Some(slot),
            }),
            method,
            kind,
        }
    }

    /// Construct a cancellation fence before lane-owned state is available.
    /// Registration refresh builders use this form because their immutable
    /// REGISTER snapshot (including the exact next CSeq) must be derived only
    /// after the generation-qualified session lane has been acquired.
    pub(crate) fn new_deferred(
        method: rvoip_sip_core::Method,
        kind: PendingOptionsSlotKind,
    ) -> Self {
        Self {
            state: Mutex::new(StageDispatchClaimState {
                status: StageDispatchClaimStatus::Unclaimed,
                slot: None,
            }),
            method,
            kind,
        }
    }

    pub(crate) fn method(&self) -> rvoip_sip_core::Method {
        self.method.clone()
    }

    pub(crate) fn kind(&self) -> PendingOptionsSlotKind {
        self.kind
    }

    /// Install the lane-derived immutable snapshot into a deferred claim.
    /// Cancellation and installation share the same mutex, so a public future
    /// cannot race an exact snapshot into the session after it has elected to
    /// abort before claim.
    fn install_deferred_slot(&self, slot: PendingOptionsSlot) -> crate::errors::Result<()> {
        if slot.method() != self.method() || slot.kind() != self.kind {
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "deferred outbound {} dispatch received a mismatched options slot",
                self.method
            )));
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.status {
            StageDispatchClaimStatus::Cancelled => {
                Err(crate::errors::SessionError::InvalidTransition(format!(
                    "outbound {} dispatch was cancelled before deriving staged options",
                    self.method
                )))
            }
            StageDispatchClaimStatus::Claimed => {
                Err(crate::errors::SessionError::InvalidTransition(format!(
                    "outbound {} dispatch already claimed staged options",
                    self.method
                )))
            }
            StageDispatchClaimStatus::Unclaimed if state.slot.is_some() => {
                Err(crate::errors::SessionError::InvalidTransition(format!(
                    "outbound {} dispatch already installed staged options",
                    self.method
                )))
            }
            StageDispatchClaimStatus::Unclaimed => {
                state.slot = Some(slot);
                Ok(())
            }
        }
    }

    /// Claim and remove the exact staged Arc from the current session
    /// revision. Callers must invoke this while owning the exact session's
    /// complete-event lane so the slot and cancellation state change
    /// atomically with respect to every other signaling transition.
    pub(crate) fn claim_exact(
        &self,
        session: &mut SessionState,
    ) -> crate::errors::Result<PendingOptionsSlot> {
        self.claim_exact_with_retention(session, false)
    }

    /// Claim the exact staged Arc without clearing it from the lane-owned
    /// session revision. Retry-owned requests use this transfer: caller
    /// cancellation must detach after the first wire owner starts, while the
    /// immutable snapshot remains pointer-exact for authentication or
    /// protocol retries until its final-response owner clears it.
    pub(crate) fn claim_retained_exact(
        &self,
        session: &mut SessionState,
    ) -> crate::errors::Result<PendingOptionsSlot> {
        self.claim_exact_with_retention(session, true)
    }

    fn claim_exact_with_retention(
        &self,
        session: &mut SessionState,
        retain: bool,
    ) -> crate::errors::Result<PendingOptionsSlot> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.status {
            StageDispatchClaimStatus::Cancelled => {
                return Err(crate::errors::SessionError::InvalidTransition(format!(
                    "outbound {} dispatch was cancelled before claiming staged options",
                    self.method
                )));
            }
            StageDispatchClaimStatus::Claimed => {
                return Err(crate::errors::SessionError::InvalidTransition(format!(
                    "outbound {} dispatch already claimed staged options",
                    self.method
                )));
            }
            StageDispatchClaimStatus::Unclaimed => {}
        }
        let slot = state.slot.as_ref().cloned().ok_or_else(|| {
            crate::errors::SessionError::InvalidTransition(format!(
                "outbound {} dispatch has no lane-derived staged options",
                self.method
            ))
        })?;
        if !slot.is_exact_staged_on(session) {
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "outbound {} dispatch no longer owns its exact staged options",
                self.method
            )));
        }
        if !retain {
            let cleared = slot.clear_if_exact(session);
            debug_assert!(cleared, "exact stage changed while its lane was held");
        }
        state.status = StageDispatchClaimStatus::Claimed;
        Ok(slot)
    }

    /// Return true when the dispatch task must be aborted. Once the exact
    /// stage has been claimed, dropping the caller instead detaches the task.
    pub(crate) fn cancel_before_claim(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.status {
            StageDispatchClaimStatus::Unclaimed => {
                state.status = StageDispatchClaimStatus::Cancelled;
                true
            }
            StageDispatchClaimStatus::Cancelled => true,
            StageDispatchClaimStatus::Claimed => false,
        }
    }

    fn is_claimed(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .status
            == StageDispatchClaimStatus::Claimed
    }
}

/// Cancellation-safe ownership of one exact builder staging Arc.
pub(crate) struct PendingOptionsStageGuard {
    store: Arc<SessionStore>,
    handle: SessionRegistryHandle,
    slot: PendingOptionsSlot,
    dispatch_claim: Arc<StageDispatchClaim>,
    armed: bool,
}

impl PendingOptionsStageGuard {
    #[cfg(test)]
    fn new(
        store: Arc<SessionStore>,
        handle: SessionRegistryHandle,
        slot: PendingOptionsSlot,
    ) -> Self {
        let dispatch_claim = Arc::new(StageDispatchClaim::new(slot.clone()));
        Self::new_with_claim(store, handle, slot, dispatch_claim)
    }

    fn new_with_claim(
        store: Arc<SessionStore>,
        handle: SessionRegistryHandle,
        slot: PendingOptionsSlot,
        dispatch_claim: Arc<StageDispatchClaim>,
    ) -> Self {
        Self {
            store,
            handle,
            slot,
            dispatch_claim,
            armed: true,
        }
    }

    pub(crate) async fn confirm_claimed(mut self) -> crate::errors::Result<()> {
        if !self.dispatch_claim.is_claimed() {
            self.dispatch_claim.cancel_before_claim();
            let _ = self
                .store
                .update_session_exact_now(&self.handle, |session| {
                    self.slot.clear_if_exact(session)
                });
            self.armed = false;
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "outbound {} dispatch did not claim its exact staged options",
                self.slot.method()
            )));
        }
        self.armed = false;
        Ok(())
    }
}

impl Drop for PendingOptionsStageGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if !self.dispatch_claim.cancel_before_claim() {
            return;
        }
        let _ = self
            .store
            .update_session_exact_now(&self.handle, |session| self.slot.clear_if_exact(session));
    }
}

fn state_machine_stage_for_event(event: &EventType) -> CleanupStage {
    match event {
        EventType::IncomingCall { .. } | EventType::IncomingCallAutoAccept { .. } => {
            CleanupStage::StateMachineIncomingCall
        }
        EventType::AcceptCall => CleanupStage::StateMachineAcceptCall,
        EventType::DialogBYE | EventType::DialogTerminated | EventType::DialogCANCEL => {
            CleanupStage::StateMachineTerminalEvent
        }
        _ => CleanupStage::StateMachineOtherEvent,
    }
}

fn state_machine_event_name(event: &EventType) -> &'static str {
    match event {
        EventType::IncomingCall { .. } => "IncomingCall",
        EventType::IncomingCallAutoAccept { .. } => "IncomingCallAutoAccept",
        EventType::AcceptCall => "AcceptCall",
        EventType::DialogBYE => "DialogBYE",
        EventType::DialogTerminated => "DialogTerminated",
        EventType::InternalCheckReady => "InternalCheckReady",
        EventType::DialogCreated { .. } => "DialogCreated",
        EventType::Dialog200OK => "Dialog200OK",
        EventType::DialogACK => "DialogACK",
        EventType::DialogCANCEL => "DialogCANCEL",
        EventType::ReceiveNOTIFY => "ReceiveNOTIFY",
        _ => "Other",
    }
}

fn apply_incoming_call_event_state(session: &mut SessionState, from: &str, sdp: Option<&str>) {
    session.remote_uri = Some(from.to_string());
    if let Some(sdp) = sdp {
        session.remote_sdp = Some(sdp.to_string());
    }
}

fn is_missing_credentials_for_auth_error(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    matches!(
        error.downcast_ref::<crate::errors::SessionError>(),
        Some(crate::errors::SessionError::MissingCredentialsForInviteAuth)
            | Some(crate::errors::SessionError::MissingCredentialsForRequestAuth { .. })
    )
}

fn action_diagnostic_class(action: &Action) -> &'static str {
    match action {
        Action::SendINVITEWithAuth => "invite-auth",
        Action::SendRequestWithAuth => "request-auth",
        Action::SendREGISTERWithAuth => "register-auth",
        Action::StoreAuthChallenge => "store-auth-challenge",
        Action::CleanupDialog => "cleanup-dialog",
        Action::CleanupMedia => "cleanup-media",
        Action::CreateMediaSession => "create-media-session",
        Action::GenerateLocalSDP => "generate-local-sdp",
        Action::RetryWithContact => "retry-with-contact",
        _ => "state-machine-action",
    }
}

fn action_authors_final_response(action: &Action, session: &SessionState) -> bool {
    match action {
        Action::SendSIPResponse(code, _) => {
            (200..=699).contains(&session.pending_response_status_override.unwrap_or(*code))
        }
        Action::SendRejectResponse | Action::SendRedirectResponse | Action::SendReferAccepted => {
            true
        }
        _ => false,
    }
}

fn action_error_diagnostic_class(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> &'static str {
    if is_missing_credentials_for_auth_error(error) {
        "missing-credentials"
    } else if error
        .downcast_ref::<crate::errors::SessionError>()
        .is_some()
    {
        "session-error"
    } else {
        "action-error"
    }
}

fn is_local_teardown_dispatch_only_transition(transition: &Transition) -> bool {
    transition.publish_events.is_empty()
        && transition.actions.iter().any(|action| {
            matches!(
                action,
                Action::SendBYE | Action::SendBYEWithOptions | Action::SendCANCELWithOptions
            )
        })
}

fn is_refer_dispatch_only_transition(transition: &Transition) -> bool {
    transition.next_state.is_none()
        && transition.condition_updates.dialog_established.is_none()
        && transition.condition_updates.media_session_ready.is_none()
        && transition.condition_updates.sdp_negotiated.is_none()
        && transition.publish_events.is_empty()
        && matches!(
            transition.actions.as_slice(),
            [Action::SendREFERWithOptions]
        )
}

fn is_exact_retirement_safe_dispatch_only_transition(transition: &Transition) -> bool {
    is_local_teardown_dispatch_only_transition(transition)
        || is_refer_dispatch_only_transition(transition)
}

fn completed_transition_result(
    old_state: CallState,
    transition: &Transition,
    actions_executed: Vec<Action>,
) -> ProcessEventResult {
    ProcessEventResult {
        old_state,
        next_state: transition.next_state,
        transition: Some(transition.clone()),
        actions_executed,
        events_published: transition.publish_events.clone(),
    }
}

fn commit_lane_state(
    store: &SessionStore,
    session: SessionState,
) -> Result<Arc<SessionStateSnapshot>, Box<dyn std::error::Error + Send + Sync>> {
    store.update_state_machine_session_and_snapshot(session)
}

#[cfg(test)]
mod action_execution_barrier {
    use std::collections::HashMap;
    use std::sync::{Arc, LazyLock, Mutex};

    use tokio::sync::Notify;

    use crate::state_table::SessionId;

    static BARRIERS: LazyLock<Mutex<HashMap<String, Arc<Barrier>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    pub(super) struct Barrier {
        entered: Notify,
        release: Notify,
    }

    impl Barrier {
        pub(super) async fn wait_until_entered(&self) {
            self.entered.notified().await;
        }

        pub(super) fn release(&self) {
            self.release.notify_one();
        }
    }

    pub(super) fn install(session_id: &SessionId) -> Arc<Barrier> {
        let barrier = Arc::new(Barrier {
            entered: Notify::new(),
            release: Notify::new(),
        });
        BARRIERS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id.0.clone(), Arc::clone(&barrier));
        barrier
    }

    pub(super) async fn pause_once(session_id: &SessionId) {
        let barrier = BARRIERS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&session_id.0);
        if let Some(barrier) = barrier {
            barrier.entered.notify_one();
            barrier.release.notified().await;
        }
    }
}

/// Append diagnostic history for an event rejected before action admission
/// without publishing any of that event's speculative typed or payload state.
///
/// The exact-session lane remains held by the caller, so this canonical
/// pre-event image cannot overwrite a competing retained-auth writer.
fn commit_rejected_transition_history(
    store: &SessionStore,
    pre_event_snapshot: &SessionStateSnapshot,
    record: crate::session_store::TransitionRecord,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !pre_event_snapshot.state().history_recording_enabled() {
        return Ok(());
    }

    let mut canonical = pre_event_snapshot.state().clone();
    canonical.record_transition(record);
    commit_lane_state(store, canonical)?;
    Ok(())
}

async fn clear_tracked_request_auth_state_in_exact_lane(
    store: &SessionStore,
    handle: &SessionRegistryHandle,
    transaction_id: &str,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let lane = store.state_machine_lane_exact(handle).ok_or_else(|| {
        Box::new(crate::errors::SessionError::SessionNotFound(
            handle.session_id().to_string(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;
    let _state_machine_lane = lane.lock_owned().await;
    store.get_session_snapshot_exact(handle).map_err(|_| {
        Box::new(crate::errors::SessionError::SessionNotFound(
            handle.session_id().to_string(),
        )) as Box<dyn std::error::Error + Send + Sync>
    })?;

    let mut session = store.get_session_exact(handle).await?;
    if !session.clear_tracked_auth_if_transaction(transaction_id) {
        return Ok(false);
    }

    commit_lane_state(store, session)?;
    Ok(true)
}

fn admit_committed_transfer_observation(
    event: crate::api::events::Event,
    publish: impl FnOnce(crate::api::events::Event),
) {
    publish(event);
}

/// Events that flow through the system
#[derive(Clone)]
pub enum SessionEvent {
    StateChanged {
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
    },
    MediaFlowEstablished {
        session_id: SessionId,
        local_addr: String,
        remote_addr: String,
        direction: crate::state_table::MediaFlowDirection,
    },
    CallEstablished {
        session_id: SessionId,
    },
    CallTerminated {
        session_id: SessionId,
    },
    CallCancelled {
        session_id: SessionId,
    },
    CallOnHold {
        session_id: SessionId,
    },
    CallResumed {
        session_id: SessionId,
    },
    Custom {
        session_id: SessionId,
        event: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObservationPublishOutcome {
    Published,
    Saturated,
    Closed,
}

fn publish_observational_event(
    sender: &tokio::sync::mpsc::Sender<SessionEvent>,
    event: SessionEvent,
) -> ObservationPublishOutcome {
    match sender.try_send(event) {
        Ok(()) => ObservationPublishOutcome::Published,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            ObservationPublishOutcome::Saturated
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => ObservationPublishOutcome::Closed,
    }
}

impl std::fmt::Debug for SessionEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StateChanged {
                old_state,
                new_state,
                ..
            } => formatter
                .debug_struct("StateChanged")
                .field("old_state", old_state)
                .field("new_state", new_state)
                .finish(),
            Self::MediaFlowEstablished {
                local_addr,
                remote_addr,
                direction,
                ..
            } => formatter
                .debug_struct("MediaFlowEstablished")
                .field("local_addr_bytes", &local_addr.len())
                .field("remote_addr_bytes", &remote_addr.len())
                .field("direction", direction)
                .finish(),
            Self::CallEstablished { .. } => formatter.write_str("CallEstablished"),
            Self::CallTerminated { .. } => formatter.write_str("CallTerminated"),
            Self::CallCancelled { .. } => formatter.write_str("CallCancelled"),
            Self::CallOnHold { .. } => formatter.write_str("CallOnHold"),
            Self::CallResumed { .. } => formatter.write_str("CallResumed"),
            Self::Custom { event, .. } => formatter
                .debug_struct("Custom")
                .field("event_bytes", &event.len())
                .finish(),
        }
    }
}

fn registration_refresh_options(
    session: &SessionState,
    expires_override: Option<u32>,
    extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
) -> crate::errors::Result<rvoip_sip_dialog::api::unified::RegisterRequestOptions> {
    let registrar_uri = session
        .registrar_uri
        .clone()
        .or_else(|| session.remote_uri.clone())
        .ok_or_else(|| {
            crate::errors::SessionError::InvalidTransition(
                "registration refresh has no retained registrar URI".to_string(),
            )
        })?;
    let contact_uri = session
        .registration_contact
        .clone()
        .or_else(|| session.local_uri.clone())
        .ok_or_else(|| {
            crate::errors::SessionError::InvalidTransition(
                "registration refresh has no retained Contact URI".to_string(),
            )
        })?;
    let aor_uri = session
        .local_uri
        .clone()
        .unwrap_or_else(|| contact_uri.clone());
    let call_id = session.registration_call_id.clone().ok_or_else(|| {
        crate::errors::SessionError::InvalidTransition(
            "registration refresh has no retained Call-ID".to_string(),
        )
    })?;
    let cseq = session.registration_cseq.checked_add(1).ok_or_else(|| {
        crate::errors::SessionError::InvalidTransition(
            "registration refresh CSeq cannot advance".to_string(),
        )
    })?;

    Ok(rvoip_sip_dialog::api::unified::RegisterRequestOptions {
        registrar_uri,
        aor_uri,
        contact_uri,
        expires: expires_override
            .or(session.registration_expires)
            .unwrap_or(3600),
        authorization: None,
        proxy_authorization: None,
        call_id: Some(call_id),
        cseq: Some(cseq),
        outbound_contact: None,
        outbound_proxy_uri: None,
        extra_headers,
        refresh: true,
    })
}

impl StateMachine {
    pub fn new(
        table: Arc<crate::state_table::MasterStateTable>,
        store: Arc<SessionStore>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
    ) -> Self {
        Self {
            table,
            store,
            dialog_adapter,
            media_adapter,
            event_tx: None, // No event channel by default
            exact_api_event_publisher: OnceLock::new(),
            auto_180_ringing: true,
            // SimplePeer events handled by SessionCrossCrateEventHandler
        }
    }

    // new_with_simple_peer_events removed - using SessionCrossCrateEventHandler for event forwarding

    pub fn new_with_adapters(
        store: Arc<SessionStore>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        event_tx: tokio::sync::mpsc::Sender<SessionEvent>,
    ) -> Self {
        Self {
            table: MASTER_TABLE.clone(),
            store,
            dialog_adapter,
            media_adapter,
            event_tx: Some(event_tx),
            exact_api_event_publisher: OnceLock::new(),
            auto_180_ringing: true,
            // SimplePeer events handled by SessionCrossCrateEventHandler
        }
    }

    pub fn new_with_custom_table(
        table: Arc<crate::state_table::MasterStateTable>,
        store: Arc<SessionStore>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        event_tx: tokio::sync::mpsc::Sender<SessionEvent>,
        auto_180_ringing: bool,
    ) -> Self {
        Self {
            table,
            store,
            dialog_adapter,
            media_adapter,
            event_tx: Some(event_tx),
            exact_api_event_publisher: OnceLock::new(),
            auto_180_ringing,
            // SimplePeer events handled by SessionCrossCrateEventHandler
        }
    }

    pub(crate) fn init_exact_api_event_publisher(
        &self,
        publisher: crate::api::lifecycle::SessionEventPublisher,
    ) {
        let _ = self.exact_api_event_publisher.set(publisher);
    }

    /// Publish through the coordinator-owned private application path when it
    /// has been installed. The boolean lets isolated adapter tests fall back
    /// to a sanitized public observation without inventing exact authority.
    pub(crate) fn publish_api_event(&self, event: crate::api::events::Event) -> bool {
        let Some(publisher) = self.exact_api_event_publisher.get() else {
            return false;
        };
        publisher.publish(event);
        true
    }

    /// Publish an application event with authority from the already-held
    /// exact session lifetime. The handle never enters the event coordinator.
    pub(crate) fn publish_api_event_exact(
        &self,
        lifecycle_handle: &SessionRegistryHandle,
        event: crate::api::events::Event,
    ) -> bool {
        let Some(publisher) = self.exact_api_event_publisher.get() else {
            return false;
        };
        publisher.publish_exact(lifecycle_handle, event);
        true
    }

    pub(crate) fn publish_diagnostic_event_exact(
        &self,
        lifecycle_handle: &SessionRegistryHandle,
        event: crate::api::events::DiagnosticEvent,
    ) -> bool {
        let Some(publisher) = self.exact_api_event_publisher.get() else {
            return false;
        };
        publisher.publish_diagnostic_exact(lifecycle_handle, event);
        true
    }

    fn schedule_deferred_action_effects(
        &self,
        handle: &SessionRegistryHandle,
        effects: Vec<actions::DeferredActionEffect>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for effect in effects {
            match effect {
                actions::DeferredActionEffect::AuthRetryObservation(observation) => {
                    let event = observation.event();
                    let diagnostic = observation.into_diagnostic();
                    self.dialog_adapter.publish_api_event_exact(handle, event);
                    let _ = self.publish_diagnostic_event_exact(handle, diagnostic);
                }
                actions::DeferredActionEffect::TransferNotify(effect) => {
                    let store = Arc::clone(&self.store);
                    let dialog_adapter = Arc::clone(&self.dialog_adapter);
                    let authority = Arc::clone(store.authority());
                    let operation_key = effect.transferor.key().clone();
                    let hard_timeout = dialog_adapter
                        .non_invite_transaction_timeout()
                        .saturating_add(REFER_NOTIFY_COMPLETION_GRACE);
                    if let Err(error) = authority.spawn_owned_exact(
                        &operation_key,
                        SessionOperationKind::Signaling,
                        hard_timeout,
                        move |operation| {
                            run_deferred_transfer_notify(operation, store, dialog_adapter, effect)
                        },
                    ) {
                        debug!(
                            session_id = %operation_key.session_id,
                            %error,
                            "stale transferor suppressed deferred REFER NOTIFY"
                        );
                    }
                }
                actions::DeferredActionEffect::Registration(effect) => {
                    self.dialog_adapter
                        .complete_registration_post_commit(handle, effect);
                }
                actions::DeferredActionEffect::SessionRefreshTimer(effect) => {
                    let state_machine = self
                        .dialog_adapter
                        .state_machine
                        .get()
                        .cloned()
                        .ok_or_else(|| {
                            crate::errors::SessionError::InternalError(
                                "RFC 4028 timer requires initialized state-machine authority"
                                    .to_string(),
                            )
                        })?;
                    let authority = Arc::clone(self.store.authority());
                    let operation_key = handle.key().clone();
                    let retained_handle = handle.clone();
                    let hard_timeout = effect
                        .delay
                        .saturating_add(SESSION_REFRESH_COMPLETION_GRACE);
                    authority.spawn_owned_exact(
                        &operation_key,
                        SessionOperationKind::Signaling,
                        hard_timeout,
                        move |operation| {
                            run_deferred_session_refresh(
                                operation,
                                state_machine,
                                retained_handle,
                                effect,
                            )
                        },
                    )?;
                }
                actions::DeferredActionEffect::ReinviteRetry(effect) => {
                    let store = Arc::clone(&self.store);
                    let media_adapter = Arc::clone(&self.media_adapter);
                    let dialog_adapter = Arc::clone(&self.dialog_adapter);
                    let authority = Arc::clone(store.authority());
                    let operation_key = handle.key().clone();
                    let retained_handle = handle.clone();
                    let attempt = effect.attempt;
                    let hard_timeout = effect
                        .backoff
                        .saturating_add(REINVITE_RETRY_COMPLETION_GRACE);

                    // The waiter is intentionally not awaited from the exact
                    // state-machine lane. `spawn_owned_exact` retains the task
                    // in the lifecycle supervisor, so quiesce cancels it and
                    // teardown drains it even after this receiver is dropped.
                    let _waiter = authority.spawn_owned_exact(
                        &operation_key,
                        SessionOperationKind::Signaling,
                        hard_timeout,
                        move |operation| {
                            let dispatch_store = Arc::clone(&store);
                            let dispatch_handle = retained_handle.clone();
                            run_deferred_reinvite_retry(
                                operation,
                                store,
                                retained_handle,
                                effect,
                                move |kind| async move {
                                    let current = dispatch_store
                                        .get_session_snapshot_exact(&dispatch_handle)
                                        .map_err(|error| error.to_string())?;
                                    let mut session = current.state().clone();
                                    if !reinvite_retry_matches(&session, &kind, attempt) {
                                        return Err(
                                            "re-INVITE retry intent was superseded before dispatch"
                                                .to_string(),
                                        );
                                    }
                                    let generated_sdp = match &kind {
                                        crate::session_store::state::PendingReinvite::Hold => {
                                            media_adapter
                                                .create_hold_sdp_for_session_lane_owned(
                                                    &mut session,
                                                )
                                                .await
                                                .map_err(|error| {
                                                    format!("create_hold_sdp failed: {error}")
                                                })
                                        }
                                        crate::session_store::state::PendingReinvite::Resume => {
                                            media_adapter
                                                .create_active_sdp_for_session_lane_owned(
                                                    &mut session,
                                                )
                                                .await
                                                .map_err(|error| {
                                                    format!("create_active_sdp failed: {error}")
                                                })
                                        }
                                        crate::session_store::state::PendingReinvite::SdpUpdate(
                                            sdp,
                                        ) => Ok(sdp.clone()),
                                    };
                                    let sdp = match generated_sdp {
                                        Ok(sdp) => sdp,
                                        Err(error) => {
                                            // Origin version advancement is monotonic even when
                                            // SDP rendering fails. Publish the lane-owned mutation
                                            // before returning the bounded generation error.
                                            commit_lane_state(&dispatch_store, session)
                                                .map_err(|commit| commit.to_string())?;
                                            return Err(error);
                                        }
                                    };
                                    session.replace_pending_local_offer(sdp.clone());
                                    actions::stage_reinvite_local_sdp(&mut session, sdp.clone());
                                    let committed = commit_lane_state(&dispatch_store, session)
                                        .map_err(|error| error.to_string())?;
                                    let options = Arc::new(
                                        rvoip_sip_dialog::api::unified::ReInviteRequestOptions {
                                            sdp: Some(sdp),
                                            ..Default::default()
                                        },
                                    );
                                    let lease = match dialog_adapter
                                        .outbound_request_tracker
                                        .prepare(
                                            &dispatch_handle,
                                            crate::adapters::outbound_request_tracker::TrackedInDialogOptions::Reinvite(
                                                Arc::clone(&options),
                                            ),
                                        )
                                    {
                                        Ok(lease) => lease,
                                        Err(error) => {
                                            let mut rollback = committed.state().clone();
                                            rollback.rollback_offer_answer();
                                            rollback.pending_reinvite = None;
                                            rollback.reinvite_retry_attempts = 0;
                                            media_adapter
                                                .discard_pending_srtp_offer_for_session(&rollback);
                                            media_adapter
                                                .discard_staged_media_negotiation_for_session(&rollback);
                                            commit_lane_state(&dispatch_store, rollback)
                                                .map_err(|commit| commit.to_string())?;
                                            return Err(error.to_string());
                                        }
                                    };

                                    let transaction_id = match dialog_adapter
                                        .send_reinvite_with_options_lane_owned(
                                            committed.state(),
                                            (*options).clone(),
                                        )
                                        .await
                                    {
                                        Ok(transaction_id) => transaction_id,
                                        Err(error) => {
                                            let current = dispatch_store
                                                .get_session_snapshot_exact(&dispatch_handle)
                                                .map_err(|lookup| lookup.to_string())?;
                                            let mut rollback = current.state().clone();
                                            rollback.rollback_offer_answer();
                                            rollback.pending_reinvite = None;
                                            rollback.reinvite_retry_attempts = 0;
                                            media_adapter
                                                .discard_pending_srtp_offer_for_session(&rollback);
                                            media_adapter
                                                .discard_staged_media_negotiation_for_session(&rollback);
                                            commit_lane_state(&dispatch_store, rollback)
                                                .map_err(|commit| commit.to_string())?;
                                            return Err(error.to_string());
                                        }
                                    };

                                    // A 491 retry has a fresh client branch;
                                    // correlate the pending answer with this
                                    // transaction before publishing tracker state.
                                    let current = dispatch_store
                                        .get_session_snapshot_exact(&dispatch_handle)
                                        .map_err(|error| error.to_string())?;
                                    let mut session = current.state().clone();
                                    if !reinvite_retry_matches(&session, &kind, attempt) {
                                        return Err(
                                            "re-INVITE retry completed after its intent was superseded"
                                                .to_string(),
                                        );
                                    }
                                    if let Err(error) = session
                                        .bind_offer_answer_transaction(transaction_id.clone())
                                    {
                                        session.rollback_offer_answer();
                                        session.pending_reinvite = None;
                                        session.reinvite_retry_attempts = 0;
                                        media_adapter
                                            .discard_pending_srtp_offer_for_session(&session);
                                        media_adapter
                                            .discard_staged_media_negotiation_for_session(&session);
                                        commit_lane_state(&dispatch_store, session)
                                            .map_err(|commit| commit.to_string())?;
                                        return Err(error.to_string());
                                    }
                                    commit_lane_state(&dispatch_store, session)
                                        .map_err(|error| error.to_string())?;
                                    if let Err(error) = dialog_adapter
                                        .outbound_request_tracker
                                        .activate(lease, transaction_id)
                                    {
                                        let current = dispatch_store
                                            .get_session_snapshot_exact(&dispatch_handle)
                                            .map_err(|lookup| lookup.to_string())?;
                                        let mut rollback = current.state().clone();
                                        rollback.rollback_offer_answer();
                                        rollback.pending_reinvite = None;
                                        rollback.reinvite_retry_attempts = 0;
                                        media_adapter
                                            .discard_pending_srtp_offer_for_session(&rollback);
                                        media_adapter
                                            .discard_staged_media_negotiation_for_session(&rollback);
                                        commit_lane_state(&dispatch_store, rollback)
                                            .map_err(|commit| commit.to_string())?;
                                        return Err(error.to_string());
                                    }
                                    Ok(())
                                },
                            )
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    // Callback registry removed - using event-driven approach

    /// Check if a transition exists for the given state key
    pub fn has_transition(&self, key: &StateKey) -> bool {
        self.table.has_transition(key)
    }

    /// SIP_API_DESIGN_2 §7.3 invariants #1 + #5 — atomically check the
    /// matching `pending_<method>_options` staging slot and the authoritative
    /// in-flight request tracker. If both are empty, write the provided
    /// `Arc<XxxRequestOptions>`. If either is occupied (a prior `.send()` is
    /// still staging or in flight for the same method on this session) return
    /// `Err(SessionError::Conflict { method })` without mutating
    /// anything.
    ///
    /// Builders call this *before* queuing the matching
    /// `EventType::SendOutbound<METHOD>` event so the state-table
    /// transition's `Action::Send<METHOD>WithOptions` handler can transfer
    /// the immutable snapshot into the tracker before the request reaches the
    /// wire. INFO/REFER/NOTIFY/UPDATE staging slots are then cleared; their
    /// same-method conflict remains enforced by the tracker until the exact
    /// terminal transaction event. Other methods retain their legacy stash
    /// lifecycle.
    pub async fn stage_outbound_options(
        &self,
        session_id: &SessionId,
        slot: PendingOptionsSlot,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Stage through the same exact-lifetime lane as event execution. A
        // complete event owns one local working state until its final commit;
        // staging outside this lane could otherwise be overwritten by that
        // publication before the matching event consumes the request options.
        let (handle, _state_machine_lane) = self.acquire_state_machine_lane(session_id).await?;
        self.stage_outbound_options_exact(&handle, slot)
    }

    fn stage_outbound_options_exact(
        &self,
        handle: &SessionRegistryHandle,
        slot: PendingOptionsSlot,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let session_id = handle.session_id();
        let method = slot.method();
        let tracked_method =
            crate::adapters::outbound_request_tracker::TrackedInDialogMethod::from_sip_method(
                &method,
            );
        let outbound_request_tracker = self.dialog_adapter.outbound_request_tracker.clone();
        self.store
            .update_session_exact_with(handle, None, |session| {
                if tracked_method.is_some_and(|tracked_method| {
                    outbound_request_tracker.has_request(handle, tracked_method)
                }) {
                    return Err(crate::errors::SessionError::Conflict {
                        method: method.clone(),
                    });
                }
                slot.stage_if_vacant(session)
            })
            .map_err(|e| {
                Box::<dyn std::error::Error + Send + Sync>::from(format!(
                    "stage_outbound_options: session {} not found: {}",
                    session_id, e
                ))
            })??;
        Ok(())
    }

    /// Acquire the complete-event lane for one exact session lifetime.
    ///
    /// A transition owns one event-local working state across async actions
    /// and publishes it once when the event succeeds or fails. The lane keeps
    /// response-driven events queued until that canonical publication, rather
    /// than allowing two full snapshots to commit in opposite order. It lives
    /// on `SessionStateCell`, so there is no global contention and raw-ID reuse
    /// receives a different lock. Revalidate after waiting to reject a queued
    /// event whose captured lifetime retired in the meantime.
    async fn acquire_state_machine_lane(
        &self,
        session_id: &SessionId,
    ) -> Result<
        (SessionRegistryHandle, tokio::sync::OwnedMutexGuard<()>),
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (handle, lane) = self.store.state_machine_lane(session_id).ok_or_else(|| {
            Box::new(crate::errors::SessionError::SessionNotFound(
                session_id.to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        let guard = lane.lock_owned().await;
        self.store
            .get_session_snapshot_exact(&handle)
            .map_err(|_| {
                Box::new(crate::errors::SessionError::SessionNotFound(
                    session_id.to_string(),
                )) as Box<dyn std::error::Error + Send + Sync>
            })?;
        Ok((handle, guard))
    }

    /// Acquire the complete-event lane for an already retained registry
    /// handle. Unlike the raw-ID entry point, this cannot resolve to a later
    /// lifetime after a delayed callback wakes.
    async fn acquire_state_machine_lane_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<tokio::sync::OwnedMutexGuard<()>, Box<dyn std::error::Error + Send + Sync>> {
        let lane = self.store.state_machine_lane_exact(handle).ok_or_else(|| {
            Box::new(crate::errors::SessionError::SessionNotFound(
                handle.session_id().to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        let guard = lane.lock_owned().await;
        self.store.get_session_snapshot_exact(handle).map_err(|_| {
            Box::new(crate::errors::SessionError::SessionNotFound(
                handle.session_id().to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        Ok(guard)
    }

    /// Process an event for a session
    pub async fn process_event(
        &self,
        session_id: &SessionId,
        event: EventType,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let (handle, _state_machine_lane) = self.acquire_state_machine_lane(session_id).await?;
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!("{}:{}", session_id, state_machine_event_name(&event)),
        );
        // `process_event_inner` contains the complete queued-event executor.
        // Keep its generated state behind a heap boundary so callers do not
        // combine that large future with their own protocol task stack.
        let result = Box::pin(self.process_event_inner(&handle, event, None, None)).await;
        match &result {
            Ok(_) => guard.finish_success(),
            Err(_) => guard.finish_failure(),
        }
        result
    }

    /// Process a delayed event only for the generation and registry revision
    /// captured when that work was scheduled.
    pub(crate) async fn process_event_exact(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        let session_id = handle.session_id();
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!("{}:{}", session_id, state_machine_event_name(&event)),
        );
        let result = Box::pin(self.process_event_inner(handle, event, None, None)).await;
        match &result {
            Ok(_) => guard.finish_success(),
            Err(_) => guard.finish_failure(),
        }
        result
    }

    /// Stage one immutable builder snapshot and dispatch its matching event
    /// while holding the same exact-session lane. The public two-step
    /// stage/dispatch API remains available as a compatibility facade, while
    /// crate-owned builders use this path so no unrelated transition can run
    /// between the two operations.
    /// Exact-lifetime form of atomic option staging plus dispatch. The
    /// captured handle is retained across lane acquisition and every await,
    /// so a queued builder cannot stage onto a replacement session that
    /// reused the same raw Call-ID.
    pub(crate) async fn process_event_with_staged_options_exact(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
        slot: PendingOptionsSlot,
        stage_claim: Arc<StageDispatchClaim>,
        outbound_session: Option<OutboundSessionStateInput>,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        self.stage_outbound_options_exact(handle, slot.clone())?;
        let staging = PendingOptionsStageGuard::new_with_claim(
            Arc::clone(&self.store),
            handle.clone(),
            slot,
            Arc::clone(&stage_claim),
        );
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!(
                "{}:{}",
                handle.session_id(),
                state_machine_event_name(&event)
            ),
        );
        let state_input = outbound_session.map(|outbound_session| EventStateInput {
            outbound_session: Some(outbound_session),
            ..Default::default()
        });
        let result =
            Box::pin(self.process_event_inner(handle, event, Some(stage_claim), state_input)).await;
        match result {
            Ok(result) => {
                if let Err(error) = staging.confirm_claimed().await {
                    guard.finish_failure();
                    return Err(error.into());
                }
                guard.finish_success();
                Ok(result)
            }
            Err(error) => {
                guard.finish_failure();
                Err(error)
            }
        }
    }

    /// Derive and dispatch one registration refresh entirely inside the
    /// generation-qualified session lane. The retained registration identity,
    /// accepted expiry, and `CSeq + 1` are read only after lane acquisition;
    /// staging and the YAML event then run without an intervening writer.
    ///
    /// `SendOutboundRegister` preserves the public builder's existing state
    /// transition, while `RefreshRegistration` preserves automatic refresh
    /// lifecycle behavior. Both events use the same immutable REGISTER
    /// snapshot and cancellation fence.
    pub(crate) async fn process_registration_refresh_exact(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
        expires_override: Option<u32>,
        extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
        stage_claim: Arc<StageDispatchClaim>,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        if !matches!(
            event,
            EventType::SendOutboundRegister | EventType::RefreshRegistration
        ) || stage_claim.kind() != PendingOptionsSlotKind::Register
        {
            return Err(crate::errors::SessionError::InvalidTransition(
                "registration refresh requires exact REGISTER dispatch authority".to_string(),
            )
            .into());
        }

        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        let session = self.store.get_session_snapshot_exact(handle).map_err(|_| {
            crate::errors::SessionError::SessionNotFound(handle.session_id().to_string())
        })?;
        let options = registration_refresh_options(&session, expires_override, extra_headers)?;
        let slot = PendingOptionsSlot::Register(Arc::new(options));
        stage_claim.install_deferred_slot(slot.clone())?;
        self.stage_outbound_options_exact(handle, slot.clone())?;
        let staging = PendingOptionsStageGuard::new_with_claim(
            Arc::clone(&self.store),
            handle.clone(),
            slot,
            Arc::clone(&stage_claim),
        );
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!(
                "{}:{}",
                handle.session_id(),
                state_machine_event_name(&event)
            ),
        );
        let result =
            Box::pin(self.process_event_inner(handle, event, Some(stage_claim), None)).await;
        match result {
            Ok(result) => {
                if let Err(error) = staging.confirm_claimed().await {
                    guard.finish_failure();
                    return Err(error.into());
                }
                guard.finish_success();
                Ok(result)
            }
            Err(error) => {
                guard.finish_failure();
                Err(error)
            }
        }
    }

    /// Apply outbound lifecycle metadata and dispatch one event under the
    /// same exact-session lane. Transfer-leg creation uses this path without
    /// manufacturing an options-staging session.
    pub(crate) async fn process_event_with_outbound_session_input(
        &self,
        session_id: &SessionId,
        event: EventType,
        outbound_session: OutboundSessionStateInput,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        self.process_event_with_state_input(
            session_id,
            event,
            EventStateInput {
                outbound_session: Some(outbound_session),
                ..Default::default()
            },
        )
        .await
    }

    pub(crate) async fn process_event_with_registration_start_input(
        &self,
        session_id: &SessionId,
        event: EventType,
        registration_start: RegistrationStartInput,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        self.process_event_with_state_input(
            session_id,
            event,
            EventStateInput {
                registration_start: Some(registration_start),
                ..Default::default()
            },
        )
        .await
    }

    /// Apply exact wire correlation and execute `AuthRequired` on one retained
    /// session lifetime. Both the correlation fields and the transition are
    /// owned by the same generation-qualified execution lane.
    pub(crate) async fn process_auth_required_exact(
        &self,
        handle: &SessionRegistryHandle,
        status_code: u16,
        challenge: String,
        method: String,
        auth_required: AuthRequiredStateInput,
    ) -> Result<AuthRequiredProcessOutcome, Box<dyn std::error::Error + Send + Sync>> {
        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        let state_before_auth = self.store.get_session_snapshot_exact(handle)?.call_state;
        let event = EventType::AuthRequired {
            status_code,
            challenge,
            method,
        };
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!(
                "{}:{}",
                handle.session_id(),
                state_machine_event_name(&event)
            ),
        );
        let result = Box::pin(self.process_event_inner(
            handle,
            event,
            None,
            Some(EventStateInput {
                auth_required: Some(auth_required),
                ..Default::default()
            }),
        ))
        .await;
        match &result {
            Ok(_) => guard.finish_success(),
            Err(_) => guard.finish_failure(),
        }
        Ok(AuthRequiredProcessOutcome {
            state_before_auth,
            result,
        })
    }

    /// Release auth coordination only for the retained lifetime and exact
    /// transaction that completed. A stale callback cannot resolve this
    /// handle to a later session generation.
    pub(crate) async fn clear_tracked_request_auth_state_exact(
        &self,
        handle: &SessionRegistryHandle,
        transaction_id: &str,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        clear_tracked_request_auth_state_in_exact_lane(&self.store, handle, transaction_id).await
    }

    /// Install transfer linkage on an existing exact lifetime while excluding
    /// concurrent state-machine work for that session.
    pub(crate) async fn set_transferor_session(
        &self,
        session_id: &SessionId,
        transferor_session_id: SessionId,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let transferor_lifecycle_handle = self
            .store
            .lifecycle_handle(&transferor_session_id)
            .ok_or_else(|| {
                Box::new(crate::errors::SessionError::SessionNotFound(
                    transferor_session_id.to_string(),
                )) as Box<dyn std::error::Error + Send + Sync>
            })?;
        let (handle, _state_machine_lane) = self.acquire_state_machine_lane(session_id).await?;
        self.store
            .update_session_exact_with(&handle, None, |session| {
                session.transferor_session_id = Some(transferor_session_id);
                session.transferor_lifecycle_handle = Some(transferor_lifecycle_handle);
                session.is_transfer_call = true;
            })?;
        Ok(())
    }

    /// Stage an inbound REFER on one lane-owned working state and commit it
    /// before publishing the application request. This is intentionally a
    /// no-transition commit: the YAML `TransferRequested` row must run only
    /// after application acceptance or the retained delayed default.
    pub(crate) async fn stage_transfer_request_exact(
        &self,
        handle: &SessionRegistryHandle,
        input: TransferRequestStateInput,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        let mut session = self.store.get_session_exact(handle).await?;
        EventStateInput {
            transfer_request: Some(input),
            ..Default::default()
        }
        .apply(&mut session);
        commit_lane_state(&self.store, session)?;
        Ok(())
    }

    /// Apply one parsed REFER progress NOTIFY and dispatch the existing YAML
    /// `ReceiveNOTIFY` transition while holding the captured exact lifetime's
    /// lane. The returned observation descriptor is released only after the
    /// YAML action and canonical state commit succeed.
    pub(crate) async fn process_refer_notify_exact(
        &self,
        handle: &SessionRegistryHandle,
        input: ReferNotifyInput,
    ) -> Result<ReferNotifyOutcome, Box<dyn std::error::Error + Send + Sync>> {
        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        let session = self.store.get_session_exact(handle).await?;
        let event = EventType::ReceiveNOTIFY;
        let key = StateKey {
            role: session.role,
            state: session.call_state,
            event: event.clone(),
        };
        if self.table.get(&key).is_none() {
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "ReceiveNOTIFY has no YAML transition for exact session {}",
                handle.session_id()
            ))
            .into());
        }
        let outcome = input.outcome(&session);
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!(
                "{}:{}",
                handle.session_id(),
                state_machine_event_name(&event)
            ),
        );
        let result = Box::pin(self.process_event_inner(
            handle,
            event,
            None,
            Some(EventStateInput {
                refer_notify: Some(input),
                ..Default::default()
            }),
        ))
        .await;
        match result {
            Ok(result) if result.transition.is_some() => {
                guard.finish_success();
                Ok(outcome)
            }
            Ok(_) => {
                guard.finish_failure();
                Err(crate::errors::SessionError::InvalidTransition(format!(
                    "ReceiveNOTIFY has no YAML transition for exact session {}",
                    handle.session_id()
                ))
                .into())
            }
            Err(error) => {
                guard.finish_failure();
                Err(error)
            }
        }
    }

    /// Reject a pending inbound REFER through its exact transaction and clear
    /// only the matching pending transfer state under the session lane.
    pub(crate) async fn reject_refer(
        &self,
        session_id: &SessionId,
        status_code: u16,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let handle = self.store.lifecycle_handle(session_id).ok_or_else(|| {
            Box::new(crate::errors::SessionError::SessionNotFound(
                session_id.to_string(),
            )) as Box<dyn std::error::Error + Send + Sync>
        })?;
        self.reject_refer_exact(&handle, status_code).await
    }

    /// Reject a REFER only for the generation that received it.
    pub(crate) async fn reject_refer_exact(
        &self,
        handle: &SessionRegistryHandle,
        status_code: u16,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        let session_id = handle.session_id();
        let transaction_id = self
            .store
            .get_session_snapshot_exact(handle)?
            .refer_transaction_id
            .clone()
            .ok_or_else(|| {
                crate::errors::SessionError::Other(format!(
                    "No pending REFER transaction for session {session_id}"
                ))
            })?;
        let terminal = actions::send_exact_refer_final_response(
            session_id,
            &transaction_id,
            &self.dialog_adapter,
            status_code,
        )
        .await?;
        self.store
            .update_session_exact_with(handle, None, |session| {
                if session.refer_transaction_id.as_deref() == Some(transaction_id.as_str()) {
                    session.refer_transaction_id = None;
                    session.transfer_target = None;
                    session.transfer_state = crate::session_store::state::TransferState::None;
                }
            })?;
        if let Some(error) = terminal.terminal_error {
            return Err(error);
        }
        Ok(())
    }

    /// Apply remote SDP only to the generation captured by causal ingress.
    pub(crate) async fn process_event_with_remote_sdp_exact(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
        remote_sdp: Option<String>,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let preserve_committed_provisional_sdp = matches!(event, EventType::Dialog200OK);
        self.process_event_with_state_input_exact(
            handle,
            event,
            EventStateInput {
                remote_sdp,
                remote_sdp_supplied: true,
                preserve_committed_provisional_sdp,
                ..Default::default()
            },
        )
        .await
    }

    pub(crate) async fn process_invite_2xx_answer_exact(
        &self,
        handle: &SessionRegistryHandle,
        remote_sdp: Option<String>,
        ack: Invite2xxAckStateInput,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        self.process_event_with_state_input_exact(
            handle,
            EventType::Dialog200OK,
            EventStateInput {
                remote_sdp,
                remote_sdp_supplied: true,
                preserve_committed_provisional_sdp: true,
                invite_2xx_ack: Some(ack),
                ..Default::default()
            },
        )
        .await
    }

    /// Process an inbound in-dialog INVITE or UPDATE with the exact server
    /// transaction derived from that event's preserved request. The authority
    /// remains event-local while this method owns the exact-session lane.
    pub(crate) async fn process_inbound_response_event_exact(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
        inbound_response: InboundResponseStateInput,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        self.process_event_with_state_input_exact(
            handle,
            event,
            EventStateInput {
                inbound_response: Some(inbound_response),
                ..Default::default()
            },
        )
        .await
    }

    /// Process an event while applying a caller-supplied local SDP answer
    /// only after the exact session lane has been acquired.
    pub(crate) async fn process_event_with_local_sdp(
        &self,
        session_id: &SessionId,
        event: EventType,
        local_sdp: String,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        self.process_event_with_state_input(
            session_id,
            event,
            EventStateInput {
                local_sdp: Some(local_sdp),
                sdp_negotiated: Some(true),
                ..Default::default()
            },
        )
        .await
    }

    /// Process an application-authored response only for the exact lifetime
    /// captured by the incoming request or response builder.
    pub(crate) async fn process_event_with_response_input_exact(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
        response: ResponseStateInput,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        self.process_event_with_state_input_exact(
            handle,
            event,
            EventStateInput {
                response: Some(response),
                ..Default::default()
            },
        )
        .await
    }

    /// Drive one private RFC 4028 decision on the captured session lifetime.
    /// The typed input is validated under the same lane that applies it and
    /// executes the reserved YAML row; a delayed task or transaction from an
    /// earlier lifetime cannot address a replacement session with the same
    /// public identifier.
    pub(crate) async fn process_session_refresh_exact(
        &self,
        handle: &SessionRegistryHandle,
        input: SessionRefreshStateInput,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        let snapshot = self.store.get_session_snapshot_exact(handle)?;
        input.validate(snapshot.state())?;
        let event = input.event();
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!(
                "{}:{}",
                handle.session_id(),
                state_machine_event_name(&event)
            ),
        );
        let result = Box::pin(self.process_event_inner(
            handle,
            event,
            None,
            Some(EventStateInput {
                session_refresh: Some(input),
                ..Default::default()
            }),
        ))
        .await;
        match &result {
            Ok(result) => {
                guard.finish_success();
                if result.transition.is_some() {
                    if let Some(publisher) = self.exact_api_event_publisher.get() {
                        match input {
                            SessionRefreshStateInput::UpdateSucceeded
                            | SessionRefreshStateInput::ReinviteSucceeded => {
                                if let Ok(committed) = self.store.get_session_snapshot_exact(handle)
                                {
                                    if let Some(expires_secs) =
                                        committed.session_refresh_interval_secs
                                    {
                                        publisher.publish_exact(
                                            handle,
                                            crate::api::events::Event::SessionRefreshed {
                                                call_id: handle.session_id().clone(),
                                                expires_secs,
                                            },
                                        );
                                    }
                                }
                            }
                            SessionRefreshStateInput::ReinviteFailed
                            | SessionRefreshStateInput::PeerExpired { .. } => {
                                publisher.publish_exact(
                                    handle,
                                    crate::api::events::Event::SessionRefreshFailed {
                                        call_id: handle.session_id().clone(),
                                        reason: "Session expired (RFC 4028; SIP cause=408)"
                                            .to_string(),
                                    },
                                );
                            }
                            SessionRefreshStateInput::UpdateDue { .. }
                            | SessionRefreshStateInput::ReinviteDue { .. }
                            | SessionRefreshStateInput::UpdateFailed => {}
                        }
                    }
                }
            }
            Err(_) => guard.finish_failure(),
        }
        result
    }

    /// Drive the private confirmed-dialog negotiation-failure transition on
    /// the captured exact lifetime. A public `MediaEvent(String)` carrying the
    /// same reserved tag is rejected without this typed sidecar.
    pub(crate) async fn process_confirmed_negotiation_failure_exact(
        &self,
        handle: &SessionRegistryHandle,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        let event = EventType::MediaEvent(
            crate::state_table::types::CONFIRMED_NEGOTIATION_FAILURE_EVENT.to_string(),
        );
        Box::pin(self.process_event_inner(
            handle,
            event,
            None,
            Some(EventStateInput {
                confirmed_negotiation_failure: true,
                ..Default::default()
            }),
        ))
        .await
    }

    async fn process_event_with_state_input(
        &self,
        session_id: &SessionId,
        event: EventType,
        input: EventStateInput,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let (handle, _state_machine_lane) = self.acquire_state_machine_lane(session_id).await?;
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!("{}:{}", session_id, state_machine_event_name(&event)),
        );
        let result = Box::pin(self.process_event_inner(&handle, event, None, Some(input))).await;
        match &result {
            Ok(_) => guard.finish_success(),
            Err(_) => guard.finish_failure(),
        }
        result
    }

    async fn process_event_with_state_input_exact(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
        input: EventStateInput,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let _state_machine_lane = self.acquire_state_machine_lane_exact(handle).await?;
        let session_id = handle.session_id();
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!("{}:{}", session_id, state_machine_event_name(&event)),
        );
        let result = Box::pin(self.process_event_inner(handle, event, None, Some(input))).await;
        match &result {
            Ok(_) => guard.finish_success(),
            Err(_) => guard.finish_failure(),
        }
        result
    }

    async fn process_event_inner(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
        mut initial_stage_claim: Option<Arc<StageDispatchClaim>>,
        mut initial_state_input: Option<EventStateInput>,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        use std::collections::VecDeque;

        const MAX_INTERNAL_EVENTS: usize = 32;
        let session_id = handle.session_id();

        let mut queue = VecDeque::new();
        queue.push_back(event);
        let mut first_result = None;
        let mut processed = 0usize;

        while let Some(event) = queue.pop_front() {
            processed += 1;
            if processed > MAX_INTERNAL_EVENTS {
                return Err(crate::errors::SessionError::InternalError(format!(
                    "state-machine internal event limit ({}) exceeded for session {}",
                    MAX_INTERNAL_EVENTS, session_id
                ))
                .into());
            }

            // `process_one_event` owns the transition table and every action
            // variant. Boxing this boundary keeps its large debug-build poll
            // state out of the queue executor's stack frame.
            let stage_claim = if processed == 1 {
                initial_stage_claim.take()
            } else {
                None
            };
            let state_input = if processed == 1 {
                initial_state_input.take()
            } else {
                None
            };
            let result = Box::pin(self.process_one_event(
                handle,
                event,
                &mut queue,
                stage_claim.as_ref(),
                state_input,
            ))
            .await?;
            if first_result.is_none() {
                first_result = Some(result);
            }
        }

        first_result.ok_or_else(|| {
            crate::errors::SessionError::InternalError(format!(
                "state-machine queue was empty for session {}",
                session_id
            ))
            .into()
        })
    }

    async fn process_one_event(
        &self,
        handle: &SessionRegistryHandle,
        event: EventType,
        queued_follow_up_events: &mut std::collections::VecDeque<EventType>,
        stage_claim: Option<&Arc<StageDispatchClaim>>,
        state_input: Option<EventStateInput>,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        use crate::session_store::history::history_event_snapshot;
        use crate::session_store::{ActionRecord, GuardResult, TransitionRecord};
        use std::time::Instant;
        let session_id = handle.session_id();

        debug!(
            "Processing event {} for session {}",
            state_machine_event_name(&event),
            session_id
        );
        let transition_start = Instant::now();
        let history_event = history_event_snapshot(&event);
        // 1. Get the current immutable revision and retain it as the canonical
        // pre-event image. Typed inputs and event payloads must be visible to
        // YAML guards, but a missing row or failed guard may publish only a
        // diagnostic history record attached to this unmodified preimage.
        let pre_event_snapshot = match self.store.get_session_snapshot_exact(handle) {
            Ok(snapshot) => snapshot,
            Err(e) => {
                // Demoted from error — under test teardown races the
                // session can be removed between event enqueue and
                // dispatch. The caller still surfaces SessionNotFound
                // through the return value, which is the load-bearing
                // signal; the log line is purely diagnostic.
                debug!("Failed to get session {}: {}", session_id, e);
                return Err(
                    crate::errors::SessionError::SessionNotFound(session_id.to_string()).into(),
                );
            }
        };
        let rejected_history_enabled = pre_event_snapshot.state().history_recording_enabled();
        let old_state = pre_event_snapshot.state().call_state;
        let mut session = pre_event_snapshot.state().clone();
        let mut refer_notify_input = None;
        let mut inbound_response_input = None;
        let mut invite_2xx_ack_input = None;
        let mut response_input_active = false;
        let reserved_session_refresh_event = matches!(
            &event,
            EventType::MediaEvent(tag) if tag.starts_with(SESSION_REFRESH_EVENT_PREFIX)
        );
        let reserved_confirmed_negotiation_failure = matches!(
            &event,
            EventType::MediaEvent(tag)
                if tag == crate::state_table::types::CONFIRMED_NEGOTIATION_FAILURE_EVENT
        );
        let supplied_confirmed_negotiation_failure = state_input
            .as_ref()
            .is_some_and(|input| input.confirmed_negotiation_failure);
        let supplied_session_refresh_event = state_input
            .as_ref()
            .and_then(|input| input.session_refresh.as_ref())
            .map(SessionRefreshStateInput::event);
        if reserved_session_refresh_event && supplied_session_refresh_event.as_ref() != Some(&event)
        {
            return Err(crate::errors::SessionError::InvalidTransition(
                "reserved RFC 4028 event requires matching exact typed input".to_string(),
            )
            .into());
        }
        if reserved_confirmed_negotiation_failure != supplied_confirmed_negotiation_failure {
            return Err(crate::errors::SessionError::InvalidTransition(
                "reserved confirmed-negotiation failure requires matching exact typed input"
                    .to_string(),
            )
            .into());
        }
        if let Some(mut input) = state_input {
            refer_notify_input = input.refer_notify.take();
            inbound_response_input = input.inbound_response.take();
            invite_2xx_ack_input = input.invite_2xx_ack.take();
            if let Some(inbound_response) = inbound_response_input.as_ref() {
                inbound_response.validate_event(&event)?;
            }
            if let Some(ack) = invite_2xx_ack_input.as_ref() {
                ack.validate_event(&event)?;
            }
            response_input_active = input.response.is_some();
            input.apply(&mut session);
        }
        // Initialize tracking for history
        let mut guards_evaluated = Vec::new();
        let mut actions_executed_history = Vec::new();
        let mut errors = Vec::new();

        // 1a. Store event-specific data in session state
        match &event {
            EventType::MakeCall { target } => {
                session.remote_uri = Some(target.clone());
                // local_uri should be set when session is created
            }
            EventType::IncomingCall { from, sdp }
            | EventType::IncomingCallAutoAccept { from, sdp } => {
                apply_incoming_call_event_state(&mut session, from, sdp.as_deref());
            }
            EventType::RejectCall { status, reason } => {
                session.reject_status = Some(*status);
                session.reject_reason = Some(reason.clone());
            }
            EventType::RedirectCall { status, contacts } => {
                session.redirect_response_status = Some(*status);
                session.redirect_response_contacts = contacts.clone();
            }
            EventType::SendEarlyMedia {
                sdp: Some(sdp_data),
            } => {
                session.early_media_sdp = Some(sdp_data.clone());
            }
            EventType::AuthRequired {
                status_code,
                challenge,
                method,
            } => {
                session.pending_auth = Some((*status_code, challenge.clone()));
                session.pending_auth_method = if method.is_empty() {
                    None
                } else {
                    Some(method.clone())
                };
            }
            EventType::SessionIntervalTooSmall { min_se_secs } => {
                // RFC 4028 §6 — stash the peer's required floor for the
                // retry action to consume. Normalize 0 / missing to None so
                // the action's "no Min-SE cached" guard fires cleanly.
                session.session_timer_min_se = if *min_se_secs > 0 {
                    Some(*min_se_secs)
                } else {
                    None
                };
            }
            EventType::Dialog3xxRedirect { targets, .. } => {
                // Append to any existing targets (keeps earlier hops' fallbacks
                // reachable in case the newly-suggested target also redirects).
                // Dedupe trivially to avoid fast loops.
                for t in targets {
                    if !session.redirect_targets.contains(t) {
                        session.redirect_targets.push(t.clone());
                    }
                }
            }
            // BlindTransfer event removed
            EventType::TransferRequested {
                refer_to,
                transfer_type,
                transaction_id,
            } => {
                session.transfer_target = Some(refer_to.clone());
                session.transfer_notify_dialog = session.dialog_id;
                session.refer_transaction_id = Some(transaction_id.clone());
                debug!(
                    target_present = !refer_to.is_empty(),
                    target_bytes = refer_to.len(),
                    transfer_type = ?transfer_type,
                    transaction_present = !transaction_id.is_empty(),
                    transaction_bytes = transaction_id.len(),
                    "Set transfer state from REFER"
                );
            }
            // StartAttendedTransfer event removed
            EventType::ReinviteReceived { sdp } => {
                // RFC 3261 §14.1 UAS-side glare — if we have an
                // outbound builder-API re-INVITE in flight (state stays
                // `Active`, so the state-based detection covering
                // HoldPending/Resuming does not fire), respond 491
                // Request Pending and short-circuit the table lookup.
                // The peer is expected to back off and retry. The state
                // machine's HoldPending/Resuming rows handle the
                // hold/resume flavours via state alone.
                if session.call_state == crate::types::CallState::Active
                    && (session.pending_reinvite.is_some()
                        || session.pending_offer_answer.is_some())
                {
                    info!(
                        "RFC 3261 §14.1 UAS-side glare: peer re-INVITE arrived while \
                         our builder-API re-INVITE is in flight on session {} — \
                         responding 491 Request Pending",
                        session.session_id
                    );
                    let terminal = actions::send_exact_inbound_final_response(
                        &session.session_id,
                        inbound_response_input.as_mut(),
                        &self.dialog_adapter,
                        491,
                        None,
                        None,
                    )
                    .await?;
                    if let Some(error) = terminal.terminal_error {
                        return Err(error);
                    }
                    return Ok(ProcessEventResult {
                        old_state,
                        next_state: None,
                        transition: None,
                        actions_executed: vec![],
                        events_published: vec![],
                    });
                }
                // Stash the peer's new SDP offer so NegotiateSDPAsUAS
                // picks it up when it fires later in this transition.
                // Force renegotiation — the peer's offer supersedes any
                // previously negotiated remote SDP.
                if let Some(sdp_data) = sdp {
                    session.remote_sdp = Some(sdp_data.clone());
                    session.sdp_negotiated = false;
                }
            }
            EventType::UpdateReceived {
                sdp: Some(sdp_data),
            } => {
                if session.pending_offer_answer.is_some() {
                    let terminal = actions::send_exact_inbound_final_response(
                        &session.session_id,
                        inbound_response_input.as_mut(),
                        &self.dialog_adapter,
                        491,
                        None,
                        None,
                    )
                    .await?;
                    if let Some(error) = terminal.terminal_error {
                        return Err(error);
                    }
                    return Ok(ProcessEventResult {
                        old_state,
                        next_state: None,
                        transition: None,
                        actions_executed: vec![],
                        events_published: vec![],
                    });
                }
                // RFC 4028 UPDATE for session-timer refresh carries no SDP,
                // but if a peer sends an UPDATE body (RFC 3311 session
                // modification), record it so a future transition with
                // NegotiateSDPAsUAS can act on it.
                session.remote_sdp = Some(sdp_data.clone());
                session.sdp_negotiated = false;
            }
            _ => {}
        }

        // 2. Build state key for lookup
        let key = StateKey {
            role: session.role,
            state: session.call_state,
            event: event.clone(),
        };

        // 3. Look up transition in table
        let transition = match self.table.get(&key) {
            Some(t) => t,
            None => {
                let event_name = state_machine_event_name(&event);
                debug!(
                    "No transition defined for role={:?}, state={:?}, event={}",
                    key.role, key.state, event_name
                );

                // Record failed transition attempt in history
                if rejected_history_enabled {
                    let now = Instant::now();
                    let record = TransitionRecord {
                        sequence: 0, // Will be set by history
                        timestamp: now,
                        timestamp_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        from_state: old_state,
                        event: history_event.clone(),
                        to_state: Some(old_state),
                        guards_evaluated: vec![],
                        actions_executed: vec![],
                        duration_ms: transition_start.elapsed().as_millis() as u64,
                        errors: vec![format!(
                            "No transition defined for role={:?}, state={:?}, event={}",
                            key.role, key.state, event_name
                        )],
                        events_published: vec![],
                    };
                    commit_rejected_transition_history(
                        &self.store,
                        pre_event_snapshot.as_ref(),
                        record,
                    )?;
                }

                return Ok(ProcessEventResult {
                    old_state,
                    next_state: None,
                    transition: None,
                    actions_executed: vec![],
                    events_published: vec![],
                });
            }
        };

        // 4. Check guards
        for guard in &transition.guards {
            let guard_start = Instant::now();
            let satisfied = guards::check_guard(guard, &session).await;
            let guard_duration = guard_start.elapsed().as_millis() as u64;

            guards_evaluated.push(GuardResult {
                guard: guard.clone(),
                passed: satisfied,
                evaluation_time_us: guard_duration * 1000,
            });

            if !satisfied {
                debug!("Guard {:?} not satisfied, skipping transition", guard);

                // Record guard failure in history
                if rejected_history_enabled {
                    let now = Instant::now();
                    let record = TransitionRecord {
                        sequence: 0,
                        timestamp: now,
                        timestamp_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        from_state: old_state,
                        event: history_event.clone(),
                        to_state: Some(old_state),
                        guards_evaluated,
                        actions_executed: vec![],
                        duration_ms: transition_start.elapsed().as_millis() as u64,
                        errors: vec![format!("Guard {:?} not satisfied", guard)],
                        events_published: vec![],
                    };
                    commit_rejected_transition_history(
                        &self.store,
                        pre_event_snapshot.as_ref(),
                        record,
                    )?;
                }

                return Ok(ProcessEventResult {
                    old_state,
                    next_state: None,
                    transition: None,
                    actions_executed: vec![],
                    events_published: vec![],
                });
            }
        }

        info!(
            "Executing transition for {:?} + {}",
            old_state,
            state_machine_event_name(&event)
        );

        // REFER-NOTIFY state is deliberately held out of the generic
        // pre-lookup input application. A missing row or failed guard must
        // neither mutate the exact lifetime nor release derived observations.
        // Once YAML admits the transition, the lane-owned input application
        // and canonical final publication own this working-state update;
        // ProcessNOTIFY remains the explicit no-op wiring marker.
        if let Some(input) = refer_notify_input {
            input.apply(&mut session);
        }

        // Apply next_state to the lane-owned working state before executing
        // actions. Follow-up events stay in this executor's private queue and
        // run only after this event publishes its complete result. A failed
        // action also publishes this working state exactly once before the
        // original error is returned, preserving partial-effect semantics
        // without exposing an intermediate full snapshot. A failure before
        // an ordered final-response action retains those partial preparations
        // but restores the pre-wire state identity so the same exact response
        // obligation can be retried.
        if let Some(new_state) = transition.next_state {
            info!("State transition: {:?} -> {:?}", old_state, new_state);
            session.call_state = new_state;
            session.entered_state_at = Instant::now();

            // SIP_API_DESIGN_2 §7.3 invariant #2 — final-state backstop.
            // Clear every pending-options slot unconditionally on entry
            // to any final state so a YAML row that forgets to emit the
            // matching `ClearPending*Options` action can never leave a
            // stash permanently occupied. The per-method clear actions
            // emitted on final-response transitions are the primary
            // mechanism; this is the safety net.
            if new_state.is_final() {
                session.clear_pending_request_state_for_final_transition();
            }
        }

        // 5. Execute actions
        let mut actions_executed = Vec::new();
        let mut deferred_effects = Vec::new();
        let mut terminal_exact_response_error = None;
        let mut terminal_exact_response_completed = false;
        for (action_index, action) in transition.actions.iter().enumerate() {
            if (terminal_exact_response_error.is_some() || terminal_exact_response_completed)
                && matches!(
                    action,
                    Action::SendSIPResponse(_, _)
                        | Action::SendRejectResponse
                        | Action::SendRedirectResponse
                        | Action::SendReferAccepted
                )
            {
                // A wire-unknown final response has consumed this event's
                // response authority. Continue deterministic cleanup, but
                // never let a duplicate response action author the wire.
                continue;
            }
            if self.should_skip_action(action) {
                continue;
            }
            #[cfg(test)]
            action_execution_barrier::pause_once(&session.session_id).await;
            let authors_final_response = action_authors_final_response(action, &session);
            let action_start = Instant::now();
            let result = Box::pin(actions::execute_action(
                action,
                &event,
                &mut session,
                (&self.dialog_adapter, &self.media_adapter),
                stage_claim.map(Arc::as_ref),
                inbound_response_input.as_mut(),
                invite_2xx_ack_input.as_ref(),
            ))
            .await;
            let action_duration = action_start.elapsed().as_millis() as u64;

            let (success, error_opt, exec_error) = match result {
                Ok(outcome) => {
                    actions_executed.push(action.clone());
                    queued_follow_up_events.extend(outcome.follow_up_events);
                    deferred_effects.extend(outcome.deferred_effects);
                    if authors_final_response {
                        terminal_exact_response_completed = true;
                    }
                    (true, None, None)
                }
                Err(e) => {
                    let action_class = action_diagnostic_class(action);
                    let error_class = action_error_diagnostic_class(e.as_ref());
                    let error_msg =
                        format!("action failed (action={action_class}, class={error_class})");
                    if is_missing_credentials_for_auth_error(e.as_ref()) {
                        debug!(
                            action = action_class,
                            error_class, "State-machine action failed"
                        );
                    } else {
                        error!(
                            action = action_class,
                            error_class, "State-machine action failed"
                        );
                    }
                    errors.push(error_msg.clone());
                    (false, Some(error_msg), Some(e))
                }
            };

            actions_executed_history.push(ActionRecord {
                action: action.clone(),
                success,
                execution_time_us: action_duration * 1000,
                error: error_opt,
            });

            if !success {
                let exact_response_disposition = exec_error.as_ref().and_then(|error| {
                    actions::exact_sip_response_failure_disposition(error.as_ref())
                });
                if exact_response_disposition
                    == Some(
                        rvoip_sip_dialog::FinalResponseCompletionDisposition::WireUnknownErrorTerminal,
                    )
                {
                    // Transaction-core has made final-response retry unsafe,
                    // but this YAML transition still owns deterministic local
                    // teardown. Retain the uncertainty for the caller and run
                    // every remaining ordered cleanup action before the one
                    // canonical state publication.
                    terminal_exact_response_error = exec_error;
                    continue;
                }
                let zero_wire_exact_response = exact_response_disposition
                    == Some(
                        rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable,
                    );
                let failed_before_ordered_final_response = terminal_exact_response_error.is_none()
                    && !terminal_exact_response_completed
                    && !authors_final_response
                    && transition.actions[action_index + 1..].iter().any(|later| {
                        !self.should_skip_action(later)
                            && action_authors_final_response(later, &session)
                    });
                let retryable_pre_wire_failure =
                    zero_wire_exact_response || failed_before_ordered_final_response;
                if retryable_pre_wire_failure {
                    // Pre-wire preparation (for example generated SDP/media)
                    // remains lane-owned and retryable, but the YAML state
                    // transition is a post-wire fact. Restore its pre-event
                    // identity while retaining the exact transaction, timing,
                    // status override, and response headers that the action
                    // deliberately left untouched.
                    session.call_state = old_state;
                    session.entered_state_at = pre_event_snapshot.state().entered_state_at;
                }
                if response_input_active && !retryable_pre_wire_failure {
                    session.clear_pending_response_input();
                }
                // Record failed action in history
                if session.history_recording_enabled() {
                    let now = Instant::now();
                    let record = TransitionRecord {
                        sequence: 0,
                        timestamp: now,
                        timestamp_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        from_state: old_state,
                        event: history_event.clone(),
                        to_state: Some(session.call_state),
                        guards_evaluated,
                        actions_executed: actions_executed_history,
                        duration_ms: transition_start.elapsed().as_millis() as u64,
                        errors,
                        events_published: vec![],
                    };
                    session.record_transition(record);
                }

                // The lane-owned working state is authoritative even when the
                // optional diagnostic history is disabled. Pre-wire failures
                // publish a retry envelope in the pre-wire call state;
                // terminal and unrelated failures retain the established
                // partial-effect semantics.
                let media_security_observation = (pre_event_snapshot.media_security
                    != session.media_security)
                    .then(|| {
                        session
                            .media_security
                            .clone()
                            .map(|state| ((*handle).clone(), state))
                    })
                    .flatten();
                commit_lane_state(&self.store, session)?;
                if let Some((lifecycle_handle, state)) = media_security_observation {
                    self.media_adapter
                        .publish_media_security_observation(lifecycle_handle, state);
                }

                return Err(exec_error.unwrap());
            }
        }

        // Response envelopes are single-event inputs. Actions normally take
        // them at the wire boundary; this finalizer also covers customized
        // YAML rows with no response action and failures in an earlier action.
        if response_input_active {
            session.clear_pending_response_input();
        }

        // A successful BYE or CANCEL can synchronously receive the peer's
        // terminal response, and a successful REFER can synchronously complete
        // the replacement and terminate the original dialog. Dialog-core then
        // publishes the terminal event while this dispatch transition is still
        // unwinding; that path may quiesce and remove the exact session before
        // the ordinary save/reload steps below. Never resurrect the stale local
        // snapshot. Keep the REFER exception narrower than teardown: its sole
        // action must have succeeded and its row must be state-, condition-,
        // and event-neutral.
        if is_exact_retirement_safe_dispatch_only_transition(transition)
            && self.exact_lifetime_is_no_longer_current(&session)
        {
            debug!(
                session_id = %session_id,
                "terminal confirmation retired the exact session during outbound dispatch"
            );
            return Ok(completed_transition_result(
                old_state,
                transition,
                actions_executed,
            ));
        }

        // 6. Record successful transition in history (state already applied
        // above, before the action loop)
        let next_state = transition.next_state;
        if session.history_recording_enabled() {
            let now = Instant::now();
            let record = TransitionRecord {
                sequence: 0,
                timestamp: now,
                timestamp_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                from_state: old_state,
                event: history_event,
                to_state: next_state,
                guards_evaluated,
                actions_executed: actions_executed_history,
                duration_ms: transition_start.elapsed().as_millis() as u64,
                errors,
                events_published: transition.publish_events.clone(),
            };
            session.record_transition(record);
        }

        // 7. Apply condition updates
        session.apply_condition_updates(&transition.condition_updates);

        let media_security_observation = (pre_event_snapshot.media_security
            != session.media_security)
            .then(|| {
                session
                    .media_security
                    .clone()
                    .map(|state| ((*handle).clone(), state))
            })
            .flatten();

        // 8. Move the complete event-local state into the store exactly once
        // and retain the immutable revision that was published.
        let lifecycle_handle = session.lifecycle_handle.clone();
        let published = match commit_lane_state(&self.store, session) {
            Ok(published) => published,
            Err(error) => {
                if is_exact_retirement_safe_dispatch_only_transition(transition)
                    && self.exact_handle_is_no_longer_current(lifecycle_handle.as_ref())
                {
                    debug!(
                        session_id = %session_id,
                        "terminal confirmation won the outbound-dispatch exact-session save race"
                    );
                    return Ok(completed_transition_result(
                        old_state,
                        transition,
                        actions_executed,
                    ));
                }
                return Err(error);
            }
        };
        let session = published.state();

        if let Some((lifecycle_handle, state)) = media_security_observation {
            self.media_adapter
                .publish_media_security_observation(lifecycle_handle, state);
        }

        // Post-commit effects are admitted only after the complete transition
        // state is visible. Transfer observations and registration callbacks
        // therefore cannot report an uncommitted outcome; retained retry tasks
        // sleep outside this lane and reacquire the exact lifetime before I/O.
        self.schedule_deferred_action_effects(handle, deferred_effects)?;

        // 9. Publish observational compatibility events after the committed
        // state is visible. Observer pressure must never hold the exact
        // signaling lane or change wire/lifecycle progress.
        if let Some(ref event_tx) = self.event_tx {
            for event_template in &transition.publish_events {
                if let Some(publisher) = self.exact_api_event_publisher.get() {
                    let exact_api_event = match event_template {
                        EventTemplate::CallOnHold => Some(crate::api::events::Event::CallOnHold {
                            call_id: session.session_id.clone(),
                        }),
                        EventTemplate::CallResumed => {
                            Some(crate::api::events::Event::CallResumed {
                                call_id: session.session_id.clone(),
                            })
                        }
                        _ => None,
                    };
                    if let Some(exact_api_event) = exact_api_event {
                        publisher.publish_exact(handle, exact_api_event);
                        continue;
                    }
                }
                let event = self
                    .instantiate_event(event_template, session, old_state)
                    .await;
                let guard = cleanup_diag::stage_guard(
                    CleanupStage::StateMachineEventPublish,
                    &session.session_id.0,
                );
                match publish_observational_event(event_tx, event) {
                    ObservationPublishOutcome::Published => guard.finish_success(),
                    ObservationPublishOutcome::Saturated => {
                        guard.finish_failure();
                        debug!(
                            session_id = %session.session_id,
                            "dropping saturated observational state-machine event"
                        );
                    }
                    ObservationPublishOutcome::Closed => {
                        guard.finish_failure();
                        debug!(
                            session_id = %session.session_id,
                            "observational state-machine event receiver is closed"
                        );
                    }
                }
            }
        }

        // 10. The returned publication is the exact state just committed, so
        // readiness checks need neither a map lookup nor an owned reload.
        let all_conditions_met = session.all_conditions_met();
        let call_established_triggered = session.call_established_triggered;

        // 11. Check if conditions trigger internal events
        // 12. Trigger internal events after saving
        if all_conditions_met && !call_established_triggered {
            debug!("All conditions met, triggering InternalCheckReady");
            queued_follow_up_events.push_back(EventType::InternalCheckReady);
        }

        if let Some(error) = terminal_exact_response_error {
            return Err(error);
        }

        Ok(ProcessEventResult {
            old_state,
            next_state: transition.next_state,
            transition: Some(transition.clone()),
            actions_executed,
            events_published: transition.publish_events.clone(),
        })
    }

    fn should_skip_action(&self, action: &Action) -> bool {
        matches!(action, Action::SendSIPResponse(180, _)) && !self.auto_180_ringing
    }

    fn exact_lifetime_is_no_longer_current(&self, session: &SessionState) -> bool {
        self.exact_handle_is_no_longer_current(session.lifecycle_handle.as_ref())
    }

    fn exact_handle_is_no_longer_current(
        &self,
        handle: Option<&crate::session_registry::SessionRegistryHandle>,
    ) -> bool {
        let Some(handle) = handle else {
            return false;
        };
        !self.store.authority().is_current(handle.key())
    }

    /// Convert event template to concrete event
    async fn instantiate_event(
        &self,
        template: &EventTemplate,
        session: &SessionState,
        old_state: CallState,
    ) -> SessionEvent {
        match template {
            EventTemplate::StateChanged => SessionEvent::StateChanged {
                session_id: session.session_id.clone(),
                old_state,
                new_state: session.call_state,
            },
            EventTemplate::MediaFlowEstablished => {
                let negotiated = session.negotiated_config.as_ref();
                SessionEvent::MediaFlowEstablished {
                    session_id: session.session_id.clone(),
                    local_addr: negotiated
                        .map(|n| n.local_addr.to_string())
                        .unwrap_or_default(),
                    remote_addr: negotiated
                        .map(|n| n.remote_addr.to_string())
                        .unwrap_or_default(),
                    direction: crate::state_table::MediaFlowDirection::Both,
                }
            }
            EventTemplate::CallEstablished => SessionEvent::CallEstablished {
                session_id: session.session_id.clone(),
            },
            EventTemplate::CallTerminated => SessionEvent::CallTerminated {
                session_id: session.session_id.clone(),
            },
            EventTemplate::CallCancelled => SessionEvent::CallCancelled {
                session_id: session.session_id.clone(),
            },
            EventTemplate::CallOnHold => SessionEvent::CallOnHold {
                session_id: session.session_id.clone(),
            },
            EventTemplate::CallResumed => SessionEvent::CallResumed {
                session_id: session.session_id.clone(),
            },
            EventTemplate::Custom(event) => SessionEvent::Custom {
                session_id: session.session_id.clone(),
                event: event.clone(),
            },
            _ => SessionEvent::Custom {
                session_id: session.session_id.clone(),
                event: format!("{:?}", template),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_store::{HistoryConfig, SessionHistory};
    use crate::state_table::{Condition, ConditionUpdates, Guard, MasterStateTable, Role};
    use std::time::Instant;

    #[test]
    fn session_refresh_input_rejects_stale_generations_and_wrong_phases() {
        use crate::session_store::state::SessionRefreshPhase;

        let mut session = SessionState::new(SessionId("refresh-capability".to_string()), Role::UAC);
        session.session_refresh_timer_generation = 7;
        session.session_refresh_interval_secs = Some(120);
        session.session_refresh_local_refresher = true;
        session.session_refresh_phase = SessionRefreshPhase::Idle;

        assert!(SessionRefreshStateInput::UpdateDue {
            timer_generation: 7
        }
        .validate(&session)
        .is_ok());
        assert!(SessionRefreshStateInput::UpdateDue {
            timer_generation: 6
        }
        .validate(&session)
        .is_err());
        assert!(SessionRefreshStateInput::PeerExpired {
            timer_generation: 7
        }
        .validate(&session)
        .is_err());

        session.session_refresh_phase = SessionRefreshPhase::UpdateInFlight;
        assert!(SessionRefreshStateInput::UpdateSucceeded
            .validate(&session)
            .is_ok());
        assert!(SessionRefreshStateInput::ReinviteSucceeded
            .validate(&session)
            .is_err());

        session.session_refresh_phase = SessionRefreshPhase::Idle;
        session.session_refresh_local_refresher = false;
        assert!(SessionRefreshStateInput::PeerExpired {
            timer_generation: 7
        }
        .validate(&session)
        .is_ok());
        assert!(SessionRefreshStateInput::ReinviteDue {
            timer_generation: 7
        }
        .validate(&session)
        .is_err());
    }

    async fn input_admission_coordinator(
        name: &str,
    ) -> Arc<crate::api::unified::UnifiedCoordinator> {
        crate::api::unified::UnifiedCoordinator::new(crate::api::unified::Config::local(name, 0))
            .await
            .expect("create input-admission coordinator")
    }

    fn state_machine_with_table(
        coordinator: &Arc<crate::api::unified::UnifiedCoordinator>,
        table: MasterStateTable,
    ) -> Arc<StateMachine> {
        let baseline = &coordinator.helpers.state_machine;
        Arc::new(StateMachine::new(
            Arc::new(table),
            Arc::clone(&baseline.store),
            Arc::clone(&baseline.dialog_adapter),
            Arc::clone(&baseline.media_adapter),
        ))
    }

    async fn create_input_admission_session(
        machine: &StateMachine,
        name: &str,
        history: Option<HistoryConfig>,
    ) -> (SessionId, SessionRegistryHandle) {
        let session_id = SessionId(name.to_string());
        machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAC, false, move |session| {
                if let Some(config) = history {
                    session.history = Some(SessionHistory::new(config));
                }
            })
            .await
            .expect("create input-admission session");
        let handle = machine
            .store
            .lifecycle_handle(&session_id)
            .expect("capture input-admission exact handle");
        (session_id, handle)
    }

    #[tokio::test]
    async fn uas_create_dialog_adopts_the_exact_inbound_registry_owner() {
        let coordinator = input_admission_coordinator("uas-dialog-adoption").await;
        let incoming = EventType::IncomingCall {
            from: "sip:alice@example.test".to_string(),
            sdp: None,
        };
        let mut table = MasterStateTable::new();
        table.insert(
            StateKey {
                role: Role::UAS,
                state: CallState::Idle,
                event: incoming.clone(),
            },
            Transition {
                guards: Vec::new(),
                actions: vec![Action::CreateDialog],
                next_state: Some(CallState::Ringing),
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
        let machine = state_machine_with_table(&coordinator, table);
        let session_id = SessionId("uas-dialog-adoption-session".to_string());
        machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAS, false, |session| {
                session.local_uri = Some("sip:bob@example.test".to_string());
                session.remote_uri = Some("sip:alice@example.test".to_string());
            })
            .await
            .expect("create inbound UAS lifetime");
        let handle = machine
            .store
            .lifecycle_handle(&session_id)
            .expect("capture inbound UAS lifetime");
        let dialog_id = crate::types::DialogId::new();
        machine
            .store
            .registry()
            .install_dialog_identity_handle(
                &handle,
                dialog_id,
                "uas-dialog-adoption-wire-call".to_string(),
            )
            .expect("install exact inbound dialog owner");

        machine
            .process_event_exact(&handle, incoming)
            .await
            .expect("process inbound UAS CreateDialog");
        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read committed inbound UAS state");
        assert_eq!(committed.dialog_id.as_ref(), Some(&dialog_id));
        assert_eq!(
            machine.store.registry().get_dialog_handle_exact(&handle),
            Some(dialog_id)
        );

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown UAS dialog-adoption coordinator");
    }

    #[tokio::test]
    async fn public_media_event_cannot_enter_reserved_session_refresh_rows() {
        let coordinator = input_admission_coordinator("reserved-refresh-event").await;
        let machine = Arc::clone(&coordinator.helpers.state_machine);
        let session_id = SessionId("reserved-refresh-event-session".to_string());
        machine
            .store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create session for reserved event test");

        let error = machine
            .process_event(
                &session_id,
                EventType::MediaEvent(SESSION_REFRESH_DUE_EVENT.to_string()),
            )
            .await
            .expect_err("a public string must not act as an RFC 4028 capability");
        assert!(matches!(
            error.downcast_ref::<crate::errors::SessionError>(),
            Some(crate::errors::SessionError::InvalidTransition(_))
        ));

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown reserved event coordinator");
    }

    #[tokio::test]
    async fn public_media_event_cannot_spoof_confirmed_negotiation_failure() {
        let coordinator = input_admission_coordinator("reserved-negotiation-failure").await;
        let event = EventType::MediaEvent(
            crate::state_table::types::CONFIRMED_NEGOTIATION_FAILURE_EVENT.to_string(),
        );
        let mut table = MasterStateTable::new();
        table.insert(
            StateKey {
                role: Role::UAC,
                state: CallState::Active,
                event: event.clone(),
            },
            Transition {
                guards: Vec::new(),
                actions: Vec::new(),
                next_state: Some(CallState::OnHold),
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
        let machine = state_machine_with_table(&coordinator, table);
        let session_id = SessionId("reserved-negotiation-failure-session".to_string());
        machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAC, false, |session| {
                session.call_state = CallState::Active;
            })
            .await
            .expect("create confirmed-negotiation failure session");
        let handle = machine
            .store
            .lifecycle_handle(&session_id)
            .expect("capture confirmed-negotiation failure handle");
        let initial = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read initial state");

        let error = machine
            .process_event_exact(&handle, event)
            .await
            .expect_err("a public string must not act as a negotiation-failure capability");
        assert!(matches!(
            error.downcast_ref::<crate::errors::SessionError>(),
            Some(crate::errors::SessionError::InvalidTransition(_))
        ));
        let rejected = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read rejected state");
        assert_eq!(rejected.call_state, CallState::Active);
        assert_eq!(rejected.revision(), initial.revision());

        let committed = machine
            .process_confirmed_negotiation_failure_exact(&handle)
            .await
            .expect("typed negotiation-failure capability");
        assert_eq!(committed.next_state, Some(CallState::OnHold));
        let accepted = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read accepted state");
        assert_eq!(accepted.call_state, CallState::OnHold);
        assert!(accepted.revision() > rejected.revision());

        let repeated = machine
            .process_confirmed_negotiation_failure_exact(&handle)
            .await
            .expect("repeated typed signal is an idempotent no-op");
        assert!(repeated.transition.is_none());
        let after_repeat = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read state after repeated typed signal");
        assert_eq!(after_repeat.call_state, CallState::OnHold);
        assert_eq!(after_repeat.revision(), accepted.revision());

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown reserved negotiation-failure coordinator");
    }

    #[test]
    fn session_refresh_deadlines_are_exact_off_lane_and_typed() {
        let source = include_str!("executor.rs");
        let runner = source
            .split("async fn run_deferred_session_refresh")
            .nth(1)
            .and_then(|tail| tail.split("fn reinvite_retry_matches").next())
            .expect("RFC 4028 deferred runner");

        assert!(runner.contains("tokio::time::sleep(effect.delay)"));
        assert!(runner.contains("input.validate(current.state())"));
        assert!(runner.contains("process_session_refresh_exact(&handle, input)"));
        assert!(!runner.contains("state_machine_lane_exact"));

        let scheduler = source
            .split("fn schedule_deferred_action_effects")
            .nth(1)
            .and_then(|tail| tail.split("// Callback registry removed").next())
            .expect("post-commit effect scheduler");
        assert!(scheduler.contains("DeferredActionEffect::SessionRefreshTimer"));
        assert!(scheduler.contains("spawn_owned_exact"));
        assert!(scheduler.contains("SessionOperationKind::Signaling"));
    }

    #[test]
    fn auth_retry_observation_is_post_commit_and_inline() {
        let source = include_str!("executor.rs");
        let transition = source
            .split("async fn process_one_event")
            .nth(1)
            .and_then(|tail| tail.split("fn should_skip_action").next())
            .expect("single-event canonical commit source");
        let commit = transition
            .find("let published = match commit_lane_state(&self.store, session)")
            .expect("canonical exact-state publication");
        let schedule = transition
            .find("self.schedule_deferred_action_effects")
            .expect("post-commit effect scheduler");
        assert!(commit < schedule);

        let scheduler = source
            .split("fn schedule_deferred_action_effects")
            .nth(1)
            .and_then(|tail| tail.split("DeferredActionEffect::TransferNotify").next())
            .expect("API observation scheduler arm");
        assert!(scheduler.contains("DeferredActionEffect::AuthRetryObservation"));
        assert!(scheduler.contains("dialog_adapter.publish_api_event_exact(handle, event)"));
        assert!(scheduler.contains("publish_diagnostic_event_exact"));
        assert!(!scheduler.contains("spawn"));
        assert!(!scheduler.contains("await"));
    }

    fn accept_call_table(
        guards: Vec<Guard>,
        actions: Vec<Action>,
        next_state: Option<CallState>,
    ) -> MasterStateTable {
        let mut table = MasterStateTable::new();
        table.insert(
            StateKey {
                role: Role::UAC,
                state: CallState::Idle,
                event: EventType::AcceptCall,
            },
            Transition {
                guards,
                actions,
                next_state,
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
        table
    }

    fn uas_initial_invite_response_table() -> MasterStateTable {
        let mut table = MasterStateTable::new();
        for (event, actions, next_state) in [
            (
                EventType::AcceptCall,
                vec![Action::SendSIPResponse(200, "OK".to_string())],
                CallState::Answering,
            ),
            (
                EventType::RejectCall {
                    status: 0,
                    reason: String::new(),
                },
                vec![
                    Action::SendRejectResponse,
                    Action::CleanupDialog,
                    Action::CleanupMedia,
                ],
                CallState::Terminated,
            ),
            (
                EventType::RedirectCall {
                    status: 0,
                    contacts: Vec::new(),
                },
                vec![
                    Action::SendRedirectResponse,
                    Action::CleanupDialog,
                    Action::CleanupMedia,
                ],
                CallState::Terminated,
            ),
        ] {
            table.insert(
                StateKey {
                    role: Role::UAS,
                    state: CallState::Ringing,
                    event,
                },
                Transition {
                    guards: Vec::new(),
                    actions,
                    next_state: Some(next_state),
                    condition_updates: ConditionUpdates::none(),
                    publish_events: Vec::new(),
                },
            );
        }
        table
    }

    fn uas_initial_invite_provisional_table() -> MasterStateTable {
        let mut table = MasterStateTable::new();
        table.insert(
            StateKey {
                role: Role::UAS,
                state: CallState::Idle,
                event: EventType::IncomingCall {
                    from: "sip:alice@example.test".to_string(),
                    sdp: None,
                },
            },
            Transition {
                guards: Vec::new(),
                actions: vec![Action::SendSIPResponse(180, "Ringing".to_string())],
                next_state: Some(CallState::Ringing),
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
        table.insert(
            StateKey {
                role: Role::UAS,
                state: CallState::Ringing,
                event: EventType::SendEarlyMedia { sdp: None },
            },
            Transition {
                guards: Vec::new(),
                actions: vec![Action::SendSIPResponse(183, "Session Progress".to_string())],
                next_state: Some(CallState::EarlyMedia),
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
        table
    }

    async fn create_retryable_uas_invite_session(
        machine: &StateMachine,
        name: &str,
    ) -> (
        SessionId,
        SessionRegistryHandle,
        rvoip_sip_dialog::transaction::TransactionKey,
        Instant,
    ) {
        let session_id = SessionId(name.to_string());
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            format!("z9hG4bK-{name}"),
            rvoip_sip_core::Method::Invite,
            true,
        );
        let received_at = Instant::now();
        let transaction_for_state = transaction.clone();
        machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAS, false, move |session| {
                session.call_state = CallState::Ringing;
                session.local_sdp = Some("v=0\r\na=x-retryable-answer\r\n".to_string());
                session.pending_inbound_invite_transaction_id = Some(transaction_for_state);
                session.incoming_invite_received_at = Some(received_at);
            })
            .await
            .expect("create retryable UAS INVITE session");
        let handle = machine
            .store
            .lifecycle_handle(&session_id)
            .expect("capture retryable UAS exact handle");
        (session_id, handle, transaction, received_at)
    }

    fn inbound_final_response_table(
        state: CallState,
        event: EventType,
        next_state: CallState,
        actions: Vec<Action>,
    ) -> MasterStateTable {
        let mut table = MasterStateTable::new();
        table.insert(
            StateKey {
                role: Role::UAC,
                state,
                event,
            },
            Transition {
                guards: Vec::new(),
                actions,
                next_state: Some(next_state),
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
        table
    }

    fn inbound_refer_event(transaction_id: &str) -> EventType {
        EventType::TransferRequested {
            refer_to: "sip:carol@example.test".to_string(),
            transfer_type: "blind".to_string(),
            transaction_id: transaction_id.to_string(),
        }
    }

    fn inbound_refer_response_table(event: EventType, actions: Vec<Action>) -> MasterStateTable {
        let mut table = MasterStateTable::new();
        table.insert(
            StateKey {
                role: Role::UAC,
                state: CallState::Active,
                event,
            },
            Transition {
                guards: Vec::new(),
                actions,
                next_state: Some(CallState::Transferring),
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
        table
    }

    async fn create_inbound_refer_session(
        machine: &StateMachine,
        name: &str,
        transaction_id: String,
    ) -> (SessionId, SessionRegistryHandle) {
        let session_id = SessionId(name.to_string());
        machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAC, false, move |session| {
                session.call_state = CallState::Active;
                session.refer_transaction_id = Some(transaction_id);
            })
            .await
            .expect("create inbound REFER response session");
        let handle = machine
            .store
            .lifecycle_handle(&session_id)
            .expect("capture inbound REFER lifecycle");
        (session_id, handle)
    }

    fn insert_inbound_final_response_transition(table: &mut MasterStateTable, event: EventType) {
        table.insert(
            StateKey {
                role: Role::UAC,
                state: CallState::Active,
                event,
            },
            Transition {
                guards: Vec::new(),
                actions: vec![Action::SendSIPResponse(200, "OK".to_string())],
                next_state: Some(CallState::Active),
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
    }

    async fn create_inbound_response_session(
        machine: &StateMachine,
        name: &str,
        state: CallState,
        pending_reinvite: Option<crate::session_store::state::PendingReinvite>,
    ) -> (SessionId, SessionRegistryHandle) {
        let session_id = SessionId(name.to_string());
        machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAC, false, move |session| {
                session.call_state = state;
                session.local_sdp = Some("v=0\r\na=x-in-dialog-answer\r\n".to_string());
                session.pending_reinvite = pending_reinvite;
            })
            .await
            .expect("create inbound-response session");
        let handle = machine
            .store
            .lifecycle_handle(&session_id)
            .expect("capture inbound-response exact handle");
        (session_id, handle)
    }

    fn inbound_response_input(
        method: &str,
        branch: &str,
    ) -> (
        InboundResponseStateInput,
        rvoip_sip_dialog::transaction::TransactionKey,
    ) {
        let raw = format!(
            "{method} sip:bob@example.invalid SIP/2.0\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5060;branch={branch}\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:alice@example.invalid>;tag=alice-tag\r\n\
             To: <sip:bob@example.invalid>;tag=bob-tag\r\n\
             Call-ID: inbound-response@example.invalid\r\n\
             CSeq: 2 {method}\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let request = match rvoip_sip_core::parse_message(raw.as_bytes())
            .expect("parse inbound in-dialog request")
        {
            rvoip_sip_core::Message::Request(request) => request,
            rvoip_sip_core::Message::Response(_) => panic!("parsed request as response"),
        };
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::from_request(&request)
            .expect("derive inbound server transaction");
        let input = InboundResponseStateInput::from_request(method, &request)
            .expect("construct exact inbound response authority");
        (input, transaction)
    }

    async fn staged_info_guard(
        name: &str,
    ) -> (
        Arc<SessionStore>,
        SessionId,
        SessionRegistryHandle,
        PendingOptionsStageGuard,
        Arc<rvoip_sip_dialog::api::unified::InfoRequestOptions>,
    ) {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId(name.to_string());
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create exact session lifetime");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("current exact session handle");
        let options = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions::default());
        let slot = PendingOptionsSlot::Info(Arc::clone(&options));
        store
            .update_session_exact_with(&handle, None, |session| {
                slot.clone().stage_if_vacant(session)
            })
            .expect("stage exact session revision")
            .expect("INFO staging slot is vacant");
        let guard = PendingOptionsStageGuard::new(Arc::clone(&store), handle.clone(), slot);
        (store, session_id, handle, guard, options)
    }

    async fn retry_fixture(
        name: &str,
        kind: crate::session_store::state::PendingReinvite,
        attempt: u8,
    ) -> (Arc<SessionStore>, SessionRegistryHandle) {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId(name.to_string());
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create exact retry session");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("retry session exact handle");
        store
            .update_session_exact_with(&handle, None, |session| {
                session.pending_reinvite = Some(kind);
                session.reinvite_retry_attempts = attempt;
            })
            .expect("publish retry intent");
        (store, handle)
    }

    fn refer_dispatch_transition() -> Transition {
        Transition {
            guards: Vec::new(),
            actions: vec![Action::SendREFERWithOptions],
            next_state: None,
            condition_updates: ConditionUpdates::none(),
            publish_events: Vec::new(),
        }
    }

    #[tokio::test]
    async fn rejected_typed_input_is_history_invariant() {
        let coordinator = input_admission_coordinator("rejected-input-history").await;
        let machine = state_machine_with_table(&coordinator, MasterStateTable::new());

        let paused = HistoryConfig {
            enabled: false,
            ..Default::default()
        };
        for (name, history, expect_record) in [
            ("history-enabled", Some(HistoryConfig::default()), true),
            ("history-absent", None, false),
            ("history-paused", Some(paused), false),
        ] {
            let (session_id, handle) =
                create_input_admission_session(&machine, name, history).await;
            let before = machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read pre-event revision");

            let result = machine
                .process_event_with_local_sdp(
                    &session_id,
                    EventType::AcceptCall,
                    "v=0\r\na=x-rejected\r\n".to_string(),
                )
                .await
                .expect("missing row remains a non-transition result");
            assert!(result.transition.is_none());

            let after = machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read post-event revision");
            assert!(after.local_sdp.is_none(), "{name}: rejected SDP leaked");
            assert!(!after.sdp_negotiated, "{name}: rejected flag leaked");
            assert_eq!(after.call_state, CallState::Idle);
            assert_eq!(
                after.revision(),
                before.revision() + u64::from(expect_record),
                "{name}: only an enabled rejection record may publish"
            );
            let records = after
                .history
                .as_ref()
                .map(|history| history.get_recent(1))
                .unwrap_or_default();
            assert_eq!(records.len(), usize::from(expect_record), "{name}");
            if let Some(record) = records.first() {
                assert_eq!(record.from_state, CallState::Idle);
                assert_eq!(record.to_state, Some(CallState::Idle));
                assert!(!record.errors.is_empty());
            }
        }

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown rejected-input coordinator");
    }

    #[tokio::test]
    async fn failed_guard_records_preimage_while_typed_input_remains_guard_visible() {
        let coordinator = input_admission_coordinator("guard-input-admission").await;
        let rejecting = state_machine_with_table(
            &coordinator,
            accept_call_table(
                vec![Guard::HasRemoteSDP],
                Vec::new(),
                Some(CallState::Active),
            ),
        );
        let tracked = HistoryConfig {
            track_guards: true,
            ..Default::default()
        };
        let (rejected_id, rejected_handle) =
            create_input_admission_session(&rejecting, "guard-rejected", Some(tracked)).await;
        let rejected_before = rejecting
            .store
            .get_session_snapshot_exact(&rejected_handle)
            .expect("read guard preimage");

        let rejected = rejecting
            .process_event_with_local_sdp(
                &rejected_id,
                EventType::AcceptCall,
                "v=0\r\na=x-guard-rejected\r\n".to_string(),
            )
            .await
            .expect("guard rejection remains a non-transition result");
        assert!(rejected.transition.is_none());
        let rejected_after = rejecting
            .store
            .get_session_snapshot_exact(&rejected_handle)
            .expect("read guard rejection");
        assert_eq!(rejected_after.revision(), rejected_before.revision() + 1);
        assert!(rejected_after.local_sdp.is_none());
        assert!(!rejected_after.sdp_negotiated);
        assert_eq!(rejected_after.call_state, CallState::Idle);
        let rejection = rejected_after
            .history
            .as_ref()
            .expect("guard history exists")
            .get_recent(1)
            .pop()
            .expect("guard rejection recorded");
        assert_eq!(rejection.guards_evaluated.len(), 1);
        assert!(!rejection.guards_evaluated[0].passed);

        let admitting = state_machine_with_table(
            &coordinator,
            accept_call_table(
                vec![Guard::HasLocalSDP],
                Vec::new(),
                Some(CallState::Active),
            ),
        );
        let (admitted_id, admitted_handle) =
            create_input_admission_session(&admitting, "guard-admitted", None).await;
        let admitted = admitting
            .process_event_with_local_sdp(
                &admitted_id,
                EventType::AcceptCall,
                "v=0\r\na=x-guard-admitted\r\n".to_string(),
            )
            .await
            .expect("typed input satisfies YAML guard");
        assert!(admitted.transition.is_some());
        let admitted_after = admitting
            .store
            .get_session_snapshot_exact(&admitted_handle)
            .expect("read admitted typed input");
        assert_eq!(admitted_after.call_state, CallState::Active);
        assert_eq!(
            admitted_after.local_sdp.as_deref(),
            Some("v=0\r\na=x-guard-admitted\r\n")
        );
        assert!(admitted_after.sdp_negotiated);

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown guard-input coordinator");
    }

    #[tokio::test]
    async fn admitted_response_input_commits_state_but_not_a_reusable_envelope() {
        let coordinator = input_admission_coordinator("response-input-scope").await;
        let machine = state_machine_with_table(
            &coordinator,
            accept_call_table(Vec::new(), Vec::new(), Some(CallState::Active)),
        );
        let (session_id, handle) =
            create_input_admission_session(&machine, "response-input-session", None).await;

        let result = machine
            .process_event_with_response_input_exact(
                &handle,
                EventType::AcceptCall,
                ResponseStateInput::accept(
                    Some("v=0\r\na=x-response-input\r\n".to_string()),
                    Vec::new(),
                ),
            )
            .await
            .expect("admit exact response input");
        assert!(result.transition.is_some());

        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read committed response input");
        assert_eq!(committed.session_id, session_id);
        assert_eq!(committed.call_state, CallState::Active);
        assert_eq!(
            committed.local_sdp.as_deref(),
            Some("v=0\r\na=x-response-input\r\n")
        );
        assert!(committed.sdp_negotiated);
        assert!(committed.reject_response_extras.is_none());
        assert!(committed.pending_response_status_override.is_none());

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown response-input coordinator");
    }

    #[tokio::test]
    async fn rejected_event_payload_records_history_on_canonical_preimage() {
        let coordinator = input_admission_coordinator("rejected-event-payload").await;
        let machine = state_machine_with_table(&coordinator, MasterStateTable::new());
        let (session_id, handle) = create_input_admission_session(
            &machine,
            "rejected-reject-call",
            Some(HistoryConfig::default()),
        )
        .await;

        let result = machine
            .process_event(
                &session_id,
                EventType::RejectCall {
                    status: 486,
                    reason: "Busy Here".to_string(),
                },
            )
            .await
            .expect("missing RejectCall row remains a non-transition result");
        assert!(result.transition.is_none());
        let after = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read rejected payload state");
        assert!(after.reject_status.is_none());
        assert!(after.reject_reason.is_none());
        assert_eq!(
            after
                .history
                .as_ref()
                .expect("payload rejection history")
                .get_recent(1)
                .len(),
            1
        );

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown rejected-payload coordinator");
    }

    #[tokio::test]
    async fn rejected_auth_input_preserves_pre_event_coordination() {
        let coordinator = input_admission_coordinator("rejected-auth-input").await;
        let machine = state_machine_with_table(&coordinator, MasterStateTable::new());
        let (session_id, handle) = create_input_admission_session(
            &machine,
            "rejected-auth-required",
            Some(HistoryConfig::default()),
        )
        .await;
        machine
            .store
            .update_session_exact_with(&handle, None, |session| {
                session.pending_auth = Some((401, "prior challenge".to_string()));
                session.pending_auth_method = Some("INVITE".to_string());
                session.pending_auth_transaction_id = Some("prior-transaction".to_string());
                session.pending_auth_request_uri = Some("sip:prior@example.test".to_string());
            })
            .expect("publish prior auth coordination");

        let outcome = machine
            .process_auth_required_exact(
                &handle,
                407,
                "replacement challenge".to_string(),
                "BYE".to_string(),
                AuthRequiredStateInput::new(
                    None,
                    Some("replacement-transaction".to_string()),
                    Some("sip:replacement@example.test".to_string()),
                ),
            )
            .await
            .expect("process rejected exact auth input");
        let result = outcome
            .result
            .expect("missing auth row remains a non-transition result");
        assert!(result.transition.is_none());

        let after = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read rejected auth state");
        assert_eq!(after.session_id, session_id);
        assert_eq!(
            after.pending_auth.as_ref().map(|(status, _)| *status),
            Some(401)
        );
        assert_eq!(after.pending_auth_method.as_deref(), Some("INVITE"));
        assert_eq!(
            after.pending_auth_transaction_id.as_deref(),
            Some("prior-transaction")
        );
        assert_eq!(
            after.pending_auth_request_uri.as_deref(),
            Some("sip:prior@example.test")
        );
        assert_eq!(
            after
                .history
                .as_ref()
                .expect("auth rejection history")
                .get_recent(1)
                .len(),
            1
        );

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown rejected-auth coordinator");
    }

    #[tokio::test]
    async fn admitted_action_error_keeps_input_and_records_committed_state() {
        let coordinator = input_admission_coordinator("admitted-action-error").await;
        let machine = state_machine_with_table(
            &coordinator,
            accept_call_table(
                vec![Guard::HasLocalSDP],
                vec![Action::SendBYEWithOptions],
                Some(CallState::Terminating),
            ),
        );

        for (name, history) in [
            ("action-error-history", Some(HistoryConfig::default())),
            ("action-error-no-history", None),
        ] {
            let (_session_id, handle) =
                create_input_admission_session(&machine, name, history).await;
            assert!(machine
                .process_event_with_response_input_exact(
                    &handle,
                    EventType::AcceptCall,
                    ResponseStateInput {
                        local_sdp: Some(format!("v=0\r\na=x-{name}\r\n")),
                        sdp_negotiated: Some(true),
                        extra_headers: Vec::new(),
                        status_override: Some(181),
                    },
                )
                .await
                .is_err());
            let committed = machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read admitted action-error state");
            assert_eq!(committed.call_state, CallState::Terminating, "{name}");
            assert!(committed.local_sdp.is_some(), "{name}");
            assert!(committed.sdp_negotiated, "{name}");
            assert!(committed.reject_response_extras.is_none(), "{name}");
            assert!(
                committed.pending_response_status_override.is_none(),
                "{name}"
            );
            if let Some(history) = committed.history.as_ref() {
                let record = history
                    .get_recent(1)
                    .pop()
                    .expect("action error history record");
                assert_eq!(record.to_state, Some(CallState::Terminating));
                assert_eq!(record.actions_executed.len(), 1);
                assert!(!record.actions_executed[0].success);
            }
        }

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown action-error coordinator");
    }

    #[tokio::test]
    async fn initial_invite_180_and_183_use_one_captured_transaction_without_fallback() {
        use actions::exact_response_dispatch_test_hook::{self, Step};

        let coordinator = input_admission_coordinator("initial-provisional-exact").await;
        let machine =
            state_machine_with_table(&coordinator, uas_initial_invite_provisional_table());
        let session_id = SessionId("initial-provisional-exact-session".to_string());
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-initial-provisional-exact".to_string(),
            rvoip_sip_core::Method::Invite,
            true,
        );
        let retained_transaction = transaction.clone();
        machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAS, false, move |session| {
                session.pending_inbound_invite_transaction_id = Some(retained_transaction);
                session.incoming_invite_received_at = Some(Instant::now());
            })
            .await
            .expect("create exact provisional UAS session");
        let handle = machine
            .store
            .lifecycle_handle(&session_id)
            .expect("capture exact provisional lifecycle");
        let script = exact_response_dispatch_test_hook::install(
            &session_id,
            vec![Step::Written, Step::Written],
        );
        let inbound_response =
            InboundResponseStateInput::from_initial_invite_transaction(transaction.clone())
                .expect("capture initial INVITE response authority");

        machine
            .process_inbound_response_event_exact(
                &handle,
                EventType::IncomingCall {
                    from: "sip:alice@example.test".to_string(),
                    sdp: None,
                },
                inbound_response,
            )
            .await
            .expect("send exact 180 response");
        let ringing = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read ringing state");
        assert_eq!(ringing.call_state, CallState::Ringing);
        assert_eq!(
            ringing.pending_inbound_invite_transaction_id.as_ref(),
            Some(&transaction)
        );

        machine
            .process_event_exact(&handle, EventType::SendEarlyMedia { sdp: None })
            .await
            .expect("send exact 183 response");
        let early_media = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read early-media state");
        assert_eq!(early_media.call_state, CallState::EarlyMedia);
        assert_eq!(
            early_media.pending_inbound_invite_transaction_id.as_ref(),
            Some(&transaction)
        );
        assert_eq!(
            script.dispatches(),
            vec![(transaction.clone(), 180), (transaction, 183)]
        );
        assert_eq!(script.wire_authorships(), 2);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown exact provisional coordinator");
    }

    #[tokio::test]
    async fn refer_final_zero_wire_retains_exact_authority_for_retry() {
        use actions::exact_response_dispatch_test_hook::{self, Step};
        use rvoip_sip_dialog::FinalResponseCompletionDisposition;

        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-refer-zero-wire".to_string(),
            rvoip_sip_core::Method::Refer,
            true,
        );
        let transaction_text = transaction.to_string();
        let event = inbound_refer_event(&transaction_text);
        let coordinator = input_admission_coordinator("refer-zero-wire").await;
        let machine = state_machine_with_table(
            &coordinator,
            inbound_refer_response_table(event.clone(), vec![Action::SendReferAccepted]),
        );
        let (session_id, handle) = create_inbound_refer_session(
            machine.as_ref(),
            "refer-zero-wire-session",
            transaction_text.clone(),
        )
        .await;
        let script = exact_response_dispatch_test_hook::install(
            &session_id,
            vec![Step::ZeroWire, Step::Written],
        );

        let error = machine
            .process_event_exact(&handle, event.clone())
            .await
            .expect_err("zero-wire REFER acceptance must remain retryable");
        assert_eq!(
            actions::exact_sip_response_failure_disposition(error.as_ref()),
            Some(FinalResponseCompletionDisposition::ZeroWireRetryable)
        );
        let retryable = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read retryable REFER state");
        assert_eq!(retryable.call_state, CallState::Active);
        assert_eq!(
            retryable.refer_transaction_id.as_deref(),
            Some(transaction_text.as_str())
        );

        machine
            .process_event_exact(&handle, event)
            .await
            .expect("retry exact REFER acceptance");
        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read committed REFER acceptance");
        assert_eq!(committed.call_state, CallState::Transferring);
        assert!(committed.refer_transaction_id.is_none());
        assert_eq!(
            script.dispatches(),
            vec![(transaction.clone(), 202), (transaction, 202)]
        );
        assert_eq!(script.wire_authorships(), 1);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown REFER zero-wire coordinator");
    }

    #[tokio::test]
    async fn refer_final_wire_unknown_retires_authority_without_duplicate() {
        use actions::exact_response_dispatch_test_hook::{self, Step};
        use rvoip_sip_dialog::FinalResponseCompletionDisposition;

        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-refer-wire-unknown".to_string(),
            rvoip_sip_core::Method::Refer,
            true,
        );
        let transaction_text = transaction.to_string();
        let event = inbound_refer_event(&transaction_text);
        let coordinator = input_admission_coordinator("refer-wire-unknown").await;
        let machine = state_machine_with_table(
            &coordinator,
            inbound_refer_response_table(
                event.clone(),
                vec![Action::SendReferAccepted, Action::SendReferAccepted],
            ),
        );
        let (session_id, handle) = create_inbound_refer_session(
            machine.as_ref(),
            "refer-wire-unknown-session",
            transaction_text,
        )
        .await;
        let script =
            exact_response_dispatch_test_hook::install(&session_id, vec![Step::WireUnknown]);

        let error = machine
            .process_event_exact(&handle, event)
            .await
            .expect_err("REFER wire uncertainty remains caller-visible");
        assert_eq!(
            actions::exact_sip_response_failure_disposition(error.as_ref()),
            Some(FinalResponseCompletionDisposition::WireUnknownErrorTerminal)
        );
        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read terminal REFER uncertainty");
        assert_eq!(committed.call_state, CallState::Transferring);
        assert!(committed.refer_transaction_id.is_none());
        assert_eq!(script.dispatches(), vec![(transaction, 202)]);
        assert_eq!(script.attempts(), 1);
        assert_eq!(script.wire_authorships(), 1);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown REFER wire-unknown coordinator");
    }

    #[tokio::test]
    async fn cancelled_refer_waiter_observes_transaction_owned_completion() {
        use actions::exact_response_dispatch_test_hook::{self, OwnedCompletion, Step};

        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-refer-cancelled-waiter".to_string(),
            rvoip_sip_core::Method::Refer,
            true,
        );
        let transaction_text = transaction.to_string();
        let event = inbound_refer_event(&transaction_text);
        let coordinator = input_admission_coordinator("refer-cancelled-waiter").await;
        let machine = state_machine_with_table(
            &coordinator,
            inbound_refer_response_table(event.clone(), vec![Action::SendReferAccepted]),
        );
        let (session_id, handle) = create_inbound_refer_session(
            machine.as_ref(),
            "refer-cancelled-waiter-session",
            transaction_text,
        )
        .await;
        let owned = OwnedCompletion::new();
        let script = exact_response_dispatch_test_hook::install(
            &session_id,
            vec![
                Step::Owned(Arc::clone(&owned)),
                Step::Owned(Arc::clone(&owned)),
            ],
        );
        let before = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read cancelled REFER preimage");

        let in_flight = {
            let machine = Arc::clone(&machine);
            let handle = handle.clone();
            let event = event.clone();
            tokio::spawn(async move { machine.process_event_exact(&handle, event).await })
        };
        owned.wait_until_entered().await;
        in_flight.abort();
        assert!(in_flight
            .await
            .expect_err("cancelled REFER waiter must stop")
            .is_cancelled());
        let after_cancel = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read cancelled REFER state");
        assert_eq!(after_cancel.revision(), before.revision());
        assert!(after_cancel.refer_transaction_id.is_some());
        assert_eq!(script.wire_authorships(), 1);

        owned.complete_written();
        machine
            .process_event_exact(&handle, event)
            .await
            .expect("replacement REFER waiter observes owned completion");
        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read replacement REFER commit");
        assert_eq!(committed.call_state, CallState::Transferring);
        assert!(committed.refer_transaction_id.is_none());
        assert_eq!(
            script.dispatches(),
            vec![(transaction.clone(), 202), (transaction, 202)]
        );
        assert_eq!(script.attempts(), 2);
        assert_eq!(script.wire_authorships(), 1);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown cancelled REFER coordinator");
    }

    #[tokio::test]
    async fn reject_refer_zero_wire_retains_authority_until_exact_retry() {
        use actions::exact_response_dispatch_test_hook::{self, Step};
        use rvoip_sip_dialog::FinalResponseCompletionDisposition;

        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-reject-refer-zero-wire".to_string(),
            rvoip_sip_core::Method::Refer,
            true,
        );
        let transaction_text = transaction.to_string();
        let coordinator = input_admission_coordinator("reject-refer-zero-wire").await;
        let machine = state_machine_with_table(&coordinator, MasterStateTable::new());
        let (session_id, handle) = create_inbound_refer_session(
            machine.as_ref(),
            "reject-refer-zero-wire-session",
            transaction_text.clone(),
        )
        .await;
        machine
            .store
            .update_session_exact_with(&handle, None, |session| {
                session.transfer_target = Some("sip:carol@example.test".to_string());
            })
            .expect("stage rejected transfer target");
        let script = exact_response_dispatch_test_hook::install(
            &session_id,
            vec![Step::ZeroWire, Step::Written],
        );

        let error = machine
            .reject_refer_exact(&handle, 603)
            .await
            .expect_err("zero-wire REFER rejection must remain retryable");
        assert_eq!(
            actions::exact_sip_response_failure_disposition(error.as_ref()),
            Some(FinalResponseCompletionDisposition::ZeroWireRetryable)
        );
        let retryable = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read retryable REFER rejection");
        assert_eq!(
            retryable.refer_transaction_id.as_deref(),
            Some(transaction_text.as_str())
        );
        assert!(retryable.transfer_target.is_some());

        machine
            .reject_refer_exact(&handle, 603)
            .await
            .expect("retry exact REFER rejection");
        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read committed REFER rejection");
        assert!(committed.refer_transaction_id.is_none());
        assert!(committed.transfer_target.is_none());
        assert_eq!(
            script.dispatches(),
            vec![(transaction.clone(), 603), (transaction, 603)]
        );
        assert_eq!(script.wire_authorships(), 1);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown rejected REFER zero-wire coordinator");
    }

    #[tokio::test]
    async fn reject_refer_wire_unknown_retires_authority_without_retry() {
        use actions::exact_response_dispatch_test_hook::{self, Step};
        use rvoip_sip_dialog::FinalResponseCompletionDisposition;

        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-reject-refer-wire-unknown".to_string(),
            rvoip_sip_core::Method::Refer,
            true,
        );
        let transaction_text = transaction.to_string();
        let coordinator = input_admission_coordinator("reject-refer-wire-unknown").await;
        let machine = state_machine_with_table(&coordinator, MasterStateTable::new());
        let (session_id, handle) = create_inbound_refer_session(
            machine.as_ref(),
            "reject-refer-wire-unknown-session",
            transaction_text,
        )
        .await;
        let script =
            exact_response_dispatch_test_hook::install(&session_id, vec![Step::WireUnknown]);

        let error = machine
            .reject_refer_exact(&handle, 603)
            .await
            .expect_err("wire-unknown REFER rejection remains visible");
        assert_eq!(
            actions::exact_sip_response_failure_disposition(error.as_ref()),
            Some(FinalResponseCompletionDisposition::WireUnknownErrorTerminal)
        );
        assert!(machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read terminal REFER rejection")
            .refer_transaction_id
            .is_none());
        machine
            .reject_refer_exact(&handle, 603)
            .await
            .expect_err("retired REFER authority must fail closed");
        assert_eq!(script.dispatches(), vec![(transaction, 603)]);
        assert_eq!(script.attempts(), 1);
        assert_eq!(script.wire_authorships(), 1);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown rejected REFER wire-unknown coordinator");
    }

    #[tokio::test]
    async fn cancelled_reject_refer_waiter_joins_owned_completion() {
        use actions::exact_response_dispatch_test_hook::{self, OwnedCompletion, Step};

        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-reject-refer-cancelled".to_string(),
            rvoip_sip_core::Method::Refer,
            true,
        );
        let transaction_text = transaction.to_string();
        let coordinator = input_admission_coordinator("reject-refer-cancelled").await;
        let machine = state_machine_with_table(&coordinator, MasterStateTable::new());
        let (session_id, handle) = create_inbound_refer_session(
            machine.as_ref(),
            "reject-refer-cancelled-session",
            transaction_text,
        )
        .await;
        let owned = OwnedCompletion::new();
        let script = exact_response_dispatch_test_hook::install(
            &session_id,
            vec![
                Step::Owned(Arc::clone(&owned)),
                Step::Owned(Arc::clone(&owned)),
            ],
        );

        let waiter = {
            let machine = Arc::clone(&machine);
            let handle = handle.clone();
            tokio::spawn(async move { machine.reject_refer_exact(&handle, 603).await })
        };
        owned.wait_until_entered().await;
        waiter.abort();
        assert!(waiter
            .await
            .expect_err("cancelled REFER reject waiter")
            .is_cancelled());
        assert!(machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read cancelled rejection state")
            .refer_transaction_id
            .is_some());

        owned.complete_written();
        machine
            .reject_refer_exact(&handle, 603)
            .await
            .expect("replacement REFER reject waiter observes owned write");
        assert!(machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read replacement rejection state")
            .refer_transaction_id
            .is_none());
        assert_eq!(
            script.dispatches(),
            vec![(transaction.clone(), 603), (transaction, 603)]
        );
        assert_eq!(script.attempts(), 2);
        assert_eq!(script.wire_authorships(), 1);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown cancelled REFER rejection coordinator");
    }

    #[tokio::test]
    async fn initial_invite_zero_wire_failure_retains_exact_retry_envelope() {
        use actions::exact_response_dispatch_test_hook::{self, Step};
        use rvoip_sip_dialog::FinalResponseCompletionDisposition;

        let coordinator = input_admission_coordinator("initial-invite-zero-wire").await;
        let machine = state_machine_with_table(&coordinator, uas_initial_invite_response_table());
        let (session_id, handle, transaction, received_at) =
            create_retryable_uas_invite_session(&machine, "initial-invite-zero-wire-session").await;
        let before = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read initial INVITE preimage");
        let entered_state_at = before.entered_state_at;
        let script = exact_response_dispatch_test_hook::install(
            &session_id,
            vec![Step::ZeroWire, Step::Written],
        );

        let error = machine
            .process_event_with_response_input_exact(
                &handle,
                EventType::AcceptCall,
                ResponseStateInput {
                    local_sdp: Some("v=0\r\na=x-exact-retry-answer\r\n".to_string()),
                    sdp_negotiated: Some(true),
                    extra_headers: Vec::new(),
                    status_override: Some(200),
                },
            )
            .await
            .expect_err("scripted zero-wire response must fail");
        assert_eq!(
            actions::exact_sip_response_failure_disposition(error.as_ref()),
            Some(FinalResponseCompletionDisposition::ZeroWireRetryable)
        );

        let retryable = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read retryable initial INVITE state");
        assert_eq!(retryable.revision(), before.revision() + 1);
        assert_eq!(retryable.call_state, CallState::Ringing);
        assert_eq!(retryable.entered_state_at, entered_state_at);
        assert_eq!(
            retryable.pending_inbound_invite_transaction_id.as_ref(),
            Some(&transaction)
        );
        assert_eq!(retryable.incoming_invite_received_at, Some(received_at));
        assert_eq!(retryable.pending_response_status_override, Some(200));
        assert_eq!(retryable.reject_response_extras, Some(Vec::new()));
        assert_eq!(
            retryable.local_sdp.as_deref(),
            Some("v=0\r\na=x-exact-retry-answer\r\n")
        );
        assert_eq!(script.attempts(), 1);
        assert_eq!(script.wire_authorships(), 0);

        machine
            .process_event(&session_id, EventType::AcceptCall)
            .await
            .expect("retry exact initial INVITE response");
        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read committed initial INVITE response");
        assert_eq!(committed.call_state, CallState::Answering);
        assert!(committed.dialog_established);
        assert!(committed.pending_inbound_invite_transaction_id.is_none());
        assert!(committed.incoming_invite_received_at.is_none());
        assert!(committed.pending_response_status_override.is_none());
        assert!(committed.reject_response_extras.is_none());
        assert_eq!(script.attempts(), 2);
        assert_eq!(
            script.wire_authorships(),
            1,
            "zero-wire retry must author exactly one final response"
        );

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown zero-wire response coordinator");
    }

    #[tokio::test]
    async fn initial_invite_reject_and_redirect_zero_wire_failures_are_retryable() {
        use actions::exact_response_dispatch_test_hook::{self, Step};
        use rvoip_sip_dialog::FinalResponseCompletionDisposition;

        let coordinator = input_admission_coordinator("initial-invite-negative-final").await;
        let machine = state_machine_with_table(&coordinator, uas_initial_invite_response_table());
        let cases = [
            (
                "initial-invite-reject-zero-wire",
                EventType::RejectCall {
                    status: 486,
                    reason: "Busy Here".to_string(),
                },
            ),
            (
                "initial-invite-redirect-zero-wire",
                EventType::RedirectCall {
                    status: 302,
                    contacts: vec!["sip:alternate@example.test".to_string()],
                },
            ),
        ];

        for (name, event) in cases {
            let (session_id, handle, transaction, received_at) =
                create_retryable_uas_invite_session(&machine, name).await;
            let script = exact_response_dispatch_test_hook::install(
                &session_id,
                vec![Step::ZeroWire, Step::Written],
            );
            let error = machine
                .process_event_with_response_input_exact(
                    &handle,
                    event.clone(),
                    ResponseStateInput::headers(Vec::new()),
                )
                .await
                .expect_err("negative final response must expose zero-wire failure");
            assert_eq!(
                actions::exact_sip_response_failure_disposition(error.as_ref()),
                Some(FinalResponseCompletionDisposition::ZeroWireRetryable),
                "{name}"
            );

            let retryable = machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read retryable negative final response");
            assert_eq!(retryable.call_state, CallState::Ringing, "{name}");
            assert_eq!(
                retryable.pending_inbound_invite_transaction_id.as_ref(),
                Some(&transaction),
                "{name}"
            );
            assert_eq!(
                retryable.incoming_invite_received_at,
                Some(received_at),
                "{name}"
            );
            assert_eq!(retryable.reject_response_extras, Some(Vec::new()), "{name}");
            match &event {
                EventType::RejectCall { status, reason } => {
                    assert_eq!(retryable.reject_status, Some(*status));
                    assert_eq!(retryable.reject_reason.as_deref(), Some(reason.as_str()));
                }
                EventType::RedirectCall { status, contacts } => {
                    assert_eq!(retryable.redirect_response_status, Some(*status));
                    assert_eq!(&retryable.redirect_response_contacts, contacts);
                }
                _ => unreachable!("negative final-response test event"),
            }
            assert_eq!(script.attempts(), 1, "{name}");
            assert_eq!(script.wire_authorships(), 0, "{name}");

            machine
                .process_event(&session_id, event)
                .await
                .expect("retry negative initial INVITE final response");
            let committed = machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read committed negative final response");
            assert_eq!(committed.call_state, CallState::Terminated, "{name}");
            assert!(
                committed.pending_inbound_invite_transaction_id.is_none(),
                "{name}"
            );
            assert!(committed.incoming_invite_received_at.is_none(), "{name}");
            assert!(committed.reject_response_extras.is_none(), "{name}");
            assert_eq!(script.attempts(), 2, "{name}");
            assert_eq!(
                script.wire_authorships(),
                1,
                "{name}: retry must author exactly one final response"
            );
            exact_response_dispatch_test_hook::remove(&session_id);
        }

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown negative-final response coordinator");
    }

    #[tokio::test]
    async fn cancelled_initial_invite_waiter_reuses_one_owned_response() {
        use actions::exact_response_dispatch_test_hook::{self, OwnedCompletion, Step};

        let coordinator = input_admission_coordinator("initial-invite-cancelled-waiter").await;
        let machine = state_machine_with_table(&coordinator, uas_initial_invite_response_table());
        let (session_id, handle, transaction, received_at) = create_retryable_uas_invite_session(
            &machine,
            "initial-invite-cancelled-waiter-session",
        )
        .await;
        machine
            .store
            .update_session_exact_with(&handle, None, |session| {
                session.pending_response_status_override = Some(200);
                session.reject_response_extras = Some(Vec::new());
            })
            .expect("stage cancellation-safe response envelope");
        let before = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read cancellation preimage");
        let owned = OwnedCompletion::new();
        let script = exact_response_dispatch_test_hook::install(
            &session_id,
            vec![
                Step::Owned(Arc::clone(&owned)),
                Step::Owned(Arc::clone(&owned)),
            ],
        );

        let in_flight = {
            let machine = Arc::clone(&machine);
            let session_id = session_id.clone();
            tokio::spawn(async move {
                machine
                    .process_event(&session_id, EventType::AcceptCall)
                    .await
            })
        };
        tokio::time::timeout(Duration::from_secs(1), owned.wait_until_entered())
            .await
            .expect("exact response waiter did not yield after runner ownership");
        tokio::task::yield_now().await;
        in_flight.abort();
        assert!(in_flight
            .await
            .expect_err("aborted exact response waiter must be cancelled")
            .is_cancelled());

        let after_cancel = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read state after cancelled waiter");
        assert_eq!(after_cancel.revision(), before.revision());
        assert_eq!(after_cancel.call_state, CallState::Ringing);
        assert_eq!(
            after_cancel.pending_inbound_invite_transaction_id.as_ref(),
            Some(&transaction)
        );
        assert_eq!(after_cancel.incoming_invite_received_at, Some(received_at));
        assert_eq!(after_cancel.pending_response_status_override, Some(200));
        assert_eq!(after_cancel.reject_response_extras, Some(Vec::new()));
        assert_eq!(script.attempts(), 1);
        assert_eq!(script.wire_authorships(), 1);

        owned.complete_written();
        machine
            .process_event(&session_id, EventType::AcceptCall)
            .await
            .expect("replacement waiter observes owned final response");
        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read replacement-waiter commit");
        assert_eq!(committed.call_state, CallState::Answering);
        assert!(committed.dialog_established);
        assert!(committed.pending_inbound_invite_transaction_id.is_none());
        assert!(committed.incoming_invite_received_at.is_none());
        assert!(committed.pending_response_status_override.is_none());
        assert!(committed.reject_response_extras.is_none());
        assert_eq!(script.attempts(), 2);
        assert_eq!(
            script.wire_authorships(),
            1,
            "replacement waiter must classify the runner-owned response without duplicating it"
        );

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown cancelled-waiter response coordinator");
    }

    #[tokio::test]
    async fn initial_invite_wire_unknown_is_terminal_and_commits_post_wire_state() {
        use actions::exact_response_dispatch_test_hook::{self, Step};
        use rvoip_sip_dialog::FinalResponseCompletionDisposition;

        let coordinator = input_admission_coordinator("initial-invite-wire-unknown").await;
        let machine = state_machine_with_table(&coordinator, uas_initial_invite_response_table());
        let (session_id, handle, _transaction, _received_at) =
            create_retryable_uas_invite_session(&machine, "initial-invite-wire-unknown-session")
                .await;
        let script =
            exact_response_dispatch_test_hook::install(&session_id, vec![Step::WireUnknown]);

        let error = machine
            .process_event_with_response_input_exact(
                &handle,
                EventType::AcceptCall,
                ResponseStateInput::accept(
                    Some("v=0\r\na=x-wire-unknown-answer\r\n".to_string()),
                    Vec::new(),
                ),
            )
            .await
            .expect_err("wire-unknown exact response remains observable as an error");
        assert_eq!(
            actions::exact_sip_response_failure_disposition(error.as_ref()),
            Some(FinalResponseCompletionDisposition::WireUnknownErrorTerminal)
        );

        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read terminal wire-unknown state");
        assert_eq!(committed.call_state, CallState::Answering);
        assert!(committed.dialog_established);
        assert!(committed.pending_inbound_invite_transaction_id.is_none());
        assert!(committed.incoming_invite_received_at.is_none());
        assert!(committed.pending_response_status_override.is_none());
        assert!(committed.reject_response_extras.is_none());
        assert_eq!(script.attempts(), 1);
        assert_eq!(script.wire_authorships(), 1);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown wire-unknown response coordinator");
    }

    #[tokio::test]
    async fn negative_final_wire_unknown_runs_ordered_cleanup_once() {
        use actions::exact_response_dispatch_test_hook::{self, Step};
        use rvoip_sip_dialog::FinalResponseCompletionDisposition;

        let coordinator = input_admission_coordinator("negative-final-wire-unknown").await;
        let machine = state_machine_with_table(&coordinator, uas_initial_invite_response_table());
        let cases = [
            (
                "reject-wire-unknown-cleanup",
                EventType::RejectCall {
                    status: 486,
                    reason: "Busy Here".to_string(),
                },
                Action::SendRejectResponse,
            ),
            (
                "redirect-wire-unknown-cleanup",
                EventType::RedirectCall {
                    status: 302,
                    contacts: vec!["sip:alternate@example.test".to_string()],
                },
                Action::SendRedirectResponse,
            ),
        ];

        for (name, event, response_action) in cases {
            let (session_id, handle, _transaction, _received_at) =
                create_retryable_uas_invite_session(&machine, name).await;
            machine
                .store
                .update_session_exact_with(&handle, None, |session| {
                    session.history = Some(SessionHistory::new(HistoryConfig::default()));
                    session.dialog_established = true;
                    session.media_session_ready = true;
                    session.sdp_negotiated = true;
                    session.reject_response_extras = Some(Vec::new());
                })
                .expect("stage wire-unknown cleanup fixture");
            let script =
                exact_response_dispatch_test_hook::install(&session_id, vec![Step::WireUnknown]);

            let error = machine
                .process_event(&session_id, event)
                .await
                .expect_err("wire-unknown final response remains caller-visible");
            assert_eq!(
                actions::exact_sip_response_failure_disposition(error.as_ref()),
                Some(FinalResponseCompletionDisposition::WireUnknownErrorTerminal),
                "{name}"
            );

            let committed = machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read wire-unknown cleanup commit");
            assert_eq!(committed.call_state, CallState::Terminated, "{name}");
            assert!(
                committed.pending_inbound_invite_transaction_id.is_none(),
                "{name}"
            );
            assert!(committed.incoming_invite_received_at.is_none(), "{name}");
            assert!(committed.reject_response_extras.is_none(), "{name}");
            assert!(!committed.dialog_established, "{name}");
            assert!(committed.dialog_id.is_none(), "{name}");
            assert!(!committed.media_session_ready, "{name}");
            assert!(committed.media_session_id.is_none(), "{name}");
            assert!(!committed.sdp_negotiated, "{name}");
            assert!(committed.local_sdp.is_none(), "{name}");

            let record = committed
                .history
                .as_ref()
                .expect("wire-unknown cleanup history")
                .get_recent(1)
                .pop()
                .expect("wire-unknown cleanup transition record");
            assert_eq!(record.to_state, Some(CallState::Terminated), "{name}");
            assert_eq!(record.actions_executed.len(), 3, "{name}");
            assert_eq!(record.actions_executed[0].action, response_action, "{name}");
            assert!(!record.actions_executed[0].success, "{name}");
            assert_eq!(record.actions_executed[1].action, Action::CleanupDialog);
            assert!(record.actions_executed[1].success, "{name}");
            assert_eq!(record.actions_executed[2].action, Action::CleanupMedia);
            assert!(record.actions_executed[2].success, "{name}");
            assert_eq!(script.attempts(), 1, "{name}");
            assert_eq!(script.wire_authorships(), 1, "{name}");
            exact_response_dispatch_test_hook::remove(&session_id);
        }

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown negative wire-unknown cleanup coordinator");
    }

    #[tokio::test]
    async fn inbound_response_exact_200_inputs_survive_queued_pending_index_overwrite() {
        use actions::exact_response_dispatch_test_hook::{self, OwnedCompletion, Step};

        let coordinator = input_admission_coordinator("exact-in-dialog-200").await;
        let mut table = MasterStateTable::new();
        insert_inbound_final_response_transition(
            &mut table,
            EventType::UpdateReceived { sdp: None },
        );
        insert_inbound_final_response_transition(
            &mut table,
            EventType::ReinviteReceived { sdp: None },
        );
        let machine = state_machine_with_table(&coordinator, table);
        let (session_id, handle) = create_inbound_response_session(
            machine.as_ref(),
            "exact-in-dialog-200-session",
            CallState::Active,
            None,
        )
        .await;
        let owned = OwnedCompletion::new();
        let script = exact_response_dispatch_test_hook::install(
            &session_id,
            vec![Step::Owned(Arc::clone(&owned)), Step::Written],
        );
        let (update_input, update_transaction) =
            inbound_response_input("UPDATE", "z9hG4bK-exact-update");
        let (reinvite_input, reinvite_transaction) =
            inbound_response_input("INVITE", "z9hG4bK-exact-reinvite");

        let first_machine = Arc::clone(&machine);
        let first_handle = handle.clone();
        let first = tokio::spawn(async move {
            first_machine
                .process_inbound_response_event_exact(
                    &first_handle,
                    EventType::UpdateReceived { sdp: None },
                    update_input,
                )
                .await
        });
        owned.wait_until_entered().await;

        let second_machine = Arc::clone(&machine);
        let second_handle = handle.clone();
        let second = tokio::spawn(async move {
            second_machine
                .process_inbound_response_event_exact(
                    &second_handle,
                    EventType::ReinviteReceived { sdp: None },
                    reinvite_input,
                )
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(
            script.attempts(),
            1,
            "the second request must retain its own input while queued behind the exact lane"
        );

        owned.complete_written();
        first
            .await
            .expect("join exact UPDATE response")
            .expect("write exact UPDATE response");
        second
            .await
            .expect("join exact re-INVITE response")
            .expect("write exact re-INVITE response");

        assert_eq!(
            script.dispatches(),
            vec![(update_transaction, 200), (reinvite_transaction, 200)]
        );
        assert_eq!(script.wire_authorships(), 2);
        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown exact in-dialog 200 coordinator");
    }

    #[tokio::test]
    async fn event_local_response_authority_wins_over_stale_retained_invite_key() {
        use actions::exact_response_dispatch_test_hook::{self, Step};

        let coordinator = input_admission_coordinator("exact-event-wins-stale-retained").await;
        let mut table = MasterStateTable::new();
        insert_inbound_final_response_transition(
            &mut table,
            EventType::ReinviteReceived { sdp: None },
        );
        let machine = state_machine_with_table(&coordinator, table);
        let (session_id, handle) = create_inbound_response_session(
            machine.as_ref(),
            "exact-event-wins-stale-retained-session",
            CallState::Active,
            None,
        )
        .await;
        let stale = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-stale-initial-invite".to_string(),
            rvoip_sip_core::Method::Invite,
            true,
        );
        let stale_for_state = stale.clone();
        machine
            .store
            .update_session_exact_with(&handle, None, move |session| {
                session.pending_inbound_invite_transaction_id = Some(stale_for_state);
            })
            .expect("stage stale retained response key");
        let script = exact_response_dispatch_test_hook::install(&session_id, vec![Step::Written]);
        let (input, exact) = inbound_response_input("INVITE", "z9hG4bK-event-local-response-owner");

        machine
            .process_inbound_response_event_exact(
                &handle,
                EventType::ReinviteReceived { sdp: None },
                input,
            )
            .await
            .expect("send response through event-local transaction");
        assert_eq!(script.dispatches(), vec![(exact, 200)]);
        assert_eq!(
            machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read exact event result")
                .pending_inbound_invite_transaction_id
                .as_ref(),
            Some(&stale),
            "an unrelated retained key must neither author nor be consumed"
        );

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown exact event authority coordinator");
    }

    #[tokio::test]
    async fn inbound_response_exact_491_glare_uses_request_transaction() {
        use crate::session_store::state::PendingReinvite;
        use actions::exact_response_dispatch_test_hook::{self, Step};

        let coordinator = input_admission_coordinator("exact-in-dialog-491").await;
        let machine = state_machine_with_table(&coordinator, MasterStateTable::new());
        let (session_id, handle) = create_inbound_response_session(
            machine.as_ref(),
            "exact-in-dialog-491-session",
            CallState::Active,
            Some(PendingReinvite::Hold),
        )
        .await;
        let script = exact_response_dispatch_test_hook::install(&session_id, vec![Step::Written]);
        let (input, transaction) = inbound_response_input("INVITE", "z9hG4bK-exact-glare");

        let result = machine
            .process_inbound_response_event_exact(
                &handle,
                EventType::ReinviteReceived { sdp: None },
                input,
            )
            .await
            .expect("write exact 491 glare response");
        assert!(result.transition.is_none());
        assert_eq!(script.dispatches(), vec![(transaction, 491)]);
        assert_eq!(script.wire_authorships(), 1);
        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read glare session");
        assert_eq!(committed.call_state, CallState::Active);
        assert_eq!(committed.pending_reinvite, Some(PendingReinvite::Hold));

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown exact in-dialog 491 coordinator");
    }

    #[tokio::test]
    async fn inbound_response_zero_wire_retains_transition_for_exact_retry() {
        use actions::exact_response_dispatch_test_hook::{self, Step};

        let coordinator = input_admission_coordinator("exact-in-dialog-zero-wire").await;
        let table = inbound_final_response_table(
            CallState::HoldPending,
            EventType::ReinviteReceived { sdp: None },
            CallState::OnHold,
            vec![Action::SendSIPResponse(200, "OK".to_string())],
        );
        let machine = state_machine_with_table(&coordinator, table);
        let (session_id, handle) = create_inbound_response_session(
            machine.as_ref(),
            "exact-in-dialog-zero-wire-session",
            CallState::HoldPending,
            None,
        )
        .await;
        let script = exact_response_dispatch_test_hook::install(
            &session_id,
            vec![Step::ZeroWire, Step::Written],
        );
        let (first_input, transaction) =
            inbound_response_input("INVITE", "z9hG4bK-exact-zero-wire");

        machine
            .process_inbound_response_event_exact(
                &handle,
                EventType::ReinviteReceived { sdp: None },
                first_input,
            )
            .await
            .expect_err("zero-wire response unexpectedly committed the transition");
        assert_eq!(
            machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read retryable state")
                .call_state,
            CallState::HoldPending
        );

        let (retry_input, retry_transaction) =
            inbound_response_input("INVITE", "z9hG4bK-exact-zero-wire");
        assert_eq!(retry_transaction, transaction);
        machine
            .process_inbound_response_event_exact(
                &handle,
                EventType::ReinviteReceived { sdp: None },
                retry_input,
            )
            .await
            .expect("exact zero-wire retry succeeds");
        assert_eq!(
            machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read committed retry")
                .call_state,
            CallState::OnHold
        );
        assert_eq!(
            script.dispatches(),
            vec![(transaction.clone(), 200), (transaction, 200)]
        );
        assert_eq!(script.attempts(), 2);
        assert_eq!(script.wire_authorships(), 1);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown exact zero-wire coordinator");
    }

    #[tokio::test]
    async fn inbound_response_wire_unknown_commits_once_without_duplicate_response() {
        use actions::exact_response_dispatch_test_hook::{self, Step};

        let coordinator = input_admission_coordinator("exact-in-dialog-wire-unknown").await;
        let table = inbound_final_response_table(
            CallState::Active,
            EventType::UpdateReceived { sdp: None },
            CallState::OnHold,
            vec![
                Action::SendSIPResponse(200, "OK".to_string()),
                Action::SendSIPResponse(200, "duplicate must be skipped".to_string()),
            ],
        );
        let machine = state_machine_with_table(&coordinator, table);
        let (session_id, handle) = create_inbound_response_session(
            machine.as_ref(),
            "exact-in-dialog-wire-unknown-session",
            CallState::Active,
            None,
        )
        .await;
        let script =
            exact_response_dispatch_test_hook::install(&session_id, vec![Step::WireUnknown]);
        let (input, transaction) = inbound_response_input("UPDATE", "z9hG4bK-exact-wire-unknown");

        machine
            .process_inbound_response_event_exact(
                &handle,
                EventType::UpdateReceived { sdp: None },
                input,
            )
            .await
            .expect_err("wire uncertainty must remain visible to the caller");
        assert_eq!(
            machine
                .store
                .get_session_snapshot_exact(&handle)
                .expect("read terminal wire-unknown commit")
                .call_state,
            CallState::OnHold
        );
        assert_eq!(script.dispatches(), vec![(transaction, 200)]);
        assert_eq!(script.attempts(), 1);
        assert_eq!(script.wire_authorships(), 1);

        exact_response_dispatch_test_hook::remove(&session_id);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown exact wire-unknown coordinator");
    }

    #[test]
    fn event_state_input_applies_transition_data_to_one_working_state() {
        let session_id = SessionId("typed-transition-input".to_string());
        let mut session = SessionState::new(session_id, crate::state_table::Role::UAS);

        EventStateInput {
            remote_sdp: Some("remote-sdp".to_string()),
            remote_sdp_supplied: true,
            preserve_committed_provisional_sdp: false,
            local_sdp: Some("local-sdp".to_string()),
            sdp_negotiated: Some(true),
            response: Some(ResponseStateInput::provisional(181, Vec::new())),
            outbound_session: None,
            registration_start: None,
            transfer_request: Some(TransferRequestStateInput::new(
                "sip:target@example.test".to_string(),
                "refer-transaction".to_string(),
                Some("sip:referrer@example.test".to_string()),
                Some("call-id;to-tag=to;from-tag=from".to_string()),
            )),
            refer_notify: None,
            session_refresh: None,
            inbound_response: None,
            invite_2xx_ack: None,
            auth_required: Some(AuthRequiredStateInput::new(
                Some(
                    rvoip_infra_common::events::cross_crate::SipTransportContext::new(
                        "TLS",
                        "127.0.0.1:5061",
                        "127.0.0.2:5061",
                        true,
                    ),
                ),
                Some("auth-transaction".to_string()),
                Some("sips:target@example.test".to_string()),
            )),
            confirmed_negotiation_failure: false,
        }
        .apply(&mut session);

        assert_eq!(session.remote_sdp.as_deref(), Some("remote-sdp"));
        assert_eq!(session.local_sdp.as_deref(), Some("local-sdp"));
        assert!(session.sdp_negotiated);
        assert_eq!(session.reject_response_extras, Some(Vec::new()));
        assert_eq!(session.pending_response_status_override, Some(181));
        assert_eq!(
            session.transfer_target.as_deref(),
            Some("sip:target@example.test")
        );
        assert_eq!(
            session.refer_transaction_id.as_deref(),
            Some("refer-transaction")
        );
        assert_eq!(
            session.referred_by.as_deref(),
            Some("sip:referrer@example.test")
        );
        assert_eq!(
            session.replaces_header.as_deref(),
            Some("call-id;to-tag=to;from-tag=from")
        );
        assert_eq!(
            session.pending_auth_transaction_id.as_deref(),
            Some("auth-transaction")
        );
        assert_eq!(
            session.pending_auth_request_uri.as_deref(),
            Some("sips:target@example.test")
        );
        assert!(session
            .pending_auth_transport
            .as_ref()
            .is_some_and(|transport| transport.secure));
    }

    #[test]
    fn final_invite_response_preserves_the_committed_provisional_answer() {
        let session_id = SessionId("committed-provisional-answer".to_string());
        let mut session = SessionState::new(session_id, crate::state_table::Role::UAC);
        session.call_state = CallState::EarlyMedia;
        session.remote_sdp = Some("stable-183-answer".to_string());
        session.sdp_negotiated = true;
        session.media_session_ready = true;

        EventStateInput {
            remote_sdp: Some("untrusted-final-copy".to_string()),
            remote_sdp_supplied: true,
            preserve_committed_provisional_sdp: true,
            ..Default::default()
        }
        .apply(&mut session);

        assert_eq!(session.remote_sdp.as_deref(), Some("stable-183-answer"));
    }

    #[tokio::test]
    async fn invalid_outbound_update_answer_rolls_back_the_complete_stable_snapshot() {
        const STABLE_LOCAL: &str = "v=0\r\n\
o=alice 700 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 19000 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=sendrecv\r\n";
        const STABLE_REMOTE: &str = "v=0\r\n\
o=bob 800 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 19002 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=sendrecv\r\n";
        const UPDATE_OFFER: &str = "v=0\r\n\
o=alice 700 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 19000 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=sendonly\r\n";
        const INVALID_ANSWER: &str = "v=0\r\n\
o=bob 800 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 19002 RTP/AVP 8\r\n\
a=rtpmap:8 PCMA/8000\r\n\
a=recvonly\r\n";

        let mut config = crate::api::unified::Config::local("update-answer-rollback", 0);
        config.media_mode = crate::api::unified::MediaMode::SignalingOnly { sdp_rtp_port: 9 };
        let coordinator = crate::api::unified::UnifiedCoordinator::new(config)
            .await
            .expect("create UPDATE rollback coordinator");
        let mut table = MasterStateTable::new();
        table.insert(
            StateKey {
                role: Role::UAC,
                state: CallState::Active,
                event: EventType::Dialog200OK,
            },
            Transition {
                guards: Vec::new(),
                actions: vec![Action::NegotiateSDPAsUAC],
                next_state: Some(CallState::Active),
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
        let machine = state_machine_with_table(&coordinator, table);
        let session_id = SessionId("invalid-update-answer".to_string());
        machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAC, false, |session| {
                session.call_state = CallState::Active;
                session.local_sdp = Some(STABLE_LOCAL.to_string());
                session.remote_sdp = Some(STABLE_REMOTE.to_string());
                session.sdp_negotiated = true;
                session.local_media_direction = crate::types::MediaDirection::SendRecv;
                session.remote_media_direction = crate::types::MediaDirection::SendRecv;
                session.set_negotiated_config(
                    crate::session_store::state::NegotiatedConfig {
                        local_addr: "127.0.0.1:19000".parse().unwrap(),
                        remote_addr: "127.0.0.1:19002".parse().unwrap(),
                        codec: "PCMU".to_string(),
                        sample_rate: 8_000,
                        channels: 1,
                        fmtp: None,
                    },
                    0,
                );
                session
                    .begin_offer_answer(rvoip_sip_core::Method::Update, UPDATE_OFFER.to_string())
                    .unwrap();
                session.local_sdp = Some(UPDATE_OFFER.to_string());
            })
            .await
            .expect("create pending UPDATE session");
        let handle = machine
            .store
            .lifecycle_handle(&session_id)
            .expect("capture exact UPDATE lifetime");

        machine
            .process_event_with_remote_sdp_exact(
                &handle,
                EventType::Dialog200OK,
                Some(INVALID_ANSWER.to_string()),
            )
            .await
            .expect_err("unoffered answer payload must fail negotiation");

        let stable = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read rolled-back UPDATE session");
        assert_eq!(stable.call_state, CallState::Active);
        assert_eq!(stable.local_sdp.as_deref(), Some(STABLE_LOCAL));
        assert_eq!(stable.remote_sdp.as_deref(), Some(STABLE_REMOTE));
        assert!(stable.sdp_negotiated);
        assert_eq!(
            stable.local_media_direction,
            crate::types::MediaDirection::SendRecv
        );
        assert_eq!(
            stable.remote_media_direction,
            crate::types::MediaDirection::SendRecv
        );
        assert_eq!(stable.negotiated_payload_type(), Some(0));
        assert!(stable.pending_offer_answer.is_none());

        let mut retry = machine
            .store
            .get_session_exact(&handle)
            .await
            .expect("load rolled-back UPDATE session");
        retry
            .begin_offer_answer(rvoip_sip_core::Method::Update, UPDATE_OFFER.to_string())
            .expect("a second UPDATE can acquire offer ownership");

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown UPDATE rollback coordinator");
    }

    #[tokio::test]
    async fn failed_outbound_reinvite_media_commit_rolls_back_the_complete_stable_snapshot() {
        const STABLE_LOCAL: &str = "v=0\r\n\
o=alice 710 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 19100 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=sendrecv\r\n";
        const STABLE_REMOTE: &str = "v=0\r\n\
o=bob 810 1 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 19102 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=sendrecv\r\n";
        const REINVITE_OFFER: &str = "v=0\r\n\
o=alice 710 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 19100 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=sendonly\r\n";
        const VALID_ANSWER: &str = "v=0\r\n\
o=bob 810 2 IN IP4 127.0.0.1\r\n\
s=-\r\n\
c=IN IP4 127.0.0.1\r\n\
t=0 0\r\n\
m=audio 19102 RTP/AVP 0\r\n\
a=rtpmap:0 PCMU/8000\r\n\
a=recvonly\r\n";

        let mut config = crate::api::unified::Config::local("reinvite-commit-rollback", 0);
        config.media_mode = crate::api::unified::MediaMode::SignalingOnly { sdp_rtp_port: 9 };
        let coordinator = crate::api::unified::UnifiedCoordinator::new(config)
            .await
            .expect("create re-INVITE commit-rollback coordinator");
        let mut table = MasterStateTable::new();
        table.insert(
            StateKey {
                role: Role::UAC,
                state: CallState::Active,
                event: EventType::Dialog200OK,
            },
            Transition {
                guards: Vec::new(),
                actions: vec![Action::NegotiateSDPAsUAC, Action::ClearPendingReinvite],
                next_state: Some(CallState::Active),
                condition_updates: ConditionUpdates::none(),
                publish_events: Vec::new(),
            },
        );
        let machine = state_machine_with_table(&coordinator, table);
        let session_id = SessionId("failed-reinvite-media-commit".to_string());
        machine
            .store
            .create_session_initialized(session_id.clone(), Role::UAC, false, |session| {
                session.call_state = CallState::Active;
                session.local_sdp = Some(STABLE_LOCAL.to_string());
                session.remote_sdp = Some(STABLE_REMOTE.to_string());
                session.sdp_negotiated = true;
                session.local_media_direction = crate::types::MediaDirection::SendRecv;
                session.remote_media_direction = crate::types::MediaDirection::SendRecv;
                session.set_negotiated_config(
                    crate::session_store::state::NegotiatedConfig {
                        local_addr: "127.0.0.1:19100".parse().unwrap(),
                        remote_addr: "127.0.0.1:19102".parse().unwrap(),
                        codec: "PCMU".to_string(),
                        sample_rate: 8_000,
                        channels: 1,
                        fmtp: None,
                    },
                    0,
                );
                session.pending_reinvite = Some(crate::session_store::state::PendingReinvite::Hold);
                session.reinvite_retry_attempts = 2;
                session
                    .begin_offer_answer(rvoip_sip_core::Method::Invite, REINVITE_OFFER.to_string())
                    .unwrap();
                session.local_sdp = Some(REINVITE_OFFER.to_string());
            })
            .await
            .expect("create pending re-INVITE session");
        let handle = machine
            .store
            .lifecycle_handle(&session_id)
            .expect("capture exact re-INVITE lifetime");
        machine
            .media_adapter
            .fail_next_staged_media_commit_for_test();

        let error = machine
            .process_event_with_remote_sdp_exact(
                &handle,
                EventType::Dialog200OK,
                Some(VALID_ANSWER.to_string()),
            )
            .await
            .expect_err("injected staged-media commit must fail");
        assert!(matches!(
            error.downcast_ref::<crate::errors::SessionError>(),
            Some(crate::errors::SessionError::MediaError(detail))
                if detail == "injected staged media commit failure"
        ));

        let stable = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read rolled-back re-INVITE session");
        assert_eq!(stable.call_state, CallState::Active);
        assert_eq!(stable.local_sdp.as_deref(), Some(STABLE_LOCAL));
        assert_eq!(stable.remote_sdp.as_deref(), Some(STABLE_REMOTE));
        assert!(stable.sdp_negotiated);
        assert_eq!(
            stable.local_media_direction,
            crate::types::MediaDirection::SendRecv
        );
        assert_eq!(
            stable.remote_media_direction,
            crate::types::MediaDirection::SendRecv
        );
        assert_eq!(stable.negotiated_payload_type(), Some(0));
        assert!(stable.pending_offer_answer.is_none());
        assert!(stable.pending_reinvite.is_none());
        assert_eq!(stable.reinvite_retry_attempts, 0);
        assert!(!machine.media_adapter.has_staged_media_negotiation(&stable));

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown re-INVITE commit-rollback coordinator");
    }

    #[test]
    fn incoming_call_event_preserves_initial_sdp_and_transaction() {
        let transaction = rvoip_sip_dialog::transaction::TransactionKey::new(
            "z9hG4bK-initial-invite".to_string(),
            rvoip_sip_core::Method::Invite,
            true,
        );
        let received_at = Instant::now();
        let mut session = SessionState::new(
            SessionId("initial-invite-event".to_string()),
            crate::state_table::Role::UAS,
        );
        session.local_uri = Some("sip:bob@example.test".to_string());
        session.remote_uri = Some("sip:alice@example.test".to_string());
        session.incoming_invite_received_at = Some(received_at);
        session.pending_inbound_invite_transaction_id = Some(transaction.clone());
        session.remote_sdp = Some("v=0\r\na=x-initial\r\n".to_string());

        apply_incoming_call_event_state(
            &mut session,
            "sip:alice@example.test",
            Some("v=0\r\na=x-initial\r\n"),
        );

        assert_eq!(
            session.pending_inbound_invite_transaction_id.as_ref(),
            Some(&transaction)
        );
        assert_eq!(
            session.remote_sdp.as_deref(),
            Some("v=0\r\na=x-initial\r\n")
        );
        assert_eq!(session.incoming_invite_received_at, Some(received_at));
        assert_eq!(session.local_uri.as_deref(), Some("sip:bob@example.test"));
    }

    #[test]
    fn refer_notify_input_applies_only_progress_and_completion_state() {
        let mut session = SessionState::new(
            SessionId("refer-notify-apply".to_string()),
            crate::state_table::Role::UAC,
        );

        ReferNotifyInput::new(183, "Session Progress".to_string()).apply(&mut session);
        assert!(session.transfer_target_progress_seen);
        assert_eq!(
            session.transfer_target_last_progress,
            Some((183, "Session Progress".to_string()))
        );
        assert_eq!(
            session.transfer_state,
            crate::session_store::state::TransferState::None
        );

        ReferNotifyInput::new(200, "OK".to_string()).apply(&mut session);
        assert_eq!(
            session.transfer_state,
            crate::session_store::state::TransferState::TransferCompleted
        );
        assert_eq!(
            session.transfer_target_last_progress,
            Some((183, "Session Progress".to_string()))
        );

        ReferNotifyInput::new(486, "Busy Here".to_string()).apply(&mut session);
        assert_eq!(
            session.transfer_state,
            crate::session_store::state::TransferState::TransferCompleted
        );
        assert_eq!(
            session.transfer_target_last_progress,
            Some((183, "Session Progress".to_string()))
        );
    }

    #[test]
    fn refer_notify_outcome_retains_target_and_progress_evidence() {
        let mut session = SessionState::new(
            SessionId("refer-notify-outcome".to_string()),
            crate::state_table::Role::UAC,
        );
        session.transfer_target = Some("sip:target@example.test".to_string());
        session.transfer_target_progress_seen = true;
        session.transfer_target_last_progress =
            Some((180, "Ringing at transfer target".to_string()));

        assert_eq!(
            ReferNotifyInput::new(180, "Ringing".to_string()).outcome(&session),
            ReferNotifyOutcome::Progress
        );
        assert_eq!(
            ReferNotifyInput::new(200, "OK".to_string()).outcome(&session),
            ReferNotifyOutcome::Completed {
                transfer_target: "sip:target@example.test".to_string(),
                progress_evidence: Some((180, "Ringing at transfer target".to_string())),
            }
        );
        assert_eq!(
            ReferNotifyInput::new(486, "Busy Here".to_string()).outcome(&session),
            ReferNotifyOutcome::Failed
        );
        assert_eq!(
            ReferNotifyInput::new(700, "Invalid".to_string()).outcome(&session),
            ReferNotifyOutcome::Ignored
        );
    }

    #[test]
    fn refer_notify_outcome_requires_a_committed_yaml_transition() {
        let source = include_str!("executor.rs");
        let method = source
            .split("pub(crate) async fn process_refer_notify_exact")
            .nth(1)
            .and_then(|tail| tail.split("/// Reject a pending inbound REFER").next())
            .expect("exact REFER NOTIFY executor source");

        assert!(method.contains("Ok(result) if result.transition.is_some()"));
        assert!(method.contains("if self.table.get(&key).is_none()"));
        assert!(method.contains("ReceiveNOTIFY has no YAML transition"));
        assert!(method.contains("refer_notify: Some(input)"));
    }

    #[tokio::test]
    async fn refer_notify_exact_commit_rejects_a_stale_registry_revision() {
        let coordinator = crate::api::unified::UnifiedCoordinator::new(
            crate::api::unified::Config::local("stale-refer-notify", 0),
        )
        .await
        .expect("create REFER NOTIFY coordinator");
        let state_machine = Arc::clone(&coordinator.helpers.state_machine);
        let store = Arc::clone(&state_machine.store);
        let session_id = SessionId("reused-refer-notify".to_string());
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create original REFER NOTIFY lifetime");
        let current_handle = store
            .lifecycle_handle(&session_id)
            .expect("capture current exact REFER NOTIFY handle");
        assert_eq!(
            state_machine
                .process_refer_notify_exact(
                    &current_handle,
                    ReferNotifyInput::new(180, "Ringing".to_string()),
                )
                .await
                .expect("commit current-revision REFER progress"),
            ReferNotifyOutcome::Progress
        );
        let committed = store
            .get_session_snapshot_exact(&current_handle)
            .expect("read committed current-revision REFER progress");
        assert!(committed.transfer_target_progress_seen);
        assert_eq!(
            committed.transfer_target_last_progress.clone(),
            Some((180, "Ringing".to_string()))
        );
        let stale_handle = current_handle.with_next_slot_revision_for_test();

        assert!(state_machine
            .process_refer_notify_exact(
                &stale_handle,
                ReferNotifyInput::new(200, "stale completion".to_string()),
            )
            .await
            .is_err());
        let unchanged = store
            .get_session_snapshot_exact(&current_handle)
            .expect("read state after rejected stale REFER NOTIFY callback");
        assert!(unchanged.transfer_target_progress_seen);
        assert_eq!(
            unchanged.transfer_target_last_progress,
            Some((180, "Ringing".to_string()))
        );
        assert_eq!(
            unchanged.transfer_state,
            crate::session_store::state::TransferState::None
        );

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown REFER NOTIFY coordinator");
    }

    #[test]
    fn fast_401_and_407_replace_auth_correlation_on_one_working_state() {
        for status in [401, 407] {
            let transaction_id = format!("challenge-{status}");
            let request_uri = format!("sip:target-{status}@example.test");
            let mut session = SessionState::new(
                SessionId(format!("fast-auth-{status}")),
                crate::state_table::Role::UAC,
            );
            session.pending_auth_transaction_id = Some("older-transaction".to_string());
            session.pending_auth_request_uri = Some("sip:older@example.test".to_string());

            EventStateInput {
                auth_required: Some(AuthRequiredStateInput::new(
                    None,
                    Some(transaction_id.clone()),
                    Some(request_uri.clone()),
                )),
                ..Default::default()
            }
            .apply(&mut session);

            assert_eq!(
                session.pending_auth_transaction_id.as_deref(),
                Some(transaction_id.as_str())
            );
            assert_eq!(
                session.pending_auth_request_uri.as_deref(),
                Some(request_uri.as_str())
            );
            assert!(session.pending_auth_transport.is_none());
        }
    }

    #[tokio::test]
    async fn exact_transaction_auth_cleanup_commits_only_the_matching_owner() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("exact-auth-cleanup".to_string());
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create exact auth lifetime");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("current exact auth handle");
        store
            .update_session_exact_with(&handle, None, |session| {
                session.pending_auth_transaction_id = Some("current-transaction".to_string());
                session.pending_auth_request_uri = Some("sip:current@example.test".to_string());
                session.pending_auth_method = Some("INFO".to_string());
                session.pending_auth = Some((401, "Digest realm=\"test\"".to_string()));
            })
            .expect("stage exact auth owner");

        assert!(!clear_tracked_request_auth_state_in_exact_lane(
            &store,
            &handle,
            "older-transaction",
        )
        .await
        .expect("mismatched cleanup is a no-op"));
        assert_eq!(
            store
                .get_session_snapshot_exact(&handle)
                .expect("auth owner remains")
                .pending_auth_transaction_id
                .as_deref(),
            Some("current-transaction")
        );

        assert!(clear_tracked_request_auth_state_in_exact_lane(
            &store,
            &handle,
            "current-transaction",
        )
        .await
        .expect("matching cleanup commits"));
        let cleared = store
            .get_session_snapshot_exact(&handle)
            .expect("read cleared exact lifetime");
        assert!(cleared.pending_auth_transaction_id.is_none());
        assert!(cleared.pending_auth_request_uri.is_none());
        assert!(cleared.pending_auth.is_none());
    }

    #[tokio::test]
    async fn stale_auth_cleanup_cannot_target_reused_session_generation() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("reused-auth-cleanup".to_string());
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create original auth lifetime");
        let stale_handle = store
            .lifecycle_handle(&session_id)
            .expect("original exact handle");
        store
            .remove_session_exact(&stale_handle)
            .await
            .expect("retire original auth lifetime");

        assert!(clear_tracked_request_auth_state_in_exact_lane(
            &store,
            &stale_handle,
            "original-transaction",
        )
        .await
        .is_err());
        let reuse_error = store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect_err("retained stale handle must block raw-ID reuse");
        assert!(matches!(
            reuse_error.downcast_ref::<crate::session_lifecycle::SessionAdmissionError>(),
            Some(crate::session_lifecycle::SessionAdmissionError::ReuseBlocked)
        ));

        // A retained exact callback and a replacement lifetime cannot coexist.
        // Once the stale handle is released, the raw ID may be admitted again,
        // but there is no remaining capability that can mutate it as the old
        // generation.
        drop(stale_handle);
        assert!(store.authority().elapse_reuse_horizon_for_test(&session_id));
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create replacement auth lifetime after stale handle release");
        let replacement_handle = store
            .lifecycle_handle(&session_id)
            .expect("replacement exact handle");
        store
            .update_session_exact_with(&replacement_handle, None, |session| {
                session.pending_auth_transaction_id = Some("replacement-transaction".to_string());
                session.pending_auth_method = Some("INFO".to_string());
            })
            .expect("stage replacement auth owner");

        assert_eq!(
            store
                .get_session_snapshot_exact(&replacement_handle)
                .expect("replacement lifetime remains")
                .pending_auth_transaction_id
                .as_deref(),
            Some("replacement-transaction")
        );
    }

    #[tokio::test]
    async fn async_action_exposes_no_intermediate_state_and_success_commits_once() {
        let coordinator = input_admission_coordinator("single-success-commit").await;
        let machine = state_machine_with_table(
            &coordinator,
            accept_call_table(
                Vec::new(),
                vec![Action::SetCondition(Condition::DialogEstablished, true)],
                Some(CallState::Initiating),
            ),
        );
        let (session_id, handle) =
            create_input_admission_session(&machine, "single-success-commit", None).await;
        let before = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read pre-event revision");
        let barrier = action_execution_barrier::install(&session_id);

        let in_flight = {
            let machine = Arc::clone(&machine);
            let session_id = session_id.clone();
            tokio::spawn(async move {
                machine
                    .process_event(&session_id, EventType::AcceptCall)
                    .await
            })
        };
        barrier.wait_until_entered().await;

        let during_action = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read store while action is paused");
        assert_eq!(during_action.revision(), before.revision());
        assert_eq!(during_action.call_state, CallState::Idle);
        assert!(!during_action.dialog_established);

        barrier.release();
        in_flight
            .await
            .expect("join successful transition")
            .expect("execute successful transition");

        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read final successful revision");
        assert_eq!(committed.revision(), before.revision() + 1);
        assert_eq!(committed.call_state, CallState::Initiating);
        assert!(committed.dialog_established);

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown successful-commit coordinator");
    }

    #[tokio::test]
    async fn action_failure_exposes_no_intermediate_state_and_commits_once() {
        let coordinator = input_admission_coordinator("single-failure-commit").await;
        let machine = state_machine_with_table(
            &coordinator,
            accept_call_table(
                Vec::new(),
                vec![
                    Action::SetCondition(Condition::DialogEstablished, true),
                    Action::PrepareEarlyMediaSDP,
                ],
                Some(CallState::Initiating),
            ),
        );
        let (session_id, handle) =
            create_input_admission_session(&machine, "single-failure-commit", None).await;
        let before = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read pre-event failure revision");
        let barrier = action_execution_barrier::install(&session_id);

        let in_flight = {
            let machine = Arc::clone(&machine);
            let session_id = session_id.clone();
            tokio::spawn(async move {
                machine
                    .process_event(&session_id, EventType::AcceptCall)
                    .await
            })
        };
        barrier.wait_until_entered().await;

        let during_action = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read store while failing action sequence is paused");
        assert_eq!(during_action.revision(), before.revision());
        assert_eq!(during_action.call_state, CallState::Idle);
        assert!(!during_action.dialog_established);

        barrier.release();
        let error = in_flight
            .await
            .expect("join failing transition")
            .expect_err("missing SDP must fail PrepareEarlyMediaSDP");
        assert!(error.to_string().contains("no caller-supplied SDP"));

        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read final action-failure revision");
        assert_eq!(committed.revision(), before.revision() + 1);
        assert_eq!(committed.call_state, CallState::Initiating);
        assert!(
            committed.dialog_established,
            "mutations from ordered actions before the failure remain committed"
        );

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown failure-commit coordinator");
    }

    #[tokio::test]
    async fn action_failure_before_final_response_restores_pre_wire_state_and_envelope() {
        let coordinator = input_admission_coordinator("pre-wire-action-failure").await;
        let machine = state_machine_with_table(
            &coordinator,
            accept_call_table(
                Vec::new(),
                vec![
                    Action::SetCondition(Condition::DialogEstablished, true),
                    Action::PrepareEarlyMediaSDP,
                    Action::SendSIPResponse(200, "OK".to_string()),
                ],
                Some(CallState::Initiating),
            ),
        );
        let (_session_id, handle) =
            create_input_admission_session(&machine, "pre-wire-action-failure", None).await;
        let before = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read pre-wire input revision");

        let error = machine
            .process_event_with_response_input_exact(
                &handle,
                EventType::AcceptCall,
                ResponseStateInput::accept(None, Vec::new()),
            )
            .await
            .expect_err("missing SDP must fail before the ordered final response");
        assert!(error.to_string().contains("no caller-supplied SDP"));

        let committed = machine
            .store
            .get_session_snapshot_exact(&handle)
            .expect("read retryable pre-wire publication");
        assert_eq!(committed.revision(), before.revision() + 1);
        assert_eq!(committed.call_state, CallState::Idle);
        assert_eq!(committed.entered_state_at, before.entered_state_at);
        assert!(
            committed.dialog_established,
            "completed lane-owned preparation remains committed for retry"
        );
        assert!(
            committed.reject_response_extras.is_some(),
            "the exact response envelope remains available to the retry"
        );

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown pre-wire failure coordinator");
    }

    #[tokio::test]
    async fn fast_response_event_waits_for_lane_owned_state_publication() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("fast-response-lane-barrier".to_string());
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create exact session lifetime");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("current exact handle");
        let lane = store
            .state_machine_lane_exact(&handle)
            .expect("current exact state-machine lane");
        let lane_guard = lane.lock_owned().await;

        let mut working = store
            .get_session_exact(&handle)
            .await
            .expect("load lane-owned working state");
        working.local_sdp = Some("v=0\r\na=x-lane-owned\r\n".to_string());

        let response_store = Arc::clone(&store);
        let response_handle = handle.clone();
        let (attempting_tx, attempting_rx) = tokio::sync::oneshot::channel();
        let response = tokio::spawn(async move {
            let response_lane = response_store
                .state_machine_lane_exact(&response_handle)
                .expect("response resolves the same exact lane");
            let _ = attempting_tx.send(());
            let _response_guard = response_lane.lock_owned().await;
            response_store
                .get_session_snapshot_exact(&response_handle)
                .expect("response reads committed state")
                .local_sdp
                .clone()
        });

        attempting_rx
            .await
            .expect("response reached the lane barrier");
        tokio::task::yield_now().await;
        assert!(
            !response.is_finished(),
            "a fast response must queue behind the in-flight transition"
        );

        store
            .update_state_machine_session_and_snapshot(working)
            .expect("publish the lane-owned transition state");
        drop(lane_guard);

        assert_eq!(
            response.await.expect("response task completed").as_deref(),
            Some("v=0\r\na=x-lane-owned\r\n"),
            "the queued response must observe the canonical publication"
        );
    }

    #[tokio::test]
    async fn action_failure_publication_persists_zero_wire_clear_without_history() {
        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("zero-wire-failure-clear".to_string());
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create history-disabled session");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("current exact handle");
        store
            .update_session_exact_with(&handle, None, |session| {
                session.pending_bye_options = Some(Arc::new(
                    rvoip_sip_dialog::api::unified::ByeRequestOptions::default(),
                ));
            })
            .expect("stage retained BYE snapshot");

        let mut working = store
            .get_session_exact(&handle)
            .await
            .expect("load lane-owned state");
        assert!(working.history.is_none());
        working.call_state = CallState::Terminating;
        working.pending_bye_options = None;

        commit_lane_state(&store, working).expect("publish the action-failure working state");

        let committed = store
            .get_session_snapshot_exact(&handle)
            .expect("read committed failure state");
        assert_eq!(committed.call_state, CallState::Terminating);
        assert!(
            committed.pending_bye_options.is_none(),
            "zero-wire failure clear must not depend on diagnostic history"
        );
    }

    #[test]
    fn post_commit_transfer_scheduler_follows_canonical_commit() {
        let source = include_str!("executor.rs");
        let transition_commit = source
            .split("async fn process_one_event")
            .nth(1)
            .and_then(|tail| tail.split("fn should_skip_action").next())
            .expect("single-event canonical commit source");
        let commit = transition_commit
            .find("let published = match commit_lane_state(&self.store, session)")
            .expect("canonical exact-state publication");
        let committed_snapshot = transition_commit
            .find("let session = published.state()")
            .expect("immutable canonical publication snapshot");
        let schedule = transition_commit
            .find("self.schedule_deferred_action_effects")
            .expect("post-commit effect scheduler");
        assert!(commit < committed_snapshot);
        assert!(committed_snapshot < schedule);

        let scheduler = source
            .split("fn schedule_deferred_action_effects")
            .nth(1)
            .and_then(|tail| tail.split("// Callback registry removed").next())
            .expect("deferred effect scheduler source");
        assert!(scheduler.contains("DeferredActionEffect::TransferNotify"));
        assert!(scheduler.contains("run_deferred_transfer_notify"));
    }

    #[tokio::test]
    async fn post_commit_transfer_descriptor_is_inert_until_admission() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let store = Arc::new(SessionStore::new());
        let session_id = SessionId("post-commit-transfer-observation".to_string());
        store
            .create_session(session_id.clone(), crate::state_table::Role::UAC, false)
            .await
            .expect("create exact transfer session");
        let handle = store
            .lifecycle_handle(&session_id)
            .expect("current exact handle");
        let effect = actions::DeferredActionEffect::TransferNotify(actions::TransferNotifyEffect {
            transferor: handle.clone(),
            status_code: 180,
            reason: "Ringing".to_string(),
            observations: vec![crate::api::events::Event::ReferProgress {
                call_id: session_id,
                status_code: 180,
                reason: "Ringing".to_string(),
            }],
        });
        let observed = AtomicUsize::new(0);
        assert_eq!(observed.load(Ordering::SeqCst), 0);

        let mut working = store
            .get_session_exact(&handle)
            .await
            .expect("load lane-owned transfer state");
        working.call_state = CallState::Active;
        store
            .update_state_machine_session_and_snapshot(working)
            .expect("commit transfer transition before observation");

        let actions::DeferredActionEffect::TransferNotify(effect) = effect else {
            panic!("expected transfer NOTIFY descriptor");
        };
        let [event] = <[_; 1]>::try_from(effect.observations).expect("one transfer observation");
        admit_committed_transfer_observation(event, |event| {
            assert!(matches!(
                event,
                crate::api::events::Event::ReferProgress {
                    status_code: 180,
                    ..
                }
            ));
            assert_eq!(
                store
                    .get_session_snapshot_exact(&handle)
                    .expect("observation sees exact committed state")
                    .call_state,
                CallState::Active
            );
            observed.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(observed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn deferred_transfer_notify_rejects_a_stale_registry_revision_before_wire() {
        let coordinator = input_admission_coordinator("stale-transfer-notify").await;
        let store = Arc::clone(&coordinator.helpers.state_machine.store);
        let dialog_adapter = Arc::clone(&coordinator.helpers.state_machine.dialog_adapter);
        let session_id = SessionId("stale-transfer-notify-session".to_string());
        store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create exact transferor lifetime");
        let current = store
            .lifecycle_handle(&session_id)
            .expect("capture current transferor lifetime");
        let stale = current.with_next_slot_revision_for_test();
        let effect = actions::TransferNotifyEffect {
            transferor: stale,
            status_code: 180,
            reason: "Ringing".to_string(),
            observations: vec![crate::api::events::Event::ReferProgress {
                call_id: session_id,
                status_code: 180,
                reason: "Ringing".to_string(),
            }],
        };
        let authority = Arc::clone(store.authority());
        let operation_store = Arc::clone(&store);
        let waiter = authority
            .spawn_owned_exact(
                current.key(),
                SessionOperationKind::Signaling,
                Duration::from_secs(2),
                move |operation| {
                    run_deferred_transfer_notify(operation, operation_store, dialog_adapter, effect)
                },
            )
            .expect("admit exact transferor operation");

        assert_eq!(waiter.await, Ok(DeferredTransferNotifyResult::Stale));

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown stale transfer fixture");
    }

    #[test]
    fn observational_event_publication_never_waits_for_receiver_capacity() {
        let session_id = SessionId("observer-pressure".to_string());
        let event = || SessionEvent::CallOnHold {
            session_id: session_id.clone(),
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(1);

        assert_eq!(
            publish_observational_event(&sender, event()),
            ObservationPublishOutcome::Published
        );
        assert_eq!(
            publish_observational_event(&sender, event()),
            ObservationPublishOutcome::Saturated
        );

        drop(receiver);
        assert_eq!(
            publish_observational_event(&sender, event()),
            ObservationPublishOutcome::Closed
        );
    }

    #[test]
    fn exact_retirement_accepts_only_neutral_refer_dispatch_rows() {
        assert!(is_exact_retirement_safe_dispatch_only_transition(
            &refer_dispatch_transition()
        ));

        let mut state_changing = refer_dispatch_transition();
        state_changing.next_state = Some(CallState::Active);
        assert!(!is_exact_retirement_safe_dispatch_only_transition(
            &state_changing
        ));

        let mut condition_changing = refer_dispatch_transition();
        condition_changing.condition_updates = ConditionUpdates::set_dialog_established(true);
        assert!(!is_exact_retirement_safe_dispatch_only_transition(
            &condition_changing
        ));

        let mut event_publishing = refer_dispatch_transition();
        event_publishing
            .publish_events
            .push(EventTemplate::CallTerminated);
        assert!(!is_exact_retirement_safe_dispatch_only_transition(
            &event_publishing
        ));

        let mut extra_action = refer_dispatch_transition();
        extra_action.actions.push(Action::SendINFOWithOptions);
        assert!(!is_exact_retirement_safe_dispatch_only_transition(
            &extra_action
        ));
    }

    #[test]
    fn exact_retirement_preserves_local_teardown_compatibility() {
        let transition = Transition {
            guards: Vec::new(),
            actions: vec![Action::SendBYEWithOptions],
            next_state: None,
            condition_updates: ConditionUpdates::none(),
            publish_events: Vec::new(),
        };
        assert!(is_exact_retirement_safe_dispatch_only_transition(
            &transition
        ));
    }

    #[test]
    fn cancelled_stage_cleanup_cannot_clear_newer_arc() {
        let session_id = SessionId("exact-stage-cleanup".to_string());
        let mut session = SessionState::new(session_id, crate::state_table::Role::UAC);
        let old = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions::default());
        let newer = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions::default());
        let old_slot = PendingOptionsSlot::Info(Arc::clone(&old));
        session.pending_info_options = Some(Arc::clone(&newer));

        assert!(!old_slot.clear_if_exact(&mut session));
        assert!(session
            .pending_info_options
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &newer)));

        let newer_slot = PendingOptionsSlot::Info(newer);
        assert!(newer_slot.clear_if_exact(&mut session));
        assert!(session.pending_info_options.is_none());
    }

    #[tokio::test]
    async fn dropping_unclaimed_stage_allows_immediate_same_method_restage() {
        let (store, _session_id, handle, guard, _old) =
            staged_info_guard("drop-then-immediate-restage").await;

        drop(guard);

        // There is intentionally no yield or await between dropping the
        // guard and attempting the replacement. Drop must synchronously make
        // the slot vacant instead of scheduling eventual cleanup.
        let replacement = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions::default());
        let replacement_slot = PendingOptionsSlot::Info(Arc::clone(&replacement));
        store
            .update_session_exact_with(&handle, None, |session| {
                replacement_slot.stage_if_vacant(session)
            })
            .expect("restage exact session revision")
            .expect("same-method restage must not observe a stale stage");

        store
            .with_session(handle.session_id(), |session| {
                assert!(session
                    .pending_info_options
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &replacement)));
            })
            .expect("read restaged session");
    }

    #[tokio::test]
    async fn stale_unclaimed_guard_cannot_clear_replacement_stage() {
        let (store, _session_id, handle, guard, old) =
            staged_info_guard("stale-guard-preserves-replacement").await;
        let replacement = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions::default());
        let replacement_slot = PendingOptionsSlot::Info(Arc::clone(&replacement));

        store
            .update_session_exact_with(&handle, None, |session| {
                assert!(PendingOptionsSlot::Info(Arc::clone(&old)).clear_if_exact(session));
                replacement_slot.stage_if_vacant(session)
            })
            .expect("replace exact session staging revision")
            .expect("replacement INFO stage is vacant");

        drop(guard);

        store
            .with_session(handle.session_id(), |session| {
                assert!(session
                    .pending_info_options
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &replacement)));
            })
            .expect("read replacement session");
    }

    #[test]
    fn unclaimed_stage_cleanup_does_not_require_tokio_runtime() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build fixture runtime");
        let (store, _session_id, handle, guard, _old) =
            runtime.block_on(staged_info_guard("drop-after-runtime"));
        drop(runtime);

        // This destructor runs with no entered or live Tokio runtime. The
        // exact slot still must be gone when Drop returns.
        drop(guard);

        store
            .with_session(handle.session_id(), |session| {
                assert!(session.pending_info_options.is_none());
            })
            .expect("read session after runtime shutdown");
    }

    #[test]
    fn cancellation_before_exact_claim_prevents_dispatch_ownership() {
        let session_id = SessionId("cancel-before-stage-claim".to_string());
        let mut session = SessionState::new(session_id, crate::state_table::Role::UAC);
        let options = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions::default());
        let slot = PendingOptionsSlot::Info(Arc::clone(&options));
        session.pending_info_options = Some(options);
        let claim = StageDispatchClaim::new(slot);

        assert!(
            claim.cancel_before_claim(),
            "dispatch must abort before claim"
        );
        assert!(claim.claim_exact(&mut session).is_err());
        assert!(
            session.pending_info_options.is_some(),
            "cancelled action must not consume the stage"
        );
    }

    #[test]
    fn deferred_register_claim_rejects_install_after_preclaim_cancellation() {
        let claim = StageDispatchClaim::new_deferred(
            rvoip_sip_core::Method::Register,
            PendingOptionsSlotKind::Register,
        );

        assert!(claim.cancel_before_claim());
        let error = claim
            .install_deferred_slot(PendingOptionsSlot::Register(Arc::new(
                rvoip_sip_dialog::api::unified::RegisterRequestOptions::default(),
            )))
            .expect_err("cancelled deferred claim must reject a late snapshot");
        assert!(matches!(
            error,
            crate::errors::SessionError::InvalidTransition(_)
        ));
    }

    #[test]
    fn registration_refresh_derives_retained_identity_and_exact_next_cseq() {
        use rvoip_sip_core::types::{header::HeaderName, headers::HeaderValue, TypedHeader};

        let mut session = SessionState::new(SessionId("refresh-options".to_string()), Role::UAC);
        session.registrar_uri = Some("sip:registrar.example.test".to_string());
        session.remote_uri = Some("sip:fallback.example.test".to_string());
        session.local_uri = Some("sip:alice@example.test".to_string());
        session.registration_contact = Some("sip:alice@192.0.2.10:5060".to_string());
        session.registration_call_id = Some("registration-call-id".to_string());
        session.registration_cseq = 41;
        session.registration_expires = Some(300);
        let header = TypedHeader::Other(
            HeaderName::Other("X-Refresh-Canary".to_string()),
            HeaderValue::Raw(b"lane-owned".to_vec()),
        );

        let options = registration_refresh_options(&session, Some(180), vec![header])
            .expect("derive exact refresh options");

        assert_eq!(options.registrar_uri, "sip:registrar.example.test");
        assert_eq!(options.aor_uri, "sip:alice@example.test");
        assert_eq!(options.contact_uri, "sip:alice@192.0.2.10:5060");
        assert_eq!(options.call_id.as_deref(), Some("registration-call-id"));
        assert_eq!(options.cseq, Some(42));
        assert_eq!(options.expires, 180);
        assert_eq!(options.extra_headers.len(), 1);
        assert!(options.refresh);
    }

    #[test]
    fn registration_refresh_rejects_missing_identity_and_cseq_overflow() {
        let mut session =
            SessionState::new(SessionId("invalid-refresh-options".to_string()), Role::UAC);
        session.registrar_uri = Some("sip:registrar.example.test".to_string());
        session.local_uri = Some("sip:alice@example.test".to_string());
        session.registration_contact = Some("sip:alice@192.0.2.10:5060".to_string());

        let missing_call_id = registration_refresh_options(&session, None, Vec::new())
            .expect_err("refresh without retained Call-ID must fail closed");
        assert!(matches!(
            missing_call_id,
            crate::errors::SessionError::InvalidTransition(_)
        ));

        session.registration_call_id = Some("registration-call-id".to_string());
        session.registration_cseq = u32::MAX;
        let overflow = registration_refresh_options(&session, None, Vec::new())
            .expect_err("refresh CSeq overflow must fail closed");
        assert!(matches!(
            overflow,
            crate::errors::SessionError::InvalidTransition(_)
        ));
    }

    #[test]
    fn cancellation_after_exact_claim_detaches_dispatch() {
        let session_id = SessionId("cancel-after-stage-claim".to_string());
        let mut session = SessionState::new(session_id, crate::state_table::Role::UAC);
        let options = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions::default());
        let slot = PendingOptionsSlot::Info(Arc::clone(&options));
        session.pending_info_options = Some(options);
        let claim = StageDispatchClaim::new(slot);

        assert!(claim.claim_exact(&mut session).is_ok());
        assert!(session.pending_info_options.is_none());
        assert!(
            !claim.cancel_before_claim(),
            "claimed/wire-started dispatch must detach rather than abort"
        );
    }

    #[test]
    fn retained_exact_claim_keeps_the_pointer_exact_retry_snapshot() {
        let session_id = SessionId("retained-stage-pointer-exact".to_string());
        let mut session = SessionState::new(session_id, crate::state_table::Role::UAC);
        let options =
            Arc::new(crate::api::send::outbound_call::OutboundCallOptionsSnapshot::default());
        let slot = PendingOptionsSlot::Invite(Arc::clone(&options));
        session.pending_invite_options = Some(Arc::clone(&options));
        let claim = StageDispatchClaim::new(slot);

        let claimed = claim
            .claim_retained_exact(&mut session)
            .expect("exact retained stage claim");

        let PendingOptionsSlot::Invite(claimed_options) = claimed else {
            panic!("retained claim changed its staged-options kind");
        };
        assert!(Arc::ptr_eq(&claimed_options, &options));
        assert!(session
            .pending_invite_options
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &options)));
    }

    #[test]
    fn retained_exact_claim_rejects_a_stale_replacement() {
        let session_id = SessionId("retained-stage-stale-replacement".to_string());
        let mut session = SessionState::new(session_id, crate::state_table::Role::UAC);
        let original =
            Arc::new(crate::api::send::outbound_call::OutboundCallOptionsSnapshot::default());
        let replacement =
            Arc::new(crate::api::send::outbound_call::OutboundCallOptionsSnapshot::default());
        let claim = StageDispatchClaim::new(PendingOptionsSlot::Invite(original));
        session.pending_invite_options = Some(Arc::clone(&replacement));

        assert!(claim.claim_retained_exact(&mut session).is_err());
        assert!(session
            .pending_invite_options
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &replacement)));
        assert!(
            claim.cancel_before_claim(),
            "a rejected stale claim must remain cancellable"
        );
    }

    #[test]
    fn cancellation_after_retained_exact_claim_detaches_dispatch() {
        let session_id = SessionId("cancel-after-retained-stage-claim".to_string());
        let mut session = SessionState::new(session_id, crate::state_table::Role::UAC);
        let options =
            Arc::new(crate::api::send::outbound_call::OutboundCallOptionsSnapshot::default());
        let slot = PendingOptionsSlot::Invite(Arc::clone(&options));
        session.pending_invite_options = Some(Arc::clone(&options));
        let claim = StageDispatchClaim::new(slot);

        claim
            .claim_retained_exact(&mut session)
            .expect("claim retained stage");
        assert!(
            !claim.cancel_before_claim(),
            "retained retry owner must detach rather than abort"
        );
        assert!(session
            .pending_invite_options
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &options)));
    }

    #[tokio::test(start_paused = true)]
    async fn reinvite_retry_backoff_does_not_hold_the_exact_lane() {
        use crate::session_store::state::PendingReinvite;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (store, handle) = retry_fixture("off-lane-glare-retry", PendingReinvite::Hold, 1).await;
        let effect = actions::ReinviteRetryEffect {
            kind: PendingReinvite::Hold,
            attempt: 1,
            backoff: Duration::from_secs(2),
        };
        let dispatched = Arc::new(AtomicUsize::new(0));
        let dispatched_by_retry = Arc::clone(&dispatched);
        let retry_store = Arc::clone(&store);
        let retry_handle = handle.clone();
        let waiter = store
            .authority()
            .spawn_owned_exact(
                handle.key(),
                SessionOperationKind::Signaling,
                Duration::from_secs(10),
                move |operation| {
                    run_deferred_reinvite_retry(
                        operation,
                        retry_store,
                        retry_handle,
                        effect,
                        move |kind| async move {
                            assert_eq!(kind, PendingReinvite::Hold);
                            dispatched_by_retry.fetch_add(1, Ordering::SeqCst);
                            Ok::<(), String>(())
                        },
                    )
                },
            )
            .expect("retain exact retry operation");

        tokio::task::yield_now().await;
        let lane = store
            .state_machine_lane_exact(&handle)
            .expect("current exact lane");
        let lane_guard = lane
            .try_lock_owned()
            .expect("retry backoff must not own the state-machine lane");

        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        assert_eq!(dispatched.load(Ordering::SeqCst), 0);

        drop(lane_guard);
        assert_eq!(waiter.await, Ok(DeferredReinviteRetryResult::Dispatched));
        assert_eq!(dispatched.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn reinvite_retry_rejects_a_stale_exact_registry_revision() {
        use crate::session_store::state::PendingReinvite;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (store, handle) = retry_fixture("stale-glare-retry", PendingReinvite::Resume, 2).await;
        let stale_handle = handle.with_next_slot_revision_for_test();
        let effect = actions::ReinviteRetryEffect {
            kind: PendingReinvite::Resume,
            attempt: 2,
            backoff: Duration::ZERO,
        };
        let dispatched = Arc::new(AtomicUsize::new(0));
        let dispatched_by_retry = Arc::clone(&dispatched);
        let retry_store = Arc::clone(&store);
        let waiter = store
            .authority()
            .spawn_owned_exact(
                handle.key(),
                SessionOperationKind::Signaling,
                Duration::from_secs(5),
                move |operation| {
                    run_deferred_reinvite_retry(
                        operation,
                        retry_store,
                        stale_handle,
                        effect,
                        move |_| async move {
                            dispatched_by_retry.fetch_add(1, Ordering::SeqCst);
                            Ok::<(), String>(())
                        },
                    )
                },
            )
            .expect("retain operation for stale-handle check");

        assert_eq!(waiter.await, Ok(DeferredReinviteRetryResult::Stale));
        assert_eq!(dispatched.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn teardown_cancels_and_drains_a_sleeping_reinvite_retry() {
        use crate::session_store::state::PendingReinvite;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (store, handle) =
            retry_fixture("cancelled-glare-retry", PendingReinvite::Hold, 1).await;
        let effect = actions::ReinviteRetryEffect {
            kind: PendingReinvite::Hold,
            attempt: 1,
            backoff: Duration::from_secs(60),
        };
        let dispatched = Arc::new(AtomicUsize::new(0));
        let dispatched_by_retry = Arc::clone(&dispatched);
        let retry_store = Arc::clone(&store);
        let retry_handle = handle.clone();
        let waiter = store
            .authority()
            .spawn_owned_exact(
                handle.key(),
                SessionOperationKind::Signaling,
                Duration::from_secs(90),
                move |operation| {
                    run_deferred_reinvite_retry(
                        operation,
                        retry_store,
                        retry_handle,
                        effect,
                        move |_| async move {
                            dispatched_by_retry.fetch_add(1, Ordering::SeqCst);
                            Ok::<(), String>(())
                        },
                    )
                },
            )
            .expect("retain sleeping retry");
        tokio::task::yield_now().await;

        let teardown_store = Arc::clone(&store);
        let teardown_handle = handle.clone();
        let teardown = tokio::spawn(async move {
            teardown_store
                .remove_session_exact(&teardown_handle)
                .await
                .expect("teardown drains retry supervisor");
        });

        assert_eq!(waiter.await, Ok(DeferredReinviteRetryResult::Cancelled));
        teardown.await.expect("teardown task joins");
        assert_eq!(dispatched.load(Ordering::SeqCst), 0);
        assert!(store.lifecycle_handle(handle.session_id()).is_none());
    }
}
