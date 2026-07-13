//! Batteries-included SIP → Amazon Connect screen-pop server.
//!
//! [`ConnectScreenPopServer`] is the turnkey entry point: give it one
//! [`ScreenPopServerConfig`] and call [`ConnectScreenPopServer::serve`]. It
//! stands up a SIP UAS, and for every inbound INVITE (e.g. a Vapi blind
//! transfer) it:
//!
//! 1. reads the custom SIP headers,
//! 2. translates them to Amazon Connect contact attributes (the screen-pop
//!    channel) via the configured [`AttributeMapping`],
//! 3. answers the SIP leg,
//! 4. places an inbound WebRTC contact into Connect ([`AmazonConnectAdapter`]),
//! 5. bridges the SIP audio (G.711) to the Connect audio (Opus), transcoding.
//!
//! The Connect contact flow + agent CCP then perform the actual screen pop from
//! the attributes (an AWS-side configuration task).

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex as SyncMutex;
use rvoip_core::adapter::{AdapterEvent, ConnectionAdapter, EndReason};
use rvoip_core::ids::ConnectionId;
use rvoip_core::stream::MediaStream;
use rvoip_sip::{
    Config as SipConfig, Event as SipEvent, IncomingCall, SessionId as SipSessionId,
    UnifiedCoordinator,
};
use rvoip_sip_core::types::headers::{HeaderAccess, HeaderName, TypedHeader};
use tokio::sync::{broadcast, watch};
use tracing::{info, warn};

use crate::adapter::{
    AmazonConnectAdapter, ContactSetupObserver, ContactSetupStage, ContactTarget,
};
use crate::bridge::{bridge_streams, StreamBridge};
use crate::config::ConnectConfig;
use crate::control::ConnectContactStarter;
use crate::errors::{ConnectError, Result};
use crate::mapping::AttributeMapping;
use crate::media::ConnectMediaConnector;

/// The Connect target a [`ContactRouter`] selected for one inbound call.
///
/// Every `None` field falls back to the server-wide [`ConnectConfig`] /
/// [`AttributeMapping`], so a route only needs to carry what differs per
/// tenant.
#[derive(Clone, Default)]
pub struct ContactRoute {
    /// Metrics/logging label for this route (e.g. the tenant name). Keyed
    /// into [`ConnectScreenPopServer::route_metrics`].
    pub label: String,
    /// Amazon Connect instance id override.
    pub instance_id: Option<String>,
    /// Contact-flow id override.
    pub contact_flow_id: Option<String>,
    /// Per-route SIP-header → attribute mapping override.
    pub attribute_mapping: Option<AttributeMapping>,
    /// Display-name fallback override (used when the INVITE supplies none).
    pub default_display_name: Option<String>,
}

impl std::fmt::Debug for ContactRoute {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContactRoute")
            .field("label_present", &!self.label.is_empty())
            .field("instance_id_present", &self.instance_id.is_some())
            .field("contact_flow_id_present", &self.contact_flow_id.is_some())
            .field(
                "attribute_mapping_present",
                &self.attribute_mapping.is_some(),
            )
            .field(
                "default_display_name_present",
                &self.default_display_name.is_some(),
            )
            .finish()
    }
}

/// A [`ContactRouter`]'s verdict for one inbound INVITE.
pub enum RouteDecision {
    /// Bridge the call into Connect with these per-call parameters.
    Route(ContactRoute),
    /// Reject the INVITE with this SIP status/reason (e.g. `404 Not Found`
    /// for an unknown tenant).
    Reject {
        /// SIP status code (4xx/5xx/6xx).
        status: u16,
        /// SIP reason phrase.
        reason: String,
    },
}

/// Per-call routing hook: inspect the inbound INVITE (Request-URI / To user
/// part, headers, …) and pick the Connect target — the multi-tenant enabler.
pub type ContactRouter = Arc<dyn Fn(&IncomingCall) -> RouteDecision + Send + Sync>;

/// Configuration for the turnkey screen-pop server — one object, batteries
/// included.
pub struct ScreenPopServerConfig {
    /// SIP UAS settings (bind address, local URI, timers). Build with
    /// `rvoip_sip::Config::local(name, port)` or `Config::on(name, ip, port)`.
    pub sip: SipConfig,
    /// Amazon Connect control + media settings (instance/flow/region, mapping,
    /// timeouts).
    pub connect: ConnectConfig,
    /// The control-plane starter. Use `AwsConnectStarter` (feature
    /// `aws-control`) for the real path, or a mock in tests.
    pub starter: Arc<dyn ConnectContactStarter>,
    /// Optional per-call router. `None` preserves the classic behaviour:
    /// every INVITE goes to `connect`'s instance/flow with its mapping.
    pub router: Option<ContactRouter>,
}

impl ScreenPopServerConfig {
    /// Construct with the three required pieces (no per-call routing).
    pub fn new(
        sip: SipConfig,
        connect: ConnectConfig,
        starter: Arc<dyn ConnectContactStarter>,
    ) -> Self {
        Self {
            sip,
            connect,
            starter,
            router: None,
        }
    }

    /// Set the per-call router (builder-style).
    pub fn with_router(mut self, router: ContactRouter) -> Self {
        self.router = Some(router);
        self
    }
}

/// Active bridged contact: keeps the SIP↔Connect bridge alive and remembers the
/// Connect connection so it can be torn down when the SIP leg ends.
struct ActiveContact {
    _bridge: StreamBridge,
    sip_graph: rvoip_core::media_graph::MediaGraphHandle,
    connect_graph: rvoip_core::media_graph::MediaGraphHandle,
    connect_conn: ConnectionId,
    /// Route label for per-route metrics (`None` on the unrouted path).
    route_label: Option<String>,
    setup: Arc<SetupAttempt>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScreenPopMediaLeg {
    Sip,
    Connect,
}

/// Sanitized lifecycle stages exposed to authenticated control planes. No SIP
/// headers, AWS contact ids, SDP, credentials, or error strings are included.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenPopLifecycleStage {
    SipInviteReceived,
    AttributesMapped,
    ContactStarted,
    MediaConnected,
    TeardownStarted,
    Terminated,
    Failed,
}

/// One sanitized screen-pop lifecycle notification.
#[derive(Clone, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ScreenPopLifecycleEvent {
    pub stage: ScreenPopLifecycleStage,
    /// Sanitized, length-bounded correlation id from `X-Correlation-Id`.
    pub correlation_id: Option<String>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for ScreenPopLifecycleEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScreenPopLifecycleEvent")
            .field("stage", &self.stage)
            .field("correlation_id_present", &self.correlation_id.is_some())
            .field("occurred_at", &self.occurred_at)
            .finish()
    }
}

#[derive(Default)]
struct SetupAttemptState {
    cancelled: bool,
    cleanup_claimed: bool,
    promoted: bool,
    connect_conn: Option<ConnectionId>,
    lifecycle_stage: Option<ScreenPopLifecycleStage>,
    cleanup_attempt_complete: bool,
}

struct SetupAttempt {
    session_id: SipSessionId,
    correlation_id: Option<String>,
    state: SyncMutex<SetupAttemptState>,
    cancel_tx: watch::Sender<bool>,
}

impl SetupAttempt {
    fn new(session_id: SipSessionId, correlation_id: Option<String>) -> Arc<Self> {
        let (cancel_tx, _) = watch::channel(false);
        Arc::new(Self {
            session_id,
            correlation_id,
            state: SyncMutex::new(SetupAttemptState::default()),
            cancel_tx,
        })
    }

