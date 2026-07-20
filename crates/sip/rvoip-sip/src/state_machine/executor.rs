use crate::state_table::SessionId;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info};

use crate::{
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
    cleanup_diag::{self, CleanupStage},
    session_registry::SessionRegistryHandle,
    session_store::{SessionState, SessionStore},
    state_table::{Action, EventTemplate, EventType, StateKey, Transition, MASTER_TABLE},
    types::CallState,
    // Event import removed - events handled by SessionCrossCrateEventHandler
};

use super::{actions, guards};

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

#[derive(Default)]
struct EventStateInput {
    remote_sdp: Option<String>,
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
enum StageDispatchClaimState {
    Unclaimed,
    Claimed,
    Cancelled,
}

/// Coordinates cancellation with the exact transfer of a builder's staged
/// request options into the authoritative outbound-request tracker.
///
/// The state mutex is deliberately acquired while the session cell is locked
/// by `SessionStore::update_session_exact_with`. This makes cancellation-before-
/// claim and claim-before-cancellation mutually exclusive: before claim the
/// dispatch task is aborted and no request reaches the wire; after claim the
/// task is detached on caller cancellation so it can finish transaction
/// activation and preserve the exact completion owner.
pub(crate) struct StageDispatchClaim {
    slot: PendingOptionsSlot,
    state: Mutex<StageDispatchClaimState>,
}

impl StageDispatchClaim {
    pub(crate) fn new(slot: PendingOptionsSlot) -> Self {
        Self {
            slot,
            state: Mutex::new(StageDispatchClaimState::Unclaimed),
        }
    }

    pub(crate) fn method(&self) -> rvoip_sip_core::Method {
        self.slot.method()
    }

