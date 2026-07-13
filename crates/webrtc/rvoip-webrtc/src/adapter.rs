//! `WebRtcAdapter` — `rvoip_core::ConnectionAdapter` for WebRTC interop.

use std::collections::{HashSet, VecDeque};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::Utc;
use dashmap::{mapref::entry::Entry, DashMap};
use parking_lot::{Mutex as SyncMutex, RwLock as SyncRwLock};
use rvoip_core::adapter::{
    legacy_normalized_event_receiver, AdapterEvent, AdapterKind, AdapterLifecycleCapabilities,
    AdapterLifecycleSink, AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    InboundConnectionContext, InboundContextError, InboundRoutingHint, InboundSignalingMetadata,
    OrchestratorAdapterEvent, OriginateRequest, RejectReason, SignatureHeaders, TerminalDelivery,
    TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, IdentityAssurance, PrincipalOwnershipKey,
};
use rvoip_core::ids::{ConnectionId, StreamId};
use rvoip_core::message::{ContentType, Message};
use rvoip_core::stream::MediaStream;
use rvoip_core::{DataMessage, DataReliability};
use rvoip_sip_core::types::sdp::SdpSession;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::{mpsc, watch, Mutex as AsyncMutex, Notify};
use tracing::{debug, info, instrument, warn};
use webrtc::data_channel::{DataChannel, DataChannelEvent, RTCDataChannelState};

use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};
use crate::media::{from_tracks_with_dtmf_events, WebRtcMediaStream};
use crate::peer::{PeerRole, RvoipPeerConnection};
use crate::sdp::{negotiate_audio, sdp_to_string};

pub const ADAPTER_EVENT_CAP: usize = 256;

const OUTBOUND_MESSAGE_CHANNEL_LABEL: &str = "rvoip-messages";
const MAX_DATA_CHANNELS_PER_ROUTE: usize = 64;
const DATA_CHANNEL_SCAN_INTERVAL: Duration = Duration::from_millis(25);
const DATA_CHANNEL_OPERATION_TIMEOUT: Duration = Duration::from_secs(2);
const INBOUND_EVENT_DELIVERY_TIMEOUT: Duration = Duration::from_secs(2);
const OUTBOUND_EVENT_STAGE_CAPACITY: usize = 64;

/// Upper bound for the optional core admission-confirmation wait.
///
/// A bounded timeout keeps a missing admission gate or stalled policy worker
/// from retaining a provisional peer indefinitely. Deployments normally use
/// a value at or below their call-setup deadline.
pub const MAX_INBOUND_ADMISSION_CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Background reaper poll interval.
const REAPER_TICK: Duration = Duration::from_secs(30);

/// Snapshot of operational metrics exposed by [`WebRtcAdapter::metrics`].
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct WebRtcMetrics {
    pub inbound_total: u64,
    pub outbound_total: u64,
    pub active_sessions: usize,
    pub signaling_errors_total: u64,
    pub sessions_rejected_over_cap: u64,
    pub reaped_total: u64,
    pub data_messages_dropped_total: u64,
}

/// Typed `TransportHandle` carrying the originating connection id and a weak
/// pointer to the adapter route table so orchestrators can introspect / clean
/// up without holding a strong reference.
pub struct WebRtcTransportHandle {
    pub connection_id: ConnectionId,
    routes: std::sync::Weak<DashMap<ConnectionId, Route>>,
    cancel: Arc<Notify>,
    data_cancel: watch::Sender<bool>,
}

impl WebRtcTransportHandle {
    pub fn cancel(&self) {
        self.cancel.notify_waiters();
        let _ = self.data_cancel.send(true);
    }

    pub fn route_exists(&self) -> bool {
        self.routes
            .upgrade()
            .map(|r| r.contains_key(&self.connection_id))
            .unwrap_or(false)
    }
}

#[derive(Clone)]
#[non_exhaustive]
pub struct Route {
    pub peer: Arc<RvoipPeerConnection>,
    pub streams: Arc<DashMap<StreamId, Arc<WebRtcMediaStream>>>,
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    /// Outbound/reusable channels keyed by exact label + RFC 8832 reliability.
    pub data_channel: Arc<DashMap<String, Arc<dyn DataChannel>>>,
    data_channel_create: Arc<AsyncMutex<()>>,
    data_channels_pumped: Arc<SyncMutex<HashSet<usize>>>,
    data_channel_keys: Arc<DashMap<usize, String>>,
    data_pump_started: Arc<AtomicBool>,
    data_cancel: watch::Sender<bool>,
    pub negotiated: NegotiatedCodecs,
    pub held: bool,
    /// Notify all per-route background tasks (track attacher, fail watcher, stats) to exit.
    pub cancel: Arc<Notify>,
    /// Set by the fail watcher when the underlying PC enters `Failed`/`Closed`.
    pub failed_at: Arc<SyncMutex<Option<Instant>>>,
    /// True only after the inbound atomic handoff or outbound activation has
    /// crossed the core publication boundary. Pre-publication peer failures
    /// are cleaned locally and are never emitted as unknown terminal events.
    core_published: Arc<AtomicBool>,
    /// True once a secure inbound lifecycle has been handed to core. Unlike
    /// `core_published`, this may be true while policy admission is pending;
    /// it makes a concurrent local failure visible to core so a queued
    /// lifecycle cannot later become an orphan.
    core_handoff_started: Arc<AtomicBool>,
    /// Signaling identity that owns this network-visible route. Keeping the
    /// authorization record on the route makes ownership transport-neutral:
    /// WHIP, WHEP, WS and WSS all consult the same boundary.
    authorization: Option<RouteAuthorization>,
    /// Single-use inbound routing context. Cloned `Route` handles share this
    /// slot; terminal route removal drops an untaken value.
    inbound_context: Arc<SyncMutex<Option<InboundConnectionContext>>>,
    /// Present only for secure inbound routes. The waiter is created before
    /// the authenticated inbound event can be published.
    inbound_admission_waiter: Option<Arc<InboundAdmissionWaiter>>,
}

impl Route {
    fn cancel_tasks(&self) {
        if let Some(waiter) = &self.inbound_admission_waiter {
            waiter.cancel();
        }
        self.cancel.notify_waiters();
        let _ = self.data_cancel.send(true);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InboundAdmissionOutcome {
    Pending,
    Accepted,
    Rejected,
    Cancelled,
}

#[derive(Debug)]
struct InboundAdmissionWaiterState {
    generation: Option<u64>,
    accepted: Option<bool>,
    cancelled: bool,
}

/// Synchronous adapter-side endpoint for the core admission callback.
///
/// The mutex covers only three scalar fields and a nonblocking watch update;
/// the core event loop never waits on signaling, peer I/O, or an async lock.
struct InboundAdmissionWaiter {
    state: StdMutex<InboundAdmissionWaiterState>,
    updates: watch::Sender<InboundAdmissionOutcome>,
}

impl InboundAdmissionWaiter {
    fn new() -> Arc<Self> {
        let (updates, _) = watch::channel(InboundAdmissionOutcome::Pending);
        Arc::new(Self {
            state: StdMutex::new(InboundAdmissionWaiterState {
                generation: None,
                accepted: None,
                cancelled: false,
            }),
            updates,
        })
    }

    fn resolve(&self, lifecycle_generation: u64, accepted: bool, on_first_accept: impl FnOnce()) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.cancelled {
            return;
        }
        match (state.generation, state.accepted) {
            (None, None) => {
                state.generation = Some(lifecycle_generation);
                state.accepted = Some(accepted);
                if accepted {
                    on_first_accept();
                }
                self.updates.send_replace(if accepted {
                    InboundAdmissionOutcome::Accepted
                } else {
                    InboundAdmissionOutcome::Rejected
                });
            }
            (Some(generation), Some(previous))
                if generation == lifecycle_generation && previous == accepted =>
            {
                // Exact duplicate: idempotent by contract.
            }
            _ => {
                // A stale generation or contradictory duplicate must never
                // mutate the outcome of the current route.
            }
        }
    }

    fn cancel(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.cancelled {
            return;
        }
        state.cancelled = true;
        self.updates
            .send_replace(InboundAdmissionOutcome::Cancelled);
    }

    fn is_accepted_and_live(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        !state.cancelled && state.accepted == Some(true)
    }

    async fn wait(&self, timeout: Duration) -> InboundAdmissionOutcome {
        let mut updates = self.updates.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let outcome = *updates.borrow_and_update();
            if outcome != InboundAdmissionOutcome::Pending {
                return outcome;
            }
            match tokio::time::timeout_at(deadline, updates.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return InboundAdmissionOutcome::Cancelled,
                Err(_) => return InboundAdmissionOutcome::Pending,
            }
        }
    }
}

enum OutboundEventStageState {
    Dormant {
        events: VecDeque<AdapterEvent>,
        overflowed: bool,
    },
    Activated,
}

struct OutboundEventStage {
    state: StdMutex<OutboundEventStageState>,
}

impl Default for OutboundEventStage {
    fn default() -> Self {
        Self {
            state: StdMutex::new(OutboundEventStageState::Dormant {
                events: VecDeque::with_capacity(OUTBOUND_EVENT_STAGE_CAPACITY),
                overflowed: false,
            }),
        }
    }
}

/// Adapter-owned authorization key for network signaling routes.
///
/// Complete principals are compared with [`PrincipalOwnershipKey`]. Legacy
/// hooks which predate `AuthenticatedPrincipal` retain subject isolation, and
/// the anonymous variant preserves the crate's authentication-disabled mode.
#[derive(Clone, Eq, PartialEq)]
pub(crate) enum RouteOwnerKey {
    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    Anonymous,
    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    LegacySubject(String),
    Principal(PrincipalOwnershipKey),
}

impl std::fmt::Debug for RouteOwnerKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
            Self::Anonymous => formatter.write_str("Anonymous"),
            #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
            Self::LegacySubject(subject) => formatter
                .debug_struct("LegacySubject")
                .field("subject_present", &!subject.is_empty())
                .finish(),
            Self::Principal(_) => formatter.write_str("Principal"),
        }
    }
}

#[derive(Clone)]
pub(crate) struct RouteAuthorization {
    owner: RouteOwnerKey,
    principal: Option<AuthenticatedPrincipal>,
}

impl std::fmt::Debug for RouteAuthorization {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RouteAuthorization")
            .field("owner", &self.owner)
            .field("principal_present", &self.principal.is_some())
            .finish()
    }
}

impl RouteAuthorization {
    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    pub(crate) fn anonymous() -> Self {
        Self {
            owner: RouteOwnerKey::Anonymous,
            principal: None,
        }
    }

    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    pub(crate) fn legacy_subject(subject: impl Into<String>) -> Self {
        Self {
            owner: RouteOwnerKey::LegacySubject(subject.into()),
            principal: None,
        }
    }

    pub(crate) fn principal(principal: AuthenticatedPrincipal) -> Self {
        Self {
            owner: RouteOwnerKey::Principal(principal.ownership_key()),
            principal: Some(principal),
        }
    }

    fn ensure_active(&self) -> Result<()> {
        if self
            .principal
            .as_ref()
            .is_some_and(AuthenticatedPrincipal::is_expired)
        {
            return Err(WebRtcError::Unauthorized(
                "authenticated principal has expired".into(),
            ));
        }
        Ok(())
    }
}

/// D2 — per-route DTLS fingerprint pinning policy.
///
/// Implementations return the set of fingerprints allowed for a given
/// session. The adapter takes the **union** of this list with
/// [`WebRtcConfig::pinned_fingerprints`](crate::config::WebRtcConfig::pinned_fingerprints)
/// and, if the union is non-empty, rejects any peer whose negotiated
/// fingerprint isn't in the union with
/// [`WebRtcError::FingerprintNotPinned`].
///
/// `session_hint` is a free-form identifier the caller can use to scope
/// pinning per tenant / per call (e.g. a WHIP `session_id` or a UCTP
/// request id). Pass `None` when no hint is available.
#[async_trait]
pub trait FingerprintPolicyHook: Send + Sync {
    async fn allowed_fingerprints(
        &self,
        conn: &ConnectionId,
        session_hint: Option<&str>,
    ) -> Vec<crate::identity::DtlsFingerprint>;
}