    async fn cancelled(&self) {
        let mut rx = self.cancel_tx.subscribe();
        if *rx.borrow() {
            return;
        }
        let _ = rx.wait_for(|cancelled| *cancelled).await;
    }

    fn signal_cancel(&self) {
        self.cancel_tx.send_replace(true);
    }

    fn transition(&self, next: ScreenPopLifecycleStage) -> bool {
        let mut state = self.state.lock();
        if matches!(
            next,
            ScreenPopLifecycleStage::Terminated | ScreenPopLifecycleStage::Failed
        ) && !state.cleanup_attempt_complete
        {
            return false;
        }
        let next_rank = lifecycle_rank(next);
        if state
            .lifecycle_stage
            .is_some_and(|current| lifecycle_rank(current) >= next_rank)
        {
            return false;
        }
        state.lifecycle_stage = Some(next);
        true
    }

    fn mark_cleanup_attempt_complete(&self) {
        self.state.lock().cleanup_attempt_complete = true;
    }
}

fn lifecycle_rank(stage: ScreenPopLifecycleStage) -> u8 {
    match stage {
        ScreenPopLifecycleStage::SipInviteReceived => 0,
        ScreenPopLifecycleStage::AttributesMapped => 1,
        ScreenPopLifecycleStage::ContactStarted => 2,
        ScreenPopLifecycleStage::MediaConnected => 3,
        ScreenPopLifecycleStage::TeardownStarted => 4,
        ScreenPopLifecycleStage::Terminated | ScreenPopLifecycleStage::Failed => 5,
    }
}

enum ContactClaim<T> {
    Setup {
        attempt: Arc<SetupAttempt>,
        connect_conn: Option<ConnectionId>,
    },
    Active(T),
    EarlySipEnd,
    EarlyConnectEnd,
    None,
}

const EVENT_TOMBSTONE_CAPACITY: usize = 4_096;
const EVENT_TOMBSTONE_TTL: Duration = Duration::from_secs(120);

struct BoundedTombstones<K> {
    entries: SyncMutex<HashMap<K, Instant>>,
}

impl<K> Default for BoundedTombstones<K> {
    fn default() -> Self {
        Self {
            entries: SyncMutex::new(HashMap::new()),
        }
    }
}

impl<K> BoundedTombstones<K>
where
    K: Clone + Eq + Hash,
{
    fn prune(entries: &mut HashMap<K, Instant>, now: Instant) {
        entries.retain(|_, inserted| now.duration_since(*inserted) <= EVENT_TOMBSTONE_TTL);
        while entries.len() >= EVENT_TOMBSTONE_CAPACITY {
            let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, inserted)| **inserted)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            entries.remove(&oldest);
        }
    }

    fn insert(&self, key: K) {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        Self::prune(&mut entries, now);
        entries.insert(key, now);
    }

    fn take(&self, key: &K) -> bool {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        Self::prune(&mut entries, now);
        entries.remove(key).is_some()
    }

    fn contains(&self, key: &K) -> bool {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        Self::prune(&mut entries, now);
        entries.contains_key(key)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.lock().len()
    }
}

/// All setup/active/reverse indexes share this registry. Promotion and setup
/// cancellation serialize on `SetupAttempt::state`, eliminating the old gap
/// where reverse routing existed but the active entry did not.
struct ContactRegistry<T> {
    setups: DashMap<SipSessionId, Arc<SetupAttempt>>,
    active: DashMap<SipSessionId, T>,
    by_connect: DashMap<ConnectionId, SipSessionId>,
    early_sip_ends: BoundedTombstones<SipSessionId>,
    early_connect_ends: BoundedTombstones<ConnectionId>,
    finished_sip: BoundedTombstones<SipSessionId>,
    finished_connect: BoundedTombstones<ConnectionId>,
    routing_lock: SyncMutex<()>,
}

impl<T> Default for ContactRegistry<T> {
    fn default() -> Self {
        Self {
            setups: DashMap::new(),
            active: DashMap::new(),
            by_connect: DashMap::new(),
            early_sip_ends: BoundedTombstones::default(),
            early_connect_ends: BoundedTombstones::default(),
            finished_sip: BoundedTombstones::default(),
            finished_connect: BoundedTombstones::default(),
            routing_lock: SyncMutex::new(()),
        }
    }
}

impl<T> ContactRegistry<T> {
    fn register(
        &self,
        session_id: SipSessionId,
        correlation_id: Option<String>,
    ) -> Option<Arc<SetupAttempt>> {
        use dashmap::mapref::entry::Entry;
        let _routing = self.routing_lock.lock();
        if self.active.contains_key(&session_id) || self.finished_sip.contains(&session_id) {
            return None;
        }
        match self.setups.entry(session_id.clone()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => {
                let attempt = SetupAttempt::new(session_id, correlation_id);
                if self.early_sip_ends.take(&attempt.session_id) {
                    attempt.state.lock().cancelled = true;
                    attempt.signal_cancel();
                }
                entry.insert(Arc::clone(&attempt));
                Some(attempt)
            }
        }
    }

    fn bind_connect(&self, attempt: &Arc<SetupAttempt>, conn: ConnectionId) -> bool {
        let _routing = self.routing_lock.lock();
        let mut state = attempt.state.lock();
        if state.cancelled || state.cleanup_claimed {
            return false;
        }
        state.connect_conn = Some(conn.clone());
        self.by_connect
            .insert(conn.clone(), attempt.session_id.clone());
        if self.early_connect_ends.take(&conn) {
            state.cancelled = true;
            attempt.signal_cancel();
            return false;
        }
        true
    }

    fn promote(&self, attempt: &Arc<SetupAttempt>, active: T) -> std::result::Result<(), T> {
        use dashmap::mapref::entry::Entry;
        let _routing = self.routing_lock.lock();
        let mut state = attempt.state.lock();
        if state.cancelled || state.cleanup_claimed {
            return Err(active);
        }
        match self.active.entry(attempt.session_id.clone()) {
            Entry::Occupied(_) => return Err(active),
            Entry::Vacant(entry) => {
                entry.insert(active);
            }
        }
        state.promoted = true;
        drop(state);
        self.setups.remove(&attempt.session_id);
        Ok(())
    }

    fn claim_sip(&self, session_id: &SipSessionId) -> ContactClaim<T> {
        let attempt = {
            let _routing = self.routing_lock.lock();
            self.setups
                .get(session_id)
                .map(|entry| Arc::clone(entry.value()))
        };
        if let Some(attempt) = attempt {
            let mut state = attempt.state.lock();
            if state.promoted {
                drop(state);
                return self.claim_active_or_early(session_id);
            }
            if state.cleanup_claimed {
                return ContactClaim::None;
            }
            state.cancelled = true;
            state.cleanup_claimed = true;
            let connect_conn = state.connect_conn.clone();
            drop(state);
            attempt.signal_cancel();
            return ContactClaim::Setup {
                attempt,
                connect_conn,
            };
        }
        self.claim_active_or_early(session_id)
    }

    fn claim_active_or_early(&self, session_id: &SipSessionId) -> ContactClaim<T> {
        let _routing = self.routing_lock.lock();
        if let Some((_, active)) = self.active.remove(session_id) {
            // Mark finished while holding the same lock that register uses,
            // before cleanup performs any await.
            self.finished_sip.insert(session_id.clone());
            ContactClaim::Active(active)
        } else if self.finished_sip.contains(session_id) {
            ContactClaim::None
        } else {
            self.early_sip_ends.insert(session_id.clone());
            ContactClaim::EarlySipEnd
        }
    }

