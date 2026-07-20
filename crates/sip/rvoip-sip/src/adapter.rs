//! `SipAdapter` — the [`rvoip_core::ConnectionAdapter`] implementation that
//! plugs the proven [`crate::api::UnifiedCoordinator`] surface into
//! [`rvoip_core::Orchestrator`].
//!
//! Per CARVE_PLAN §2 layering rule: every method here ultimately calls into
//! [`crate::api::UnifiedCoordinator`] (the sole sanctioned path to
//! [`rvoip_sip_dialog`] / [`rvoip_media_core`] from this crate). No new
//! state machine, no parallel SIP plumbing — just translation between the
//! [`rvoip_core`] vocabulary and the [`UnifiedCoordinator`] API.

use crate::api::events::Event as ApiEvent;
use crate::api::headers::SipRequestOptions;
use crate::api::unified::{Config as ApiConfig, InboundInviteObservation, UnifiedCoordinator};
use crate::originate::SipOriginateContext;
use crate::retained_tasks::RetainedTasks;
use crate::session_registry::SessionRegistryHandle;
use crate::types::CallState;
use crate::SessionId;
use chrono::Utc;
use dashmap::DashMap;
use futures::FutureExt;
use rvoip_core::adapter::{
    legacy_normalized_event_receiver, AdapterEvent, AdapterKind, AdapterLifecycleCapabilities,
    AdapterLifecycleSink, AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    ExternalConnectionReference, InboundConnectionContext, InboundContextError, InboundRoutingHint,
    InboundSignalingMetadata, OrchestratorAdapterEvent, OriginateRequest, OutboundActivation,
    RejectReason, SignatureHeaders, TerminalDelivery, TransferAttemptId, TransferStatus,
    TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as CoreResult, RvoipError};
use rvoip_core::identity::{AuthenticatedPrincipal, IdentityAssurance};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId as CoreSessionId};
use rvoip_core::message::Message;
use rvoip_core::stream::{MediaStream, MediaStreamHandle};
use rvoip_core::DataMessage;
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderName, HeaderValue, TypedHeader};
use rvoip_sip_core::types::uri::Scheme;
use rvoip_sip_core::Uri;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch, Mutex as AsyncMutex};
use tracing::{debug, warn};

const MAX_SIP_INBOUND_ALLOWLIST_HEADERS: usize = 32;
const MAX_PENDING_SIP_INBOUND_CONTEXTS: usize = 4_096;
const PENDING_SIP_INBOUND_CONTEXT_TTL: Duration = Duration::from_secs(120);
const PENDING_SIP_INBOUND_CONTEXT_REAPER_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(test)]
const SIP_INBOUND_EVENT_DELIVERY_TIMEOUT: Duration = Duration::from_secs(2);
const SIP_ADAPTER_EVENT_CAPACITY: usize = 256;
const DEFAULT_SIP_ACTIVE_CONNECTION_BUDGET: usize = 262_144;
const SIP_OUTBOUND_EVENT_STAGE_CAPACITY: usize = 32;
const MAX_SIP_OUTBOUND_TARGET_BYTES: usize = 4_096;
const SIP_RETAINED_TASK_TIMEOUT: Duration = Duration::from_secs(2);
const SIP_OUTBOUND_ACTIVATION_TIMEOUT: Duration = Duration::from_secs(30);
const SIP_CONFIRMED_HANGUP_TIMEOUT: Duration = Duration::from_secs(3);
const SIP_ADAPTER_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
const SIP_RFC4733_INTER_DIGIT_MS: u32 =
    rvoip_media_core::relay::controller::dtmf_transmitter::DEFAULT_DTMF_INTER_DIGIT_MS;

fn parse_sip_info_dtmf(request: &crate::api::incoming::IncomingRequest) -> Option<(String, u32)> {
    let wire = request.raw_request()?;
    let content_type = wire.raw_header_value(&HeaderName::ContentType)?;
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/dtmf-relay"))
    {
        return None;
    }
    let body = std::str::from_utf8(wire.body()).ok()?;
    let mut signal = None;
    let mut duration_ms = 100_u32;
    for line in body.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("signal") {
            signal = Some(value.trim().to_ascii_uppercase());
        } else if name.trim().eq_ignore_ascii_case("duration") {
            duration_ms = value.trim().parse::<u32>().ok()?.clamp(40, 5_000);
        }
    }
    let signal = signal?;
    if signal.len() != 1
        || !signal
            .chars()
            .all(|digit| matches!(digit, '0'..='9' | '*' | '#' | 'A'..='D'))
    {
        return None;
    }
    Some((signal, duration_ms))
}

#[derive(Clone)]
struct SipRouteEpoch {
    session_id: SessionId,
    connection_id: ConnectionId,
    owner: SipRouteEpochOwner,
}

/// One route-lifetime transfer reservation.
///
/// The current raw SIP `ApiEvent::Refer*` surface identifies only the call's
/// `SessionId`; it does not expose a REFER transaction or subscription key.
/// Consequently, a delayed NOTIFY cannot be distinguished from a later REFER
/// on the same live route. We fail closed by allowing one transfer submission
/// per exact route epoch. Final status clears `active` correlation while this
/// history entry remains until exact route teardown.
#[derive(Clone)]
struct SipTransferAttemptState {
    epoch: SipRouteEpoch,
    attempt_id: Option<TransferAttemptId>,
    active: bool,
}

#[derive(Clone)]
struct SipSessionBinding {
    connection_id: ConnectionId,
    owner: SipRouteBindingOwner,
}

#[derive(Clone)]
struct SipConnectionBinding {
    session_id: SessionId,
    owner: SipRouteBindingOwner,
}

#[derive(Clone)]
enum SipRouteBindingOwner {
    Prepared(Weak<SipOutboundRoute>),
    Admitted(SessionRegistryHandle),
}

#[derive(Clone)]
enum SipRouteEpochOwner {
    Prepared(Arc<SipOutboundRoute>),
    Admitted(SessionRegistryHandle),
}

impl SipRouteBindingOwner {
    fn epoch_owner(&self) -> Option<SipRouteEpochOwner> {
        match self {
            Self::Prepared(route) => route.upgrade().map(SipRouteEpochOwner::Prepared),
            Self::Admitted(handle) => Some(SipRouteEpochOwner::Admitted(handle.clone())),
        }
    }

    fn equivalent(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Prepared(left), Self::Prepared(right)) => Weak::ptr_eq(left, right),
            (Self::Admitted(left), Self::Admitted(right)) => left == right,
            _ => false,
        }
    }
}

impl SipRouteEpoch {
    fn admitted_handle(&self) -> Option<&SessionRegistryHandle> {
        match &self.owner {
            SipRouteEpochOwner::Admitted(handle) => Some(handle),
            SipRouteEpochOwner::Prepared(_) => None,
        }
    }

    fn matches_route(&self, route: &Arc<SipOutboundRoute>) -> bool {
        match &self.owner {
            SipRouteEpochOwner::Prepared(expected) => Arc::ptr_eq(expected, route),
            SipRouteEpochOwner::Admitted(handle) => {
                route.lifecycle_handle().as_ref() == Some(handle)
            }
        }
    }
}

impl PartialEq for SipRouteEpoch {
    fn eq(&self, other: &Self) -> bool {
        self.session_id == other.session_id
            && self.connection_id == other.connection_id
            && match (&self.owner, &other.owner) {
                (SipRouteEpochOwner::Prepared(left), SipRouteEpochOwner::Prepared(right)) => {
                    Arc::ptr_eq(left, right)
                }
                (SipRouteEpochOwner::Admitted(left), SipRouteEpochOwner::Admitted(right)) => {
                    left == right
                }
                _ => false,
            }
    }
}

impl Eq for SipRouteEpoch {}

impl fmt::Debug for SipRouteEpoch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipRouteEpoch")
            .field("session_id", &self.session_id)
            .field("connection_id", &self.connection_id)
            .field("admitted", &self.admitted_handle().is_some())
            .finish()
    }
}

struct SipRemovedEpoch {
    stream: Option<Arc<crate::media_stream::SipMediaStream>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SipOutboundRoutePhase {
    Prepared,
    Activating,
    Flushing,
    Active,
    Terminating,
    Terminated,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SipOutboundWireState {
    NotStarted,
    Possible,
    Sent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SipActivationFailure {
    RouteEnded,
    EventOverflow,
    EventPublication,
    InvalidPlan,
    InviteFailed,
    MediaFailed,
}

impl SipActivationFailure {
    fn into_error(self) -> RvoipError {
        let detail = match self {
            Self::RouteEnded => "SIP outbound route ended during activation",
            Self::EventOverflow => "SIP outbound lifecycle event stage overflowed",
            Self::EventPublication => "SIP outbound lifecycle event publication was unavailable",
            Self::InvalidPlan => "SIP outbound activation plan was invalid",
            Self::InviteFailed => "SIP outbound INVITE activation failed",
            Self::MediaFailed => "SIP outbound media activation failed",
        };
        RvoipError::Adapter(detail.to_string())
    }
}

#[derive(Clone, Debug)]
enum SipActivationCompletion {
    Succeeded(OutboundActivation),
    Failed(SipActivationFailure),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SipRouteStageDisposition {
    Retained,
    Forward,
    Discard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SipInboundPublication {
    Published,
    Backpressured,
    Retired,
}

struct SipOutboundRouteState {
    phase: SipOutboundRoutePhase,
    wire: SipOutboundWireState,
    events: VecDeque<AdapterEvent>,
    terminal: Option<AdapterEvent>,
    cleanup_event: Option<AdapterEvent>,
    remote_terminal_seen: bool,
    overflowed: bool,
    activation_completed: bool,
    cleanup_started: bool,
}

struct SipOutboundRoute {
    connection_id: ConnectionId,
    session_id: SessionId,
    lifecycle_handle: OnceLock<SessionRegistryHandle>,
    target: String,
    context: Arc<SipOriginateContext>,
    sip_call_id: Arc<str>,
    stream: Arc<crate::media_stream::SipMediaStream>,
    state: StdMutex<SipOutboundRouteState>,
    activation: watch::Sender<Option<SipActivationCompletion>>,
    cleanup: watch::Sender<Option<bool>>,
    cancel: watch::Sender<bool>,
}

impl SipOutboundRoute {
    fn new(
        connection_id: ConnectionId,
        session_id: SessionId,
        target: String,
        context: Arc<SipOriginateContext>,
        stream: Arc<crate::media_stream::SipMediaStream>,
    ) -> Arc<Self> {
        let (activation, _) = watch::channel(None);
        let (cleanup, _) = watch::channel(None);
        let (cancel, _) = watch::channel(false);
        let sip_call_id: Arc<str> =
            crate::adapters::dialog_adapter::deterministic_outbound_call_id(&session_id).into();
        Arc::new(Self {
            connection_id,
            session_id,
            lifecycle_handle: OnceLock::new(),
            target,
            context,
            sip_call_id,
            stream,
            state: StdMutex::new(SipOutboundRouteState {
                phase: SipOutboundRoutePhase::Prepared,
                wire: SipOutboundWireState::NotStarted,
                events: VecDeque::with_capacity(SIP_OUTBOUND_EVENT_STAGE_CAPACITY),
                terminal: None,
                cleanup_event: None,
                remote_terminal_seen: false,
                overflowed: false,
                activation_completed: false,
                cleanup_started: false,
            }),
            activation,
            cleanup,
            cancel,
        })
    }

    fn attach_lifecycle_handle(&self, handle: SessionRegistryHandle) -> bool {
        if handle.session_id() != &self.session_id {
            return false;
        }
        if let Some(existing) = self.lifecycle_handle.get() {
            return existing == &handle;
        }
        self.lifecycle_handle.set(handle).is_ok()
    }

    fn lifecycle_handle(&self) -> Option<SessionRegistryHandle> {
        self.lifecycle_handle.get().cloned()
    }

    fn claim_activation(&self) -> Result<bool, SipActivationFailure> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match state.phase {
            SipOutboundRoutePhase::Prepared => {
                if state.overflowed {
                    return Err(SipActivationFailure::EventOverflow);
                }
                state.phase = SipOutboundRoutePhase::Activating;
                Ok(true)
            }
            SipOutboundRoutePhase::Activating
            | SipOutboundRoutePhase::Flushing
            | SipOutboundRoutePhase::Active => Ok(false),
            SipOutboundRoutePhase::Terminating
            | SipOutboundRoutePhase::Terminated
            | SipOutboundRoutePhase::Failed => Err(SipActivationFailure::RouteEnded),
        }
    }

    fn begin_wire_send(&self) -> Result<(), SipActivationFailure> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.phase != SipOutboundRoutePhase::Activating || *self.cancel.borrow() {
            return Err(SipActivationFailure::RouteEnded);
        }
        if state.overflowed {
            return Err(SipActivationFailure::EventOverflow);
        }
        if state.remote_terminal_seen {
            return Err(SipActivationFailure::RouteEnded);
        }
        state.wire = SipOutboundWireState::Possible;
        Ok(())
    }

    fn mark_wire_sent(&self) {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .wire = SipOutboundWireState::Sent;
    }

    fn stage_event(&self, event: AdapterEvent) -> SipRouteStageDisposition {
        let terminal = matches!(
            event,
            AdapterEvent::Ended { .. } | AdapterEvent::Failed { .. }
        );
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let disposition = match state.phase {
            SipOutboundRoutePhase::Prepared
            | SipOutboundRoutePhase::Activating
            | SipOutboundRoutePhase::Flushing => {
                if terminal {
                    state.remote_terminal_seen = true;
                    if state.terminal.is_none() {
                        state.terminal = Some(event);
                    }
                    state.phase = SipOutboundRoutePhase::Terminating;
                } else if state.terminal.is_none() {
                    if state.events.len() == SIP_OUTBOUND_EVENT_STAGE_CAPACITY {
                        state.overflowed = true;
                    } else {
                        state.events.push_back(event);
                    }
                }
                SipRouteStageDisposition::Retained
            }
            SipOutboundRoutePhase::Active => {
                if terminal {
                    state.remote_terminal_seen = true;
                    if state.terminal.is_none() {
                        state.terminal = Some(event);
                    }
                    state.phase = SipOutboundRoutePhase::Terminating;
                    SipRouteStageDisposition::Retained
                } else if state.remote_terminal_seen {
                    SipRouteStageDisposition::Discard
                } else {
                    SipRouteStageDisposition::Forward
                }
            }
            SipOutboundRoutePhase::Terminating | SipOutboundRoutePhase::Failed => {
                if terminal && state.terminal.is_none() {
                    state.remote_terminal_seen = true;
                    state.terminal = Some(event);
                    SipRouteStageDisposition::Retained
                } else {
                    SipRouteStageDisposition::Discard
                }
            }
            SipOutboundRoutePhase::Terminated => SipRouteStageDisposition::Discard,
        };
        drop(state);
        if terminal && disposition == SipRouteStageDisposition::Retained {
            self.cancel.send_replace(true);
        }
        disposition
    }

    async fn publish_staged_events(
        &self,
        events: &mpsc::Sender<OrchestratorAdapterEvent>,
    ) -> Result<bool, SipActivationFailure> {
        let deadline = tokio::time::Instant::now() + SIP_RETAINED_TASK_TIMEOUT;
        let mut cancel = self.cancel.subscribe();
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.phase != SipOutboundRoutePhase::Activating
                || state.remote_terminal_seen
                || *self.cancel.borrow()
            {
                return Err(SipActivationFailure::RouteEnded);
            }
            if state.overflowed {
                return Err(SipActivationFailure::EventOverflow);
            }
            state.phase = SipOutboundRoutePhase::Flushing;
        }
        loop {
            let next = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if state.phase != SipOutboundRoutePhase::Flushing {
                    return Err(SipActivationFailure::RouteEnded);
                }
                if state.overflowed {
                    return Err(SipActivationFailure::EventOverflow);
                }
                match state.events.pop_front() {
                    Some(event) => Some(event),
                    None => {
                        let remote_terminal = state.remote_terminal_seen;
                        state.phase = if remote_terminal {
                            SipOutboundRoutePhase::Terminating
                        } else {
                            SipOutboundRoutePhase::Active
                        };
                        return Ok(remote_terminal);
                    }
                }
            };

            let Some(event) = next else {
                continue;
            };
            let publication = async {
                tokio::select! {
                    biased;
                    _ = wait_for_route_cancel(&mut cancel) => Err(()),
                    result = events.send(OrchestratorAdapterEvent::Public(event)) => {
                        result.map_err(|_| ())
                    }
                }
            };
            match tokio::time::timeout_at(deadline, publication).await {
                Ok(Ok(())) => {}
                Ok(Err(())) | Err(_) => {
                    return Err(SipActivationFailure::EventPublication);
                }
            }
        }
    }

    fn complete_activation_failure(&self, failure: SipActivationFailure) {
        let publish = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.activation_completed {
                false
            } else {
                state.activation_completed = true;
                if !matches!(
                    state.phase,
                    SipOutboundRoutePhase::Terminating | SipOutboundRoutePhase::Terminated
                ) {
                    state.phase = SipOutboundRoutePhase::Failed;
                }
                true
            }
        };
        if publish {
            self.activation
                .send_replace(Some(SipActivationCompletion::Failed(failure)));
        }
    }

    /// Commit successful activation for receipt publication only if that
    /// commit linearizes before every terminal/cancellation transition.
    /// Controls and media key off this commit point; watch notification follows
    /// synchronously. A terminal already visible makes every activation waiter
    /// fail instead of exposing a successful receipt for a dead route.
    fn complete_activation_success(&self, receipt: OutboundActivation) -> bool {
        let publish = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.activation_completed
                || state.phase != SipOutboundRoutePhase::Active
                || state.remote_terminal_seen
                || *self.cancel.borrow()
            {
                false
            } else {
                state.activation_completed = true;
                self.stream.activate_outbound_writes();
                true
            }
        };
        if publish {
            self.activation
                .send_replace(Some(SipActivationCompletion::Succeeded(receipt)));
        }
        publish
    }

    async fn wait_activation(&self) -> CoreResult<OutboundActivation> {
        let mut activation = self.activation.subscribe();
        loop {
            if let Some(completion) = activation.borrow_and_update().clone() {
                return match completion {
                    SipActivationCompletion::Succeeded(receipt) => Ok(receipt),
                    SipActivationCompletion::Failed(error) => Err(error.into_error()),
                };
            }
            activation.changed().await.map_err(|_| {
                RvoipError::InvalidState("SIP outbound activation supervisor ended")
            })?;
        }
    }

    fn request_cleanup(&self, event: Option<AdapterEvent>) -> bool {
        let start = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(event) = event {
                if state.cleanup_event.is_none() {
                    state.cleanup_event = Some(event);
                }
            }
            state.phase = match state.phase {
                SipOutboundRoutePhase::Terminated => SipOutboundRoutePhase::Terminated,
                _ => SipOutboundRoutePhase::Terminating,
            };
            if state.cleanup_started {
                false
            } else {
                state.cleanup_started = true;
                true
            }
        };
        self.cancel.send_replace(true);
        self.stream.request_close();
        start
    }

    fn cleanup_snapshot(&self) -> (SipOutboundWireState, bool) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (state.wire, state.remote_terminal_seen)
    }

    fn session_for_non_terminal_control(&self) -> CoreResult<SessionId> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.phase != SipOutboundRoutePhase::Active
            || !state.activation_completed
            || *self.cancel.borrow()
        {
            return Err(RvoipError::InvalidState(
                "SIP outbound route is not activated",
            ));
        }
        Ok(self.session_id.clone())
    }

    fn is_publicly_live(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        matches!(
            state.phase,
            SipOutboundRoutePhase::Prepared
                | SipOutboundRoutePhase::Activating
                | SipOutboundRoutePhase::Flushing
                | SipOutboundRoutePhase::Active
        ) && !*self.cancel.borrow()
    }

    fn seal_cleanup_event(&self) -> Option<AdapterEvent> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.phase = SipOutboundRoutePhase::Terminated;
        state.terminal.take().or_else(|| state.cleanup_event.take())
    }

    fn complete_cleanup(&self, success: bool) {
        self.cleanup.send_replace(Some(success));
    }

    fn cleanup_result(&self) -> Option<bool> {
        *self.cleanup.borrow()
    }

    async fn wait_cleanup(&self) -> CoreResult<()> {
        let mut cleanup = self.cleanup.subscribe();
        loop {
            if let Some(success) = *cleanup.borrow_and_update() {
                return if success {
                    Ok(())
                } else {
                    Err(RvoipError::Adapter(
                        "SIP outbound network cleanup failed".to_string(),
                    ))
                };
            }
            cleanup
                .changed()
                .await
                .map_err(|_| RvoipError::InvalidState("SIP outbound cleanup supervisor ended"))?;
        }
    }
}

impl Drop for SipOutboundRoute {
    fn drop(&mut self) {
        self.cancel.send_replace(true);
        self.stream.request_close();
    }
}

/// Configuration error for the SIP inbound signaling-metadata allowlist.
///
/// Variants carry no caller-supplied strings so error formatting cannot
/// accidentally disclose a header value or routing token.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum SipInboundContextPolicyError {
    /// More names were supplied than one inbound context can retain.
    #[error("too many SIP inbound context header names")]
    TooManyHeaders,
    /// A supplied name was empty, malformed, or too large.
    #[error("invalid SIP inbound context header name")]
    InvalidHeaderName,
    /// The name is owned by SIP routing/authentication or Bridgefu identity.
    #[error("forbidden SIP inbound context header name")]
    ForbiddenHeaderName,
}

/// Explicit allowlist for SIP headers exposed as inbound signaling metadata.
///
/// The default allowlist is empty. Request-URI routing remains independent of
/// this list. Only `X-*` application-extension headers are eligible; standard
/// SIP headers and the reserved `X-Bridgefu-*`/`X-Rvoip-*` namespaces remain
/// unavailable even when named explicitly.
#[derive(Clone, Default)]
pub struct SipInboundContextPolicy {
    allowed_headers: Arc<HashSet<String>>,
}

impl SipInboundContextPolicy {
    /// Build a policy from case-insensitive SIP header names.
    pub fn new<I, S>(allowed_headers: I) -> std::result::Result<Self, SipInboundContextPolicyError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut allowed = HashSet::new();
        for supplied in allowed_headers {
            let supplied = supplied.as_ref();
            if supplied.is_empty()
                || supplied.len() > rvoip_core_traits::adapter::MAX_INBOUND_METADATA_NAME_BYTES
                || !supplied.bytes().all(is_sip_metadata_name_byte)
            {
                return Err(SipInboundContextPolicyError::InvalidHeaderName);
            }
            let header = HeaderName::from_str(supplied)
                .map_err(|_| SipInboundContextPolicyError::InvalidHeaderName)?;
            if !sip_inbound_header_is_application_extension(&header) {
                return Err(SipInboundContextPolicyError::ForbiddenHeaderName);
            }
            let normalized = header.as_str().to_ascii_lowercase();
            if allowed.contains(&normalized) {
                continue;
            }
            if allowed.len() >= MAX_SIP_INBOUND_ALLOWLIST_HEADERS {
                return Err(SipInboundContextPolicyError::TooManyHeaders);
            }
            allowed.insert(normalized);
        }
        Ok(Self {
            allowed_headers: Arc::new(allowed),
        })
    }

    /// Number of distinct case-insensitive header names in the allowlist.
    pub fn allowed_header_count(&self) -> usize {
        self.allowed_headers.len()
    }

    fn captures(&self, header: &HeaderName) -> bool {
        self.allowed_headers
            .contains(&header.as_str().to_ascii_lowercase())
    }

    fn capture(
        &self,
        observation: &InboundInviteObservation,
    ) -> std::result::Result<Option<PendingSipInboundContext>, InboundContextError> {
        if observation.principal.is_none() {
            return Ok(None);
        }
        let (routing_hint, metadata) = if let Some(request) = observation.request.as_ref() {
            let routing_hint = request
                .uri()
                .username()
                .map(|username| InboundRoutingHint::new(username.to_owned()))
                .transpose()?;
            let metadata = request
                .headers
                .iter()
                .filter_map(|header| {
                    let name = header.name();
                    self.captures(&name).then(|| {
                        sip_header_value(header).map(|value| (name.as_str().to_owned(), value))
                    })
                })
                .collect::<Option<Vec<_>>>()
                .ok_or(InboundContextError::InvalidMetadataValue)?;
            (routing_hint, InboundSignalingMetadata::new(metadata)?)
        } else {
            // Authentication remains usable even when a legacy compatibility
            // event did not retain parseable raw INVITE bytes. The context is
            // still principal-bound; it simply carries no routing metadata.
            (None, InboundSignalingMetadata::default())
        };

        Ok(Some(PendingSipInboundContext {
            routing_hint,
            metadata,
        }))
    }
}

impl fmt::Debug for SipInboundContextPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SipInboundContextPolicy")
            .field("allowed_header_count", &self.allowed_headers.len())
            .finish()
    }
}

const fn is_sip_metadata_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn sip_inbound_header_is_application_extension(header: &HeaderName) -> bool {
    let HeaderName::Other(name) = header else {
        return false;
    };
    let normalized = name.to_ascii_lowercase();
    normalized.len() > 2
        && normalized.starts_with("x-")
        && normalized != "x-bridgefu"
        && normalized != "x-rvoip"
        && !normalized.starts_with("x-bridgefu-")
        && !normalized.starts_with("x-rvoip-")
}

fn sip_header_value(header: &TypedHeader) -> Option<String> {
    let rendered = header.to_string();
    let (rendered_name, value) = rendered.split_once(':')?;
    if !rendered_name
        .trim()
        .eq_ignore_ascii_case(header.name().as_str())
    {
        return None;
    }
    Some(value.trim().to_owned())
}

