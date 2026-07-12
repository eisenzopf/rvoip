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
use crate::api::unified::{Config as ApiConfig, InboundInviteObservation, UnifiedCoordinator};
use crate::SessionId;
use chrono::Utc;
use dashmap::DashMap;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, AdapterLifecycleSink, AdapterLifecycleSinkSlot, ConnectionAdapter,
    ConnectionHandle, EndReason, InboundConnectionContext, InboundContextError, InboundRoutingHint,
    InboundSignalingMetadata, OriginateRequest, RejectReason, SignatureHeaders, TerminalDelivery,
    TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as CoreResult, RvoipError};
use rvoip_core::identity::{AuthenticatedPrincipal, IdentityAssurance};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId as CoreSessionId};
use rvoip_core::message::Message;
use rvoip_core::stream::{MediaStream, MediaStreamHandle};
use rvoip_sip_core::types::headers::{HeaderName, TypedHeader};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

const MAX_SIP_INBOUND_ALLOWLIST_HEADERS: usize = 32;
const MAX_PENDING_SIP_INBOUND_CONTEXTS: usize = 4_096;
const PENDING_SIP_INBOUND_CONTEXT_TTL: Duration = Duration::from_secs(120);
const PENDING_SIP_INBOUND_CONTEXT_REAPER_INTERVAL: Duration = Duration::from_secs(1);
const SIP_INBOUND_EVENT_DELIVERY_TIMEOUT: Duration = Duration::from_secs(2);

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
        let Some(request) = observation.request.as_ref() else {
            return Ok(None);
        };

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
        let metadata = InboundSignalingMetadata::new(metadata)?;

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
    normalized.starts_with("x-")
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

struct PendingSipInboundContext {
    routing_hint: Option<InboundRoutingHint>,
    metadata: InboundSignalingMetadata,
}

struct PendingSipInboundObservation {
    observed_at: Instant,
    principal: Option<AuthenticatedPrincipal>,
    context: Option<Box<PendingSipInboundContext>>,
}

enum SipInboundContextState {
    Available(InboundConnectionContext),
    Consumed,
}

enum SipInboundBinding {
    Observed(Option<AuthenticatedPrincipal>),
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
        let principal = pending.principal.clone();