    fn claim_connect(&self, conn: &ConnectionId) -> ContactClaim<T> {
        let session_id = {
            let _routing = self.routing_lock.lock();
            let Some(session_id) = self.by_connect.get(conn).map(|entry| entry.value().clone())
            else {
                if self.finished_connect.contains(conn) {
                    return ContactClaim::None;
                }
                self.early_connect_ends.insert(conn.clone());
                return ContactClaim::EarlyConnectEnd;
            };
            session_id
        };
        self.claim_sip(&session_id)
    }

    fn finish(&self, attempt: &SetupAttempt, conn: Option<&ConnectionId>) {
        self.setups.remove(&attempt.session_id);
        self.early_sip_ends.take(&attempt.session_id);
        self.finished_sip.insert(attempt.session_id.clone());
        if let Some(conn) = conn {
            let _routing = self.routing_lock.lock();
            self.by_connect.remove(conn);
            self.early_connect_ends.take(conn);
            self.finished_connect.insert(conn.clone());
        }
    }

    #[cfg(test)]
    fn live_is_empty(&self) -> bool {
        self.setups.is_empty()
            && self.active.is_empty()
            && self.by_connect.is_empty()
            && self.early_sip_ends.len() == 0
            && self.early_connect_ends.len() == 0
    }
}

#[async_trait]
trait EstablishedConnectionCleanup: Send + Sync {
    async fn end_established(&self, connection_id: ConnectionId);
}

#[async_trait]
impl EstablishedConnectionCleanup for AmazonConnectAdapter {
    async fn end_established(&self, connection_id: ConnectionId) {
        if let Err(error) = self.end(connection_id, EndReason::Normal).await {
            warn!(%error, "failed to end Connect connection completed after cancellation");
        }
    }
}

/// Owns the establishment task independently of the caller future. If the
/// caller is cancelled or dropped, `Drop` installs a consumer that waits for
/// setup and immediately ends any connection it returns.
struct EstablishmentOwner {
    task: Option<tokio::task::JoinHandle<Result<ConnectionId>>>,
    cleanup: Arc<dyn EstablishedConnectionCleanup>,
}

impl EstablishmentOwner {
    fn new(
        task: tokio::task::JoinHandle<Result<ConnectionId>>,
        cleanup: Arc<dyn EstablishedConnectionCleanup>,
    ) -> Self {
        Self {
            task: Some(task),
            cleanup,
        }
    }

    async fn wait(&mut self) -> Result<ConnectionId> {
        let result = match self.task.as_mut() {
            Some(task) => task
                .await
                .map_err(|error| ConnectError::Signaling(format!("Connect setup task: {error}")))?,
            None => {
                return Err(ConnectError::Signaling(
                    "Connect setup task was already consumed".into(),
                ))
            }
        };
        self.task.take();
        result
    }
}

impl Drop for EstablishmentOwner {
    fn drop(&mut self) {
        let Some(task) = self.task.take() else {
            return;
        };
        let cleanup = Arc::clone(&self.cleanup);
        tokio::spawn(async move {
            if let Ok(Ok(connection_id)) = task.await {
                cleanup.end_established(connection_id).await;
            }
        });
    }
}

#[async_trait]
trait ScreenPopCleanupActions: Send + Sync {
    async fn hangup_sip(&self, session_id: &SipSessionId) -> std::result::Result<(), String>;
    async fn stop_connect(&self, connection_id: &ConnectionId) -> std::result::Result<(), String>;
}

struct RuntimeCleanupActions {
    coordinator: Arc<UnifiedCoordinator>,
    adapter: Arc<AmazonConnectAdapter>,
}

#[async_trait]
impl ScreenPopCleanupActions for RuntimeCleanupActions {
    async fn hangup_sip(&self, session_id: &SipSessionId) -> std::result::Result<(), String> {
        match self.coordinator.hangup(session_id).await {
            Ok(()) => Ok(()),
            Err(error) => {
                let detail = error.to_string();
                let normalized = detail.to_ascii_lowercase();
                if normalized.contains("already")
                    || normalized.contains("ended")
                    || normalized.contains("not found")
                    || normalized.contains("unknown")
                {
                    Ok(())
                } else {
                    Err(detail)
                }
            }
        }
    }

    async fn stop_connect(&self, connection_id: &ConnectionId) -> std::result::Result<(), String> {
        self.adapter
            .end(connection_id.clone(), EndReason::Normal)
            .await
            .map_err(|error| error.to_string())
    }
}

