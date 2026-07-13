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
use crate::types::CallState;
use crate::SessionId;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_core::adapter::{
    legacy_normalized_event_receiver, AdapterEvent, AdapterKind, AdapterLifecycleCapabilities,
    AdapterLifecycleSink, AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    ExternalConnectionReference, InboundConnectionContext, InboundContextError, InboundRoutingHint,
    InboundSignalingMetadata, OrchestratorAdapterEvent, OriginateRequest, OutboundActivation,
    RejectReason, SignatureHeaders, TerminalDelivery, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as CoreResult, RvoipError};
use rvoip_core::identity::{AuthenticatedPrincipal, IdentityAssurance};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId as CoreSessionId};
use rvoip_core::message::Message;
use rvoip_core::stream::{MediaStream, MediaStreamHandle};
use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
use rvoip_sip_core::types::uri::Scheme;
use rvoip_sip_core::Uri;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

const MAX_SIP_INBOUND_ALLOWLIST_HEADERS: usize = 32;
const MAX_PENDING_SIP_INBOUND_CONTEXTS: usize = 4_096;
const PENDING_SIP_INBOUND_CONTEXT_TTL: Duration = Duration::from_secs(120);
const PENDING_SIP_INBOUND_CONTEXT_REAPER_INTERVAL: Duration = Duration::from_secs(1);
const SIP_INBOUND_EVENT_DELIVERY_TIMEOUT: Duration = Duration::from_secs(2);
const SIP_ADAPTER_EVENT_CAPACITY: usize = 256;
const DEFAULT_SIP_ACTIVE_CONNECTION_BUDGET: usize = 262_144;
const DEFAULT_SIP_RETIRED_SESSION_BUDGET: usize = 262_144;
const SIP_OUTBOUND_EVENT_STAGE_CAPACITY: usize = 32;
const MAX_SIP_OUTBOUND_TARGET_BYTES: usize = 4_096;
const SIP_RETAINED_TASK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SipOutboundRoutePhase {
    Prepared,
    Activating,
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
            SipOutboundRoutePhase::Activating | SipOutboundRoutePhase::Active => Ok(false),
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
        match state.phase {
            SipOutboundRoutePhase::Prepared | SipOutboundRoutePhase::Activating => {
                if terminal {
                    state.remote_terminal_seen = true;
                    if state.terminal.is_none() {
                        state.terminal = Some(event);
                    }
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
        }
    }

    async fn publish_staged_events(
        &self,
        events: &mpsc::Sender<OrchestratorAdapterEvent>,
    ) -> Result<bool, SipActivationFailure> {
        let deadline = tokio::time::Instant::now() + SIP_RETAINED_TASK_TIMEOUT;
        let mut cancel = self.cancel.subscribe();
        loop {
            let next = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if state.phase != SipOutboundRoutePhase::Activating {
                    return Err(SipActivationFailure::RouteEnded);
                }
                if state.overflowed {
                    return Err(SipActivationFailure::EventOverflow);
                }
                match state.events.pop_front() {
                    Some(event) => Some(event),
                    None => {
                        state.phase = SipOutboundRoutePhase::Active;
                        return Ok(state.remote_terminal_seen);
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

    fn complete_activation(&self, completion: SipActivationCompletion) {
        let publish = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.activation_completed {
                false
            } else {
                state.activation_completed = true;
                if matches!(completion, SipActivationCompletion::Failed(_))
                    && !matches!(
                        state.phase,
                        SipOutboundRoutePhase::Terminating | SipOutboundRoutePhase::Terminated
                    )
                {
                    state.phase = SipOutboundRoutePhase::Failed;
                }
                true
            }
        };
        if publish {
            self.activation.send_replace(Some(completion));
        }
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
    by_connection: Arc<DashMap<ConnectionId, SessionId>>,
    /// SIP api SessionId → rvoip-core ConnectionId. Used by the event
    /// translator task to map outgoing api::Event → AdapterEvent.
    by_session: Arc<DashMap<SessionId, ConnectionId>>,
    /// Serializes paired forward/reverse mapping changes and retirement.
    mapping_lock: StdMutex<()>,
    /// Process-lifetime Session tombstones. SIP API events carry no route
    /// epoch, so recently terminal Session IDs must not be mapped again.
    retired_sessions: DashMap<SessionId, ()>,
    /// Set when the finite tombstone budget is exhausted. Since SIP API
    /// events carry no route epoch, admitting any further unknown Session ID
    /// could resurrect an evicted route; saturation therefore fails closed.
    lifecycle_admission_poisoned: AtomicBool,
    /// Configurable active-route and tombstone limits. They may only be
    /// changed while the adapter has no live or retired mappings.
    active_connection_budget: AtomicUsize,
    retired_session_budget: AtomicUsize,
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
        let mut events = coordinator.events().await?;
        let (out_tx, out_rx) = mpsc::channel(SIP_ADAPTER_EVENT_CAPACITY);
        let (translator_cancel, mut translator_cancel_rx) = watch::channel(false);
        let context_reaper_cancel_rx = translator_cancel.subscribe();
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
            retired_sessions: DashMap::new(),
            lifecycle_admission_poisoned: AtomicBool::new(false),
            active_connection_budget: AtomicUsize::new(DEFAULT_SIP_ACTIVE_CONNECTION_BUDGET),
            retired_session_budget: AtomicUsize::new(DEFAULT_SIP_RETIRED_SESSION_BUDGET),
            outbound_routes: DashMap::new(),
            out_tx: out_tx.clone(),
            out_rx: StdMutex::new(Some(out_rx)),
            streams_cache: Arc::new(DashMap::new()),
            inbound_contexts,
            authenticated_inbound_sessions: DashMap::new(),
            lifecycle: AdapterLifecycleSinkSlot::default(),
            translator_cancel,
            inbound_invite_observer_id,
        });

        // Subscribe to the coordinator's typed event stream and spawn the
        // translator task. EventReceiver yields api::Event values; we map
        // each into AdapterEvent and forward.
        let me = Arc::downgrade(&adapter);
        tokio::spawn(async move {
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
        tokio::spawn(Self::run_pending_context_reaper(
            Arc::downgrade(&adapter.inbound_contexts),
            context_reaper_cancel_rx,
            PENDING_SIP_INBOUND_CONTEXT_REAPER_INTERVAL,
        ));

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

    fn ensure_mapped(&self, session_id: SessionId) -> Option<ConnectionId> {
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = self.by_session.get(&session_id) {
            return Some(entry.value().clone());
        }
        if self.retired_sessions.contains_key(&session_id)
            || self.lifecycle_admission_poisoned.load(Ordering::Acquire)
            || self.by_session.len() >= self.active_connection_budget.load(Ordering::Acquire)
        {
            return None;
        }
        let conn_id = ConnectionId::new();
        self.by_session.insert(session_id.clone(), conn_id.clone());
        self.by_connection.insert(conn_id.clone(), session_id);
        Some(conn_id)
    }

    fn reserve_outbound_route(&self, route: Arc<SipOutboundRoute>) -> CoreResult<()> {
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.retired_sessions.contains_key(&route.session_id)
            || self.lifecycle_admission_poisoned.load(Ordering::Acquire)
            || self.by_session.contains_key(&route.session_id)
            || self.by_connection.contains_key(&route.connection_id)
            || self.outbound_routes.contains_key(&route.connection_id)
            || self.streams_cache.contains_key(&route.connection_id)
            || self.by_session.len() >= self.active_connection_budget.load(Ordering::Acquire)
        {
            return Err(RvoipError::AdmissionRejected(
                "SIP outbound lifecycle reservation was unavailable",
            ));
        }
        self.by_session
            .insert(route.session_id.clone(), route.connection_id.clone());
        self.by_connection
            .insert(route.connection_id.clone(), route.session_id.clone());
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
            | AdapterEvent::Authenticated { connection_id, .. }
            | AdapterEvent::PrincipalAuthenticated { connection_id, .. }
            | AdapterEvent::Ended { connection_id, .. }
            | AdapterEvent::Failed { connection_id, .. }
            | AdapterEvent::Dtmf { connection_id, .. }
            | AdapterEvent::Quality { connection_id, .. }
            | AdapterEvent::Message { connection_id, .. }
            | AdapterEvent::DataMessage { connection_id, .. }
            | AdapterEvent::StepUpResponse { connection_id, .. } => Some(connection_id),
            _ => None,
        }
    }

    /// Return `None` when the event was retained by a dormant outbound route,
    /// or the original event when it should use the normal channel.
    fn stage_outbound_event(&self, event: AdapterEvent) -> Option<AdapterEvent> {
        let Some(connection_id) = Self::adapter_event_connection_id(&event).cloned() else {
            return Some(event);
        };
        self.stage_outbound_event_for(&connection_id, event)
    }

    fn stage_outbound_event_for(
        &self,
        connection_id: &ConnectionId,
        event: AdapterEvent,
    ) -> Option<AdapterEvent> {
        let Some(route) = self.outbound_routes.get(connection_id) else {
            return Some(event);
        };
        match route.stage_event(event.clone()) {
            SipRouteStageDisposition::Retained | SipRouteStageDisposition::Discard => None,
            SipRouteStageDisposition::Forward => Some(event),
        }
    }

    /// Configure the maximum number of active SIP mappings and recently
    /// retired Session-ID tombstones. Configuration is accepted only before
    /// the first route is admitted, which keeps admission deterministic.
    pub fn configure_lifecycle_limits(
        &self,
        active_connections: usize,
        retired_sessions: usize,
    ) -> CoreResult<()> {
        if active_connections == 0 || retired_sessions == 0 {
            return Err(RvoipError::InvalidState(
                "SIP lifecycle limits must both be greater than zero",
            ));
        }
        let _mapping = self
            .mapping_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.by_session.is_empty()
            || !self.retired_sessions.is_empty()
            || self.lifecycle_admission_poisoned.load(Ordering::Acquire)
        {
            return Err(RvoipError::InvalidState(
                "SIP lifecycle limits cannot change after route admission",
            ));
        }
        self.active_connection_budget
            .store(active_connections, Ordering::Release);
        self.retired_session_budget
            .store(retired_sessions, Ordering::Release);
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

    fn forget(&self, session_id: &SessionId) {
        let stream = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            self.authenticated_inbound_sessions.remove(session_id);
            let stream = if let Some((_, conn_id)) = self.by_session.remove(session_id) {
                self.by_connection.remove(&conn_id);
                self.outbound_routes.remove(&conn_id);
                self.inbound_contexts.forget(session_id, &conn_id);
                self.streams_cache
                    .remove(&conn_id)
                    .map(|(_, stream)| stream)
            } else {
                self.inbound_contexts.forget_pending(session_id);
                None
            };
            self.retire_session_locked(session_id);
            stream
        };
        if let Some(stream) = stream {
            stream.request_close();
            tokio::spawn(async move {
                let _ = tokio::time::timeout(
                    SIP_RETAINED_TASK_TIMEOUT,
                    (stream as Arc<dyn MediaStream>).close(),
                )
                .await;
            });
        }
    }

    fn retire_outbound_route(&self, route: &Arc<SipOutboundRoute>) {
        let stream = {
            let _mapping = self
                .mapping_lock
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let exact_route = self
                .outbound_routes
                .get(&route.connection_id)
                .is_some_and(|entry| Arc::ptr_eq(entry.value(), route));
            let exact_session = self
                .by_connection
                .get(&route.connection_id)
                .is_some_and(|entry| entry.value() == &route.session_id);
            if !exact_route || !exact_session {
                return;
            }
            self.outbound_routes.remove(&route.connection_id);
            self.by_connection.remove(&route.connection_id);
            self.by_session.remove(&route.session_id);
            self.authenticated_inbound_sessions
                .remove(&route.session_id);
            self.inbound_contexts
                .forget(&route.session_id, &route.connection_id);
            let stream = self
                .streams_cache
                .remove(&route.connection_id)
                .map(|(_, stream)| stream);
            self.retire_session_locked(&route.session_id);
            stream
        };
        if let Some(stream) = stream {
            stream.request_close();
        }
    }

    fn retire_session_locked(&self, session_id: &SessionId) {
        if self.retired_sessions.contains_key(session_id) {
            return;
        }
        let budget = self.retired_session_budget.load(Ordering::Acquire);
        if self.retired_sessions.len() < budget {
            self.retired_sessions.insert(session_id.clone(), ());
        } else {
            self.lifecycle_admission_poisoned
                .store(true, Ordering::Release);
            warn!(
                budget,
                "SIP lifecycle tombstone budget exhausted; rejecting all new Session IDs"
            );
        }
    }

    async fn terminate_failed_inbound(
        &self,
        session_id: &SessionId,
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
        self.forget(session_id);

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
            .map(|e| e.value().clone())
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))
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
        let stream = crate::media_stream::SipMediaStream::dormant(direction);
        self.streams_cache.insert(conn.clone(), Arc::clone(&stream));
        Some(stream)
    }

    fn start_stream_bind(
        &self,
        conn: ConnectionId,
        stream: Arc<crate::media_stream::SipMediaStream>,
    ) {
        let Some(session_id) = self
            .by_connection
            .get(&conn)
            .map(|entry| entry.value().clone())
        else {
            return;
        };
        let coordinator = Arc::clone(&self.coordinator);
        let weak_adapter = self.self_weak.clone();
        tokio::spawn(async move {
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
                    adapter
                        .terminate_failed_media(&conn, &session_id, coordinator)
                        .await;
                }
            }
        });
    }

    async fn terminate_failed_media(
        &self,
        connection_id: &ConnectionId,
        session_id: &SessionId,
        coordinator: Arc<UnifiedCoordinator>,
    ) {
        let still_exact = self
            .by_connection
            .get(connection_id)
            .is_some_and(|entry| entry.value() == session_id);
        if !still_exact || self.outbound_routes.contains_key(connection_id) {
            return;
        }
        self.forget(session_id);
        let hangup =
            tokio::time::timeout(SIP_RETAINED_TASK_TIMEOUT, coordinator.hangup(session_id)).await;
        if !matches!(hangup, Ok(Ok(()))) {
            let _ = tokio::time::timeout(
                SIP_RETAINED_TASK_TIMEOUT,
                coordinator.finalize_local_bye(session_id, "SIP media driver failed"),
            )
            .await;
        }
        self.deliver_terminal_event(
            AdapterEvent::Failed {
                connection_id: connection_id.clone(),
                detail: "SIP media driver failed".to_string(),
            },
            "media-failed",
        )
        .await;
    }

    async fn translate_api_event(&self, event: ApiEvent) {
        match event {
            ApiEvent::IncomingCall { call_id, .. } => {
                let Some(conn_id) = self.ensure_mapped(call_id.clone()) else {
                    warn!("SipAdapter rejected inbound route after lifecycle retirement/capacity");
                    self.terminate_failed_inbound(&call_id, 503, "Connection Capacity Exhausted")
                        .await;
                    return;
                };
                let principal = match self.inbound_contexts.bind(&call_id, &conn_id) {
                    SipInboundBinding::Observed(principal) => principal,
                    SipInboundBinding::Rejected(error) => {
                        warn!(
                            ?error,
                            "SipAdapter rejected invalid authenticated inbound route"
                        );
                        self.terminate_failed_inbound(
                            &call_id,
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
                if !self.send_inbound_event(adapter_event).await {
                    // Keep the consumed tombstone until terminal cleanup so a
                    // replayed IncomingCall cannot recreate the secret.
                    self.inbound_contexts.discard(&conn_id);
                    self.terminate_failed_inbound(&call_id, 503, "Signaling Event Backpressure")
                        .await;
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
                let Some(conn_id) = self.ensure_mapped(call_id) else {
                    return;
                };
                self.try_send(AdapterEvent::Connected {
                    connection_id: conn_id,
                });
            }
            ApiEvent::CallProgress {
                call_id,
                status_code,
                reason,
                ..
            } => {
                let Some(connection_id) = self.ensure_mapped(call_id) else {
                    return;
                };
                self.try_send_for_connection(
                    &connection_id,
                    AdapterEvent::Native {
                        kind: "sip.call_progress",
                        detail: format!("{} {}", status_code, reason),
                    },
                );
            }
            ApiEvent::CallEnded { call_id, reason } => {
                let Some(conn_id) = self.ensure_mapped(call_id.clone()) else {
                    return;
                };
                self.deliver_terminal_for_session(
                    &call_id,
                    AdapterEvent::Ended {
                        connection_id: conn_id,
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
                let Some(conn_id) = self.ensure_mapped(call_id.clone()) else {
                    return;
                };
                self.deliver_terminal_for_session(
                    &call_id,
                    AdapterEvent::Failed {
                        connection_id: conn_id,
                        detail: format!("{} {}", status_code, reason),
                    },
                    "call-failed",
                )
                .await;
            }
            ApiEvent::CallCancelled { call_id } => {
                let Some(conn_id) = self.ensure_mapped(call_id.clone()) else {
                    return;
                };
                self.deliver_terminal_for_session(
                    &call_id,
                    AdapterEvent::Ended {
                        connection_id: conn_id,
                        reason: EndReason::Cancelled,
                    },
                    "call-cancelled",
                )
                .await;
            }
            ApiEvent::DtmfReceived { call_id, digit } => {
                // P12.8 — surface inbound DTMF (RFC 2833 + SIP INFO,
                // decoded by media-core's DTMF detector) as an
                // AdapterEvent the orchestrator translates to
                // Event::DtmfReceived. Duration is the typical RFC
                // 4733 default (100ms) — the underlying ApiEvent
                // doesn't carry per-digit timing.
                let Some(conn_id) = self.ensure_mapped(call_id) else {
                    return;
                };
                self.try_send(AdapterEvent::Dtmf {
                    connection_id: conn_id,
                    digits: digit.to_string(),
                    duration_ms: 100,
                });
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
                let Some(conn_id) = self.ensure_mapped(call_id) else {
                    return;
                };
                self.try_send(AdapterEvent::Quality {
                    connection_id: conn_id,
                    snapshot: rvoip_core::stream::QualitySnapshot {
                        jitter_ms: jitter_ms as f32,
                        packet_loss_pct: packet_loss_percent as f32,
                        mos: None,
                    },
                });
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
        let Some(event) = self.stage_outbound_event(event) else {
            return true;
        };
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

    fn try_send_for_connection(&self, connection_id: &ConnectionId, event: AdapterEvent) -> bool {
        let Some(event) = self.stage_outbound_event_for(connection_id, event) else {
            return true;
        };
        if let Err(error) = self
            .out_tx
            .try_send(OrchestratorAdapterEvent::Public(event))
        {
            warn!(%error, "SipAdapter event channel full or closed");
            return false;
        }
        true
    }

    async fn send_inbound_event(&self, event: OrchestratorAdapterEvent) -> bool {
        Self::send_inbound_event_to(&self.out_tx, event).await
    }

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

    async fn deliver_terminal_for_session(
        &self,
        session_id: &SessionId,
        event: AdapterEvent,
        source: &'static str,
    ) {
        let route = self.by_session.get(session_id).and_then(|connection| {
            self.outbound_routes
                .get(connection.value())
                .map(|route| Arc::clone(route.value()))
        });
        if let Some(route) = route {
            match route.stage_event(event) {
                SipRouteStageDisposition::Retained => {
                    begin_outbound_cleanup(
                        self.self_weak.clone(),
                        Arc::clone(&self.coordinator),
                        route,
                        self.lifecycle.clone(),
                        self.out_tx.clone(),
                        None,
                        source,
                    );
                }
                SipRouteStageDisposition::Forward => {
                    // Outbound terminals are always retained by the route so
                    // compensation and exact-map retirement precede delivery.
                    warn!(
                        source,
                        "SipAdapter outbound terminal escaped retained route"
                    );
                }
                SipRouteStageDisposition::Discard => {}
            }
        } else {
            self.forget(session_id);
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

async fn run_outbound_activation(
    weak_adapter: Weak<SipAdapter>,
    coordinator: Arc<UnifiedCoordinator>,
    route: Arc<SipOutboundRoute>,
    lifecycle: AdapterLifecycleSinkSlot,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
) {
    let result =
        activate_outbound_route(Arc::clone(&coordinator), Arc::clone(&route), events.clone()).await;
    match result {
        Ok((receipt, remote_terminal)) => {
            if !remote_terminal {
                let media_weak_adapter = weak_adapter.clone();
                let media_coordinator = Arc::clone(&coordinator);
                let media_route = Arc::clone(&route);
                let media_lifecycle = lifecycle.clone();
                let media_events = events.clone();
                tokio::spawn(async move {
                    monitor_outbound_media(
                        media_weak_adapter,
                        media_coordinator,
                        media_route,
                        media_lifecycle,
                        media_events,
                    )
                    .await;
                });
            }
            route.complete_activation(SipActivationCompletion::Succeeded(receipt));
            if remote_terminal {
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
            route.complete_activation(SipActivationCompletion::Failed(failure));
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
    coordinator: Arc<UnifiedCoordinator>,
    route: Arc<SipOutboundRoute>,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
) -> Result<(OutboundActivation, bool), SipActivationFailure> {
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

    let external_reference =
        ExternalConnectionReference::new("sip.call-id", Arc::clone(&route.sip_call_id))
            .map_err(|_| SipActivationFailure::InvalidPlan)?;
    let receipt = OutboundActivation::with_external_reference(external_reference);

    let remote_terminal = route.publish_staged_events(&events).await?;
    if remote_terminal {
        return Ok((receipt, true));
    }

    let mut cancel = route.cancel.subscribe();
    let bind = route
        .stream
        .bind(Arc::clone(&coordinator), route.session_id.clone());
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

    Ok((receipt, false))
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
        match *media.borrow_and_update() {
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
    route.complete_activation(SipActivationCompletion::Failed(
        SipActivationFailure::RouteEnded,
    ));
    if !route.request_cleanup(event) {
        return;
    }
    tokio::spawn(async move {
        run_outbound_cleanup(weak_adapter, coordinator, route, lifecycle, events, source).await;
    });
}

async fn run_outbound_cleanup(
    weak_adapter: Weak<SipAdapter>,
    coordinator: Arc<UnifiedCoordinator>,
    route: Arc<SipOutboundRoute>,
    lifecycle: AdapterLifecycleSinkSlot,
    events: mpsc::Sender<OrchestratorAdapterEvent>,
    source: &'static str,
) {
    let (wire, remote_terminal) = route.cleanup_snapshot();
    let mut network_success = true;
    if wire != SipOutboundWireState::NotStarted && !remote_terminal {
        let hangup = tokio::time::timeout(
            SIP_RETAINED_TASK_TIMEOUT,
            coordinator.hangup(&route.session_id),
        )
        .await;
        if !matches!(hangup, Ok(Ok(()))) {
            network_success = matches!(
                tokio::time::timeout(
                    SIP_RETAINED_TASK_TIMEOUT,
                    coordinator
                        .finalize_local_bye(&route.session_id, "SIP outbound retained cleanup",),
                )
                .await,
                Ok(Ok(()))
            );
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
    route.complete_cleanup(network_success && media_success);
}

impl Drop for SipAdapter {
    fn drop(&mut self) {
        let _ = self.translator_cancel.send(true);
        for stream in self.streams_cache.iter() {
            stream.value().request_close();
        }
        self.coordinator
            .remove_inbound_invite_observer(self.inbound_invite_observer_id);
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
        self.by_connection.contains_key(conn)
    }

    fn take_inbound_context(&self, conn: &ConnectionId) -> Option<InboundConnectionContext> {
        self.inbound_contexts.take(conn)
    }

    async fn originate(&self, request: OriginateRequest) -> CoreResult<ConnectionHandle> {
        Self::validate_outbound_target(&request.target)?;
        let originate_context = Self::outbound_originate_context(&request)?;
        let session_id = SessionId::new();
        let conn_id = ConnectionId::new();
        let stream = crate::media_stream::SipMediaStream::dormant(Direction::Outbound);
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
        match route.claim_activation() {
            Ok(true) => {
                let weak_adapter = self.self_weak.clone();
                let coordinator = Arc::clone(&self.coordinator);
                let lifecycle = self.lifecycle.clone();
                let events = self.out_tx.clone();
                let retained_route = Arc::clone(&route);
                tokio::spawn(async move {
                    run_outbound_activation(
                        weak_adapter,
                        coordinator,
                        retained_route,
                        lifecycle,
                        events,
                    )
                    .await;
                });
            }
            Ok(false) => {}
            Err(error) => route.complete_activation(SipActivationCompletion::Failed(error)),
        }
        route.wait_activation().await
    }

    async fn accept(&self, conn: ConnectionId) -> CoreResult<()> {
        if self.outbound_routes.contains_key(&conn) {
            self.activate_outbound(conn.clone()).await?;
        }
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .accept_call(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        let terminal_detail = format!("session rejected locally: {reason:?}");
        if let Some(route) = self
            .outbound_routes
            .get(&conn)
            .map(|entry| Arc::clone(entry.value()))
        {
            route.complete_activation(SipActivationCompletion::Failed(
                SipActivationFailure::RouteEnded,
            ));
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
        self.forget(&session_id);
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
        let session_id = self.lookup_session(&conn)?;
        if let Some(route) = self
            .outbound_routes
            .get(&conn)
            .map(|entry| Arc::clone(entry.value()))
        {
            route.complete_activation(SipActivationCompletion::Failed(
                SipActivationFailure::RouteEnded,
            ));
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
        self.forget(&session_id);
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
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .hold(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn resume(&self, conn: ConnectionId) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .resume(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn transfer(&self, conn: ConnectionId, target: TransferTarget) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        let refer_to = match target {
            TransferTarget::Uri(uri) => uri,
            TransferTarget::Connection(_) | TransferTarget::Session(_) => {
                return Err(RvoipError::NotImplemented(
                    "unsupported beta feature: attended transfer by Connection/Session target is post-beta for SipAdapter",
                ));
            }
        };
        self.coordinator
            .refer(&session_id, refer_to)
            .send()
            .await
            .map_err(Self::map_session_err)
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

    async fn send_dtmf(
        &self,
        conn: ConnectionId,
        digits: &str,
        _duration_ms: u32,
    ) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        // api::send_dtmf takes one digit per call; loop the string.
        for ch in digits.chars() {
            self.coordinator
                .send_dtmf(&session_id, ch)
                .await
                .map_err(Self::map_session_err)?;
        }
        Ok(())
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
        let session_id = self.lookup_session(&conn)?;
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingLifecycleSink {
        deliveries: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl AdapterLifecycleSink for RecordingLifecycleSink {
        async fn deliver_terminal(&self, _event: AdapterEvent) {
            self.deliveries.fetch_add(1, Ordering::SeqCst);
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
        assert!(context.from_uri().is_none());
        assert!(context.auth().is_none());
        assert!(context.initial_headers().is_empty());
        assert_eq!(
            format!("{context:?}"),
            "SipOriginateContext { has_from_uri: false, has_auth: false, initial_header_count: 0 }"
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
            "first-secret",
            "second-secret",
        ] {
            assert!(!debug.contains(secret));
        }
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
        assert!(adapter.retired_sessions.is_empty());
        assert!(coordinator.list_sessions().await.is_empty());

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
    async fn concurrent_activation_sends_one_invite_and_receipt_matches_wire_call_id() {
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
        let (bytes, _) =
            tokio::time::timeout(Duration::from_secs(5), capture.recv_from(&mut packet))
                .await
                .expect("INVITE deadline")
                .expect("INVITE packet");
        let invite = std::str::from_utf8(&packet[..bytes]).expect("SIP text");
        assert!(invite.starts_with("INVITE "));
        let wire_call_id = invite
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("Call-ID")
                    .then(|| value.trim().to_string())
            })
            .expect("wire Call-ID");

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
        assert!(
            tokio::time::timeout(Duration::from_millis(150), capture.recv_from(&mut packet))
                .await
                .is_err(),
            "concurrent activation must not emit a second INVITE"
        );

        ConnectionAdapter::end(adapter.as_ref(), connection_id, EndReason::Cancelled)
            .await
            .expect("retained cleanup");
        drop(adapter);
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
            let connection_id = adapter
                .ensure_mapped(session_id.clone())
                .expect("admit mapping");
            let gate = Arc::new(tokio::sync::Barrier::new(3));
            let insert_adapter = Arc::clone(&adapter);
            let insert_connection = connection_id.clone();
            let insert_gate = Arc::clone(&gate);
            let insert = tokio::spawn(async move {
                insert_gate.wait().await;
                insert_adapter.get_or_insert_dormant_stream(&insert_connection, Direction::Inbound)
            });
            let retire_adapter = Arc::clone(&adapter);
            let retire_session = session_id.clone();
            let retire_gate = Arc::clone(&gate);
            let retire = tokio::spawn(async move {
                retire_gate.wait().await;
                retire_adapter.forget(&retire_session);
            });
            gate.wait().await;
            let _ = insert.await.expect("insert task");
            retire.await.expect("retire task");

            assert!(!adapter.by_session.contains_key(&session_id));
            assert!(!adapter.by_connection.contains_key(&connection_id));
            assert!(!adapter.streams_cache.contains_key(&connection_id));
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
        let connection_id = adapter
            .ensure_mapped(session_id.clone())
            .expect("admit mapping");
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

        adapter.forget(&session_id);
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
        let connection_id = adapter
            .ensure_mapped(session_id.clone())
            .expect("admit mapping");
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
    async fn failed_atomic_delivery_tombstone_suppresses_late_principal_event() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("atomic-failure", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        let mut events = ConnectionAdapter::subscribe_events(adapter.as_ref());
        let session_id = SessionId::new();
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
        adapter.forget(&session_id);
        assert!(!adapter
            .authenticated_inbound_sessions
            .contains_key(&session_id));

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

        let connection_id = adapter
            .ensure_mapped(session_id.clone())
            .expect("test route mapping");
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
            .terminate_failed_inbound(&session_id, 503, "test saturated publication")
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
    async fn tombstone_saturation_fails_closed_and_late_events_never_remap() {
        let coordinator = UnifiedCoordinator::new(ApiConfig::local("tombstone-saturation", 0))
            .await
            .expect("coordinator");
        let adapter = SipAdapter::new(Arc::clone(&coordinator))
            .await
            .expect("adapter");
        adapter
            .configure_lifecycle_limits(2, 1)
            .expect("configure before admission");
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");

        let first = SessionId::new();
        let first_connection = adapter.ensure_mapped(first.clone()).expect("first mapping");
        adapter.forget(&first);
        assert!(adapter.retired_sessions.contains_key(&first));

        let saturated = SessionId::new();
        let saturated_connection = adapter
            .ensure_mapped(saturated.clone())
            .expect("mapping before tombstone saturation");
        adapter.forget(&saturated);
        assert!(adapter.lifecycle_admission_poisoned.load(Ordering::Acquire));
        assert!(!adapter.retired_sessions.contains_key(&saturated));

        assert!(adapter.ensure_mapped(first.clone()).is_none());
        assert!(adapter.ensure_mapped(saturated.clone()).is_none());
        assert!(adapter.ensure_mapped(SessionId::new()).is_none());
        assert!(!adapter.by_connection.contains_key(&first_connection));
        assert!(!adapter.by_connection.contains_key(&saturated_connection));

        adapter
            .translate_api_event(ApiEvent::CallAnswered {
                call_id: saturated,
                sdp: None,
            })
            .await;
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
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
            crate::media_stream::SipMediaStream::dormant(Direction::Outbound),
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
        tokio::task::yield_now().await;
        assert!(!flush.is_finished(), "second staged event awaits capacity");

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
            crate::media_stream::SipMediaStream::dormant(Direction::Outbound),
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
            crate::media_stream::SipMediaStream::dormant(Direction::Outbound),
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
        route.complete_activation(SipActivationCompletion::Succeeded(
            OutboundActivation::with_external_reference(reference),
        ));
        for waiter in waiters {
            assert!(waiter.await.expect("activation waiter").is_ok());
        }
    }

    #[tokio::test]
    async fn fast_terminal_is_separate_from_public_fifo_and_survives_overflow() {
        let connection_id = ConnectionId::new();
        let route = SipOutboundRoute::new(
            connection_id.clone(),
            SessionId::new(),
            "sip:target@example.test".to_string(),
            Arc::new(SipOriginateContext::default()),
            crate::media_stream::SipMediaStream::dormant(Direction::Outbound),
        );
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
        assert_eq!(
            route.stage_event(AdapterEvent::Ended {
                connection_id: connection_id.clone(),
                reason: EndReason::Normal,
            }),
            SipRouteStageDisposition::Retained
        );
        assert!(matches!(
            route.claim_activation(),
            Err(SipActivationFailure::EventOverflow)
        ));
        route.request_cleanup(None);
        assert!(matches!(
            route.seal_cleanup_event(),
            Some(AdapterEvent::Ended { connection_id: id, .. }) if id == connection_id
        ));
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

        drop(adapter);
        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown");
    }
}