    /// Claim and remove the exact staged Arc from the current session
    /// revision. Callers must invoke this from inside an exact session-cell
    /// update so the slot and cancellation state change atomically.
    pub(crate) fn claim_exact(
        &self,
        session: &mut SessionState,
    ) -> crate::errors::Result<PendingOptionsSlot> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match *state {
            StageDispatchClaimState::Cancelled => {
                return Err(crate::errors::SessionError::InvalidTransition(format!(
                    "outbound {} dispatch was cancelled before claiming staged options",
                    self.slot.method()
                )));
            }
            StageDispatchClaimState::Claimed => {
                return Err(crate::errors::SessionError::InvalidTransition(format!(
                    "outbound {} dispatch already claimed staged options",
                    self.slot.method()
                )));
            }
            StageDispatchClaimState::Unclaimed => {}
        }
        if !self.slot.clear_if_exact(session) {
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "outbound {} dispatch no longer owns its exact staged options",
                self.slot.method()
            )));
        }
        *state = StageDispatchClaimState::Claimed;
        Ok(self.slot.clone())
    }

    /// Return true when the dispatch task must be aborted. Once the exact
    /// stage has been claimed, dropping the caller instead detaches the task.
    pub(crate) fn cancel_before_claim(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match *state {
            StageDispatchClaimState::Unclaimed => {
                *state = StageDispatchClaimState::Cancelled;
                true
            }
            StageDispatchClaimState::Cancelled => true,
            StageDispatchClaimState::Claimed => false,
        }
    }

    fn is_claimed(&self) -> bool {
        *self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            == StageDispatchClaimState::Claimed
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
    fn new(
        store: Arc<SessionStore>,
        handle: SessionRegistryHandle,
        slot: PendingOptionsSlot,
    ) -> Self {
        let dispatch_claim = Arc::new(StageDispatchClaim::new(slot.clone()));
        Self {
            store,
            handle,
            slot,
            dispatch_claim,
            armed: true,
        }
    }

    pub(crate) fn dispatch_claim(&self) -> Arc<StageDispatchClaim> {
        Arc::clone(&self.dispatch_claim)
    }

    pub(crate) async fn confirm_consumed(mut self) -> crate::errors::Result<()> {
        if !self.dispatch_claim.is_claimed() {
            self.dispatch_claim.cancel_before_claim();
            let _ = self
                .store
                .clear_staged_options_exact(&self.handle, |session| {
                    self.slot.clear_if_exact(session)
                });
            self.armed = false;
            return Err(crate::errors::SessionError::InvalidTransition(format!(
                "outbound {} dispatch did not consume its exact staged options",
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
            .clear_staged_options_exact(&self.handle, |session| self.slot.clear_if_exact(session));
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
        _ => "Other",
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
        _ => "state-machine-action",
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
            auto_180_ringing,
            // SimplePeer events handled by SessionCrossCrateEventHandler
        }
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
        // Stage through the same exact-lifetime lane as event execution. If
        // an older transition is still publishing a full working snapshot,
        // staging outside this lane lets that publication erase the builder's
        // immutable request options before the matching event can consume it.
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
                    outbound_request_tracker.has_request(session_id, tracked_method)
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

    pub(crate) async fn stage_outbound_options_guarded(
        &self,
        session_id: &SessionId,
        slot: PendingOptionsSlot,
    ) -> Result<PendingOptionsStageGuard, Box<dyn std::error::Error + Send + Sync>> {
        let (handle, _state_machine_lane) = self.acquire_state_machine_lane(session_id).await?;
        self.stage_outbound_options_exact(&handle, slot.clone())?;
        Ok(PendingOptionsStageGuard::new(
            Arc::clone(&self.store),
            handle,
            slot,
        ))
    }

    /// Acquire the complete-event lane for one exact session lifetime.
    ///
    /// A transition publishes its next state before executing async actions.
    /// Without this lane, a response-driven event can clone that intermediate
    /// revision and both events can later publish full snapshots in opposite
    /// order, erasing fields owned by the other transition. The lane lives on
    /// `SessionStateCell`, so there is no global contention and raw-ID reuse
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

    pub(crate) async fn process_event_with_stage_claim(
        &self,
        session_id: &SessionId,
        event: EventType,
        stage_claim: Arc<StageDispatchClaim>,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let (handle, _state_machine_lane) = self.acquire_state_machine_lane(session_id).await?;
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!("{}:{}", session_id, state_machine_event_name(&event)),
        );
        let result =
            Box::pin(self.process_event_inner(&handle, event, Some(stage_claim), None)).await;
        match &result {
            Ok(_) => guard.finish_success(),
            Err(_) => guard.finish_failure(),
        }
        result
    }

    /// Process a response event while applying its SDP only after acquiring
    /// the exact session lane. Writing SDP in the cross-crate handler before
    /// waiting would let an older in-flight transition overwrite that input.
    pub(crate) async fn process_event_with_remote_sdp(
        &self,
        session_id: &SessionId,
        event: EventType,
        remote_sdp: Option<String>,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let (handle, _state_machine_lane) = self.acquire_state_machine_lane(session_id).await?;
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!("{}:{}", session_id, state_machine_event_name(&event)),
        );
        let input = EventStateInput { remote_sdp };
        let result = Box::pin(self.process_event_inner(&handle, event, None, Some(input))).await;
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
        let auth_required_event = matches!(&event, EventType::AuthRequired { .. });

        // 1. Get current session state
        let mut session = match self.store.get_session_exact(handle).await {
            Ok(s) => s,
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
        if let Some(input) = state_input {
            if let Some(remote_sdp) = input.remote_sdp {
                session.remote_sdp = Some(remote_sdp);
            }
        }
        let old_state = session.call_state;

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
                session.remote_uri = Some(from.clone());
                if let Some(sdp_data) = sdp {
                    session.remote_sdp = Some(sdp_data.clone());
                }
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
                    && session.pending_reinvite.is_some()
                {
                    info!(
                        "RFC 3261 §14.1 UAS-side glare: peer re-INVITE arrived while \
                         our builder-API re-INVITE is in flight on session {} — \
                         responding 491 Request Pending",
                        session.session_id
                    );
                    if let Err(e) = self
                        .dialog_adapter
                        .send_response(&session.session_id, 491, None)
                        .await
                    {
                        tracing::warn!(
                            "Failed to send 491 Request Pending for session {}: {}",
                            session.session_id,
                            e
                        );
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
                if session.history.is_some() {
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
                    session.record_transition(record);
                    self.store
                        .update_state_machine_session_and_snapshot(session, auth_required_event)?;
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
                if session.history.is_some() {
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
                    session.record_transition(record);
                    self.store
                        .update_state_machine_session_and_snapshot(session, auth_required_event)?;
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

        // Apply next_state and persist BEFORE executing actions so that any
        // follow-up event queued by an action observes the post-transition
        // state. If an action fails after this point the state change stays
        // committed — mirrors how most state machines handle partial
        // side-effect failures (caller sees the error and decides how to
        // recover).
        let mut transition_state_published_before_actions = false;
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

            // Only actions can expose the transition to concurrent protocol
            // work before the final publication below.  State-only rows do
            // not need an intermediate full-state clone and revision.
            if transition
                .actions
                .iter()
                .any(|action| !self.should_skip_action(action))
            {
                self.store.update_state_machine_session_and_snapshot(
                    session.clone(),
                    auth_required_event,
                )?;
                transition_state_published_before_actions = true;
            }
        }

        // 5. Execute actions
        let mut actions_executed = Vec::new();
        for action in &transition.actions {
            if self.should_skip_action(action) {
                continue;
            }
            let action_start = Instant::now();
            let result = Box::pin(actions::execute_action(
                action,
                &event,
                &mut session,
                &self.dialog_adapter,
                &self.media_adapter,
                &self.store,
                self.auto_180_ringing,
                &None, // No SimplePeer event channel - handled by SessionCrossCrateEventHandler
                stage_claim.map(Arc::as_ref),
            ))
            .await;
            let action_duration = action_start.elapsed().as_millis() as u64;

            let (success, error_opt, exec_error) = match result {
                Ok(outcome) => {
                    actions_executed.push(action.clone());
                    queued_follow_up_events.extend(outcome.follow_up_events);
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
                // Record failed action in history
                if session.history.is_some() {
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
                        actions_executed: actions_executed_history,
                        duration_ms: transition_start.elapsed().as_millis() as u64,
                        errors,
                        events_published: vec![],
                    };
                    session.record_transition(record);
                    self.store
                        .update_state_machine_session_and_snapshot(session, auth_required_event)?;
                }

                return Err(exec_error.unwrap());
            }
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
        if session.history.is_some() {
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

        // Before saving, check whether a *concurrent* process_event (e.g.
        // the Dialog200OK handler that fired during our own SendReINVITE
        // await) has since committed a different `call_state`. If so,
        // preserve its commit — overwriting would race-clobber the
        // response-driven transition (e.g. HoldPending → OnHold).
        if let Ok(current) = self.store.get_session_snapshot_exact(handle) {
            let current_call_state = current.call_state.clone();
            if current_call_state != session.call_state
                && (transition_state_published_before_actions || current_call_state != old_state)
            {
                debug!(
                    "session {} call_state changed during action phase ({:?} -> {:?}); preserving store value",
                    session_id, session.call_state, current_call_state
                );
                session.call_state = current_call_state;
                session.entered_state_at = current.entered_state_at;
            }
            if current.sdp_origin_version > session.sdp_origin_version {
                session.sdp_origin_session_id = current.sdp_origin_session_id.clone();
                session.sdp_origin_version = current.sdp_origin_version;
            }
            if session.media_security.is_none() {
                session.media_security = current.media_security.clone();
            }
        }

        // 8. Move the event-local state into the store and retain the exact
        // immutable revision that was published.  The old path cloned the
        // complete state here and then loaded the cell again below; both are
        // on every successful transition's hot path.
        let lifecycle_handle = session.lifecycle_handle.clone();
        let published = match self
            .store
            .update_state_machine_session_and_snapshot(session, auth_required_event)
        {
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

        // 9. Publish events (if channel is available)
        if let Some(ref event_tx) = self.event_tx {
            for event_template in &transition.publish_events {
                let event = self
                    .instantiate_event(event_template, &session, old_state)
                    .await;
                let guard = cleanup_diag::stage_guard(
                    CleanupStage::StateMachineEventPublish,
                    &session.session_id.0,
                );
                match event_tx.send(event).await {
                    Ok(()) => guard.finish_success(),
                    Err(e) => {
                        guard.finish_failure();
                        error!("Failed to publish event: {}", e);
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
    use crate::state_table::ConditionUpdates;

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

    fn refer_dispatch_transition() -> Transition {
        Transition {
            guards: Vec::new(),
            actions: vec![Action::SendREFERWithOptions],
            next_state: None,
            condition_updates: ConditionUpdates::none(),
            publish_events: Vec::new(),
        }
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
}