        self.by_connection
            .entry(connection_id.clone())
            .or_insert_with(|| match pending {
                PendingSipInboundObservation {
                    principal: Some(principal),
                    context: Some(pending),
                    ..
                } => {
                    let pending = *pending;
                    match InboundConnectionContext::new(
                        connection_id.clone(),
                        Transport::Sip,
                        &principal,
                        pending.routing_hint,
                        pending.metadata,
                    ) {
                        Ok(context) => SipInboundContextState::Available(context),
                        Err(error) => {
                            warn!(
                                ?connection_id,
                                ?error,
                                "SipAdapter rejected invalid inbound connection context"
                            );
                            SipInboundContextState::Consumed
                        }
                    }
                }
                _ => SipInboundContextState::Consumed,
            });
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
    coordinator: Arc<UnifiedCoordinator>,
    /// rvoip-core ConnectionId → SIP api SessionId.
    by_connection: Arc<DashMap<ConnectionId, SessionId>>,
    /// SIP api SessionId → rvoip-core ConnectionId. Used by the event
    /// translator task to map outgoing api::Event → AdapterEvent.
    by_session: Arc<DashMap<SessionId, ConnectionId>>,
    out_tx: mpsc::Sender<AdapterEvent>,
    /// Single-take receiver for [`ConnectionAdapter::subscribe_events`].
    out_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
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
        let (out_tx, out_rx) = mpsc::channel(256);
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
                let context = match policy.capture(&observation) {
                    Ok(context) => context,
                    Err(error) => {
                        warn!(
                            ?error,
                            "SipAdapter rejected malformed inbound signaling context"
                        );
                        None
                    }
                };
                // First observation wins. Retransmitted/replayed INVITE
                // notifications cannot replace a context already tied to the
                // session's authenticated request.
                if !contexts.observe(session_id, principal, context) {
                    warn!("SipAdapter inbound context admission capacity reached");
                }
            },
        ))?;
        let adapter = Arc::new(Self {
            coordinator: Arc::clone(&coordinator),
            by_connection: Arc::new(DashMap::new()),
            by_session: Arc::new(DashMap::new()),
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

    fn ensure_mapped(&self, session_id: SessionId) -> ConnectionId {
        if let Some(entry) = self.by_session.get(&session_id) {
            return entry.value().clone();
        }
        let conn_id = ConnectionId::new();
        self.by_session.insert(session_id.clone(), conn_id.clone());
        self.by_connection.insert(conn_id.clone(), session_id);
        conn_id
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
        self.authenticated_inbound_sessions.remove(session_id);
        if let Some((_, conn_id)) = self.by_session.remove(session_id) {
            self.by_connection.remove(&conn_id);
            self.streams_cache.remove(&conn_id);
            self.inbound_contexts.forget(session_id, &conn_id);
        } else {
            self.inbound_contexts.forget_pending(session_id);
        }
    }

    fn lookup_session(&self, conn: &ConnectionId) -> CoreResult<SessionId> {
        self.by_connection
            .get(conn)
            .map(|e| e.value().clone())
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))
    }

    async fn build_connection(&self, conn_id: ConnectionId, direction: Direction) -> Connection {
        // Eagerly construct (and cache) one SipMediaStream so consumers
        // can read `connection.streams` synchronously off the
        // `Event::ConnectionInbound` event — QUIC/WT parity, gap plan §2.2.
        // Stream construction can fail (e.g. coordinator is shutting
        // down or the session was torn down before we got here); in
        // that case we still hand back a `Connection` with an empty
        // streams vec — that's no worse than the pre-eager behavior.
        let streams = match self.get_or_init_stream(&conn_id, direction).await {
            Some(stream) => vec![MediaStreamHandle::new(stream as Arc<dyn MediaStream>)],
            None => Vec::new(),
        };
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

    /// Look up the cached `SipMediaStream` for `conn`, constructing it
    /// (and caching) on first call. Returns `None` if construction
    /// fails — the connection-mapping or audio subscribe may not be
    /// ready yet (e.g. the session was cleaned up between events).
    async fn get_or_init_stream(
        &self,
        conn: &ConnectionId,
        direction: Direction,
    ) -> Option<Arc<crate::media_stream::SipMediaStream>> {
        if let Some(entry) = self.streams_cache.get(conn) {
            return Some(Arc::clone(entry.value()));
        }
        let session_id = self.by_connection.get(conn)?.value().clone();
        match crate::media_stream::SipMediaStream::new(
            Arc::clone(&self.coordinator),
            session_id,
            direction,
        )
        .await
        {
            Ok(stream) => {
                self.streams_cache.insert(conn.clone(), Arc::clone(&stream));
                Some(stream)
            }
            Err(e) => {
                warn!(?conn, error = %e, "SipAdapter: failed to construct SipMediaStream eagerly");
                None
            }
        }
    }

    async fn translate_api_event(&self, event: ApiEvent) {
        match event {
            ApiEvent::IncomingCall { call_id, .. } => {
                let conn_id = self.ensure_mapped(call_id.clone());
                let principal = match self.inbound_contexts.bind(&call_id, &conn_id) {
                    SipInboundBinding::Observed(principal) => principal,
                    SipInboundBinding::Missing => {
                        // Every new inbound call is synchronously observed
                        // before its public event is emitted. A missing entry
                        // therefore means admission overflow, expiry, or a
                        // startup race; fail closed instead of publishing a
                        // route without its authentication state.
                        warn!("SipAdapter rejected inbound route without its observation");
                        if let Err(error) = self
                            .coordinator
                            .reject(&call_id)
                            .with_status(503)
                            .with_reason("Signaling Context Unavailable")
                            .send()
                            .await
                        {
                            warn!(%error, "SipAdapter failed to reject contextless inbound route");
                        }
                        return;
                    }
                };
                let connection = self
                    .build_connection(conn_id.clone(), Direction::Inbound)
                    .await;
                let adapter_event = if let Some(principal) = principal {
                    self.authenticated_inbound_sessions
                        .insert(call_id.clone(), ());
                    AdapterEvent::AuthenticatedInboundConnection {
                        participant_id: principal.subject.clone(),
                        connection,
                        principal,
                    }
                } else {
                    AdapterEvent::InboundConnection { connection }
                };
                if !self.send_inbound_event(adapter_event).await {
                    // Keep the consumed tombstone until terminal cleanup so a
                    // replayed IncomingCall cannot recreate the secret.
                    self.inbound_contexts.discard(&conn_id);
                    if let Err(error) = self
                        .coordinator
                        .reject(&call_id)
                        .with_status(503)
                        .with_reason("Signaling Event Backpressure")
                        .send()
                        .await
                    {
                        warn!(%error, "SipAdapter failed to reject undeliverable inbound route");
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
                    subject = %principal.subject,
                    "SipAdapter suppressed unmatched inbound authentication event"
                );
            }
            ApiEvent::CallAnswered { call_id, .. } => {
                let conn_id = self.ensure_mapped(call_id);
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
                let _conn_id = self.ensure_mapped(call_id);
                self.try_send(AdapterEvent::Native {
                    kind: "sip.call_progress",
                    detail: format!("{} {}", status_code, reason),
                });
            }
            ApiEvent::CallEnded { call_id, reason } => {
                let conn_id = self.ensure_mapped(call_id.clone());
                self.forget(&call_id);
                self.deliver_terminal_event(
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
                let conn_id = self.ensure_mapped(call_id.clone());
                self.forget(&call_id);
                self.deliver_terminal_event(
                    AdapterEvent::Failed {
                        connection_id: conn_id,
                        detail: format!("{} {}", status_code, reason),
                    },
                    "call-failed",
                )
                .await;
            }
            ApiEvent::CallCancelled { call_id } => {
                let conn_id = self.ensure_mapped(call_id.clone());
                self.forget(&call_id);
                self.deliver_terminal_event(
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
                let conn_id = self.ensure_mapped(call_id);
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
                let conn_id = self.ensure_mapped(call_id);
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
        if let Err(e) = self.out_tx.try_send(event) {
            warn!(
                ?e,
                "SipAdapter event channel full or closed; dropping event"
            );
            false
        } else {
            true
        }
    }

    async fn send_inbound_event(&self, event: AdapterEvent) -> bool {
        Self::send_inbound_event_to(&self.out_tx, event).await
    }

    async fn send_inbound_event_to(
        events: &mpsc::Sender<AdapterEvent>,
        event: AdapterEvent,
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

    async fn deliver_terminal_event_to(
        lifecycle: &AdapterLifecycleSinkSlot,
        events: &mpsc::Sender<AdapterEvent>,
        event: AdapterEvent,
        source: &'static str,
    ) {
        let delivery = lifecycle.queue_or_deliver_terminal(events, event).await;
        if delivery == TerminalDelivery::Undeliverable {
            warn!(source, "SipAdapter terminal event was undeliverable");
        }
    }

    fn map_session_err(err: crate::errors::SessionError) -> RvoipError {
        RvoipError::Adapter(format!("rvoip-sip: {}", err))
    }
}

impl Drop for SipAdapter {
    fn drop(&mut self) {
        let _ = self.translator_cancel.send(true);
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
        // The OriginateRequest's `target` is the SIP URI to dial; without an
        // explicit `from` we synthesize a local AOR. Step-7 keeps this simple;
        // step 9 wires real auth/PAI when orchestration-core flows through.
        let from = "sip:anonymous@invalid";
        let session_id = self
            .coordinator
            .invite(Some(from.to_string()), request.target.clone())
            .send()
            .await
            .map_err(Self::map_session_err)?;
        let conn_id = self.ensure_mapped(session_id);
        let mut connection = self.build_connection(conn_id, Direction::Outbound).await;
        // Carry the caller-supplied vocabulary IDs through so the consumer's
        // session/participant stay coherent.
        connection.session_id = request.session_id;
        connection.participant_id = request.participant_id;
        connection.capabilities = request.capabilities;
        Ok(ConnectionHandle { connection })
    }

    async fn accept(&self, conn: ConnectionId) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .accept_call(&session_id)
            .await
            .map_err(Self::map_session_err)
    }

    async fn reject(&self, conn: ConnectionId, reason: RejectReason) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        let (status, phrase) = match reason {
            RejectReason::Busy => (486, "Busy Here"),
            RejectReason::Decline => (603, "Decline"),
            RejectReason::NotFound => (404, "Not Found"),
            RejectReason::Forbidden => (403, "Forbidden"),
            RejectReason::NotAcceptable => (488, "Not Acceptable Here"),
            RejectReason::ServerError => (500, "Server Internal Error"),
            RejectReason::Custom { code, ref phrase } => (code, phrase.as_str()),
        };
        self.coordinator
            .reject(&session_id)
            .with_status(status)
            .with_reason(phrase)
            .send()
            .await
            .map_err(Self::map_session_err)
    }

    async fn end(&self, conn: ConnectionId, _reason: EndReason) -> CoreResult<()> {
        let session_id = self.lookup_session(&conn)?;
        self.coordinator
            .hangup(&session_id)
            .await
            .map_err(Self::map_session_err)
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
        // Streams are eagerly populated in `build_connection` (gap plan
        // §2.2 — QUIC/WT parity), so this is a straight lookup. We still
        // construct on demand if the eager path failed earlier (e.g.
        // `subscribe_to_audio` errored at IncomingCall time), so the
        // existing lazy-create semantics remain a fallback.
        self.lookup_session(&conn)?;
        match self.get_or_init_stream(&conn, Direction::Outbound).await {
            Some(stream) => Ok(vec![stream as Arc<dyn MediaStream>]),
            None => Err(RvoipError::Adapter(
                "SipAdapter::streams: SipMediaStream construction failed".into(),
            )),
        }
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
        self.out_rx
            .lock()
            .unwrap()
            .take()
            .expect("SipAdapter::subscribe_events already consumed")
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
        let mut headers = format!(
            "Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-test\r\n\
             From: <sip:caller@example.test>;tag=from-tag\r\n\
             To: <sip:bridge@example.test>\r\n\
             Call-ID: inbound-context@example.test\r\n\
             CSeq: 1 INVITE\r\n\
             Max-Forwards: 70\r\n"
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
            "X-Rvoip-Route",
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
            .send(AdapterEvent::Native {
                kind: "queue-filler",
                detail: String::new(),
            })
            .await
            .unwrap();
        let connection_id = ConnectionId::new();
        let event = AdapterEvent::AuthenticatedInboundConnection {
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
            Some(AdapterEvent::Native { .. })
        ));
        assert!(matches!(
            events_rx.recv().await,
            Some(AdapterEvent::AuthenticatedInboundConnection { connection, principal, .. })
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
            .send(AdapterEvent::Native {
                kind: "queue-filler",
                detail: String::new(),
            })
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
            Some(AdapterEvent::Native { .. })
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
}