pub struct WebRtcAdapter {
    config: WebRtcConfig,
    routes: Arc<DashMap<ConnectionId, Route>>,
    events_tx: mpsc::Sender<OrchestratorAdapterEvent>,
    events_rx: StdMutex<Option<mpsc::Receiver<OrchestratorAdapterEvent>>>,
    /// Per-outbound-route FIFO. Operational and terminal events remain
    /// dormant until the orchestrator commits the returned Connection.
    outbound_event_stages: Arc<DashMap<ConnectionId, Arc<OutboundEventStage>>>,
    lifecycle: AdapterLifecycleSinkSlot,
    /// Cancel for the global reaper task spawned in [`WebRtcAdapter::new`].
    reaper_cancel: Arc<Notify>,
    metrics_inbound: Arc<AtomicU64>,
    metrics_outbound: Arc<AtomicU64>,
    metrics_errors: Arc<AtomicU64>,
    metrics_rejected: Arc<AtomicU64>,
    metrics_reaped: Arc<AtomicU64>,
    metrics_data_dropped: Arc<AtomicU64>,
    /// Live session count incremented before any per-session work and
    /// decremented on route removal. Replaces `routes.len()` for cap checks
    /// so concurrent originate/apply_remote_offer can't race past the cap.
    live_sessions: Arc<std::sync::atomic::AtomicUsize>,
    /// D2 — optional per-route fingerprint pinning hook. Set via
    /// [`WebRtcAdapter::set_fingerprint_policy`]; `None` means "use only
    /// the static `WebRtcConfig::pinned_fingerprints` list".
    fingerprint_policy: SyncRwLock<Option<Arc<dyn FingerprintPolicyHook>>>,
    /// Opt-in fail-closed wait for the orchestrator's durable inbound policy
    /// decision. `None` preserves the historical direct-adapter behavior.
    inbound_admission_confirmation_timeout: Option<Duration>,
}

impl WebRtcAdapter {
    pub fn new(config: WebRtcConfig) -> Arc<Self> {
        Self::new_inner(config, None)
    }

    /// Construct an adapter that withholds inbound protocol success until the
    /// orchestrator confirms durable admission.
    pub fn new_with_inbound_admission_confirmation(
        config: WebRtcConfig,
        timeout: Duration,
    ) -> Result<Arc<Self>> {
        Self::validate_inbound_admission_confirmation_timeout(timeout)?;
        Ok(Self::new_inner(config, Some(timeout)))
    }

    fn new_inner(
        config: WebRtcConfig,
        inbound_admission_confirmation_timeout: Option<Duration>,
    ) -> Arc<Self> {
        let (events_tx, events_rx) = mpsc::channel(ADAPTER_EVENT_CAP);
        let reaper_cancel = Arc::new(Notify::new());
        let metrics_reaped = Arc::new(AtomicU64::new(0));
        let adapter = Arc::new(Self {
            config,
            routes: Arc::new(DashMap::new()),
            events_tx,
            events_rx: StdMutex::new(Some(events_rx)),
            outbound_event_stages: Arc::new(DashMap::new()),
            lifecycle: AdapterLifecycleSinkSlot::default(),
            reaper_cancel: Arc::clone(&reaper_cancel),
            metrics_inbound: Arc::new(AtomicU64::new(0)),
            metrics_outbound: Arc::new(AtomicU64::new(0)),
            metrics_errors: Arc::new(AtomicU64::new(0)),
            metrics_rejected: Arc::new(AtomicU64::new(0)),
            metrics_reaped: Arc::clone(&metrics_reaped),
            metrics_data_dropped: Arc::new(AtomicU64::new(0)),
            live_sessions: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            fingerprint_policy: SyncRwLock::new(None),
            inbound_admission_confirmation_timeout,
        });

        // Spawn session reaper (idempotent: TTL=0 disables in-loop work).
        let ttl_secs = adapter.config.session_idle_ttl_secs;
        if ttl_secs > 0 {
            let routes = Arc::clone(&adapter.routes);
            let events_tx = adapter.events_tx.clone();
            let outbound_event_stages = Arc::clone(&adapter.outbound_event_stages);
            let lifecycle = adapter.lifecycle.clone();
            let live = Arc::clone(&adapter.live_sessions);
            tokio::spawn(async move {
                Self::run_reaper(
                    routes,
                    events_tx,
                    outbound_event_stages,
                    reaper_cancel,
                    ttl_secs,
                    metrics_reaped,
                    live,
                    lifecycle,
                )
                .await;
            });
        }

        // P12.8 — periodic per-Connection quality emitter. Walks the
        // routes table every 5 seconds and emits one
        // `AdapterEvent::Quality` per connection from the aggregated
        // per-stream snapshots already collected by
        // `crate::media::stats::spawn_webrtc_stats_collector`. The
        // orchestrator feeds these into its `QualityAggregator` so
        // `Event::SessionEnded` reports include WebRTC-side numbers.
        Self::spawn_quality_emitter(
            Arc::clone(&adapter.routes),
            Arc::clone(&adapter.outbound_event_stages),
            adapter.events_tx.clone(),
        );

        adapter
    }

    fn validate_inbound_admission_confirmation_timeout(timeout: Duration) -> Result<()> {
        if timeout.is_zero() || timeout > MAX_INBOUND_ADMISSION_CONFIRMATION_TIMEOUT {
            return Err(WebRtcError::InvalidArgument(
                "inbound admission confirmation timeout must be nonzero and at most 30 seconds"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Configured secure inbound-admission timeout, or `None` in legacy mode.
    pub fn inbound_admission_confirmation_timeout(&self) -> Option<Duration> {
        self.inbound_admission_confirmation_timeout
    }

    fn spawn_quality_emitter(
        routes: Arc<DashMap<ConnectionId, Route>>,
        outbound_event_stages: Arc<DashMap<ConnectionId, Arc<OutboundEventStage>>>,
        events_tx: mpsc::Sender<OrchestratorAdapterEvent>,
    ) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                for entry in routes.iter() {
                    let conn_id = entry.key().clone();
                    // Per-Connection aggregate: average jitter / loss
                    // across this connection's streams. MOS is dropped
                    // for now — the orchestrator's QualityAggregator
                    // only consumes jitter and loss fields. Skip
                    // connections with no streams to avoid emitting
                    // bogus zero snapshots.
                    let streams = &entry.value().streams;
                    if streams.is_empty() {
                        continue;
                    }
                    let mut count = 0u32;
                    let mut jitter_sum = 0.0f32;
                    let mut loss_sum = 0.0f32;
                    for stream in streams.iter() {
                        let snap = stream.value().webrtc_stats_snapshot();
                        jitter_sum += snap.jitter_ms;
                        loss_sum += snap.packet_loss_pct;
                        count += 1;
                    }
                    if count == 0 {
                        continue;
                    }
                    let snapshot = rvoip_core::stream::QualitySnapshot {
                        jitter_ms: jitter_sum / count as f32,
                        packet_loss_pct: loss_sum / count as f32,
                        mos: None,
                    };
                    let _ = Self::publish_or_stage_to(
                        &outbound_event_stages,
                        &events_tx,
                        AdapterEvent::Quality {
                            connection_id: conn_id,
                            snapshot,
                        },
                    );
                }
            }
        });
    }

    /// Snapshot of operational counters and live session count.
    pub fn metrics(&self) -> WebRtcMetrics {
        WebRtcMetrics {
            inbound_total: self.metrics_inbound.load(Ordering::Relaxed),
            outbound_total: self.metrics_outbound.load(Ordering::Relaxed),
            active_sessions: self.routes.len(),
            signaling_errors_total: self.metrics_errors.load(Ordering::Relaxed),
            sessions_rejected_over_cap: self.metrics_rejected.load(Ordering::Relaxed),
            reaped_total: self.metrics_reaped.load(Ordering::Relaxed),
            data_messages_dropped_total: self.metrics_data_dropped.load(Ordering::Relaxed),
        }
    }

    /// G12 — reset every counter to zero. Useful for operators that rotate
    /// Prometheus scrape windows or for hand-rolled rate-of-change graphs.
    /// Does **not** touch the live session count or running routes.
    pub fn reset_metrics(&self) {
        self.metrics_inbound.store(0, Ordering::Relaxed);
        self.metrics_outbound.store(0, Ordering::Relaxed);
        self.metrics_errors.store(0, Ordering::Relaxed);
        self.metrics_rejected.store(0, Ordering::Relaxed);
        self.metrics_reaped.store(0, Ordering::Relaxed);
        self.metrics_data_dropped.store(0, Ordering::Relaxed);
    }

    /// Public accessor for the configured concurrent-session cap.
    pub fn max_concurrent_sessions(&self) -> usize {
        self.config.max_concurrent_sessions
    }

    /// Per-IP WHIP rate limit (POSTs/min). `0` = disabled.
    pub fn whip_rate_limit_cap_per_min(&self) -> u32 {
        self.config.whip_per_ip_per_min
    }

    /// CORS allow-list. Empty = no CORS layer.
    pub fn cors_origins(&self) -> &[String] {
        &self.config.cors_origins
    }

    /// ICE server URLs flattened from the config (for `Link: rel=ice-server`).
    pub fn ice_server_urls(&self) -> Vec<String> {
        self.config
            .ice_servers
            .iter()
            .flat_map(|s| s.urls.iter().cloned())
            .collect()
    }

    /// Configured ICE servers (with optional TURN credentials). Used by the
    /// WHIP handler to emit `Link: <url>; rel="ice-server"; username="…";
    /// credential="…"` headers per RFC 9725 §4.6.
    pub fn ice_servers(&self) -> &[crate::config::IceServerConfig] {
        &self.config.ice_servers
    }

    /// WebSocket max message size in bytes.
    pub fn ws_max_message_size(&self) -> usize {
        self.config.ws_max_message_size
    }

    /// WebSocket server-driven ping interval. `0` = disabled.
    pub fn ws_keepalive_secs(&self) -> u64 {
        self.config.ws_keepalive_secs
    }

    /// Whether the adapter was built in trickle-ICE mode.
    pub fn trickle_ice_enabled(&self) -> bool {
        self.config.trickle_ice
    }

    /// Policy applied to inbound mDNS (`.local`) trickle candidates.
    pub fn mdns_candidate_policy(&self) -> crate::config::MdnsCandidatePolicy {
        self.config.mdns_candidate_policy
    }

    /// Remote DTLS-SRTP fingerprints (one per `a=fingerprint:` line) from the
    /// stored remote SDP. Returns `Err(ConnectionNotFound)` if there is no
    /// such route, or `Ok(vec![])` if the route has no remote SDP yet (e.g.
    /// outbound originate before the answer arrives).
    ///
    /// D2 — [`ConnectionAdapter::verify_request_signature`] now surfaces
    /// the first canonical fingerprint here as
    /// [`IdentityAssurance::DtlsFingerprint`].
    pub fn remote_dtls_fingerprint(
        &self,
        conn: &ConnectionId,
    ) -> Result<Vec<crate::identity::DtlsFingerprint>> {
        let route = self.route(conn)?;
        Ok(route
            .remote_sdp
            .as_deref()
            .map(crate::identity::extract_fingerprints)
            .unwrap_or_default())
    }

    /// D2 — register a per-route fingerprint pinning hook. The hook's
    /// returned list is unioned with [`WebRtcConfig::pinned_fingerprints`];
    /// when the union is non-empty, peers whose `a=fingerprint:` doesn't
    /// match are rejected with
    /// [`WebRtcError::FingerprintNotPinned`].
    pub fn set_fingerprint_policy(&self, hook: Arc<dyn FingerprintPolicyHook>) {
        *self.fingerprint_policy.write() = Some(hook);
    }

    /// D2 — clear any previously-registered policy hook. Static
    /// `WebRtcConfig::pinned_fingerprints` still applies.
    pub fn clear_fingerprint_policy(&self) {
        *self.fingerprint_policy.write() = None;
    }

    fn parse_shared_sdp(sdp: &str) -> Result<SdpSession> {
        SdpSession::from_str(sdp)
            .map_err(|err| WebRtcError::Sdp(format!("shared SDP parse failed: {err}")))
    }

    /// D2 — evaluate the union of static + dynamic pin lists against the
    /// remote SDP's fingerprints. `Ok(())` when allowed (including when no
    /// pinning is in effect); `Err(FingerprintNotPinned)` when the remote
    /// has at least one fingerprint and none match.
    async fn enforce_fingerprint_policy(
        &self,
        conn: &ConnectionId,
        remote_sdp: &str,
        session_hint: Option<&str>,
    ) -> Result<()> {
        let mut allowed: Vec<crate::identity::DtlsFingerprint> =
            self.config.pinned_fingerprints.clone();
        // Drop the read guard before awaiting — parking_lot guards are not Send.
        let hook = self.fingerprint_policy.read().clone();
        if let Some(hook) = hook {
            allowed.extend(hook.allowed_fingerprints(conn, session_hint).await);
        }
        if allowed.is_empty() {
            return Ok(());
        }
        let remote = crate::identity::extract_fingerprints(remote_sdp);
        if remote.is_empty() {
            return Err(WebRtcError::FingerprintNotPinned);
        }
        let normalize = |fp: &crate::identity::DtlsFingerprint| {
            (
                fp.algorithm.to_ascii_lowercase(),
                fp.value.to_ascii_lowercase(),
            )
        };
        let allowed_norm: std::collections::HashSet<_> = allowed.iter().map(normalize).collect();
        if !remote.iter().any(|r| allowed_norm.contains(&normalize(r))) {
            return Err(WebRtcError::FingerprintNotPinned);
        }
        Ok(())
    }