fn validate_sip_principal_at(
    principal: &AuthenticatedPrincipal,
    now: chrono::DateTime<Utc>,
) -> std::result::Result<(), InboundContextError> {
    if principal.tenant.as_deref().is_none_or(str::is_empty) {
        return Err(InboundContextError::MissingTenant);
    }
    if principal.is_expired_at(now) {
        return Err(InboundContextError::ExpiredPrincipal);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FailedInboundTermination {
    Reject,
    Hangup,
    CleanupOnly,
}

fn failed_inbound_termination(
    state: Option<CallState>,
    fast_auto_accept: bool,
) -> FailedInboundTermination {
    let Some(state) = state else {
        return FailedInboundTermination::CleanupOnly;
    };
    if state.is_final() || state == CallState::Terminating {
        return FailedInboundTermination::CleanupOnly;
    }
    if fast_auto_accept {
        return FailedInboundTermination::Hangup;
    }
    if matches!(
        state,
        CallState::Idle | CallState::Initiating | CallState::Ringing | CallState::EarlyMedia
    ) {
        FailedInboundTermination::Reject
    } else {
        FailedInboundTermination::Hangup
    }
}

struct PendingSipInboundContext {
    routing_hint: Option<InboundRoutingHint>,
    metadata: InboundSignalingMetadata,
}

struct PendingSipInboundObservation {
    observed_at: Instant,
    principal: Option<AuthenticatedPrincipal>,
    context: Option<Box<PendingSipInboundContext>>,
    context_error: Option<InboundContextError>,
}

enum SipInboundContextState {
    Available(InboundConnectionContext),
    Consumed,
}

enum SipInboundBinding {
    Observed(Option<AuthenticatedPrincipal>),
    Rejected(InboundContextError),
    Missing,
}

struct SipInboundContextStore {
    pending_by_session: StdMutex<HashMap<SessionId, PendingSipInboundObservation>>,
    by_connection: DashMap<ConnectionId, SipInboundContextState>,
    max_pending: usize,
    pending_ttl: Duration,
}

impl Default for SipInboundContextStore {
    fn default() -> Self {
        Self {
            pending_by_session: StdMutex::new(HashMap::new()),
            by_connection: DashMap::new(),
            max_pending: MAX_PENDING_SIP_INBOUND_CONTEXTS,
            pending_ttl: PENDING_SIP_INBOUND_CONTEXT_TTL,
        }
    }
}

impl SipInboundContextStore {
    #[cfg(test)]
    fn with_pending_limits(max_pending: usize, pending_ttl: Duration) -> Self {
        Self {
            max_pending,
            pending_ttl,
            ..Self::default()
        }
    }

    fn observe(
        &self,
        session_id: SessionId,
        principal: Option<AuthenticatedPrincipal>,
        context: Option<PendingSipInboundContext>,
    ) -> bool {
        self.observe_entry(session_id, principal, context, None)
    }

    fn observe_rejected(
        &self,
        session_id: SessionId,
        principal: Option<AuthenticatedPrincipal>,
        error: InboundContextError,
    ) -> bool {
        self.observe_entry(session_id, principal, None, Some(error))
    }

    fn observe_entry(
        &self,
        session_id: SessionId,
        principal: Option<AuthenticatedPrincipal>,
        context: Option<PendingSipInboundContext>,
        context_error: Option<InboundContextError>,
    ) -> bool {
        let now = Instant::now();
        let mut pending = self
            .pending_by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        pending.retain(|_, observation| {
            now.saturating_duration_since(observation.observed_at) <= self.pending_ttl
        });
        if pending.contains_key(&session_id) {
            return true;
        }
        if pending.len() >= self.max_pending {
            return false;
        }
        pending.insert(
            session_id,
            PendingSipInboundObservation {
                observed_at: now,
                principal,
                context: context.map(Box::new),
                context_error,
            },
        );
        true
    }

    fn bind(&self, session_id: &SessionId, connection_id: &ConnectionId) -> SipInboundBinding {
        let pending = self
            .pending_by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id)
            .filter(|observation| observation.observed_at.elapsed() <= self.pending_ttl);
        let Some(pending) = pending else {
            self.by_connection
                .insert(connection_id.clone(), SipInboundContextState::Consumed);
            return SipInboundBinding::Missing;
        };
        let principal = pending.principal;
        if let Some(error) = pending.context_error {
            self.by_connection
                .entry(connection_id.clone())
                .or_insert(SipInboundContextState::Consumed);
            return SipInboundBinding::Rejected(error);
        }

        let state = match (principal.as_ref(), pending.context) {
            (Some(principal), Some(pending)) => {
                let pending = *pending;
                match InboundConnectionContext::new(
                    connection_id.clone(),
                    Transport::Sip,
                    principal,
                    pending.routing_hint,
                    pending.metadata,
                ) {
                    Ok(context) => SipInboundContextState::Available(context),
                    Err(error) => {
                        self.by_connection
                            .entry(connection_id.clone())
                            .or_insert(SipInboundContextState::Consumed);
                        return SipInboundBinding::Rejected(error);
                    }
                }
            }
            (Some(_), None) => {
                self.by_connection
                    .entry(connection_id.clone())
                    .or_insert(SipInboundContextState::Consumed);
                return SipInboundBinding::Rejected(InboundContextError::InvalidMetadataValue);
            }
            (None, _) => SipInboundContextState::Consumed,
        };
        self.by_connection
            .entry(connection_id.clone())
            .or_insert(state);
        SipInboundBinding::Observed(principal)
    }

    fn has_pending(&self, session_id: &SessionId) -> bool {
        let now = Instant::now();
        let mut pending = self
            .pending_by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let is_fresh = pending.get(session_id).is_some_and(|observation| {
            now.saturating_duration_since(observation.observed_at) <= self.pending_ttl
        });
        if !is_fresh {
            pending.remove(session_id);
        }
        is_fresh
    }

    fn purge_expired(&self) -> usize {
        let now = Instant::now();
        let mut pending = self
            .pending_by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let before = pending.len();
        pending.retain(|_, observation| {
            now.saturating_duration_since(observation.observed_at) <= self.pending_ttl
        });
        before.saturating_sub(pending.len())
    }

    fn take(&self, connection_id: &ConnectionId) -> Option<InboundConnectionContext> {
        let mut entry = self.by_connection.get_mut(connection_id)?;
        match std::mem::replace(entry.value_mut(), SipInboundContextState::Consumed) {
            SipInboundContextState::Available(context) => Some(context),
            SipInboundContextState::Consumed => None,
        }
    }

    fn discard(&self, connection_id: &ConnectionId) {
        if let Some(mut entry) = self.by_connection.get_mut(connection_id) {
            *entry = SipInboundContextState::Consumed;
        }
    }

    fn forget(&self, session_id: &SessionId, connection_id: &ConnectionId) {
        self.forget_pending(session_id);
        self.by_connection.remove(connection_id);
    }

    fn forget_pending(&self, session_id: &SessionId) {
        self.pending_by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id);
    }

    fn clear(&self) {
        self.pending_by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.by_connection.clear();
    }

    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.pending_by_session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

/// SIP-protocol adapter. Wraps an [`UnifiedCoordinator`]; every
/// `ConnectionAdapter` method dispatches to it.
pub struct SipAdapter {
    /// Weak self-reference used by retained supervisors. Tasks never retain
    /// the adapter that owns their route registry.
    self_weak: Weak<SipAdapter>,
    coordinator: Arc<UnifiedCoordinator>,
    /// rvoip-core ConnectionId → SIP api SessionId.
    by_connection: Arc<DashMap<ConnectionId, SipConnectionBinding>>,
    /// SIP api SessionId → rvoip-core ConnectionId. Used by the event
    /// translator task to map outgoing api::Event → AdapterEvent.
    by_session: Arc<DashMap<SessionId, SipSessionBinding>>,
    /// Serializes paired forward/reverse mapping changes and exact removal.
    mapping_lock: StdMutex<()>,
    /// Route-lifetime transfer history. Every access is serialized by
    /// `mapping_lock` so reservation and exact epoch validation are atomic.
    transfer_attempts: DashMap<ConnectionId, SipTransferAttemptState>,
    /// Configurable active-route limit. Admission identity and anti-reuse are
    /// owned solely by the coordinator's `SessionLeaseAuthority`.
    active_connection_budget: AtomicUsize,
    /// One retained route owns local preparation, activation, event staging,
    /// media binding, receipt capture, and terminal compensation.
    outbound_routes: DashMap<ConnectionId, Arc<SipOutboundRoute>>,
    out_tx: mpsc::Sender<OrchestratorAdapterEvent>,
    /// Single-take receiver for [`ConnectionAdapter::subscribe_events`].
    out_rx: StdMutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    /// Cache of `SipMediaStream` instances. One stream per connection —
    /// the orchestrator-side `frames_in() / frames_out()` channels are
    /// single-take, so caching lets the orchestrator hand the same
    /// handle to the bridge pump and to a stats reader. Populated
    /// eagerly at connection-construction time so consumers that
    /// inspect `Connection.streams` off the `InboundConnection` event
    /// see a non-empty vec (QUIC/WT parity, gap plan §2.2).
    streams_cache: Arc<DashMap<ConnectionId, Arc<crate::media_stream::SipMediaStream>>>,
    inbound_contexts: Arc<SipInboundContextStore>,
    authenticated_inbound_sessions: DashMap<SessionId, ()>,
    lifecycle: AdapterLifecycleSinkSlot,
    translator_cancel: watch::Sender<bool>,
    retained_tasks: Arc<RetainedTasks>,
    draining: AtomicBool,
    drained: AtomicBool,
    /// The first destructive drain failure is sticky. Once route ownership
    /// has been retired, a later invocation must not infer success merely
    /// because the registries are empty.
    drain_failure: StdMutex<Option<String>>,
    #[cfg(test)]
    force_drain_compensation_failure: AtomicBool,
    drain_gate: AsyncMutex<()>,
    observer_registered: AtomicBool,
    inbound_invite_observer_id: u64,
}

impl SipAdapter {
    fn outbound_originate_context(
        request: &OriginateRequest,
    ) -> CoreResult<Arc<SipOriginateContext>> {
        let context = if request.context.is_empty() {
            Arc::new(SipOriginateContext::default())
        } else {
            request
                .context
                .downcast_arc::<SipOriginateContext>()
                .ok_or(RvoipError::AdmissionRejected(
                    "outbound SIP originate context type mismatch",
                ))?
        };
        context.validate().map_err(|_| {
            RvoipError::AdmissionRejected("outbound SIP originate context failed validation")
        })?;
        Ok(context)
    }

    fn validate_outbound_target(target: &str) -> CoreResult<()> {
        if target.is_empty()
            || target.len() > MAX_SIP_OUTBOUND_TARGET_BYTES
            || target.chars().any(char::is_control)
        {
            return Err(RvoipError::AdmissionRejected(
                "outbound SIP target failed local validation",
            ));
        }
        let uri = Uri::from_str(target).map_err(|_| {
            RvoipError::AdmissionRejected("outbound SIP target failed local validation")
        })?;
        if !matches!(uri.scheme, Scheme::Sip | Scheme::Sips) {
            return Err(RvoipError::AdmissionRejected(
                "outbound SIP target must use sip or sips",
            ));
        }
        Ok(())
    }

    /// Construct from a fully-configured [`UnifiedCoordinator`]. Spawns the
    /// background event-translation task; the returned `Arc<SipAdapter>` is
    /// what gets registered with [`rvoip_core::Orchestrator::register`].
    pub async fn new(coordinator: Arc<UnifiedCoordinator>) -> crate::errors::Result<Arc<Self>> {
        Self::new_with_inbound_context_policy(coordinator, SipInboundContextPolicy::default()).await
    }

    /// Construct with an explicit allowlist for sanitized inbound SIP
    /// signaling metadata. The routing hint is always derived solely from the
    /// parsed Request-URI username and is never taken from a header.
    pub async fn new_with_inbound_context_policy(
        coordinator: Arc<UnifiedCoordinator>,
        policy: SipInboundContextPolicy,
    ) -> crate::errors::Result<Arc<Self>> {
        // Open the event subscription before installing the synchronous
        // observer. Calls already in flight before installation safely have no
        // context; calls observed after installation cannot outrun this
        // receiver and lose their matching IncomingCall event.
        let mut events = coordinator.events_with_control().await?;
        let (out_tx, out_rx) = mpsc::channel(SIP_ADAPTER_EVENT_CAPACITY);
        let (translator_cancel, mut translator_cancel_rx) = watch::channel(false);
        let context_reaper_cancel_rx = translator_cancel.subscribe();
        let retained_tasks = RetainedTasks::new();
        let inbound_contexts = Arc::new(SipInboundContextStore::default());
        let contexts_for_observer = Arc::downgrade(&inbound_contexts);
        let inbound_invite_observer_id = coordinator.add_inbound_invite_observer(Arc::new(
            move |observation: InboundInviteObservation| {
                let Some(contexts) = contexts_for_observer.upgrade() else {
                    return;
                };
                let session_id = observation.session_id.clone();
                let principal = observation.principal.clone();
                let captured = policy.capture(&observation);
                let admitted = match captured {
                    Ok(context) => contexts.observe(session_id, principal, context),
                    Err(error) => {
                        warn!(
                            ?error,
                            "SipAdapter rejected malformed inbound signaling context"
                        );
                        contexts.observe_rejected(session_id, principal, error)
                    }
                };
                // First observation wins. Retransmitted/replayed INVITE
                // notifications cannot replace a context already tied to the
                // session's authenticated request.
                if !admitted {
                    warn!("SipAdapter inbound context admission capacity reached");
                }
            },
        ))?;
        let adapter = Arc::new_cyclic(|self_weak| Self {
            self_weak: self_weak.clone(),
            coordinator: Arc::clone(&coordinator),
            by_connection: Arc::new(DashMap::new()),
            by_session: Arc::new(DashMap::new()),
            mapping_lock: StdMutex::new(()),
            transfer_attempts: DashMap::new(),
            active_connection_budget: AtomicUsize::new(DEFAULT_SIP_ACTIVE_CONNECTION_BUDGET),
            outbound_routes: DashMap::new(),
            out_tx: out_tx.clone(),
            out_rx: StdMutex::new(Some(out_rx)),
            streams_cache: Arc::new(DashMap::new()),
            inbound_contexts,
            authenticated_inbound_sessions: DashMap::new(),
            lifecycle: AdapterLifecycleSinkSlot::default(),
            translator_cancel,
            retained_tasks: Arc::clone(&retained_tasks),
            draining: AtomicBool::new(false),
            drained: AtomicBool::new(false),
            drain_failure: StdMutex::new(None),
            #[cfg(test)]
            force_drain_compensation_failure: AtomicBool::new(false),
            drain_gate: AsyncMutex::new(()),
            observer_registered: AtomicBool::new(true),
            inbound_invite_observer_id,
        });

        // Subscribe to the coordinator's typed event stream and spawn the
        // translator task. EventReceiver yields api::Event values; we map
        // each into AdapterEvent and forward.
        let me = Arc::downgrade(&adapter);
        let translator_spawned = retained_tasks.spawn(async move {
            loop {
                tokio::select! {
                    changed = translator_cancel_rx.changed() => {
                        if changed.is_err() || *translator_cancel_rx.borrow() {
                            break;
                        }
                    }
                    event = events.next() => {
                        let Some(event) = event else {
                            break;
                        };
                        let Some(adapter) = me.upgrade() else {
                            break;
                        };
                        adapter.translate_api_event(event).await;
                    }
                }
            }
            debug!("SipAdapter event translator stream ended");
        });
        debug_assert!(translator_spawned);
        let reaper_spawned = adapter
            .retained_tasks
            .spawn(Self::run_pending_context_reaper(
                Arc::downgrade(&adapter.inbound_contexts),
                context_reaper_cancel_rx,
                PENDING_SIP_INBOUND_CONTEXT_REAPER_INTERVAL,
            ));
        debug_assert!(reaper_spawned);

        Ok(adapter)
    }

    /// Build a coordinator from `Config` and install an explicit inbound
    /// context policy.
    pub async fn from_config_with_inbound_context_policy(
        config: ApiConfig,
        policy: SipInboundContextPolicy,
    ) -> crate::errors::Result<Arc<Self>> {
        let coordinator = UnifiedCoordinator::new(config).await?;
        Self::new_with_inbound_context_policy(coordinator, policy).await
    }

    /// Convenience: build a coordinator from `Config` and wrap it.
    pub async fn from_config(config: ApiConfig) -> crate::errors::Result<Arc<Self>> {
        let coordinator = UnifiedCoordinator::new(config).await?;
        Self::new(coordinator).await
    }

    /// Build a coordinator whose SIP listener enforces the supplied policy,
    /// then wrap it as a core adapter.
    pub async fn from_config_with_listener_auth(
        config: ApiConfig,
        policy: crate::auth::SipListenerAuthPolicy,
    ) -> crate::errors::Result<Arc<Self>> {
        let coordinator = UnifiedCoordinator::new_with_listener_auth(config, policy).await?;
        Self::new(coordinator).await
    }

    /// Borrow the underlying coordinator (for code that needs both surfaces
    /// during the carve transition — e.g. server::*  helpers).
    pub fn coordinator(&self) -> &Arc<UnifiedCoordinator> {
        &self.coordinator
    }

    /// Whether this adapter has crossed its one-way admission/drain boundary.
    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }

    /// Number of adapter-owned supervisors that have not yet terminated.
    /// This is intended for shutdown diagnostics and leak assertions.
    pub fn retained_task_count(&self) -> usize {
        self.retained_tasks.count()
    }

    fn prior_drain_failure(&self) -> Option<RvoipError> {
        self.drain_failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
            .map(RvoipError::Adapter)
    }

    fn fail_drain(&self, message: impl Into<String>) -> RvoipError {
        let mut failure = self
            .drain_failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let message = message.into();
        let retained = failure.get_or_insert(message);
        RvoipError::Adapter(retained.clone())
    }

    /// Stop admission, compensate every live SIP route, and wait until every
    /// adapter-owned task and media driver has terminated.
    ///
    /// Outbound routes use their retained wire phase: prepared routes remain
    /// zero-wire, while possibly-sent routes receive CANCEL/BYE cleanup.
    /// Inbound routes are rejected before answer and hung up after answer.
    /// This operation is idempotent but intentionally one-way.
    pub async fn drain(&self) -> CoreResult<()> {
        let adapter = self.self_weak.upgrade().ok_or_else(|| {
            RvoipError::Adapter("SIP adapter drain supervisor is unavailable".to_string())
        })?;
        let panic_owner = Arc::clone(&adapter);
        let driver = tokio::spawn(async move {
            match std::panic::AssertUnwindSafe(adapter.drain_inner())
                .catch_unwind()
                .await
            {
                Ok(result) => result,
                Err(_) => Err(panic_owner.fail_drain("SIP adapter drain driver panicked")),
            }
        });
        match driver.await {
            Ok(result) => result,
            Err(error) => Err(self.fail_drain(format!(
                "SIP adapter drain driver stopped unexpectedly: {error}"
            ))),
        }
    }

    /// Cancellation-safe destructive drain driver. The public waiter owns
    /// only a Tokio join handle; dropping that waiter detaches this future and
    /// leaves the adapter-owned `Arc` alive until cleanup converges.
    async fn drain_inner(self: Arc<Self>) -> CoreResult<()> {
        let _drain = self.drain_gate.lock().await;
        if self.drained.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Some(failure) = self.prior_drain_failure() {
            return Err(failure);
        }

        let (outbound, inbound) = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.draining.store(true, Ordering::Release);

            let outbound = self
                .outbound_routes
                .iter()
                .map(|entry| Arc::clone(entry.value()))
                .collect::<Vec<_>>();
            let inbound_connection_ids = self
                .by_connection
                .iter()
                .filter(|entry| !self.outbound_routes.contains_key(entry.key()))
                .map(|entry| entry.key().clone())
                .collect::<Vec<_>>();
            let inbound_epochs = inbound_connection_ids
                .iter()
                .filter_map(|connection_id| self.route_epoch_for_connection_locked(connection_id))
                .collect::<Vec<_>>();
            let mut inbound = Vec::with_capacity(inbound_epochs.len());
            for epoch in inbound_epochs {
                let stream = self
                    .remove_epoch_locked(&epoch)
                    .and_then(|removed| removed.stream);
                inbound.push((epoch, stream));
            }
            (outbound, inbound)
        };

        self.translator_cancel.send_replace(true);
        if self.observer_registered.swap(false, Ordering::AcqRel) {
            self.coordinator
                .remove_inbound_invite_observer(self.inbound_invite_observer_id);
        }

        let outbound_results = outbound.clone();
        for route in outbound {
            route.complete_activation_failure(SipActivationFailure::RouteEnded);
            let terminal = AdapterEvent::Ended {
                connection_id: route.connection_id.clone(),
                reason: EndReason::BridgeTorn,
            };
            begin_outbound_cleanup(
                self.self_weak.clone(),
                Arc::clone(&self.coordinator),
                route,
                self.lifecycle.clone(),
                self.out_tx.clone(),
                Some(terminal),
                "adapter-drain-outbound",
            );
        }

        let mut spawn_failed = false;
        let mut inbound_results = Vec::with_capacity(inbound.len());
        let fast_auto_accept = self.coordinator.fast_auto_accept_incoming_calls();
        for (epoch, stream) in inbound {
            if let Some(stream) = stream.as_ref() {
                stream.request_close();
            }
            let coordinator = Arc::clone(&self.coordinator);
            let lifecycle = self.lifecycle.clone();
            let events = self.out_tx.clone();
            let cleanup_result = Arc::new(AtomicBool::new(false));
            inbound_results.push(Arc::clone(&cleanup_result));
            if !self.retained_tasks.spawn(run_inbound_drain(
                coordinator,
                epoch,
                stream,
                fast_auto_accept,
                lifecycle,
                events,
                cleanup_result,
            )) {
                spawn_failed = true;
            }
        }

        for stream in self.streams_cache.iter() {
            stream.value().request_close();
        }
        self.retained_tasks.close();
        if tokio::time::timeout(SIP_ADAPTER_DRAIN_TIMEOUT, self.retained_tasks.wait_idle())
            .await
            .is_err()
        {
            return Err(self.fail_drain(format!(
                "SIP adapter drain timed out with {} retained tasks",
                self.retained_tasks.count()
            )));
        }
        if spawn_failed {
            return Err(
                self.fail_drain("SIP adapter drain could not retain an inbound cleanup task")
            );
        }
        if self.retained_tasks.panicked() {
            return Err(self.fail_drain("SIP adapter retained task panicked during drain"));
        }
        #[cfg(test)]
        let forced_compensation_failure = self
            .force_drain_compensation_failure
            .load(Ordering::Acquire);
        #[cfg(not(test))]
        let forced_compensation_failure = false;
        if forced_compensation_failure
            || outbound_results
                .iter()
                .any(|route| route.cleanup_result() != Some(true))
            || inbound_results
                .iter()
                .any(|result| !result.load(Ordering::Acquire))
        {
            return Err(self
                .fail_drain("SIP adapter drain could not complete network or media compensation"));
        }

        let registry_empty = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.by_connection.is_empty()
                && self.by_session.is_empty()
                && self.transfer_attempts.is_empty()
                && self.outbound_routes.is_empty()
                && self.streams_cache.is_empty()
                && self.authenticated_inbound_sessions.is_empty()
        };
        if !registry_empty {
            return Err(
                self.fail_drain("SIP adapter drain completed with live lifecycle registry entries")
            );
        }
        self.inbound_contexts.clear();
        self.drained.store(true, Ordering::Release);
        Ok(())
    }

    /// Drain the adapter and then stop the underlying SIP coordinator.
    pub async fn shutdown(&self) -> CoreResult<()> {
        self.drain().await?;
        self.coordinator
            .shutdown_gracefully(Some(SIP_ADAPTER_DRAIN_TIMEOUT))
            .await
            .map_err(|error| RvoipError::Adapter(format!("SIP shutdown failed: {error}")))
    }

    fn take_atomic_events(&self) -> CoreResult<mpsc::Receiver<OrchestratorAdapterEvent>> {
        self.out_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .ok_or(RvoipError::InvalidState(
                "SipAdapter event stream already consumed",
            ))
    }

    /// Subscribe directly while retaining the historical authenticated
    /// inbound sequence (`InboundConnection`, then `PrincipalAuthenticated`).
    pub fn try_subscribe_events(&self) -> CoreResult<mpsc::Receiver<AdapterEvent>> {
        self.take_atomic_events()
            .map(|events| legacy_normalized_event_receiver(events, 512))
    }

    /// Opt in to the raw one-item authenticated inbound stream used by the
    /// Orchestrator.
    pub fn try_subscribe_atomic_events(
        &self,
    ) -> CoreResult<mpsc::Receiver<OrchestratorAdapterEvent>> {
        self.take_atomic_events()
    }

    fn route_epoch_for_session_locked(&self, session_id: &SessionId) -> Option<SipRouteEpoch> {
        let binding = self.by_session.get(session_id)?;
        let reverse = self.by_connection.get(&binding.connection_id)?;
        if reverse.session_id != *session_id || !reverse.owner.equivalent(&binding.owner) {
            return None;
        }
        Some(SipRouteEpoch {
            session_id: session_id.clone(),
            connection_id: binding.connection_id.clone(),
            owner: binding.owner.epoch_owner()?,
        })
    }

    fn route_epoch_for_connection_locked(
        &self,
        connection_id: &ConnectionId,
    ) -> Option<SipRouteEpoch> {
        let binding = self.by_connection.get(connection_id)?;
        let reverse = self.by_session.get(&binding.session_id)?;
        if reverse.connection_id != *connection_id || !reverse.owner.equivalent(&binding.owner) {
            return None;
        }
        Some(SipRouteEpoch {
            session_id: binding.session_id.clone(),
            connection_id: connection_id.clone(),
            owner: binding.owner.epoch_owner()?,
        })
    }

    fn epoch_is_current_locked(&self, epoch: &SipRouteEpoch) -> bool {
        self.route_epoch_for_connection_locked(&epoch.connection_id)
            .is_some_and(|current| current == *epoch)
    }

    fn current_session_handle(&self, session_id: &SessionId) -> Option<SessionRegistryHandle> {
        self.coordinator
            .helpers
            .state_machine
            .store
            .lifecycle_handle(session_id)
    }

    fn promote_prepared_epoch_locked(
        &self,
        epoch: SipRouteEpoch,
        handle: SessionRegistryHandle,
    ) -> Option<SipRouteEpoch> {
        if handle.session_id() != &epoch.session_id {
            return None;
        }
        match epoch.owner {
            SipRouteEpochOwner::Admitted(existing) => {
                (existing == handle).then_some(SipRouteEpoch {
                    session_id: epoch.session_id,
                    connection_id: epoch.connection_id,
                    owner: SipRouteEpochOwner::Admitted(existing),
                })
            }
            SipRouteEpochOwner::Prepared(route) => {
                if !route.attach_lifecycle_handle(handle.clone()) {
                    return None;
                }
                let owner = SipRouteBindingOwner::Admitted(handle.clone());
                self.by_session.insert(
                    epoch.session_id.clone(),
                    SipSessionBinding {
                        connection_id: epoch.connection_id.clone(),
                        owner: owner.clone(),
                    },
                );
                self.by_connection.insert(
                    epoch.connection_id.clone(),
                    SipConnectionBinding {
                        session_id: epoch.session_id.clone(),
                        owner,
                    },
                );
                Some(SipRouteEpoch {
                    session_id: epoch.session_id,
                    connection_id: epoch.connection_id,
                    owner: SipRouteEpochOwner::Admitted(handle),
                })
            }
        }
    }

    /// Lookup-only route resolution for every event after initial inbound
    /// admission. A prepared outbound route may be promoted once the shared
    /// session authority has admitted it, but a raw late event never creates a
    /// new mapping.
    fn existing_mapped_epoch(&self, session_id: &SessionId) -> Option<SipRouteEpoch> {
        let current_handle = self.current_session_handle(session_id);
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let epoch = self.route_epoch_for_session_locked(session_id)?;
        match (&epoch.owner, current_handle) {
            (SipRouteEpochOwner::Prepared(_), Some(handle)) => {
                self.promote_prepared_epoch_locked(epoch, handle)
            }
            (SipRouteEpochOwner::Prepared(_), None) => Some(epoch),
            (SipRouteEpochOwner::Admitted(existing), Some(handle)) if existing == &handle => {
                Some(epoch)
            }
            (SipRouteEpochOwner::Admitted(_), _) => None,
        }
    }

    /// Resolve a route for an already-published terminal API event.
    ///
    /// Terminal publication deliberately precedes coordinator/session release,
    /// but the adapter's event translator may observe that publication only
    /// after the exact session handle has retired. In that window the adapter
    /// mapping is the last retained exact owner and must still be removable.
    /// A different current handle for the same raw `SessionId` remains a hard
    /// fence so a delayed terminal event cannot retire a reused generation.
    fn existing_mapped_terminal_epoch(&self, session_id: &SessionId) -> Option<SipRouteEpoch> {
        let current_handle = self.current_session_handle(session_id);
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let epoch = self.route_epoch_for_session_locked(session_id)?;
        match (&epoch.owner, current_handle) {
            (SipRouteEpochOwner::Prepared(_), Some(handle)) => {
                self.promote_prepared_epoch_locked(epoch, handle)
            }
            (SipRouteEpochOwner::Prepared(_), None) => Some(epoch),
            (SipRouteEpochOwner::Admitted(existing), Some(handle)) if existing == &handle => {
                Some(epoch)
            }
            (SipRouteEpochOwner::Admitted(_), Some(_)) => None,
            (SipRouteEpochOwner::Admitted(_), None) => Some(epoch),
        }
    }

    /// Admit the first inbound adapter route only when the coordinator already
    /// exposes the exact shared session handle.
    fn ensure_mapped_epoch(&self, session_id: SessionId) -> Option<SipRouteEpoch> {
        let handle = self.current_session_handle(&session_id)?;
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(epoch) = self.route_epoch_for_session_locked(&session_id) {
            return self.promote_prepared_epoch_locked(epoch, handle);
        }
        if self.draining.load(Ordering::Acquire)
            || self.by_session.len() >= self.active_connection_budget.load(Ordering::Acquire)
        {
            return None;
        }
        let connection_id = ConnectionId::new();
        if self.transfer_attempts.contains_key(&connection_id) {
            return None;
        }
        let owner = SipRouteBindingOwner::Admitted(handle.clone());
        self.by_session.insert(
            session_id.clone(),
            SipSessionBinding {
                connection_id: connection_id.clone(),
                owner: owner.clone(),
            },
        );
        self.by_connection.insert(
            connection_id.clone(),
            SipConnectionBinding {
                session_id: session_id.clone(),
                owner,
            },
        );
        Some(SipRouteEpoch {
            session_id,
            connection_id,
            owner: SipRouteEpochOwner::Admitted(handle),
        })
    }

    #[cfg(test)]
    fn ensure_mapped(&self, session_id: SessionId) -> Option<ConnectionId> {
        self.ensure_mapped_epoch(session_id)
            .map(|epoch| epoch.connection_id)
    }

    fn reserve_outbound_route(&self, route: Arc<SipOutboundRoute>) -> CoreResult<()> {
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.draining.load(Ordering::Acquire)
            || self.by_session.contains_key(&route.session_id)
            || self.by_connection.contains_key(&route.connection_id)
            || self.transfer_attempts.contains_key(&route.connection_id)
            || self.outbound_routes.contains_key(&route.connection_id)
            || self.streams_cache.contains_key(&route.connection_id)
            || self.by_session.len() >= self.active_connection_budget.load(Ordering::Acquire)
        {
            return Err(RvoipError::AdmissionRejected(
                "SIP outbound lifecycle reservation was unavailable",
            ));
        }
        let owner = SipRouteBindingOwner::Prepared(Arc::downgrade(&route));
        self.by_session.insert(
            route.session_id.clone(),
            SipSessionBinding {
                connection_id: route.connection_id.clone(),
                owner: owner.clone(),
            },
        );
        self.by_connection.insert(
            route.connection_id.clone(),
            SipConnectionBinding {
                session_id: route.session_id.clone(),
                owner,
            },
        );
        self.streams_cache
            .insert(route.connection_id.clone(), Arc::clone(&route.stream));
        self.outbound_routes
            .insert(route.connection_id.clone(), route);
        Ok(())
    }

    fn adapter_event_connection_id(event: &AdapterEvent) -> Option<&ConnectionId> {
        match event {
            AdapterEvent::InboundConnection { connection } => Some(&connection.id),
            AdapterEvent::Connected { connection_id }
            | AdapterEvent::Progress { connection_id, .. }
            | AdapterEvent::Authenticated { connection_id, .. }
            | AdapterEvent::PrincipalAuthenticated { connection_id, .. }
            | AdapterEvent::Ended { connection_id, .. }
            | AdapterEvent::Failed { connection_id, .. }
            | AdapterEvent::Dtmf { connection_id, .. }
            | AdapterEvent::Quality { connection_id, .. }
            | AdapterEvent::Message { connection_id, .. }
            | AdapterEvent::DataMessage { connection_id, .. }
            | AdapterEvent::TransferStatus { connection_id, .. }
            | AdapterEvent::StepUpResponse { connection_id, .. } => Some(connection_id),
            _ => None,
        }
    }

    /// Configure the maximum number of active SIP mappings. The second value
    /// remains source-compatible but retirement capacity is owned by the
    /// shared `SessionLeaseAuthority` and is intentionally ignored here.
    pub fn configure_lifecycle_limits(
        &self,
        active_connections: usize,
        _retired_sessions: usize,
    ) -> CoreResult<()> {
        if active_connections == 0 {
            return Err(RvoipError::InvalidState(
                "SIP active-connection capacity must be non-zero",
            ));
        }
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.by_session.is_empty() {
            return Err(RvoipError::InvalidState(
                "SIP lifecycle limits cannot change after route admission",
            ));
        }
        self.active_connection_budget
            .store(active_connections, Ordering::Release);
        Ok(())
    }

    async fn run_pending_context_reaper(
        contexts: Weak<SipInboundContextStore>,
        mut cancel: watch::Receiver<bool>,
        interval: Duration,
    ) {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tick.tick().await;
        loop {
            tokio::select! {
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        break;
                    }
                }
                _ = tick.tick() => {
                    let Some(contexts) = contexts.upgrade() else {
                        break;
                    };
                    let removed = contexts.purge_expired();
                    if removed > 0 {
                        debug!(removed, "SipAdapter purged expired inbound contexts");
                    }
                }
            }
        }
    }

    /// Remove exactly one authority-qualified route while holding
    /// `mapping_lock`.
    ///
    /// The paired maps, stream cache, outbound route, authentication marker,
    /// change as one registry transaction. A stale supervisor can therefore
    /// never retire a newer route that reused an identifier.
    fn remove_epoch_locked(&self, epoch: &SipRouteEpoch) -> Option<SipRemovedEpoch> {
        if !self.epoch_is_current_locked(epoch) {
            return None;
        }
        let owns_transfer_history = self
            .transfer_attempts
            .get(&epoch.connection_id)
            .is_some_and(|state| state.epoch == *epoch);
        if owns_transfer_history {
            self.transfer_attempts.remove(&epoch.connection_id);
        }
        self.by_connection.remove(&epoch.connection_id);
        self.by_session.remove(&epoch.session_id);
        let owns_outbound_route = self
            .outbound_routes
            .get(&epoch.connection_id)
            .is_some_and(|route| epoch.matches_route(route.value()));
        if owns_outbound_route {
            self.outbound_routes.remove(&epoch.connection_id);
        }
        self.authenticated_inbound_sessions
            .remove(&epoch.session_id);
        self.inbound_contexts
            .forget(&epoch.session_id, &epoch.connection_id);
        let stream = self
            .streams_cache
            .remove(&epoch.connection_id)
            .map(|(_, stream)| stream);
        Some(SipRemovedEpoch { stream })
    }

    fn close_stream_retained(&self, stream: Arc<crate::media_stream::SipMediaStream>) {
        stream.request_close();
        let spawned = self.retained_tasks.spawn(async move {
            let _ = tokio::time::timeout(
                SIP_RETAINED_TASK_TIMEOUT,
                (stream as Arc<dyn MediaStream>).close(),
            )
            .await;
        });
        if !spawned {
            debug!("SipAdapter stream close requested after retained-task admission closed");
        }
    }

    fn forget_epoch(&self, epoch: &SipRouteEpoch) -> bool {
        let removed = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.remove_epoch_locked(epoch)
        };
        let Some(removed) = removed else {
            return false;
        };
        if let Some(stream) = removed.stream {
            self.close_stream_retained(stream);
        }
        true
    }

    fn retire_outbound_route(&self, route: &Arc<SipOutboundRoute>) {
        let owner = match route.lifecycle_handle() {
            Some(handle) => SipRouteEpochOwner::Admitted(handle),
            None => SipRouteEpochOwner::Prepared(Arc::clone(route)),
        };
        let epoch = SipRouteEpoch {
            session_id: route.session_id.clone(),
            connection_id: route.connection_id.clone(),
            owner,
        };
        let stream = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let exact_route = self
                .outbound_routes
                .get(&route.connection_id)
                .is_some_and(|entry| Arc::ptr_eq(entry.value(), route));
            if !exact_route {
                return;
            }
            self.remove_epoch_locked(&epoch)
                .and_then(|removed| removed.stream)
        };
        if let Some(stream) = stream {
            stream.request_close();
        }
    }

    async fn terminate_failed_inbound(
        &self,
        session_id: &SessionId,
        epoch: Option<&SipRouteEpoch>,
        status: u16,
        reason: &'static str,
    ) {
        let state = self
            .coordinator
            .session_state(session_id)
            .await
            .ok()
            .map(|session| session.call_state);
        let action =
            failed_inbound_termination(state, self.coordinator.fast_auto_accept_incoming_calls());

        // Local visibility and admission state must converge even when the
        // network transaction or dialog teardown fails. Removing this first
        // also makes queued non-terminal adapter events fail the live-route
        // check instead of resurrecting the rejected connection.
        if let Some(epoch) = epoch {
            self.forget_epoch(epoch);
        } else {
            // No exact adapter route was admitted. It is safe to discard only
            // the unpublished observation; a raw SessionId must never remove
            // a route that may already belong to another authority handle.
            self.inbound_contexts.forget_pending(session_id);
        }

        let force_cleanup = match action {
            FailedInboundTermination::Reject => {
                match self
                    .coordinator
                    .reject(session_id)
                    .with_status(status)
                    .with_reason(reason)
                    .send()
                    .await
                {
                    Ok(()) => false,
                    Err(reject_error) => {
                        warn!(
                            %reject_error,
                            "SipAdapter failed to reject unpublished inbound route; trying hangup"
                        );
                        match self.coordinator.hangup(session_id).await {
                            Ok(()) => false,
                            Err(hangup_error) => {
                                warn!(
                                    %hangup_error,
                                    "SipAdapter fallback hangup failed for unpublished inbound route"
                                );
                                true
                            }
                        }
                    }
                }
            }
            FailedInboundTermination::Hangup => {
                if let Err(error) = self.coordinator.hangup(session_id).await {
                    warn!(%error, "SipAdapter failed to hang up unpublished inbound route");
                    true
                } else {
                    false
                }
            }
            FailedInboundTermination::CleanupOnly => true,
        };

        if force_cleanup {
            if let Err(error) = self
                .coordinator
                .finalize_local_bye(session_id, reason)
                .await
            {
                warn!(%error, "SipAdapter forced inbound cleanup did not publish terminal state");
            }
        }
    }

    fn lookup_session(&self, conn: &ConnectionId) -> CoreResult<SessionId> {
        self.by_connection
            .get(conn)
            .map(|e| e.session_id.clone())
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))
    }

    fn lookup_session_for_non_terminal_control(
        &self,
        conn: &ConnectionId,
    ) -> CoreResult<SessionId> {
        if let Some(route) = self
            .outbound_routes
            .get(conn)
            .map(|entry| Arc::clone(entry.value()))
        {
            route.session_for_non_terminal_control()
        } else {
            self.lookup_session(conn)
        }
    }

    /// Atomically reserve the only transfer attempt permitted for this exact
    /// live route. The history entry is intentionally not reusable after a
    /// terminal status: raw REFER events lack transaction correlation, so a
    /// second attempt on the same route could consume a delayed first result.
    fn reserve_transfer_attempt(
        &self,
        conn: &ConnectionId,
        attempt_id: Option<TransferAttemptId>,
    ) -> CoreResult<SipRouteEpoch> {
        let session_id = self.lookup_session_for_non_terminal_control(conn)?;
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let epoch = self
            .route_epoch_for_connection_locked(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        if epoch.session_id != session_id {
            return Err(RvoipError::InvalidState(
                "SIP transfer route changed during reservation",
            ));
        }
        if self.transfer_attempts.contains_key(conn) {
            return Err(RvoipError::InvalidState(
                "SIP permits only one transfer attempt per live route",
            ));
        }
        self.transfer_attempts.insert(
            conn.clone(),
            SipTransferAttemptState {
                epoch: epoch.clone(),
                attempt_id,
                active: true,
            },
        );
        Ok(epoch)
    }

    /// Resolve a raw REFER status to the route's active attempt. The outer
    /// `Option` distinguishes a valid legacy attempt (`Some(None)`) from an
    /// uncorrelated or duplicate raw event (`None`). Final status retains only
    /// the route-lifetime used marker until exact epoch cleanup.
    fn transfer_attempt_for_status(
        &self,
        epoch: &SipRouteEpoch,
        terminal: bool,
    ) -> Option<Option<TransferAttemptId>> {
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.epoch_is_current_locked(epoch) {
            return None;
        }
        let mut state = self.transfer_attempts.get_mut(&epoch.connection_id)?;
        if state.epoch != *epoch || !state.active {
            return None;
        }
        let attempt_id = state.attempt_id.clone();
        if terminal {
            state.active = false;
            state.attempt_id = None;
        }
        Some(attempt_id)
    }

    async fn submit_transfer(
        &self,
        conn: ConnectionId,
        attempt_id: Option<TransferAttemptId>,
        target: TransferTarget,
    ) -> CoreResult<()> {
        let refer_to = match target {
            TransferTarget::Uri(uri) => uri,
            TransferTarget::Connection(_) | TransferTarget::Session(_) => {
                return Err(RvoipError::NotImplemented(
                    "unsupported beta feature: attended transfer by Connection/Session target is post-beta for SipAdapter",
                ));
            }
        };
        let epoch = self.reserve_transfer_attempt(&conn, attempt_id)?;
        let result = self
            .coordinator
            .refer(&epoch.session_id, refer_to)
            .send()
            .await
            .map_err(Self::map_session_err);
        if result.is_err() {
            // A submission error is terminal for active correlation but still
            // consumes this route's sole attempt. The lower API does not
            // classify pre-wire versus ambiguous post-wire failure.
            let _ = self.transfer_attempt_for_status(&epoch, true);
        }
        result
    }

    /// Capture one exact live route for a non-terminal operation.
    ///
    /// The returned epoch retains the exact shared lifecycle handle (or the
    /// retained outbound preparation object before admission), preventing a
    /// later raw-`SessionId` lookup from crossing an identifier reuse.
    fn data_message_route_epoch(&self, conn: &ConnectionId) -> CoreResult<SipRouteEpoch> {
        if let Some(route) = self
            .outbound_routes
            .get(conn)
            .map(|entry| Arc::clone(entry.value()))
        {
            route.session_for_non_terminal_control()?;
        }
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.route_epoch_for_connection_locked(conn)
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))
    }

    /// Capture the exact lower dialog resource and prove that the route did
    /// not change on either side of that synchronous capture.
    fn data_message_dialog_for_epoch(
        &self,
        epoch: &SipRouteEpoch,
    ) -> CoreResult<rvoip_sip_dialog::DialogId> {
        let handle = epoch.admitted_handle().ok_or(RvoipError::InvalidState(
            "SIP data message route has not completed lifecycle admission",
        ))?;
        {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !self.epoch_is_current_locked(epoch) {
                return Err(RvoipError::InvalidState(
                    "SIP data message route is no longer current",
                ));
            }
        }
        let dialog_id = self
            .coordinator
            .session_registry
            .get_dialog_exact(handle.key(), handle.slot_revision())
            .map(Into::into)
            .ok_or(RvoipError::InvalidState(
                "SIP data message requires an exact established dialog",
            ))?;
        {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !self.epoch_is_current_locked(epoch) {
                return Err(RvoipError::InvalidState(
                    "SIP data message route changed during dialog capture",
                ));
            }
        }
        Ok(dialog_id)
    }

    fn publish_sip_data_message_for_epoch(
        &self,
        epoch: &SipRouteEpoch,
        request: &rvoip_sip_core::Request,
    ) -> std::result::Result<bool, crate::sip_data_message::SipDataMessageError> {
        let message = crate::sip_data_message::from_sip_request(request)?;
        Ok(self.try_send_for_epoch(
            epoch,
            AdapterEvent::DataMessage {
                connection_id: epoch.connection_id.clone(),
                message,
            },
        ))
    }

    async fn build_connection(&self, conn_id: ConnectionId, direction: Direction) -> Connection {
        // Eagerly allocate (and cache) one dormant SipMediaStream so consumers
        // can read `connection.streams` synchronously off the
        // `Event::ConnectionInbound` event — QUIC/WT parity, gap plan §2.2.
        // Inbound binding runs independently so a signaling-only coordinator
        // or a media session that becomes ready later cannot block the
        // authenticated connection event. Outbound streams remain dormant
        // until `activate_outbound` runs after core's durable binding.
        let streams = self
            .get_or_insert_dormant_stream(&conn_id, direction)
            .map(|stream| {
                if direction == Direction::Inbound {
                    self.start_stream_bind(conn_id.clone(), Arc::clone(&stream));
                }
                vec![MediaStreamHandle::new(stream as Arc<dyn MediaStream>)]
            })
            .unwrap_or_default();
        Connection {
            id: conn_id,
            session_id: CoreSessionId::new(),
            participant_id: ParticipantId::new(),
            transport: Transport::Sip,
            direction,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor::default(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams,
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        }
    }

    /// Return the one stable dormant stream for a mapped connection.
    ///
    /// The mapping and cache check/insert share one lock with retirement, so a
    /// concurrent forget cannot leave an orphan stream behind. No coordinator
    /// or network operation occurs here.
    fn get_or_insert_dormant_stream(
        &self,
        conn: &ConnectionId,
        direction: Direction,
    ) -> Option<Arc<crate::media_stream::SipMediaStream>> {
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.by_connection.get(conn)?;
        if let Some(stream) = self.streams_cache.get(conn) {
            return (stream.direction() == direction).then(|| Arc::clone(stream.value()));
        }
        let stream = crate::media_stream::SipMediaStream::dormant_deferred(direction);
        self.streams_cache.insert(conn.clone(), Arc::clone(&stream));
        Some(stream)
    }

    fn start_stream_bind(
        &self,
        conn: ConnectionId,
        stream: Arc<crate::media_stream::SipMediaStream>,
    ) {
        let epoch = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.route_epoch_for_connection_locked(&conn)
        };
        let Some(epoch) = epoch else { return };
        let session_id = epoch.session_id.clone();
        let coordinator = Arc::clone(&self.coordinator);
        let weak_adapter = self.self_weak.clone();
        let spawned = self.retained_tasks.spawn(async move {
            let binding = stream
                .bind(Arc::clone(&coordinator), session_id.clone())
                .await;
            let mut lifecycle = stream.subscribe_lifecycle();
            let failed = if binding.is_err() {
                matches!(
                    *lifecycle.borrow_and_update(),
                    crate::media_stream::SipMediaLifecycle::Failed
                )
            } else {
                loop {
                    match *lifecycle.borrow_and_update() {
                        crate::media_stream::SipMediaLifecycle::Failed => break true,
                        crate::media_stream::SipMediaLifecycle::Closing
                        | crate::media_stream::SipMediaLifecycle::Closed => break false,
                        crate::media_stream::SipMediaLifecycle::Dormant
                        | crate::media_stream::SipMediaLifecycle::Binding
                        | crate::media_stream::SipMediaLifecycle::Bound => {}
                    }
                    if lifecycle.changed().await.is_err() {
                        break false;
                    }
                }
            };
            if failed {
                if let Some(adapter) = weak_adapter.upgrade() {
                    adapter.terminate_failed_media(&epoch, coordinator).await;
                }
            }
        });
        if !spawned {
            debug!("SipAdapter skipped media bind after retained-task admission closed");
        }
    }

    async fn terminate_failed_media(
        &self,
        epoch: &SipRouteEpoch,
        coordinator: Arc<UnifiedCoordinator>,
    ) {
        let still_exact = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.epoch_is_current_locked(epoch)
                && !self.outbound_routes.contains_key(&epoch.connection_id)
        };
        if !still_exact {
            return;
        }
        if !self.forget_epoch(epoch) {
            return;
        }
        let hangup = tokio::time::timeout(
            SIP_RETAINED_TASK_TIMEOUT,
            coordinator.hangup(&epoch.session_id),
        )
        .await;
        if !matches!(hangup, Ok(Ok(()))) {
            let _ = tokio::time::timeout(
                SIP_RETAINED_TASK_TIMEOUT,
                coordinator.finalize_local_bye(&epoch.session_id, "SIP media driver failed"),
            )
            .await;
        }
        self.deliver_terminal_event(
            AdapterEvent::Failed {
                connection_id: epoch.connection_id.clone(),
                detail: "SIP media driver failed".to_string(),
            },
            "media-failed",
        )
        .await;
    }

    async fn translate_api_event(&self, event: ApiEvent) {
        match event {
            ApiEvent::IncomingCall { call_id, .. } => {
                let Some(epoch) = self.ensure_mapped_epoch(call_id.clone()) else {
                    warn!("SipAdapter rejected inbound route after lifecycle retirement/capacity");
                    self.terminate_failed_inbound(
                        &call_id,
                        None,
                        503,
                        "Connection Capacity Exhausted",
                    )
                    .await;
                    return;
                };
                let conn_id = epoch.connection_id.clone();
                let principal = match self.inbound_contexts.bind(&call_id, &conn_id) {
                    SipInboundBinding::Observed(principal) => principal,
                    SipInboundBinding::Rejected(error) => {
                        warn!(
                            ?error,
                            "SipAdapter rejected invalid authenticated inbound route"
                        );
                        self.terminate_failed_inbound(
                            &call_id,
                            Some(&epoch),
                            403,
                            "Authenticated Signaling Context Rejected",
                        )
                        .await;
                        return;
                    }
                    SipInboundBinding::Missing => {
                        // Every new inbound call is synchronously observed
                        // before its public event is emitted. A missing entry
                        // therefore means admission overflow, expiry, or a
                        // startup race; fail closed instead of publishing a
                        // route without its authentication state.
                        warn!("SipAdapter rejected inbound route without its observation");
                        self.terminate_failed_inbound(
                            &call_id,
                            Some(&epoch),
                            503,
                            "Signaling Context Unavailable",
                        )
                        .await;
                        return;
                    }
                };
                let connection = self
                    .build_connection(conn_id.clone(), Direction::Inbound)
                    .await;
                if let Some(principal) = principal.as_ref() {
                    if let Err(error) = validate_sip_principal_at(principal, Utc::now()) {
                        warn!(
                            ?error,
                            "SipAdapter principal expired or became invalid before publication"
                        );
                        self.terminate_failed_inbound(
                            &call_id,
                            Some(&epoch),
                            403,
                            "Authenticated Principal No Longer Active",
                        )
                        .await;
                        return;
                    }
                }
                let adapter_event = if let Some(principal) = principal {
                    self.authenticated_inbound_sessions
                        .insert(call_id.clone(), ());
                    OrchestratorAdapterEvent::AuthenticatedInboundConnection {
                        participant_id: principal.subject.clone(),
                        connection,
                        principal,
                    }
                } else {
                    OrchestratorAdapterEvent::Public(AdapterEvent::InboundConnection { connection })
                };
                match self.try_publish_inbound_for_epoch(&epoch, adapter_event) {
                    SipInboundPublication::Published => {}
                    SipInboundPublication::Backpressured => {
                        // Keep the consumed tombstone until terminal cleanup so a
                        // replayed IncomingCall cannot recreate the secret.
                        self.inbound_contexts.discard(&conn_id);
                        self.terminate_failed_inbound(
                            &call_id,
                            Some(&epoch),
                            503,
                            "Signaling Event Backpressure",
                        )
                        .await;
                    }
                    SipInboundPublication::Retired => {
                        // Drain or a concurrent terminal already owns cleanup.
                        self.inbound_contexts.discard(&conn_id);
                    }
                }
            }
            ApiEvent::IncomingCallAuthenticated { call_id, principal } => {
                if self.authenticated_inbound_sessions.contains_key(&call_id) {
                    return;
                }
                // The coordinator normally publishes IncomingCall first, but
                // its bounded broadcast bridge may report a later auth event
                // first after lag recovery. The synchronous observation is
                // already present, so defer to the atomic inbound event rather
                // than exposing a principal-only route.
                if self.inbound_contexts.has_pending(&call_id) {
                    return;
                }
                // Authentication is published only as part of the atomic
                // inbound handoff. A principal-only event here could create a
                // partially authenticated route after admission overflow,
                // expiry, or an undeliverable inbound queue.
                warn!(
                    subject_present = !principal.subject.is_empty(),
                    subject_bytes = principal.subject.len(),
                    "SipAdapter suppressed unmatched inbound authentication event"
                );
            }
            ApiEvent::CallAnswered { call_id, .. } => {
                let Some(epoch) = self.existing_mapped_epoch(&call_id) else {
                    return;
                };
                self.try_send_for_epoch(
                    &epoch,
                    AdapterEvent::Connected {
                        connection_id: epoch.connection_id.clone(),
                    },
                );
            }
            ApiEvent::CallProgress {
                call_id,
                status_code,
                reason,
                sdp,
            } => {
                let Some(epoch) = self.existing_mapped_epoch(&call_id) else {
                    return;
                };
                self.try_send_for_epoch(
                    &epoch,
                    AdapterEvent::Progress {
                        connection_id: epoch.connection_id.clone(),
                        status_code,
                        reason,
                        early_media: status_code == 183 && sdp.is_some(),
                    },
                );
            }
            ApiEvent::CallEnded { call_id, reason } => {
                let Some(epoch) = self.existing_mapped_terminal_epoch(&call_id) else {
                    return;
                };
                self.deliver_terminal_for_epoch(
                    &epoch,
                    AdapterEvent::Ended {
                        connection_id: epoch.connection_id.clone(),
                        reason: EndReason::Failed { detail: reason },
                    },
                    "call-ended",
                )
                .await;
            }
            ApiEvent::CallFailed {
                call_id,
                status_code,
                reason,
            } => {
                let Some(epoch) = self.existing_mapped_terminal_epoch(&call_id) else {
                    return;
                };
                self.deliver_terminal_for_epoch(
                    &epoch,
                    AdapterEvent::Failed {
                        connection_id: epoch.connection_id.clone(),
                        detail: format!("{} {}", status_code, reason),
                    },
                    "call-failed",
                )
                .await;
            }
            ApiEvent::CallCancelled { call_id } => {
                let Some(epoch) = self.existing_mapped_terminal_epoch(&call_id) else {
                    return;
                };
                self.deliver_terminal_for_epoch(
                    &epoch,
                    AdapterEvent::Ended {
                        connection_id: epoch.connection_id.clone(),
                        reason: EndReason::Cancelled,
                    },
                    "call-cancelled",
                )
                .await;
            }
            ApiEvent::InfoReceived { call_id, request } => {
                let status = if let Some((digits, duration_ms)) = parse_sip_info_dtmf(&request) {
                    match self.existing_mapped_epoch(&call_id) {
                        Some(epoch)
                            if self.try_send_for_epoch(
                                &epoch,
                                AdapterEvent::Dtmf {
                                    connection_id: epoch.connection_id.clone(),
                                    digits,
                                    duration_ms,
                                },
                            ) =>
                        {
                            200
                        }
                        Some(_) => 503,
                        None => 481,
                    }
                } else {
                    501
                };
                match request.respond(status) {
                    Ok(response) => {
                        if let Err(error) = response.send().await {
                            warn!(
                                status_code = status,
                                error_class = "exact-info-response",
                                "SipAdapter failed to answer inbound SIP INFO: {error}"
                            );
                        }
                    }
                    Err(error) => {
                        warn!(
                            status_code = status,
                            error_class = "exact-info-builder",
                            "SipAdapter could not correlate inbound SIP INFO: {error}"
                        );
                    }
                }
            }
            ApiEvent::DtmfReceived { call_id, digit } => {
                // P12.8 — surface inbound DTMF (RFC 2833 + SIP INFO,
                // decoded by media-core's DTMF detector) as an
                // AdapterEvent the orchestrator translates to
                // Event::DtmfReceived. Duration is the typical RFC
                // 4733 default (100ms) — the underlying ApiEvent
                // doesn't carry per-digit timing.
                let Some(epoch) = self.existing_mapped_epoch(&call_id) else {
                    return;
                };
                self.try_send_for_epoch(
                    &epoch,
                    AdapterEvent::Dtmf {
                        connection_id: epoch.connection_id.clone(),
                        digits: digit.to_string(),
                        duration_ms: 100,
                    },
                );
            }
            ApiEvent::MessageReceived { call_id, request } => {
                // MESSAGE is meaningful only on an already admitted dialog.
                // Never create a route from a late or out-of-dialog event.
                let epoch = self.existing_mapped_epoch(&call_id);
                let Some(epoch) = epoch else {
                    return;
                };
                let Some(raw_request) = request.raw_request() else {
                    warn!("SipAdapter discarded SIP MESSAGE without retained wire request");
                    return;
                };
                match self.publish_sip_data_message_for_epoch(&epoch, raw_request) {
                    Ok(true) => {}
                    Ok(false) => {
                        warn!("SipAdapter data message event channel was unavailable");
                    }
                    Err(error) => {
                        warn!(
                            error_class = if error.is_unsupported() {
                                "unsupported-reliability"
                            } else {
                                "invalid-message"
                            },
                            "SipAdapter rejected inbound SIP MESSAGE"
                        );
                    }
                }
            }
            ApiEvent::TransferAccepted { call_id, .. } => {
                let Some(epoch) = self.existing_mapped_epoch(&call_id) else {
                    return;
                };
                let Some(attempt_id) = self.transfer_attempt_for_status(&epoch, false) else {
                    return;
                };
                self.try_send_for_epoch(
                    &epoch,
                    AdapterEvent::TransferStatus {
                        connection_id: epoch.connection_id.clone(),
                        attempt_id,
                        status: TransferStatus::Accepted,
                    },
                );
            }
            ApiEvent::ReferProgress {
                call_id,
                status_code,
                reason,
            } => {
                let Some(epoch) = self.existing_mapped_epoch(&call_id) else {
                    return;
                };
                let Some(attempt_id) = self.transfer_attempt_for_status(&epoch, false) else {
                    return;
                };
                self.try_send_for_epoch(
                    &epoch,
                    AdapterEvent::TransferStatus {
                        connection_id: epoch.connection_id.clone(),
                        attempt_id,
                        status: TransferStatus::Progress {
                            status_code,
                            reason,
                        },
                    },
                );
            }
            ApiEvent::ReferCompleted {
                call_id,
                status_code,
                reason,
                ..
            } => {
                let Some(epoch) = self.existing_mapped_epoch(&call_id) else {
                    return;
                };
                let Some(attempt_id) = self.transfer_attempt_for_status(&epoch, true) else {
                    return;
                };
                self.try_send_for_epoch(
                    &epoch,
                    AdapterEvent::TransferStatus {
                        connection_id: epoch.connection_id.clone(),
                        attempt_id,
                        status: TransferStatus::Completed {
                            status_code,
                            reason,
                        },
                    },
                );
            }
            ApiEvent::TransferFailed {
                call_id,
                status_code,
                reason,
            } => {
                let Some(epoch) = self.existing_mapped_epoch(&call_id) else {
                    return;
                };
                let Some(attempt_id) = self.transfer_attempt_for_status(&epoch, true) else {
                    return;
                };
                self.try_send_for_epoch(
                    &epoch,
                    AdapterEvent::TransferStatus {
                        connection_id: epoch.connection_id.clone(),
                        attempt_id,
                        status: TransferStatus::Failed {
                            status_code,
                            reason,
                        },
                    },
                );
            }
            ApiEvent::MediaQualityChanged {
                call_id,
                packet_loss_percent,
                jitter_ms,
            } => {
                // P12.8 — surface per-Connection media quality (RTCP
                // RR / XR, distilled by media-core) into the
                // orchestrator's `QualityAggregator` via
                // `AdapterEvent::Quality`. MOS estimation lives in
                // media-core and is not propagated through the
                // current ApiEvent shape; leave as `None` until the
                // ApiEvent grows a `mos` field.
                let Some(epoch) = self.existing_mapped_epoch(&call_id) else {
                    return;
                };
                self.try_send_for_epoch(
                    &epoch,
                    AdapterEvent::Quality {
                        connection_id: epoch.connection_id.clone(),
                        snapshot: rvoip_core::stream::QualitySnapshot {
                            jitter_ms: jitter_ms as f32,
                            packet_loss_pct: packet_loss_percent as f32,
                            mos: None,
                        },
                    },
                );
            }
            other => {
                self.try_send(AdapterEvent::Native {
                    kind: "sip.api_event",
                    detail: format!("{:?}", other),
                });
            }
        }
    }

    fn try_send(&self, event: AdapterEvent) -> bool {
        if let Some(connection_id) = Self::adapter_event_connection_id(&event) {
            let epoch = {
                let _mapping = self
                    .mapping_lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                self.route_epoch_for_connection_locked(connection_id)
            };
            return epoch.is_some_and(|epoch| self.try_send_for_epoch(&epoch, event));
        }
        if let Err(e) = self
            .out_tx
            .try_send(OrchestratorAdapterEvent::Public(event))
        {
            warn!(
                ?e,
                "SipAdapter event channel full or closed; dropping event"
            );
            false
        } else {
            true
        }
    }

    /// Stage or publish an event while the route epoch remains current.
    ///
    /// The nonblocking channel write occurs under `mapping_lock`, so terminal
    /// retirement cannot linearize between the liveness check and enqueue.
    /// Stale supervisors are treated as successfully discarded.
    fn try_send_for_epoch(&self, epoch: &SipRouteEpoch, event: AdapterEvent) -> bool {
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.epoch_is_current_locked(epoch) {
            return true;
        }
        if Self::adapter_event_connection_id(&event)
            .is_some_and(|connection_id| *connection_id != epoch.connection_id)
        {
            warn!("SipAdapter discarded event whose connection did not match its route epoch");
            return true;
        }
        if let Some(route) = self.outbound_routes.get(&epoch.connection_id) {
            if !epoch.matches_route(route.value()) {
                return true;
            }
            match route.stage_event(event.clone()) {
                SipRouteStageDisposition::Retained | SipRouteStageDisposition::Discard => {
                    return true;
                }
                SipRouteStageDisposition::Forward => {}
            }
        }
        if let Err(error) = self
            .out_tx
            .try_send(OrchestratorAdapterEvent::Public(event))
        {
            warn!(%error, "SipAdapter event channel full or closed");
            return false;
        }
        true
    }

    fn try_publish_inbound_for_epoch(
        &self,
        epoch: &SipRouteEpoch,
        event: OrchestratorAdapterEvent,
    ) -> SipInboundPublication {
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.draining.load(Ordering::Acquire) || !self.epoch_is_current_locked(epoch) {
            return SipInboundPublication::Retired;
        }
        match self.out_tx.try_send(event) {
            Ok(()) => SipInboundPublication::Published,
            Err(error) => {
                warn!(%error, "SipAdapter inbound event channel full or closed");
                SipInboundPublication::Backpressured
            }
        }
    }

    #[cfg(test)]
    async fn send_inbound_event(&self, event: OrchestratorAdapterEvent) -> bool {
        Self::send_inbound_event_to(&self.out_tx, event).await
    }

    #[cfg(test)]
    async fn send_inbound_event_to(
        events: &mpsc::Sender<OrchestratorAdapterEvent>,
        event: OrchestratorAdapterEvent,
    ) -> bool {
        match tokio::time::timeout(SIP_INBOUND_EVENT_DELIVERY_TIMEOUT, events.send(event)).await {
            Ok(Ok(())) => true,
            Ok(Err(_)) => {
                warn!("SipAdapter inbound event channel closed");
                false
            }
            Err(_) => {
                warn!("SipAdapter inbound event delivery timed out");
                false
            }
        }
    }

    async fn deliver_terminal_event(&self, event: AdapterEvent, source: &'static str) {
        Self::deliver_terminal_event_to(&self.lifecycle, &self.out_tx, event, source).await;
    }

    async fn deliver_terminal_for_epoch(
        &self,
        epoch: &SipRouteEpoch,
        event: AdapterEvent,
        source: &'static str,
    ) {
        let (route, stream, deliver_inbound) = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !self.epoch_is_current_locked(epoch) {
                return;
            }
            if let Some(route) = self.outbound_routes.get(&epoch.connection_id) {
                if !epoch.matches_route(route.value()) {
                    return;
                }
                let route = Arc::clone(route.value());
                match route.stage_event(event.clone()) {
                    SipRouteStageDisposition::Retained => (Some(route), None, false),
                    SipRouteStageDisposition::Forward => {
                        warn!(
                            source,
                            "SipAdapter outbound terminal escaped retained route"
                        );
                        return;
                    }
                    SipRouteStageDisposition::Discard => return,
                }
            } else {
                (
                    None,
                    self.remove_epoch_locked(epoch)
                        .and_then(|removed| removed.stream),
                    true,
                )
            }
        };
        if let Some(route) = route {
            begin_outbound_cleanup(
                self.self_weak.clone(),
                Arc::clone(&self.coordinator),
                route,
                self.lifecycle.clone(),
                self.out_tx.clone(),
                None,
                source,
            );
        } else if deliver_inbound {
            if let Some(stream) = stream {
                self.close_stream_retained(stream);
            }
            self.deliver_terminal_event(event, source).await;
        }
    }

    async fn deliver_terminal_event_to(
        lifecycle: &AdapterLifecycleSinkSlot,
        events: &mpsc::Sender<OrchestratorAdapterEvent>,
        event: AdapterEvent,
        source: &'static str,
    ) {
        let delivery = lifecycle
            .queue_or_deliver_orchestrator_terminal(events, event)
            .await;
        if delivery == TerminalDelivery::Undeliverable {
            warn!(source, "SipAdapter terminal event was undeliverable");
        }
    }

    fn map_session_err(err: crate::errors::SessionError) -> RvoipError {
        RvoipError::Adapter(format!("rvoip-sip: {}", err))
    }
}

