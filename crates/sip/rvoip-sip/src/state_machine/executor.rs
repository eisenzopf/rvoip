use crate::state_table::SessionId;
use std::sync::Arc;
use tracing::{debug, error, info};

use crate::{
    adapters::{dialog_adapter::DialogAdapter, media_adapter::MediaAdapter},
    cleanup_diag::{self, CleanupStage},
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
    /// matching `pending_<method>_options` slot on the session, and if
    /// it is `None` write the provided `Arc<XxxRequestOptions>`. If the
    /// slot is already `Some` (a prior `.send()` is still in flight for
    /// the same method on this session) return
    /// `Err(SessionError::Conflict { method })` without mutating
    /// anything.
    ///
    /// Builders call this *before* queuing the matching
    /// `EventType::SendOutbound<METHOD>` event so the state-table
    /// transition's `Action::Send<METHOD>WithOptions` handler can read
    /// from a populated stash. The set-once / consumed-once invariant
    /// is enforced here (and cleared by
    /// `Action::ClearPending<METHOD>Options` on final response or by
    /// the executor's `Terminated` backstop).
    pub async fn stage_outbound_options(
        &self,
        session_id: &SessionId,
        slot: PendingOptionsSlot,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut session = self.store.get_session(session_id).await.map_err(|e| {
            Box::<dyn std::error::Error + Send + Sync>::from(format!(
                "stage_outbound_options: session {} not found: {}",
                session_id, e
            ))
        })?;

        let method = slot.method();
        let occupied = match &slot {
            PendingOptionsSlot::Invite(_) => session.pending_invite_options.is_some(),
            PendingOptionsSlot::ReInvite(_) => session.pending_reinvite_options.is_some(),
            PendingOptionsSlot::Register(_) => session.pending_register_options.is_some(),
            PendingOptionsSlot::Refer(_) => session.pending_refer_options.is_some(),
            PendingOptionsSlot::Bye(_) => session.pending_bye_options.is_some(),
            PendingOptionsSlot::Cancel(_) => session.pending_cancel_options.is_some(),
            PendingOptionsSlot::Notify(_) => session.pending_notify_options.is_some(),
            PendingOptionsSlot::Subscribe(_) => session.pending_subscribe_options.is_some(),
            PendingOptionsSlot::Info(_) => session.pending_info_options.is_some(),
            PendingOptionsSlot::Update(_) => session.pending_update_options.is_some(),
            PendingOptionsSlot::Message(_) => session.pending_message_options.is_some(),
            PendingOptionsSlot::Options(_) => session.pending_options_options.is_some(),
        };

        if occupied {
            return Err(crate::errors::SessionError::Conflict { method }.into());
        }

        match slot {
            PendingOptionsSlot::Invite(a) => session.pending_invite_options = Some(a),
            PendingOptionsSlot::ReInvite(a) => session.pending_reinvite_options = Some(a),
            PendingOptionsSlot::Register(a) => session.pending_register_options = Some(a),
            PendingOptionsSlot::Refer(a) => session.pending_refer_options = Some(a),
            PendingOptionsSlot::Bye(a) => session.pending_bye_options = Some(a),
            PendingOptionsSlot::Cancel(a) => session.pending_cancel_options = Some(a),
            PendingOptionsSlot::Notify(a) => session.pending_notify_options = Some(a),
            PendingOptionsSlot::Subscribe(a) => session.pending_subscribe_options = Some(a),
            PendingOptionsSlot::Info(a) => session.pending_info_options = Some(a),
            PendingOptionsSlot::Update(a) => session.pending_update_options = Some(a),
            PendingOptionsSlot::Message(a) => session.pending_message_options = Some(a),
            PendingOptionsSlot::Options(a) => session.pending_options_options = Some(a),
        }

        self.store.update_session(session).await?;
        Ok(())
    }

    /// Process an event for a session
    pub async fn process_event(
        &self,
        session_id: &SessionId,
        event: EventType,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        let guard = cleanup_diag::stage_guard(
            state_machine_stage_for_event(&event),
            format!("{}:{}", session_id, state_machine_event_name(&event)),
        );
        let result = self.process_event_inner(session_id, event).await;
        match &result {
            Ok(_) => guard.finish_success(),
            Err(_) => guard.finish_failure(),
        }
        result
    }

    async fn process_event_inner(
        &self,
        session_id: &SessionId,
        event: EventType,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        use std::collections::VecDeque;

        const MAX_INTERNAL_EVENTS: usize = 32;

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

            let result = self
                .process_one_event(session_id, event, &mut queue)
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
        session_id: &SessionId,
        event: EventType,
        queued_follow_up_events: &mut std::collections::VecDeque<EventType>,
    ) -> Result<ProcessEventResult, Box<dyn std::error::Error + Send + Sync>> {
        use crate::session_store::history::history_event_snapshot;
        use crate::session_store::{ActionRecord, GuardResult, TransitionRecord};
        use std::time::Instant;

        debug!(
            "Processing event {} for session {}",
            state_machine_event_name(&event),
            session_id
        );
        let transition_start = Instant::now();
        let history_event = history_event_snapshot(&event);

        // 1. Get current session state
        let mut session = match self.store.get_session(session_id).await {
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
            EventType::SendEarlyMedia { sdp } => {
                if let Some(sdp_data) = sdp {
                    session.early_media_sdp = Some(sdp_data.clone());
                }
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
                session.transfer_notify_dialog = session.dialog_id.clone();
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
            EventType::UpdateReceived { sdp } => {
                // RFC 4028 UPDATE for session-timer refresh carries no SDP,
                // but if a peer sends an UPDATE body (RFC 3311 session
                // modification), record it so a future transition with
                // NegotiateSDPAsUAS can act on it.
                if let Some(sdp_data) = sdp {
                    session.remote_sdp = Some(sdp_data.clone());
                    session.sdp_negotiated = false;
                }
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
                    self.store.update_session(session).await?;
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
                    self.store.update_session(session).await?;
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
                session.pending_invite_options = None;
                session.invite_authorization_credentials.clear();
                session.invite_auth_retry_count = 0;
                session.pending_reinvite_options = None;
                session.pending_register_options = None;
                session.pending_refer_options = None;
                session.pending_bye_options = None;
                session.pending_cancel_options = None;
                session.pending_notify_options = None;
                session.pending_subscribe_options = None;
                session.pending_info_options = None;
                session.pending_update_options = None;
                session.pending_message_options = None;
                session.pending_options_options = None;
            }

            self.store.update_session(session.clone()).await?;
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
                &mut session,
                &self.dialog_adapter,
                &self.media_adapter,
                &self.store,
                self.auto_180_ringing,
                &None, // No SimplePeer event channel - handled by SessionCrossCrateEventHandler
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
                    self.store.update_session(session).await?;
                }

                return Err(exec_error.unwrap());
            }
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
        if let Ok(current) = self.store.get_session(session_id).await {
            if current.call_state != session.call_state {
                debug!(
                    "session {} call_state changed during action phase ({:?} -> {:?}); preserving store value",
                    session_id, session.call_state, current.call_state
                );
                session.call_state = current.call_state;
                session.entered_state_at = current.entered_state_at;
            }
            if current.sdp_origin_version > session.sdp_origin_version {
                session.sdp_origin_session_id = current.sdp_origin_session_id;
                session.sdp_origin_version = current.sdp_origin_version;
            }
            if session.media_security.is_none() {
                session.media_security = current.media_security;
            }
        }

        // 8. Save updated session state
        self.store.update_session(session.clone()).await?;

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

        // 10. Reload session to pick up any changes made by actions
        // Actions like send_register may have updated the session (e.g., is_registered flag)
        let session = self
            .store
            .get_session(session_id)
            .await
            .map_err(|e| format!("Failed to reload session after actions: {}", e))?;

        // 11. Check if conditions trigger internal events
        let all_conditions_met = session.all_conditions_met();
        let call_established_triggered = session.call_established_triggered;

        // 12. Save the updated session state back to the store
        // CRITICAL: Session changes during process_event must be persisted!
        self.store
            .update_session(session)
            .await
            .map_err(|e| format!("Failed to save session state: {}", e))?;

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