    /// Atomically reserve a session slot. Returns a guard that releases the
    /// slot on Drop unless `commit()` is called first. Race-free under
    /// concurrent originate / apply_remote_offer.
    fn reserve_session_slot(&self) -> Result<SessionSlotGuard> {
        let cap = self.config.max_concurrent_sessions;
        let live = Arc::clone(&self.live_sessions);
        loop {
            let current = live.load(Ordering::Acquire);
            if cap > 0 && current >= cap {
                self.metrics_rejected.fetch_add(1, Ordering::Relaxed);
                return Err(WebRtcError::Adapter(format!(
                    "concurrent session cap reached ({cap})"
                )));
            }
            if live
                .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(SessionSlotGuard { live: Some(live) });
            }
        }
    }

    /// Increment the signaling-errors counter; called by the WHIP/WS handlers
    /// when something rejectable happens.
    pub fn note_signaling_error(&self) {
        self.metrics_errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the live-session counter (called when a route is removed).
    fn release_session_slot(&self) {
        Self::release_session_slot_from(&self.live_sessions);
    }

    fn release_session_slot_from(live_sessions: &std::sync::atomic::AtomicUsize) {
        // saturating sub so a double-release can't underflow.
        let mut cur = live_sessions.load(Ordering::Acquire);
        while cur > 0 {
            match live_sessions.compare_exchange(cur, cur - 1, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return,
                Err(actual) => cur = actual,
            }
        }
    }

    pub fn routes(&self) -> &Arc<DashMap<ConnectionId, Route>> {
        &self.routes
    }

    /// G4 — aggregate `WebRtcStatsSnapshot` fields across every live media
    /// stream on every route. Used by the Prometheus exporter and by
    /// dashboards that want a single rollup number per peer-connection.
    ///
    /// Returns a `(total_streams, aggregated_snapshot)` tuple. The snapshot's
    /// `selected_pair` field is taken from the first stream that has one.
    pub fn aggregated_stats(&self) -> (usize, crate::media::WebRtcStatsSnapshot) {
        use crate::media::pump::{CandidatePairStats, OutboundStats};
        let mut total = 0usize;
        let mut agg = crate::media::WebRtcStatsSnapshot::default();
        let mut sample_pair: Option<CandidatePairStats> = None;
        let mut jitter_sum: f32 = 0.0;
        let mut loss_sum: f32 = 0.0;
        let mut mos_sum: f32 = 0.0;
        for entry in self.routes.iter() {
            for stream in entry.value().streams.iter() {
                let snap = stream.value().webrtc_stats_snapshot();
                total += 1;
                agg.packets_received = agg.packets_received.saturating_add(snap.packets_received);
                agg.bytes_received = agg.bytes_received.saturating_add(snap.bytes_received);
                agg.packets_lost = agg.packets_lost.saturating_add(snap.packets_lost);
                agg.frames_dropped = agg.frames_dropped.saturating_add(snap.frames_dropped);
                jitter_sum += snap.jitter_ms;
                loss_sum += snap.packet_loss_pct;
                mos_sum += snap.mos;
                agg.outbound = OutboundStats {
                    packets_sent: agg
                        .outbound
                        .packets_sent
                        .saturating_add(snap.outbound.packets_sent),
                    bytes_sent: agg
                        .outbound
                        .bytes_sent
                        .saturating_add(snap.outbound.bytes_sent),
                    retransmitted_packets: agg
                        .outbound
                        .retransmitted_packets
                        .saturating_add(snap.outbound.retransmitted_packets),
                    retransmitted_bytes: agg
                        .outbound
                        .retransmitted_bytes
                        .saturating_add(snap.outbound.retransmitted_bytes),
                    nack_count: agg
                        .outbound
                        .nack_count
                        .saturating_add(snap.outbound.nack_count),
                    pli_count: agg
                        .outbound
                        .pli_count
                        .saturating_add(snap.outbound.pli_count),
                    fir_count: agg
                        .outbound
                        .fir_count
                        .saturating_add(snap.outbound.fir_count),
                };
                if sample_pair.is_none() {
                    sample_pair = snap.selected_pair;
                }
            }
        }
        if total > 0 {
            agg.jitter_ms = jitter_sum / total as f32;
            agg.packet_loss_pct = loss_sum / total as f32;
            agg.mos = mos_sum / total as f32;
        }
        agg.selected_pair = sample_pair;
        (total, agg)
    }

    fn try_take_atomic_events(&self) -> Result<mpsc::Receiver<OrchestratorAdapterEvent>> {
        match self.events_rx.lock() {
            Ok(mut guard) => guard.take().ok_or(WebRtcError::AlreadySubscribed),
            Err(poisoned) => {
                // Recover from a poisoned mutex (a panic occurred while holding it).
                let mut guard = poisoned.into_inner();
                guard.take().ok_or(WebRtcError::AlreadySubscribed)
            }
        }
    }

    /// Single-take public event receiver preserving the historical authenticated
    /// inbound sequence (`InboundConnection`, then `PrincipalAuthenticated`).
    ///
    /// Returns `Err(AlreadySubscribed)` on a second call. The atomic source item
    /// is expanded by a bounded forwarding task only after leaving the
    /// Orchestrator's security-sensitive queue.
    pub fn try_subscribe_events(&self) -> Result<mpsc::Receiver<AdapterEvent>> {
        self.try_take_atomic_events()
            .map(|events| legacy_normalized_event_receiver(events, ADAPTER_EVENT_CAP * 2))
    }

    /// Opt in to the raw atomic adapter event stream.
    ///
    /// This is the stream consumed by `rvoip-core::Orchestrator`; most direct
    /// callers should use [`Self::try_subscribe_events`] for compatibility.
    pub fn try_subscribe_atomic_events(&self) -> Result<mpsc::Receiver<OrchestratorAdapterEvent>> {
        self.try_take_atomic_events()
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

    /// Retain an event while its outbound route is dormant. The original
    /// event is returned when no dormant stage owns it and normal publication
    /// should continue.
    fn stage_outbound_event_to(
        stages: &DashMap<ConnectionId, Arc<OutboundEventStage>>,
        event: AdapterEvent,
    ) -> Option<AdapterEvent> {
        let Some(connection_id) = Self::adapter_event_connection_id(&event).cloned() else {
            return Some(event);
        };
        let Some(stage) = stages.get(&connection_id) else {
            return Some(event);
        };
        let mut state = stage
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &mut *state {
            OutboundEventStageState::Dormant { events, overflowed } => {
                if events.len() >= OUTBOUND_EVENT_STAGE_CAPACITY {
                    *overflowed = true;
                } else {
                    events.push_back(event);
                }
                None
            }
            OutboundEventStageState::Activated => Some(event),
        }
    }

    fn publish_or_stage_to(
        stages: &DashMap<ConnectionId, Arc<OutboundEventStage>>,
        events_tx: &mpsc::Sender<OrchestratorAdapterEvent>,
        event: AdapterEvent,
    ) -> bool {
        let Some(event) = Self::stage_outbound_event_to(stages, event) else {
            return true;
        };
        events_tx
            .try_send(OrchestratorAdapterEvent::Public(event))
            .is_ok()
    }

    fn try_send(&self, event: AdapterEvent) {
        if !Self::publish_or_stage_to(&self.outbound_event_stages, &self.events_tx, event) {
            warn!("WebRtcAdapter event channel full or closed");
        }
    }

    async fn send_inbound_event(&self, event: OrchestratorAdapterEvent) -> bool {
        Self::send_inbound_event_to(&self.events_tx, event).await
    }

    async fn send_inbound_event_to(
        events_tx: &mpsc::Sender<OrchestratorAdapterEvent>,
        event: OrchestratorAdapterEvent,
    ) -> bool {
        match tokio::time::timeout(INBOUND_EVENT_DELIVERY_TIMEOUT, events_tx.send(event)).await {
            Ok(Ok(())) => true,
            Ok(Err(_)) => {
                warn!("WebRtcAdapter inbound event channel closed");
                false
            }
            Err(_) => {
                warn!("WebRtcAdapter inbound event delivery timed out");
                false
            }
        }
    }

    async fn deliver_terminal_event(
        lifecycle: &AdapterLifecycleSinkSlot,
        events_tx: &mpsc::Sender<OrchestratorAdapterEvent>,
        event: AdapterEvent,
        source: &'static str,
    ) {
        let delivery = lifecycle
            .queue_or_deliver_orchestrator_terminal(events_tx, event)
            .await;
        if delivery == TerminalDelivery::Undeliverable {
            warn!(source, "WebRtcAdapter terminal event was undeliverable");
        }
    }

    async fn deliver_or_stage_terminal_event(
        lifecycle: &AdapterLifecycleSinkSlot,
        events_tx: &mpsc::Sender<OrchestratorAdapterEvent>,
        stages: &DashMap<ConnectionId, Arc<OutboundEventStage>>,
        event: AdapterEvent,
        source: &'static str,
    ) {
        let connection_id = Self::adapter_event_connection_id(&event).cloned();
        let Some(event) = Self::stage_outbound_event_to(stages, event) else {
            return;
        };
        Self::deliver_terminal_event(lifecycle, events_tx, event, source).await;
        if let Some(connection_id) = connection_id {
            stages.remove(&connection_id);
        }
    }

    fn spawn_data_message_manager(&self, conn: ConnectionId, route: &Route) {
        if route.data_pump_started.swap(true, Ordering::AcqRel) {
            return;
        }

        let peer = Arc::clone(&route.peer);
        let channels = Arc::clone(&route.data_channel);
        let pumped = Arc::clone(&route.data_channels_pumped);
        let channel_keys = Arc::clone(&route.data_channel_keys);
        let events_tx = self.events_tx.clone();
        let outbound_event_stages = Arc::clone(&self.outbound_event_stages);
        let dropped = Arc::clone(&self.metrics_data_dropped);
        let mut cancel = route.data_cancel.subscribe();
        tokio::spawn(async move {
            let mut scan = tokio::time::interval(DATA_CHANNEL_SCAN_INTERVAL);
            scan.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if *cancel.borrow() {
                    break;
                }

                // Locally-created channels do not pass through
                // PeerConnectionEventHandler::on_data_channel, so include the
                // route cache as well as every remotely-seen channel.
                let mut candidates: Vec<Arc<dyn DataChannel>> = channels
                    .iter()
                    .map(|entry| Arc::clone(entry.value()))
                    .collect();
                candidates.extend(peer.seen_data_channels());
                for channel in candidates {
                    if let Err(error) = Self::register_data_channel_pump(
                        conn.clone(),
                        Arc::clone(&peer),
                        channel,
                        Arc::clone(&channels),
                        Arc::clone(&pumped),
                        Arc::clone(&channel_keys),
                        events_tx.clone(),
                        Arc::clone(&outbound_event_stages),
                        Arc::clone(&dropped),
                        cancel.clone(),
                    )
                    .await
                    {
                        debug!(conn = %conn, error = %error, "ignoring invalid WebRTC data channel");
                    }
                }

                tokio::select! {
                    changed = cancel.changed() => {
                        if changed.is_err() || *cancel.borrow() {
                            break;
                        }
                    }
                    _ = scan.tick() => {}
                }
            }
        });
    }

    async fn register_data_channel_pump(
        conn: ConnectionId,
        peer: Arc<RvoipPeerConnection>,
        channel: Arc<dyn DataChannel>,
        channels: Arc<DashMap<String, Arc<dyn DataChannel>>>,
        pumped: Arc<SyncMutex<HashSet<usize>>>,
        channel_keys: Arc<DashMap<usize, String>>,
        events_tx: mpsc::Sender<OrchestratorAdapterEvent>,
        outbound_event_stages: Arc<DashMap<ConnectionId, Arc<OutboundEventStage>>>,
        dropped: Arc<AtomicU64>,
        mut cancel: watch::Receiver<bool>,
    ) -> std::result::Result<bool, String> {
        if *cancel.borrow() {
            return Ok(false);
        }

        let channel_identity = data_channel_identity(&channel);
        if pumped.lock().contains(&channel_identity) {
            if let Some(cache_key) = channel_keys.get(&channel_identity) {
                channels
                    .entry(cache_key.value().clone())
                    .or_insert_with(|| Arc::clone(&channel));
            }
            return Ok(false);
        }

        let state = match channel.ready_state().await {
            Ok(state) => state,
            Err(error) => {
                peer.forget_seen_data_channel(&channel);
                remove_cached_data_channel(&channels, &channel);
                return Err(error.to_string());
            }
        };
        if matches!(
            state,
            RTCDataChannelState::Closing | RTCDataChannelState::Closed
        ) {
            peer.forget_seen_data_channel(&channel);
            remove_cached_data_channel(&channels, &channel);
            return Ok(false);
        }
        let metadata = async {
            let label = channel.label().await.map_err(|error| error.to_string())?;
            let protocol = channel
                .protocol()
                .await
                .map_err(|error| error.to_string())?;
            let reliability = crate::data_message::reliability_from_channel(channel.as_ref())
                .await
                .map_err(|error| error.to_string())?;
            let cache_key = crate::data_message::cache_key_parts(&label, &reliability)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((label, protocol, reliability, cache_key))
        }
        .await;
        let (label, protocol, reliability, cache_key) = match metadata {
            Ok(metadata) => metadata,
            Err(error) => {
                peer.forget_seen_data_channel(&channel);
                remove_cached_data_channel(&channels, &channel);
                let _ = channel.close().await;
                return Err(error);
            }
        };
        let over_limit = {
            let mut registered = pumped.lock();
            if registered.contains(&channel_identity) {
                return Ok(false);
            }
            if registered.len() >= MAX_DATA_CHANNELS_PER_ROUTE {
                true
            } else {
                registered.insert(channel_identity);
                false
            }
        };
        if over_limit {
            peer.forget_seen_data_channel(&channel);
            remove_cached_data_channel(&channels, &channel);
            let _ = channel.close().await;
            return Err(format!(
                "per-route data-channel limit reached ({MAX_DATA_CHANNELS_PER_ROUTE})"
            ));
        }

        if protocol == crate::data_message::DATA_MESSAGE_SUBPROTOCOL {
            channel_keys.insert(channel_identity, cache_key.clone());
            channels
                .entry(cache_key)
                .or_insert_with(|| Arc::clone(&channel));
        }

        let channel_for_cleanup = Arc::clone(&channel);
        let channels_for_cleanup = Arc::clone(&channels);
        let pumped_for_cleanup = Arc::clone(&pumped);
        let keys_for_cleanup = Arc::clone(&channel_keys);
        let peer_for_cleanup = Arc::clone(&peer);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = cancel.changed() => {
                        if changed.is_err() || *cancel.borrow() {
                            break;
                        }
                    }
                    event = channel.poll() => {
                        match event {
                            Some(DataChannelEvent::OnMessage(frame)) => {
                                match crate::data_message::decode_data_message(
                                    &label,
                                    &protocol,
                                    reliability.clone(),
                                    &frame.data,
                                    frame.is_string,
                                ) {
                                    Ok(message) => {
                                        if !Self::publish_or_stage_to(&outbound_event_stages, &events_tx, AdapterEvent::DataMessage {
                                            connection_id: conn.clone(),
                                            message,
                                        }) {
                                            dropped.fetch_add(1, Ordering::Relaxed);
                                            warn!(conn = %conn, label, "WebRTC adapter event queue full; dropping data message");
                                        }
                                    }
                                    Err(error) => {
                                        warn!(conn = %conn, label, error = %error, "dropping invalid WebRTC data message");
                                    }
                                }
                            }
                            Some(DataChannelEvent::OnClose | DataChannelEvent::OnError) | None => break,
                            Some(_) => {}
                        }
                    }
                }
            }
            keys_for_cleanup.remove(&data_channel_identity(&channel_for_cleanup));
            pumped_for_cleanup
                .lock()
                .remove(&data_channel_identity(&channel_for_cleanup));
            remove_cached_data_channel(&channels_for_cleanup, &channel_for_cleanup);
            peer_for_cleanup.forget_seen_data_channel(&channel_for_cleanup);
        });
        Ok(true)
    }

    async fn wait_data_channel_open(channel: &Arc<dyn DataChannel>) -> RvoipResult<()> {
        let deadline = tokio::time::Instant::now() + DATA_CHANNEL_OPERATION_TIMEOUT;
        loop {
            let state = channel
                .ready_state()
                .await
                .map_err(|error| RvoipError::Adapter(format!("data channel state: {error}")))?;
            match state {
                RTCDataChannelState::Open => return Ok(()),
                RTCDataChannelState::Closing | RTCDataChannelState::Closed => {
                    return Err(RvoipError::Adapter(
                        "data channel closed before it opened".into(),
                    ));
                }
                _ if tokio::time::Instant::now() >= deadline => {
                    return Err(RvoipError::Adapter("data channel open timed out".into()));
                }
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    }

    async fn data_channel_for_message(
        &self,
        conn: &ConnectionId,
        route: &Route,
        message: &DataMessage,
    ) -> RvoipResult<Arc<dyn DataChannel>> {
        let cache_key = crate::data_message::cache_key(message)
            .map_err(|error| RvoipError::Adapter(error.to_string()))?;
        let cached = route
            .data_channel
            .get(&cache_key)
            .map(|entry| Arc::clone(entry.value()));
        if let Some(channel) = cached {
            match Self::wait_data_channel_open(&channel).await {
                Ok(()) => return Ok(channel),
                Err(error) => {
                    remove_cached_data_channel(&route.data_channel, &channel);
                    let _ = channel.close().await;
                    debug!(conn = %conn, error = %error, "evicted unusable cached WebRTC data channel");
                }
            }
        }

        let _create_guard = route.data_channel_create.lock().await;
        let cached = route
            .data_channel
            .get(&cache_key)
            .map(|entry| Arc::clone(entry.value()));
        if let Some(channel) = cached {
            match Self::wait_data_channel_open(&channel).await {
                Ok(()) => return Ok(channel),
                Err(error) => {
                    remove_cached_data_channel(&route.data_channel, &channel);
                    let _ = channel.close().await;
                    debug!(conn = %conn, error = %error, "evicted unusable cached WebRTC data channel");
                }
            }
        }
        if route.data_channels_pumped.lock().len() >= MAX_DATA_CHANNELS_PER_ROUTE {
            return Err(RvoipError::Adapter(format!(
                "per-route data-channel limit reached ({MAX_DATA_CHANNELS_PER_ROUTE})"
            )));
        }

        let options = crate::data_message::options_for_reliability(&message.reliability)
            .map_err(|error| RvoipError::Adapter(error.to_string()))?;
        let channel = tokio::time::timeout(
            DATA_CHANNEL_OPERATION_TIMEOUT,
            route.peer.create_data_channel(&message.label, options),
        )
        .await
        .map_err(|_| RvoipError::Adapter("create_data_channel timed out".into()))?
        .map_err(|error| RvoipError::Adapter(error.to_string()))?;
        route.data_channel.insert(cache_key, Arc::clone(&channel));
        if let Err(error) = Self::wait_data_channel_open(&channel).await {
            remove_cached_data_channel(&route.data_channel, &channel);
            let _ = channel.close().await;
            return Err(error);
        }
        Self::register_data_channel_pump(
            conn.clone(),
            Arc::clone(&route.peer),
            Arc::clone(&channel),
            Arc::clone(&route.data_channel),
            Arc::clone(&route.data_channels_pumped),
            Arc::clone(&route.data_channel_keys),
            self.events_tx.clone(),
            Arc::clone(&self.outbound_event_stages),
            Arc::clone(&self.metrics_data_dropped),
            route.data_cancel.subscribe(),
        )
        .await
        .map_err(RvoipError::Adapter)?;
        Ok(channel)
    }

    fn build_connection(
        &self,
        conn_id: ConnectionId,
        direction: Direction,
        negotiated: NegotiatedCodecs,
        handle: Arc<WebRtcTransportHandle>,
    ) -> Connection {
        Connection {
            id: conn_id,
            session_id: rvoip_core::ids::SessionId::new(),
            participant_id: rvoip_core::ids::ParticipantId::new(),
            transport: Transport::WebRtc,
            direction,
            state: ConnectionState::Connecting,
            capabilities: self.config.capabilities.clone(),
            negotiated_codecs: negotiated,
            streams: vec![],
            messaging_enabled: true,
            transport_handle: TransportHandle(handle),
            opened_at: Utc::now(),
            closed_at: None,
        }
    }

    fn make_transport_handle(
        &self,
        conn_id: ConnectionId,
        cancel: Arc<Notify>,
        data_cancel: watch::Sender<bool>,
    ) -> Arc<WebRtcTransportHandle> {
        Arc::new(WebRtcTransportHandle {
            connection_id: conn_id,
            routes: Arc::downgrade(&self.routes),
            cancel,
            data_cancel,
        })
    }

    /// Create the audio media stream for this route. Mirrors the original
    /// (pre-H1) behavior: wait up to 500ms for the remote track via
    /// `wait_remote_track`, then build the stream with the remote inline (if
    /// arrived) or as send-only (if not — late tracks attach via the
    /// track-attacher spawned in `insert_route`).
    async fn seed_media_stream(&self, conn: &ConnectionId, route: &Route) -> Result<()> {
        if !route.streams.is_empty() {
            return Ok(());
        }
        if route.peer.local_audio_track().is_none() {
            debug!(
                conn = %conn,
                "WebRTC route has no negotiated audio yet; leaving media streams empty"
            );
            return Ok(());
        }

        let codec = route.negotiated.audio.clone().unwrap_or_else(|| CodecInfo {
            name: "opus".into(),
            clock_rate_hz: 48000,
            channels: 2,
            fmtp: None,
        });

        let local = route
            .peer
            .local_audio_track()
            .ok_or_else(|| WebRtcError::Adapter("no local audio track".into()))?;
        let local_ssrc = route
            .peer
            .local_audio_ssrc()
            .ok_or_else(|| WebRtcError::Adapter("no local audio SSRC".into()))?;
        let payload_type = payload_type_for_audio_codec(&codec);

        let remote = route
            .peer
            .wait_remote_track(Duration::from_millis(500))
            .await
            .or(route.peer.try_recv_remote_track().await);

        let stream_id = StreamId::new();
        let has_remote = remote.is_some();
        let (dtmf_tx, mut dtmf_rx) = mpsc::channel::<crate::media::dtmf::DecodedDtmfEvent>(32);
        let events_tx = self.events_tx.clone();
        let outbound_event_stages = Arc::clone(&self.outbound_event_stages);
        let conn_for_dtmf = conn.clone();
        tokio::spawn(async move {
            while let Some(event) = dtmf_rx.recv().await {
                let _ = Self::publish_or_stage_to(
                    &outbound_event_stages,
                    &events_tx,
                    AdapterEvent::Dtmf {
                        connection_id: conn_for_dtmf.clone(),
                        digits: event.digit.to_string(),
                        duration_ms: event.duration_ms,
                    },
                );
            }
        });
        let media = from_tracks_with_dtmf_events(
            stream_id.clone(),
            codec,
            local,
            local_ssrc,
            payload_type,
            remote,
            Some(dtmf_tx),
        );
        if has_remote {
            media.enable_webrtc_stats(
                Arc::clone(route.peer.peer_connection()),
                Arc::clone(&route.cancel),
            );
        }
        route.streams.insert(stream_id, media);
        Ok(())
    }

    fn route(&self, conn: &ConnectionId) -> Result<Route> {
        self.routes
            .get(conn)
            .map(|e| e.value().clone())
            .ok_or(WebRtcError::ConnectionNotFound)
    }

    /// Return the complete principal retained for a network signaling route.
    /// Anonymous and legacy subject-only hooks intentionally return `None`.
    pub fn authenticated_principal(
        &self,
        conn: &ConnectionId,
    ) -> Result<Option<AuthenticatedPrincipal>> {
        Ok(self
            .route(conn)?
            .authorization
            .and_then(|authorization| authorization.principal))
    }

    /// Enforce the adapter-owned authorization boundary shared by every
    /// network signaling surface. Unowned routes remain accessible only to
    /// anonymous signaling, preserving source compatibility when auth is
    /// disabled; authenticated callers must use a route explicitly bound to
    /// their full ownership key.
    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    pub(crate) fn authorize_network_route(
        &self,
        conn: &ConnectionId,
        authorization: &RouteAuthorization,
    ) -> Result<()> {
        authorization.ensure_active()?;
        let route = self.route(conn)?;
        match route.authorization.as_ref() {
            Some(expected) if expected.owner == authorization.owner => Ok(()),
            None if authorization.owner == RouteOwnerKey::Anonymous => Ok(()),
            Some(_) | None => Err(WebRtcError::Forbidden(
                "connection belongs to another principal".into(),
            )),
        }
    }

    /// Bind an already-created route (notably a WHEP/originate route) before
    /// its connection id is exposed to the network.
    pub(crate) fn assign_route_authorization(
        &self,
        conn: &ConnectionId,
        authorization: RouteAuthorization,
        participant_id: String,
    ) -> Result<()> {
        authorization.ensure_active()?;
        let principal = authorization.principal.clone();
        let mut route = self
            .routes
            .get_mut(conn)
            .ok_or(WebRtcError::ConnectionNotFound)?;
        let assigned = match route.authorization.as_ref() {
            None => {
                route.authorization = Some(authorization);
                true
            }
            Some(existing) if existing.owner == authorization.owner => false,
            Some(_) => {
                return Err(WebRtcError::Forbidden(
                    "connection already belongs to another principal".into(),
                ))
            }
        };
        drop(route);
        if assigned {
            if let Some(principal) = principal {
                self.try_send(AdapterEvent::PrincipalAuthenticated {
                    connection_id: conn.clone(),
                    participant_id,
                    principal,
                });
            }
        }
        Ok(())
    }

    /// Bind a principal to an outbound route before exposing its connection
    /// id to authenticated WS/WSS signaling. WHEP performs this binding
    /// automatically; generic outbound signaling can use this method with the
    /// participant id returned by [`ConnectionAdapter::originate`].
    pub fn bind_authenticated_principal(
        &self,
        conn: &ConnectionId,
        participant_id: impl Into<String>,
        principal: AuthenticatedPrincipal,
    ) -> Result<()> {
        self.assign_route_authorization(
            conn,
            RouteAuthorization::principal(principal),
            participant_id.into(),
        )
    }

    /// D2 — update the stored remote SDP for an existing route (e.g. after
    /// `apply_remote_answer` lands the offerer's answer). No-op when the
    /// route has already been reaped.
    fn update_remote_sdp(&self, conn: &ConnectionId, sdp: &str) {
        if let Some(mut entry) = self.routes.get_mut(conn) {
            entry.remote_sdp = Some(sdp.to_owned());
        }
    }

    fn insert_route(&self, conn_id: ConnectionId, route: Route) -> Result<()> {
        let peer_track = Arc::clone(&route.peer);
        let peer_fail = Arc::clone(&route.peer);
        match self.routes.entry(conn_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(route);
            }
            Entry::Occupied(_) => {
                return Err(WebRtcError::Adapter(
                    "generated WebRTC connection id already exists".into(),
                ));
            }
        }

        // Track-attacher: wire the answerer's inbound RTP into each
        // `WebRtcMediaStream`'s frames_in pump once a remote track is
        // observed.
        //
        // The attacher *used* to only consume the `remote_track_rx`
        // channel (`try_recv_remote_track`) and `break` after the first
        // hit. That race-loses against any other caller that also reads
        // the channel — notably the test helper
        // `RvoipPeerConnection::prime_remote_track`, which calls
        // `wait_remote_track` (also consumes the channel). When the test
        // won the race the attacher looped forever and the inbound pump
        // was never spawned, so the QUIC bridge test
        // (`webrtc_quic_bridge_e2e::whip_webrtc_bridged_to_real_quic_leg`)
        // would time out at `client_in.recv()`.
        //
        // Fix: fall back to `discover_remote_track` (transceiver scan,
        // non-consuming) when the channel poll returns None. The
        // attacher also keeps looping after the first attach so a second
        // m-line (e.g. D1's DTMF or a future video track) gets its own
        // pump on a later iteration; `attach_remote`'s `compare_exchange`
        // guard makes the call idempotent per stream.
        let routes_track = Arc::clone(&self.routes);
        let conn_track = conn_id.clone();
        tokio::spawn(async move {
            use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
            use rvoip_core::stream::StreamKind;
            loop {
                if !routes_track.contains_key(&conn_track) {
                    break;
                }
                // 1) Fast path: drain anything sitting in the handler
                //    channel from `on_track` firings.
                while let Some(track) = peer_track.try_recv_remote_track().await {
                    attach_track_to_streams(&routes_track, &conn_track, &track).await;
                }
                // 2) Fallback: even if another consumer drained the
                //    channel, the underlying transceiver still exposes
                //    the receiver's track. Scan and attach. Idempotent
                //    via `WebRtcMediaStream::attach_remote`.
                if let Some(audio) = peer_track.discover_remote_track(RtpCodecKind::Audio).await {
                    attach_track_to_streams_matching(
                        &routes_track,
                        &conn_track,
                        &audio,
                        StreamKind::Audio,
                    )
                    .await;
                }
                if let Some(video) = peer_track.discover_remote_track(RtpCodecKind::Video).await {
                    attach_track_to_streams_matching(
                        &routes_track,
                        &conn_track,
                        &video,
                        StreamKind::Video,
                    )
                    .await;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });

        let routes_fail = Arc::clone(&self.routes);
        let events_fail = self.events_tx.clone();
        let outbound_stages_fail = Arc::clone(&self.outbound_event_stages);
        let lifecycle_fail = self.lifecycle.clone();
        let live_sessions_fail = Arc::clone(&self.live_sessions);
        let conn_fail = conn_id.clone();
        tokio::spawn(async move {
            peer_fail.wait_failed().await;
            if let Some((_, route)) = routes_fail.remove(&conn_fail) {
                route.cancel_tasks();
                Self::release_session_slot_from(&live_sessions_fail);
                if outbound_stages_fail.contains_key(&conn_fail)
                    || route.core_handoff_started.load(Ordering::Acquire)
                    || route.core_published.load(Ordering::Acquire)
                {
                    Self::deliver_or_stage_terminal_event(
                        &lifecycle_fail,
                        &events_fail,
                        &outbound_stages_fail,
                        AdapterEvent::Failed {
                            connection_id: conn_fail.clone(),
                            detail: "peer connection failed".into(),
                        },
                        "peer-failure",
                    )
                    .await;
                }
            }
        });
        Ok(())
    }

    // (H1 had two helper functions `spawn_track_attacher` and `spawn_fail_watcher`
    // factored out; reverted in the H4-followup bisect because the inline
    // original better matches webrtc-rs 0.20-alpha's timing expectations.
    // See `insert_route` above.)

    /// Background reaper: every `REAPER_TICK`, walk routes and remove peers that
    /// have been in `Failed` state for at least `ttl_secs` (gives users a window
    /// to introspect via `routes()` before GC).
    async fn run_reaper(
        routes: Arc<DashMap<ConnectionId, Route>>,
        events_tx: mpsc::Sender<OrchestratorAdapterEvent>,
        outbound_event_stages: Arc<DashMap<ConnectionId, Arc<OutboundEventStage>>>,
        cancel: Arc<Notify>,
        ttl_secs: u64,
        reaped_counter: Arc<AtomicU64>,
        live_sessions: Arc<std::sync::atomic::AtomicUsize>,
        lifecycle: AdapterLifecycleSinkSlot,
    ) {
        let ttl = Duration::from_secs(ttl_secs);
        loop {
            tokio::select! {
                _ = cancel.notified() => break,
                _ = tokio::time::sleep(REAPER_TICK) => {}
            }

            let mut victims: Vec<ConnectionId> = Vec::new();
            for entry in routes.iter() {
                let failed = *entry.value().failed_at.lock();
                if let Some(at) = failed {
                    if at.elapsed() >= ttl {
                        victims.push(entry.key().clone());
                    }
                }
            }
            for id in victims {
                if let Some((_, route)) = routes.remove(&id) {
                    route.cancel_tasks();
                    Self::release_session_slot_from(&live_sessions);
                    Self::deliver_or_stage_terminal_event(
                        &lifecycle,
                        &events_tx,
                        &outbound_event_stages,
                        AdapterEvent::Ended {
                            connection_id: id.clone(),
                            reason: EndReason::Normal,
                        },
                        "session-reaper",
                    )
                    .await;
                    if tokio::time::timeout(DATA_CHANNEL_OPERATION_TIMEOUT, route.peer.close())
                        .await
                        .is_err()
                    {
                        warn!(connection_id = %id, "WebRTC reaper peer close timed out");
                    }
                    reaped_counter.fetch_add(1, Ordering::Relaxed);
                    debug!("session reaper removed idle/failed route");
                }
            }
        }
    }

    /// Apply a remote SDP answer to an outbound (offerer) connection.
    pub async fn apply_remote_answer(&self, conn: ConnectionId, answer_sdp: &str) -> Result<()> {
        let _parsed_answer = Self::parse_shared_sdp(answer_sdp)?;
        // D2 — enforce pinned fingerprints against the answer's `a=fingerprint:`
        // lines before handing it to webrtc-rs. Rejecting here avoids
        // completing the DTLS handshake with an un-pinned peer.
        self.enforce_fingerprint_policy(&conn, answer_sdp, None)
            .await?;
        let route = self.route(&conn)?;
        route.peer.set_remote_answer(answer_sdp).await?;
        // Update the stored remote SDP so subsequent verify_request_signature
        // / remote_dtls_fingerprint calls see the answer's fingerprint.
        self.update_remote_sdp(&conn, answer_sdp);
        Ok(())
    }

    /// Handle an inbound SDP offer — creates answerer PC and emits `InboundConnection`.
    #[instrument(skip(self, offer_sdp), fields(sdp_bytes = offer_sdp.len()))]
    pub async fn apply_remote_offer(&self, offer_sdp: &str) -> Result<ConnectionId> {
        self.apply_remote_offer_inner(offer_sdp, None, None).await
    }

    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    pub(crate) async fn apply_remote_offer_authorized_with_hint(
        &self,
        offer_sdp: &str,
        authorization: RouteAuthorization,
        routing_hint: Option<InboundRoutingHint>,
    ) -> Result<ConnectionId> {
        authorization.ensure_active()?;
        self.apply_remote_offer_inner(offer_sdp, Some(authorization), routing_hint)
            .await
    }

    fn authenticated_inbound_context(
        connection_id: ConnectionId,
        authorization: &RouteAuthorization,
        routing_hint: Option<InboundRoutingHint>,
    ) -> Result<Option<InboundConnectionContext>> {
        authorization.ensure_active()?;
        let Some(principal) = authorization.principal.as_ref() else {
            return Ok(None);
        };
        InboundConnectionContext::new(
            connection_id,
            Transport::WebRtc,
            principal,
            routing_hint,
            InboundSignalingMetadata::default(),
        )
        .map(Some)
        .map_err(|error| match error {
            InboundContextError::MissingTenant => {
                WebRtcError::Unauthorized("authenticated principal has no tenant".into())
            }
            InboundContextError::ExpiredPrincipal => {
                WebRtcError::Unauthorized("authenticated principal has expired".into())
            }
            _ => WebRtcError::Signaling("authenticated inbound routing context is invalid".into()),
        })
    }

    fn validate_secure_inbound_request(
        &self,
        authorization: Option<&RouteAuthorization>,
        routing_hint: Option<&InboundRoutingHint>,
    ) -> Result<()> {
        if self.inbound_admission_confirmation_timeout.is_none() {
            return Ok(());
        }
        let valid = authorization
            .and_then(|authorization| authorization.principal.as_ref())
            .is_some_and(|principal| {
                !principal.subject.trim().is_empty()
                    && principal.subject != "anonymous"
                    && principal
                        .tenant
                        .as_deref()
                        .is_some_and(|tenant| !tenant.trim().is_empty())
                    && principal
                        .issuer
                        .as_deref()
                        .is_some_and(|issuer| !issuer.trim().is_empty())
                    && principal.method != AuthenticationMethod::Anonymous
                    && !matches!(principal.assurance, IdentityAssurance::Anonymous)
                    && !principal.is_expired()
                    && routing_hint.is_some()
            });
        if !valid {
            return Err(WebRtcError::InboundAdmissionRejected);
        }
        Ok(())
    }

    async fn remove_unconfirmed_inbound_route(
        &self,
        connection_id: &ConnectionId,
        notify_core: bool,
    ) {
        let Some((_, route)) = self.routes.remove(connection_id) else {
            return;
        };
        route.cancel_tasks();
        self.release_session_slot();
        if notify_core && route.core_handoff_started.load(Ordering::Acquire) {
            Self::deliver_terminal_event(
                &self.lifecycle,
                &self.events_tx,
                AdapterEvent::Failed {
                    connection_id: connection_id.clone(),
                    detail: "inbound signaling admission did not complete".into(),
                },
                "inbound-admission-timeout",
            )
            .await;
        }
        if tokio::time::timeout(DATA_CHANNEL_OPERATION_TIMEOUT, route.peer.close())
            .await
            .is_err()
        {
            warn!(connection_id = %connection_id, "WebRTC provisional peer close timed out");
        }
    }

    async fn apply_remote_offer_inner(
        &self,
        offer_sdp: &str,
        authorization: Option<RouteAuthorization>,
        routing_hint: Option<InboundRoutingHint>,
    ) -> Result<ConnectionId> {
        self.validate_secure_inbound_request(authorization.as_ref(), routing_hint.as_ref())?;
        let slot = self.reserve_session_slot()?;
        self.metrics_inbound.fetch_add(1, Ordering::Relaxed);
        let conn_id = ConnectionId::new();
        let _parsed_offer = Self::parse_shared_sdp(offer_sdp)?;
        // D2 — enforce pinned fingerprints against the offer's
        // `a=fingerprint:` lines BEFORE allocating a peer connection, so
        // an un-pinned peer never triggers DTLS negotiation costs.
        self.enforce_fingerprint_policy(&conn_id, offer_sdp, None)
            .await?;
        let peer = RvoipPeerConnection::new(&self.config, PeerRole::Answerer).await?;
        let answer_sdp = peer.accept_offer_and_gather(offer_sdp).await?;

        let negotiated = negotiate_audio(&self.config.capabilities, &self.config.capabilities)?;

        let cancel = Arc::new(Notify::new());
        let (data_cancel, _) = watch::channel(false);
        // SDP parsing, fingerprint policy, PeerConnection construction, and
        // ICE gathering can outlive a short-lived access token. Revalidate at
        // the publication boundary and propagate context failures rather than
        // creating a route the now-expired owner cannot update or delete.
        let inbound_context = match authorization.as_ref() {
            Some(authorization) => {
                Self::authenticated_inbound_context(conn_id.clone(), authorization, routing_hint)?
            }
            None => None,
        };
        let inbound_admission_waiter = self
            .inbound_admission_confirmation_timeout
            .map(|_| InboundAdmissionWaiter::new());
        let route = Route {
            peer: Arc::clone(&peer),
            streams: Arc::new(DashMap::new()),
            local_sdp: Some(answer_sdp),
            remote_sdp: Some(offer_sdp.to_owned()),
            data_channel: Arc::new(DashMap::new()),
            data_channel_create: Arc::new(AsyncMutex::new(())),
            data_channels_pumped: Arc::new(SyncMutex::new(HashSet::new())),
            data_channel_keys: Arc::new(DashMap::new()),
            data_pump_started: Arc::new(AtomicBool::new(false)),
            data_cancel: data_cancel.clone(),
            negotiated: negotiated.clone(),
            held: false,
            cancel: Arc::clone(&cancel),
            failed_at: Arc::new(SyncMutex::new(None)),
            core_published: Arc::new(AtomicBool::new(false)),
            core_handoff_started: Arc::new(AtomicBool::new(false)),
            authorization: authorization.clone(),
            inbound_context: Arc::new(SyncMutex::new(inbound_context)),
            inbound_admission_waiter: inbound_admission_waiter.clone(),
        };

        // Don't seed media stream here — the track-attacher (spawned in
        // insert_route) buffers any early on_track event and `accept()` /
        // `streams()` will create the stream lazily. Eager seeding before
        // `accept()` was attempted but interacted badly with webrtc-rs
        // 0.20-alpha's negotiation timing.

        self.insert_route(conn_id.clone(), route)?;
        slot.commit();

        let handle = self.make_transport_handle(conn_id.clone(), cancel, data_cancel);
        let connection =
            self.build_connection(conn_id.clone(), Direction::Inbound, negotiated, handle);
        let participant_id = connection.participant_id.clone();
        if inbound_admission_waiter.is_some() {
            let Some(route) = self.routes.get(&conn_id) else {
                return Err(WebRtcError::InboundAdmissionRejected);
            };
            route.core_handoff_started.store(true, Ordering::Release);
        }
        let delivered = if let Some(principal) = authorization.and_then(|value| value.principal) {
            self.send_inbound_event(OrchestratorAdapterEvent::AuthenticatedInboundConnection {
                connection,
                participant_id: participant_id.to_string(),
                principal,
            })
            .await
        } else {
            self.send_inbound_event(OrchestratorAdapterEvent::Public(
                AdapterEvent::InboundConnection { connection },
            ))
            .await
        };
        if !delivered {
            self.remove_unconfirmed_inbound_route(&conn_id, true).await;
            return Err(WebRtcError::InboundAdmissionRejected);
        }

        if let (Some(timeout), Some(waiter)) = (
            self.inbound_admission_confirmation_timeout,
            inbound_admission_waiter,
        ) {
            match waiter.wait(timeout).await {
                InboundAdmissionOutcome::Accepted => {
                    let ready = self.routes.get(&conn_id).is_some_and(|route| {
                        route
                            .inbound_admission_waiter
                            .as_ref()
                            .is_some_and(|registered| Arc::ptr_eq(registered, &waiter))
                            && route.core_published.load(Ordering::Acquire)
                            && waiter.is_accepted_and_live()
                    });
                    if !ready {
                        self.remove_unconfirmed_inbound_route(&conn_id, true).await;
                        return Err(WebRtcError::InboundAdmissionRejected);
                    }
                }
                InboundAdmissionOutcome::Pending => {
                    waiter.cancel();
                    self.remove_unconfirmed_inbound_route(&conn_id, true).await;
                    return Err(WebRtcError::InboundAdmissionRejected);
                }
                InboundAdmissionOutcome::Rejected | InboundAdmissionOutcome::Cancelled => {
                    self.remove_unconfirmed_inbound_route(&conn_id, false).await;
                    return Err(WebRtcError::InboundAdmissionRejected);
                }
            }
        } else {
            let Some(route) = self.routes.get(&conn_id) else {
                return Err(WebRtcError::Signaling(
                    "WebRTC route ended during inbound publication".into(),
                ));
            };
            route.core_published.store(true, Ordering::Release);
        }

        Ok(conn_id)
    }

    pub fn local_sdp(&self, conn: &ConnectionId) -> Result<String> {
        let route = self.route(conn)?;
        if route
            .inbound_admission_waiter
            .as_ref()
            .is_some_and(|waiter| !waiter.is_accepted_and_live())
        {
            return Err(WebRtcError::InboundAdmissionRejected);
        }
        route
            .local_sdp
            .clone()
            .ok_or_else(|| WebRtcError::Sdp("no local SDP".into()))
    }

    /// Ensure the route has a media stream — idempotent. Called from
    /// `accept()` (after wait_connected) and `streams()`.
    async fn ensure_media_streams(&self, conn: &ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        if route.streams.is_empty() {
            self.seed_media_stream(conn, &route)
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        }
        Ok(())
    }

    /// Apply a remote SDP answer to a WHEP/outbound offerer connection and bring it up.
    pub async fn accept_remote_answer(&self, conn: ConnectionId, answer_sdp: &str) -> Result<()> {
        self.apply_remote_answer(conn.clone(), answer_sdp).await?;
        ConnectionAdapter::accept(self, conn)
            .await
            .map_err(|e| WebRtcError::Adapter(format!("{e}")))?;
        Ok(())
    }

    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    pub(crate) async fn accept_remote_answer_authorized(
        &self,
        conn: ConnectionId,
        answer_sdp: &str,
        authorization: &RouteAuthorization,
    ) -> Result<()> {
        self.authorize_network_route(&conn, authorization)?;
        self.accept_remote_answer(conn, answer_sdp).await
    }

    /// WHIP ICE restart: apply a new offer on an inbound (answerer) connection.
    pub async fn apply_ice_restart_offer(
        &self,
        conn: ConnectionId,
        offer_sdp: &str,
    ) -> Result<String> {
        let _parsed_offer = Self::parse_shared_sdp(offer_sdp)?;
        let route = self.route(&conn)?;
        if route.peer.role() != PeerRole::Answerer {
            return Err(WebRtcError::Adapter(
                "WHIP ICE restart requires an inbound (answerer) connection".into(),
            ));
        }
        route
            .peer
            .prepare_answerer_media_for_offer(offer_sdp)
            .await?;
        let answer = route.peer.renegotiate_as_answerer(offer_sdp).await?;
        if let Some(mut route_mut) = self.routes.get_mut(&conn) {
            route_mut.local_sdp = Some(answer.clone());
            route_mut.remote_sdp = Some(offer_sdp.to_owned());
        }
        Ok(answer)
    }

    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    pub(crate) async fn apply_ice_restart_offer_authorized(
        &self,
        conn: ConnectionId,
        offer_sdp: &str,
        authorization: &RouteAuthorization,
    ) -> Result<String> {
        self.authorize_network_route(&conn, authorization)?;
        self.apply_ice_restart_offer(conn, offer_sdp).await
    }

    /// Apply a trickle ICE candidate (JSON `RTCIceCandidateInit` shape) to the
    /// peer identified by `conn`. Returns `ConnectionNotFound` if there is no
    /// such route. Drops `.local` mDNS candidates when
    /// `WebRtcConfig::mdns_candidate_policy` is `Drop` (the default).
    #[instrument(skip(self, candidate), fields(conn = %conn))]
    pub async fn apply_trickle_candidate(
        &self,
        conn: &ConnectionId,
        candidate: webrtc::peer_connection::RTCIceCandidateInit,
    ) -> Result<()> {
        let route = self.route(conn)?;
        if matches!(
            self.config.mdns_candidate_policy,
            crate::config::MdnsCandidatePolicy::Drop
        ) && crate::config::MdnsCandidatePolicy::is_mdns_candidate(&candidate.candidate)
        {
            debug!(conn = %conn, "dropping mDNS (.local) trickle candidate per policy");
            return Ok(());
        }
        route.peer.add_remote_ice_candidate(candidate).await
    }

    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    pub(crate) async fn apply_trickle_candidate_authorized(
        &self,
        conn: &ConnectionId,
        candidate: webrtc::peer_connection::RTCIceCandidateInit,
        authorization: &RouteAuthorization,
    ) -> Result<()> {
        self.authorize_network_route(conn, authorization)?;
        self.apply_trickle_candidate(conn, candidate).await
    }

    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    pub(crate) async fn end_authorized(
        &self,
        conn: ConnectionId,
        reason: EndReason,
        authorization: &RouteAuthorization,
    ) -> Result<()> {
        self.authorize_network_route(&conn, authorization)?;
        ConnectionAdapter::end(self, conn, reason)
            .await
            .map_err(WebRtcError::from)
    }

    /// Re-create a local SDP after a transceiver direction change (hold/resume).
    /// Stores it on the route as `local_sdp` and returns it. The caller (or
    /// signaling layer) is responsible for pushing the new SDP to the remote.
    async fn renegotiate_after_direction_change(&self, conn: &ConnectionId) -> Result<String> {
        let route = self.route(conn)?;
        let sdp = match route.peer.role() {
            PeerRole::Offerer => route.peer.renegotiate_as_offerer().await?,
            PeerRole::Answerer => {
                let offer = route.remote_sdp.clone().ok_or_else(|| {
                    WebRtcError::Sdp("no remote offer stored to renegotiate against".into())
                })?;
                route.peer.renegotiate_as_answerer(&offer).await?
            }
        };
        if let Some(mut route_mut) = self.routes.get_mut(conn) {
            route_mut.local_sdp = Some(sdp.clone());
        }
        Ok(sdp)
    }

    /// Trigger ICE restart and produce a fresh local SDP. Caller is
    /// responsible for re-signaling the resulting SDP to the remote peer.
    #[instrument(skip(self), fields(conn = %conn))]
    pub async fn restart_ice(&self, conn: &ConnectionId) -> Result<String> {
        let route = self.route(conn)?;
        route.peer.restart_ice().await?;
        let sdp = match route.peer.role() {
            PeerRole::Offerer => route.peer.renegotiate_as_offerer().await?,
            PeerRole::Answerer => {
                let offer = route
                    .remote_sdp
                    .clone()
                    .ok_or_else(|| WebRtcError::Sdp("no remote offer to restart against".into()))?;
                route.peer.renegotiate_as_answerer(&offer).await?
            }
        };
        if let Some(mut route_mut) = self.routes.get_mut(conn) {
            route_mut.local_sdp = Some(sdp.clone());
        }
        Ok(sdp)
    }
}

#[async_trait]
impl ConnectionAdapter for WebRtcAdapter {
    fn transport(&self) -> Transport {
        Transport::WebRtc
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

    fn supports_inbound_admission_confirmation(&self) -> bool {
        self.inbound_admission_confirmation_timeout.is_some()
    }

    fn notify_inbound_admission_outcome(
        &self,
        connection_id: &ConnectionId,
        lifecycle_generation: u64,
        accepted: bool,
    ) {
        let Some(route) = self.routes.get(connection_id) else {
            return;
        };
        let Some(waiter) = route.inbound_admission_waiter.as_ref() else {
            return;
        };
        waiter.resolve(lifecycle_generation, accepted, || {
            route.core_published.store(true, Ordering::Release);
        });
    }

    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> RvoipResult<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("WebRTC lifecycle sink already installed"))
    }

    fn is_connection_live(&self, conn: &ConnectionId) -> bool {
        self.routes.contains_key(conn)
    }

    fn take_inbound_context(&self, conn: &ConnectionId) -> Option<InboundConnectionContext> {
        self.routes
            .get(conn)
            .and_then(|route| route.inbound_context.lock().take())
    }

    #[instrument(skip(self, request), fields(session = %request.session_id))]
    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        let slot = self
            .reserve_session_slot()
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        self.metrics_outbound.fetch_add(1, Ordering::Relaxed);
        let conn_id = ConnectionId::new();
        let peer = RvoipPeerConnection::new(&self.config, PeerRole::Offerer)
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        // Pre-attach a video track when the caller wants outbound offers to
        // include `m=video`. `create_offer_and_gather` skips its auto-audio
        // path when *any* local track is already present, so we still need
        // an explicit audio attach below to keep symmetry with the default
        // behavior.
        if self.config.originate_include_video {
            peer.add_local_audio_track()
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
            peer.add_local_video_track()
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        }

        // Ensure the initial offer contains an SCTP m-line. Once the
        // association is negotiated, arbitrary labeled channels can be
        // opened later without media renegotiation.
        let bootstrap_reliability = DataReliability::ReliableOrdered;
        let bootstrap_options =
            crate::data_message::options_for_reliability(&bootstrap_reliability)
                .map_err(|error| RvoipError::Adapter(error.to_string()))?;
        let bootstrap_channel = peer
            .create_data_channel(OUTBOUND_MESSAGE_CHANNEL_LABEL, bootstrap_options)
            .await
            .map_err(|error| RvoipError::Adapter(error.to_string()))?;
        let bootstrap_key = crate::data_message::cache_key_parts(
            OUTBOUND_MESSAGE_CHANNEL_LABEL,
            &bootstrap_reliability,
        )
        .map_err(|error| RvoipError::Adapter(error.to_string()))?;

        let offer_sdp = peer
            .create_offer_and_gather()
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let negotiated = negotiate_audio(&request.capabilities, &self.config.capabilities)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let cancel = Arc::new(Notify::new());
        let (data_cancel, _) = watch::channel(false);
        let data_channels = Arc::new(DashMap::new());
        data_channels.insert(bootstrap_key, bootstrap_channel);
        let route = Route {
            peer,
            streams: Arc::new(DashMap::new()),
            local_sdp: Some(offer_sdp),
            remote_sdp: None,
            data_channel: data_channels,
            data_channel_create: Arc::new(AsyncMutex::new(())),
            data_channels_pumped: Arc::new(SyncMutex::new(HashSet::new())),
            data_channel_keys: Arc::new(DashMap::new()),
            data_pump_started: Arc::new(AtomicBool::new(false)),
            data_cancel: data_cancel.clone(),
            negotiated: negotiated.clone(),
            held: false,
            cancel: Arc::clone(&cancel),
            failed_at: Arc::new(SyncMutex::new(None)),
            core_published: Arc::new(AtomicBool::new(false)),
            core_handoff_started: Arc::new(AtomicBool::new(false)),
            authorization: None,
            inbound_context: Arc::new(SyncMutex::new(None)),
            inbound_admission_waiter: None,
        };

        // Same rationale as `apply_remote_offer`: lazy seeding in `accept()`.
        // Install the dormant stage before the route and its failure watcher
        // become visible so no pre-commit event can escape to core.
        self.outbound_event_stages
            .insert(conn_id.clone(), Arc::new(OutboundEventStage::default()));
        if let Err(error) = self.insert_route(conn_id.clone(), route) {
            self.outbound_event_stages.remove(&conn_id);
            return Err(RvoipError::Adapter(error.to_string()));
        }
        slot.commit();

        if !self.is_connection_live(&conn_id) {
            self.outbound_event_stages.remove(&conn_id);
            return Err(RvoipError::AdmissionRejected(
                "WebRTC outbound route ended before lifecycle activation",
            ));
        }

        let handle = self.make_transport_handle(conn_id.clone(), cancel, data_cancel);
        let mut connection =
            self.build_connection(conn_id, Direction::Outbound, negotiated, handle);
        connection.session_id = request.session_id;
        connection.participant_id = request.participant_id;

        Ok(ConnectionHandle::new(connection))
    }

    async fn activate_outbound(&self, conn: ConnectionId) -> RvoipResult<()> {
        if !self.is_connection_live(&conn) {
            self.outbound_event_stages.remove(&conn);
            return Err(RvoipError::ConnectionNotFound(conn));
        }
        let stage = self
            .outbound_event_stages
            .get(&conn)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        let mut state = stage
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match &mut *state {
            OutboundEventStageState::Activated => return Ok(()),
            OutboundEventStageState::Dormant { events, overflowed } => {
                if *overflowed {
                    events.clear();
                    return Err(RvoipError::AdmissionRejected(
                        "WebRTC outbound lifecycle event stage overflowed",
                    ));
                }
                let mut permits = self.events_tx.try_reserve_many(events.len()).map_err(|_| {
                    RvoipError::AdmissionRejected(
                        "WebRTC outbound lifecycle event publication was unavailable",
                    )
                })?;
                for (permit, event) in permits.by_ref().zip(events.drain(..)) {
                    permit.send(OrchestratorAdapterEvent::Public(event));
                }
                *state = OutboundEventStageState::Activated;
            }
        }
        drop(state);
        let Some(route) = self.routes.get(&conn) else {
            return Err(RvoipError::ConnectionNotFound(conn));
        };
        route.core_published.store(true, Ordering::Release);
        Ok(())
    }

    #[instrument(skip(self), fields(conn = %conn))]
    async fn accept(&self, conn: ConnectionId) -> RvoipResult<()> {
        if self.outbound_event_stages.contains_key(&conn) {
            self.activate_outbound(conn.clone()).await?;
        }
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        route
            .peer
            .wait_connected(Duration::from_secs(self.config.gather_timeout_secs + 10))
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        self.ensure_media_streams(&conn).await?;
        self.spawn_data_message_manager(conn.clone(), &route);
        self.try_send(AdapterEvent::Connected {
            connection_id: conn,
        });
        Ok(())
    }

    async fn reject(&self, conn: ConnectionId, _reason: RejectReason) -> RvoipResult<()> {
        self.outbound_event_stages.remove(&conn);
        if let Some((_, route)) = self.routes.remove(&conn) {
            route.cancel_tasks();
            self.release_session_slot();
            Self::deliver_terminal_event(
                &self.lifecycle,
                &self.events_tx,
                AdapterEvent::Failed {
                    connection_id: conn.clone(),
                    detail: "rejected".into(),
                },
                "reject",
            )
            .await;
            if tokio::time::timeout(DATA_CHANNEL_OPERATION_TIMEOUT, route.peer.close())
                .await
                .is_err()
            {
                warn!(connection_id = %conn, "WebRTC rejected peer close timed out");
            }
        }
        Ok(())
    }

    #[instrument(skip(self), fields(conn = %conn, reason = ?reason))]
    async fn end(&self, conn: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        self.outbound_event_stages.remove(&conn);
        if let Some((_, route)) = self.routes.remove(&conn) {
            route.cancel_tasks();
            self.release_session_slot();
            info!(conn = %conn, "ended");
            Self::deliver_terminal_event(
                &self.lifecycle,
                &self.events_tx,
                AdapterEvent::Ended {
                    connection_id: conn.clone(),
                    reason,
                },
                "end",
            )
            .await;
            if tokio::time::timeout(DATA_CHANNEL_OPERATION_TIMEOUT, route.peer.close())
                .await
                .is_err()
            {
                warn!(connection_id = %conn, "WebRTC ended peer close timed out");
            }
        }
        Ok(())
    }

    async fn hold(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        route
            .peer
            .hold_audio()
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        if self.config.hold_renegotiate {
            // Best-effort SDP renegotiation so peers that ignore mute also stop sending.
            self.renegotiate_after_direction_change(&conn)
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        }
        if let Some(mut route_mut) = self.routes.get_mut(&conn) {
            route_mut.held = true;
        }
        Ok(())
    }

    async fn resume(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        route
            .peer
            .resume_audio()
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        if self.config.hold_renegotiate {
            self.renegotiate_after_direction_change(&conn)
                .await
                .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        }
        if let Some(mut route_mut) = self.routes.get_mut(&conn) {
            route_mut.held = false;
        }
        Ok(())
    }

    async fn transfer(&self, _conn: ConnectionId, _target: TransferTarget) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented(
            "WebRTC transfer requires SIP REFER or renegotiation to a new peer; deferred in v1",
        ))
    }

    async fn streams(&self, conn: ConnectionId) -> RvoipResult<Vec<Arc<dyn MediaStream>>> {
        self.ensure_media_streams(&conn).await?;
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        Ok(route
            .streams
            .iter()
            .map(|e| Arc::clone(e.value()) as Arc<dyn MediaStream>)
            .collect())
    }

    async fn send_dtmf(
        &self,
        conn: ConnectionId,
        digits: &str,
        duration_ms: u32,
    ) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        crate::media::dtmf::send_dtmf(&route.peer, digits, duration_ms)
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))
    }

    async fn send_message(&self, conn: ConnectionId, message: Message) -> RvoipResult<()> {
        let content_type = legacy_message_content_type(&message.content_type);
        let data_message = DataMessage {
            label: OUTBOUND_MESSAGE_CHANNEL_LABEL.into(),
            content_type,
            bytes: message.body,
            reliability: DataReliability::ReliableOrdered,
            message_id: message.id,
        };
        self.send_data_message(conn, data_message).await
    }

    async fn send_data_message(&self, conn: ConnectionId, message: DataMessage) -> RvoipResult<()> {
        let encoded = crate::data_message::encode_data_message(&message)
            .map_err(|error| RvoipError::Adapter(error.to_string()))?;
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        let channel = self
            .data_channel_for_message(&conn, &route, &message)
            .await?;
        let send = async {
            match encoded {
                crate::data_message::EncodedDataMessage::Text(frame) => {
                    channel.send_text(&frame).await
                }
                crate::data_message::EncodedDataMessage::Binary(frame) => channel.send(frame).await,
            }
        };
        let result = match tokio::time::timeout(DATA_CHANNEL_OPERATION_TIMEOUT, send).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => RvoipError::Adapter(error.to_string()),
            Err(_) => RvoipError::Adapter("data channel send timed out".into()),
        };
        remove_cached_data_channel(&route.data_channel, &channel);
        let _ = channel.close().await;
        Err(result)
    }

    async fn renegotiate_media(
        &self,
        conn: ConnectionId,
        capabilities: CapabilityDescriptor,
    ) -> RvoipResult<NegotiatedCodecs> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let negotiated = negotiate_audio(&capabilities, &self.config.capabilities)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let offer = tokio::time::timeout(
            Duration::from_secs(2),
            route.peer.peer_connection().create_offer(None),
        )
        .await
        .map_err(|_| RvoipError::Adapter("create_offer timed out".into()))?
        .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        tokio::time::timeout(
            Duration::from_secs(2),
            route.peer.peer_connection().set_local_description(offer),
        )
        .await
        .map_err(|_| RvoipError::Adapter("set_local_description timed out".into()))?
        .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        if let Some(desc) = route.peer.peer_connection().local_description().await {
            if let Ok(sdp) = sdp_to_string(&desc) {
                if let Some(mut route_mut) = self.routes.get_mut(&conn) {
                    route_mut.local_sdp = Some(sdp);
                }
            }
        }

        Ok(negotiated)
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        match self.try_subscribe_events() {
            Ok(rx) => rx,
            Err(_) => {
                warn!(
                    "WebRtcAdapter::subscribe_events called more than once; \
                     returning closed receiver. Prefer try_subscribe_events() to detect."
                );
                let (_tx, rx) = mpsc::channel(1);
                rx
            }
        }
    }

    fn subscribe_orchestrator_events(&self) -> mpsc::Receiver<OrchestratorAdapterEvent> {
        match self.try_subscribe_atomic_events() {
            Ok(receiver) => receiver,
            Err(_) => {
                warn!(
                    "WebRtcAdapter atomic event stream was already consumed; returning closed receiver"
                );
                let (_sender, receiver) = mpsc::channel(1);
                receiver
            }
        }
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        self.config.capabilities.clone()
    }

    async fn verify_request_signature(
        &self,
        conn: ConnectionId,
        _signature: SignatureHeaders,
    ) -> RvoipResult<IdentityAssurance> {
        // D2 — surface the negotiated peer's DTLS fingerprint as the
        // assurance. The variant is key-binding without a real-world
        // identity (see `IdentityAssurance::DtlsFingerprint` docs); higher
        // assurance levels require a credential flow handled by auth-core
        // before this point. Returns `Anonymous` when there's no remote SDP
        // yet (outbound originate before the answer lands) or the route is
        // unknown.
        let fps = self.remote_dtls_fingerprint(&conn).unwrap_or_default();
        match fps.into_iter().next() {
            Some(fp) => Ok(IdentityAssurance::DtlsFingerprint {
                algorithm: fp.algorithm,
                value: fp.value,
            }),
            None => Ok(IdentityAssurance::Anonymous),
        }
    }
}