async fn release_resources(
    cleanup: &dyn ScreenPopCleanupActions,
    session_id: &SipSessionId,
    connection_id: Option<&ConnectionId>,
) -> std::result::Result<(), Vec<String>> {
    let mut failures = Vec::new();
    if let Err(error) = cleanup.hangup_sip(session_id).await {
        failures.push(format!("SIP hangup: {error}"));
    }
    if let Some(connection_id) = connection_id {
        if let Err(error) = cleanup.stop_connect(connection_id).await {
            failures.push(format!("Connect stop: {error}"));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// Per-route (per-tenant) counters, updated by `handle_call`/teardown.
#[derive(Default)]
struct RouteStats {
    contacts_started: AtomicU64,
    failures: AtomicU64,
    active_sessions: AtomicI64,
}

/// Snapshot of one route's counters (see
/// [`ConnectScreenPopServer::route_metrics`]).
#[derive(Clone, Debug, Default)]
pub struct RouteMetrics {
    /// Contacts successfully started (StartWebRTCContact succeeded).
    pub contacts_started: u64,
    /// Calls that failed anywhere between accept and bridge.
    pub failures: u64,
    /// Currently bridged calls.
    pub active_sessions: u64,
}

/// The running server.
pub struct ConnectScreenPopServer {
    coordinator: Arc<UnifiedCoordinator>,
    adapter: Arc<AmazonConnectAdapter>,
    mapping: AttributeMapping,
    router: Option<ContactRouter>,
    /// Per-route-label counters; populated only when a router is configured.
    route_stats: DashMap<String, Arc<RouteStats>>,
    /// Authoritative map of live bridges, keyed by SIP session. Removal from
    /// this map is the single teardown "claim" so the two directions
    /// (SIP-ended, Connect-ended) never double-tear-down.
    registry: Arc<ContactRegistry<ActiveContact>>,
    lifecycle_tx: broadcast::Sender<ScreenPopLifecycleEvent>,
    cleanup: Arc<dyn ScreenPopCleanupActions>,
}

impl ConnectScreenPopServer {
    /// Build the server: start the SIP coordinator and the Connect adapter.
    pub async fn build(config: ScreenPopServerConfig) -> Result<Arc<Self>> {
        Self::build_inner(config, None).await
    }

    /// Build the server with an explicit Amazon media connector.
    ///
    /// This additive construction seam is intended for hermetic integration
    /// tests and specialized media policy. [`Self::build`] continues to install
    /// the production Chime + rvoip-WebRTC connector.
    pub async fn build_with_media_connector(
        config: ScreenPopServerConfig,
        media_connector: Arc<dyn ConnectMediaConnector>,
    ) -> Result<Arc<Self>> {
        Self::build_inner(config, Some(media_connector)).await
    }

    async fn build_inner(
        config: ScreenPopServerConfig,
        media_connector: Option<Arc<dyn ConnectMediaConnector>>,
    ) -> Result<Arc<Self>> {
        let mapping = config.connect.attribute_mapping.clone();
        let coordinator = UnifiedCoordinator::new(config.sip)
            .await
            .map_err(|e| ConnectError::Signaling(format!("SIP coordinator: {e}")))?;
        let adapter = match media_connector {
            Some(media_connector) => AmazonConnectAdapter::builder(config.connect, config.starter)
                .with_media_connector(media_connector)
                .build(),
            None => AmazonConnectAdapter::new(config.connect, config.starter),
        };
        let cleanup: Arc<dyn ScreenPopCleanupActions> = Arc::new(RuntimeCleanupActions {
            coordinator: Arc::clone(&coordinator),
            adapter: Arc::clone(&adapter),
        });
        let (lifecycle_tx, _) = broadcast::channel(256);

        Ok(Arc::new(Self {
            coordinator,
            adapter,
            mapping,
            router: config.router,
            route_stats: DashMap::new(),
            registry: Arc::new(ContactRegistry::default()),
            lifecycle_tx,
            cleanup,
        }))
    }

    /// The underlying Connect adapter (e.g. to read metrics).
    pub fn adapter(&self) -> &Arc<AmazonConnectAdapter> {
        &self.adapter
    }

    /// Subscribe to sanitized lifecycle events. Authentication and tenant
    /// authorization belong to the caller exposing these diagnostics.
    pub fn subscribe_lifecycle(&self) -> broadcast::Receiver<ScreenPopLifecycleEvent> {
        self.lifecycle_tx.subscribe()
    }

    fn emit_lifecycle(&self, attempt: &SetupAttempt, stage: ScreenPopLifecycleStage) {
        if !attempt.transition(stage) {
            return;
        }
        let _ = self.lifecycle_tx.send(ScreenPopLifecycleEvent {
            stage,
            correlation_id: attempt.correlation_id.clone(),
            occurred_at: chrono::Utc::now(),
        });
    }

    fn emit_terminal_once(&self, attempt: &SetupAttempt, stage: ScreenPopLifecycleStage) {
        self.emit_lifecycle(attempt, stage);
    }

    /// Clone the source graph for an active screen-pop call. This is the
    /// observer seam used by Bridgefu to add UCTP or MOQT broadcast sinks
    /// without competing with the SIP↔Connect bridge for `frames_in()`.
    pub fn media_graph(
        &self,
        call_id: &str,
        leg: ScreenPopMediaLeg,
    ) -> Option<rvoip_core::media_graph::MediaGraphHandle> {
        self.registry
            .active
            .iter()
            .find(|entry| entry.key().to_string() == call_id)
            .map(|entry| match leg {
                ScreenPopMediaLeg::Sip => entry.value().sip_graph.clone(),
                ScreenPopMediaLeg::Connect => entry.value().connect_graph.clone(),
            })
    }

    pub fn active_call_ids(&self) -> Vec<String> {
        self.registry
            .active
            .iter()
            .map(|entry| entry.key().to_string())
            .collect()
    }

    /// Snapshot of the per-route counters, keyed by [`ContactRoute::label`].
    /// Empty when no router is configured (use
    /// [`AmazonConnectAdapter::metrics`] for the process-wide view).
    pub fn route_metrics(&self) -> BTreeMap<String, RouteMetrics> {
        self.route_stats
            .iter()
            .map(|e| {
                (
                    e.key().clone(),
                    RouteMetrics {
                        contacts_started: e.contacts_started.load(Ordering::Relaxed),
                        failures: e.failures.load(Ordering::Relaxed),
                        active_sessions: e.active_sessions.load(Ordering::Relaxed).max(0) as u64,
                    },
                )
            })
            .collect()
    }

    fn stats_for(&self, label: &str) -> Arc<RouteStats> {
        self.route_stats
            .entry(label.to_string())
            .or_default()
            .clone()
    }

    /// Run the accept loop forever: each inbound INVITE is translated, the
    /// Connect contact is placed, and the two legs are bridged. Per-call
    /// failures are logged and skipped; the loop continues.
    pub async fn serve(self: Arc<Self>) -> Result<()> {
        // Bidirectional teardown:
        //  • SIP leg ends  → LEAVE the Chime meeting (spawn_teardown_watcher).
        //  • Connect leg ends (agent hangup) → BYE the SIP carrier
        //    (spawn_connect_end_watcher).
        self.spawn_teardown_watcher().await?;
        self.spawn_connect_end_watcher();

        let mut events = self
            .coordinator
            .events()
            .await
            .map_err(|e| ConnectError::Signaling(format!("SIP events: {e}")))?;
        info!("ConnectScreenPopServer listening for inbound SIP calls");

        loop {
            let incoming = match self.coordinator.next_incoming_call(&mut events).await {
                Ok(Some(call)) => call,
                Ok(None) => {
                    info!("SIP event stream ended; stopping server");
                    return Ok(());
                }
                Err(_error) => {
                    warn!(error_present = true, "error waiting for incoming SIP call");
                    continue;
                }
            };

            let me = Arc::clone(&self);
            // Handle each call on its own task so a slow Connect handshake
            // doesn't block the next inbound INVITE.
            tokio::spawn(async move {
                if let Err(e) = me.handle_call(incoming).await {
                    warn!(error = %e, "failed to bridge inbound call to Amazon Connect");
                }
            });
        }
    }

    /// Route the call, then translate → answer → originate → bridge. A
    /// configured router can divert the call to a per-tenant Connect target
    /// or reject it outright (e.g. `404` for an unknown tenant).
    async fn handle_call(self: &Arc<Self>, call: IncomingCall) -> Result<()> {
        let session_id = call.call_id.clone();
        let correlation_id = correlation_id_from_headers(&extract_headers(&call));
        let Some(setup) = self.registry.register(session_id.clone(), correlation_id) else {
            call.reject(482, "Call Already Exists");
            return Ok(());
        };
        self.emit_lifecycle(&setup, ScreenPopLifecycleStage::SipInviteReceived);

        // 0. Per-call routing decision (multi-tenant hook).
        let route = match &self.router {
            Some(router) => match router(&call) {
                RouteDecision::Route(route) => Some(route),
                RouteDecision::Reject { status, reason } => {
                    info!(
                        status,
                        reason_present = !reason.is_empty(),
                        "router rejected inbound SIP call"
                    );
                    call.reject(status, &reason);
                    self.emit_lifecycle(&setup, ScreenPopLifecycleStage::TeardownStarted);
                    setup.mark_cleanup_attempt_complete();
                    self.emit_terminal_once(&setup, ScreenPopLifecycleStage::Terminated);
                    self.registry.finish(&setup, None);
                    return Ok(());
                }
            },
            None => None,
        };

        let stats = route.as_ref().map(|r| self.stats_for(&r.label));
        let result = self.bridge_call(call, route, Arc::clone(&setup)).await;
        let cancelled = matches!(result, Err(ConnectError::Cancelled));
        if result.is_err() {
            if !cancelled {
                if let Some(stats) = stats {
                    stats.failures.fetch_add(1, Ordering::Relaxed);
                }
            }
            if let ContactClaim::Setup {
                attempt,
                connect_conn,
            } = self.registry.claim_sip(&session_id)
            {
                self.cleanup_setup(attempt, connect_conn, !cancelled).await;
            }
        }
        if cancelled {
            Ok(())
        } else {
            result
        }
    }

    /// Translate headers → attributes, answer SIP, originate Connect, bridge.
    async fn bridge_call(
        self: &Arc<Self>,
        call: IncomingCall,
        route: Option<ContactRoute>,
        setup: Arc<SetupAttempt>,
    ) -> Result<()> {
        let session_id = call.call_id.clone();
        let display_name = Some(call.from.clone());
        let route_label = route.as_ref().map(|r| r.label.clone());

        // 1. Extract custom headers and translate to Connect attributes.
        let headers = extract_headers(&call);
        // Diagnostics deliberately expose only cardinality. Header names and
        // values may carry authentication or customer context.
        tracing::debug!(
            target: "rvoip_amazon_connect::sip_headers",
            count = headers.len(),
            "inbound INVITE headers"
        );
        let mapping = route
            .as_ref()
            .and_then(|r| r.attribute_mapping.as_ref())
            .unwrap_or(&self.mapping);
        let mapped = mapping.translate(headers);
        self.emit_lifecycle(&setup, ScreenPopLifecycleStage::AttributesMapped);
        tracing::debug!(
            target: "rvoip_amazon_connect::sip_headers",
            attributes = mapped.attributes.len(),
            skipped = mapped.skipped.len(),
            dropped_for_size = mapped.dropped_for_size,
            "mapped Connect contact attributes"
        );
        info!(
            route_present = route_label.is_some(),
            attributes = mapped.attributes.len(),
            "inbound SIP call → Amazon Connect screen pop"
        );

        // 2. Answer the SIP leg.
        let handle = tokio::select! {
            biased;
            _ = setup.cancelled() => return Err(ConnectError::Cancelled),
            accepted = call.accept() => accepted
                .map_err(|e| ConnectError::Signaling(format!("SIP accept: {e}")))?,
        };
        let sip_session: SipSessionId = handle.id().clone();

        // 3. Build the SIP media stream (inbound G.711).
        let sip_stream = tokio::select! {
            biased;
            _ = setup.cancelled() => return Err(ConnectError::Cancelled),
            stream = rvoip_sip::media_stream::SipMediaStream::new(
                Arc::clone(&self.coordinator),
                sip_session.clone(),
                rvoip_core::connection::Direction::Inbound,
            ) => stream
                .map_err(|e| ConnectError::Signaling(format!("SIP media stream: {e}")))?
                    as Arc<dyn MediaStream>,
        };

        // 4. Place the inbound WebRTC contact into Amazon Connect, honouring
        //    the route's per-call instance/flow override.
        let target = route
            .as_ref()
            .map(|r| ContactTarget {
                instance_id: r.instance_id.clone(),
                contact_flow_id: r.contact_flow_id.clone(),
                default_display_name: r.default_display_name.clone(),
            })
            .unwrap_or_default();
        let weak_server = Arc::downgrade(self);
        let setup_for_observer = Arc::clone(&setup);
        let observer: ContactSetupObserver = Arc::new(move |stage| {
            if stage == ContactSetupStage::ContactStarted {
                if let Some(server) = weak_server.upgrade() {
                    server.emit_lifecycle(
                        &setup_for_observer,
                        ScreenPopLifecycleStage::ContactStarted,
                    );
                }
            }
        });
        let adapter = Arc::clone(&self.adapter);
        let cleanup: Arc<dyn EstablishedConnectionCleanup> = adapter.clone();
        let task = tokio::spawn(async move {
            adapter
                .originate_contact_to_observed(
                    target,
                    mapped.attributes,
                    display_name,
                    None,
                    Some(observer),
                )
                .await
        });
        let mut establishment = EstablishmentOwner::new(task, cleanup);
        let connect_conn = tokio::select! {
            biased;
            _ = setup.cancelled() => {
                match establishment.wait().await {
                    Ok(connection_id) => {
                        self.adapter
                            .end(connection_id, EndReason::Normal)
                            .await
                            .map_err(|error| ConnectError::Control(error.to_string()))?;
                        return Err(ConnectError::Cancelled);
                    }
                    Err(error) => return Err(error),
                }
            },
            connected = establishment.wait() => connected?,
        };
        if !self.registry.bind_connect(&setup, connect_conn.clone()) {
            let _ = self.adapter.end(connect_conn, EndReason::Normal).await;
            return Err(ConnectError::Cancelled);
        }
        if let Some(label) = &route_label {
            self.stats_for(label)
                .contacts_started
                .fetch_add(1, Ordering::Relaxed);
        }

        let connect_streams = self
            .adapter
            .streams_for(&connect_conn)
            .ok_or(ConnectError::UnknownConnection(connect_conn.to_string()))?;
        let connect_stream = connect_streams
            .into_iter()
            .next()
            .ok_or_else(|| ConnectError::WebRtc("Connect contact has no media stream".into()))?;

        // 5. Bridge the two legs (transcoding G.711 ⟷ Opus).
        let bridge = bridge_streams(sip_stream, connect_stream)?;
        let sip_graph = bridge.a_graph();
        let connect_graph = bridge.b_graph();
        let active = ActiveContact {
            _bridge: bridge,
            sip_graph,
            connect_graph,
            connect_conn: connect_conn.clone(),
            route_label: route_label.clone(),
            setup: Arc::clone(&setup),
        };
        if let Err(active) = self.registry.promote(&setup, active) {
            drop(active);
            let _ = self.adapter.end(connect_conn, EndReason::Normal).await;
            return Err(ConnectError::Cancelled);
        }
        self.emit_lifecycle(&setup, ScreenPopLifecycleStage::MediaConnected);
        if let Some(label) = &route_label {
            self.stats_for(label)
                .active_sessions
                .fetch_add(1, Ordering::Relaxed);
        }
        info!(
            session = %session_id,
            route = route_label.as_deref().unwrap_or("-"),
            "bridged SIP ⟷ Amazon Connect"
        );

        Ok(())
    }

    /// Subscribe a dedicated event stream and end the Connect leg whenever the
    /// matching SIP leg terminates (`CallEnded`/`CallFailed`/`CallCancelled`).
    /// Uses its own broadcast subscription so it never competes with the
    /// incoming-call loop.
    async fn spawn_teardown_watcher(self: &Arc<Self>) -> Result<()> {
        let mut events = self
            .coordinator
            .events()
            .await
            .map_err(|e| ConnectError::Signaling(format!("SIP teardown events: {e}")))?;
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = events.next().await {
                let (call_id, failed) = match event {
                    SipEvent::CallEnded { call_id, .. } | SipEvent::CallCancelled { call_id } => {
                        (call_id, false)
                    }
                    SipEvent::CallFailed { call_id, .. } => (call_id, true),
                    _ => continue,
                };
                me.on_sip_ended_with_status(&call_id, failed).await;
            }
        });
        Ok(())
    }

    /// Subscribe the Connect adapter's event stream and BYE the SIP carrier when
    /// the Connect/agent leg ends (`Ended`/`Failed`) — the reverse direction.
    fn spawn_connect_end_watcher(self: &Arc<Self>) {
        let mut events = self.adapter.subscribe_events();
        let me = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                let (connect_conn, failed) = match event {
                    AdapterEvent::Ended { connection_id, .. } => (connection_id, false),
                    AdapterEvent::Failed { connection_id, .. } => (connection_id, true),
                    _ => continue,
                };
                me.on_connect_ended(&connect_conn, failed).await;
            }
        });
    }

    /// SIP leg ended → LEAVE the Chime meeting. Claims teardown by removing the
    /// `active` entry (so the reverse watcher no-ops).
    async fn on_sip_ended(&self, sip_session: &SipSessionId) {
        self.on_sip_ended_with_status(sip_session, false).await;
    }

    async fn on_sip_ended_with_status(&self, sip_session: &SipSessionId, failed: bool) {
        match self.registry.claim_sip(sip_session) {
            ContactClaim::Setup {
                attempt,
                connect_conn,
            } => self.cleanup_setup(attempt, connect_conn, failed).await,
            ContactClaim::Active(active) => {
                info!(session = %sip_session, "SIP leg ended — leaving Amazon Connect meeting");
                self.cleanup_active(active, failed).await;
            }
            ContactClaim::EarlySipEnd | ContactClaim::EarlyConnectEnd | ContactClaim::None => {}
        }
    }

    /// Connect/agent leg ended → BYE the SIP carrier. Resolves the SIP session
    /// from the reverse index, then claims teardown via the same `active`
    /// removal so the two directions can't double-fire.
    async fn on_connect_ended(&self, connect_conn: &ConnectionId, failed: bool) {
        match self.registry.claim_connect(connect_conn) {
            ContactClaim::Setup {
                attempt,
                connect_conn,
            } => self.cleanup_setup(attempt, connect_conn, failed).await,
            ContactClaim::Active(active) => {
                info!(session = %active.setup.session_id, "Amazon Connect leg ended — hanging up SIP carrier (BYE)");
                self.cleanup_active(active, failed).await;
            }
            ContactClaim::EarlySipEnd | ContactClaim::EarlyConnectEnd | ContactClaim::None => {}
        }
    }

    async fn cleanup_setup(
        &self,
        attempt: Arc<SetupAttempt>,
        connect_conn: Option<ConnectionId>,
        failed: bool,
    ) {
        self.emit_lifecycle(&attempt, ScreenPopLifecycleStage::TeardownStarted);
        // Both operations are intentionally idempotent. If cancellation raced
        // before the adapter returned a connection id, its started-contact
        // guard owns StopContact while this still releases the SIP resource.
        let cleanup_result = release_resources(
            self.cleanup.as_ref(),
            &attempt.session_id,
            connect_conn.as_ref(),
        )
        .await;
        if let Err(errors) = &cleanup_result {
            warn!(error_count = errors.len(), session = %attempt.session_id, "screen-pop setup cleanup incomplete");
        }
        self.registry.finish(&attempt, connect_conn.as_ref());
        attempt.mark_cleanup_attempt_complete();
        self.emit_terminal_once(
            &attempt,
            if failed || cleanup_result.is_err() {
                ScreenPopLifecycleStage::Failed
            } else {
                ScreenPopLifecycleStage::Terminated
            },
        );
    }

    async fn cleanup_active(&self, active: ActiveContact, failed: bool) {
        self.emit_lifecycle(&active.setup, ScreenPopLifecycleStage::TeardownStarted);
        self.release_route_slot(&active);
        let cleanup_result = release_resources(
            self.cleanup.as_ref(),
            &active.setup.session_id,
            Some(&active.connect_conn),
        )
        .await;
        if let Err(errors) = &cleanup_result {
            warn!(error_count = errors.len(), session = %active.setup.session_id, "active screen-pop cleanup incomplete");
        }
        self.registry
            .finish(&active.setup, Some(&active.connect_conn));
        active.setup.mark_cleanup_attempt_complete();
        self.emit_terminal_once(
            &active.setup,
            if failed || cleanup_result.is_err() {
                ScreenPopLifecycleStage::Failed
            } else {
                ScreenPopLifecycleStage::Terminated
            },
        );
        // Dropping active stops both graph directions.
    }

    /// Tear down a bridged contact by SIP session (public manual teardown).
    pub async fn end(&self, sip_session: &SipSessionId) {
        self.on_sip_ended(sip_session).await;
    }

    /// Tear down a bridged contact by the stable string form returned from
    /// [`Self::active_call_ids`]. Returns `false` when the call is no longer
    /// active, which lets HTTP control planes make hangup idempotent.
    pub async fn end_by_call_id(&self, call_id: &str) -> bool {
        let session = self
            .registry
            .setups
            .iter()
            .find(|entry| entry.key().to_string() == call_id)
            .map(|entry| entry.key().clone())
            .or_else(|| {
                self.registry
                    .active
                    .iter()
                    .find(|entry| entry.key().to_string() == call_id)
                    .map(|entry| entry.key().clone())
            });
        let Some(session) = session else {
            return false;
        };
        self.on_sip_ended(&session).await;
        true
    }

    /// Decrement the per-route active-session gauge for a torn-down contact.
    fn release_route_slot(&self, active: &ActiveContact) {
        if let Some(label) = &active.route_label {
            self.stats_for(label)
                .active_sessions
                .fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// User part of the INVITE Request-URI — the primary multi-tenant routing
/// key (CONTRACTS B.4: R-URI user, then To user, then default tenant).
pub fn request_uri_user(call: &IncomingCall) -> Option<String> {
    call.raw_request().and_then(|r| r.uri().user.clone())
}

/// User part of the To-header URI — the fallback routing key. Reads the
/// typed To header when the parsed request is available, else parses the
/// legacy `call.to` string.
pub fn to_uri_user(call: &IncomingCall) -> Option<String> {
    if let Some(to) = call.raw_request().and_then(|r| r.to()) {
        return to.0.uri.user.clone();
    }
    uri_user_part(&call.to).map(str::to_string)
}

/// Extract the user part from a SIP URI string, tolerating a display name,
/// angle brackets, and URI/header params: `"Bob" <sip:sales@x.y;tag=1>` →
/// `sales`. Returns `None` when there is no user part.
pub fn uri_user_part(uri: &str) -> Option<&str> {
    let s = uri.trim();
    // Strip a display name + angle brackets if present.
    let s = match (s.find('<'), s.find('>')) {
        (Some(open), Some(close)) if open < close => &s[open + 1..close],
        _ => s,
    };
    let s = s
        .strip_prefix("sips:")
        .or_else(|| s.strip_prefix("sip:"))
        .unwrap_or(s);
    let user = &s[..s.find('@')?];
    // Drop password / params that legally precede '@' only via ';'/':'.
    let user = user.split(|c| c == ':' || c == ';').next().unwrap_or(user);
    (!user.is_empty()).then_some(user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Barrier;

    #[test]
    fn parses_plain_and_bracketed_uris() {
        assert_eq!(uri_user_part("sip:banking@10.0.0.1"), Some("banking"));
        assert_eq!(uri_user_part("sips:sales@example.com:5061"), Some("sales"));
        assert_eq!(
            uri_user_part("\"Vapi\" <sip:support@example.com;transport=udp>;tag=abc"),
            Some("support")
        );
        assert_eq!(uri_user_part("<sip:a@b>"), Some("a"));
    }

    #[test]
    fn strips_password_and_uri_params_from_user() {
        assert_eq!(uri_user_part("sip:bob:secret@example.com"), Some("bob"));
        assert_eq!(uri_user_part("sip:bob;p=1@example.com"), Some("bob"));
    }

    #[test]
    fn no_user_part_yields_none() {
        assert_eq!(uri_user_part("sip:10.0.0.1"), None);
        assert_eq!(uri_user_part("sip:@example.com"), None);
        assert_eq!(uri_user_part(""), None);
        assert_eq!(uri_user_part("tel:+14155550100"), None);
    }

    #[test]
    fn correlation_id_preserves_safe_phone_fingerprint() {
        assert_eq!(
            sanitize_correlation_id("  +14155550199  "),
            Some("+14155550199".into())
        );
        assert_eq!(
            sanitize_correlation_id("corr@example.com"),
            Some("corr@example.com".into())
        );
        assert_eq!(sanitize_correlation_id("unsafe\r\nvalue"), None);
        assert_eq!(sanitize_correlation_id(&"x".repeat(129)), None);
    }

    #[tokio::test]
    async fn connect_end_between_reverse_bind_and_active_promotion_cancels_setup() {
        let registry = Arc::new(ContactRegistry::<()>::default());
        let session = SipSessionId::from_string("race-session");
        let attempt = registry
            .register(session.clone(), Some("corr-race".into()))
            .expect("register setup");
        let conn = ConnectionId::new();
        let reverse_bound = Arc::new(Barrier::new(2));
        let allow_promotion = Arc::new(Barrier::new(2));

        let promoter = tokio::spawn({
            let registry = Arc::clone(&registry);
            let attempt = Arc::clone(&attempt);
            let conn = conn.clone();
            let reverse_bound = Arc::clone(&reverse_bound);
            let allow_promotion = Arc::clone(&allow_promotion);
            async move {
                assert!(registry.bind_connect(&attempt, conn));
                reverse_bound.wait().await;
                allow_promotion.wait().await;
                registry.promote(&attempt, ())
            }
        });

        reverse_bound.wait().await;
        match registry.claim_connect(&conn) {
            ContactClaim::Setup {
                attempt: claimed,
                connect_conn,
            } => {
                assert!(Arc::ptr_eq(&attempt, &claimed));
                assert_eq!(connect_conn.as_ref(), Some(&conn));
            }
            _ => panic!("Connect end must claim the in-progress setup"),
        }
        allow_promotion.wait().await;
        assert!(promoter.await.expect("promoter task").is_err());
        assert!(*attempt.cancel_tx.borrow());
        assert!(matches!(registry.claim_sip(&session), ContactClaim::None));
        registry.finish(&attempt, Some(&conn));
        assert!(matches!(registry.claim_connect(&conn), ContactClaim::None));
        assert!(registry.live_is_empty());
    }

    #[tokio::test]
    async fn connect_end_before_reverse_bind_is_replayed_at_bind() {
        let registry = ContactRegistry::<()>::default();
        let session = SipSessionId::from_string("early-end-session");
        let attempt = registry
            .register(session, Some("corr-early".into()))
            .expect("register setup");
        let conn = ConnectionId::new();

        assert!(matches!(
            registry.claim_connect(&conn),
            ContactClaim::EarlyConnectEnd
        ));
        assert!(!registry.bind_connect(&attempt, conn.clone()));
        assert!(*attempt.cancel_tx.borrow());
        assert!(registry.promote(&attempt, ()).is_err());
        assert!(matches!(
            registry.claim_sip(&attempt.session_id),
            ContactClaim::Setup { .. }
        ));
        registry.finish(&attempt, Some(&conn));
        assert!(matches!(registry.claim_connect(&conn), ContactClaim::None));
        assert!(registry.live_is_empty());
    }

    #[test]
    fn sip_cancel_arriving_before_setup_registration_is_replayed() {
        let registry = ContactRegistry::<()>::default();
        let session = SipSessionId::from_string("pre-register-cancel");
        assert!(matches!(
            registry.claim_sip(&session),
            ContactClaim::EarlySipEnd
        ));

        let attempt = registry
            .register(session, Some("corr-cancel".into()))
            .expect("register setup");
        assert!(*attempt.cancel_tx.borrow());
        assert!(registry.promote(&attempt, ()).is_err());
        assert!(matches!(
            registry.claim_sip(&attempt.session_id),
            ContactClaim::Setup { .. }
        ));
        registry.finish(&attempt, None);
        assert!(registry.live_is_empty());
    }

    #[test]
    fn duplicate_session_is_rejected_after_promotion_without_replacement() {
        let registry = ContactRegistry::<()>::default();
        let session = SipSessionId::from_string("duplicate-active");
        let attempt = registry
            .register(session.clone(), Some("corr-active".into()))
            .expect("register setup");
        let conn = ConnectionId::new();
        assert!(registry.bind_connect(&attempt, conn.clone()));
        assert!(registry.promote(&attempt, ()).is_ok());

        assert!(registry.register(session.clone(), None).is_none());
        assert!(matches!(
            registry.claim_sip(&session),
            ContactClaim::Active(())
        ));
        registry.finish(&attempt, Some(&conn));
        assert!(registry.register(session, None).is_none());
        assert!(matches!(
            registry.claim_sip(&attempt.session_id),
            ContactClaim::None
        ));
        assert!(matches!(registry.claim_connect(&conn), ContactClaim::None));
        assert!(registry.live_is_empty());
    }

    #[test]
    fn unmatched_event_tombstones_are_bounded() {
        let registry = ContactRegistry::<()>::default();
        for index in 0..(EVENT_TOMBSTONE_CAPACITY + 32) {
            let session = SipSessionId::from_string(format!("unmatched-{index}"));
            assert!(matches!(
                registry.claim_sip(&session),
                ContactClaim::EarlySipEnd
            ));
        }
        assert!(registry.early_sip_ends.len() <= EVENT_TOMBSTONE_CAPACITY);
    }

    #[derive(Default)]
    struct MockEstablishedCleanup {
        ended: tokio::sync::Mutex<Vec<ConnectionId>>,
    }

    #[async_trait]
    impl EstablishedConnectionCleanup for MockEstablishedCleanup {
        async fn end_established(&self, connection_id: ConnectionId) {
            self.ended.lock().await.push(connection_id);
        }
    }

    #[tokio::test]
    async fn caller_cancel_after_route_owned_barrier_consumes_and_ends_late_success() {
        let route_owned = Arc::new(Barrier::new(2));
        let allow_return = Arc::new(Barrier::new(2));
        let conn = ConnectionId::new();
        let task = tokio::spawn({
            let route_owned = Arc::clone(&route_owned);
            let allow_return = Arc::clone(&allow_return);
            let conn = conn.clone();
            async move {
                // Exact test barrier corresponding to ContactSetupStage::RouteOwned:
                // the adapter route exists and its cleanup guard is disarmed.
                route_owned.wait().await;
                allow_return.wait().await;
                Ok(conn)
            }
        });
        let cleanup = Arc::new(MockEstablishedCleanup::default());
        let cleanup_trait: Arc<dyn EstablishedConnectionCleanup> = cleanup.clone();
        let owner = EstablishmentOwner::new(task, cleanup_trait);

        route_owned.wait().await;
        drop(owner);
        allow_return.wait().await;

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if cleanup.ended.lock().await.as_slice() == [conn.clone()] {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("late successful connection was ended");
    }

    #[tokio::test]
    async fn lifecycle_rejects_contact_started_after_teardown_wins_race() {
        let attempt = SetupAttempt::new(
            SipSessionId::from_string("lifecycle-race"),
            Some("corr-life".into()),
        );
        assert!(attempt.transition(ScreenPopLifecycleStage::SipInviteReceived));
        let teardown_entered = Arc::new(Barrier::new(2));
        let allow_late_start = Arc::new(Barrier::new(2));
        let late_start = tokio::spawn({
            let attempt = Arc::clone(&attempt);
            let teardown_entered = Arc::clone(&teardown_entered);
            let allow_late_start = Arc::clone(&allow_late_start);
            async move {
                teardown_entered.wait().await;
                allow_late_start.wait().await;
                attempt.transition(ScreenPopLifecycleStage::ContactStarted)
            }
        });

        assert!(attempt.transition(ScreenPopLifecycleStage::TeardownStarted));
        teardown_entered.wait().await;
        allow_late_start.wait().await;
        assert!(!late_start.await.expect("late start task"));
        assert!(!attempt.transition(ScreenPopLifecycleStage::Terminated));
        attempt.mark_cleanup_attempt_complete();
        assert!(attempt.transition(ScreenPopLifecycleStage::Terminated));
    }

    #[derive(Default)]
    struct MockCleanupActions {
        sip: SyncMutex<Vec<SipSessionId>>,
        connect: SyncMutex<Vec<ConnectionId>>,
        sip_failure: Option<String>,
        connect_failure: Option<String>,
    }

    #[async_trait]
    impl ScreenPopCleanupActions for MockCleanupActions {
        async fn hangup_sip(&self, session_id: &SipSessionId) -> std::result::Result<(), String> {
            self.sip.lock().push(session_id.clone());
            self.sip_failure.clone().map_or(Ok(()), Err)
        }

        async fn stop_connect(
            &self,
            connection_id: &ConnectionId,
        ) -> std::result::Result<(), String> {
            self.connect.lock().push(connection_id.clone());
            self.connect_failure.clone().map_or(Ok(()), Err)
        }
    }

    #[tokio::test]
    async fn setup_cleanup_releases_sip_and_connect_resources() {
        let cleanup = MockCleanupActions::default();
        let session = SipSessionId::from_string("cleanup-session");
        let conn = ConnectionId::new();

        release_resources(&cleanup, &session, Some(&conn))
            .await
            .expect("both resources released");

        assert_eq!(cleanup.sip.lock().as_slice(), &[session]);
        assert_eq!(cleanup.connect.lock().as_slice(), &[conn]);
    }

    struct PausingCleanupActions {
        cleanup_started: Arc<Barrier>,
        allow_cleanup: Arc<Barrier>,
    }

    #[async_trait]
    impl ScreenPopCleanupActions for PausingCleanupActions {
        async fn hangup_sip(&self, _session_id: &SipSessionId) -> std::result::Result<(), String> {
            self.cleanup_started.wait().await;
            self.allow_cleanup.wait().await;
            Ok(())
        }

        async fn stop_connect(
            &self,
            _connection_id: &ConnectionId,
        ) -> std::result::Result<(), String> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn duplicate_registration_is_rejected_while_active_cleanup_is_awaiting() {
        let registry = Arc::new(ContactRegistry::<()>::default());
        let session = SipSessionId::from_string("cleanup-barrier");
        let attempt = registry
            .register(session.clone(), Some("corr-cleanup".into()))
            .expect("register setup");
        let conn = ConnectionId::new();
        assert!(registry.bind_connect(&attempt, conn.clone()));
        assert!(registry.promote(&attempt, ()).is_ok());
        assert!(matches!(
            registry.claim_sip(&session),
            ContactClaim::Active(())
        ));

        let cleanup_started = Arc::new(Barrier::new(2));
        let allow_cleanup = Arc::new(Barrier::new(2));
        let cleanup = Arc::new(PausingCleanupActions {
            cleanup_started: Arc::clone(&cleanup_started),
            allow_cleanup: Arc::clone(&allow_cleanup),
        });
        let cleanup_task = tokio::spawn({
            let cleanup = Arc::clone(&cleanup);
            let session = session.clone();
            let conn = conn.clone();
            async move { release_resources(cleanup.as_ref(), &session, Some(&conn)).await }
        });

        cleanup_started.wait().await;
        assert!(
            registry.register(session.clone(), None).is_none(),
            "tearing-down session must remain reserved during awaited cleanup"
        );
        allow_cleanup.wait().await;
        cleanup_task
            .await
            .expect("cleanup task")
            .expect("cleanup succeeds");
        registry.finish(&attempt, Some(&conn));
        assert!(registry.register(session, None).is_none());
        assert!(registry.live_is_empty());
    }

    #[tokio::test]
    async fn incomplete_cleanup_attempts_both_resources_and_requires_failed_terminal() {
        let cleanup = MockCleanupActions {
            connect_failure: Some("ownership retained".into()),
            ..Default::default()
        };
        let session = SipSessionId::from_string("cleanup-failure");
        let conn = ConnectionId::new();
        let attempt = SetupAttempt::new(session.clone(), Some("corr-failure".into()));
        assert!(attempt.transition(ScreenPopLifecycleStage::TeardownStarted));

        let result = release_resources(&cleanup, &session, Some(&conn)).await;
        assert!(result.is_err());
        assert_eq!(cleanup.sip.lock().as_slice(), &[session]);
        assert_eq!(cleanup.connect.lock().as_slice(), &[conn]);
        attempt.mark_cleanup_attempt_complete();
        assert!(attempt.transition(ScreenPopLifecycleStage::Failed));
        assert!(!attempt.transition(ScreenPopLifecycleStage::Terminated));
    }

    #[test]
    fn lifecycle_stage_serializes_to_stable_snake_case() {
        assert_eq!(
            serde_json::to_string(&ScreenPopLifecycleStage::SipInviteReceived).unwrap(),
            "\"sip_invite_received\""
        );
        assert_eq!(
            serde_json::to_string(&ScreenPopLifecycleStage::MediaConnected).unwrap(),
            "\"media_connected\""
        );
    }

    #[test]
    fn route_and_lifecycle_diagnostics_are_value_free() {
        let route = ContactRoute {
            label: "tenant-secret".into(),
            instance_id: Some("instance-secret".into()),
            contact_flow_id: Some("flow-secret".into()),
            attribute_mapping: Some(
                AttributeMapping::default().rename("header-secret", "attribute-secret"),
            ),
            default_display_name: Some("display-secret".into()),
        };
        let event = ScreenPopLifecycleEvent {
            stage: ScreenPopLifecycleStage::ContactStarted,
            correlation_id: Some("correlation-secret".into()),
            occurred_at: chrono::Utc::now(),
        };
        let diagnostic = format!("{route:?} {event:?}");
        for secret in [
            "tenant-secret",
            "instance-secret",
            "flow-secret",
            "header-secret",
            "attribute-secret",
            "display-secret",
            "correlation-secret",
        ] {
            assert!(!diagnostic.contains(secret), "leaked {secret}");
        }
    }
}

/// Pull every custom (`Other`) header off the inbound INVITE as
/// `(name, value)` pairs, preserving original-case names and clean values
/// (`raw_header_value`, not the `"Name: value"` `Display`). Falls back to the
/// legacy `headers` map (lowercased keys, `"Name: value"` values stripped) when
/// the parsed request is unavailable.
fn extract_headers(call: &IncomingCall) -> Vec<(String, String)> {
    if let Some(req) = call.raw_request() {
        let mut out = Vec::new();
        for hdr in &req.headers {
            if let TypedHeader::Other(name @ HeaderName::Other(key), _) = hdr {
                if let Some(value) = req.raw_header_value(name) {
                    out.push((key.clone(), value));
                }
            }
        }
        return out;
    }
    // Legacy fallback: values are "Name: value"; strip the prefix.
    call.headers
        .iter()
        .map(|(k, v)| {
            let value = v.splitn(2, ": ").nth(1).unwrap_or(v).to_string();
            (k.clone(), value)
        })
        .collect()
}

fn correlation_id_from_headers(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("X-Correlation-Id"))
        .and_then(|(_, value)| sanitize_correlation_id(value))
}

fn sanitize_correlation_id(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() || value.len() > 128 {
        return None;
    }
    value
        .bytes()
        .all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'+' | b'@')
        })
        .then(|| value.to_string())
}