async fn wait_for_route_cancel(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

/// Linearize a successful adapter activation with the exact SIP dialog being
/// answered. INVITE dispatch and dormant media binding are not sufficient:
/// returning a receipt while the coordinator is still `Initiating` lets an
/// immediate application hangup enter the CANCEL path even if a peer's fast
/// 200 OK has already won the race. The late-answer compensation then authors
/// a BYE whose final response is outside the caller's teardown result.
async fn wait_for_outbound_session_active(
    coordinator: &UnifiedCoordinator,
    route: &SipOutboundRoute,
) -> Result<(), SipActivationFailure> {
    let deadline = tokio::time::Instant::now() + SIP_OUTBOUND_ACTIVATION_TIMEOUT;
    let mut lifecycle = coordinator.lifecycle_watcher(&route.session_id);
    let mut cancel = route.cancel.subscribe();

    loop {
        let snapshot = coordinator.lifecycle_snapshot(&route.session_id).await;
        if snapshot.terminal.is_some() {
            return Err(SipActivationFailure::RouteEnded);
        }
        match snapshot.state {
            Some(CallState::Active) => return Ok(()),
            Some(state)
                if state.is_final()
                    || matches!(
                        state,
                        CallState::CancelPending | CallState::Cancelling | CallState::Terminating
                    ) =>
            {
                return Err(SipActivationFailure::RouteEnded);
            }
            None => return Err(SipActivationFailure::RouteEnded),
            Some(_) => {}
        }

        tokio::select! {
            biased;
            _ = wait_for_route_cancel(&mut cancel) => {
                return Err(SipActivationFailure::RouteEnded);
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(SipActivationFailure::InviteFailed);
            }
            changed = lifecycle.changed() => {
                if changed.is_err() {
                    return Err(SipActivationFailure::RouteEnded);
                }
            }
        }
    }
}

async fn wait_for_outbound_session_release(
    coordinator: &UnifiedCoordinator,
    session_id: &SessionId,
) -> bool {
    let deadline = tokio::time::Instant::now() + SIP_RETAINED_TASK_TIMEOUT;
    let mut poll_delay = Duration::from_millis(5);
    loop {
        match tokio::time::timeout_at(deadline, coordinator.get_state(session_id)).await {
            Ok(Ok(_)) => {}
            Ok(Err(_)) => return true,
            Err(_) => return false,
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        tokio::time::sleep_until(std::cmp::min(deadline, now + poll_delay)).await;
        poll_delay = std::cmp::min(poll_delay.saturating_mul(2), Duration::from_millis(100));
    }
}

fn outbound_force_reclaim_required(finalized: bool, session_released: bool) -> bool {
    !finalized || !session_released
}

async fn run_inbound_drain(
    coordinator: Arc<UnifiedCoordinator>,
    epoch: SipRouteEpoch,
    stream: Option<Arc<crate::media_stream::SipMediaStream>>,
    fast_auto_accept: bool,
    lifecycle: AdapterLifecycleSinkSlot,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
    cleanup_result: Arc<AtomicBool>,
) {
    let state = tokio::time::timeout(
        SIP_RETAINED_TASK_TIMEOUT,
        coordinator.session_state(&epoch.session_id),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .map(|session| session.call_state);
    let action = failed_inbound_termination(state, fast_auto_accept);
    let network_complete = match action {
        FailedInboundTermination::Reject => {
            let rejected = matches!(
                tokio::time::timeout(
                    SIP_RETAINED_TASK_TIMEOUT,
                    coordinator
                        .reject(&epoch.session_id)
                        .with_status(503)
                        .with_reason("SIP Adapter Draining")
                        .send(),
                )
                .await,
                Ok(Ok(()))
            );
            rejected
                || matches!(
                    tokio::time::timeout(
                        SIP_RETAINED_TASK_TIMEOUT,
                        coordinator.hangup(&epoch.session_id),
                    )
                    .await,
                    Ok(Ok(()))
                )
        }
        FailedInboundTermination::Hangup => matches!(
            tokio::time::timeout(
                SIP_RETAINED_TASK_TIMEOUT,
                coordinator.hangup(&epoch.session_id),
            )
            .await,
            Ok(Ok(()))
        ),
        FailedInboundTermination::CleanupOnly => false,
    };
    let network_success = network_complete
        || matches!(
            tokio::time::timeout(
                SIP_RETAINED_TASK_TIMEOUT,
                coordinator.finalize_local_bye(&epoch.session_id, "SIP adapter drain"),
            )
            .await,
            Ok(Ok(()))
        );

    let media_success = if let Some(stream) = stream {
        matches!(
            tokio::time::timeout(
                SIP_RETAINED_TASK_TIMEOUT,
                (stream as Arc<dyn MediaStream>).close(),
            )
            .await,
            Ok(Ok(()))
        )
    } else {
        true
    };
    cleanup_result.store(network_success && media_success, Ordering::Release);
    SipAdapter::deliver_terminal_event_to(
        &lifecycle,
        &events,
        AdapterEvent::Ended {
            connection_id: epoch.connection_id,
            reason: EndReason::BridgeTorn,
        },
        "adapter-drain-inbound",
    )
    .await;
}

async fn run_outbound_activation(
    weak_adapter: Weak<SipAdapter>,
    coordinator: Arc<UnifiedCoordinator>,
    route: Arc<SipOutboundRoute>,
    lifecycle: AdapterLifecycleSinkSlot,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
    retained_tasks: Arc<RetainedTasks>,
) {
    let result = activate_outbound_route(
        weak_adapter.clone(),
        Arc::clone(&coordinator),
        Arc::clone(&route),
        events.clone(),
    )
    .await;
    match result {
        Ok(receipt) => {
            let media_weak_adapter = weak_adapter.clone();
            let media_coordinator = Arc::clone(&coordinator);
            let media_route = Arc::clone(&route);
            let media_lifecycle = lifecycle.clone();
            let media_events = events.clone();
            let monitor_spawned = retained_tasks.spawn(async move {
                monitor_outbound_media(
                    media_weak_adapter,
                    media_coordinator,
                    media_route,
                    media_lifecycle,
                    media_events,
                )
                .await;
            });
            if !monitor_spawned || !route.complete_activation_success(receipt) {
                route.complete_activation_failure(SipActivationFailure::RouteEnded);
                begin_outbound_cleanup(
                    weak_adapter,
                    coordinator,
                    route,
                    lifecycle,
                    events,
                    None,
                    "activation-terminal",
                );
            }
        }
        Err(failure) => {
            route.complete_activation_failure(failure);
            begin_outbound_cleanup(
                weak_adapter,
                coordinator,
                Arc::clone(&route),
                lifecycle,
                events,
                Some(AdapterEvent::Failed {
                    connection_id: route.connection_id.clone(),
                    detail: "SIP outbound activation failed".to_string(),
                }),
                "activation-failed",
            );
        }
    }
}

async fn activate_outbound_route(
    weak_adapter: Weak<SipAdapter>,
    coordinator: Arc<UnifiedCoordinator>,
    route: Arc<SipOutboundRoute>,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
) -> Result<OutboundActivation, SipActivationFailure> {
    let from = route
        .context
        .from_uri()
        .map(str::to_owned)
        .unwrap_or_else(|| "sip:anonymous@invalid".to_string());
    let mut builder = coordinator
        .invite(Some(from), route.target.clone())
        .with_reserved_session_id(route.session_id.clone());
    if let Some(auth) = route.context.auth() {
        builder = builder.with_auth(auth.clone());
    }
    if let Some(outbound_proxy_uri) = route.context.outbound_proxy_uri() {
        builder = builder.with_outbound_proxy(outbound_proxy_uri.to_owned());
    }
    let headers = route
        .context
        .initial_headers()
        .iter()
        .map(|(name, value)| {
            TypedHeader::Other(name.clone(), HeaderValue::Raw(value.as_bytes().to_vec()))
        })
        .collect();
    builder = builder
        .with_headers(headers)
        .map_err(|_| SipActivationFailure::InvalidPlan)?;

    // `Possible` begins immediately before the first poll of `send`: cleanup
    // must assume a packet escaped if this future is subsequently cancelled.
    route.begin_wire_send()?;
    let mut cancel = route.cancel.subscribe();
    let send = builder.send();
    tokio::pin!(send);
    let returned_session = tokio::select! {
        biased;
        _ = wait_for_route_cancel(&mut cancel) => {
            return Err(SipActivationFailure::RouteEnded);
        }
        result = tokio::time::timeout(SIP_RETAINED_TASK_TIMEOUT, &mut send) => {
            result
                .map_err(|_| SipActivationFailure::InviteFailed)?
                .map_err(|_| SipActivationFailure::InviteFailed)?
        },
    };
    if returned_session != route.session_id {
        return Err(SipActivationFailure::InviteFailed);
    }
    route.mark_wire_sent();

    // The coordinator has now admitted the reserved SessionId through the
    // shared authority. Replace the preparation-object identity with that
    // exact handle before any staged event or media binding becomes public.
    let adapter = weak_adapter
        .upgrade()
        .ok_or(SipActivationFailure::RouteEnded)?;
    let handle = adapter
        .current_session_handle(&route.session_id)
        .ok_or(SipActivationFailure::InviteFailed)?;
    let promoted = {
        let _mapping = adapter
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let epoch = adapter
            .route_epoch_for_connection_locked(&route.connection_id)
            .filter(|epoch| epoch.matches_route(&route));
        epoch.and_then(|epoch| adapter.promote_prepared_epoch_locked(epoch, handle))
    };
    if promoted.is_none() {
        return Err(SipActivationFailure::RouteEnded);
    }
    drop(adapter);

    let external_reference =
        ExternalConnectionReference::new("sip.call-id", Arc::clone(&route.sip_call_id))
            .map_err(|_| SipActivationFailure::InvalidPlan)?;
    let receipt = OutboundActivation::with_external_reference(external_reference);

    let remote_terminal = route.publish_staged_events(&events).await?;
    if remote_terminal {
        return Err(SipActivationFailure::RouteEnded);
    }

    // Bind the receive path as soon as the exact route is published. The
    // retained driver remains dormant until a 183 or final answer commits
    // negotiated media, while outbound writes stay disabled until
    // complete_activation_success. This permits receive-only early media
    // without treating provisional signaling as final activation.
    let mut cancel = route.cancel.subscribe();
    let bind = route
        .stream
        .start_bind(Arc::clone(&coordinator), route.session_id.clone());
    tokio::pin!(bind);
    tokio::select! {
        biased;
        _ = wait_for_route_cancel(&mut cancel) => {
            return Err(SipActivationFailure::RouteEnded);
        }
        result = tokio::time::timeout(SIP_RETAINED_TASK_TIMEOUT, &mut bind) => {
            result
                .map_err(|_| SipActivationFailure::MediaFailed)?
                .map_err(|_| SipActivationFailure::MediaFailed)?
        },
    }
    if !route.stream.is_bound_to(&coordinator, &route.session_id) {
        return Err(SipActivationFailure::MediaFailed);
    }

    wait_for_outbound_session_active(&coordinator, &route).await?;

    Ok(receipt)
}

async fn monitor_outbound_media(
    weak_adapter: Weak<SipAdapter>,
    coordinator: Arc<UnifiedCoordinator>,
    route: Arc<SipOutboundRoute>,
    lifecycle: AdapterLifecycleSinkSlot,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
) {
    let mut media = route.stream.subscribe_lifecycle();
    loop {
        // Copy the state out of the watch guard before an arm can request
        // cleanup. Cleanup closes the same stream and publishes a new
        // lifecycle state; retaining the read guard across that publication
        // would self-deadlock the Tokio worker.
        let media_state = *media.borrow_and_update();
        match media_state {
            crate::media_stream::SipMediaLifecycle::Failed => {
                begin_outbound_cleanup(
                    weak_adapter,
                    coordinator,
                    Arc::clone(&route),
                    lifecycle,
                    events,
                    Some(AdapterEvent::Failed {
                        connection_id: route.connection_id.clone(),
                        detail: "SIP media driver failed".to_string(),
                    }),
                    "media-failed",
                );
                return;
            }
            crate::media_stream::SipMediaLifecycle::Closing
            | crate::media_stream::SipMediaLifecycle::Closed => return,
            crate::media_stream::SipMediaLifecycle::Dormant
            | crate::media_stream::SipMediaLifecycle::Binding
            | crate::media_stream::SipMediaLifecycle::Bound => {}
        }
        if media.changed().await.is_err() {
            return;
        }
    }
}

fn begin_outbound_cleanup(
    weak_adapter: Weak<SipAdapter>,
    coordinator: Arc<UnifiedCoordinator>,
    route: Arc<SipOutboundRoute>,
    lifecycle: AdapterLifecycleSinkSlot,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
    event: Option<AdapterEvent>,
    source: &'static str,
) {
    route.complete_activation_failure(SipActivationFailure::RouteEnded);
    if !route.request_cleanup(event) {
        return;
    }
    let Some(adapter) = weak_adapter.upgrade() else {
        route.complete_cleanup(false);
        return;
    };
    let retained_tasks = Arc::clone(&adapter.retained_tasks);
    let cleanup_tasks = Arc::clone(&retained_tasks);
    drop(adapter);
    let completion_route = Arc::clone(&route);
    let spawned = retained_tasks.spawn(async move {
        run_outbound_cleanup(
            weak_adapter,
            coordinator,
            route,
            lifecycle,
            events,
            source,
            cleanup_tasks,
        )
        .await;
    });
    if !spawned {
        completion_route.complete_cleanup(false);
    }
}

async fn run_outbound_cleanup(
    weak_adapter: Weak<SipAdapter>,
    coordinator: Arc<UnifiedCoordinator>,
    route: Arc<SipOutboundRoute>,
    lifecycle: AdapterLifecycleSinkSlot,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
    source: &'static str,
    retained_tasks: Arc<RetainedTasks>,
) {
    let (wire, remote_terminal) = route.cleanup_snapshot();
    let mut wire_teardown_success = wire == SipOutboundWireState::NotStarted || remote_terminal;
    let mut confirmed_hangup_timed_out = false;
    let mut local_release_success = true;
    if wire != SipOutboundWireState::NotStarted {
        if !remote_terminal {
            // `hangup` is the sole network compensation authority. Depending
            // on dialog phase it emits the one legal CANCEL or BYE; this
            // helper never retries it and therefore cannot duplicate the
            // teardown request.
            let hangup = tokio::time::timeout(
                SIP_CONFIRMED_HANGUP_TIMEOUT,
                coordinator.hangup(&route.session_id),
            )
            .await;
            confirmed_hangup_timed_out = hangup.is_err();
            wire_teardown_success = matches!(hangup, Ok(Ok(())));
        }

        // Early CANCEL returns after dispatch and the coordinator's normal
        // peer-terminal watchdog is intentionally much longer than adapter
        // drain. Give the peer one bounded window, then release local dialog,
        // media, and session ownership without sending another packet. An
        // established BYE whose confirmed-hangup deadline already elapsed has
        // consumed that bounded window, so proceed directly to exact local
        // finalization instead of spending a second full wait interval.
        local_release_success = if confirmed_hangup_timed_out {
            false
        } else {
            wait_for_outbound_session_release(&coordinator, &route.session_id).await
        };
        if !local_release_success {
            let finalized = matches!(
                tokio::time::timeout(
                    SIP_RETAINED_TASK_TIMEOUT,
                    coordinator
                        .finalize_local_bye(&route.session_id, "SIP outbound retained cleanup",),
                )
                .await,
                Ok(Ok(()))
            );
            let released_after_finalization = if finalized {
                wait_for_outbound_session_release(&coordinator, &route.session_id).await
            } else {
                false
            };
            if outbound_force_reclaim_required(finalized, released_after_finalization) {
                // Terminal event publication is deliberately synchronous on
                // the normal path, but a blocked/failed application consumer
                // must never retain SIP capacity. This fallback omits event
                // publication and makes authoritative local reclamation the
                // first cancellation-safe operation.
                if let Ok(cleanup) = tokio::time::timeout(
                    SIP_RETAINED_TASK_TIMEOUT,
                    coordinator.begin_force_reclaim_local_session(&route.session_id),
                )
                .await
                {
                    // Authoritative store/registry ownership was already
                    // released. The lower continuation remains retained even
                    // if adapter drain closes new task admission while this
                    // parent cleanup is running.
                    retained_tasks.spawn_child(cleanup.finish());
                }
            }
            local_release_success =
                wait_for_outbound_session_release(&coordinator, &route.session_id).await;
        }
    }

    // SipMediaStream::close owns bounded driver joins and publishes Closed
    // only after they finish or are aborted.
    let stream: Arc<dyn MediaStream> = Arc::clone(&route.stream) as Arc<dyn MediaStream>;
    let media_success = stream.close().await.is_ok();
    let terminal = route.seal_cleanup_event();
    if let Some(adapter) = weak_adapter.upgrade() {
        adapter.retire_outbound_route(&route);
    }
    if let Some(event) = terminal {
        SipAdapter::deliver_terminal_event_to(&lifecycle, &events, event, source).await;
    }
    route.complete_cleanup(wire_teardown_success && local_release_success && media_success);
}

impl Drop for SipAdapter {
    fn drop(&mut self) {
        self.draining.store(true, Ordering::Release);
        self.translator_cancel.send_replace(true);
        for stream in self.streams_cache.iter() {
            stream.value().request_close();
        }
        self.retained_tasks.close();
        if self.observer_registered.swap(false, Ordering::AcqRel) {
            self.coordinator
                .remove_inbound_invite_observer(self.inbound_invite_observer_id);
        }
    }
}

#[async_trait::async_trait]
impl ConnectionAdapter for SipAdapter {
    fn transport(&self) -> Transport {
        Transport::Sip
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities {
            authoritative_liveness: true,
            atomic_inbound_handoff: true,
            terminal_fallback: true,
            staged_outbound_activation: true,
        }
    }

    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> CoreResult<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("SIP lifecycle sink already installed"))
    }

    fn is_connection_live(&self, conn: &ConnectionId) -> bool {
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.by_connection.contains_key(conn) {
            return false;
        }
        self.outbound_routes
            .get(conn)
            .is_none_or(|route| route.is_publicly_live())
    }

    fn take_inbound_context(&self, conn: &ConnectionId) -> Option<InboundConnectionContext> {
        self.inbound_contexts.take(conn)
    }

    async fn originate(&self, request: OriginateRequest) -> CoreResult<ConnectionHandle> {
        Self::validate_outbound_target(&request.target)?;
        let originate_context = Self::outbound_originate_context(&request)?;
        let session_id = SessionId::new();
        let conn_id = ConnectionId::new();
        let stream = crate::media_stream::SipMediaStream::dormant_deferred(Direction::Outbound);
        let route = SipOutboundRoute::new(
            conn_id.clone(),
            session_id,
            request.target.clone(),
            originate_context,
            stream,
        );
        self.reserve_outbound_route(Arc::clone(&route))?;
        let mut connection = self
            .build_connection(conn_id.clone(), Direction::Outbound)
            .await;
        if !self.is_connection_live(&conn_id) {
            return Err(RvoipError::AdmissionRejected(
                "SIP outbound route ended during connection construction",
            ));
        }
        // Carry the caller-supplied vocabulary IDs through so the consumer's
        // session/participant stay coherent.
        connection.session_id = request.session_id;
        connection.participant_id = request.participant_id;
        connection.capabilities = request.capabilities;
        Ok(ConnectionHandle::new(connection))
    }

    async fn activate_outbound(&self, conn: ConnectionId) -> CoreResult<()> {
        self.activate_outbound_with_receipt(conn).await.map(|_| ())
    }

    async fn activate_outbound_with_receipt(
        &self,
        conn: ConnectionId,
    ) -> CoreResult<OutboundActivation> {
        let route = self
            .outbound_routes
            .get(&conn)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        if self.draining.load(Ordering::Acquire) {
            route.complete_activation_failure(SipActivationFailure::RouteEnded);
            begin_outbound_cleanup(
                self.self_weak.clone(),
                Arc::clone(&self.coordinator),
                Arc::clone(&route),
                self.lifecycle.clone(),
                self.out_tx.clone(),
                None,
                "activation-during-drain",
            );
            return Err(SipActivationFailure::RouteEnded.into_error());
        }
        match route.claim_activation() {
            Ok(true) => {
                let weak_adapter = self.self_weak.clone();
                let coordinator = Arc::clone(&self.coordinator);
                let lifecycle = self.lifecycle.clone();
                let events = self.out_tx.clone();
                let retained_route = Arc::clone(&route);
                let retained_tasks = Arc::clone(&self.retained_tasks);
                let activation_tasks = Arc::clone(&retained_tasks);
                let spawned = retained_tasks.spawn(async move {
                    run_outbound_activation(
                        weak_adapter,
                        coordinator,
                        retained_route,
                        lifecycle,
                        events,
                        activation_tasks,
                    )
                    .await;
                });
                if !spawned {
                    route.complete_activation_failure(SipActivationFailure::RouteEnded);
                    begin_outbound_cleanup(
                        self.self_weak.clone(),
                        Arc::clone(&self.coordinator),
                        Arc::clone(&route),
                        self.lifecycle.clone(),
                        self.out_tx.clone(),
                        None,
                        "activation-not-admitted",
                    );
                }
            }
            Ok(false) => {}
            Err(error) => return Err(error.into_error()),
        }
        route.wait_activation().await
    }

    async fn start_inbound_early_media(&self, conn: ConnectionId) -> CoreResult<()> {
        if self.outbound_routes.contains_key(&conn) {
            return Err(RvoipError::InvalidState(
                "SIP provisional early media requires an inbound route",
            ));
        }
        let epoch = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.route_epoch_for_connection_locked(&conn)
        }
        .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let stream = self
            .streams_cache
            .get(&conn)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::InvalidState("SIP inbound media stream is unavailable"))?;

        self.coordinator
            .send_early_media(&epoch.session_id, None)
            .await
            .map_err(Self::map_session_err)?;
        // The eager inbound bind task and this waiter converge on the same
        // immutable coordinator/session target. Waiting here makes a
        // successful core provisional-route call mean the negotiated codec
        // and SRTP-backed writer are ready, without taking the source receiver.
        stream
            .bind(Arc::clone(&self.coordinator), epoch.session_id.clone())
            .await
            .map_err(Self::map_session_err)?;
        let still_current = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.epoch_is_current_locked(&epoch)
        };
        if !still_current {
            return Err(RvoipError::ConnectionNotFound(conn));
        }
        Ok(())
    }

    async fn accept(&self, conn: ConnectionId) -> CoreResult<()> {
        let outbound = self.outbound_routes.contains_key(&conn);
        if outbound {
            self.activate_outbound(conn.clone()).await?;
        }
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .accept_call(&session_id)
            .await
            .map_err(Self::map_session_err)?;
        if outbound {
            return Ok(());
        }

        // `accept_call` completes after sending the inbound UAS 200 response;
        // the SIP connection is not established until the peer's ACK moves
        // the shared session from Answering to Active. Outbound UAC sessions
        // publish `CallAnswered` when their 200 arrives, but no equivalent API
        // event is emitted for an inbound ACK. Waiting here keeps that
        // protocol distinction out of the public event stream and lets the
        // adapter publish exactly the transport-neutral Connected event at
        // the real establishment boundary.
        let deadline = Instant::now() + self.coordinator.setup_teardown_timeout_duration();
        loop {
            if self.draining.load(Ordering::Acquire) {
                return Err(RvoipError::AdmissionRejected(
                    "SIP adapter began draining while an inbound accept awaited ACK",
                ));
            }
            match self.coordinator.get_state(&session_id).await {
                Ok(CallState::Active | CallState::Bridged) => {
                    let epoch = self
                        .existing_mapped_epoch(&session_id)
                        .filter(|epoch| epoch.connection_id == conn)
                        .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
                    if !self.try_send_for_epoch(
                        &epoch,
                        AdapterEvent::Connected {
                            connection_id: conn.clone(),
                        },
                    ) {
                        return Err(RvoipError::AdmissionRejected(
                            "SIP connected event was backpressured",
                        ));
                    }
                    return Ok(());
                }
                Ok(state) if state.is_final() || state == CallState::Terminating => {
                    return Err(RvoipError::InvalidState(
                        "SIP inbound session ended before ACK establishment",
                    ));
                }
                Ok(_) => {}
                Err(_) if self.lookup_session(&conn).is_err() => {
                    return Err(RvoipError::ConnectionNotFound(conn));
                }
                Err(_) => {}
            }
            if Instant::now() >= deadline {
                return Err(RvoipError::AdmissionRejected(
                    "SIP inbound accept timed out awaiting ACK",
                ));
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> CoreResult<()> {
        let epoch = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.route_epoch_for_connection_locked(&conn)
        }
        .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let session_id = epoch.session_id.clone();
        let terminal_detail = format!("session rejected locally: {reason:?}");
        if let Some(route) = self
            .outbound_routes
            .get(&conn)
            .map(|entry| Arc::clone(entry.value()))
        {
            route.complete_activation_failure(SipActivationFailure::RouteEnded);
            begin_outbound_cleanup(
                self.self_weak.clone(),
                Arc::clone(&self.coordinator),
                Arc::clone(&route),
                self.lifecycle.clone(),
                self.out_tx.clone(),
                Some(AdapterEvent::Failed {
                    connection_id: conn,
                    detail: terminal_detail,
                }),
                "reject",
            );
            return route.wait_cleanup().await;
        }
        let (status, phrase) = match reason {
            RejectReason::Busy => (486, "Busy Here"),
            RejectReason::Decline => (603, "Decline"),
            RejectReason::NotFound => (404, "Not Found"),
            RejectReason::Forbidden => (403, "Forbidden"),
            RejectReason::NotAcceptable => (488, "Not Acceptable Here"),
            RejectReason::ServerError => (500, "Server Internal Error"),
            RejectReason::Custom { code, ref phrase } => (code, phrase.as_str()),
        };
        if !self.forget_epoch(&epoch) {
            return Err(RvoipError::ConnectionNotFound(conn));
        }
        let network_result = self
            .coordinator
            .reject(&session_id)
            .with_status(status)
            .with_reason(phrase)
            .send()
            .await
            .map_err(Self::map_session_err);
        self.deliver_terminal_event(
            AdapterEvent::Failed {
                connection_id: conn,
                detail: terminal_detail,
            },
            "reject",
        )
        .await;
        network_result
    }

    async fn end(&self, conn: ConnectionId, reason: EndReason) -> CoreResult<()> {
        let epoch = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.route_epoch_for_connection_locked(&conn)
        }
        .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let session_id = epoch.session_id.clone();
        if let Some(route) = self
            .outbound_routes
            .get(&conn)
            .map(|entry| Arc::clone(entry.value()))
        {
            route.complete_activation_failure(SipActivationFailure::RouteEnded);
            begin_outbound_cleanup(
                self.self_weak.clone(),
                Arc::clone(&self.coordinator),
                Arc::clone(&route),
                self.lifecycle.clone(),
                self.out_tx.clone(),
                Some(AdapterEvent::Ended {
                    connection_id: conn,
                    reason,
                }),
                "end",
            );
            return route.wait_cleanup().await;
        }
        if !self.forget_epoch(&epoch) {
            return Err(RvoipError::ConnectionNotFound(conn));
        }
        let network_result = self
            .coordinator
            .hangup(&session_id)
            .await
            .map_err(Self::map_session_err);
        self.deliver_terminal_event(
            AdapterEvent::Ended {
                connection_id: conn,
                reason,
            },
            "end",
        )
        .await;
        network_result
    }

    async fn hold(&self, conn: ConnectionId) -> CoreResult<()> {
        let session_id = self.lookup_session_for_non_terminal_control(&conn)?;
        self.coordinator
            .hold(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn resume(&self, conn: ConnectionId) -> CoreResult<()> {
        let session_id = self.lookup_session_for_non_terminal_control(&conn)?;
        self.coordinator
            .resume(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn transfer(&self, conn: ConnectionId, target: TransferTarget) -> CoreResult<()> {
        self.submit_transfer(conn, None, target).await
    }

    async fn transfer_with_attempt(
        &self,
        conn: ConnectionId,
        attempt_id: TransferAttemptId,
        target: TransferTarget,
    ) -> CoreResult<()> {
        self.submit_transfer(conn, Some(attempt_id), target).await
    }

    async fn streams(&self, conn: ConnectionId) -> CoreResult<Vec<Arc<dyn MediaStream>>> {
        // Streams are allocated locally in `build_connection`, so this lookup
        // never waits for coordinator media. Inbound binding starts in the
        // background; outbound binding starts only after durable activation.
        self.lookup_session(&conn)?;
        self.streams_cache
            .get(&conn)
            .map(|entry| vec![Arc::clone(entry.value()) as Arc<dyn MediaStream>])
            .ok_or_else(|| RvoipError::Adapter("SipAdapter media stream is unavailable".into()))
    }

    async fn send_message(&self, _conn: ConnectionId, _message: Message) -> CoreResult<()> {
        // SIP MESSAGE wiring lives in api::UnifiedCoordinator::send_message
        // (Step 8 hooks it up once the rvoip-core Message → SIP MESSAGE body
        //  shape is decided).
        Err(RvoipError::NotImplemented(
            "unsupported beta feature: rvoip-core Message to SIP MESSAGE bridging is post-beta for SipAdapter",
        ))
    }

    async fn send_data_message(&self, conn: ConnectionId, message: DataMessage) -> CoreResult<()> {
        let message = crate::sip_data_message::to_sip_data_message(&message).map_err(|error| {
            if error.is_unsupported() {
                RvoipError::NotImplemented(
                    "SIP MESSAGE supports reliable-ordered DataMessage delivery only",
                )
            } else {
                RvoipError::AdmissionRejected(
                    "SIP data message failed local header or body validation",
                )
            }
        })?;
        let epoch = self.data_message_route_epoch(&conn)?;
        let dialog_id = self.data_message_dialog_for_epoch(&epoch)?;
        self.coordinator
            .dialog_adapter()
            .send_data_message_on_dialog(&dialog_id, message)
            .await
            .map_err(|error| match error {
                crate::errors::SessionError::SessionNotFound(_)
                | crate::errors::SessionError::InvalidTransition(_) => {
                    RvoipError::InvalidState("SIP data message dialog is unavailable")
                }
                _ => RvoipError::Adapter(
                    "SIP data message dispatch failed (class=dialog-dispatch)".to_string(),
                ),
            })
    }

    async fn send_dtmf(
        &self,
        conn: ConnectionId,
        digits: &str,
        duration_ms: u32,
    ) -> CoreResult<()> {
        rvoip_media_core::relay::controller::dtmf_transmitter::validate_dtmf_sequence(
            digits,
            duration_ms,
            SIP_RFC4733_INTER_DIGIT_MS,
        )
        .map_err(|_| {
            RvoipError::AdmissionRejected(
                "SIP RFC 4733 sequence failed digit, duration, or schedule validation",
            )
        })?;
        let session_id = self.lookup_session_for_non_terminal_control(&conn)?;
        self.coordinator
            .send_dtmf_sequence(&session_id, digits, duration_ms, SIP_RFC4733_INTER_DIGIT_MS)
            .await
            .map_err(Self::map_session_err)
    }

    async fn renegotiate_media(
        &self,
        conn: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> CoreResult<NegotiatedCodecs> {
        // Gap plan §4.2C v1 punch list — fire a re-INVITE via the
        // existing `UnifiedCoordinator::reinvite(...).send()` builder.
        // The state machine handles the 200 OK SDP answer through
        // its `NegotiateSDPAsUAC` action; downstream session state
        // updates automatically.
        //
        // **Codec preference caveat.** This impl does NOT yet inject
        // the orchestrator-provided `capabilities.audio_codecs` into
        // the re-INVITE SDP — that would need a per-session
        // `set_offered_codecs` coordinator method that today only
        // exists at MediaAdapter construction time. The re-INVITE
        // still uses the SIP layer's configured `offered_codecs`.
        // For the cross-bridge use case (which is the v1 driver) this
        // is acceptable: the peer's answer SDP determines what the
        // bridged-side codec becomes, and the orchestrator's
        // transcoder hot-swap reads the post-renegotiation codec
        // off the negotiated session state.
        if capabilities.audio_codecs.is_empty() {
            return Err(RvoipError::UnsupportedCodec(
                "SipAdapter::renegotiate_media: empty audio_codecs in new capabilities".into(),
            ));
        }
        let session_id = self.lookup_session_for_non_terminal_control(&conn)?;
        self.coordinator
            .reinvite(&session_id)
            .send()
            .await
            .map_err(Self::map_session_err)?;
        // Optimistic return — the requested top preference. The state
        // machine's NegotiateSDPAsUAC asynchronously updates
        // `session.negotiated_config` when the 200 OK arrives; the
        // orchestrator-level hot-swap then reads the live codec via
        // `adapter.streams(...)` (see `Orchestrator::renegotiate_media`).
        let chosen = capabilities.audio_codecs.first().cloned().unwrap();
        Ok(NegotiatedCodecs {
            audio: Some(chosen),
            video: None,
        })
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        self.try_subscribe_events()
            .expect("SipAdapter::subscribe_events already consumed")
    }

    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        self.try_subscribe_atomic_events()
            .expect("SipAdapter atomic event stream already consumed")
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        // Step-7 returns the empty descriptor. Real codec/feature discovery
        // happens by inspecting the negotiated session in step 8+.
        CapabilityDescriptor::default()
    }

    async fn verify_request_signature(
        &self,
        _conn: ConnectionId,
        _signature: SignatureHeaders,
    ) -> CoreResult<IdentityAssurance> {
        // Per INTERFACE_DESIGN §6: SIP/WebRTC interop adapters return
        // Anonymous unless the peer presents an HTTP-mediated AAuth/OAuth
        // surface. For v1 SIP we always return Anonymous.
        Ok(IdentityAssurance::Anonymous)
    }
}

#[cfg(test)]
mod inbound_context_tests {
    use super::*;
    use crate::state_table::types::Role;
    use rvoip_core::identity::{AuthenticationMethod, IdentityAssurance};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct RecordingLifecycleSink {
        deliveries: AtomicUsize,
    }

    #[test]
    fn sip_info_dtmf_parser_accepts_only_valid_dtmf_relay_payloads() {
        let raw = b"INFO sip:bob@example.test SIP/2.0\r\n\
Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-adapter-info\r\n\
From: <sip:alice@example.test>;tag=from\r\n\
To: <sip:bob@example.test>;tag=to\r\n\
Call-ID: adapter-info\r\n\
CSeq: 1 INFO\r\n\
Content-Type: application/dtmf-relay\r\n\
Content-Length: 24\r\n\r\n\
Signal=5\r\nDuration=160\r\n";
        let request = match rvoip_sip_core::parse_message(raw).expect("parse INFO") {
            rvoip_sip_core::Message::Request(request) => request,
            other => panic!("expected INFO request, got {other:?}"),
        };
        let incoming = crate::api::incoming::IncomingRequest::from_bus_request(
            SessionId::new(),
            "sip:alice@example.test".into(),
            "sip:bob@example.test".into(),
            rvoip_sip_core::Method::Info,
            Arc::new(request),
        );
        assert_eq!(parse_sip_info_dtmf(&incoming), Some(("5".into(), 160)));
    }

    #[tokio::test]
    async fn sip_adapter_claims_the_single_response_capable_control_stream() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("adapter-control", 35998))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");

        assert!(
            coordinator.events_with_control().await.is_err(),
            "SipAdapter did not claim the exact-response control stream"
        );

        adapter.drain().await.expect("adapter drain");
        coordinator
            .shutdown_gracefully(Some(Duration::ZERO))
            .await
            .expect("coordinator shutdown");
    }

    #[async_trait::async_trait]
    impl AdapterLifecycleSink for RecordingLifecycleSink {
        async fn deliver_terminal(&self, _event: AdapterEvent) {
            self.deliveries.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct BlockingTerminalAppHandler {
        entered: Arc<AtomicBool>,
    }

    #[async_trait::async_trait]
    impl rvoip_infra_common::events::coordinator::CrossCrateEventHandler
        for BlockingTerminalAppHandler
    {
        async fn handle(
            &self,
            event: Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>,
        ) -> anyhow::Result<()> {
            let terminal = event
                .as_any()
                .downcast_ref::<crate::adapters::SessionApiCrossCrateEvent>()
                .is_some_and(|event| {
                    matches!(&event.event, crate::api::events::Event::CallEnded { .. })
                });
            if terminal {
                self.entered.store(true, Ordering::Release);
                std::future::pending::<()>().await;
            }
            Ok(())
        }
    }

    fn principal(tenant: &str) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: "sip-peer".to_string(),
            tenant: Some(tenant.to_string()),
            scopes: vec!["call:attach".to_string()],
            issuer: Some("sip-listener-test".to_string()),
            expires_at: None,
            method: AuthenticationMethod::Bearer,
            assurance: IdentityAssurance::Anonymous,
        }
    }

    fn request(route: &str, correlation_values: &[&str]) -> Arc<rvoip_sip_core::Request> {
        let mut headers = String::from(
            "Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-test\r\n\
             From: <sip:caller@example.test>;tag=from-tag\r\n\
             To: <sip:bridge@example.test>\r\n\
             Call-ID: inbound-context@example.test\r\n\
             CSeq: 1 INVITE\r\n\
             Max-Forwards: 70\r\n",
        );
        for value in correlation_values {
            headers.push_str(&format!("X-Correlation-Id: {value}\r\n"));
        }
        headers.push_str("X-Unlisted: must-not-escape\r\nContent-Length: 0\r\n\r\n");
        let wire = format!("INVITE sip:{route}@example.test SIP/2.0\r\n{headers}");
        match rvoip_sip_core::parse_message(wire.as_bytes()).expect("parse INVITE") {
            rvoip_sip_core::Message::Request(request) => Arc::new(request),
            _ => panic!("expected request"),
        }
    }

    fn observation(
        session_id: SessionId,
        route: &str,
        correlation_values: &[&str],
        principal: Option<AuthenticatedPrincipal>,
    ) -> InboundInviteObservation {
        InboundInviteObservation {
            session_id,
            request: Some(request(route, correlation_values)),
            principal,
        }
    }

    fn inbound_connection(connection_id: ConnectionId) -> Connection {
        Connection {
            id: connection_id,
            session_id: CoreSessionId::new(),
            participant_id: ParticipantId::new(),
            transport: Transport::Sip,
            direction: Direction::Inbound,
            state: ConnectionState::Connecting,
            capabilities: CapabilityDescriptor::default(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: Vec::new(),
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        }
    }

    async fn admit_test_session(
        coordinator: &UnifiedCoordinator,
        session_id: &SessionId,
    ) -> SessionRegistryHandle {
        let session = coordinator
            .helpers
            .state_machine
            .store
            .create_session(session_id.clone(), Role::UAS, true)
            .await
            .unwrap_or_else(|error| panic!("test session admission failed: {error}"));
        session.lifecycle_handle.expect("exact test session handle")
    }

    async fn retire_test_session(coordinator: &UnifiedCoordinator, handle: &SessionRegistryHandle) {
        let store = &coordinator.helpers.state_machine.store;
        let outcome = store
            .quiesce_session_exact(handle)
            .await
            .expect("exact test session quiesce");
        assert!(matches!(
            outcome,
            crate::session_lifecycle::TeardownOutcome::Retired { .. }
        ));
        coordinator
            .helpers
            .cleanup_session(handle.session_id())
            .await;
        store
            .remove_quiesced_session_exact(handle)
            .expect("exact test session removal");
    }

    async fn retire_test_session_if_current(
        coordinator: &UnifiedCoordinator,
        handle: &SessionRegistryHandle,
    ) {
        let current = coordinator
            .helpers
            .state_machine
            .store
            .lifecycle_handle(handle.session_id());
        if current.as_ref() == Some(handle) {
            retire_test_session(coordinator, handle).await;
        }
    }

    fn elapse_test_reuse_horizon(coordinator: &UnifiedCoordinator, session_id: &SessionId) {
        assert!(
            coordinator
                .helpers
                .state_machine
                .store
                .authority()
                .elapse_reuse_horizon_for_test(session_id),
            "exact retired identifier must own a reusable authority slot"
        );
    }

    async fn set_test_call_state(
        coordinator: &UnifiedCoordinator,
        session_id: &SessionId,
        call_state: CallState,
    ) {
        let mut state = coordinator
            .session_state(session_id)
            .await
            .expect("exact test session state");
        state.call_state = call_state;
        coordinator
            .update_session_state(state)
            .await
            .expect("exact test state update");
    }

    async fn assert_invalid_context_is_zero_wire(
        context: SipOriginateContext,
        expected: crate::SipOriginateContextError,
        adapter: &SipAdapter,
        coordinator: &UnifiedCoordinator,
        capture: &tokio::net::UdpSocket,
        case: &str,
    ) {
        assert_eq!(context.validate(), Err(expected), "{case}: validation");
        let target = capture.local_addr().expect("capture address");
        let request = OriginateRequest::new(
            CoreSessionId::new(),
            ParticipantId::new(),
            format!("sip:target@{target}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip)
        .with_context(context);
        assert!(matches!(
            ConnectionAdapter::originate(adapter, request).await,
            Err(RvoipError::AdmissionRejected(
                "outbound SIP originate context failed validation"
            ))
        ));
        assert!(adapter.by_connection.is_empty(), "{case}: connection map");
        assert!(adapter.by_session.is_empty(), "{case}: session map");
        assert!(adapter.outbound_routes.is_empty(), "{case}: route map");
        assert!(adapter.streams_cache.is_empty(), "{case}: stream cache");
        assert!(
            coordinator.list_sessions().await.is_empty(),
            "{case}: coordinator session"
        );
        let mut packet = [0u8; 2_048];
        assert!(
            tokio::time::timeout(Duration::from_millis(25), capture.recv_from(&mut packet))
                .await
                .is_err(),
            "{case}: validation failure emitted a SIP packet"
        );
    }

    #[test]
    fn empty_originate_context_uses_redacted_sip_defaults() {
        let request = OriginateRequest::new(
            CoreSessionId::new(),
            ParticipantId::new(),
            "sip:target@example.test",
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip);
        let context = SipAdapter::outbound_originate_context(&request).expect("SIP defaults");
        assert!(context.profile_revision().is_none());
        assert!(context.from_uri().is_none());
        assert!(context.outbound_proxy_uri().is_none());
        assert!(context.auth().is_none());
        assert!(context.initial_headers().is_empty());
        assert_eq!(
            format!("{context:?}"),
            "SipOriginateContext { has_profile_revision: false, has_from_uri: false, has_outbound_proxy: false, has_auth: false, initial_header_count: 0 }"
        );
    }

    #[test]
    fn forced_reclaim_follows_observed_release_not_only_finalizer_result() {
        assert!(!outbound_force_reclaim_required(true, true));
        assert!(outbound_force_reclaim_required(false, false));
        assert!(outbound_force_reclaim_required(false, true));
        assert!(
            outbound_force_reclaim_required(true, false),
            "a successful finalizer that retains authoritative state still requires reclaim"
        );
    }

    #[test]
    fn typed_originate_context_survives_the_admission_downcast_exactly() {
        let typed = SipOriginateContext::new()
            .with_from_uri("sips:private-caller@example.test")
            .expect("valid From URI")
            .with_auth(crate::auth::SipClientAuth::digest(
                "private-user",
                "private-password",
            ))
            .expect("bounded auth")
            .with_outbound_proxy("sips:private-proxy@example.test;lr")
            .expect("bounded outbound proxy")
            .with_initial_headers(
                crate::SipInitialHeaders::new([
                    ("X-App-Context", "first-secret"),
                    ("x-app-context", "second-secret"),
                ])
                .expect("validated headers"),
            );
        let opaque = rvoip_core::adapter::OriginateContext::new(typed);
        let expected = opaque
            .downcast_arc::<SipOriginateContext>()
            .expect("typed context before request construction");
        let request = OriginateRequest::new(
            CoreSessionId::new(),
            ParticipantId::new(),
            "sip:target@example.test",
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip)
        .with_originate_context(opaque);

        let admitted = SipAdapter::outbound_originate_context(&request).expect("typed admission");
        assert!(Arc::ptr_eq(&expected, &admitted));
        assert_eq!(
            admitted.from_uri(),
            Some("sips:private-caller@example.test")
        );
        assert_eq!(
            admitted.outbound_proxy_uri(),
            Some("sips:private-proxy@example.test;lr")
        );
        let Some(crate::auth::SipClientAuth::Digest(credentials)) = admitted.auth() else {
            panic!("expected retained Digest auth")
        };
        assert_eq!(credentials.username, "private-user");
        assert_eq!(credentials.password, "private-password");
        assert_eq!(
            admitted
                .initial_headers()
                .iter()
                .map(|(name, value)| (name.as_str(), value))
                .collect::<Vec<_>>(),
            vec![
                ("X-App-Context", "first-secret"),
                ("x-app-context", "second-secret"),
            ]
        );
        let debug = format!("{admitted:?}");
        for secret in [
            "private-caller",
            "private-user",
            "private-password",
            "private-proxy",
            "first-secret",
            "second-secret",
        ] {
            assert!(!debug.contains(secret));
        }
    }

    #[tokio::test]
    async fn every_originate_validation_failure_is_zero_wire_before_reservation() {
        use crate::originate::{
            MAX_SIP_INITIAL_HEADERS, MAX_SIP_INITIAL_HEADER_BYTES,
            MAX_SIP_INITIAL_HEADER_NAME_BYTES, MAX_SIP_INITIAL_HEADER_VALUE_BYTES,
            MAX_SIP_ORIGINATE_AUTH_OPTIONS, MAX_SIP_ORIGINATE_AUTH_PASSWORD_BYTES,
            MAX_SIP_ORIGINATE_AUTH_REALM_BYTES, MAX_SIP_ORIGINATE_AUTH_USERNAME_BYTES,
            MAX_SIP_ORIGINATE_BEARER_TOKEN_BYTES, MAX_SIP_ORIGINATE_FROM_URI_BYTES,
        };
        use crate::SipOriginateContextError;

        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture socket");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("validation-zero-wire", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");

        macro_rules! rejected {
            ($context:expr, $expected:expr, $case:literal) => {{
                assert_invalid_context_is_zero_wire(
                    $context,
                    $expected,
                    adapter.as_ref(),
                    coordinator.as_ref(),
                    &capture,
                    $case,
                )
                .await;
            }};
        }

        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                None,
                (0..=MAX_SIP_INITIAL_HEADERS)
                    .map(|index| (format!("X-{index}"), "v".to_string()))
                    .collect()
            ),
            SipOriginateContextError::InvalidInitialHeaders,
            "header count"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                None,
                vec![(String::new(), "v".to_string())]
            ),
            SipOriginateContextError::InvalidInitialHeaders,
            "empty header name"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                None,
                vec![(
                    format!("X{}", "n".repeat(MAX_SIP_INITIAL_HEADER_NAME_BYTES)),
                    "v".to_string()
                )]
            ),
            SipOriginateContextError::InvalidInitialHeaders,
            "header name bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                None,
                vec![("Bad Header".to_string(), "v".to_string())]
            ),
            SipOriginateContextError::InvalidInitialHeaders,
            "invalid header token"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                None,
                vec![("Via".to_string(), "secret".to_string())]
            ),
            SipOriginateContextError::InvalidInitialHeaders,
            "stack-owned header"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                None,
                vec![(
                    "X-Large".to_string(),
                    "v".repeat(MAX_SIP_INITIAL_HEADER_VALUE_BYTES + 1)
                )]
            ),
            SipOriginateContextError::InvalidInitialHeaders,
            "header value bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                None,
                vec![(
                    "X-Control".to_string(),
                    "value\r\nX-Injected: yes".to_string()
                )]
            ),
            SipOriginateContextError::InvalidInitialHeaders,
            "header control"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                None,
                (0..5)
                    .map(|index| (
                        format!("X-Aggregate-{index}"),
                        "v".repeat(MAX_SIP_INITIAL_HEADER_BYTES / 4)
                    ))
                    .collect()
            ),
            SipOriginateContextError::InvalidInitialHeaders,
            "header aggregate"
        );

        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                Some("https://private.invalid/call".to_string()),
                None,
                Vec::new()
            ),
            SipOriginateContextError::InvalidFromUri,
            "invalid From URI"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                Some("sip:caller@example.test\r\n".to_string()),
                None,
                Vec::new()
            ),
            SipOriginateContextError::InvalidFromUri,
            "From URI control"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                Some(format!(
                    "sip:{}@example.test",
                    "u".repeat(MAX_SIP_ORIGINATE_FROM_URI_BYTES)
                )),
                None,
                Vec::new()
            ),
            SipOriginateContextError::FromUriTooLarge,
            "From URI bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::bearer_token(
                    "token\r\nX-Injected: yes"
                )),
                Vec::new()
            ),
            SipOriginateContextError::InvalidAuthMaterial,
            "auth control"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::digest("", "password")),
                Vec::new()
            ),
            SipOriginateContextError::InvalidAuthMaterial,
            "empty Digest username"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::digest("username", "")),
                Vec::new()
            ),
            SipOriginateContextError::InvalidAuthMaterial,
            "empty Digest password"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::Digest(
                    crate::types::Credentials::new("username", "password").with_realm("")
                )),
                Vec::new()
            ),
            SipOriginateContextError::InvalidAuthMaterial,
            "empty Digest realm"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::Digest(
                    crate::types::Credentials::new("username", "password")
                        .with_realm("realm\ncontrol")
                )),
                Vec::new()
            ),
            SipOriginateContextError::InvalidAuthMaterial,
            "Digest realm control"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::basic("user:name", "password")),
                Vec::new()
            ),
            SipOriginateContextError::InvalidAuthMaterial,
            "Basic username colon"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::basic(
                    "username",
                    "password\0control"
                )),
                Vec::new()
            ),
            SipOriginateContextError::InvalidAuthMaterial,
            "Basic password control"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::bearer_token("")),
                Vec::new()
            ),
            SipOriginateContextError::InvalidAuthMaterial,
            "empty Bearer token"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::any(Vec::<
                    crate::auth::SipClientAuth,
                >::new())),
                Vec::new()
            ),
            SipOriginateContextError::InvalidAuthMaterial,
            "empty auth alternatives"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::digest(
                    "u".repeat(MAX_SIP_ORIGINATE_AUTH_USERNAME_BYTES + 1),
                    "password"
                )),
                Vec::new()
            ),
            SipOriginateContextError::AuthUsernameTooLarge,
            "auth username bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::basic(
                    "u".repeat(MAX_SIP_ORIGINATE_AUTH_USERNAME_BYTES + 1),
                    "password"
                )),
                Vec::new()
            ),
            SipOriginateContextError::AuthUsernameTooLarge,
            "Basic username bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::basic(
                    "username",
                    "p".repeat(MAX_SIP_ORIGINATE_AUTH_PASSWORD_BYTES + 1)
                )),
                Vec::new()
            ),
            SipOriginateContextError::AuthPasswordTooLarge,
            "auth password bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::digest(
                    "username",
                    "p".repeat(MAX_SIP_ORIGINATE_AUTH_PASSWORD_BYTES + 1)
                )),
                Vec::new()
            ),
            SipOriginateContextError::AuthPasswordTooLarge,
            "Digest password bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::Digest(
                    crate::types::Credentials::new("username", "password")
                        .with_realm("r".repeat(MAX_SIP_ORIGINATE_AUTH_REALM_BYTES + 1))
                )),
                Vec::new()
            ),
            SipOriginateContextError::AuthRealmTooLarge,
            "auth realm bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::bearer_token(
                    "t".repeat(MAX_SIP_ORIGINATE_BEARER_TOKEN_BYTES + 1)
                )),
                Vec::new()
            ),
            SipOriginateContextError::BearerTokenTooLarge,
            "bearer bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::BearerTokenCleartextAllowed(
                    "t".repeat(MAX_SIP_ORIGINATE_BEARER_TOKEN_BYTES + 1),
                )),
                Vec::new()
            ),
            SipOriginateContextError::BearerTokenTooLarge,
            "cleartext Bearer bound"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::any(
                    (0..=MAX_SIP_ORIGINATE_AUTH_OPTIONS).map(|index| {
                        crate::auth::SipClientAuth::bearer_token(format!("token-{index}"))
                    })
                )),
                Vec::new()
            ),
            SipOriginateContextError::TooManyAuthOptions,
            "auth option count"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::any([
                    crate::auth::SipClientAuth::any([crate::auth::SipClientAuth::bearer_token(
                        "nested"
                    )])
                ])),
                Vec::new()
            ),
            SipOriginateContextError::NestedAuthOptions,
            "nested auth"
        );
        rejected!(
            SipOriginateContext::unvalidated_for_adapter_test(
                None,
                Some(crate::auth::SipClientAuth::any((0..5).map(|index| {
                    crate::auth::SipClientAuth::bearer_token(format!(
                        "{index}{}",
                        "t".repeat(MAX_SIP_ORIGINATE_BEARER_TOKEN_BYTES - 1)
                    ))
                }))),
                Vec::new()
            ),
            SipOriginateContextError::AuthAggregateTooLarge,
            "auth aggregate"
        );

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn wrong_nonempty_originate_context_fails_before_any_sip_side_effect() {
        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture socket");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("wrong-context", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let retained_tasks_before_prepare = adapter.retained_task_count();
        let request = OriginateRequest::new(
            CoreSessionId::new(),
            ParticipantId::new(),
            format!("sip:target@{target}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip)
        .with_context(String::from("wrong-context-secret"));

        assert!(matches!(
            ConnectionAdapter::originate(adapter.as_ref(), request).await,
            Err(RvoipError::AdmissionRejected(
                "outbound SIP originate context type mismatch"
            ))
        ));
        assert!(adapter.by_connection.is_empty());
        assert!(adapter.by_session.is_empty());
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());
        assert_eq!(
            adapter.retained_task_count(),
            retained_tasks_before_prepare,
            "rejected context must not spawn a per-route task or timer"
        );

        let mut packet = [0u8; 2_048];
        assert!(
            tokio::time::timeout(Duration::from_millis(100), capture.recv_from(&mut packet))
                .await
                .is_err(),
            "wrong context must not emit a SIP packet"
        );

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn prepared_outbound_route_and_pre_send_end_are_zero_wire() {
        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture socket");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("zero-wire-prepare", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let retained_tasks_before_prepare = adapter.retained_task_count();
        let request = OriginateRequest::new(
            CoreSessionId::new(),
            ParticipantId::new(),
            format!("sip:target@{target}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip);

        let prepared = ConnectionAdapter::originate(adapter.as_ref(), request)
            .await
            .expect("local preparation");
        let connection_id = prepared.connection.id.clone();
        let route = adapter
            .outbound_routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .expect("retained route");
        assert_eq!(
            route.stream.subscribe_lifecycle().borrow().to_owned(),
            crate::media_stream::SipMediaLifecycle::Dormant
        );
        assert!(coordinator.list_sessions().await.is_empty());
        assert_eq!(
            adapter.retained_task_count(),
            retained_tasks_before_prepare,
            "preparation must not spawn a per-route task or timer"
        );

        let mut packet = [0u8; 2_048];
        assert!(
            tokio::time::timeout(Duration::from_millis(100), capture.recv_from(&mut packet))
                .await
                .is_err(),
            "preparation must not emit a SIP packet"
        );

        ConnectionAdapter::end(adapter.as_ref(), connection_id, EndReason::Cancelled)
            .await
            .expect("pre-send local cleanup");
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());
        assert_eq!(
            adapter.retained_task_count(),
            retained_tasks_before_prepare,
            "zero-wire cleanup must not leave a retained route task"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), capture.recv_from(&mut packet))
                .await
                .is_err(),
            "pre-send cleanup must not emit CANCEL, BYE, or INVITE"
        );

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn prepared_outbound_non_terminal_controls_and_media_are_typed_zero_wire_failures() {
        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture socket");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("prepared-controls", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let prepared = ConnectionAdapter::originate(
            adapter.as_ref(),
            OriginateRequest::new(
                CoreSessionId::new(),
                ParticipantId::new(),
                format!("sip:target@{target}"),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip),
        )
        .await
        .expect("prepared route");
        let connection_id = prepared.connection.id.clone();
        let stream = adapter
            .streams_cache
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .expect("dormant media stream");

        for result in [
            ConnectionAdapter::hold(adapter.as_ref(), connection_id.clone()).await,
            ConnectionAdapter::resume(adapter.as_ref(), connection_id.clone()).await,
            ConnectionAdapter::transfer(
                adapter.as_ref(),
                connection_id.clone(),
                TransferTarget::Uri("sip:other@example.test".to_string()),
            )
            .await,
            ConnectionAdapter::send_dtmf(adapter.as_ref(), connection_id.clone(), "5", 100).await,
        ] {
            assert!(matches!(
                result,
                Err(RvoipError::InvalidState(
                    "SIP outbound route is not activated"
                ))
            ));
        }
        for (digits, duration_ms) in [("5X", 100), ("5", 39), ("5", 6_001), ("", 100)] {
            assert!(matches!(
                ConnectionAdapter::send_dtmf(
                    adapter.as_ref(),
                    connection_id.clone(),
                    digits,
                    duration_ms,
                )
                .await,
                Err(RvoipError::AdmissionRejected(_))
            ));
        }
        assert!(matches!(
            stream.try_frames_out(),
            Err(RvoipError::InvalidState(
                "SIP media stream is not activated"
            ))
        ));
        assert!(stream.frames_out().is_closed());
        assert!(coordinator.list_sessions().await.is_empty());

        let mut packet = [0u8; 2_048];
        assert!(
            tokio::time::timeout(Duration::from_millis(100), capture.recv_from(&mut packet))
                .await
                .is_err(),
            "pre-activation controls and media must not emit SIP/RTP"
        );

        ConnectionAdapter::end(adapter.as_ref(), connection_id, EndReason::Cancelled)
            .await
            .expect("end remains allowed before activation");
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), capture.recv_from(&mut packet))
                .await
                .is_err(),
            "pre-send end remains zero wire"
        );

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn explicit_drain_is_phase_aware_and_joins_every_retained_task() {
        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture socket");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("adapter-drain", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        assert_eq!(coordinator.inbound_invite_observer_count(), 1);

        let request = OriginateRequest::new(
            CoreSessionId::new(),
            ParticipantId::new(),
            format!("sip:target@{target}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip);
        ConnectionAdapter::originate(adapter.as_ref(), request)
            .await
            .expect("prepared outbound route");

        let inbound_session = SessionId::new();
        coordinator
            .helpers
            .create_session(
                inbound_session.clone(),
                "sip:bridge@example.test".into(),
                "sip:caller@example.test".into(),
                Role::UAS,
            )
            .await
            .expect("inbound session");
        let mut inbound_state = coordinator
            .session_state(&inbound_session)
            .await
            .expect("inbound state");
        inbound_state.call_state = CallState::Active;
        coordinator
            .update_session_state(inbound_state)
            .await
            .expect("active inbound state");
        adapter
            .ensure_mapped_epoch(inbound_session)
            .expect("inbound mapping");

        adapter.drain().await.expect("explicit adapter drain");
        assert!(adapter.is_draining());
        assert_eq!(adapter.retained_task_count(), 0);
        assert_eq!(coordinator.inbound_invite_observer_count(), 0);
        assert!(adapter.by_connection.is_empty());
        assert!(adapter.by_session.is_empty());
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());
        assert!(adapter.drain().await.is_ok(), "drain is idempotent");

        let mut packet = [0u8; 2_048];
        assert!(
            tokio::time::timeout(Duration::from_millis(100), capture.recv_from(&mut packet))
                .await
                .is_err(),
            "a prepared outbound route remains zero-wire during drain"
        );
        assert!(matches!(
            ConnectionAdapter::originate(
                adapter.as_ref(),
                OriginateRequest::new(
                    CoreSessionId::new(),
                    ParticipantId::new(),
                    format!("sip:target@{target}"),
                    Direction::Outbound,
                    CapabilityDescriptor::default(),
                )
                .with_transport(Transport::Sip),
            )
            .await,
            Err(RvoipError::AdmissionRejected(_))
        ));
        adapter.shutdown().await.expect("coordinator shutdown");
    }

    #[tokio::test]
    async fn aborting_drain_waiter_does_not_cancel_destructive_cleanup() {
        let coordinator =
            UnifiedCoordinator::new(ApiConfig::local("adapter-drain-cancelled-waiter", 0))
                .await
                .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let (blocker_started_tx, blocker_started_rx) = tokio::sync::oneshot::channel();
        let (release_blocker_tx, release_blocker_rx) = tokio::sync::oneshot::channel();
        assert!(adapter.retained_tasks.spawn(async move {
            let _ = blocker_started_tx.send(());
            let _ = release_blocker_rx.await;
        }));
        blocker_started_rx.await.expect("retained blocker started");

        let drain_adapter = Arc::clone(&adapter);
        let waiter = tokio::spawn(async move { drain_adapter.drain().await });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !adapter.is_draining() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("destructive drain started");
        assert!(!adapter.drained.load(Ordering::Acquire));
        waiter.abort();
        assert!(waiter.await.is_err(), "public drain waiter was cancelled");

        let _ = release_blocker_tx.send(());
        tokio::time::timeout(Duration::from_secs(2), async {
            while !adapter.drained.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached drain driver completed cleanup");
        assert_eq!(adapter.retained_task_count(), 0);
        assert_eq!(coordinator.inbound_invite_observer_count(), 0);
        assert!(adapter.drain().await.is_ok(), "later drain joins success");
        adapter.shutdown().await.expect("coordinator shutdown");
    }

    #[tokio::test]
    async fn failed_destructive_drain_remains_sticky_on_retry() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("adapter-drain-sticky", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");

        let request = OriginateRequest::new(
            CoreSessionId::new(),
            ParticipantId::new(),
            "sip:target@example.test",
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip);
        ConnectionAdapter::originate(adapter.as_ref(), request)
            .await
            .expect("prepared route");
        // Force the result boundary after real zero-wire cleanup has retired
        // every route. A later call must not reinterpret those empty maps as
        // proof that the failed compensation succeeded.
        adapter
            .force_drain_compensation_failure
            .store(true, Ordering::Release);
        let first = adapter.drain().await.expect_err("first drain must fail");
        let second = adapter
            .drain()
            .await
            .expect_err("destructive drain failure must remain sticky");
        assert_eq!(first.to_string(), second.to_string());
        assert!(!adapter.drained.load(Ordering::Acquire));
        assert!(adapter.by_connection.is_empty());
        assert!(adapter.by_session.is_empty());
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert_eq!(adapter.retained_task_count(), 0);

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("coordinator shutdown");
    }

    #[tokio::test]
    async fn concurrent_activation_sends_one_invite_and_receipt_matches_wire_call_id() {
        use rvoip_sip_core::{parse_message, Message as SipMessage, Method, StatusCode};
        use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture socket");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("activation-singleflight", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let request = OriginateRequest::new(
            CoreSessionId::new(),
            ParticipantId::new(),
            format!("sip:target@{target}"),
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::Sip);
        let prepared = ConnectionAdapter::originate(adapter.as_ref(), request)
            .await
            .expect("prepare route");
        let connection_id = prepared.connection.id.clone();

        let mut callers = Vec::new();
        for _ in 0..100 {
            let caller_adapter = Arc::clone(&adapter);
            let caller_connection = connection_id.clone();
            callers.push(tokio::spawn(async move {
                ConnectionAdapter::activate_outbound_with_receipt(
                    caller_adapter.as_ref(),
                    caller_connection,
                )
                .await
            }));
        }

        let mut packet = [0u8; 8_192];
        let (bytes, uac) =
            tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                .await
                .expect("INVITE deadline")
                .expect("INVITE packet");
        let SipMessage::Request(invite) = parse_message(&packet[..bytes]).expect("parse INVITE")
        else {
            panic!("expected INVITE request")
        };
        assert_eq!(invite.method(), Method::Invite);
        let wire_call_id = invite.call_id().expect("wire Call-ID").value();

        let mut accepted = create_response(&invite, StatusCode::Ok);
        if let Some(TypedHeader::To(to)) = accepted
            .headers
            .iter_mut()
            .find(|header| matches!(header, TypedHeader::To(_)))
        {
            to.set_tag("activation-singleflight-uas");
        }
        accepted.headers.push(TypedHeader::Other(
            HeaderName::Contact,
            HeaderValue::Raw(format!("<sip:target@{target}>").into_bytes()),
        ));
        accepted.headers.push(TypedHeader::ContentType(
            rvoip_sip_core::types::ContentType::sdp(),
        ));
        accepted.body = bytes::Bytes::from(format!(
            "v=0\r\no=capture 1 1 IN IP4 127.0.0.1\r\ns=activation\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {} RTP/AVP 8 101\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=sendrecv\r\n",
            target.port()
        ));
        accepted
            .headers
            .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
        accepted.headers.push(TypedHeader::ContentLength(
            rvoip_sip_core::types::ContentLength::new(accepted.body.len() as u32),
        ));
        capture
            .send_to(&SipMessage::Response(accepted).to_bytes(), uac)
            .await
            .expect("accept INVITE");

        let mut expected_receipt = None;
        for caller in callers {
            let receipt = tokio::time::timeout(Duration::from_secs(5), caller)
                .await
                .expect("activation deadline")
                .expect("activation task")
                .expect("activation receipt");
            let external = receipt
                .external_references()
                .iter()
                .find(|reference| reference.kind() == "sip.call-id")
                .expect("SIP Call-ID receipt")
                .expose_secret()
                .to_string();
            assert_eq!(external, wire_call_id);
            if let Some(expected) = expected_receipt.as_ref() {
                assert_eq!(expected, &external);
            } else {
                expected_receipt = Some(external);
            }
        }
        let no_second_invite_deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        loop {
            match tokio::time::timeout_at(no_second_invite_deadline, capture.recv_from(&mut packet))
                .await
            {
                Err(_) => break,
                Ok(Ok((bytes, _))) => {
                    if let Ok(SipMessage::Request(request)) = parse_message(&packet[..bytes]) {
                        assert_ne!(
                            request.method(),
                            Method::Invite,
                            "concurrent activation must not emit a second INVITE"
                        );
                    }
                }
                Ok(Err(error)) => panic!("capture failed: {error}"),
            }
        }

        let end_adapter = Arc::clone(&adapter);
        let end_connection = connection_id;
        let end = tokio::spawn(async move {
            ConnectionAdapter::end(end_adapter.as_ref(), end_connection, EndReason::Cancelled).await
        });
        loop {
            let (bytes, peer) =
                tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                    .await
                    .expect("BYE deadline")
                    .expect("dialog datagram");
            let Ok(SipMessage::Request(request)) = parse_message(&packet[..bytes]) else {
                continue;
            };
            match request.method() {
                Method::Ack => {}
                Method::Bye => {
                    let ok = create_response(&request, StatusCode::Ok);
                    capture
                        .send_to(&SipMessage::Response(ok).to_bytes(), peer)
                        .await
                        .expect("acknowledge BYE");
                    break;
                }
                Method::Invite => panic!("concurrent activation emitted a second INVITE"),
                method => panic!("unexpected dialog request: {method}"),
            }
        }
        tokio::time::timeout(Duration::from_secs(5), end)
            .await
            .expect("bounded cleanup deadline")
            .expect("end task")
            .expect("retained cleanup");
        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn activation_receipt_follows_answer_then_media_binds_and_cleans_up() {
        use rvoip_sip_core::{parse_message, Message as SipMessage, Method, StatusCode};
        use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture UAS");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("deferred-media-bind", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let prepared = ConnectionAdapter::originate(
            adapter.as_ref(),
            OriginateRequest::new(
                CoreSessionId::new(),
                ParticipantId::new(),
                format!("sip:target@{target}"),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip),
        )
        .await
        .expect("prepared route");
        let connection_id = prepared.connection.id.clone();
        let route = adapter
            .outbound_routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .expect("retained route");

        let activation_adapter = Arc::clone(&adapter);
        let activation_connection = connection_id.clone();
        let activation = tokio::spawn(async move {
            ConnectionAdapter::activate_outbound_with_receipt(
                activation_adapter.as_ref(),
                activation_connection,
            )
            .await
        });

        let mut packet = [0u8; 16_384];
        let (invite_bytes, uac) =
            tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                .await
                .expect("INVITE deadline")
                .expect("INVITE datagram");
        let SipMessage::Request(invite) =
            parse_message(&packet[..invite_bytes]).expect("parse INVITE")
        else {
            panic!("expected INVITE request")
        };
        assert_eq!(invite.method(), Method::Invite);
        let invite_call_id = invite.call_id().expect("INVITE Call-ID").value();

        let mut media = route.stream.subscribe_lifecycle();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if *media.borrow_and_update() == crate::media_stream::SipMediaLifecycle::Binding {
                    break;
                }
                media.changed().await.expect("media lifecycle remains live");
            }
        })
        .await
        .expect("media binding begins before the answer");
        assert!(
            !activation.is_finished(),
            "activation receipt must wait for a final answer"
        );
        assert!(matches!(
            route.stream.try_frames_out(),
            Err(RvoipError::InvalidState(
                "SIP media stream is not activated"
            ))
        ));

        let mut accepted = create_response(&invite, StatusCode::Ok);
        if let Some(TypedHeader::To(to)) = accepted
            .headers
            .iter_mut()
            .find(|header| matches!(header, TypedHeader::To(_)))
        {
            to.set_tag("deferred-media-capture-uas");
        }
        accepted.headers.push(TypedHeader::Other(
            HeaderName::Contact,
            HeaderValue::Raw(format!("<sip:target@{target}>").into_bytes()),
        ));
        accepted.headers.push(TypedHeader::ContentType(
            rvoip_sip_core::types::ContentType::sdp(),
        ));
        accepted.body = bytes::Bytes::from(format!(
            "v=0\r\no=capture 1 1 IN IP4 127.0.0.1\r\ns=deferred-media\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio {} RTP/AVP 8 101\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-15\r\na=sendrecv\r\n",
            target.port()
        ));
        accepted
            .headers
            .retain(|header| !matches!(header, TypedHeader::ContentLength(_)));
        accepted.headers.push(TypedHeader::ContentLength(
            rvoip_sip_core::types::ContentLength::new(accepted.body.len() as u32),
        ));
        capture
            .send_to(&SipMessage::Response(accepted).to_bytes(), uac)
            .await
            .expect("accept INVITE");

        let receipt = tokio::time::timeout(Duration::from_secs(5), activation)
            .await
            .expect("activation deadline")
            .expect("activation task")
            .expect("activation receipt");
        assert_eq!(
            receipt
                .external_references()
                .iter()
                .find(|reference| reference.kind() == "sip.call-id")
                .expect("SIP Call-ID receipt")
                .expose_secret(),
            invite_call_id
        );

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match *media.borrow_and_update() {
                    crate::media_stream::SipMediaLifecycle::Bound => break,
                    state @ (crate::media_stream::SipMediaLifecycle::Failed
                    | crate::media_stream::SipMediaLifecycle::Closing
                    | crate::media_stream::SipMediaLifecycle::Closed) => {
                        panic!("media bind terminated before negotiation completed: {state:?}")
                    }
                    crate::media_stream::SipMediaLifecycle::Dormant
                    | crate::media_stream::SipMediaLifecycle::Binding => {}
                }
                media.changed().await.expect("media lifecycle remains live");
            }
        })
        .await
        .expect("media bind deadline");
        assert_eq!(route.stream.codec().name, "g.711-a");
        assert!(route.stream.try_frames_out().is_ok());

        let end_adapter = Arc::clone(&adapter);
        let end_connection = connection_id.clone();
        let end = tokio::spawn(async move {
            ConnectionAdapter::end(end_adapter.as_ref(), end_connection, EndReason::Normal).await
        });
        loop {
            let (bytes, peer) =
                tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                    .await
                    .expect("dialog teardown deadline")
                    .expect("dialog datagram");
            let SipMessage::Request(request) =
                parse_message(&packet[..bytes]).expect("parse dialog request")
            else {
                continue;
            };
            match request.method() {
                Method::Ack => {}
                Method::Bye => {
                    let ok = create_response(&request, StatusCode::Ok);
                    capture
                        .send_to(&SipMessage::Response(ok).to_bytes(), peer)
                        .await
                        .expect("acknowledge BYE");
                    break;
                }
                method => panic!("unexpected dialog request: {method}"),
            }
        }
        tokio::time::timeout(Duration::from_secs(5), end)
            .await
            .expect("bounded cleanup deadline")
            .expect("end task")
            .expect("cleanup");
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert!(adapter.by_connection.is_empty());
        assert!(adapter.by_session.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());
        adapter.drain().await.expect("adapter drain");
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn post_send_activation_cancel_emits_one_legal_cancel_then_forces_local_reclaim() {
        use rvoip_sip_core::types::headers::HeaderAccess;
        use rvoip_sip_core::{parse_message, Message as SipMessage, Method, StatusCode};
        use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture UAS");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("cancel-compensation", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic event receiver");
        for _ in 0..SIP_ADAPTER_EVENT_CAPACITY {
            adapter
                .out_tx
                .try_send(OrchestratorAdapterEvent::Public(AdapterEvent::Native {
                    kind: "cancel-test-filler",
                    detail: String::new(),
                }))
                .expect("fill operational event queue");
        }

        let prepared = ConnectionAdapter::originate(
            adapter.as_ref(),
            OriginateRequest::new(
                CoreSessionId::new(),
                ParticipantId::new(),
                format!("sip:target@{target}"),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip),
        )
        .await
        .expect("prepared route");
        let connection_id = prepared.connection.id.clone();
        let route = adapter
            .outbound_routes
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .expect("retained route");
        assert_eq!(
            route.stage_event(AdapterEvent::Connected {
                connection_id: connection_id.clone(),
            }),
            SipRouteStageDisposition::Retained
        );

        let activation_adapter = Arc::clone(&adapter);
        let activation_connection = connection_id.clone();
        let activation = tokio::spawn(async move {
            ConnectionAdapter::activate_outbound_with_receipt(
                activation_adapter.as_ref(),
                activation_connection,
            )
            .await
        });

        let mut packet = [0u8; 16_384];
        let (invite_bytes, uac) =
            tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                .await
                .expect("INVITE deadline")
                .expect("INVITE datagram");
        let SipMessage::Request(invite) =
            parse_message(&packet[..invite_bytes]).expect("parse INVITE")
        else {
            panic!("expected INVITE request")
        };
        assert_eq!(invite.method(), Method::Invite);
        let invite_call_id = invite.call_id().expect("INVITE Call-ID").value();
        let invite_cseq = invite.cseq().expect("INVITE CSeq").sequence();
        let invite_uri = invite.uri().to_string();
        let invite_from = invite
            .raw_header_value(&HeaderName::From)
            .expect("INVITE From");
        let invite_to = invite.raw_header_value(&HeaderName::To).expect("INVITE To");
        let invite_via = invite
            .raw_header_value(&HeaderName::Via)
            .expect("INVITE Via");

        let ringing = create_response(&invite, StatusCode::Ringing);
        capture
            .send_to(&SipMessage::Response(ringing).to_bytes(), uac)
            .await
            .expect("send provisional response");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let (phase, wire) = {
                    let state = route
                        .state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    (state.phase, state.wire)
                };
                let ringing = matches!(
                    coordinator.get_state(&route.session_id).await,
                    Ok(CallState::Ringing | CallState::EarlyMedia)
                );
                if phase == SipOutboundRoutePhase::Flushing
                    && wire == SipOutboundWireState::Sent
                    && ringing
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("activation reached post-send flushing phase");

        let blocked_terminal_publish = Arc::new(AtomicBool::new(false));
        coordinator
            .global_coordinator
            .register_handler(
                crate::adapters::SESSION_TO_APP_CHANNEL,
                BlockingTerminalAppHandler {
                    entered: Arc::clone(&blocked_terminal_publish),
                },
            )
            .await
            .expect("install blocked terminal app publisher");

        let end_adapter = Arc::clone(&adapter);
        let end_connection = connection_id.clone();
        let end = tokio::spawn(async move {
            ConnectionAdapter::end(end_adapter.as_ref(), end_connection, EndReason::Cancelled).await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while !*route.cancel.borrow() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("route cancellation became visible");
        for _ in 0..SIP_ADAPTER_EVENT_CAPACITY {
            assert!(matches!(
                events.recv().await,
                Some(OrchestratorAdapterEvent::Public(AdapterEvent::Native {
                    kind: "cancel-test-filler",
                    ..
                }))
            ));
        }

        let cancel = loop {
            let (bytes, peer) =
                tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                    .await
                    .expect("compensation deadline")
                    .expect("compensation datagram");
            let SipMessage::Request(request) =
                parse_message(&packet[..bytes]).expect("parse compensation request")
            else {
                continue;
            };
            if request.method() == Method::Cancel {
                let ok = create_response(&request, StatusCode::Ok);
                capture
                    .send_to(&SipMessage::Response(ok).to_bytes(), peer)
                    .await
                    .expect("acknowledge CANCEL transaction");
                break request;
            }
            assert_ne!(
                request.method(),
                Method::Bye,
                "a provisional INVITE must be compensated with CANCEL, not BYE"
            );
        };
        assert_eq!(
            cancel.call_id().expect("CANCEL Call-ID").value(),
            invite_call_id
        );
        assert_eq!(cancel.cseq().expect("CANCEL CSeq").sequence(), invite_cseq);
        assert_eq!(
            *cancel.cseq().expect("CANCEL CSeq").method(),
            Method::Cancel
        );
        assert_eq!(cancel.uri().to_string(), invite_uri);
        assert_eq!(
            cancel.raw_header_value(&HeaderName::From).as_deref(),
            Some(invite_from.as_str())
        );
        assert_eq!(
            cancel.raw_header_value(&HeaderName::To).as_deref(),
            Some(invite_to.as_str())
        );
        assert_eq!(
            cancel.raw_header_value(&HeaderName::Via).as_deref(),
            Some(invite_via.as_str()),
            "CANCEL must reuse the INVITE client transaction branch"
        );

        assert!(
            activation.await.expect("activation task").is_err(),
            "post-send cancellation withholds the activation receipt"
        );
        tokio::time::timeout(Duration::from_secs(8), end)
            .await
            .expect("bounded forced cleanup deadline")
            .expect("end task")
            .expect("forced local cleanup");
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert!(adapter.by_connection.is_empty());
        assert!(adapter.by_session.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());
        assert!(
            blocked_terminal_publish.load(Ordering::Acquire),
            "forced reclaim must be exercised after terminal publication blocks"
        );

        let duplicate_teardown = tokio::time::timeout(Duration::from_millis(1_100), async {
            loop {
                let (bytes, _) = capture.recv_from(&mut packet).await.expect("capture");
                if let Ok(SipMessage::Request(request)) = parse_message(&packet[..bytes]) {
                    if matches!(request.method(), Method::Cancel | Method::Bye) {
                        return request.method().clone();
                    }
                }
            }
        })
        .await;
        assert!(
            duplicate_teardown.is_err(),
            "compensation emitted a duplicate CANCEL/BYE beyond SIP T1"
        );

        adapter.drain().await.expect("adapter drain");
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn active_outbound_end_emits_one_legal_bye_and_reclaims_route() {
        use rvoip_sip_core::types::headers::HeaderAccess;
        use rvoip_sip_core::{parse_message, Message as SipMessage, Method, StatusCode};
        use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture UAS");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("bye-compensation", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let prepared = ConnectionAdapter::originate(
            adapter.as_ref(),
            OriginateRequest::new(
                CoreSessionId::new(),
                ParticipantId::new(),
                format!("sip:target@{target}"),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip),
        )
        .await
        .expect("prepared route");
        let connection_id = prepared.connection.id.clone();
        let outbound_session_id = adapter
            .outbound_routes
            .get(&connection_id)
            .expect("prepared outbound route")
            .session_id
            .clone();
        let activation_adapter = Arc::clone(&adapter);
        let activation_connection = connection_id.clone();
        let activation = tokio::spawn(async move {
            ConnectionAdapter::activate_outbound_with_receipt(
                activation_adapter.as_ref(),
                activation_connection,
            )
            .await
        });

        let mut packet = [0u8; 16_384];
        let (invite_bytes, uac) =
            tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                .await
                .expect("INVITE deadline")
                .expect("INVITE datagram");
        let SipMessage::Request(invite) =
            parse_message(&packet[..invite_bytes]).expect("parse INVITE")
        else {
            panic!("expected INVITE request")
        };
        assert_eq!(invite.method(), Method::Invite);
        let invite_call_id = invite.call_id().expect("INVITE Call-ID").value();
        let invite_cseq = invite.cseq().expect("INVITE CSeq").sequence();
        let invite_from = invite
            .raw_header_value(&HeaderName::From)
            .expect("INVITE From");
        let contact_uri = format!("sip:target@{target}");

        let mut accepted = create_response(&invite, StatusCode::Ok);
        if let Some(TypedHeader::To(to)) = accepted
            .headers
            .iter_mut()
            .find(|header| matches!(header, TypedHeader::To(_)))
        {
            to.set_tag("adapter-bye-capture-uas");
        }
        accepted.headers.push(TypedHeader::Other(
            HeaderName::Contact,
            HeaderValue::Raw(format!("<{contact_uri}>").into_bytes()),
        ));
        let accepted_to = accepted
            .raw_header_value(&HeaderName::To)
            .expect("accepted To");
        capture
            .send_to(&SipMessage::Response(accepted).to_bytes(), uac)
            .await
            .expect("accept INVITE");
        tokio::time::timeout(Duration::from_secs(5), activation)
            .await
            .expect("activation deadline")
            .expect("activation task")
            .expect("activation receipt");
        assert_eq!(
            coordinator
                .get_state(&outbound_session_id)
                .await
                .expect("activated SIP session state"),
            CallState::Active,
            "a successful SIP activation receipt must linearize after the exact session is Active"
        );

        let end_adapter = Arc::clone(&adapter);
        let end_connection = connection_id.clone();
        let end = tokio::spawn(async move {
            ConnectionAdapter::end(end_adapter.as_ref(), end_connection, EndReason::Normal).await
        });
        let (bye, bye_peer) = loop {
            let (bytes, peer) =
                tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                    .await
                    .expect("BYE deadline")
                    .expect("dialog datagram");
            let SipMessage::Request(request) =
                parse_message(&packet[..bytes]).expect("parse dialog request")
            else {
                continue;
            };
            match request.method() {
                Method::Ack => continue,
                Method::Bye => break (request, peer),
                Method::Cancel => panic!("an established dialog must use BYE, not CANCEL"),
                method => panic!("unexpected dialog request: {method}"),
            }
        };

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !end.is_finished(),
            "adapter end acknowledged established SIP teardown before the peer's final BYE response"
        );
        let ok = create_response(&bye, StatusCode::Ok);
        capture
            .send_to(&SipMessage::Response(ok).to_bytes(), bye_peer)
            .await
            .expect("acknowledge BYE transaction");

        assert_eq!(bye.call_id().expect("BYE Call-ID").value(), invite_call_id);
        assert_eq!(*bye.cseq().expect("BYE CSeq").method(), Method::Bye);
        assert!(
            bye.cseq().expect("BYE CSeq").sequence() > invite_cseq,
            "BYE must advance the dialog CSeq"
        );
        assert_eq!(bye.uri().to_string(), contact_uri);
        assert_eq!(
            bye.raw_header_value(&HeaderName::From).as_deref(),
            Some(invite_from.as_str())
        );
        assert_eq!(
            bye.raw_header_value(&HeaderName::To).as_deref(),
            Some(accepted_to.as_str())
        );

        tokio::time::timeout(Duration::from_secs(5), end)
            .await
            .expect("bounded BYE cleanup deadline")
            .expect("end task")
            .expect("BYE cleanup");
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert!(adapter.by_connection.is_empty());
        assert!(adapter.by_session.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());

        let duplicate_teardown = tokio::time::timeout(Duration::from_millis(1_100), async {
            loop {
                let (bytes, _) = capture.recv_from(&mut packet).await.expect("capture");
                if let Ok(SipMessage::Request(request)) = parse_message(&packet[..bytes]) {
                    if matches!(request.method(), Method::Cancel | Method::Bye) {
                        return request.method().clone();
                    }
                }
            }
        })
        .await;
        assert!(
            duplicate_teardown.is_err(),
            "established teardown emitted a duplicate CANCEL/BYE beyond SIP T1"
        );

        adapter.drain().await.expect("adapter drain");
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn active_outbound_end_reclaims_locally_but_fails_when_bye_is_unanswered() {
        use rvoip_sip_core::{parse_message, Message as SipMessage, Method, StatusCode};
        use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture UAS");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("bye-timeout", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let prepared = ConnectionAdapter::originate(
            adapter.as_ref(),
            OriginateRequest::new(
                CoreSessionId::new(),
                ParticipantId::new(),
                format!("sip:target@{target}"),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip),
        )
        .await
        .expect("prepared route");
        let connection_id = prepared.connection.id.clone();
        let activation_adapter = Arc::clone(&adapter);
        let activation_connection = connection_id.clone();
        let activation = tokio::spawn(async move {
            ConnectionAdapter::activate_outbound_with_receipt(
                activation_adapter.as_ref(),
                activation_connection,
            )
            .await
        });

        let mut packet = [0u8; 16_384];
        let (invite_bytes, uac) =
            tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                .await
                .expect("INVITE deadline")
                .expect("INVITE datagram");
        let SipMessage::Request(invite) =
            parse_message(&packet[..invite_bytes]).expect("parse INVITE")
        else {
            panic!("expected INVITE request")
        };
        let mut accepted = create_response(&invite, StatusCode::Ok);
        if let Some(TypedHeader::To(to)) = accepted
            .headers
            .iter_mut()
            .find(|header| matches!(header, TypedHeader::To(_)))
        {
            to.set_tag("adapter-bye-timeout-uas");
        }
        accepted.headers.push(TypedHeader::Other(
            HeaderName::Contact,
            HeaderValue::Raw(format!("<sip:target@{target}>").into_bytes()),
        ));
        capture
            .send_to(&SipMessage::Response(accepted).to_bytes(), uac)
            .await
            .expect("accept INVITE");
        tokio::time::timeout(Duration::from_secs(5), activation)
            .await
            .expect("activation deadline")
            .expect("activation task")
            .expect("activation receipt");

        let end_adapter = Arc::clone(&adapter);
        let end_connection = connection_id.clone();
        let end = tokio::spawn(async move {
            ConnectionAdapter::end(end_adapter.as_ref(), end_connection, EndReason::Normal).await
        });
        let first_bye = loop {
            let (bytes, _) =
                tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                    .await
                    .expect("BYE deadline")
                    .expect("dialog datagram");
            let SipMessage::Request(request) =
                parse_message(&packet[..bytes]).expect("parse dialog request")
            else {
                continue;
            };
            match request.method() {
                Method::Ack => continue,
                Method::Bye => break request,
                method => panic!("unexpected dialog request: {method}"),
            }
        };
        let bye_call_id = first_bye.call_id().expect("BYE Call-ID").value();
        let bye_cseq = first_bye.cseq().expect("BYE CSeq").sequence();

        let error = tokio::time::timeout(Duration::from_secs(5), end)
            .await
            .expect("bounded BYE timeout cleanup")
            .expect("end task")
            .expect_err("unanswered BYE must not acknowledge transport teardown");
        assert!(matches!(error, RvoipError::Adapter(_)));
        assert!(!adapter.is_connection_live(&connection_id));
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert!(adapter.by_connection.is_empty());
        assert!(adapter.by_session.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());

        // UDP retransmissions are legal, but timeout cleanup must not author a
        // second BYE transaction with a different dialog CSeq or Call-ID.
        while let Ok(Ok((bytes, _))) =
            tokio::time::timeout(Duration::from_millis(150), capture.recv_from(&mut packet)).await
        {
            let Ok(SipMessage::Request(request)) = parse_message(&packet[..bytes]) else {
                continue;
            };
            if request.method() == Method::Bye {
                assert_eq!(
                    request.call_id().expect("retry BYE Call-ID").value(),
                    bye_call_id
                );
                assert_eq!(request.cseq().expect("retry BYE CSeq").sequence(), bye_cseq);
            }
        }

        adapter.drain().await.expect("adapter drain");
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn active_outbound_end_reclaims_locally_but_fails_when_bye_is_rejected() {
        use rvoip_sip_core::{parse_message, Message as SipMessage, Method, StatusCode};
        use rvoip_sip_dialog::transaction::utils::response_builders::create_response;

        let capture = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("capture UAS");
        let target = capture.local_addr().expect("capture address");
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("bye-rejected", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let prepared = ConnectionAdapter::originate(
            adapter.as_ref(),
            OriginateRequest::new(
                CoreSessionId::new(),
                ParticipantId::new(),
                format!("sip:target@{target}"),
                Direction::Outbound,
                CapabilityDescriptor::default(),
            )
            .with_transport(Transport::Sip),
        )
        .await
        .expect("prepared route");
        let connection_id = prepared.connection.id.clone();
        let activation_adapter = Arc::clone(&adapter);
        let activation_connection = connection_id.clone();
        let activation = tokio::spawn(async move {
            ConnectionAdapter::activate_outbound_with_receipt(
                activation_adapter.as_ref(),
                activation_connection,
            )
            .await
        });

        let mut packet = [0u8; 16_384];
        let (invite_bytes, uac) =
            tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                .await
                .expect("INVITE deadline")
                .expect("INVITE datagram");
        let SipMessage::Request(invite) =
            parse_message(&packet[..invite_bytes]).expect("parse INVITE")
        else {
            panic!("expected INVITE request")
        };
        let mut accepted = create_response(&invite, StatusCode::Ok);
        if let Some(TypedHeader::To(to)) = accepted
            .headers
            .iter_mut()
            .find(|header| matches!(header, TypedHeader::To(_)))
        {
            to.set_tag("adapter-bye-rejected-uas");
        }
        accepted.headers.push(TypedHeader::Other(
            HeaderName::Contact,
            HeaderValue::Raw(format!("<sip:target@{target}>").into_bytes()),
        ));
        capture
            .send_to(&SipMessage::Response(accepted).to_bytes(), uac)
            .await
            .expect("accept INVITE");
        tokio::time::timeout(Duration::from_secs(5), activation)
            .await
            .expect("activation deadline")
            .expect("activation task")
            .expect("activation receipt");

        let end_adapter = Arc::clone(&adapter);
        let end_connection = connection_id.clone();
        let end = tokio::spawn(async move {
            ConnectionAdapter::end(end_adapter.as_ref(), end_connection, EndReason::Normal).await
        });
        let (first_bye, bye_peer) = loop {
            let (bytes, peer) =
                tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                    .await
                    .expect("BYE deadline")
                    .expect("dialog datagram");
            let SipMessage::Request(request) =
                parse_message(&packet[..bytes]).expect("parse dialog request")
            else {
                continue;
            };
            match request.method() {
                Method::Ack => continue,
                Method::Bye => break (request, peer),
                method => panic!("unexpected dialog request: {method}"),
            }
        };
        let bye_call_id = first_bye.call_id().expect("BYE Call-ID").value();
        let bye_cseq = first_bye.cseq().expect("BYE CSeq").sequence();
        let rejected = create_response(&first_bye, StatusCode::ServerInternalError);
        capture
            .send_to(&SipMessage::Response(rejected).to_bytes(), bye_peer)
            .await
            .expect("reject BYE transaction");

        let error = tokio::time::timeout(Duration::from_secs(5), end)
            .await
            .expect("bounded rejected-BYE cleanup")
            .expect("end task")
            .expect_err("non-success BYE response must not acknowledge transport teardown");
        assert!(matches!(error, RvoipError::Adapter(_)));
        assert!(!adapter.is_connection_live(&connection_id));
        assert!(adapter.outbound_routes.is_empty());
        assert!(adapter.streams_cache.is_empty());
        assert!(adapter.by_connection.is_empty());
        assert!(adapter.by_session.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());

        // A rejected transaction is terminal. Cleanup may observe a queued
        // retransmission, but it must never author a second BYE transaction.
        while let Ok(Ok((bytes, _))) =
            tokio::time::timeout(Duration::from_millis(150), capture.recv_from(&mut packet)).await
        {
            let Ok(SipMessage::Request(request)) = parse_message(&packet[..bytes]) else {
                continue;
            };
            if request.method() == Method::Bye {
                assert_eq!(
                    request.call_id().expect("retry BYE Call-ID").value(),
                    bye_call_id
                );
                assert_eq!(request.cseq().expect("retry BYE CSeq").sequence(), bye_cseq);
            }
        }

        adapter.drain().await.expect("adapter drain");
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn stream_cache_insert_and_retirement_are_atomic_under_race() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("cache-race", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");

        for _ in 0..100 {
            let session_id = SessionId::new();
            let handle = admit_test_session(&coordinator, &session_id).await;
            let epoch = adapter
                .ensure_mapped_epoch(session_id.clone())
                .expect("admit mapping");
            let connection_id = epoch.connection_id.clone();
            let gate = Arc::new(tokio::sync::Barrier::new(3));
            let insert_adapter = Arc::clone(&adapter);
            let insert_connection = connection_id.clone();
            let insert_gate = Arc::clone(&gate);
            let insert = tokio::spawn(async move {
                insert_gate.wait().await;
                insert_adapter.get_or_insert_dormant_stream(&insert_connection, Direction::Inbound)
            });
            let retire_adapter = Arc::clone(&adapter);
            let retire_epoch = epoch.clone();
            let retire_gate = Arc::clone(&gate);
            let retire = tokio::spawn(async move {
                retire_gate.wait().await;
                retire_adapter.forget_epoch(&retire_epoch)
            });
            gate.wait().await;
            let _ = insert.await.expect("insert task");
            assert!(retire.await.expect("retire task"));

            assert!(!adapter.by_session.contains_key(&session_id));
            assert!(!adapter.by_connection.contains_key(&connection_id));
            assert!(!adapter.streams_cache.contains_key(&connection_id));
            retire_test_session(&coordinator, &handle).await;
        }

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn cached_stream_direction_is_immutable() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("cache-direction", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let session_id = SessionId::new();
        let handle = admit_test_session(&coordinator, &session_id).await;
        let epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("admit mapping");
        let connection_id = epoch.connection_id.clone();
        let inbound = adapter
            .get_or_insert_dormant_stream(&connection_id, Direction::Inbound)
            .expect("inbound stream");
        assert!(adapter
            .get_or_insert_dormant_stream(&connection_id, Direction::Outbound)
            .is_none());
        let cached = adapter
            .streams_cache
            .get(&connection_id)
            .map(|entry| Arc::clone(entry.value()))
            .expect("stable cache");
        assert!(Arc::ptr_eq(&inbound, &cached));

        assert!(adapter.forget_epoch(&epoch));
        retire_test_session(&coordinator, &handle).await;
        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn inbound_media_bind_failure_retires_route_and_delivers_terminal() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("media-bind-failure", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");
        let session_id = SessionId::new();
        let handle = admit_test_session(&coordinator, &session_id).await;
        let connection_id = adapter
            .ensure_mapped(session_id.clone())
            .expect("admit mapping");
        // Reproduce the real race this supervisor protects: signaling admitted
        // an exact route, then the authoritative session retired before the
        // independently retained media bind started. Keeping the adapter epoch
        // mapped here lets the failed bind prove exact retirement and terminal
        // delivery without relying on an invalid, never-admitted fixture.
        retire_test_session(&coordinator, &handle).await;
        let _connection = adapter
            .build_connection(connection_id.clone(), Direction::Inbound)
            .await;

        let terminal = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("bounded media cleanup")
            .expect("terminal event");
        assert!(matches!(
            terminal,
            OrchestratorAdapterEvent::Public(AdapterEvent::Failed { connection_id: id, .. })
                if id == connection_id
        ));
        assert!(!adapter.by_session.contains_key(&session_id));
        assert!(!adapter.by_connection.contains_key(&connection_id));
        assert!(!adapter.streams_cache.contains_key(&connection_id));

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[test]
    fn policy_is_explicit_and_hard_denies_routing_auth_and_identity_headers() {
        let policy = SipInboundContextPolicy::new(["X-Correlation-Id", "X-App-Context"])
            .expect("safe application-extension allowlist");
        assert_eq!(policy.allowed_header_count(), 2);

        for forbidden in [
            "Via",
            "Route",
            "Call-ID",
            "Authorization",
            "Identity",
            "P-Asserted-Identity",
            "Proxy-Require",
            "Refer-To",
            "RAck",
            "RSeq",
            "In-Reply-To",
            "Session-Expires",
            "Min-SE",
            "Event",
            "Subscription-State",
            "SIP-ETag",
            "SIP-If-Match",
            "Replaces",
            "Join",
            "Target-Dialog",
            "X-Bridgefu-Tenant-Id",
            "X-Bridgefu-Call-Id",
            "X-Bridgefu-Correlation-Id",
            "X-Bridgefu",
            "X-Rvoip-Route",
            "X-Rvoip",
            "X-",
        ] {
            assert!(
                matches!(
                    SipInboundContextPolicy::new([forbidden]),
                    Err(SipInboundContextPolicyError::ForbiddenHeaderName)
                ),
                "{forbidden} must remain impossible to expose"
            );
        }
        assert!(matches!(
            SipInboundContextPolicy::new(["bad\r\nname"]),
            Err(SipInboundContextPolicyError::InvalidHeaderName)
        ));
        assert!(matches!(
            SipInboundContextPolicy::new(["bad header"]),
            Err(SipInboundContextPolicyError::InvalidHeaderName)
        ));
    }

    #[test]
    fn request_uri_user_and_allowlisted_duplicate_headers_are_captured_once() {
        let policy = SipInboundContextPolicy::new(["x-correlation-id"]).unwrap();
        let session_id = SessionId::new();
        let connection_id = ConnectionId::new();
        let principal = principal("tenant-a");
        let initial_observation = observation(
            session_id.clone(),
            "attach-secret-a",
            &["first", "second"],
            Some(principal.clone()),
        );
        let pending = policy
            .capture(&initial_observation)
            .unwrap()
            .expect("authenticated context");

        let store = SipInboundContextStore::default();
        assert!(store.observe(session_id.clone(), Some(principal.clone()), Some(pending)));
        assert!(matches!(
            store.bind(&session_id, &connection_id),
            SipInboundBinding::Observed(Some(bound))
                if bound.tenant.as_deref() == Some("tenant-a")
        ));

        let context = store.take(&connection_id).expect("first take");
        assert!(context.is_bound_to(&connection_id, Transport::Sip, &principal));
        assert_eq!(
            context.routing_hint().unwrap().expose_secret(),
            "attach-secret-a"
        );
        assert_eq!(
            context
                .metadata()
                .values("X-Correlation-Id")
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert!(context.metadata().values("X-Unlisted").next().is_none());
        assert!(
            store.take(&connection_id).is_none(),
            "context is single-take"
        );

        // A retransmitted observation plus replayed IncomingCall cannot
        // overwrite the consumed per-live-route tombstone.
        let replay_observation = observation(
            session_id.clone(),
            "attacker-replay",
            &["replacement"],
            Some(principal.clone()),
        );
        let replay = policy.capture(&replay_observation).unwrap().unwrap();
        assert!(store.observe(session_id.clone(), Some(principal), Some(replay)));
        store.bind(&session_id, &connection_id);
        assert!(store.take(&connection_id).is_none());

        store.forget(&session_id, &connection_id);
        assert!(store.take(&connection_id).is_none());
        assert_eq!(store.pending_len(), 0);
    }

    #[test]
    fn interleaved_sessions_bind_only_their_own_request_and_principal() {
        let policy = SipInboundContextPolicy::default();
        let session_a = SessionId::new();
        let session_b = SessionId::new();
        let connection_a = ConnectionId::new();
        let connection_b = ConnectionId::new();
        let principal_a = principal("tenant-a");
        let principal_b = principal("tenant-b");
        let store = SipInboundContextStore::default();

        let observation_b =
            observation(session_b.clone(), "token-b", &[], Some(principal_b.clone()));
        let pending_b = policy.capture(&observation_b).unwrap().unwrap();
        let observation_a =
            observation(session_a.clone(), "token-a", &[], Some(principal_a.clone()));
        let pending_a = policy.capture(&observation_a).unwrap().unwrap();
        assert!(store.observe(
            session_b.clone(),
            Some(principal_b.clone()),
            Some(pending_b)
        ));
        assert!(store.observe(
            session_a.clone(),
            Some(principal_a.clone()),
            Some(pending_a)
        ));

        // Bind in the opposite order from observation.
        store.bind(&session_a, &connection_a);
        store.bind(&session_b, &connection_b);
        let context_b = store.take(&connection_b).unwrap();
        let context_a = store.take(&connection_a).unwrap();
        assert_eq!(context_a.routing_hint().unwrap().expose_secret(), "token-a");
        assert_eq!(context_b.routing_hint().unwrap().expose_secret(), "token-b");
        assert!(context_a.is_bound_to(&connection_a, Transport::Sip, &principal_a));
        assert!(context_b.is_bound_to(&connection_b, Transport::Sip, &principal_b));
        assert!(!context_a.is_bound_to(&connection_a, Transport::Sip, &principal_b));
    }

    #[test]
    fn unauthenticated_invite_and_terminal_cleanup_expose_no_context() {
        let policy = SipInboundContextPolicy::default();
        let session_id = SessionId::new();
        let connection_id = ConnectionId::new();
        let anonymous = observation(session_id.clone(), "anonymous-token", &[], None);
        assert!(policy.capture(&anonymous).unwrap().is_none());

        let store = SipInboundContextStore::default();
        assert!(store.observe(session_id.clone(), None, None));
        store.bind(&session_id, &connection_id);
        assert!(store.take(&connection_id).is_none());
        store.forget(&session_id, &connection_id);
        assert!(store.take(&connection_id).is_none());
    }

    #[test]
    fn bind_rejects_tenantless_expired_and_malformed_authenticated_contexts() {
        let policy = SipInboundContextPolicy::default();

        let tenantless_session = SessionId::new();
        let tenantless_connection = ConnectionId::new();
        let tenantless = principal("");
        let tenantless_observation = observation(
            tenantless_session.clone(),
            "tenantless",
            &[],
            Some(tenantless.clone()),
        );
        let tenantless_context = policy
            .capture(&tenantless_observation)
            .expect("capture")
            .expect("authenticated context");
        let store = SipInboundContextStore::default();
        assert!(store.observe(
            tenantless_session.clone(),
            Some(tenantless),
            Some(tenantless_context)
        ));
        assert!(matches!(
            store.bind(&tenantless_session, &tenantless_connection),
            SipInboundBinding::Rejected(InboundContextError::MissingTenant)
        ));
        assert!(store.take(&tenantless_connection).is_none());

        let expired_session = SessionId::new();
        let expired_connection = ConnectionId::new();
        let mut expired = principal("tenant-a");
        expired.expires_at = Some(Utc::now() - chrono::Duration::seconds(1));
        let expired_observation = observation(
            expired_session.clone(),
            "expired",
            &[],
            Some(expired.clone()),
        );
        let expired_context = policy
            .capture(&expired_observation)
            .expect("capture")
            .expect("authenticated context");
        assert!(store.observe(
            expired_session.clone(),
            Some(expired),
            Some(expired_context)
        ));
        assert!(matches!(
            store.bind(&expired_session, &expired_connection),
            SipInboundBinding::Rejected(InboundContextError::ExpiredPrincipal)
        ));

        let malformed_session = SessionId::new();
        assert!(store.observe_rejected(
            malformed_session.clone(),
            Some(principal("tenant-a")),
            InboundContextError::RoutingHintTooLarge,
        ));
        assert!(matches!(
            store.bind(&malformed_session, &ConnectionId::new()),
            SipInboundBinding::Rejected(InboundContextError::RoutingHintTooLarge)
        ));
    }

    #[test]
    fn publication_boundary_deterministically_rechecks_expiry_after_binding() {
        let expires_at = Utc::now() + chrono::Duration::hours(1);
        let mut expiring = principal("tenant-a");
        expiring.expires_at = Some(expires_at);
        assert!(validate_sip_principal_at(
            &expiring,
            expires_at - chrono::Duration::nanoseconds(1)
        )
        .is_ok());
        assert_eq!(
            validate_sip_principal_at(&expiring, expires_at),
            Err(InboundContextError::ExpiredPrincipal)
        );
    }

    #[test]
    fn pending_observations_are_strictly_bounded_and_expire() {
        let store = SipInboundContextStore::with_pending_limits(2, Duration::from_secs(1));
        let first = SessionId::new();
        let second = SessionId::new();
        let third = SessionId::new();
        assert!(store.observe(first, Some(principal("tenant-a")), None));
        assert!(store.observe(second, Some(principal("tenant-a")), None));
        assert!(!store.observe(third.clone(), Some(principal("tenant-a")), None));
        assert_eq!(store.pending_len(), 2);
        assert!(matches!(
            store.bind(&third, &ConnectionId::new()),
            SipInboundBinding::Missing
        ));

        let expiring = SipInboundContextStore::with_pending_limits(2, Duration::from_millis(2));
        assert!(expiring.observe(SessionId::new(), Some(principal("tenant-a")), None));
        std::thread::sleep(Duration::from_millis(5));
        assert!(expiring.observe(third, Some(principal("tenant-a")), None));
        assert_eq!(expiring.pending_len(), 1);
    }

    #[tokio::test]
    async fn periodic_reaper_physically_removes_idle_expired_contexts() {
        let store = Arc::new(SipInboundContextStore::with_pending_limits(
            2,
            Duration::from_millis(2),
        ));
        assert!(store.observe(SessionId::new(), Some(principal("tenant-a")), None));
        let (cancel, cancel_rx) = watch::channel(false);
        let reaper = tokio::spawn(SipAdapter::run_pending_context_reaper(
            Arc::downgrade(&store),
            cancel_rx,
            Duration::from_millis(1),
        ));
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if store.pending_len() == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("periodic reaper removes expired state without another signaling event");
        cancel.send(true).unwrap();
        reaper.await.unwrap();
    }

    #[tokio::test]
    async fn authenticated_inbound_event_is_one_bounded_queue_item() {
        let (events_tx, mut events_rx) = mpsc::channel(1);
        events_tx
            .send(OrchestratorAdapterEvent::Public(AdapterEvent::Native {
                kind: "queue-filler",
                detail: String::new(),
            }))
            .await
            .unwrap();
        let connection_id = ConnectionId::new();
        let event = OrchestratorAdapterEvent::AuthenticatedInboundConnection {
            connection: inbound_connection(connection_id.clone()),
            participant_id: "sip-peer".into(),
            principal: principal("tenant-a"),
        };
        let sender = events_tx.clone();
        let send =
            tokio::spawn(async move { SipAdapter::send_inbound_event_to(&sender, event).await });
        tokio::task::yield_now().await;
        assert!(
            !send.is_finished(),
            "full queue applies bounded backpressure"
        );
        assert!(matches!(
            events_rx.recv().await,
            Some(OrchestratorAdapterEvent::Public(
                AdapterEvent::Native { .. }
            ))
        ));
        assert!(matches!(
            events_rx.recv().await,
            Some(OrchestratorAdapterEvent::AuthenticatedInboundConnection { connection, principal, .. })
                if connection.id == connection_id && principal.tenant.as_deref() == Some("tenant-a")
        ));
        assert!(send.await.unwrap());
        assert!(matches!(
            events_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn saturated_and_closed_terminal_queues_fallback_exactly_once() {
        let lifecycle = AdapterLifecycleSinkSlot::default();
        let sink = Arc::new(RecordingLifecycleSink {
            deliveries: AtomicUsize::new(0),
        });
        assert!(lifecycle.install(sink.clone()).is_ok());

        let (events_tx, mut events_rx) = mpsc::channel(1);
        events_tx
            .send(OrchestratorAdapterEvent::Public(AdapterEvent::Native {
                kind: "queue-filler",
                detail: String::new(),
            }))
            .await
            .unwrap();
        SipAdapter::deliver_terminal_event_to(
            &lifecycle,
            &events_tx,
            AdapterEvent::Ended {
                connection_id: ConnectionId::new(),
                reason: EndReason::Normal,
            },
            "test-full",
        )
        .await;
        assert_eq!(sink.deliveries.load(Ordering::SeqCst), 1);
        assert!(matches!(
            events_rx.recv().await,
            Some(OrchestratorAdapterEvent::Public(
                AdapterEvent::Native { .. }
            ))
        ));
        assert!(matches!(
            events_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        drop(events_rx);
        SipAdapter::deliver_terminal_event_to(
            &lifecycle,
            &events_tx,
            AdapterEvent::Failed {
                connection_id: ConnectionId::new(),
                detail: "closed".into(),
            },
            "test-closed",
        )
        .await;
        assert_eq!(sink.deliveries.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn adapter_drop_cancels_translator_and_unregisters_weak_observer() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("adapter-drop", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        assert_eq!(coordinator.inbound_invite_observer_count(), 1);
        let weak = Arc::downgrade(&adapter);
        drop(adapter);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if weak.upgrade().is_none() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("translator releases its weak adapter handle");
        assert_eq!(coordinator.inbound_invite_observer_count(), 0);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn failed_atomic_delivery_exact_retirement_suppresses_late_principal_event() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("atomic-failure", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = ConnectionAdapter::subscribe_events(adapter.as_ref());
        let session_id = SessionId::new();
        let handle = admit_test_session(&coordinator, &session_id).await;
        let epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("exact inbound route");
        adapter
            .authenticated_inbound_sessions
            .insert(session_id.clone(), ());

        adapter
            .translate_api_event(ApiEvent::IncomingCallAuthenticated {
                call_id: session_id.clone(),
                principal: principal("tenant-a"),
            })
            .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(25), events.recv())
                .await
                .is_err()
        );
        assert!(adapter
            .authenticated_inbound_sessions
            .contains_key(&session_id));
        assert!(adapter.forget_epoch(&epoch));
        assert!(!adapter
            .authenticated_inbound_sessions
            .contains_key(&session_id));
        retire_test_session(&coordinator, &handle).await;

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[test]
    fn failed_inbound_termination_is_state_aware() {
        assert_eq!(
            failed_inbound_termination(Some(CallState::Ringing), false),
            FailedInboundTermination::Reject
        );
        assert_eq!(
            failed_inbound_termination(Some(CallState::Active), false),
            FailedInboundTermination::Hangup
        );
        assert_eq!(
            failed_inbound_termination(Some(CallState::Ringing), true),
            FailedInboundTermination::Hangup
        );
        assert_eq!(
            failed_inbound_termination(Some(CallState::Terminated), true),
            FailedInboundTermination::CleanupOnly
        );
    }

    #[tokio::test]
    async fn saturated_delivery_fast_auto_accept_hangs_up_and_releases_local_capacity() {
        let config =
            ApiConfig::local("atomic-fast-cleanup", 0).with_fast_auto_accept_incoming_calls(true);
        let coordinator = UnifiedCoordinator::new(config).await.expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let session_id = SessionId::new();
        coordinator
            .helpers
            .create_session(
                session_id.clone(),
                "sip:bridge@example.test".into(),
                "sip:caller@example.test".into(),
                Role::UAS,
            )
            .await
            .expect("session");
        let mut session = coordinator
            .session_state(&session_id)
            .await
            .expect("session state");
        session.call_state = CallState::Active;
        coordinator
            .update_session_state(session)
            .await
            .expect("active state");

        let epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("test route mapping");
        let connection_id = epoch.connection_id.clone();
        adapter
            .authenticated_inbound_sessions
            .insert(session_id.clone(), ());
        adapter
            .inbound_contexts
            .by_connection
            .insert(connection_id.clone(), SipInboundContextState::Consumed);
        for _ in 0..256 {
            adapter
                .out_tx
                .try_send(OrchestratorAdapterEvent::Public(AdapterEvent::Native {
                    kind: "queue-filler",
                    detail: String::new(),
                }))
                .expect("fill bounded event queue");
        }

        let delivered = adapter
            .send_inbound_event(OrchestratorAdapterEvent::AuthenticatedInboundConnection {
                connection: inbound_connection(connection_id.clone()),
                participant_id: "sip-peer".into(),
                principal: principal("tenant-a"),
            })
            .await;
        assert!(!delivered, "saturated queue must reject atomic handoff");
        adapter
            .terminate_failed_inbound(&session_id, Some(&epoch), 503, "test saturated publication")
            .await;

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if !adapter.by_session.contains_key(&session_id)
                    && !adapter.by_connection.contains_key(&connection_id)
                    && !adapter
                        .authenticated_inbound_sessions
                        .contains_key(&session_id)
                    && !adapter
                        .inbound_contexts
                        .by_connection
                        .contains_key(&connection_id)
                    && !adapter.streams_cache.contains_key(&connection_id)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all adapter-owned capacity is released");
        assert!(coordinator.session_state(&session_id).await.is_err());

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn exact_epochs_isolate_two_calls_raw_id_reuse_and_delayed_cleanup() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("exact-route-reuse", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        adapter
            .configure_lifecycle_limits(2, 2)
            .expect("configure before admission");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");

        let reused_session = SessionId::new();
        let first_handle = admit_test_session(&coordinator, &reused_session).await;
        let first_epoch = adapter
            .ensure_mapped_epoch(reused_session.clone())
            .expect("first exact route");
        assert_eq!(first_epoch.admitted_handle(), Some(&first_handle));

        let unrelated_session = SessionId::new();
        let unrelated_handle = admit_test_session(&coordinator, &unrelated_session).await;
        let unrelated_epoch = adapter
            .ensure_mapped_epoch(unrelated_session.clone())
            .expect("interleaved exact route");
        assert_eq!(unrelated_epoch.admitted_handle(), Some(&unrelated_handle));

        // Normal teardown removes the first route. A supervisor that retained
        // the same exact epoch is deliberately delayed until after the raw ID
        // has been admitted under a new authority handle.
        assert!(adapter.forget_epoch(&first_epoch));
        let delayed_gate = Arc::new(tokio::sync::Barrier::new(2));
        let delayed_adapter = Arc::clone(&adapter);
        let delayed_epoch = first_epoch.clone();
        let delayed_release = Arc::clone(&delayed_gate);
        let delayed_cleanup = tokio::spawn(async move {
            delayed_release.wait().await;
            delayed_adapter.forget_epoch(&delayed_epoch)
        });

        retire_test_session(&coordinator, &first_handle).await;
        elapse_test_reuse_horizon(&coordinator, &reused_session);
        let current_handle = admit_test_session(&coordinator, &reused_session).await;
        assert_ne!(first_handle, current_handle);
        let current_epoch = adapter
            .ensure_mapped_epoch(reused_session.clone())
            .expect("same raw SessionId with a new exact handle");
        assert_eq!(current_epoch.admitted_handle(), Some(&current_handle));
        assert_ne!(first_epoch, current_epoch);
        assert_ne!(first_epoch.connection_id, current_epoch.connection_id);

        delayed_gate.wait().await;
        assert!(
            !delayed_cleanup.await.expect("delayed cleanup task"),
            "a delayed exact cleanup must not remove the reused raw ID"
        );
        assert!(adapter.try_send_for_epoch(
            &first_epoch,
            AdapterEvent::Connected {
                connection_id: first_epoch.connection_id.clone(),
            },
        ));
        assert!(!adapter.forget_epoch(&first_epoch));
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        let retained_epoch = {
            let _mapping = adapter.mapping_lock.lock().unwrap();
            adapter
                .route_epoch_for_connection_locked(&current_epoch.connection_id)
                .expect("delayed cleanup cannot remove current exact route")
        };
        assert_eq!(retained_epoch, current_epoch);

        let overflow_session = SessionId::new();
        let overflow_handle = admit_test_session(&coordinator, &overflow_session).await;
        assert!(
            adapter.ensure_mapped(overflow_session).is_none(),
            "two unrelated exact routes consume the configured active budget"
        );
        retire_test_session(&coordinator, &overflow_handle).await;

        assert!(adapter.try_send_for_epoch(
            &current_epoch,
            AdapterEvent::Connected {
                connection_id: current_epoch.connection_id.clone(),
            },
        ));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Connected { connection_id }))
                if connection_id == current_epoch.connection_id
        ));
        assert!(adapter.try_send_for_epoch(
            &unrelated_epoch,
            AdapterEvent::Connected {
                connection_id: unrelated_epoch.connection_id.clone(),
            },
        ));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Connected { connection_id }))
                if connection_id == unrelated_epoch.connection_id
        ));
        assert!(adapter.forget_epoch(&current_epoch));
        assert!(adapter.forget_epoch(&unrelated_epoch));
        retire_test_session(&coordinator, &current_handle).await;
        retire_test_session(&coordinator, &unrelated_handle).await;

        adapter.shutdown().await.expect("adapter shutdown");
    }

    #[tokio::test]
    async fn terminal_event_retires_route_after_exact_session_release_wins_race() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("terminal-after-release", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");
        let session_id = SessionId::new();
        let handle = admit_test_session(&coordinator, &session_id).await;
        let epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("exact inbound route");
        let connection_id = epoch.connection_id.clone();

        // Reproduce the event-bus race: terminal publication has happened,
        // then coordinator cleanup retires the exact session before the
        // adapter translator consumes that publication.
        retire_test_session(&coordinator, &handle).await;
        assert!(coordinator.session_state(&session_id).await.is_err());
        assert!(adapter.is_connection_live(&connection_id));

        adapter
            .translate_api_event(ApiEvent::CallEnded {
                call_id: session_id.clone(),
                reason: "peer BYE".into(),
            })
            .await;

        assert!(!adapter.is_connection_live(&connection_id));
        assert!(!adapter.by_session.contains_key(&session_id));
        assert!(!adapter.by_connection.contains_key(&connection_id));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Ended {
                connection_id: observed,
                ..
            })) if observed == connection_id
        ));

        adapter.shutdown().await.expect("adapter shutdown");
    }

    #[tokio::test]
    async fn retired_route_terminal_cannot_cross_same_session_id_generation() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("terminal-generation-fence", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");
        let session_id = SessionId::from("terminal-generation-fence".to_string());
        let old_handle = admit_test_session(&coordinator, &session_id).await;
        let old_epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("old exact route");
        let old_connection = old_epoch.connection_id.clone();

        retire_test_session(&coordinator, &old_handle).await;
        elapse_test_reuse_horizon(&coordinator, &session_id);
        let new_handle = admit_test_session(&coordinator, &session_id).await;
        assert_ne!(old_handle, new_handle);

        // The adapter still retains the old exact route while the authority
        // already owns a newer generation. A late raw terminal publication
        // must not remove that mapping under the newer session's authority.
        adapter
            .translate_api_event(ApiEvent::CallEnded {
                call_id: session_id.clone(),
                reason: "delayed old-generation terminal".into(),
            })
            .await;
        assert!(adapter.is_connection_live(&old_connection));
        assert_eq!(
            adapter.current_session_handle(&session_id),
            Some(new_handle.clone())
        );
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        // Once the old mapping is explicitly retired, the new exact route can
        // be admitted. A stale exact-epoch continuation remains fenced from it.
        assert!(adapter.forget_epoch(&old_epoch));
        let new_epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("new exact route");
        let new_connection = new_epoch.connection_id.clone();
        adapter
            .deliver_terminal_for_epoch(
                &old_epoch,
                AdapterEvent::Ended {
                    connection_id: old_connection,
                    reason: EndReason::Normal,
                },
                "stale-generation-test",
            )
            .await;
        assert!(adapter.is_connection_live(&new_connection));
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        assert!(adapter.forget_epoch(&new_epoch));
        retire_test_session(&coordinator, &new_handle).await;
        adapter.shutdown().await.expect("adapter shutdown");
    }

    #[tokio::test]
    async fn retained_publication_waits_for_capacity_and_flushes_fifo_once() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("staged-activation", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");
        let session_id = SessionId::new();
        let connection_id = ConnectionId::new();
        let route = SipOutboundRoute::new(
            connection_id.clone(),
            session_id,
            "sip:target@example.test".to_string(),
            Arc::new(SipOriginateContext::default()),
            crate::media_stream::SipMediaStream::dormant_deferred(Direction::Outbound),
        );
        assert!(route.claim_activation().expect("first activation"));
        assert_eq!(
            route.stage_event(AdapterEvent::Connected {
                connection_id: connection_id.clone(),
            }),
            SipRouteStageDisposition::Retained
        );
        assert_eq!(
            route.stage_event(AdapterEvent::Dtmf {
                connection_id: connection_id.clone(),
                digits: "5".into(),
                duration_ms: 100,
            }),
            SipRouteStageDisposition::Retained
        );

        for _ in 0..(SIP_ADAPTER_EVENT_CAPACITY - 1) {
            adapter
                .out_tx
                .try_send(OrchestratorAdapterEvent::Public(AdapterEvent::Native {
                    kind: "queue-filler",
                    detail: String::new(),
                }))
                .expect("fill all but one queue slot");
        }
        let flush_route = Arc::clone(&route);
        let flush_events = adapter.out_tx.clone();
        let flush =
            tokio::spawn(async move { flush_route.publish_staged_events(&flush_events).await });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if route
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .phase
                    == SipOutboundRoutePhase::Flushing
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("publisher entered explicit flushing phase");
        assert!(!flush.is_finished(), "second staged event awaits capacity");
        assert_eq!(
            route
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .phase,
            SipOutboundRoutePhase::Flushing
        );
        assert_eq!(
            route.stage_event(AdapterEvent::Dtmf {
                connection_id: connection_id.clone(),
                digits: "6".into(),
                duration_ms: 100,
            }),
            SipRouteStageDisposition::Retained,
            "events arriving during the explicit flushing phase remain FIFO"
        );

        for _ in 0..(SIP_ADAPTER_EVENT_CAPACITY - 1) {
            assert!(matches!(
                events.recv().await,
                Some(OrchestratorAdapterEvent::Public(AdapterEvent::Native {
                    kind: "queue-filler",
                    ..
                }))
            ));
        }
        assert!(!flush.await.expect("retained publisher").expect("flush"));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Connected { connection_id: id }))
                if id == connection_id
        ));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Dtmf { connection_id: id, digits, .. }))
                if id == connection_id && digits == "5"
        ));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Dtmf { connection_id: id, digits, .. }))
                if id == connection_id && digits == "6"
        ));
        assert_eq!(
            route.stage_event(AdapterEvent::Connected {
                connection_id: connection_id.clone(),
            }),
            SipRouteStageDisposition::Forward
        );

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn retained_publication_does_not_retry_after_terminal_failure() {
        let (events, receiver) = mpsc::channel(1);
        drop(receiver);
        let connection_id = ConnectionId::new();
        let route = SipOutboundRoute::new(
            connection_id.clone(),
            SessionId::new(),
            "sip:target@example.test".to_string(),
            Arc::new(SipOriginateContext::default()),
            crate::media_stream::SipMediaStream::dormant_deferred(Direction::Outbound),
        );
        assert!(route.claim_activation().expect("claim"));
        assert_eq!(
            route.stage_event(AdapterEvent::Connected { connection_id }),
            SipRouteStageDisposition::Retained
        );
        assert!(matches!(
            route.publish_staged_events(&events).await,
            Err(SipActivationFailure::EventPublication)
        ));
        assert!(!route.claim_activation().expect("driver remains claimed"));
    }

    #[tokio::test]
    async fn one_hundred_activation_callers_claim_one_retained_driver() {
        let connection_id = ConnectionId::new();
        let route = SipOutboundRoute::new(
            connection_id,
            SessionId::new(),
            "sip:target@example.test".to_string(),
            Arc::new(SipOriginateContext::default()),
            crate::media_stream::SipMediaStream::dormant_deferred(Direction::Outbound),
        );
        let gate = Arc::new(tokio::sync::Barrier::new(101));
        let mut claims = Vec::new();
        for _ in 0..100 {
            let claim_route = Arc::clone(&route);
            let claim_gate = Arc::clone(&gate);
            claims.push(tokio::spawn(async move {
                claim_gate.wait().await;
                claim_route.claim_activation()
            }));
        }
        gate.wait().await;
        let mut first = 0;
        for claim in claims {
            if claim.await.expect("claim task").expect("live route") {
                first += 1;
            }
        }
        assert_eq!(first, 1);

        let cancelled_route = Arc::clone(&route);
        let cancelled = tokio::spawn(async move { cancelled_route.wait_activation().await });
        cancelled.abort();
        let mut waiters = Vec::new();
        for _ in 0..99 {
            let waiter_route = Arc::clone(&route);
            waiters.push(tokio::spawn(
                async move { waiter_route.wait_activation().await },
            ));
        }
        let reference =
            ExternalConnectionReference::new("sip.call-id", "one@example.test").expect("reference");
        route
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .phase = SipOutboundRoutePhase::Active;
        assert!(route
            .complete_activation_success(OutboundActivation::with_external_reference(reference,)));
        for waiter in waiters {
            assert!(waiter.await.expect("activation waiter").is_ok());
        }
    }

    #[tokio::test]
    async fn fast_terminal_fails_every_waiter_before_any_activation_receipt() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("fast-terminal", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");
        let connection_id = ConnectionId::new();
        let route = SipOutboundRoute::new(
            connection_id.clone(),
            SessionId::new(),
            "sip:target@example.test".to_string(),
            Arc::new(SipOriginateContext::default()),
            crate::media_stream::SipMediaStream::dormant_deferred(Direction::Outbound),
        );
        adapter
            .reserve_outbound_route(Arc::clone(&route))
            .expect("route reservation");
        let epoch = SipRouteEpoch {
            session_id: route.session_id.clone(),
            connection_id: connection_id.clone(),
            owner: SipRouteEpochOwner::Prepared(Arc::clone(&route)),
        };
        assert!(route.claim_activation().expect("activation claim"));
        let mut waiters = Vec::new();
        for _ in 0..100 {
            let waiter_route = Arc::clone(&route);
            waiters.push(tokio::spawn(
                async move { waiter_route.wait_activation().await },
            ));
        }
        for _ in 0..SIP_OUTBOUND_EVENT_STAGE_CAPACITY {
            assert_eq!(
                route.stage_event(AdapterEvent::Connected {
                    connection_id: connection_id.clone(),
                }),
                SipRouteStageDisposition::Retained
            );
        }
        assert_eq!(
            route.stage_event(AdapterEvent::Dtmf {
                connection_id: connection_id.clone(),
                digits: "8".to_string(),
                duration_ms: 100,
            }),
            SipRouteStageDisposition::Retained
        );
        adapter
            .deliver_terminal_for_epoch(
                &epoch,
                AdapterEvent::Ended {
                    connection_id: connection_id.clone(),
                    reason: EndReason::Normal,
                },
                "test-fast-terminal",
            )
            .await;
        assert!(!route.is_publicly_live());
        let reference = ExternalConnectionReference::new("sip.call-id", "too-late@example.test")
            .expect("reference");
        assert!(!route
            .complete_activation_success(OutboundActivation::with_external_reference(reference)));
        for waiter in waiters {
            assert!(waiter.await.expect("activation waiter").is_err());
        }
        route.wait_cleanup().await.expect("zero-wire cleanup");
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Ended { connection_id: id, .. }))
                if id == connection_id
        ));
        adapter.shutdown().await.expect("adapter shutdown");
    }

    #[tokio::test]
    async fn activation_after_terminal_never_replays_cached_success() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("terminal-reactivation", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let connection_id = ConnectionId::new();
        let route = SipOutboundRoute::new(
            connection_id.clone(),
            SessionId::new(),
            "sip:target@example.test".to_string(),
            Arc::new(SipOriginateContext::default()),
            crate::media_stream::SipMediaStream::dormant_deferred(Direction::Outbound),
        );
        adapter
            .reserve_outbound_route(Arc::clone(&route))
            .expect("route reservation");
        assert!(route.claim_activation().expect("initial activation claim"));
        route
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .phase = SipOutboundRoutePhase::Active;
        let reference = ExternalConnectionReference::new(
            "sip.call-id",
            "completed-before-terminal@example.test",
        )
        .expect("reference");
        assert!(route
            .complete_activation_success(OutboundActivation::with_external_reference(reference),));
        assert_eq!(
            route.stage_event(AdapterEvent::Ended {
                connection_id: connection_id.clone(),
                reason: EndReason::Normal,
            }),
            SipRouteStageDisposition::Retained
        );

        let mut callers = Vec::new();
        for _ in 0..100 {
            let caller = Arc::clone(&adapter);
            let caller_connection = connection_id.clone();
            callers.push(tokio::spawn(async move {
                ConnectionAdapter::activate_outbound_with_receipt(
                    caller.as_ref(),
                    caller_connection,
                )
                .await
            }));
        }
        for caller in callers {
            assert!(
                tokio::time::timeout(Duration::from_secs(1), caller)
                    .await
                    .expect("reactivation deadline")
                    .expect("reactivation task")
                    .is_err(),
                "a terminating route must never return its cached receipt"
            );
        }

        adapter.drain().await.expect("zero-wire route drain");
        adapter.shutdown().await.expect("coordinator shutdown");
    }

    #[tokio::test]
    async fn local_terminal_cleanup_survives_io_failure_and_suppresses_late_api_events() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("terminal-cleanup", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");

        let ended_session = SessionId::new();
        let ended_handle = admit_test_session(&coordinator, &ended_session).await;
        set_test_call_state(&coordinator, &ended_session, CallState::Active).await;
        let ended_connection = adapter
            .ensure_mapped(ended_session.clone())
            .expect("end mapping");
        assert!(adapter
            .end(ended_connection.clone(), EndReason::Normal)
            .await
            .is_err());
        assert!(!adapter.is_connection_live(&ended_connection));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Ended { connection_id, .. }))
                if connection_id == ended_connection
        ));
        adapter
            .translate_api_event(ApiEvent::CallEnded {
                call_id: ended_session,
                reason: "late peer terminal".into(),
            })
            .await;
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let rejected_session = SessionId::new();
        let rejected_handle = admit_test_session(&coordinator, &rejected_session).await;
        set_test_call_state(&coordinator, &rejected_session, CallState::Ringing).await;
        let rejected_connection = adapter
            .ensure_mapped(rejected_session.clone())
            .expect("reject mapping");
        assert!(adapter
            .reject(rejected_connection.clone(), RejectReason::Decline)
            .await
            .is_err());
        assert!(!adapter.is_connection_live(&rejected_connection));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Failed { connection_id, .. }))
                if connection_id == rejected_connection
        ));
        adapter
            .translate_api_event(ApiEvent::CallFailed {
                call_id: rejected_session,
                status_code: 603,
                reason: "late reject".into(),
            })
            .await;
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        retire_test_session_if_current(&coordinator, &ended_handle).await;
        retire_test_session_if_current(&coordinator, &rejected_handle).await;

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn call_progress_is_connection_scoped_and_marks_only_sdp_183_as_early_media() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("typed-call-progress", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");
        let session_id = SessionId::new();
        let handle = admit_test_session(&coordinator, &session_id).await;
        let epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("exact route");

        adapter
            .translate_api_event(ApiEvent::CallProgress {
                call_id: session_id.clone(),
                status_code: 180,
                reason: "Ringing".into(),
                sdp: None,
            })
            .await;
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Progress {
                connection_id,
                status_code: 180,
                reason,
                early_media: false,
            })) if connection_id == epoch.connection_id && reason == "Ringing"
        ));

        adapter
            .translate_api_event(ApiEvent::CallProgress {
                call_id: session_id.clone(),
                status_code: 183,
                reason: "Session Progress".into(),
                sdp: Some("v=0\r\nm=audio 40000 RTP/SAVP 0\r\n".into()),
            })
            .await;
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Progress {
                connection_id,
                status_code: 183,
                reason,
                early_media: true,
            })) if connection_id == epoch.connection_id && reason == "Session Progress"
        ));

        assert!(adapter.forget_epoch(&epoch));
        retire_test_session_if_current(&coordinator, &handle).await;
        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    fn sip_data_message_request(message: &DataMessage) -> rvoip_sip_core::Request {
        use rvoip_sip_core::types::ContentType;

        let wire = crate::sip_data_message::to_sip_data_message(message).expect("SIP mapping");
        let mut request = rvoip_sip_core::Request::new(
            rvoip_sip_core::Method::Message,
            Uri::from_str("sip:peer@example.test").expect("URI"),
        )
        .with_body(wire.bytes);
        request.headers.push(TypedHeader::ContentType(
            ContentType::from_str(&wire.content_type).expect("content type"),
        ));
        request.headers.extend(wire.extra_headers);
        request
    }

    #[tokio::test]
    async fn sip_data_message_stale_exact_handle_cannot_reach_reused_session() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("data-message-exact-handle", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");
        let session_id = SessionId::from("data-message-reused-session".to_string());
        let old_handle = admit_test_session(&coordinator, &session_id).await;
        let old_epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("old exact route");
        assert_eq!(old_epoch.admitted_handle(), Some(&old_handle));
        assert!(adapter.forget_epoch(&old_epoch));
        retire_test_session(&coordinator, &old_handle).await;
        elapse_test_reuse_horizon(&coordinator, &session_id);
        let new_handle = admit_test_session(&coordinator, &session_id).await;
        let new_epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("new exact route");
        assert_eq!(new_epoch.admitted_handle(), Some(&new_handle));
        assert_ne!(old_handle, new_handle);
        assert_ne!(old_epoch.connection_id, new_epoch.connection_id);

        let message = DataMessage::try_new(
            "bridgefu.context.v1",
            "application/json",
            bytes::Bytes::from_static(br#"{"event":"screen-pop"}"#),
            rvoip_core::DataReliability::ReliableOrdered,
            rvoip_core::MessageId::from_string("msg-exact-handle-test"),
        )
        .expect("message");
        let request = sip_data_message_request(&message);

        assert!(adapter
            .publish_sip_data_message_for_epoch(&old_epoch, &request)
            .expect("stale conversion"));
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        assert!(adapter
            .publish_sip_data_message_for_epoch(&new_epoch, &request)
            .expect("current conversion"));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::DataMessage {
                connection_id,
                message: delivered,
            })) if connection_id == new_epoch.connection_id && delivered == message
        ));

        assert!(adapter.forget_epoch(&new_epoch));
        retire_test_session(&coordinator, &new_handle).await;
        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn refer_updates_are_typed_ordered_and_bound_to_the_exact_live_route() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("typed-refer-status", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");
        let session_id = SessionId::new();
        let handle = admit_test_session(&coordinator, &session_id).await;
        let epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("route mapping");
        let connection_id = epoch.connection_id.clone();
        let attempt_id = TransferAttemptId::new();
        assert_eq!(
            adapter
                .reserve_transfer_attempt(&connection_id, Some(attempt_id.clone()))
                .expect("transfer reservation"),
            epoch
        );

        adapter
            .translate_api_event(ApiEvent::TransferAccepted {
                call_id: session_id.clone(),
                refer_to: "sip:target@example.test".into(),
            })
            .await;
        adapter
            .translate_api_event(ApiEvent::ReferProgress {
                call_id: session_id.clone(),
                status_code: 180,
                reason: "Ringing".into(),
            })
            .await;
        adapter
            .translate_api_event(ApiEvent::ReferCompleted {
                call_id: session_id.clone(),
                target: "sip:target@example.test".into(),
                status_code: 200,
                reason: "OK".into(),
            })
            .await;

        let accepted = events.recv().await.expect("accepted event");
        assert!(matches!(
            accepted,
            OrchestratorAdapterEvent::Public(AdapterEvent::TransferStatus {
                connection_id: id,
                attempt_id: Some(ref observed_attempt_id),
                status: TransferStatus::Accepted,
            }) if id == connection_id && observed_attempt_id == &attempt_id
        ));
        let progress = events.recv().await.expect("progress event");
        assert!(matches!(
            progress,
            OrchestratorAdapterEvent::Public(AdapterEvent::TransferStatus {
                connection_id: id,
                attempt_id: Some(ref observed_attempt_id),
                status: TransferStatus::Progress {
                    status_code: 180,
                    ref reason,
                },
            }) if id == connection_id
                && observed_attempt_id == &attempt_id
                && reason == "Ringing"
        ));
        let completed = events.recv().await.expect("completed event");
        assert!(matches!(
            completed,
            OrchestratorAdapterEvent::Public(AdapterEvent::TransferStatus {
                connection_id: id,
                attempt_id: Some(ref observed_attempt_id),
                status: TransferStatus::Completed {
                    status_code: 200,
                    ref reason,
                },
            }) if id == connection_id
                && observed_attempt_id == &attempt_id
                && reason == "OK"
        ));

        assert!(adapter
            .reserve_transfer_attempt(&connection_id, Some(TransferAttemptId::new()))
            .is_err());
        adapter
            .translate_api_event(ApiEvent::TransferFailed {
                call_id: session_id.clone(),
                status_code: 503,
                reason: "duplicate terminal failure".into(),
            })
            .await;
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        assert!(adapter.forget_epoch(&epoch));
        assert!(!adapter.transfer_attempts.contains_key(&connection_id));
        let next_epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("replacement exact route");
        let next_attempt_id = TransferAttemptId::new();
        adapter
            .reserve_transfer_attempt(&next_epoch.connection_id, Some(next_attempt_id.clone()))
            .expect("replacement route transfer reservation");
        assert!(!adapter.forget_epoch(&epoch));
        assert!(adapter
            .transfer_attempts
            .get(&next_epoch.connection_id)
            .is_some_and(|state| state.attempt_id.as_ref() == Some(&next_attempt_id)));
        assert!(adapter.forget_epoch(&next_epoch));
        adapter
            .translate_api_event(ApiEvent::TransferFailed {
                call_id: session_id.clone(),
                status_code: 503,
                reason: "late failure".into(),
            })
            .await;
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        retire_test_session(&coordinator, &handle).await;
        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }

    #[tokio::test]
    async fn sip_data_message_requires_exact_live_dialog_and_supported_reliability() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("data-message-dialog", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let session_id = SessionId::new();
        let handle = admit_test_session(&coordinator, &session_id).await;
        let epoch = adapter
            .ensure_mapped_epoch(session_id.clone())
            .expect("route mapping");
        let connection_id = epoch.connection_id.clone();

        let error = ConnectionAdapter::send_data_message(
            adapter.as_ref(),
            connection_id.clone(),
            DataMessage::reliable("context", "application/json", "{}"),
        )
        .await
        .expect_err("mapping without a dialog must fail closed");
        assert!(matches!(error, RvoipError::InvalidState(_)));

        let mut unsupported = DataMessage::reliable("context", "application/json", "{}");
        unsupported.reliability = rvoip_core::DataReliability::MaxLifetime {
            ordered: true,
            milliseconds: 1_000,
        };
        let error =
            ConnectionAdapter::send_data_message(adapter.as_ref(), connection_id, unsupported)
                .await
                .expect_err("SIP must not emulate an unsupported reliability policy");
        assert!(matches!(error, RvoipError::NotImplemented(_)));

        assert!(adapter.forget_epoch(&epoch));
        retire_test_session(&coordinator, &handle).await;
        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }
}