/// Guard returned by [`WebRtcAdapter::reserve_session_slot`]. Drops release
/// the slot; `commit()` promotes it to a permanent occupant (released when
/// the route is removed by `end`/`reject`/reaper).
struct SessionSlotGuard {
    live: Option<Arc<std::sync::atomic::AtomicUsize>>,
}

impl SessionSlotGuard {
    /// Promote this reservation into a held slot — the live counter stays
    /// incremented until the matching route is removed. Caller must ensure
    /// a matching release happens (handled in [`WebRtcAdapter::release_session_slot`]).
    fn commit(mut self) {
        self.live = None; // skip the Drop decrement
    }
}

impl Drop for SessionSlotGuard {
    fn drop(&mut self) {
        if let Some(live) = self.live.take() {
            live.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for WebRtcAdapter {
    fn drop(&mut self) {
        // Stop the reaper.
        self.reaper_cancel.notify_waiters();
        // Cancel each route's background tasks; peer connections will be dropped
        // when their Arc refcount hits zero.
        for entry in self.routes.iter() {
            entry.value().cancel_tasks();
        }
    }
}

fn remove_cached_data_channel(
    channels: &DashMap<String, Arc<dyn DataChannel>>,
    channel: &Arc<dyn DataChannel>,
) {
    let keys: Vec<String> = channels
        .iter()
        .filter(|entry| Arc::ptr_eq(entry.value(), channel))
        .map(|entry| entry.key().clone())
        .collect();
    for key in keys {
        let should_remove = channels
            .get(&key)
            .map(|entry| Arc::ptr_eq(entry.value(), channel))
            .unwrap_or(false);
        if should_remove {
            channels.remove(&key);
        }
    }
}

fn data_channel_identity(channel: &Arc<dyn DataChannel>) -> usize {
    Arc::as_ptr(channel) as *const () as usize
}

/// QUIC-bridge-flake fix — attach `track` to **every** stream in the
/// route. Idempotent via [`WebRtcMediaStream::attach_remote`]'s
/// `compare_exchange` guard.
async fn attach_track_to_streams(
    routes: &Arc<DashMap<ConnectionId, Route>>,
    conn: &ConnectionId,
    track: &Arc<dyn webrtc::media_stream::track_remote::TrackRemote>,
) {
    if let Some(route) = routes.get(conn) {
        for entry in route.streams.iter() {
            entry.value().attach_remote(track.clone());
        }
    }
}

/// QUIC-bridge-flake fix — same as above but only attach to streams of
/// the matching kind, so a future video track doesn't end up wired into
/// the audio inbound pump (and vice versa).
async fn attach_track_to_streams_matching(
    routes: &Arc<DashMap<ConnectionId, Route>>,
    conn: &ConnectionId,
    track: &Arc<dyn webrtc::media_stream::track_remote::TrackRemote>,
    kind: rvoip_core::stream::StreamKind,
) {
    if let Some(route) = routes.get(conn) {
        for entry in route.streams.iter() {
            if entry.value().kind() == kind {
                entry.value().attach_remote(track.clone());
            }
        }
    }
}

/// D4 follow-up — map a negotiated audio `CodecInfo` to the RTP payload
/// type the outbound pump should stamp on each packet. Matches the codec
/// table registered by
/// [`build_media_engine`](crate::peer::builder::build_media_engine).
fn payload_type_for_audio_codec(codec: &CodecInfo) -> u8 {
    let name = codec.name.to_ascii_lowercase();
    if name.contains("opus") {
        crate::media::pump::OPUS_PT_DEFAULT
    } else if name.contains("pcmu") || name.starts_with("g.711") && !name.contains("a-law") {
        0 // PCMU
    } else if name.contains("pcma") || name.contains("a-law") {
        8 // PCMA
    } else {
        // Fall back to Opus PT — the engine only registers a handful of
        // audio codecs and the negotiation path narrows to Opus by default.
        crate::media::pump::OPUS_PT_DEFAULT
    }
}

fn legacy_message_content_type(content_type: &ContentType) -> String {
    match content_type {
        ContentType::Text => "text/plain; charset=utf-8".into(),
        ContentType::Json => "application/json".into(),
        ContentType::Binary | ContentType::Image | ContentType::Audio => {
            "application/octet-stream".into()
        }
        ContentType::Attachment(value) => {
            let candidate = DataMessage::reliable(
                OUTBOUND_MESSAGE_CHANNEL_LABEL,
                value.clone(),
                bytes::Bytes::new(),
            );
            if candidate.validate().is_ok() {
                value.clone()
            } else {
                "application/octet-stream".into()
            }
        }
    }
}

/// Export SDP from a live peer connection (for WHIP/WHEP responses).
pub async fn export_local_sdp(peer: &Arc<RvoipPeerConnection>) -> Result<String> {
    let desc = peer
        .peer_connection()
        .local_description()
        .await
        .ok_or_else(|| WebRtcError::Sdp("no local description".into()))?;
    sdp_to_string(&desc)
}

#[cfg(test)]
mod inbound_hardening_tests {
    use super::*;
    use rvoip_core::identity::{AuthenticationMethod, CredentialKind};
    use std::sync::atomic::AtomicUsize;

    struct RecordingLifecycleSink {
        deliveries: AtomicUsize,
    }

    #[async_trait]
    impl AdapterLifecycleSink for RecordingLifecycleSink {
        async fn deliver_terminal(&self, _event: AdapterEvent) {
            self.deliveries.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn principal(
        tenant: Option<&str>,
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> AuthenticatedPrincipal {
        AuthenticatedPrincipal {
            subject: "webrtc-owner".into(),
            tenant: tenant.map(str::to_owned),
            scopes: vec!["webrtc:connect".into()],
            issuer: Some("webrtc-hardening-test".into()),
            expires_at,
            method: AuthenticationMethod::Jwt,
            assurance: IdentityAssurance::Identified {
                credential_kind: CredentialKind::Oidc,
            },
        }
    }

    fn inbound_connection(connection_id: ConnectionId) -> Connection {
        Connection {
            id: connection_id,
            session_id: rvoip_core::ids::SessionId::new(),
            participant_id: rvoip_core::ids::ParticipantId::new(),
            transport: Transport::WebRtc,
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

    #[tokio::test]
    async fn admission_waiter_is_generation_aware_idempotent_and_fail_closed() {
        let waiter = InboundAdmissionWaiter::new();
        waiter.resolve(7, false, || panic!("reject callback cannot accept"));
        waiter.resolve(7, false, || panic!("duplicate reject cannot accept"));
        waiter.resolve(7, true, || {
            panic!("contradictory duplicate must be ignored")
        });
        waiter.resolve(8, true, || panic!("stale generation must be ignored"));
        assert_eq!(
            waiter.wait(Duration::from_millis(10)).await,
            InboundAdmissionOutcome::Rejected
        );

        let accepted = InboundAdmissionWaiter::new();
        let published = AtomicBool::new(false);
        accepted.resolve(3, true, || published.store(true, Ordering::Release));
        assert!(published.load(Ordering::Acquire));
        accepted.cancel();
        assert_eq!(
            accepted.wait(Duration::from_millis(10)).await,
            InboundAdmissionOutcome::Cancelled,
            "local teardown overrides an unread accepted update"
        );
    }

    #[tokio::test]
    async fn secure_admission_is_explicit_and_timeout_bounded() {
        let legacy = WebRtcAdapter::new(WebRtcConfig::loopback());
        assert!(!legacy.supports_inbound_admission_confirmation());
        assert_eq!(legacy.inbound_admission_confirmation_timeout(), None);

        let secure = WebRtcAdapter::new_with_inbound_admission_confirmation(
            WebRtcConfig::loopback(),
            Duration::from_secs(2),
        )
        .expect("valid secure adapter");
        assert!(secure.supports_inbound_admission_confirmation());
        assert_eq!(
            secure.inbound_admission_confirmation_timeout(),
            Some(Duration::from_secs(2))
        );

        assert!(matches!(
            WebRtcAdapter::new_with_inbound_admission_confirmation(
                WebRtcConfig::loopback(),
                Duration::ZERO
            ),
            Err(WebRtcError::InvalidArgument(_))
        ));
        assert!(matches!(
            WebRtcAdapter::new_with_inbound_admission_confirmation(
                WebRtcConfig::loopback(),
                MAX_INBOUND_ADMISSION_CONFIRMATION_TIMEOUT + Duration::from_millis(1)
            ),
            Err(WebRtcError::InvalidArgument(_))
        ));
    }

    #[cfg(any(feature = "signaling-whip", feature = "signaling-ws"))]
    #[tokio::test]
    async fn secure_request_rejects_anonymous_identity_only_tenantless_and_missing_hint() {
        let secure = WebRtcAdapter::new_with_inbound_admission_confirmation(
            WebRtcConfig::loopback(),
            Duration::from_secs(1),
        )
        .expect("secure adapter");
        let hint = InboundRoutingHint::new("attachment").unwrap();
        assert!(matches!(
            secure.validate_secure_inbound_request(None, Some(&hint)),
            Err(WebRtcError::InboundAdmissionRejected)
        ));
        let identity_only = RouteAuthorization::legacy_subject("legacy-user");
        assert!(matches!(
            secure.validate_secure_inbound_request(Some(&identity_only), Some(&hint)),
            Err(WebRtcError::InboundAdmissionRejected)
        ));
        let tenantless = RouteAuthorization::principal(principal(None, None));
        assert!(matches!(
            secure.validate_secure_inbound_request(Some(&tenantless), Some(&hint)),
            Err(WebRtcError::InboundAdmissionRejected)
        ));
        let complete = RouteAuthorization::principal(principal(Some("tenant-a"), None));
        assert!(matches!(
            secure.validate_secure_inbound_request(Some(&complete), None),
            Err(WebRtcError::InboundAdmissionRejected)
        ));
        secure
            .validate_secure_inbound_request(Some(&complete), Some(&hint))
            .expect("complete principal and hint");
    }

    #[test]
    fn publication_boundary_rejects_tenantless_and_newly_expired_principals() {
        let connection_id = ConnectionId::new();
        let tenantless = RouteAuthorization::principal(principal(None, None));
        assert!(matches!(
            WebRtcAdapter::authenticated_inbound_context(
                connection_id.clone(),
                &tenantless,
                Some(InboundRoutingHint::new("tenantless").unwrap()),
            ),
            Err(WebRtcError::Unauthorized(detail)) if detail == "authenticated principal has no tenant"
        ));

        let expires_at = Utc::now() + chrono::Duration::milliseconds(5);
        let expiring = RouteAuthorization::principal(principal(Some("tenant-a"), Some(expires_at)));
        expiring.ensure_active().expect("principal starts active");
        std::thread::sleep(Duration::from_millis(10));
        assert!(matches!(
            WebRtcAdapter::authenticated_inbound_context(
                connection_id,
                &expiring,
                Some(InboundRoutingHint::new("expired-after-gather").unwrap()),
            ),
            Err(WebRtcError::Unauthorized(detail)) if detail == "authenticated principal has expired"
        ));
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
            participant_id: "webrtc-owner".into(),
            principal: principal(Some("tenant-a"), None),
        };
        let sender = events_tx.clone();
        let send =
            tokio::spawn(async move { WebRtcAdapter::send_inbound_event_to(&sender, event).await });
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
        WebRtcAdapter::deliver_terminal_event(
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
        WebRtcAdapter::deliver_terminal_event(
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
    async fn outbound_events_remain_fifo_and_activation_is_all_or_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let adapter = WebRtcAdapter::new(WebRtcConfig::loopback());
        let mut events = adapter
            .try_subscribe_atomic_events()
            .expect("atomic events");
        let handle = adapter
            .originate(OriginateRequest {
                session_id: rvoip_core::ids::SessionId::new(),
                participant_id: rvoip_core::ids::ParticipantId::new(),
                target: String::new(),
                direction: Direction::Outbound,
                capabilities: adapter.capabilities(),
                transport: None,
                context: Default::default(),
            })
            .await
            .expect("outbound route");
        let connection_id = handle.connection.id;
        adapter
            .bind_authenticated_principal(
                &connection_id,
                "webrtc-owner",
                principal(Some("tenant-a"), None),
            )
            .expect("stage principal");
        adapter.try_send(AdapterEvent::Dtmf {
            connection_id: connection_id.clone(),
            digits: "7".into(),
            duration_ms: 100,
        });

        for _ in 0..(ADAPTER_EVENT_CAP - 1) {
            adapter
                .events_tx
                .try_send(OrchestratorAdapterEvent::Public(AdapterEvent::Native {
                    kind: "queue-filler",
                    detail: String::new(),
                }))
                .expect("fill all but one queue slot");
        }
        assert!(adapter
            .activate_outbound(connection_id.clone())
            .await
            .is_err());
        for _ in 0..(ADAPTER_EVENT_CAP - 1) {
            assert!(matches!(
                events.recv().await,
                Some(OrchestratorAdapterEvent::Public(AdapterEvent::Native {
                    kind: "queue-filler",
                    ..
                }))
            ));
        }
        assert!(matches!(
            events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("retry flushes intact FIFO");
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(
                AdapterEvent::PrincipalAuthenticated { connection_id: id, .. }
            )) if id == connection_id
        ));
        assert!(matches!(
            events.recv().await,
            Some(OrchestratorAdapterEvent::Public(AdapterEvent::Dtmf { connection_id: id, digits, .. }))
                if id == connection_id && digits == "7"
        ));

        adapter
            .end(connection_id, EndReason::Normal)
            .await
            .expect("cleanup route");
    }
}
