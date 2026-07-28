//! [`AmazonConnectAdapter`] — `ConnectionAdapter` implementation that delivers
//! a call to an Amazon Connect agent over the Chime SDK WebRTC media plane.
//!
//! The natural entry point is [`AmazonConnectAdapter::originate_contact`], which
//! runs the full control + signaling + media establishment and returns a
//! connected [`ConnectionId`] ready to be bridged to the inbound leg via
//! `Orchestrator::bridge_connections`. The generic [`ConnectionAdapter::originate`]
//! now admits only an exact typed context and remains I/O-dormant until the
//! staged lifecycle lands; the legacy wrapper remains behavior-compatible.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::{Mutex as SyncMutex, RwLock as SyncRwLock};
use tokio::sync::{
    mpsc, oneshot, watch, Mutex as AsyncMutex, Notify, OwnedSemaphorePermit, Semaphore,
};
use tokio::task::JoinHandle;
use tracing::{info, instrument, warn};
use zeroize::Zeroize;

use chrono::Utc;
use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, AdapterLifecycleCapabilities, AdapterLifecycleSink,
    AdapterLifecycleSinkSlot, ConnectionAdapter, ConnectionHandle, EndReason,
    ExternalConnectionReference, OriginateRequest, OutboundActivation, RejectReason,
    SignatureHeaders, TerminalDelivery, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;

use rvoip_webrtc::WebRtcConfig;

use crate::config::ConnectConfig;
use crate::control::{ConnectContactStarter, StartContactRequest, StopContactRequest};
use crate::errors::ConnectError;
use crate::media::{
    ChimeWebRtcMediaConnector, ConnectMediaCloseOutcome, ConnectMediaConnectOptions,
    ConnectMediaConnector, ConnectMediaSession, ConnectMediaTerminalCause,
};
use crate::originate::{AmazonConnectOriginateContext, ConnectProfileId};

/// Event channel depth (mirrors rvoip-webrtc's `ADAPTER_EVENT_CAP`).
pub const ADAPTER_EVENT_CAP: usize = 256;

/// Exact durable cleanup authority for one successfully created Amazon
/// Connect contact.
///
/// The values are intentionally omitted from `Debug`; control planes may
/// persist them, but must never place them in logs, traces, or metric labels.
#[derive(Clone, Eq, PartialEq)]
pub struct RetainedAmazonConnectCleanup {
    profile_id: ConnectProfileId,
    instance_id: String,
    contact_id: String,
}

impl RetainedAmazonConnectCleanup {
    pub fn new(
        profile_id: ConnectProfileId,
        instance_id: impl Into<String>,
        contact_id: impl Into<String>,
    ) -> crate::errors::Result<Self> {
        let instance_id = instance_id.into();
        let contact_id = contact_id.into();
        crate::originate::validate_connect_instance_id(&instance_id).map_err(|_| {
            ConnectError::Control("retained Amazon cleanup instance is invalid".into())
        })?;
        ExternalConnectionReference::new(AMAZON_CONNECT_CONTACT_REFERENCE_KIND, contact_id.clone())
            .map_err(|_| {
                ConnectError::Control("retained Amazon cleanup contact is invalid".into())
            })?;
        Ok(Self {
            profile_id,
            instance_id,
            contact_id,
        })
    }

    fn from_request(profile_id: ConnectProfileId, request: &StopContactRequest) -> Option<Self> {
        Self::new(
            profile_id,
            request.instance_id.clone(),
            request.contact_id.clone(),
        )
        .ok()
    }

    pub fn profile_id(&self) -> &ConnectProfileId {
        &self.profile_id
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn contact_id(&self) -> &str {
        &self.contact_id
    }
}

impl fmt::Debug for RetainedAmazonConnectCleanup {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RetainedAmazonConnectCleanup")
            .field("profile_id", &"[redacted]")
            .field("instance_id", &"[redacted]")
            .field("contact_id", &"[redacted]")
            .finish()
    }
}

/// Awaited durability hook for cleanup ownership.
///
/// `retained` is awaited immediately after a successful
/// StartWebRTCContact response and before media setup; `resolved` is awaited
/// only after StopContact succeeds or reports the contact already ended. A
/// persistence failure therefore fails closed instead of creating an
/// unjournaled remote contact.
#[async_trait]
pub trait AmazonConnectCleanupObserver: Send + Sync {
    async fn retained(&self, cleanup: RetainedAmazonConnectCleanup) -> crate::errors::Result<()>;
    async fn resolved(&self, cleanup: RetainedAmazonConnectCleanup) -> crate::errors::Result<()>;
}

type CleanupObserverSlot = Arc<SyncRwLock<Option<Arc<dyn AmazonConnectCleanupObserver>>>>;

async fn notify_cleanup_retained(
    observer: &CleanupObserverSlot,
    cleanup: RetainedAmazonConnectCleanup,
) -> crate::errors::Result<()> {
    let observer = observer.read().clone();
    if let Some(observer) = observer {
        observer.retained(cleanup).await?;
    }
    Ok(())
}

async fn notify_cleanup_resolved(
    observer: &CleanupObserverSlot,
    cleanup: RetainedAmazonConnectCleanup,
) -> crate::errors::Result<()> {
    let observer = observer.read().clone();
    if let Some(observer) = observer {
        observer.resolved(cleanup).await?;
    }
    Ok(())
}

/// Per-call override of the Amazon Connect contact target.
///
/// Every `None` field falls back to the adapter's [`ConnectConfig`], so a
/// default-constructed target reproduces the classic single-target behaviour.
/// This is the multi-tenant hook: one adapter (one credential chain, one
/// region) can place contacts into different Connect instances/flows per call.
#[derive(Clone, Default)]
pub struct ContactTarget {
    /// Amazon Connect instance id override.
    pub instance_id: Option<String>,
    /// Contact-flow id override.
    pub contact_flow_id: Option<String>,
    /// Display-name fallback override, used when the caller supplies no
    /// per-call display name.
    pub default_display_name: Option<String>,
}

impl fmt::Debug for ContactTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContactTarget")
            .field("instance_id_present", &self.instance_id.is_some())
            .field("contact_flow_id_present", &self.contact_flow_id.is_some())
            .field(
                "default_display_name_present",
                &self.default_display_name.is_some(),
            )
            .finish()
    }
}

/// One active Amazon Connect contact: the Chime peer connection, its signaling
/// session, and the bridged media stream(s).
#[derive(Clone)]
struct Route {
    /// Injectable session owns Chime, WebRTC, streams, and media lifecycle.
    media: Arc<dyn ConnectMediaSession>,
    cancel: Arc<Notify>,
    /// Control-plane ownership retained until teardown.
    stop_request: StopContactRequest,
    /// Exact configured profile that owns the retained StopContact request.
    profile_id: ConnectProfileId,
    /// Exact account/region starter selected for both Start and Stop.
    starter: Arc<dyn ConnectContactStarter>,
    cleanup_permit: Arc<OwnedSemaphorePermit>,
    /// Present only for the generic prepare/bind/activate path. The legacy
    /// screen-pop wrapper never enters the staged event machinery.
    prepared: Option<Arc<AmazonOutboundRoute>>,
    /// One owner for terminal, DTMF, health, and cancellation observation.
    supervisor: Arc<AsyncMutex<Option<JoinHandle<()>>>>,
}

const MAX_OWNED_CONTACT_CLEANUPS: usize = 4_096;
const OUTBOUND_EVENT_STAGE_CAPACITY: usize = 64;

/// Exact external-reference namespace returned for an Amazon Connect contact.
///
/// Durable callers must compare this value byte-for-byte before treating an
/// adapter receipt as authority to stop a persisted contact.
pub const AMAZON_CONNECT_CONTACT_REFERENCE_KIND: &str = "amazon-connect.contact-id";

/// Opaque handle for a locally prepared Amazon Connect route.
///
/// It intentionally exposes no target, attribute, token, or AWS identifier.
pub struct AmazonConnectTransportHandle {
    connection_id: ConnectionId,
    prepared_routes: Weak<DashMap<ConnectionId, Arc<AmazonOutboundRoute>>>,
    active_routes: Weak<DashMap<ConnectionId, Route>>,
    cancel: Arc<Notify>,
}

impl AmazonConnectTransportHandle {
    /// The exact provisional connection owned by this handle.
    pub fn connection_id(&self) -> &ConnectionId {
        &self.connection_id
    }

    /// Whether either the dormant route or its activated media route remains live.
    pub fn route_exists(&self) -> bool {
        self.prepared_routes
            .upgrade()
            .and_then(|routes| routes.get(&self.connection_id).map(|route| route.is_live()))
            .unwrap_or(false)
            || self
                .active_routes
                .upgrade()
                .is_some_and(|routes| routes.contains_key(&self.connection_id))
    }

    /// Request bounded cancellation of activation or active media.
    pub fn cancel(&self) {
        self.cancel.notify_waiters();
    }
}

impl fmt::Debug for AmazonConnectTransportHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AmazonConnectTransportHandle")
            .field("connection_id", &self.connection_id)
            .field("route_exists", &self.route_exists())
            .finish()
    }
}

#[derive(Clone)]
enum AmazonActivationOutcome {
    Pending,
    Succeeded(OutboundActivation),
    Failed(&'static str),
}

impl fmt::Debug for AmazonActivationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => formatter.write_str("Pending"),
            Self::Succeeded(receipt) => formatter.debug_tuple("Succeeded").field(receipt).finish(),
            Self::Failed(class) => formatter.debug_tuple("Failed").field(class).finish(),
        }
    }
}

enum OutboundStageState {
    Dormant {
        events: VecDeque<AdapterEvent>,
        overflowed: bool,
        terminal_seen: bool,
    },
    Activated,
}

impl Default for OutboundStageState {
    fn default() -> Self {
        Self::Dormant {
            events: VecDeque::new(),
            overflowed: false,
            terminal_seen: false,
        }
    }
}

/// Immutable, locally prepared generic route. No AWS or peer-visible I/O is
/// performed until the activation owner starts exactly one background task.
struct AmazonOutboundRoute {
    context: Arc<AmazonConnectOriginateContext>,
    starter: Arc<dyn ConnectContactStarter>,
    live: AtomicBool,
    terminal_claimed: AtomicBool,
    activation_started: AtomicBool,
    activation: watch::Sender<AmazonActivationOutcome>,
    activation_task: AsyncMutex<Option<JoinHandle<()>>>,
    stage: SyncMutex<OutboundStageState>,
    cancel: Arc<Notify>,
}

struct AmazonSetupAdmission {
    in_flight: Arc<AtomicUsize>,
    idle: Arc<Notify>,
}

impl Drop for AmazonSetupAdmission {
    fn drop(&mut self) {
        if self.in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.idle.notify_waiters();
        }
    }
}

impl AmazonOutboundRoute {
    fn new(
        context: Arc<AmazonConnectOriginateContext>,
        starter: Arc<dyn ConnectContactStarter>,
    ) -> Arc<Self> {
        let (activation, _) = watch::channel(AmazonActivationOutcome::Pending);
        Arc::new(Self {
            context,
            starter,
            live: AtomicBool::new(true),
            terminal_claimed: AtomicBool::new(false),
            activation_started: AtomicBool::new(false),
            activation,
            activation_task: AsyncMutex::new(None),
            stage: SyncMutex::new(OutboundStageState::default()),
            cancel: Arc::new(Notify::new()),
        })
    }

    fn is_live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }

    fn mark_not_live(&self) {
        let _ = self.retire_if_live();
    }

    fn retire_if_live(&self) -> bool {
        let was_live = self.live.swap(false, Ordering::AcqRel);
        self.cancel.notify_waiters();
        was_live
    }

    fn claim_terminal(&self) -> bool {
        self.terminal_claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn stage_event(&self, event: AdapterEvent) -> Option<AdapterEvent> {
        let terminal = matches!(
            event,
            AdapterEvent::Ended { .. } | AdapterEvent::Failed { .. }
        );
        let mut stage = self.stage.lock();
        match &mut *stage {
            OutboundStageState::Activated => Some(event),
            OutboundStageState::Dormant {
                events,
                overflowed,
                terminal_seen,
            } => {
                if terminal && *terminal_seen {
                    return None;
                }
                if events.len() >= OUTBOUND_EVENT_STAGE_CAPACITY {
                    *overflowed = true;
                    if terminal {
                        // The first terminal outcome must survive ordinary FIFO
                        // saturation. Retire the newest nonterminal item.
                        let _ = events.pop_back();
                        events.push_back(event);
                        *terminal_seen = true;
                    }
                } else {
                    events.push_back(event);
                    if terminal {
                        *terminal_seen = true;
                    }
                }
                None
            }
        }
    }

    fn activate_stage(&self, events_tx: &mpsc::Sender<AdapterEvent>) -> RvoipResult<()> {
        let mut stage = self.stage.lock();
        match &mut *stage {
            OutboundStageState::Activated => Ok(()),
            OutboundStageState::Dormant {
                events, overflowed, ..
            } => {
                if *overflowed {
                    events.clear();
                    return Err(RvoipError::AdmissionRejected(
                        "Amazon Connect outbound lifecycle event stage overflowed",
                    ));
                }
                let mut permits = events_tx.try_reserve_many(events.len()).map_err(|_| {
                    RvoipError::AdmissionRejected(
                        "Amazon Connect outbound lifecycle event publication was unavailable",
                    )
                })?;
                for (permit, event) in permits.by_ref().zip(events.drain(..)) {
                    permit.send(event);
                }
                *stage = OutboundStageState::Activated;
                Ok(())
            }
        }
    }
}

#[derive(Clone)]
struct AmazonActivationEnvironment {
    config: ConnectConfig,
    webrtc: WebRtcConfig,
    media_connector: Arc<dyn ConnectMediaConnector>,
    routes: Arc<DashMap<ConnectionId, Route>>,
    prepared_routes: Arc<DashMap<ConnectionId, Arc<AmazonOutboundRoute>>>,
    events_tx: mpsc::Sender<AdapterEvent>,
    lifecycle: AdapterLifecycleSinkSlot,
    contacts_started: Arc<AtomicUsize>,
    failures: Arc<AtomicUsize>,
    cleanup_slots: Arc<Semaphore>,
    pending_cleanups: PendingCleanupMap,
    cleanup_observer: CleanupObserverSlot,
}

impl AmazonActivationEnvironment {
    fn publish_or_stage(
        &self,
        prepared: Option<&Arc<AmazonOutboundRoute>>,
        event: AdapterEvent,
    ) -> bool {
        let event = match prepared {
            Some(prepared) => prepared.stage_event(event),
            None => Some(event),
        };
        event.is_none_or(|event| self.events_tx.try_send(event).is_ok())
    }

    async fn publish_terminal(
        &self,
        prepared: Option<&Arc<AmazonOutboundRoute>>,
        event: AdapterEvent,
    ) {
        let event = if let Some(prepared) = prepared {
            if !prepared.claim_terminal() {
                return;
            }
            match prepared.stage_event(event.clone()) {
                Some(event) => event,
                None => {
                    if prepared.activate_stage(&self.events_tx).is_ok() {
                        return;
                    }
                    event
                }
            }
        } else {
            event
        };
        if self
            .lifecycle
            .queue_or_deliver_terminal(&self.events_tx, event)
            .await
            == TerminalDelivery::Undeliverable
        {
            warn!("Amazon Connect terminal event was undeliverable");
        }
    }

    async fn close_media(&self, route: &Route) {
        route.cancel.notify_waiters();
        let now = Instant::now();
        let deadline = now
            .checked_add(self.config.signaling_timeout)
            .unwrap_or(now);
        match tokio::time::timeout_at(
            tokio::time::Instant::from_std(deadline),
            route.media.close_until(deadline),
        )
        .await
        {
            Ok(Ok(ConnectMediaCloseOutcome::Graceful)) => {}
            Ok(Ok(ConnectMediaCloseOutcome::DeadlineAborted)) => {
                warn!("Amazon media close reached its absolute deadline");
            }
            Ok(Err(error)) => {
                warn!(
                    error_class = ?error.classification(),
                    "Amazon media close failed"
                );
            }
            Err(_) => {
                route.media.abort();
                warn!("Amazon media connector exceeded its absolute close deadline");
            }
        }
    }

    async fn stop_owned_contact(&self, route: Route) -> crate::errors::Result<()> {
        self.close_media(&route).await;
        let cleanup = RetainedAmazonConnectCleanup::from_request(
            route.profile_id.clone(),
            &route.stop_request,
        );
        let stop_result = stop_contact_with_retry(&route.starter, route.stop_request.clone()).await;
        let resolve_result = match stop_result {
            Ok(()) => match cleanup {
                Some(cleanup) => notify_cleanup_resolved(&self.cleanup_observer, cleanup).await,
                None => Ok(()),
            },
            Err(error) => Err(error),
        };
        if let Err(error) = resolve_result {
            self.pending_cleanups.insert(
                PendingCleanupKey::from_request(&route.stop_request),
                PendingCleanupRecord {
                    profile_id: route.profile_id,
                    request: route.stop_request,
                    starter: Arc::clone(&route.starter),
                    _permit: Arc::clone(&route.cleanup_permit),
                },
            );
            return Err(error);
        }
        Ok(())
    }

    /// Remove authority before awaiting peer/control cleanup. Exactly one
    /// racing terminal path can obtain the active route and Stop ownership.
    async fn terminate_active(
        &self,
        connection_id: &ConnectionId,
        terminal: AdapterEvent,
        join_supervisor: bool,
    ) -> crate::errors::Result<bool> {
        let Some((_, route)) = self.routes.remove(connection_id) else {
            return Ok(false);
        };
        if let Some(prepared) = route.prepared.as_ref() {
            prepared.mark_not_live();
            self.prepared_routes.remove(connection_id);
        } else {
            route.cancel.notify_waiters();
        }
        let supervisor = if join_supervisor {
            route.supervisor.lock().await.take()
        } else {
            None
        };
        if let Some(mut supervisor) = supervisor {
            let join_timeout = self
                .config
                .signaling_timeout
                .min(Duration::from_secs(5))
                .max(Duration::from_millis(100));
            if tokio::time::timeout(join_timeout, &mut supervisor)
                .await
                .is_err()
            {
                supervisor.abort();
                let _ = supervisor.await;
                warn!("Amazon route supervisor exceeded its shutdown deadline");
            }
        }
        let result = self.stop_owned_contact(route.clone()).await;
        self.publish_terminal(route.prepared.as_ref(), terminal)
            .await;
        result.map(|()| true)
    }
}

struct PendingCleanupRecord {
    profile_id: ConnectProfileId,
    request: StopContactRequest,
    starter: Arc<dyn ConnectContactStarter>,
    _permit: Arc<OwnedSemaphorePermit>,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct PendingCleanupKey {
    instance_id: String,
    contact_id: String,
}

impl PendingCleanupKey {
    fn from_request(request: &StopContactRequest) -> Self {
        Self {
            instance_id: request.instance_id.clone(),
            contact_id: request.contact_id.clone(),
        }
    }
}

impl fmt::Debug for PendingCleanupKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingCleanupKey")
            .field("instance_id", &"[redacted]")
            .field("contact_id", &"[redacted]")
            .finish()
    }
}

type PendingCleanupMap = Arc<DashMap<PendingCleanupKey, PendingCleanupRecord>>;

/// Observable milestones inside the adapter's control/media setup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContactSetupStage {
    /// `StartWebRTCContact` succeeded and cleanup ownership was acquired.
    ContactStarted,
    /// Route insertion completed and the route owns StopContact cleanup.
    RouteOwned,
}

/// Non-blocking setup observer used by the screen-pop server lifecycle feed.
pub type ContactSetupObserver = Arc<dyn Fn(ContactSetupStage) + Send + Sync>;

/// Cancellation-safe owner for a successfully started Connect contact. On
/// ordinary failures and future cancellation, `Drop` schedules StopContact.
/// Successful setup disarms it only after the route owns the stop request.
struct StartedContactGuard {
    profile_id: ConnectProfileId,
    starter: Arc<dyn ConnectContactStarter>,
    request: Option<StopContactRequest>,
    permit: Arc<OwnedSemaphorePermit>,
    pending: PendingCleanupMap,
    cleanup_observer: CleanupObserverSlot,
}

impl StartedContactGuard {
    async fn new(
        profile_id: ConnectProfileId,
        starter: Arc<dyn ConnectContactStarter>,
        request: StopContactRequest,
        permit: Arc<OwnedSemaphorePermit>,
        pending: PendingCleanupMap,
        cleanup_observer: CleanupObserverSlot,
    ) -> crate::errors::Result<Self> {
        let guard = Self {
            profile_id,
            starter,
            request: Some(request),
            permit,
            pending,
            cleanup_observer,
        };
        let retained_request = guard.request.as_ref().ok_or_else(|| {
            ConnectError::Control("new cleanup guard did not retain its request".into())
        })?;
        if let Some(cleanup) =
            RetainedAmazonConnectCleanup::from_request(guard.profile_id.clone(), retained_request)
        {
            notify_cleanup_retained(&guard.cleanup_observer, cleanup).await?;
        }
        Ok(guard)
    }

    fn disarm(&mut self) -> Option<StopContactRequest> {
        self.request.take()
    }

    fn request(&self) -> Option<StopContactRequest> {
        self.request.clone()
    }

    async fn stop_now(&mut self) -> crate::errors::Result<()> {
        let Some(request) = self.request.clone() else {
            return Ok(());
        };
        match stop_contact_with_retry(&self.starter, request.clone()).await {
            Ok(()) => {
                let resolve_result = match RetainedAmazonConnectCleanup::from_request(
                    self.profile_id.clone(),
                    &request,
                ) {
                    Some(cleanup) => notify_cleanup_resolved(&self.cleanup_observer, cleanup).await,
                    None => Ok(()),
                };
                if let Err(error) = resolve_result {
                    self.request.take();
                    self.pending.insert(
                        PendingCleanupKey::from_request(&request),
                        PendingCleanupRecord {
                            profile_id: self.profile_id.clone(),
                            request,
                            starter: Arc::clone(&self.starter),
                            _permit: Arc::clone(&self.permit),
                        },
                    );
                    return Err(error);
                }
                self.request.take();
                Ok(())
            }
            Err(error) => {
                // Transfer exact ownership to the retained retry map only
                // after the bounded StopContact operation returns. If this
                // future is cancelled while awaiting I/O, `self.request`
                // remains armed and Drop continues cleanup instead of losing
                // an ambiguous contact.
                self.request.take();
                self.pending.insert(
                    PendingCleanupKey::from_request(&request),
                    PendingCleanupRecord {
                        profile_id: self.profile_id.clone(),
                        request,
                        starter: Arc::clone(&self.starter),
                        _permit: Arc::clone(&self.permit),
                    },
                );
                Err(error)
            }
        }
    }
}

const STOP_CONTACT_ATTEMPTS: usize = 3;

async fn stop_contact_with_retry(
    starter: &Arc<dyn ConnectContactStarter>,
    request: StopContactRequest,
) -> crate::errors::Result<()> {
    for attempt in 1..=STOP_CONTACT_ATTEMPTS {
        match starter.stop_contact(request.clone()).await {
            Ok(()) => return Ok(()),
            Err(error) if error.is_retryable() && attempt < STOP_CONTACT_ATTEMPTS => {
                warn!(attempt, error_class = ?error.classification(), "transient StopContact failure; retrying");
                tokio::time::sleep(Duration::from_millis(10 * attempt as u64)).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(ConnectError::TransientControl(
        "StopContact retry budget exhausted".into(),
    ))
}

impl Drop for StartedContactGuard {
    fn drop(&mut self) {
        let Some(request) = self.request.take() else {
            return;
        };
        let starter = Arc::clone(&self.starter);
        let pending = Arc::clone(&self.pending);
        let permit = Arc::clone(&self.permit);
        let profile_id = self.profile_id.clone();
        let cleanup_observer = Arc::clone(&self.cleanup_observer);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                match stop_contact_with_retry(&starter, request.clone()).await {
                    Ok(()) => {
                        let resolve_result = match RetainedAmazonConnectCleanup::from_request(
                            profile_id.clone(),
                            &request,
                        ) {
                            Some(cleanup) => {
                                notify_cleanup_resolved(&cleanup_observer, cleanup).await
                            }
                            None => Ok(()),
                        };
                        if let Err(error) = resolve_result {
                            pending.insert(
                                PendingCleanupKey::from_request(&request),
                                PendingCleanupRecord {
                                    profile_id,
                                    request,
                                    starter,
                                    _permit: permit,
                                },
                            );
                            warn!(
                                error_class = ?error.classification(),
                                "failed to persist resolved Connect cleanup"
                            );
                        }
                    }
                    Err(error) => {
                        pending.insert(
                            PendingCleanupKey::from_request(&request),
                            PendingCleanupRecord {
                                profile_id,
                                request,
                                starter,
                                _permit: permit,
                            },
                        );
                        warn!(%error, "failed to stop Connect contact during setup cleanup");
                    }
                }
            });
        } else {
            pending.insert(
                PendingCleanupKey::from_request(&request),
                PendingCleanupRecord {
                    profile_id,
                    request: request.clone(),
                    starter: Arc::clone(&self.starter),
                    _permit: permit,
                },
            );
            warn!("cannot stop Connect contact: no Tokio runtime during cleanup");
        }
    }
}

/// Cancellation-safe ownership of a contact recovered through the stable
/// `StartWebRTCContact` client token.
///
/// The recovery path performs control-plane I/O only; it never joins Chime or
/// creates WebRTC media. Dropping this value schedules an exact StopContact,
/// while [`Self::stop`] allows the durable caller to await that cleanup.
pub struct RecoveredAmazonConnectContact {
    profile_id: ConnectProfileId,
    instance_id: String,
    external_reference: ExternalConnectionReference,
    cleanup: StartedContactGuard,
}

impl RecoveredAmazonConnectContact {
    /// Exact configured profile whose starter owns Start/Stop authority.
    pub fn profile_id(&self) -> &ConnectProfileId {
        &self.profile_id
    }

    /// Exact Connect instance that owns the recovered contact.
    ///
    /// Treat this identifier as sensitive routing metadata and do not use it
    /// as a metric label.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Exact, redacted-by-default contact reference suitable for durable
    /// persistence by the application.
    pub fn external_reference(&self) -> &ExternalConnectionReference {
        &self.external_reference
    }

    /// Await the exact StopContact operation once. Repeated calls are local
    /// no-ops after ownership has been discharged or retained for retry.
    pub async fn stop(&mut self) -> crate::errors::Result<()> {
        self.cleanup.stop_now().await
    }
}

impl fmt::Debug for RecoveredAmazonConnectContact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveredAmazonConnectContact")
            .field("profile_id", &self.profile_id)
            .field("instance_id", &"[redacted]")
            .field("external_reference", &self.external_reference)
            .finish()
    }
}

impl Drop for RecoveredAmazonConnectContact {
    fn drop(&mut self) {
        self.instance_id.zeroize();
    }
}

/// Lightweight runtime counters.
#[derive(Clone, Debug, Default)]
pub struct ConnectMetrics {
    pub contacts_started: u64,
    pub active_sessions: usize,
    pub failures: u64,
}

/// Result of an absolute-deadline Amazon adapter drain.
///
/// Tasks that exceed the caller's deadline remain detached and retain their
/// exact StopContact ownership; they are never aborted in the ambiguous
/// post-Start window.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AmazonConnectDrainReport {
    pub attempted_routes: usize,
    pub completed_routes: usize,
    pub failed_routes: usize,
    pub detached_cleanups: usize,
    pub in_flight_setups: usize,
    pub remaining_routes: usize,
    pub pending_contact_cleanups: usize,
}

impl AmazonConnectDrainReport {
    pub fn is_complete(&self) -> bool {
        self.failed_routes == 0
            && self.detached_cleanups == 0
            && self.in_flight_setups == 0
            && self.remaining_routes == 0
            && self.pending_contact_cleanups == 0
    }
}

const MAX_CONNECT_PROFILES: usize = 128;

/// Safe profile-registry construction failure.
#[derive(Clone, Copy, Debug, Eq, thiserror::Error, PartialEq)]
#[non_exhaustive]
pub enum ConnectProfileResolverError {
    /// The same configured profile ID was registered more than once.
    #[error("duplicate Amazon Connect profile")]
    DuplicateProfile,
    /// The adapter's defensive profile cardinality bound was exceeded.
    #[error("too many Amazon Connect profiles")]
    TooManyProfiles,
}

/// Builder for one profile-resolving adapter.
///
/// `new` installs the supplied legacy starter under the non-secret `default`
/// profile as well as retaining it for the frozen `originate_contact*` path.
pub struct AmazonConnectAdapterBuilder {
    config: ConnectConfig,
    legacy_starter: Arc<dyn ConnectContactStarter>,
    profiles: BTreeMap<ConnectProfileId, Arc<dyn ConnectContactStarter>>,
    media_connector: Arc<dyn ConnectMediaConnector>,
}

impl AmazonConnectAdapterBuilder {
    /// Begin with the source-compatible legacy/default starter.
    pub fn new(config: ConnectConfig, starter: Arc<dyn ConnectContactStarter>) -> Self {
        let mut profiles = BTreeMap::new();
        profiles.insert(ConnectProfileId::default(), Arc::clone(&starter));
        Self {
            config,
            legacy_starter: starter,
            profiles,
            media_connector: Arc::new(ChimeWebRtcMediaConnector),
        }
    }

    /// Replace the production Chime+rvoip-WebRTC connector. This seam is
    /// intended for hermetic lifecycle tests and specialized media policy;
    /// the frozen constructor continues to install the production connector.
    pub fn set_media_connector(&mut self, connector: Arc<dyn ConnectMediaConnector>) -> &mut Self {
        self.media_connector = connector;
        self
    }

    /// Consuming builder-style media connector replacement.
    pub fn with_media_connector(mut self, connector: Arc<dyn ConnectMediaConnector>) -> Self {
        self.set_media_connector(connector);
        self
    }

    /// Register another exact profile. Duplicate IDs fail closed rather than
    /// silently replacing an account/region client.
    pub fn register_profile(
        &mut self,
        profile_id: ConnectProfileId,
        starter: Arc<dyn ConnectContactStarter>,
    ) -> Result<&mut Self, ConnectProfileResolverError> {
        if self.profiles.contains_key(&profile_id) {
            return Err(ConnectProfileResolverError::DuplicateProfile);
        }
        if self.profiles.len() >= MAX_CONNECT_PROFILES {
            return Err(ConnectProfileResolverError::TooManyProfiles);
        }
        self.profiles.insert(profile_id, starter);
        Ok(self)
    }

    /// Consuming builder-style profile registration.
    pub fn with_profile(
        mut self,
        profile_id: ConnectProfileId,
        starter: Arc<dyn ConnectContactStarter>,
    ) -> Result<Self, ConnectProfileResolverError> {
        self.register_profile(profile_id, starter)?;
        Ok(self)
    }

    /// Build one adapter with a single profile resolver.
    pub fn build(self) -> Arc<AmazonConnectAdapter> {
        AmazonConnectAdapter::from_parts(
            self.config,
            self.legacy_starter,
            self.profiles,
            self.media_connector,
        )
    }
}

impl fmt::Debug for AmazonConnectAdapterBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AmazonConnectAdapterBuilder")
            .field("profile_count", &self.profiles.len())
            .finish()
    }
}

/// Amazon Connect interop adapter.
pub struct AmazonConnectAdapter {
    config: ConnectConfig,
    /// WebRTC peer/media settings (ICE servers are overridden per-contact from
    /// the Chime JOIN_ACK TURN credentials).
    webrtc: WebRtcConfig,
    starter: Arc<dyn ConnectContactStarter>,
    profiles: BTreeMap<ConnectProfileId, Arc<dyn ConnectContactStarter>>,
    media_connector: Arc<dyn ConnectMediaConnector>,
    routes: Arc<DashMap<ConnectionId, Route>>,
    prepared_routes: Arc<DashMap<ConnectionId, Arc<AmazonOutboundRoute>>>,
    events_tx: mpsc::Sender<AdapterEvent>,
    events_rx: SyncMutex<Option<mpsc::Receiver<AdapterEvent>>>,
    lifecycle: AdapterLifecycleSinkSlot,
    contacts_started: Arc<AtomicUsize>,
    failures: Arc<AtomicUsize>,
    cleanup_slots: Arc<Semaphore>,
    pending_cleanups: PendingCleanupMap,
    cleanup_observer: CleanupObserverSlot,
    draining: Arc<AtomicBool>,
    setups_in_flight: Arc<AtomicUsize>,
    setup_idle: Arc<Notify>,
}

impl AmazonConnectAdapter {
    /// Construct with the connect configuration and a control-plane starter.
    ///
    /// Use [`crate::control::AwsConnectStarter`] (feature `aws-control`) for the
    /// real AWS path, or a mock for tests.
    pub fn new(config: ConnectConfig, starter: Arc<dyn ConnectContactStarter>) -> Arc<Self> {
        AmazonConnectAdapterBuilder::new(config, starter).build()
    }

    /// Build one adapter with multiple non-secret profile mappings.
    pub fn builder(
        config: ConnectConfig,
        starter: Arc<dyn ConnectContactStarter>,
    ) -> AmazonConnectAdapterBuilder {
        AmazonConnectAdapterBuilder::new(config, starter)
    }

    /// Create a new adapter runtime for registration with another
    /// [`rvoip_core::Orchestrator`].
    ///
    /// The fork snapshots immutable configuration and WebRTC policy while
    /// sharing the thread-safe profile starters and media connector. It never
    /// copies or shares routes, prepared activations, event receivers,
    /// lifecycle sinks, counters, drain state, cleanup capacity, or pending
    /// StopContact ownership. This is required when a screen-pop server and a
    /// generic call engine need separate consumers of the adapter event
    /// stream; registering the same adapter instance twice is unsupported.
    pub fn fork_isolated(&self) -> Arc<Self> {
        Self::from_parts(
            self.config.clone(),
            Arc::clone(&self.starter),
            self.profiles.clone(),
            Arc::clone(&self.media_connector),
        )
        .with_webrtc_config(self.webrtc.clone())
    }

    fn from_parts(
        config: ConnectConfig,
        starter: Arc<dyn ConnectContactStarter>,
        profiles: BTreeMap<ConnectProfileId, Arc<dyn ConnectContactStarter>>,
        media_connector: Arc<dyn ConnectMediaConnector>,
    ) -> Arc<Self> {
        let (events_tx, events_rx) = mpsc::channel(ADAPTER_EVENT_CAP);
        // Full-gather (trickle off) so the SUBSCRIBE frame carries a complete
        // SDP offer — Chime's signaling expects the offer inline.
        let webrtc = WebRtcConfig {
            trickle_ice: false,
            ..WebRtcConfig::default()
        };
        Arc::new(Self {
            config,
            webrtc,
            starter,
            profiles,
            media_connector,
            routes: Arc::new(DashMap::new()),
            prepared_routes: Arc::new(DashMap::new()),
            events_tx,
            events_rx: SyncMutex::new(Some(events_rx)),
            lifecycle: AdapterLifecycleSinkSlot::default(),
            contacts_started: Arc::new(AtomicUsize::new(0)),
            failures: Arc::new(AtomicUsize::new(0)),
            cleanup_slots: Arc::new(Semaphore::new(MAX_OWNED_CONTACT_CLEANUPS)),
            pending_cleanups: Arc::new(DashMap::new()),
            cleanup_observer: Arc::new(SyncRwLock::new(None)),
            draining: Arc::new(AtomicBool::new(false)),
            setups_in_flight: Arc::new(AtomicUsize::new(0)),
            setup_idle: Arc::new(Notify::new()),
        })
    }

    fn try_setup_admission(&self) -> RvoipResult<AmazonSetupAdmission> {
        if self.draining.load(Ordering::Acquire) {
            return Err(RvoipError::AdmissionRejected(
                "Amazon Connect adapter is draining",
            ));
        }
        self.setups_in_flight.fetch_add(1, Ordering::AcqRel);
        if self.draining.load(Ordering::Acquire) {
            if self.setups_in_flight.fetch_sub(1, Ordering::AcqRel) == 1 {
                self.setup_idle.notify_waiters();
            }
            return Err(RvoipError::AdmissionRejected(
                "Amazon Connect adapter is draining",
            ));
        }
        Ok(AmazonSetupAdmission {
            in_flight: Arc::clone(&self.setups_in_flight),
            idle: Arc::clone(&self.setup_idle),
        })
    }

    /// Recovery is cleanup authority, not new call admission. It remains
    /// available after `begin_drain`, but participates in the same in-flight
    /// accounting so a bounded drain can wait or report a detached owner.
    fn recovery_admission(&self) -> AmazonSetupAdmission {
        self.setups_in_flight.fetch_add(1, Ordering::AcqRel);
        AmazonSetupAdmission {
            in_flight: Arc::clone(&self.setups_in_flight),
            idle: Arc::clone(&self.setup_idle),
        }
    }

    fn try_cleanup_permit(&self) -> crate::errors::Result<Arc<OwnedSemaphorePermit>> {
        Arc::clone(&self.cleanup_slots)
            .try_acquire_owned()
            .map(Arc::new)
            .map_err(|_| {
                ConnectError::Control("Amazon Connect contact cleanup capacity is exhausted".into())
            })
    }

    /// Stop admitting new contacts. Returns `true` for the owner that began
    /// the drain and `false` when draining was already active.
    pub fn begin_drain(&self) -> bool {
        !self.draining.swap(true, Ordering::AcqRel)
    }

    pub fn is_draining(&self) -> bool {
        self.draining.load(Ordering::Acquire)
    }

    async fn wait_for_setup_quiescence(&self, deadline: Instant) -> bool {
        loop {
            let idle = self.setup_idle.notified();
            if self.setups_in_flight.load(Ordering::Acquire) == 0 {
                return true;
            }
            if tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), idle)
                .await
                .is_err()
            {
                return false;
            }
        }
    }

    /// Drain every prepared or active route to one absolute deadline.
    ///
    /// A timed-out cleanup is detached, not aborted, so a control request that
    /// may already have created a contact can still reconcile and Stop it.
    pub async fn drain_until(self: &Arc<Self>, deadline: Instant) -> AmazonConnectDrainReport {
        self.begin_drain();
        let _setups_quiesced = self.wait_for_setup_quiescence(deadline).await;

        let mut route_ids = HashSet::new();
        route_ids.extend(self.prepared_routes.iter().map(|entry| entry.key().clone()));
        route_ids.extend(self.routes.iter().map(|entry| entry.key().clone()));
        for connection_id in &route_ids {
            if let Some(prepared) = self.prepared_routes.get(connection_id) {
                prepared.mark_not_live();
            }
            if let Some(route) = self.routes.get(connection_id) {
                route.cancel.notify_waiters();
            }
        }

        let attempted_routes = route_ids.len();
        let mut tasks = route_ids
            .into_iter()
            .map(|connection_id| {
                let adapter = Arc::clone(self);
                tokio::spawn(async move { adapter.end(connection_id, EndReason::Cancelled).await })
            })
            .collect::<Vec<_>>();
        let mut completed_routes = 0;
        let mut failed_routes = 0;
        let mut detached_cleanups = 0;
        while let Some(mut task) = tasks.pop() {
            match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), &mut task).await
            {
                Ok(Ok(Ok(()))) => completed_routes += 1,
                Ok(Ok(Err(_))) | Ok(Err(_)) => failed_routes += 1,
                Err(_) => {
                    // Dropping JoinHandle detaches the task. Every future owns
                    // its route/cleanup authority and will finish independently.
                    detached_cleanups += 1;
                    drop(task);
                    for remaining in tasks.drain(..) {
                        if remaining.is_finished() {
                            match remaining.await {
                                Ok(Ok(())) => completed_routes += 1,
                                Ok(Err(_)) | Err(_) => failed_routes += 1,
                            }
                        } else {
                            detached_cleanups += 1;
                            drop(remaining);
                        }
                    }
                    break;
                }
            }
        }
        drop(tasks);

        let mut remaining_route_ids = HashSet::new();
        remaining_route_ids.extend(self.prepared_routes.iter().map(|entry| entry.key().clone()));
        remaining_route_ids.extend(self.routes.iter().map(|entry| entry.key().clone()));
        AmazonConnectDrainReport {
            attempted_routes,
            completed_routes,
            failed_routes,
            detached_cleanups,
            in_flight_setups: self.setups_in_flight.load(Ordering::Acquire),
            remaining_routes: remaining_route_ids.len(),
            pending_contact_cleanups: self.pending_cleanups.len(),
        }
    }

    /// Number of configured account/region profiles. Profile IDs are omitted
    /// to avoid accidental high-cardinality diagnostics.
    pub fn configured_profile_count(&self) -> usize {
        self.profiles.len()
    }

    fn resolve_profile(
        &self,
        profile_id: &ConnectProfileId,
    ) -> Option<Arc<dyn ConnectContactStarter>> {
        self.profiles.get(profile_id).map(Arc::clone)
    }

    fn resolve_generic_context(
        &self,
        request: &OriginateRequest,
    ) -> RvoipResult<(
        Arc<AmazonConnectOriginateContext>,
        Arc<dyn ConnectContactStarter>,
    )> {
        if request.context.is_empty() {
            return Err(RvoipError::AdmissionRejected(
                "Amazon Connect originate context is required",
            ));
        }
        let context = request
            .context
            .downcast_arc::<AmazonConnectOriginateContext>()
            .ok_or(RvoipError::AdmissionRejected(
                "Amazon Connect originate context type mismatch",
            ))?;
        context.validate().map_err(|_| {
            RvoipError::AdmissionRejected("Amazon Connect originate context failed validation")
        })?;
        let starter =
            self.resolve_profile(context.profile_id())
                .ok_or(RvoipError::AdmissionRejected(
                    "Amazon Connect profile is not configured",
                ))?;
        Ok((context, starter))
    }

    fn make_transport_handle(
        &self,
        connection_id: ConnectionId,
        cancel: Arc<Notify>,
    ) -> Arc<AmazonConnectTransportHandle> {
        Arc::new(AmazonConnectTransportHandle {
            connection_id,
            prepared_routes: Arc::downgrade(&self.prepared_routes),
            active_routes: Arc::downgrade(&self.routes),
            cancel,
        })
    }

    fn build_prepared_connection(
        &self,
        request: &OriginateRequest,
        connection_id: ConnectionId,
        handle: Arc<AmazonConnectTransportHandle>,
    ) -> Connection {
        Connection {
            id: connection_id,
            session_id: request.session_id.clone(),
            participant_id: request.participant_id.clone(),
            transport: Transport::AmazonConnect,
            direction: Direction::Outbound,
            state: ConnectionState::Connecting,
            capabilities: self.webrtc.capabilities.clone(),
            negotiated_codecs: NegotiatedCodecs::default(),
            streams: Vec::new(),
            messaging_enabled: false,
            transport_handle: TransportHandle(handle),
            opened_at: Utc::now(),
            closed_at: None,
        }
    }

    fn activation_environment(&self) -> AmazonActivationEnvironment {
        AmazonActivationEnvironment {
            config: self.config.clone(),
            webrtc: self.webrtc.clone(),
            media_connector: Arc::clone(&self.media_connector),
            routes: Arc::clone(&self.routes),
            prepared_routes: Arc::clone(&self.prepared_routes),
            events_tx: self.events_tx.clone(),
            lifecycle: self.lifecycle.clone(),
            contacts_started: Arc::clone(&self.contacts_started),
            failures: Arc::clone(&self.failures),
            cleanup_slots: Arc::clone(&self.cleanup_slots),
            pending_cleanups: Arc::clone(&self.pending_cleanups),
            cleanup_observer: Arc::clone(&self.cleanup_observer),
        }
    }

    /// Override the WebRTC peer/media configuration (builder-style). ICE
    /// servers set here are still merged with the per-contact TURN credentials.
    pub fn with_webrtc_config(mut self: Arc<Self>, webrtc: WebRtcConfig) -> Arc<Self> {
        if let Some(me) = Arc::get_mut(&mut self) {
            me.webrtc = WebRtcConfig {
                trickle_ice: false,
                ..webrtc
            };
        }
        self
    }

    /// Synchronously fetch the media streams for a connection without going
    /// through the async `ConnectionAdapter::streams` path. Returns `None` when
    /// the connection is unknown. Used by the batteries-included server to
    /// bridge a freshly-originated contact.
    pub fn streams_for(&self, conn: &ConnectionId) -> Option<Vec<Arc<dyn MediaStream>>> {
        self.routes.get(conn).map(|route| route.media.streams())
    }

    /// Snapshot of runtime counters.
    pub fn metrics(&self) -> ConnectMetrics {
        ConnectMetrics {
            contacts_started: self.contacts_started.load(Ordering::Relaxed) as u64,
            active_sessions: self.routes.len(),
            failures: self.failures.load(Ordering::Relaxed) as u64,
        }
    }

    /// Number of Connect contacts whose StopContact ownership is retained
    /// after the bounded retry budget was exhausted.
    pub fn pending_cleanup_count(&self) -> usize {
        self.pending_cleanups.len()
    }

    /// Install the process-owned durable cleanup observer before admitting
    /// calls. Replacing a live observer is rejected so two persistence
    /// authorities cannot race or acknowledge one another's records.
    pub fn install_cleanup_observer(
        &self,
        observer: Arc<dyn AmazonConnectCleanupObserver>,
    ) -> crate::errors::Result<()> {
        if self.contacts_started.load(Ordering::Acquire) != 0
            || !self.routes.is_empty()
            || !self.prepared_routes.is_empty()
            || !self.pending_cleanups.is_empty()
        {
            return Err(ConnectError::Control(
                "Amazon cleanup observer must be installed before call admission".into(),
            ));
        }
        let mut slot = self.cleanup_observer.write();
        if slot.is_some() {
            return Err(ConnectError::Control(
                "Amazon cleanup observer was already installed".into(),
            ));
        }
        *slot = Some(observer);
        Ok(())
    }

    /// Redaction-safe count plus exact caller-owned values for durable
    /// recovery. Callers must never log the returned identifiers.
    pub fn retained_cleanup_records(&self) -> Vec<RetainedAmazonConnectCleanup> {
        self.pending_cleanups
            .iter()
            .filter_map(|record| {
                RetainedAmazonConnectCleanup::from_request(
                    record.value().profile_id.clone(),
                    &record.value().request,
                )
            })
            .collect()
    }

    async fn ensure_generic_activation_started(
        &self,
        connection_id: ConnectionId,
        prepared: Arc<AmazonOutboundRoute>,
    ) {
        if prepared
            .activation_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let environment = self.activation_environment();
        let task_prepared = Arc::clone(&prepared);
        let task_connection_id = connection_id.clone();
        let task = tokio::spawn(async move {
            let outcome = Self::activate_generic_route(
                environment.clone(),
                task_connection_id.clone(),
                Arc::clone(&task_prepared),
            )
            .await;
            match outcome {
                Ok(receipt) => {
                    task_prepared
                        .activation
                        .send_replace(AmazonActivationOutcome::Succeeded(receipt));
                }
                Err(class) => {
                    environment.failures.fetch_add(1, Ordering::Relaxed);
                    let activation_owned_terminal = task_prepared.retire_if_live();
                    environment.prepared_routes.remove(&task_connection_id);
                    let failure = AdapterEvent::Failed {
                        connection_id: task_connection_id.clone(),
                        detail: "Amazon Connect outbound activation failed".into(),
                    };
                    if activation_owned_terminal {
                        environment
                            .publish_terminal(Some(&task_prepared), failure)
                            .await;
                    }
                    task_prepared
                        .activation
                        .send_replace(AmazonActivationOutcome::Failed(class));
                }
            }
        });
        *prepared.activation_task.lock().await = Some(task);
    }

    async fn await_generic_activation(
        &self,
        connection_id: ConnectionId,
        prepared: Arc<AmazonOutboundRoute>,
    ) -> RvoipResult<OutboundActivation> {
        let mut outcome = prepared.activation.subscribe();
        self.ensure_generic_activation_started(connection_id, Arc::clone(&prepared))
            .await;
        loop {
            match outcome.borrow_and_update().clone() {
                AmazonActivationOutcome::Pending => {}
                AmazonActivationOutcome::Succeeded(receipt) => return Ok(receipt),
                AmazonActivationOutcome::Failed(class) => {
                    return Err(RvoipError::Adapter(format!(
                        "Amazon Connect outbound activation failed ({class})"
                    )));
                }
            }
            outcome
                .changed()
                .await
                .map_err(|_| RvoipError::InvalidState("Amazon Connect activation owner stopped"))?;
        }
    }

    async fn start_generic_contact(
        starter: &Arc<dyn ConnectContactStarter>,
        request: StartContactRequest,
        attempt_timeout: Duration,
    ) -> crate::errors::Result<crate::control::ConnectionData> {
        const ATTEMPTS: usize = 3;
        for attempt in 1..=ATTEMPTS {
            match tokio::time::timeout(
                attempt_timeout,
                starter.start_webrtc_contact(request.clone()),
            )
            .await
            {
                Ok(Ok(connection)) => return Ok(connection),
                Ok(Err(error)) if error.is_retryable() && attempt < ATTEMPTS => {
                    warn!(
                        attempt,
                        error_class = ?error.classification(),
                        "ambiguous Amazon StartWebRTCContact; reconciling with the stable token"
                    );
                    tokio::time::sleep(Duration::from_millis(10 * attempt as u64)).await;
                }
                Ok(Err(error)) => return Err(error),
                Err(_) if attempt < ATTEMPTS => {
                    warn!(
                        attempt,
                        "ambiguous Amazon StartWebRTCContact timeout; reconciling with the stable token"
                    );
                    tokio::time::sleep(Duration::from_millis(10 * attempt as u64)).await;
                }
                Err(_) => {
                    return Err(ConnectError::TransientControl(
                        "StartWebRTCContact timed out after the reconciliation budget".into(),
                    ));
                }
            }
        }
        Err(ConnectError::TransientControl(
            "StartWebRTCContact retry budget exhausted".into(),
        ))
    }

    async fn recover_contact_without_media(
        context: AmazonConnectOriginateContext,
        starter: Arc<dyn ConnectContactStarter>,
        signaling_timeout: Duration,
        contacts_started: Arc<AtomicUsize>,
        cleanup_permit: Arc<OwnedSemaphorePermit>,
        pending_cleanups: PendingCleanupMap,
        cleanup_observer: CleanupObserverSlot,
    ) -> crate::errors::Result<RecoveredAmazonConnectContact> {
        let profile_id = context.profile_id().clone();
        let mut request = context.start_request();
        if request.validate().is_err() {
            request.zeroize_sensitive();
            return Err(ConnectError::Control(
                "Amazon Connect recovery context failed validation".into(),
            ));
        }
        let instance_id = request.instance_id.clone();
        let start_result =
            Self::start_generic_contact(&starter, request.clone(), signaling_timeout).await;
        request.zeroize_sensitive();
        let mut connection_data = start_result?;
        contacts_started.fetch_add(1, Ordering::Relaxed);
        let contact_id = connection_data.contact_id.clone();
        let mut cleanup = StartedContactGuard::new(
            profile_id.clone(),
            Arc::clone(&starter),
            StopContactRequest {
                instance_id: instance_id.clone(),
                contact_id: contact_id.clone(),
            },
            cleanup_permit,
            pending_cleanups,
            cleanup_observer,
        )
        .await?;
        if let Err(error) = connection_data.validate_cleanup_identity() {
            // A remote contact may exist even when the response cannot become
            // a durable public reference. Retain exact StopContact ownership
            // first, then compensate without exposing the malformed value.
            connection_data.zeroize_sensitive();
            let _ = cleanup.stop_now().await;
            return Err(error);
        }
        connection_data.zeroize_sensitive();
        let external_reference = match ExternalConnectionReference::new(
            AMAZON_CONNECT_CONTACT_REFERENCE_KIND,
            contact_id,
        ) {
            Ok(reference) => reference,
            Err(_) => {
                let _ = cleanup.stop_now().await;
                return Err(ConnectError::Control(
                    "Amazon Connect recovery returned an invalid contact reference".into(),
                ));
            }
        };
        Ok(RecoveredAmazonConnectContact {
            profile_id,
            instance_id,
            external_reference,
            cleanup,
        })
    }

    async fn activate_generic_route(
        environment: AmazonActivationEnvironment,
        connection_id: ConnectionId,
        prepared: Arc<AmazonOutboundRoute>,
    ) -> Result<OutboundActivation, &'static str> {
        if !prepared.is_live() {
            return Err("cancelled-before-start");
        }
        let cleanup_permit = Arc::new(
            Arc::clone(&environment.cleanup_slots)
                .try_acquire_owned()
                .map_err(|_| "cleanup-capacity")?,
        );
        let mut request = prepared.context.start_request();
        request.validate().map_err(|_| "invalid-context")?;
        let instance_id = request.instance_id.clone();
        let start_result = Self::start_generic_contact(
            &prepared.starter,
            request.clone(),
            environment.config.signaling_timeout,
        )
        .await;
        request.zeroize_sensitive();
        let mut connection_data = start_result.map_err(|_| "start-contact")?;
        environment.contacts_started.fetch_add(1, Ordering::Relaxed);
        if connection_data.validate_cleanup_identity().is_err() {
            connection_data.zeroize_sensitive();
            return Err("invalid-cleanup-identity");
        }
        let contact_id = connection_data.contact_id.clone();
        let mut cleanup = StartedContactGuard::new(
            prepared.context.profile_id().clone(),
            Arc::clone(&prepared.starter),
            StopContactRequest {
                instance_id,
                contact_id: contact_id.clone(),
            },
            Arc::clone(&cleanup_permit),
            Arc::clone(&environment.pending_cleanups),
            Arc::clone(&environment.cleanup_observer),
        )
        .await
        .map_err(|_| "cleanup-journal")?;
        if connection_data.validate().is_err() {
            let _ = cleanup.stop_now().await;
            connection_data.zeroize_sensitive();
            return Err("invalid-start-response");
        }
        let reference = match ExternalConnectionReference::new(
            AMAZON_CONNECT_CONTACT_REFERENCE_KIND,
            contact_id.clone(),
        ) {
            Ok(reference) => reference,
            Err(_) => {
                let _ = cleanup.stop_now().await;
                connection_data.zeroize_sensitive();
                return Err("invalid-contact-reference");
            }
        };
        if !prepared.is_live() {
            let _ = cleanup.stop_now().await;
            connection_data.zeroize_sensitive();
            return Err("cancelled-after-start");
        }

        let media_result = tokio::time::timeout(
            environment.config.media_connect_timeout,
            environment.media_connector.connect(
                &connection_data,
                ConnectMediaConnectOptions {
                    webrtc: environment.webrtc.clone(),
                    signaling_timeout: environment.config.signaling_timeout,
                    media_connect_timeout: environment.config.media_connect_timeout,
                    keepalive_interval: environment.config.keepalive_interval,
                },
            ),
        )
        .await;
        connection_data.zeroize_sensitive();
        let media = match media_result {
            Ok(Ok(media)) => media,
            Ok(Err(_)) => {
                let _ = cleanup.stop_now().await;
                return Err("media-connect");
            }
            Err(_) => {
                let _ = cleanup.stop_now().await;
                return Err("media-connect-timeout");
            }
        };
        if !prepared.is_live() {
            media.abort();
            let _ = cleanup.stop_now().await;
            return Err("cancelled-after-media");
        }

        let stop_request = cleanup.request().ok_or("cleanup-ownership-lost")?;
        let route = Route {
            media,
            cancel: Arc::clone(&prepared.cancel),
            stop_request,
            profile_id: prepared.context.profile_id().clone(),
            starter: Arc::clone(&prepared.starter),
            cleanup_permit,
            prepared: Some(Arc::clone(&prepared)),
            supervisor: Arc::new(AsyncMutex::new(None)),
        };
        if environment
            .routes
            .insert(connection_id.clone(), route.clone())
            .is_some()
        {
            let _ = environment.routes.remove(&connection_id);
            route.media.abort();
            let _ = cleanup.stop_now().await;
            return Err("duplicate-route");
        }
        let _route_owns_cleanup = cleanup.disarm();

        let supervisor =
            Self::spawn_route_supervisor(environment.clone(), connection_id.clone(), route.clone());
        *route.supervisor.lock().await = Some(supervisor);
        if !environment.publish_or_stage(
            Some(&prepared),
            AdapterEvent::Connected {
                connection_id: connection_id.clone(),
            },
        ) {
            let _ = environment
                .terminate_active(
                    &connection_id,
                    AdapterEvent::Failed {
                        connection_id: connection_id.clone(),
                        detail: "Amazon Connect lifecycle publication failed".into(),
                    },
                    true,
                )
                .await;
            return Err("event-publication");
        }
        if prepared.activate_stage(&environment.events_tx).is_err()
            || !prepared.is_live()
            || !environment.routes.contains_key(&connection_id)
        {
            let _ = environment
                .terminate_active(
                    &connection_id,
                    AdapterEvent::Failed {
                        connection_id: connection_id.clone(),
                        detail: "Amazon Connect route ended during activation".into(),
                    },
                    true,
                )
                .await;
            return Err("ended-during-activation");
        }

        Ok(OutboundActivation::with_external_reference(reference))
    }

    fn spawn_route_supervisor(
        environment: AmazonActivationEnvironment,
        connection_id: ConnectionId,
        route: Route,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let prepared = route.prepared.as_ref().cloned();
            let mut terminal = route.media.subscribe_terminal();
            let mut dtmf = route.media.take_dtmf_events();
            let poll = environment
                .config
                .keepalive_interval
                .min(Duration::from_secs(1))
                .max(Duration::from_millis(50));
            let mut health = tokio::time::interval(poll);
            health.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                if prepared.as_ref().is_some_and(|route| !route.is_live()) {
                    return;
                }
                tokio::select! {
                    _ = route.cancel.notified() => return,
                    changed = terminal.changed() => {
                        let cause = if changed.is_err() {
                            ConnectMediaTerminalCause::TransportClosed
                        } else if let Some(cause) = *terminal.borrow_and_update() {
                            cause
                        } else {
                            continue;
                        };
                        let event = match cause {
                            ConnectMediaTerminalCause::RemoteEnded
                            | ConnectMediaTerminalCause::TransportClosed => AdapterEvent::Ended {
                                connection_id: connection_id.clone(),
                                reason: EndReason::Normal,
                            },
                            ConnectMediaTerminalCause::RemoteError { .. }
                            | ConnectMediaTerminalCause::TransportError
                            | ConnectMediaTerminalCause::PeerFailed => AdapterEvent::Failed {
                                connection_id: connection_id.clone(),
                                detail: "Amazon media session failed".into(),
                            },
                        };
                        let _ = environment
                            .terminate_active(&connection_id, event, false)
                            .await;
                        return;
                    }
                    event = async {
                        match dtmf.as_mut() {
                            Some(receiver) => receiver.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        let Some(event) = event else {
                            dtmf = None;
                            continue;
                        };
                        let _ = environment.publish_or_stage(
                            prepared.as_ref(),
                            AdapterEvent::Dtmf {
                                connection_id: connection_id.clone(),
                                digits: event.digit.to_string(),
                                duration_ms: event.duration_ms,
                            },
                        );
                    }
                    _ = health.tick() => {
                        let snapshot = route.media.health();
                        let expired = !environment.config.session_idle_ttl.is_zero()
                            && snapshot.last_pong_ago
                                .unwrap_or(snapshot.last_signaling_activity_ago)
                                >= environment.config.session_idle_ttl;
                        if snapshot.terminal.is_some()
                            || !snapshot.peer_connected
                            || !snapshot.signaling_running
                            || expired
                        {
                            let _ = environment.terminate_active(
                                &connection_id,
                                AdapterEvent::Failed {
                                    connection_id: connection_id.clone(),
                                    detail: if expired {
                                        "Amazon media PONG activity expired".into()
                                    } else {
                                        "Amazon media liveness failed".into()
                                    },
                                },
                                false,
                            ).await;
                            return;
                        }
                    }
                }
            }
        })
    }

    /// Retry one retained StopContact operation by Amazon contact id. Returns
    /// `false` when no pending ownership exists. The record and its bounded
    /// capacity permit are removed only after success/already-ended.
    pub async fn retry_pending_cleanup(&self, contact_id: &str) -> crate::errors::Result<bool> {
        let mut matches = self
            .pending_cleanups
            .iter()
            .filter(|record| record.key().contact_id == contact_id)
            .map(|record| record.key().clone());
        let Some(key) = matches.next() else {
            return Ok(false);
        };
        if matches.next().is_some() {
            return Err(ConnectError::Control(
                "pending cleanup contact is ambiguous across Connect instances".into(),
            ));
        }
        drop(matches);
        self.retry_pending_cleanup_for(&key.instance_id, &key.contact_id)
            .await
    }

    /// Retry one retained StopContact operation using its exact instance and
    /// contact identity. This avoids cross-account ambiguity when two profiles
    /// return the same instance-scoped contact identifier.
    pub async fn retry_pending_cleanup_for(
        &self,
        instance_id: &str,
        contact_id: &str,
    ) -> crate::errors::Result<bool> {
        let key = PendingCleanupKey {
            instance_id: instance_id.to_owned(),
            contact_id: contact_id.to_owned(),
        };
        let Some((profile_id, request, starter)) = self.pending_cleanups.get(&key).map(|record| {
            (
                record.profile_id.clone(),
                record.request.clone(),
                Arc::clone(&record.starter),
            )
        }) else {
            return Ok(false);
        };
        stop_contact_with_retry(&starter, request.clone()).await?;
        if let Some(cleanup) = RetainedAmazonConnectCleanup::from_request(profile_id, &request) {
            notify_cleanup_resolved(&self.cleanup_observer, cleanup).await?;
        }
        self.pending_cleanups.remove(&key);
        Ok(true)
    }

    /// Recover an ambiguously-started contact using the exact validated
    /// context and its caller-stable client token, without connecting media.
    ///
    /// Every retry sends a byte-identical [`StartContactRequest`]. A spawned
    /// owner retains the context, profile starter, cleanup permit, and drain
    /// accounting independently of the awaiting caller. If this future is
    /// cancelled, that owner finishes reconciliation and stops any recovered
    /// contact instead of dropping ambiguous cleanup authority. The context is
    /// borrowed and cloned into that owner so a durable caller also retains the
    /// stable token after a bounded transient failure and can reconcile again.
    pub async fn recover_contact(
        self: &Arc<Self>,
        context: &AmazonConnectOriginateContext,
    ) -> crate::errors::Result<RecoveredAmazonConnectContact> {
        context.validate().map_err(|_| {
            ConnectError::Control("Amazon Connect recovery context failed validation".into())
        })?;
        let starter = self.resolve_profile(context.profile_id()).ok_or_else(|| {
            ConnectError::Control("Amazon Connect recovery profile is not configured".into())
        })?;
        let cleanup_permit = self.try_cleanup_permit()?;
        let admission = self.recovery_admission();
        let signaling_timeout = self.config.signaling_timeout;
        let contacts_started = Arc::clone(&self.contacts_started);
        let pending_cleanups = Arc::clone(&self.pending_cleanups);
        let cleanup_observer = Arc::clone(&self.cleanup_observer);
        let context = context.clone();
        let (result_tx, result_rx) = oneshot::channel();
        tokio::spawn(async move {
            let _admission = admission;
            let result = Self::recover_contact_without_media(
                context,
                starter,
                signaling_timeout,
                contacts_started,
                cleanup_permit,
                pending_cleanups,
                cleanup_observer,
            )
            .await;
            if let Err(undelivered) = result_tx.send(result) {
                if let Ok(mut recovered) = undelivered {
                    if let Err(error) = recovered.stop().await {
                        warn!(
                            error_class = ?error.classification(),
                            "failed to stop caller-abandoned recovered Amazon contact"
                        );
                    }
                }
            }
        });
        result_rx.await.map_err(|_| {
            ConnectError::Control("Amazon Connect recovery owner stopped unexpectedly".into())
        })?
    }

    /// Stop a persisted contact reference with the exact configured profile
    /// and Connect instance that created it.
    ///
    /// The reference namespace is matched byte-for-byte. The operation is
    /// executed by a detached owner and remains safe if the awaiting caller is
    /// cancelled; a failed bounded StopContact is retained for explicit retry.
    pub async fn stop_persisted_contact(
        self: &Arc<Self>,
        profile_id: &ConnectProfileId,
        instance_id: &str,
        external_reference: &ExternalConnectionReference,
    ) -> crate::errors::Result<()> {
        if external_reference.kind() != AMAZON_CONNECT_CONTACT_REFERENCE_KIND {
            return Err(ConnectError::Control(
                "Amazon Connect contact reference kind does not match".into(),
            ));
        }
        crate::originate::validate_connect_instance_id(instance_id).map_err(|_| {
            ConnectError::Control("Amazon Connect persisted instance is invalid".into())
        })?;
        let starter = self.resolve_profile(profile_id).ok_or_else(|| {
            ConnectError::Control("Amazon Connect cleanup profile is not configured".into())
        })?;
        let request = StopContactRequest {
            instance_id: instance_id.to_owned(),
            contact_id: external_reference.expose_secret().to_owned(),
        };
        let profile_id = profile_id.clone();
        let cleanup_observer = Arc::clone(&self.cleanup_observer);
        let cleanup = RetainedAmazonConnectCleanup::from_request(profile_id.clone(), &request)
            .ok_or_else(|| {
                ConnectError::Control("Amazon Connect persisted cleanup identity is invalid".into())
            })?;
        notify_cleanup_retained(&cleanup_observer, cleanup.clone()).await?;
        let cleanup_permit = self.try_cleanup_permit()?;
        let pending_cleanups = Arc::clone(&self.pending_cleanups);
        let admission = self.recovery_admission();
        let (result_tx, result_rx) = oneshot::channel();
        tokio::spawn(async move {
            let _admission = admission;
            let key = PendingCleanupKey::from_request(&request);
            let result = stop_contact_with_retry(&starter, request.clone()).await;
            match result {
                Ok(()) => match notify_cleanup_resolved(&cleanup_observer, cleanup).await {
                    Ok(()) => {
                        pending_cleanups.remove(&key);
                        let _ = result_tx.send(Ok(()));
                    }
                    Err(error) => {
                        pending_cleanups.insert(
                            key,
                            PendingCleanupRecord {
                                profile_id,
                                request,
                                starter,
                                _permit: cleanup_permit,
                            },
                        );
                        let _ = result_tx.send(Err(error));
                    }
                },
                Err(error) => {
                    pending_cleanups.insert(
                        key,
                        PendingCleanupRecord {
                            profile_id,
                            request,
                            starter,
                            _permit: cleanup_permit,
                        },
                    );
                    let _ = result_tx.send(Err(error));
                }
            }
        });
        result_rx.await.map_err(|_| {
            ConnectError::Control("Amazon Connect cleanup owner stopped unexpectedly".into())
        })?
    }

    /// **Primary entry point.** Start an inbound WebRTC contact in Amazon
    /// Connect with the given contact `attributes` (the screen-pop channel),
    /// join the Chime meeting, establish audio, and return the connected
    /// [`ConnectionId`]. Bridge it to the inbound leg with
    /// `Orchestrator::bridge_connections`.
    pub async fn originate_contact(
        &self,
        attributes: BTreeMap<String, String>,
        display_name: Option<String>,
        description: Option<String>,
    ) -> crate::errors::Result<ConnectionId> {
        self.originate_contact_to(
            ContactTarget::default(),
            attributes,
            display_name,
            description,
        )
        .await
    }

    /// Like [`Self::originate_contact`], but with a per-call
    /// [`ContactTarget`] override of the instance/flow (multi-tenant
    /// routing). `None` fields fall back to the adapter's [`ConnectConfig`].
    pub async fn originate_contact_to(
        &self,
        target: ContactTarget,
        attributes: BTreeMap<String, String>,
        display_name: Option<String>,
        description: Option<String>,
    ) -> crate::errors::Result<ConnectionId> {
        self.originate_contact_to_observed(target, attributes, display_name, description, None)
            .await
    }

    /// Like [`Self::originate_contact_to`], with a non-blocking observer for
    /// control-plane setup milestones. Existing callers can keep using the
    /// compatibility wrapper above.
    pub async fn originate_contact_to_observed(
        &self,
        target: ContactTarget,
        attributes: BTreeMap<String, String>,
        display_name: Option<String>,
        description: Option<String>,
        observer: Option<ContactSetupObserver>,
    ) -> crate::errors::Result<ConnectionId> {
        let (conn_id, _negotiated) = self
            .establish(
                target,
                attributes,
                display_name,
                description,
                SessionId::new(),
                ParticipantId::new(),
                observer,
            )
            .await?;
        Ok(conn_id)
    }

    /// Drive control → signaling → media. Inserts the route and emits
    /// `Connected` on success; increments the failure counter otherwise.
    async fn establish(
        &self,
        target: ContactTarget,
        attributes: BTreeMap<String, String>,
        display_name: Option<String>,
        description: Option<String>,
        _session_id: SessionId,
        _participant_id: ParticipantId,
        observer: Option<ContactSetupObserver>,
    ) -> crate::errors::Result<(ConnectionId, NegotiatedCodecs)> {
        match self
            .establish_inner(
                Arc::clone(&self.starter),
                target,
                attributes,
                display_name,
                description,
                observer,
            )
            .await
        {
            Ok(ok) => Ok(ok),
            Err(e) => {
                self.failures.fetch_add(1, Ordering::Relaxed);
                Err(e)
            }
        }
    }

    async fn establish_inner(
        &self,
        starter: Arc<dyn ConnectContactStarter>,
        target: ContactTarget,
        attributes: BTreeMap<String, String>,
        display_name: Option<String>,
        description: Option<String>,
        observer: Option<ContactSetupObserver>,
    ) -> crate::errors::Result<(ConnectionId, NegotiatedCodecs)> {
        let _setup_admission = self
            .try_setup_admission()
            .map_err(|_| ConnectError::Control("Amazon Connect adapter is draining".into()))?;
        let cleanup_permit = Arc::new(
            Arc::clone(&self.cleanup_slots)
                .try_acquire_owned()
                .map_err(|_| {
                    ConnectError::Control("Connect cleanup ownership capacity exhausted".into())
                })?,
        );
        // 1. Control plane: StartWebRTCContact (attributes drive the screen pop).
        let request = StartContactRequest {
            instance_id: target
                .instance_id
                .unwrap_or_else(|| self.config.instance_id.clone()),
            contact_flow_id: target
                .contact_flow_id
                .unwrap_or_else(|| self.config.contact_flow_id.clone()),
            display_name: display_name
                .or(target.default_display_name)
                .unwrap_or_else(|| self.config.default_display_name.clone()),
            attributes,
            description,
            client_token: None,
        };
        let instance_id = request.instance_id.clone();
        let mut connection_data = starter.start_webrtc_contact(request).await?;
        self.contacts_started.fetch_add(1, Ordering::Relaxed);
        if let Err(error) = connection_data.validate_cleanup_identity() {
            connection_data.zeroize_sensitive();
            return Err(error);
        }
        let mut cleanup = StartedContactGuard::new(
            ConnectProfileId::default(),
            Arc::clone(&starter),
            StopContactRequest {
                instance_id,
                contact_id: connection_data.contact_id.clone(),
            },
            Arc::clone(&cleanup_permit),
            Arc::clone(&self.pending_cleanups),
            Arc::clone(&self.cleanup_observer),
        )
        .await?;
        if self.is_draining() {
            let _ = cleanup.stop_now().await;
            connection_data.zeroize_sensitive();
            return Err(ConnectError::Control(
                "Amazon Connect adapter began draining during setup".into(),
            ));
        }
        if let Err(response_error) = connection_data.validate() {
            let cleanup_result = cleanup.stop_now().await;
            connection_data.zeroize_sensitive();
            return match cleanup_result {
                Ok(()) => Err(response_error),
                Err(_) => Err(ConnectError::Control(
                    "invalid Connect response and cleanup failed".into(),
                )),
            };
        }
        if let Some(observer) = observer.as_ref() {
            observer(ContactSetupStage::ContactStarted);
        }
        info!("started Amazon Connect WebRTC contact");

        let outcome = async {
            // The injectable media seam owns Chime signaling, rvoip WebRTC,
            // media streams, terminal supervision, and bounded close.
            let media = self
                .media_connector
                .connect(
                    &connection_data,
                    ConnectMediaConnectOptions {
                        webrtc: self.webrtc.clone(),
                        signaling_timeout: self.config.signaling_timeout,
                        media_connect_timeout: self.config.media_connect_timeout,
                        keepalive_interval: self.config.keepalive_interval,
                    },
                )
                .await?;
            if self.is_draining() {
                media.abort();
                return Err(ConnectError::Control(
                    "Amazon Connect adapter began draining during media setup".into(),
                ));
            }
            let conn_id = ConnectionId::new();
            let negotiated = media.negotiated_codecs();
            let cancel = Arc::new(Notify::new());
            let Some(stop_request) = cleanup.request() else {
                media.abort();
                return Err(ConnectError::Control(
                    "started contact cleanup ownership was lost".into(),
                ));
            };
            let route = Route {
                media,
                cancel: Arc::clone(&cancel),
                stop_request,
                profile_id: ConnectProfileId::default(),
                starter,
                cleanup_permit,
                prepared: None,
                supervisor: Arc::new(AsyncMutex::new(None)),
            };
            self.routes.insert(conn_id.clone(), route.clone());
            let _route_owns_cleanup = cleanup.disarm();
            let supervisor = Self::spawn_route_supervisor(
                self.activation_environment(),
                conn_id.clone(),
                route.clone(),
            );
            *route.supervisor.lock().await = Some(supervisor);
            if let Some(observer) = observer.as_ref() {
                observer(ContactSetupStage::RouteOwned);
            }

            self.try_send(AdapterEvent::Connected {
                connection_id: conn_id.clone(),
            });
            Ok((conn_id, negotiated))
        }
        .await;
        connection_data.zeroize_sensitive();

        match outcome {
            Ok(success) => Ok(success),
            Err(setup_error) => match cleanup.stop_now().await {
                Ok(()) => Err(setup_error),
                Err(cleanup_error) => Err(ConnectError::Control(format!(
                    "setup failed ({setup_error}); StopContact cleanup failed ({cleanup_error})"
                ))),
            },
        }
    }

    fn route(&self, conn: &ConnectionId) -> crate::errors::Result<Route> {
        self.routes
            .get(conn)
            .map(|e| e.value().clone())
            .ok_or_else(|| ConnectError::UnknownConnection(conn.to_string()))
    }

    fn try_send(&self, event: AdapterEvent) {
        if self.events_tx.try_send(event).is_err() {
            warn!("AmazonConnectAdapter event channel full or closed");
        }
    }

    async fn teardown(
        &self,
        conn: &ConnectionId,
        terminal: AdapterEvent,
    ) -> crate::errors::Result<()> {
        let prepared = self
            .prepared_routes
            .get(conn)
            .map(|route| Arc::clone(route.value()));
        if let Some(prepared) = prepared.as_ref() {
            // Invalidate authority synchronously before any close, task join,
            // or StopContact I/O can yield.
            prepared.mark_not_live();
        }

        let environment = self.activation_environment();
        if environment.routes.contains_key(conn) {
            environment
                .terminate_active(conn, terminal.clone(), true)
                .await?;
            info!(conn = %conn, "ended Amazon Connect contact");
        } else if let Some(prepared) = prepared {
            self.prepared_routes.remove(conn);
            if let Some(mut task) = prepared.activation_task.lock().await.take() {
                let wait = self
                    .config
                    .signaling_timeout
                    .saturating_add(self.config.media_connect_timeout)
                    .saturating_add(self.config.signaling_timeout);
                if tokio::time::timeout(wait, &mut task).await.is_err() {
                    // StartWebRTCContact may have succeeded remotely while its
                    // response is still ambiguous locally. Detach the sole
                    // cleanup owner rather than aborting and losing the exact
                    // stable-token reconciliation/StopContact authority.
                    warn!("Amazon activation cleanup exceeded its bounded reconciliation deadline; detaching exact cleanup owner");
                    drop(task);
                }
            }
            // The activation task may have crossed route publication before
            // observing cancellation. Recheck once after joining it.
            if environment.routes.contains_key(conn) {
                environment.terminate_active(conn, terminal, true).await?;
            } else {
                environment
                    .publish_terminal(Some(&prepared), terminal)
                    .await;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl ConnectionAdapter for AmazonConnectAdapter {
    fn transport(&self) -> Transport {
        Transport::AmazonConnect
    }

    fn kind(&self) -> AdapterKind {
        AdapterKind::Interop
    }

    fn lifecycle_capabilities(&self) -> AdapterLifecycleCapabilities {
        AdapterLifecycleCapabilities {
            authoritative_liveness: true,
            // This adapter is outbound-only, so the authenticated inbound
            // handoff invariant is vacuously satisfied. Advertising `false`
            // prevented it from being registered on an Orchestrator that had
            // installed Bridgefu's fail-closed inbound admission gate even
            // though no Amazon route can enter through that gate.
            atomic_inbound_handoff: true,
            terminal_fallback: true,
            staged_outbound_activation: true,
        }
    }

    fn install_lifecycle_sink(&self, sink: Arc<dyn AdapterLifecycleSink>) -> RvoipResult<()> {
        self.lifecycle
            .install(sink)
            .map_err(|_| RvoipError::InvalidState("Amazon lifecycle sink already installed"))
    }

    fn is_connection_live(&self, conn: &ConnectionId) -> bool {
        self.routes.contains_key(conn)
            || self
                .prepared_routes
                .get(conn)
                .is_some_and(|route| route.is_live())
    }

    #[instrument(skip(self, request), fields(context_present = !request.context.is_empty()))]
    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        if request.direction != Direction::Outbound {
            return Err(RvoipError::AdmissionRejected(
                "Amazon Connect generic originate requires outbound direction",
            ));
        }
        let (context, selected_starter) = self.resolve_generic_context(&request)?;
        let _setup_admission = self.try_setup_admission()?;
        let connection_id = ConnectionId::new();
        let prepared = AmazonOutboundRoute::new(context, selected_starter);
        let handle =
            self.make_transport_handle(connection_id.clone(), Arc::clone(&prepared.cancel));
        let connection = self.build_prepared_connection(&request, connection_id.clone(), handle);
        if self
            .prepared_routes
            .insert(connection_id.clone(), prepared)
            .is_some()
        {
            self.prepared_routes.remove(&connection_id);
            return Err(RvoipError::AdmissionRejected(
                "Amazon Connect provisional connection ID collided",
            ));
        }
        Ok(ConnectionHandle::new(connection))
    }

    async fn activate_outbound(&self, conn: ConnectionId) -> RvoipResult<()> {
        self.activate_outbound_with_receipt(conn).await.map(|_| ())
    }

    async fn activate_outbound_with_receipt(
        &self,
        conn: ConnectionId,
    ) -> RvoipResult<OutboundActivation> {
        if self.is_draining() {
            return Err(RvoipError::AdmissionRejected(
                "Amazon Connect adapter is draining",
            ));
        }
        let prepared = self
            .prepared_routes
            .get(&conn)
            .map(|route| Arc::clone(route.value()))
            .ok_or_else(|| RvoipError::ConnectionNotFound(conn.clone()))?;
        if !prepared.is_live() {
            return Err(RvoipError::ConnectionNotFound(conn));
        }
        self.await_generic_activation(conn, prepared).await
    }

    async fn accept(&self, _conn: ConnectionId) -> RvoipResult<()> {
        // Connect contacts are established (media up) by the time the
        // ConnectionId exists, so accept is a no-op success.
        Ok(())
    }

    async fn reject(&self, conn: ConnectionId, _reason: RejectReason) -> RvoipResult<()> {
        let terminal = AdapterEvent::Failed {
            connection_id: conn.clone(),
            detail: "rejected".into(),
        };
        self.teardown(&conn, terminal)
            .await
            .map_err(RvoipError::from)?;
        Ok(())
    }

    #[instrument(skip(self), fields(conn = %conn, reason = ?reason))]
    async fn end(&self, conn: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        let terminal = AdapterEvent::Ended {
            connection_id: conn.clone(),
            reason,
        };
        self.teardown(&conn, terminal)
            .await
            .map_err(RvoipError::from)?;
        Ok(())
    }

    async fn hold(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self.route(&conn).map_err(RvoipError::from)?;
        route
            .media
            .hold()
            .await
            .map_err(|e| RvoipError::Adapter(format!("hold: {e}")))
    }

    async fn resume(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self.route(&conn).map_err(RvoipError::from)?;
        route
            .media
            .resume()
            .await
            .map_err(|e| RvoipError::Adapter(format!("resume: {e}")))
    }

    async fn transfer(&self, _conn: ConnectionId, _target: TransferTarget) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented(
            "Amazon Connect transfer is driven server-side by the contact flow",
        ))
    }

    async fn streams(&self, conn: ConnectionId) -> RvoipResult<Vec<Arc<dyn MediaStream>>> {
        let route = self.route(&conn).map_err(RvoipError::from)?;
        Ok(route.media.streams())
    }

    async fn send_dtmf(
        &self,
        conn: ConnectionId,
        digits: &str,
        duration_ms: u32,
    ) -> RvoipResult<()> {
        let route = self.route(&conn).map_err(RvoipError::from)?;
        route
            .media
            .send_dtmf(digits, duration_ms)
            .await
            .map_err(|e| RvoipError::Adapter(format!("send_dtmf: {e}")))
    }

    async fn send_message(&self, _conn: ConnectionId, _message: Message) -> RvoipResult<()> {
        Err(RvoipError::NotImplemented(
            "Amazon Connect adapter does not expose a data-channel messaging path",
        ))
    }

    async fn renegotiate_media(
        &self,
        _conn: ConnectionId,
        _capabilities: CapabilityDescriptor,
    ) -> RvoipResult<NegotiatedCodecs> {
        Err(RvoipError::NotImplemented(
            "Amazon Connect media renegotiation is not supported in v1",
        ))
    }

    fn subscribe_events(&self) -> mpsc::Receiver<AdapterEvent> {
        match self.events_rx.lock().take() {
            Some(rx) => rx,
            None => {
                warn!(
                    "AmazonConnectAdapter::subscribe_events called more than once; \
                     returning closed receiver"
                );
                let (_tx, rx) = mpsc::channel(1);
                rx
            }
        }
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        self.webrtc.capabilities.clone()
    }

    async fn verify_request_signature(
        &self,
        _conn: ConnectionId,
        _signature: SignatureHeaders,
    ) -> RvoipResult<IdentityAssurance> {
        // The Connect/Chime media leg is authenticated by the attendee join
        // token at the control plane, not by an rvoip-native signature.
        Ok(IdentityAssurance::Anonymous)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ConnectionData;
    use crate::errors::ConnectErrorClass;
    use crate::originate::{
        AmazonConnectOriginateContext, AmazonConnectTarget, ConnectClientToken,
    };
    use rvoip_core::config::Config as CoreConfig;
    use rvoip_core::connection::Direction;
    use rvoip_core::Orchestrator;
    use std::future::pending;
    use tokio::sync::{oneshot, watch};

    struct NoopLifecycleSink;

    #[async_trait]
    impl AdapterLifecycleSink for NoopLifecycleSink {
        async fn deliver_terminal(&self, _event: AdapterEvent) {}
    }

    #[derive(Default)]
    struct RecordingCleanupObserver {
        retained: SyncMutex<Vec<RetainedAmazonConnectCleanup>>,
        resolved: SyncMutex<Vec<RetainedAmazonConnectCleanup>>,
    }

    #[async_trait]
    impl AmazonConnectCleanupObserver for RecordingCleanupObserver {
        async fn retained(
            &self,
            cleanup: RetainedAmazonConnectCleanup,
        ) -> crate::errors::Result<()> {
            self.retained.lock().push(cleanup);
            Ok(())
        }

        async fn resolved(
            &self,
            cleanup: RetainedAmazonConnectCleanup,
        ) -> crate::errors::Result<()> {
            self.resolved.lock().push(cleanup);
            Ok(())
        }
    }

    struct FailingRetainCleanupObserver;

    #[async_trait]
    impl AmazonConnectCleanupObserver for FailingRetainCleanupObserver {
        async fn retained(
            &self,
            _cleanup: RetainedAmazonConnectCleanup,
        ) -> crate::errors::Result<()> {
            Err(ConnectError::Control(
                "test cleanup journal unavailable".into(),
            ))
        }

        async fn resolved(
            &self,
            _cleanup: RetainedAmazonConnectCleanup,
        ) -> crate::errors::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockMediaCalls {
        holds: AtomicUsize,
        resumes: AtomicUsize,
        dtmfs: AtomicUsize,
        closes: AtomicUsize,
        aborts: AtomicUsize,
    }

    struct MockMediaSession {
        calls: Arc<MockMediaCalls>,
        terminal_tx: watch::Sender<Option<ConnectMediaTerminalCause>>,
        terminal_rx: watch::Receiver<Option<ConnectMediaTerminalCause>>,
        dtmf_tx: mpsc::Sender<crate::media::ConnectMediaDtmfEvent>,
        dtmf_rx: SyncMutex<Option<mpsc::Receiver<crate::media::ConnectMediaDtmfEvent>>>,
        close_outcome: ConnectMediaCloseOutcome,
        block_close: bool,
    }

    impl MockMediaSession {
        fn new() -> Arc<Self> {
            let (terminal_tx, terminal_rx) = watch::channel(None);
            let (dtmf_tx, dtmf_rx) = mpsc::channel(4);
            Arc::new(Self {
                calls: Arc::new(MockMediaCalls::default()),
                terminal_tx,
                terminal_rx,
                dtmf_tx,
                dtmf_rx: SyncMutex::new(Some(dtmf_rx)),
                close_outcome: ConnectMediaCloseOutcome::Graceful,
                block_close: false,
            })
        }

        fn blocking_close() -> Arc<Self> {
            let mut session = Self::new();
            Arc::get_mut(&mut session)
                .expect("new mock session is unique")
                .block_close = true;
            session
        }

        fn terminal(&self, cause: ConnectMediaTerminalCause) {
            let _ = self.terminal_tx.send(Some(cause));
        }
    }

    #[async_trait]
    impl ConnectMediaSession for MockMediaSession {
        fn negotiated_codecs(&self) -> NegotiatedCodecs {
            NegotiatedCodecs::default()
        }

        fn streams(&self) -> Vec<Arc<dyn MediaStream>> {
            Vec::new()
        }

        fn take_dtmf_events(&self) -> Option<mpsc::Receiver<crate::media::ConnectMediaDtmfEvent>> {
            self.dtmf_rx.lock().take()
        }

        fn subscribe_terminal(&self) -> watch::Receiver<Option<ConnectMediaTerminalCause>> {
            self.terminal_rx.clone()
        }

        fn health(&self) -> crate::media::ConnectMediaHealth {
            crate::media::ConnectMediaHealth {
                peer_connected: true,
                signaling_running: true,
                last_signaling_activity_ago: Duration::ZERO,
                last_pong_ago: None,
                terminal: *self.terminal_rx.borrow(),
            }
        }

        async fn hold(&self) -> crate::errors::Result<()> {
            self.calls.holds.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn resume(&self) -> crate::errors::Result<()> {
            self.calls.resumes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn send_dtmf(&self, _digits: &str, _duration_ms: u32) -> crate::errors::Result<()> {
            self.calls.dtmfs.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn close_until(
            &self,
            _deadline: Instant,
        ) -> crate::errors::Result<ConnectMediaCloseOutcome> {
            self.calls.closes.fetch_add(1, Ordering::SeqCst);
            if self.block_close {
                pending::<()>().await;
            }
            Ok(self.close_outcome)
        }

        fn abort(&self) {
            self.calls.aborts.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct MockMediaConnector {
        session: Arc<MockMediaSession>,
        connects: AtomicUsize,
        fail: bool,
    }

    #[async_trait]
    impl ConnectMediaConnector for MockMediaConnector {
        async fn connect(
            &self,
            _connection: &ConnectionData,
            _options: ConnectMediaConnectOptions,
        ) -> crate::errors::Result<Arc<dyn ConnectMediaSession>> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(ConnectError::Signaling(
                    "mock connector secret detail".into(),
                ));
            }
            let session: Arc<dyn ConnectMediaSession> = self.session.clone();
            Ok(session)
        }
    }

    struct StopRecordingStarter {
        signaling_url: String,
        stopped: mpsc::UnboundedSender<StopContactRequest>,
    }

    struct ScriptedStopStarter {
        attempts: AtomicUsize,
        transient_failures: usize,
        permanent_failure: bool,
    }

    struct RecoveringStopStarter {
        attempts: AtomicUsize,
        transient_failures_remaining: AtomicUsize,
    }

    #[derive(Default)]
    struct CountingStarter {
        starts: AtomicUsize,
        stops: AtomicUsize,
    }

    #[derive(Default)]
    struct SuccessfulCountingStarter {
        starts: AtomicUsize,
        stops: AtomicUsize,
        start_delay: Duration,
    }

    fn valid_connection_data() -> ConnectionData {
        ConnectionData {
            contact_id: "generic-contact".into(),
            participant_id: "participant".into(),
            participant_token: "participant-token".into(),
            meeting_id: "meeting".into(),
            media_region: "local".into(),
            attendee_id: "attendee".into(),
            join_token: "join-token".into(),
            media_placement: crate::control::MediaPlacement {
                signaling_url: "wss://signaling.example.test/control".into(),
                audio_host_url: "audio.example.test".into(),
                ..Default::default()
            },
        }
    }

    #[async_trait]
    impl ConnectContactStarter for SuccessfulCountingStarter {
        async fn start_webrtc_contact(
            &self,
            _request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            if !self.start_delay.is_zero() {
                tokio::time::sleep(self.start_delay).await;
            }
            Ok(valid_connection_data())
        }

        async fn stop_contact(&self, _request: StopContactRequest) -> crate::errors::Result<()> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct BlockingStartStarter {
        starts: AtomicUsize,
    }

    #[async_trait]
    impl ConnectContactStarter for BlockingStartStarter {
        async fn start_webrtc_contact(
            &self,
            _request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            pending::<crate::errors::Result<ConnectionData>>().await
        }
    }

    struct GatedStopStarter {
        starts: AtomicUsize,
        stops: AtomicUsize,
        stop_entered: Arc<Semaphore>,
        stop_release: Arc<Semaphore>,
        stop_completed: Arc<Semaphore>,
    }

    struct RecordingRecoveryStarter {
        start_requests: SyncMutex<Vec<StartContactRequest>>,
        stop_requests: SyncMutex<Vec<StopContactRequest>>,
        transient_start_failures: AtomicUsize,
        connection: ConnectionData,
    }

    struct GatedRecoveryStarter {
        start_requests: SyncMutex<Vec<StartContactRequest>>,
        stop_requests: SyncMutex<Vec<StopContactRequest>>,
        start_entered: Arc<Semaphore>,
        start_release: Arc<Semaphore>,
        stop_completed: Arc<Semaphore>,
    }

    #[async_trait]
    impl ConnectContactStarter for GatedStopStarter {
        async fn start_webrtc_contact(
            &self,
            _request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Ok(valid_connection_data())
        }

        async fn stop_contact(&self, _request: StopContactRequest) -> crate::errors::Result<()> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            self.stop_entered.add_permits(1);
            self.stop_release
                .acquire()
                .await
                .expect("test stop-release semaphore stays open")
                .forget();
            self.stop_completed.add_permits(1);
            Ok(())
        }
    }

    #[async_trait]
    impl ConnectContactStarter for RecordingRecoveryStarter {
        async fn start_webrtc_contact(
            &self,
            request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            self.start_requests.lock().push(request);
            if self
                .transient_start_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(ConnectError::TransientControl(
                    "ambiguous test response".into(),
                ));
            }
            Ok(self.connection.clone())
        }

        async fn stop_contact(&self, request: StopContactRequest) -> crate::errors::Result<()> {
            self.stop_requests.lock().push(request);
            Ok(())
        }
    }

    #[async_trait]
    impl ConnectContactStarter for GatedRecoveryStarter {
        async fn start_webrtc_contact(
            &self,
            request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            self.start_requests.lock().push(request);
            self.start_entered.add_permits(1);
            self.start_release
                .acquire()
                .await
                .expect("test start-release semaphore stays open")
                .forget();
            Ok(valid_connection_data())
        }

        async fn stop_contact(&self, request: StopContactRequest) -> crate::errors::Result<()> {
            self.stop_requests.lock().push(request);
            self.stop_completed.add_permits(1);
            Ok(())
        }
    }

    #[async_trait]
    impl ConnectContactStarter for CountingStarter {
        async fn start_webrtc_contact(
            &self,
            _request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            self.starts.fetch_add(1, Ordering::SeqCst);
            Err(ConnectError::Control("unexpected test I/O".into()))
        }

        async fn stop_contact(&self, _request: StopContactRequest) -> crate::errors::Result<()> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn typed_context(profile_id: ConnectProfileId) -> AmazonConnectOriginateContext {
        AmazonConnectOriginateContext::new(
            profile_id,
            AmazonConnectTarget::new("instance", "flow").unwrap(),
            BTreeMap::new(),
            "display",
            None,
            ConnectClientToken::new("stable-token").unwrap(),
        )
        .unwrap()
    }

    fn recovery_context(profile_id: ConnectProfileId) -> AmazonConnectOriginateContext {
        AmazonConnectOriginateContext::new(
            profile_id,
            AmazonConnectTarget::new("recovery-instance-secret", "recovery-flow-secret").unwrap(),
            BTreeMap::from([("context_key".into(), "context-value-secret".into())]),
            "recovery-display-secret",
            Some("recovery-description-secret".into()),
            ConnectClientToken::new("recovery-stable-token-secret").unwrap(),
        )
        .unwrap()
    }

    fn assert_same_start_request(left: &StartContactRequest, right: &StartContactRequest) {
        assert_eq!(left.instance_id, right.instance_id);
        assert_eq!(left.contact_flow_id, right.contact_flow_id);
        assert_eq!(left.display_name, right.display_name);
        assert_eq!(left.attributes, right.attributes);
        assert_eq!(left.description, right.description);
        assert_eq!(left.client_token, right.client_token);
    }

    fn generic_request() -> OriginateRequest {
        OriginateRequest::new(
            SessionId::new(),
            ParticipantId::new(),
            "typed-context-owned-target",
            Direction::Outbound,
            CapabilityDescriptor::default(),
        )
        .with_transport(Transport::AmazonConnect)
    }

    #[tokio::test]
    async fn outbound_only_adapter_registers_with_fail_closed_inbound_gate() {
        let orchestrator = Orchestrator::new(CoreConfig::default());
        let _admission = orchestrator
            .install_inbound_admission_gate(4, Duration::from_secs(1))
            .unwrap();
        let _operational = orchestrator.install_operational_event_stream(16).unwrap();
        let starter: Arc<dyn ConnectContactStarter> = Arc::new(CountingStarter::default());
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("unused-instance", "unused-flow"),
            starter,
        );

        orchestrator
            .register(adapter as Arc<dyn ConnectionAdapter>)
            .expect("outbound-only Amazon adapter satisfies the admission lifecycle contract");
    }

    #[tokio::test]
    async fn isolated_fork_shares_dependencies_but_not_runtime_or_contact_authority() {
        let starter = Arc::new(SuccessfulCountingStarter::default());
        let profile_starter = Arc::new(RecoveringStopStarter {
            attempts: AtomicUsize::new(0),
            transient_failures_remaining: AtomicUsize::new(STOP_CONTACT_ATTEMPTS),
        });
        let profile_id = ConnectProfileId::new("tenant-profile").unwrap();
        let media = MockMediaSession::new();
        let connector = Arc::new(MockMediaConnector {
            session: media,
            connects: AtomicUsize::new(0),
            fail: false,
        });
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector.clone();
        let mut builder = AmazonConnectAdapter::builder(
            ConnectConfig::new("shared-instance-secret", "shared-flow-secret"),
            starter_trait,
        );
        builder
            .register_profile(profile_id.clone(), profile_starter.clone())
            .unwrap()
            .set_media_connector(connector_trait);
        let mut webrtc = WebRtcConfig::default();
        webrtc.udp_bind = "127.0.0.1:45678".into();
        webrtc.max_concurrent_sessions = 7;
        let source = builder.build().with_webrtc_config(webrtc);
        let mut source_events = source.subscribe_events();
        let source_sink: Arc<dyn AdapterLifecycleSink> = Arc::new(NoopLifecycleSink);
        source
            .install_lifecycle_sink(source_sink)
            .expect("source lifecycle owner installs");

        let source_connection = source
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect("prepare source route")
            .connection
            .id;
        source
            .activate_outbound(source_connection.clone())
            .await
            .expect("activate source route");
        assert!(matches!(
            source_events.recv().await,
            Some(AdapterEvent::Connected { connection_id }) if connection_id == source_connection
        ));
        let pending_reference = ExternalConnectionReference::new(
            AMAZON_CONNECT_CONTACT_REFERENCE_KIND,
            "source-pending-contact",
        )
        .unwrap();
        assert!(source
            .stop_persisted_contact(&profile_id, "source-pending-instance", &pending_reference,)
            .await
            .is_err());
        assert_eq!(source.pending_cleanup_count(), 1);
        assert!(source.begin_drain());

        // Fork after runtime ownership exists: only immutable dependencies and
        // policy may cross this boundary.
        let fork = source.fork_isolated();
        let mut fork_events = fork.subscribe_events();
        let fork_sink: Arc<dyn AdapterLifecycleSink> = Arc::new(NoopLifecycleSink);
        fork.install_lifecycle_sink(fork_sink)
            .expect("fork has independent lifecycle ownership");

        assert!(Arc::ptr_eq(&source.starter, &fork.starter));
        assert!(Arc::ptr_eq(&source.media_connector, &fork.media_connector));
        let source_profile = source.resolve_profile(&profile_id).unwrap();
        let fork_profile = fork.resolve_profile(&profile_id).unwrap();
        assert!(Arc::ptr_eq(&source_profile, &fork_profile));
        assert_eq!(source.configured_profile_count(), 2);
        assert_eq!(fork.configured_profile_count(), 2);
        assert_eq!(fork.config.instance_id, "shared-instance-secret");
        assert_eq!(fork.config.contact_flow_id, "shared-flow-secret");
        assert_eq!(fork.webrtc.udp_bind, "127.0.0.1:45678");
        assert_eq!(fork.webrtc.max_concurrent_sessions, 7);
        assert!(!fork.webrtc.trickle_ice);

        assert!(!Arc::ptr_eq(&source.routes, &fork.routes));
        assert!(!Arc::ptr_eq(&source.prepared_routes, &fork.prepared_routes));
        assert!(!source.events_tx.same_channel(&fork.events_tx));
        assert!(!Arc::ptr_eq(
            &source.contacts_started,
            &fork.contacts_started
        ));
        assert!(!Arc::ptr_eq(&source.failures, &fork.failures));
        assert!(!Arc::ptr_eq(&source.cleanup_slots, &fork.cleanup_slots));
        assert!(!Arc::ptr_eq(
            &source.pending_cleanups,
            &fork.pending_cleanups
        ));
        assert!(!Arc::ptr_eq(&source.draining, &fork.draining));
        assert!(!Arc::ptr_eq(
            &source.setups_in_flight,
            &fork.setups_in_flight
        ));
        assert!(source.is_draining());
        assert!(!fork.is_draining());
        assert!(source.is_connection_live(&source_connection));
        assert!(!fork.is_connection_live(&source_connection));
        assert_eq!(source.metrics().contacts_started, 1);
        assert_eq!(source.metrics().active_sessions, 1);
        assert_eq!(fork.metrics().contacts_started, 0);
        assert_eq!(fork.metrics().active_sessions, 0);
        assert_eq!(source.pending_cleanup_count(), 1);
        assert_eq!(fork.pending_cleanup_count(), 0);
        assert!(!fork
            .retry_pending_cleanup_for("source-pending-instance", "source-pending-contact")
            .await
            .expect("fork has no source cleanup authority"));
        assert_eq!(source.pending_cleanup_count(), 1);
        assert!(source
            .retry_pending_cleanup_for("source-pending-instance", "source-pending-contact")
            .await
            .expect("source retains exact pending cleanup authority"));
        assert_eq!(source.pending_cleanup_count(), 0);
        assert_eq!(profile_starter.attempts.load(Ordering::SeqCst), 4);
        assert!(matches!(
            fork_events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        fork.end(source_connection.clone(), EndReason::Normal)
            .await
            .expect("fork cannot mutate an unowned source route");
        assert!(source.is_connection_live(&source_connection));
        assert_eq!(starter.stops.load(Ordering::SeqCst), 0);
        assert!(matches!(
            fork_events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        source
            .end(source_connection.clone(), EndReason::Normal)
            .await
            .expect("source retains exact cleanup authority");
        assert!(matches!(
            source_events.recv().await,
            Some(AdapterEvent::Ended { connection_id, reason: EndReason::Normal })
                if connection_id == source_connection
        ));
        assert_eq!(starter.stops.load(Ordering::SeqCst), 1);

        let fork_connection = fork
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect("source drain does not affect fork admission")
            .connection
            .id;
        fork.activate_outbound(fork_connection.clone())
            .await
            .expect("fork activates independently");
        assert!(matches!(
            fork_events.recv().await,
            Some(AdapterEvent::Connected { connection_id }) if connection_id == fork_connection
        ));
        assert!(matches!(
            source_events.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        fork.end(fork_connection.clone(), EndReason::Normal)
            .await
            .expect("fork cleans only its own contact");
        assert!(matches!(
            fork_events.recv().await,
            Some(AdapterEvent::Ended { connection_id, reason: EndReason::Normal })
                if connection_id == fork_connection
        ));
        assert_eq!(source.metrics().contacts_started, 1);
        assert_eq!(fork.metrics().contacts_started, 1);
        assert_eq!(source.metrics().active_sessions, 0);
        assert_eq!(fork.metrics().active_sessions, 0);
        assert_eq!(starter.starts.load(Ordering::SeqCst), 2);
        assert_eq!(starter.stops.load(Ordering::SeqCst), 2);
        assert_eq!(connector.connects.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn control_only_recovery_reuses_exact_request_and_owns_exact_stop() {
        let starter = Arc::new(RecordingRecoveryStarter {
            start_requests: SyncMutex::new(Vec::new()),
            stop_requests: SyncMutex::new(Vec::new()),
            transient_start_failures: AtomicUsize::new(1),
            connection: valid_connection_data(),
        });
        let media = MockMediaSession::new();
        let connector = Arc::new(MockMediaConnector {
            session: media,
            connects: AtomicUsize::new(0),
            fail: false,
        });
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector.clone();
        let adapter = AmazonConnectAdapter::builder(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter_trait,
        )
        .with_media_connector(connector_trait)
        .build();

        let context = recovery_context(ConnectProfileId::default());
        let mut recovered = adapter
            .recover_contact(&context)
            .await
            .expect("stable-token reconciliation recovers the contact");

        {
            let requests = starter.start_requests.lock();
            assert_eq!(requests.len(), 2);
            assert_same_start_request(&requests[0], &requests[1]);
            assert_eq!(
                requests[0].client_token.as_deref(),
                Some("recovery-stable-token-secret")
            );
            assert_eq!(requests[0].instance_id, "recovery-instance-secret");
            assert_eq!(requests[0].contact_flow_id, "recovery-flow-secret");
        }

        assert_eq!(connector.connects.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.metrics().contacts_started, 1);
        assert_eq!(recovered.profile_id(), &ConnectProfileId::default());
        assert_eq!(recovered.instance_id(), "recovery-instance-secret");
        assert_eq!(
            recovered.external_reference().kind(),
            AMAZON_CONNECT_CONTACT_REFERENCE_KIND
        );
        assert_eq!(
            recovered.external_reference().expose_secret(),
            "generic-contact"
        );
        let diagnostic = format!("{recovered:?}");
        for secret in [
            "recovery-instance-secret",
            "generic-contact",
            "recovery-stable-token-secret",
        ] {
            assert!(!diagnostic.contains(secret), "leaked {secret}");
        }

        recovered.stop().await.expect("exact StopContact succeeds");
        recovered
            .stop()
            .await
            .expect("repeated local stop is a no-op");
        drop(recovered);
        let stops = starter.stop_requests.lock();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].instance_id, "recovery-instance-secret");
        assert_eq!(stops[0].contact_id, "generic-contact");
        assert_eq!(adapter.pending_cleanup_count(), 0);
    }

    #[tokio::test]
    async fn transient_recovery_preserves_caller_stable_token_for_later_reconciliation() {
        let starter = Arc::new(RecordingRecoveryStarter {
            start_requests: SyncMutex::new(Vec::new()),
            stop_requests: SyncMutex::new(Vec::new()),
            transient_start_failures: AtomicUsize::new(3),
            connection: valid_connection_data(),
        });
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter.clone(),
        );
        let context = recovery_context(ConnectProfileId::default());

        let first = adapter
            .recover_contact(&context)
            .await
            .expect_err("bounded reconciliation remains transient after three ambiguous replies");
        assert_eq!(first.classification(), ConnectErrorClass::ControlTransient);
        assert_eq!(
            context.start_request().client_token.as_deref(),
            Some("recovery-stable-token-secret")
        );
        assert!(starter.stop_requests.lock().is_empty());

        starter.transient_start_failures.store(0, Ordering::SeqCst);
        let mut recovered = adapter
            .recover_contact(&context)
            .await
            .expect("the caller can reconcile later with the retained context");
        recovered.stop().await.expect("reconciled contact stops");
        let requests = starter.start_requests.lock();
        assert_eq!(requests.len(), 4);
        for request in requests.iter().skip(1) {
            assert_same_start_request(&requests[0], request);
        }
        assert_eq!(starter.stop_requests.lock().len(), 1);
    }

    #[tokio::test]
    async fn cancelled_recovery_caller_cannot_orphan_a_started_contact() {
        let starter = Arc::new(GatedRecoveryStarter {
            start_requests: SyncMutex::new(Vec::new()),
            stop_requests: SyncMutex::new(Vec::new()),
            start_entered: Arc::new(Semaphore::new(0)),
            start_release: Arc::new(Semaphore::new(0)),
            stop_completed: Arc::new(Semaphore::new(0)),
        });
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter.clone(),
        );
        let recovery = tokio::spawn({
            let adapter = Arc::clone(&adapter);
            async move {
                let context = recovery_context(ConnectProfileId::default());
                adapter.recover_contact(&context).await
            }
        });
        starter
            .start_entered
            .acquire()
            .await
            .expect("recovery entered StartWebRTCContact")
            .forget();

        recovery.abort();
        let _ = recovery.await;
        starter.start_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(1), async {
            starter
                .stop_completed
                .acquire()
                .await
                .expect("recovery stop semaphore stays open")
                .forget();
            while adapter.setups_in_flight.load(Ordering::Acquire) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("independent recovery owner compensates after caller cancellation");

        assert_eq!(starter.start_requests.lock().len(), 1);
        let stops = starter.stop_requests.lock();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].instance_id, "recovery-instance-secret");
        assert_eq!(stops[0].contact_id, "generic-contact");
        assert_eq!(adapter.pending_cleanup_count(), 0);
    }

    #[tokio::test]
    async fn drain_reports_but_does_not_abort_ambiguous_recovery() {
        let starter = Arc::new(GatedRecoveryStarter {
            start_requests: SyncMutex::new(Vec::new()),
            stop_requests: SyncMutex::new(Vec::new()),
            start_entered: Arc::new(Semaphore::new(0)),
            start_release: Arc::new(Semaphore::new(0)),
            stop_completed: Arc::new(Semaphore::new(0)),
        });
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter.clone(),
        );
        let recovery = tokio::spawn({
            let adapter = Arc::clone(&adapter);
            async move {
                let context = recovery_context(ConnectProfileId::default());
                adapter.recover_contact(&context).await
            }
        });
        starter
            .start_entered
            .acquire()
            .await
            .expect("recovery entered StartWebRTCContact")
            .forget();

        let report = adapter
            .drain_until(
                Instant::now()
                    .checked_add(Duration::from_millis(25))
                    .expect("deadline"),
            )
            .await;
        assert_eq!(report.in_flight_setups, 1);
        assert!(!report.is_complete());
        assert_eq!(starter.stop_requests.lock().len(), 0);

        starter.start_release.add_permits(1);
        let mut recovered = tokio::time::timeout(Duration::from_secs(1), recovery)
            .await
            .expect("recovery survives drain deadline")
            .expect("recovery task is not aborted")
            .expect("recovery succeeds");
        recovered.stop().await.expect("recovered contact stops");
        let final_report = adapter
            .drain_until(
                Instant::now()
                    .checked_add(Duration::from_secs(1))
                    .expect("deadline"),
            )
            .await;
        assert!(
            final_report.is_complete(),
            "unexpected final report: {final_report:?}"
        );
        assert_eq!(starter.stop_requests.lock().len(), 1);
    }

    #[tokio::test]
    async fn persisted_stop_requires_exact_kind_profile_and_valid_instance() {
        let starter = Arc::new(RecordingRecoveryStarter {
            start_requests: SyncMutex::new(Vec::new()),
            stop_requests: SyncMutex::new(Vec::new()),
            transient_start_failures: AtomicUsize::new(0),
            connection: valid_connection_data(),
        });
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter.clone(),
        );
        let correct = ExternalConnectionReference::new(
            AMAZON_CONNECT_CONTACT_REFERENCE_KIND,
            "persisted-contact-secret",
        )
        .unwrap();
        let wrong_kind =
            ExternalConnectionReference::new("another-adapter.contact-id", "contact").unwrap();

        assert!(adapter
            .stop_persisted_contact(
                &ConnectProfileId::default(),
                "persisted-instance-secret",
                &wrong_kind,
            )
            .await
            .is_err());
        assert!(adapter
            .stop_persisted_contact(
                &ConnectProfileId::new("not-configured").unwrap(),
                "persisted-instance-secret",
                &correct,
            )
            .await
            .is_err());
        let invalid = adapter
            .stop_persisted_contact(&ConnectProfileId::default(), "bad\ninstance", &correct)
            .await
            .unwrap_err();
        assert!(!format!("{invalid:?}").contains("bad\ninstance"));
        assert!(starter.stop_requests.lock().is_empty());

        adapter
            .stop_persisted_contact(
                &ConnectProfileId::default(),
                "persisted-instance-secret",
                &correct,
            )
            .await
            .expect("exact persisted StopContact succeeds");
        assert!(starter.start_requests.lock().is_empty());
        let stops = starter.stop_requests.lock();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].instance_id, "persisted-instance-secret");
        assert_eq!(stops[0].contact_id, "persisted-contact-secret");
    }

    #[tokio::test]
    async fn cancelled_persisted_stop_caller_does_not_cancel_exact_cleanup() {
        let starter = Arc::new(GatedStopStarter {
            starts: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            stop_entered: Arc::new(Semaphore::new(0)),
            stop_release: Arc::new(Semaphore::new(0)),
            stop_completed: Arc::new(Semaphore::new(0)),
        });
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter.clone(),
        );
        let cleanup = tokio::spawn({
            let adapter = Arc::clone(&adapter);
            async move {
                let profile_id = ConnectProfileId::default();
                let reference = ExternalConnectionReference::new(
                    AMAZON_CONNECT_CONTACT_REFERENCE_KIND,
                    "persisted-contact-secret",
                )
                .unwrap();
                adapter
                    .stop_persisted_contact(&profile_id, "persisted-instance-secret", &reference)
                    .await
            }
        });
        starter
            .stop_entered
            .acquire()
            .await
            .expect("persisted StopContact entered")
            .forget();

        cleanup.abort();
        let _ = cleanup.await;
        starter.stop_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(1), async {
            starter
                .stop_completed
                .acquire()
                .await
                .expect("persisted stop-completion semaphore stays open")
                .forget();
            while adapter.setups_in_flight.load(Ordering::Acquire) != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached persisted cleanup completes after caller cancellation");
        assert_eq!(starter.starts.load(Ordering::SeqCst), 0);
        assert_eq!(starter.stops.load(Ordering::SeqCst), 1);
        assert_eq!(adapter.pending_cleanup_count(), 0);
    }

    #[tokio::test]
    async fn failed_persisted_stop_retains_exact_retry_authority() {
        let starter = Arc::new(RecoveringStopStarter {
            attempts: AtomicUsize::new(0),
            transient_failures_remaining: AtomicUsize::new(STOP_CONTACT_ATTEMPTS),
        });
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter.clone(),
        );
        let profile_id = ConnectProfileId::default();
        let reference = ExternalConnectionReference::new(
            AMAZON_CONNECT_CONTACT_REFERENCE_KIND,
            "persisted-contact-secret",
        )
        .unwrap();

        assert!(adapter
            .stop_persisted_contact(&profile_id, "persisted-instance-secret", &reference,)
            .await
            .is_err());
        assert_eq!(starter.attempts.load(Ordering::SeqCst), 3);
        assert_eq!(adapter.pending_cleanup_count(), 1);

        assert!(adapter
            .retry_pending_cleanup_for("persisted-instance-secret", "persisted-contact-secret")
            .await
            .expect("retained exact cleanup succeeds later"));
        assert_eq!(starter.attempts.load(Ordering::SeqCst), 4);
        assert_eq!(adapter.pending_cleanup_count(), 0);
    }

    #[tokio::test]
    async fn malformed_recovery_response_is_compensated_without_media() {
        let mut malformed_connection = valid_connection_data();
        malformed_connection.contact_id = "malformed\ncontact-secret".into();
        let starter = Arc::new(RecordingRecoveryStarter {
            start_requests: SyncMutex::new(Vec::new()),
            stop_requests: SyncMutex::new(Vec::new()),
            transient_start_failures: AtomicUsize::new(0),
            connection: malformed_connection,
        });
        let media = MockMediaSession::new();
        let connector = Arc::new(MockMediaConnector {
            session: media,
            connects: AtomicUsize::new(0),
            fail: false,
        });
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector.clone();
        let adapter = AmazonConnectAdapter::builder(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter_trait,
        )
        .with_media_connector(connector_trait)
        .build();

        let context = recovery_context(ConnectProfileId::default());
        let error = adapter
            .recover_contact(&context)
            .await
            .expect_err("malformed contact references are never published");
        assert_eq!(error.classification(), ConnectErrorClass::InvalidResponse);
        assert!(!format!("{error:?}").contains("malformed\ncontact-secret"));
        assert_eq!(connector.connects.load(Ordering::SeqCst), 0);
        let stops = starter.stop_requests.lock();
        assert_eq!(stops.len(), 1);
        assert_eq!(stops[0].instance_id, "recovery-instance-secret");
        assert_eq!(stops[0].contact_id, "malformed\ncontact-secret");
        assert_eq!(adapter.pending_cleanup_count(), 0);
    }

    #[tokio::test]
    async fn generic_originate_requires_exact_context_before_any_io() {
        let starter = Arc::new(CountingStarter::default());
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter.clone(),
        );

        let missing = adapter.originate(generic_request()).await.unwrap_err();
        assert!(matches!(
            missing,
            RvoipError::AdmissionRejected("Amazon Connect originate context is required")
        ));

        let wrong = adapter
            .originate(generic_request().with_context("wrong-context".to_owned()))
            .await
            .unwrap_err();
        assert!(matches!(
            wrong,
            RvoipError::AdmissionRejected("Amazon Connect originate context type mismatch")
        ));

        let staged = adapter
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect("typed generic route is prepared locally");
        assert_eq!(staged.connection.state, ConnectionState::Connecting);
        assert!(adapter.is_connection_live(&staged.connection.id));
        assert_eq!(starter.starts.load(Ordering::SeqCst), 0);
        assert_eq!(starter.stops.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.metrics().contacts_started, 0);
        assert_eq!(adapter.metrics().active_sessions, 0);
        adapter
            .end(staged.connection.id.clone(), EndReason::Normal)
            .await
            .expect("dormant route ends without remote I/O");
        assert!(!adapter.is_connection_live(&staged.connection.id));
        assert_eq!(starter.starts.load(Ordering::SeqCst), 0);
        assert_eq!(starter.stops.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn profile_resolver_is_exact_isolated_and_io_dormant() {
        let default = Arc::new(CountingStarter::default());
        let tenant_a = Arc::new(CountingStarter::default());
        let tenant_b = Arc::new(CountingStarter::default());
        let profile_a = ConnectProfileId::new("tenant-a").unwrap();
        let profile_b = ConnectProfileId::new("tenant-b").unwrap();
        let mut builder = AmazonConnectAdapter::builder(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            default.clone(),
        );
        builder
            .register_profile(profile_a.clone(), tenant_a.clone())
            .unwrap()
            .register_profile(profile_b.clone(), tenant_b.clone())
            .unwrap();
        let duplicate = builder
            .register_profile(profile_a.clone(), Arc::new(CountingStarter::default()))
            .unwrap_err();
        assert_eq!(duplicate, ConnectProfileResolverError::DuplicateProfile);
        let adapter = builder.build();
        assert_eq!(adapter.configured_profile_count(), 3);

        let resolved_a = adapter.resolve_profile(&profile_a).unwrap();
        let resolved_b = adapter.resolve_profile(&profile_b).unwrap();
        let tenant_a_trait: Arc<dyn ConnectContactStarter> = tenant_a.clone();
        let tenant_b_trait: Arc<dyn ConnectContactStarter> = tenant_b.clone();
        assert!(Arc::ptr_eq(&resolved_a, &tenant_a_trait));
        assert!(Arc::ptr_eq(&resolved_b, &tenant_b_trait));
        assert!(!Arc::ptr_eq(&resolved_a, &resolved_b));

        for profile in [profile_a, profile_b] {
            let handle = adapter
                .originate(generic_request().with_context(typed_context(profile)))
                .await
                .expect("configured profile prepares a dormant route");
            adapter
                .end(handle.connection.id, EndReason::Normal)
                .await
                .expect("dormant profile route ends without remote I/O");
        }
        let unknown = adapter
            .originate(generic_request().with_context(typed_context(
                ConnectProfileId::new("not-configured").unwrap(),
            )))
            .await
            .unwrap_err();
        assert!(matches!(
            unknown,
            RvoipError::AdmissionRejected("Amazon Connect profile is not configured")
        ));
        for starter in [&default, &tenant_a, &tenant_b] {
            assert_eq!(starter.starts.load(Ordering::SeqCst), 0);
            assert_eq!(starter.stops.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn generic_activation_is_single_flight_and_local_end_releases_exact_route() {
        let starter = Arc::new(SuccessfulCountingStarter {
            start_delay: Duration::from_millis(20),
            ..Default::default()
        });
        let media = MockMediaSession::new();
        let connector = Arc::new(MockMediaConnector {
            session: media.clone(),
            connects: AtomicUsize::new(0),
            fail: false,
        });
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector.clone();
        let adapter = AmazonConnectAdapter::builder(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter_trait,
        )
        .with_media_connector(connector_trait)
        .build();
        let mut events = adapter.subscribe_events();
        let handle = adapter
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect("prepare generic route");
        let conn = handle.connection.id;

        let (first, second) = tokio::join!(
            adapter.activate_outbound_with_receipt(conn.clone()),
            adapter.activate_outbound_with_receipt(conn.clone())
        );
        let first = first.expect("first activation succeeds");
        let second = second.expect("second activation observes the same result");
        assert_eq!(first, second);
        let reference = first
            .external_references()
            .first()
            .expect("contact reference");
        assert_eq!(reference.kind(), AMAZON_CONNECT_CONTACT_REFERENCE_KIND);
        assert_eq!(reference.expose_secret(), "generic-contact");
        assert_eq!(starter.starts.load(Ordering::SeqCst), 1);
        assert_eq!(connector.connects.load(Ordering::SeqCst), 1);
        assert!(adapter.is_connection_live(&conn));
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { connection_id }) if connection_id == conn
        ));

        adapter
            .end(conn.clone(), EndReason::Normal)
            .await
            .expect("local end closes media and contact");
        assert!(!adapter.is_connection_live(&conn));
        assert_eq!(starter.stops.load(Ordering::SeqCst), 1);
        assert_eq!(media.calls.closes.load(Ordering::SeqCst), 1);
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Ended { connection_id, reason: EndReason::Normal })
                if connection_id == conn
        ));
    }

    #[tokio::test]
    async fn generic_remote_end_retires_route_and_stops_contact_once() {
        let starter = Arc::new(SuccessfulCountingStarter::default());
        let media = MockMediaSession::new();
        let connector = Arc::new(MockMediaConnector {
            session: media.clone(),
            connects: AtomicUsize::new(0),
            fail: false,
        });
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector;
        let adapter = AmazonConnectAdapter::builder(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter_trait,
        )
        .with_media_connector(connector_trait)
        .build();
        let mut events = adapter.subscribe_events();
        let handle = adapter
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect("prepare generic route");
        let conn = handle.connection.id;
        adapter
            .activate_outbound(conn.clone())
            .await
            .expect("activate generic route");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { .. })
        ));

        media.terminal(ConnectMediaTerminalCause::RemoteEnded);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), events.recv()).await,
            Ok(Some(AdapterEvent::Ended { connection_id, reason: EndReason::Normal }))
                if connection_id == conn
        ));
        assert!(!adapter.is_connection_live(&conn));
        assert_eq!(starter.stops.load(Ordering::SeqCst), 1);
        adapter
            .end(conn, EndReason::Normal)
            .await
            .expect("duplicate local end is idempotent");
        assert_eq!(starter.stops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn generic_start_timeout_uses_bounded_stable_token_reconciliation() {
        let starter = Arc::new(BlockingStartStarter {
            starts: AtomicUsize::new(0),
        });
        let mut config = ConnectConfig::new("legacy-instance", "legacy-flow");
        config.signaling_timeout = Duration::from_millis(10);
        let adapter = AmazonConnectAdapter::new(config, starter.clone());
        let mut events = adapter.subscribe_events();
        let handle = adapter
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect("prepare generic route");
        let conn = handle.connection.id;

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            adapter.activate_outbound(conn.clone()),
        )
        .await
        .expect("activation has a hard reconciliation bound")
        .expect_err("all Start attempts time out");
        assert!(matches!(error, RvoipError::Adapter(_)));
        assert_eq!(starter.starts.load(Ordering::SeqCst), 3);
        assert!(!adapter.is_connection_live(&conn));
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Failed { connection_id, detail })
                if connection_id == conn
                    && detail == "Amazon Connect outbound activation failed"
        ));
    }

    #[tokio::test]
    async fn drain_rejects_new_generic_and_legacy_contacts_before_io() {
        let starter = Arc::new(CountingStarter::default());
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter.clone(),
        );
        assert!(adapter.begin_drain());
        assert!(!adapter.begin_drain());
        assert!(adapter.is_draining());

        let generic = adapter
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect_err("generic prepare is rejected during drain");
        assert!(matches!(
            generic,
            RvoipError::AdmissionRejected("Amazon Connect adapter is draining")
        ));
        assert!(adapter
            .originate_contact(BTreeMap::new(), None, None)
            .await
            .is_err());
        assert_eq!(starter.starts.load(Ordering::SeqCst), 0);
        assert_eq!(starter.stops.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn drain_retires_prepared_and_active_routes_exactly_once() {
        let starter = Arc::new(SuccessfulCountingStarter::default());
        let media = MockMediaSession::new();
        let connector = Arc::new(MockMediaConnector {
            session: media.clone(),
            connects: AtomicUsize::new(0),
            fail: false,
        });
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector;
        let adapter = AmazonConnectAdapter::builder(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter_trait,
        )
        .with_media_connector(connector_trait)
        .build();
        let active = adapter
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect("prepare active route")
            .connection
            .id;
        adapter
            .activate_outbound(active.clone())
            .await
            .expect("activate first route");
        let dormant = adapter
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect("prepare dormant route")
            .connection
            .id;

        let report = adapter
            .drain_until(
                Instant::now()
                    .checked_add(Duration::from_secs(1))
                    .expect("deadline"),
            )
            .await;
        assert_eq!(report.attempted_routes, 2);
        assert_eq!(report.completed_routes, 2);
        assert!(report.is_complete(), "unexpected drain report: {report:?}");
        assert!(!adapter.is_connection_live(&active));
        assert!(!adapter.is_connection_live(&dormant));
        assert_eq!(starter.starts.load(Ordering::SeqCst), 1);
        assert_eq!(starter.stops.load(Ordering::SeqCst), 1);
        assert_eq!(media.calls.closes.load(Ordering::SeqCst), 1);

        let repeated = adapter
            .drain_until(
                Instant::now()
                    .checked_add(Duration::from_secs(1))
                    .expect("deadline"),
            )
            .await;
        assert_eq!(repeated.attempted_routes, 0);
        assert!(repeated.is_complete());
    }

    #[tokio::test]
    async fn drain_deadline_detaches_but_does_not_abort_owned_stop() {
        let starter = Arc::new(GatedStopStarter {
            starts: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
            stop_entered: Arc::new(Semaphore::new(0)),
            stop_release: Arc::new(Semaphore::new(0)),
            stop_completed: Arc::new(Semaphore::new(0)),
        });
        let media = MockMediaSession::new();
        let connector = Arc::new(MockMediaConnector {
            session: media.clone(),
            connects: AtomicUsize::new(0),
            fail: false,
        });
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector;
        let adapter = AmazonConnectAdapter::builder(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            starter_trait,
        )
        .with_media_connector(connector_trait)
        .build();
        let connection_id = adapter
            .originate(generic_request().with_context(typed_context(ConnectProfileId::default())))
            .await
            .expect("prepare route")
            .connection
            .id;
        adapter
            .activate_outbound(connection_id.clone())
            .await
            .expect("activate route");

        let report = adapter
            .drain_until(
                Instant::now()
                    .checked_add(Duration::from_millis(25))
                    .expect("deadline"),
            )
            .await;
        assert_eq!(report.attempted_routes, 1);
        assert_eq!(report.detached_cleanups, 1);
        assert!(!report.is_complete());
        assert!(!adapter.is_connection_live(&connection_id));
        starter
            .stop_entered
            .acquire()
            .await
            .expect("StopContact was entered")
            .forget();
        starter.stop_release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(1), async {
            starter
                .stop_completed
                .acquire()
                .await
                .expect("StopContact completion semaphore stays open")
                .forget();
        })
        .await
        .expect("detached exact cleanup continues after the drain deadline");
        assert_eq!(starter.stops.load(Ordering::SeqCst), 1);
        assert_eq!(media.calls.closes.load(Ordering::SeqCst), 1);
    }

    #[async_trait]
    impl ConnectContactStarter for RecoveringStopStarter {
        async fn start_webrtc_contact(
            &self,
            _request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            Err(ConnectError::Control("unused".into()))
        }

        async fn stop_contact(&self, _request: StopContactRequest) -> crate::errors::Result<()> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            let remaining = self.transient_failures_remaining.fetch_update(
                Ordering::SeqCst,
                Ordering::SeqCst,
                |remaining| remaining.checked_sub(1),
            );
            if remaining.is_ok() {
                Err(ConnectError::TransientControl("temporary outage".into()))
            } else {
                Ok(())
            }
        }
    }

    #[async_trait]
    impl ConnectContactStarter for ScriptedStopStarter {
        async fn start_webrtc_contact(
            &self,
            _request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            Err(ConnectError::Control("unused".into()))
        }

        async fn stop_contact(&self, _request: StopContactRequest) -> crate::errors::Result<()> {
            let attempt = self.attempts.fetch_add(1, Ordering::SeqCst) + 1;
            if self.permanent_failure {
                Err(ConnectError::Control("permanent stop failure".into()))
            } else if attempt <= self.transient_failures {
                Err(ConnectError::TransientControl(format!(
                    "transient stop failure {attempt}"
                )))
            } else {
                Ok(())
            }
        }
    }

    #[async_trait]
    impl ConnectContactStarter for StopRecordingStarter {
        async fn start_webrtc_contact(
            &self,
            _request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            Ok(ConnectionData {
                contact_id: "owned-contact".into(),
                participant_id: "participant".into(),
                participant_token: "participant-token".into(),
                meeting_id: "meeting".into(),
                media_region: "local".into(),
                attendee_id: "attendee".into(),
                join_token: "join-token".into(),
                media_placement: crate::control::MediaPlacement {
                    signaling_url: self.signaling_url.clone(),
                    audio_host_url: "audio.local".into(),
                    ..Default::default()
                },
            })
        }

        async fn stop_contact(&self, request: StopContactRequest) -> crate::errors::Result<()> {
            let _ = self.stopped.send(request);
            Ok(())
        }
    }

    /// Records every `StartContactRequest`, then fails so `establish` stops
    /// before the Chime signaling step (all we test is control-plane input).
    struct CapturingStarter {
        seen: SyncMutex<Vec<StartContactRequest>>,
    }

    #[async_trait]
    impl ConnectContactStarter for CapturingStarter {
        async fn start_webrtc_contact(
            &self,
            request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            self.seen.lock().push(request);
            Err(ConnectError::Control("test stops after capture".into()))
        }
    }

    fn adapter_with_capture() -> (Arc<AmazonConnectAdapter>, Arc<CapturingStarter>) {
        let starter = Arc::new(CapturingStarter {
            seen: SyncMutex::new(Vec::new()),
        });
        let mut config = ConnectConfig::new("inst-default", "flow-default");
        config.default_display_name = "config-default".into();
        let adapter = AmazonConnectAdapter::new(config, starter.clone());
        (adapter, starter)
    }

    fn route_with_mock_media(
        adapter: &Arc<AmazonConnectAdapter>,
        media: Arc<MockMediaSession>,
        starter: Arc<dyn ConnectContactStarter>,
    ) -> Route {
        Route {
            media,
            cancel: Arc::new(Notify::new()),
            stop_request: StopContactRequest {
                instance_id: "instance".into(),
                contact_id: "contact".into(),
            },
            profile_id: ConnectProfileId::default(),
            starter,
            cleanup_permit: Arc::new(
                Arc::clone(&adapter.cleanup_slots)
                    .try_acquire_owned()
                    .expect("cleanup capacity"),
            ),
            prepared: None,
            supervisor: Arc::new(AsyncMutex::new(None)),
        }
    }

    async fn install_supervised_test_route(
        adapter: &Arc<AmazonConnectAdapter>,
        conn: ConnectionId,
        route: &Route,
    ) {
        adapter.routes.insert(conn.clone(), route.clone());
        let supervisor = AmazonConnectAdapter::spawn_route_supervisor(
            adapter.activation_environment(),
            conn,
            route.clone(),
        );
        *route.supervisor.lock().await = Some(supervisor);
    }

    #[tokio::test]
    async fn contact_target_overrides_reach_start_contact_request() {
        let (adapter, starter) = adapter_with_capture();
        let target = ContactTarget {
            instance_id: Some("inst-tenant".into()),
            contact_flow_id: Some("flow-tenant".into()),
            default_display_name: Some("tenant-name".into()),
        };
        let _ = adapter
            .originate_contact_to(target, BTreeMap::new(), None, None)
            .await;

        let seen = starter.seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].instance_id, "inst-tenant");
        assert_eq!(seen[0].contact_flow_id, "flow-tenant");
        assert_eq!(seen[0].display_name, "tenant-name");
        assert!(seen[0].client_token.is_none());
    }

    #[tokio::test]
    async fn default_target_falls_back_to_config() {
        let (adapter, starter) = adapter_with_capture();
        let _ = adapter.originate_contact(BTreeMap::new(), None, None).await;

        let seen = starter.seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].instance_id, "inst-default");
        assert_eq!(seen[0].contact_flow_id, "flow-default");
        assert_eq!(seen[0].display_name, "config-default");
        assert!(seen[0].client_token.is_none());
    }

    #[tokio::test]
    async fn caller_display_name_beats_target_default() {
        let (adapter, starter) = adapter_with_capture();
        let target = ContactTarget {
            default_display_name: Some("tenant-name".into()),
            ..Default::default()
        };
        let _ = adapter
            .originate_contact_to(target, BTreeMap::new(), Some("sip:caller@x".into()), None)
            .await;

        assert_eq!(starter.seen.lock()[0].display_name, "sip:caller@x");
    }

    /// A starter whose in-flight control request reports when cancellation
    /// drops it. This is the hermetic stand-in for an AWS SDK request future.
    struct CancellationStarter {
        entered: Notify,
        dropped: SyncMutex<Option<oneshot::Sender<()>>>,
    }

    #[async_trait]
    impl ConnectContactStarter for CancellationStarter {
        async fn start_webrtc_contact(
            &self,
            _request: StartContactRequest,
        ) -> crate::errors::Result<ConnectionData> {
            struct DropSignal(Option<oneshot::Sender<()>>);

            impl Drop for DropSignal {
                fn drop(&mut self) {
                    if let Some(tx) = self.0.take() {
                        let _ = tx.send(());
                    }
                }
            }

            let _drop_signal = DropSignal(self.dropped.lock().take());
            self.entered.notify_one();
            pending().await
        }
    }

    #[tokio::test]
    async fn cancelling_originate_drops_inflight_control_request() {
        let (dropped_tx, dropped_rx) = oneshot::channel();
        let starter = Arc::new(CancellationStarter {
            entered: Notify::new(),
            dropped: SyncMutex::new(Some(dropped_tx)),
        });
        let adapter =
            AmazonConnectAdapter::new(ConnectConfig::new("instance", "flow"), starter.clone());

        let task = tokio::spawn({
            let adapter = adapter.clone();
            async move { adapter.originate_contact(BTreeMap::new(), None, None).await }
        });
        starter.entered.notified().await;
        task.abort();
        let _ = task.await;

        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("cancelled control future was dropped")
            .expect("drop signal sender survived until cancellation");
        assert_eq!(adapter.metrics().active_sessions, 0);
        assert_eq!(adapter.metrics().contacts_started, 0);
    }

    #[tokio::test]
    async fn cancelling_after_contact_start_stops_owned_contact() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("local signaling listener");
        let address = listener.local_addr().expect("listener address");
        let (stopped_tx, mut stopped_rx) = mpsc::unbounded_channel();
        let starter = Arc::new(StopRecordingStarter {
            signaling_url: format!("ws://{address}/control/meeting"),
            stopped: stopped_tx,
        });
        let adapter =
            AmazonConnectAdapter::new(ConnectConfig::new("instance-owned", "flow"), starter);
        let (started_tx, started_rx) = oneshot::channel();
        let started_tx = Arc::new(SyncMutex::new(Some(started_tx)));
        let observer: ContactSetupObserver = Arc::new(move |stage| {
            if stage == ContactSetupStage::ContactStarted {
                if let Some(tx) = started_tx.lock().take() {
                    let _ = tx.send(());
                }
            }
        });

        let task = tokio::spawn({
            let adapter = Arc::clone(&adapter);
            async move {
                adapter
                    .originate_contact_to_observed(
                        ContactTarget::default(),
                        BTreeMap::new(),
                        None,
                        None,
                        Some(observer),
                    )
                    .await
            }
        });
        started_rx.await.expect("contact-start observer");
        task.abort();
        let _ = task.await;

        let stopped = tokio::time::timeout(Duration::from_secs(1), stopped_rx.recv())
            .await
            .expect("StopContact timeout")
            .expect("StopContact request");
        assert_eq!(stopped.instance_id, "instance-owned");
        assert_eq!(stopped.contact_id, "owned-contact");
        drop(listener);
    }

    #[tokio::test]
    async fn post_start_signaling_failure_stops_owned_contact() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("local signaling listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("signaling connection");
            drop(stream);
        });
        let (stopped_tx, mut stopped_rx) = mpsc::unbounded_channel();
        let starter = Arc::new(StopRecordingStarter {
            signaling_url: format!("ws://{address}/control/meeting"),
            stopped: stopped_tx,
        });
        let adapter =
            AmazonConnectAdapter::new(ConnectConfig::new("instance-failed", "flow"), starter);

        let result = adapter.originate_contact(BTreeMap::new(), None, None).await;
        assert!(result.is_err());
        server.await.expect("signaling server");
        let stopped = tokio::time::timeout(Duration::from_secs(1), stopped_rx.recv())
            .await
            .expect("StopContact timeout")
            .expect("StopContact request");
        assert_eq!(stopped.instance_id, "instance-failed");
        assert_eq!(stopped.contact_id, "owned-contact");
    }

    #[tokio::test]
    async fn stop_contact_retries_transient_failures_with_a_fixed_budget() {
        let starter = Arc::new(ScriptedStopStarter {
            attempts: AtomicUsize::new(0),
            transient_failures: 2,
            permanent_failure: false,
        });
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        stop_contact_with_retry(
            &starter_trait,
            StopContactRequest {
                instance_id: "instance".into(),
                contact_id: "contact".into(),
            },
        )
        .await
        .expect("third StopContact attempt succeeds");
        assert_eq!(starter.attempts.load(Ordering::SeqCst), 3);

        let exhausted = Arc::new(ScriptedStopStarter {
            attempts: AtomicUsize::new(0),
            transient_failures: STOP_CONTACT_ATTEMPTS,
            permanent_failure: false,
        });
        let exhausted_trait: Arc<dyn ConnectContactStarter> = exhausted.clone();
        assert!(stop_contact_with_retry(
            &exhausted_trait,
            StopContactRequest {
                instance_id: "instance".into(),
                contact_id: "contact".into(),
            },
        )
        .await
        .is_err());
        assert_eq!(
            exhausted.attempts.load(Ordering::SeqCst),
            STOP_CONTACT_ATTEMPTS
        );
    }

    #[tokio::test]
    async fn stop_contact_does_not_retry_permanent_failure() {
        let starter = Arc::new(ScriptedStopStarter {
            attempts: AtomicUsize::new(0),
            transient_failures: 0,
            permanent_failure: true,
        });
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        assert!(stop_contact_with_retry(
            &starter_trait,
            StopContactRequest {
                instance_id: "instance".into(),
                contact_id: "contact".into(),
            },
        )
        .await
        .is_err());
        assert_eq!(starter.attempts.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cleanup_journal_failure_stops_contact_before_media_or_route_ownership() {
        let starter = Arc::new(CountingStarter::default());
        let adapter =
            AmazonConnectAdapter::new(ConnectConfig::new("instance", "flow"), starter.clone());
        let observer: Arc<dyn AmazonConnectCleanupObserver> =
            Arc::new(FailingRetainCleanupObserver);
        adapter.install_cleanup_observer(observer).unwrap();
        let permit = Arc::new(
            Arc::clone(&adapter.cleanup_slots)
                .try_acquire_owned()
                .unwrap(),
        );
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        let result = StartedContactGuard::new(
            ConnectProfileId::default(),
            starter_trait,
            StopContactRequest {
                instance_id: "instance".into(),
                contact_id: "journal-failure-contact".into(),
            },
            permit,
            Arc::clone(&adapter.pending_cleanups),
            Arc::clone(&adapter.cleanup_observer),
        )
        .await;
        assert!(result.is_err());
        tokio::time::timeout(Duration::from_secs(1), async {
            while starter.stops.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("failed journal contact cleanup");
        assert!(adapter.routes.is_empty());
        assert!(adapter.prepared_routes.is_empty());
        assert_eq!(adapter.pending_cleanup_count(), 0);
    }

    #[tokio::test]
    async fn failed_stop_retains_bounded_ownership_until_retry_succeeds() {
        let starter = Arc::new(RecoveringStopStarter {
            attempts: AtomicUsize::new(0),
            transient_failures_remaining: AtomicUsize::new(STOP_CONTACT_ATTEMPTS),
        });
        let adapter =
            AmazonConnectAdapter::new(ConnectConfig::new("instance", "flow"), starter.clone());
        let observer = Arc::new(RecordingCleanupObserver::default());
        let observer_trait: Arc<dyn AmazonConnectCleanupObserver> = observer.clone();
        adapter
            .install_cleanup_observer(observer_trait)
            .expect("install cleanup observer before admission");
        let duplicate: Arc<dyn AmazonConnectCleanupObserver> =
            Arc::new(RecordingCleanupObserver::default());
        assert!(adapter.install_cleanup_observer(duplicate).is_err());
        let permit = Arc::new(
            Arc::clone(&adapter.cleanup_slots)
                .try_acquire_owned()
                .expect("cleanup capacity"),
        );
        let request = StopContactRequest {
            instance_id: "instance".into(),
            contact_id: "pending-contact".into(),
        };
        let starter_trait: Arc<dyn ConnectContactStarter> = starter.clone();
        let mut guard = StartedContactGuard::new(
            ConnectProfileId::default(),
            starter_trait,
            request,
            permit,
            Arc::clone(&adapter.pending_cleanups),
            Arc::clone(&adapter.cleanup_observer),
        )
        .await
        .unwrap();
        {
            let retained = observer.retained.lock();
            assert_eq!(retained.len(), 1);
            assert_eq!(retained[0].profile_id().as_str(), "default");
            assert_eq!(retained[0].instance_id(), "instance");
            assert_eq!(retained[0].contact_id(), "pending-contact");
            assert!(!format!("{:?}", retained[0]).contains("pending-contact"));
        }

        assert!(guard.stop_now().await.is_err());
        assert_eq!(
            starter.attempts.load(Ordering::SeqCst),
            STOP_CONTACT_ATTEMPTS
        );
        assert_eq!(adapter.pending_cleanup_count(), 1);

        assert!(adapter
            .retry_pending_cleanup("pending-contact")
            .await
            .expect("retry succeeds"));
        assert_eq!(
            starter.attempts.load(Ordering::SeqCst),
            STOP_CONTACT_ATTEMPTS + 1
        );
        assert_eq!(adapter.pending_cleanup_count(), 0);
        assert_eq!(observer.resolved.lock().len(), 1);
        assert!(!adapter
            .retry_pending_cleanup("pending-contact")
            .await
            .expect("missing record is not an error"));
    }

    #[tokio::test]
    async fn pending_cleanup_identity_cannot_cross_profile_instances() {
        let default = Arc::new(CountingStarter::default());
        let profile_a = Arc::new(CountingStarter::default());
        let profile_b = Arc::new(CountingStarter::default());
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("legacy-instance", "legacy-flow"),
            default.clone(),
        );
        for (profile_id, instance_id, starter) in [
            (
                ConnectProfileId::new("profile-a").unwrap(),
                "instance-a",
                profile_a.clone() as Arc<dyn ConnectContactStarter>,
            ),
            (
                ConnectProfileId::new("profile-b").unwrap(),
                "instance-b",
                profile_b.clone() as Arc<dyn ConnectContactStarter>,
            ),
        ] {
            let request = StopContactRequest {
                instance_id: instance_id.into(),
                contact_id: "same-contact".into(),
            };
            let permit = Arc::new(
                Arc::clone(&adapter.cleanup_slots)
                    .try_acquire_owned()
                    .expect("cleanup capacity"),
            );
            adapter.pending_cleanups.insert(
                PendingCleanupKey::from_request(&request),
                PendingCleanupRecord {
                    profile_id,
                    request,
                    starter,
                    _permit: permit,
                },
            );
        }

        assert!(adapter.retry_pending_cleanup("same-contact").await.is_err());
        assert_eq!(profile_a.stops.load(Ordering::SeqCst), 0);
        assert_eq!(profile_b.stops.load(Ordering::SeqCst), 0);
        assert!(adapter
            .retry_pending_cleanup_for("instance-a", "same-contact")
            .await
            .unwrap());
        assert_eq!(profile_a.stops.load(Ordering::SeqCst), 1);
        assert_eq!(profile_b.stops.load(Ordering::SeqCst), 0);
        assert!(adapter
            .retry_pending_cleanup_for("instance-b", "same-contact")
            .await
            .unwrap());
        assert_eq!(profile_b.stops.load(Ordering::SeqCst), 1);
        assert_eq!(default.stops.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.pending_cleanup_count(), 0);
    }

    #[tokio::test]
    async fn injectable_media_session_drives_legacy_controls_and_cleanup() {
        let (stopped_tx, mut stopped_rx) = mpsc::unbounded_channel();
        let starter = Arc::new(StopRecordingStarter {
            signaling_url: "wss://not-used.example.test/control".into(),
            stopped: stopped_tx,
        });
        let media = MockMediaSession::new();
        let connector = Arc::new(MockMediaConnector {
            session: media.clone(),
            connects: AtomicUsize::new(0),
            fail: false,
        });
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector.clone();
        let adapter =
            AmazonConnectAdapter::builder(ConnectConfig::new("instance", "flow"), starter)
                .with_media_connector(connector_trait)
                .build();
        let mut events = adapter.subscribe_events();

        let conn = adapter
            .originate_contact(BTreeMap::new(), None, None)
            .await
            .expect("mock media connection succeeds");
        assert_eq!(connector.connects.load(Ordering::SeqCst), 1);
        assert!(adapter.streams_for(&conn).expect("route exists").is_empty());
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Connected { .. })
        ));

        adapter.hold(conn.clone()).await.expect("hold via seam");
        adapter.resume(conn.clone()).await.expect("resume via seam");
        adapter
            .send_dtmf(conn.clone(), "5", 100)
            .await
            .expect("DTMF via seam");
        assert_eq!(media.calls.holds.load(Ordering::SeqCst), 1);
        assert_eq!(media.calls.resumes.load(Ordering::SeqCst), 1);
        assert_eq!(media.calls.dtmfs.load(Ordering::SeqCst), 1);
        media
            .dtmf_tx
            .send(crate::media::ConnectMediaDtmfEvent {
                digit: '#',
                duration_ms: 120,
            })
            .await
            .expect("inbound DTMF watcher is alive");
        assert!(matches!(
            events.recv().await,
            Some(AdapterEvent::Dtmf {
                digits,
                duration_ms: 120,
                ..
            }) if digits == "#"
        ));

        adapter
            .end(conn, EndReason::Normal)
            .await
            .expect("bounded close and StopContact succeed");
        assert_eq!(media.calls.closes.load(Ordering::SeqCst), 1);
        let stopped = stopped_rx.recv().await.expect("StopContact request");
        assert_eq!(stopped.contact_id, "owned-contact");
    }

    #[tokio::test]
    async fn media_connector_failure_stops_contact_and_redacts_detail() {
        let (stopped_tx, mut stopped_rx) = mpsc::unbounded_channel();
        let starter = Arc::new(StopRecordingStarter {
            signaling_url: "wss://not-used.example.test/control".into(),
            stopped: stopped_tx,
        });
        let connector = Arc::new(MockMediaConnector {
            session: MockMediaSession::new(),
            connects: AtomicUsize::new(0),
            fail: true,
        });
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector.clone();
        let adapter =
            AmazonConnectAdapter::builder(ConnectConfig::new("instance", "flow"), starter)
                .with_media_connector(connector_trait)
                .build();

        let error = adapter
            .originate_contact(BTreeMap::new(), None, None)
            .await
            .expect_err("mock connector fails");
        assert_eq!(connector.connects.load(Ordering::SeqCst), 1);
        assert!(!format!("{error:?} {error}").contains("mock connector secret detail"));
        assert_eq!(
            stopped_rx
                .recv()
                .await
                .expect("compensating StopContact")
                .contact_id,
            "owned-contact"
        );
        assert_eq!(adapter.metrics().active_sessions, 0);
    }

    #[tokio::test]
    async fn adapter_enforces_close_deadline_and_continues_contact_cleanup() {
        let (stopped_tx, mut stopped_rx) = mpsc::unbounded_channel();
        let starter = Arc::new(StopRecordingStarter {
            signaling_url: "wss://not-used.example.test/control".into(),
            stopped: stopped_tx,
        });
        let media = MockMediaSession::blocking_close();
        let connector = Arc::new(MockMediaConnector {
            session: media.clone(),
            connects: AtomicUsize::new(0),
            fail: false,
        });
        let connector_trait: Arc<dyn ConnectMediaConnector> = connector;
        let mut config = ConnectConfig::new("instance", "flow");
        config.signaling_timeout = Duration::from_millis(20);
        let adapter = AmazonConnectAdapter::builder(config, starter)
            .with_media_connector(connector_trait)
            .build();
        let conn = adapter
            .originate_contact(BTreeMap::new(), None, None)
            .await
            .expect("mock media connects");

        tokio::time::timeout(Duration::from_secs(1), adapter.end(conn, EndReason::Normal))
            .await
            .expect("adapter enforces connector close deadline")
            .expect("StopContact still succeeds");
        assert_eq!(media.calls.closes.load(Ordering::SeqCst), 1);
        assert_eq!(media.calls.aborts.load(Ordering::SeqCst), 1);
        assert_eq!(
            stopped_rx
                .recv()
                .await
                .expect("StopContact after abort")
                .contact_id,
            "owned-contact"
        );
    }

    #[tokio::test]
    async fn active_route_teardown_retains_failed_stop_until_retry_succeeds() {
        let default_starter = Arc::new(CountingStarter::default());
        let starter = Arc::new(RecoveringStopStarter {
            attempts: AtomicUsize::new(0),
            transient_failures_remaining: AtomicUsize::new(STOP_CONTACT_ATTEMPTS),
        });
        let adapter = AmazonConnectAdapter::new(
            ConnectConfig::new("instance", "flow"),
            default_starter.clone(),
        );
        let media = MockMediaSession::new();
        let conn = ConnectionId::new();
        let permit = Arc::new(
            Arc::clone(&adapter.cleanup_slots)
                .try_acquire_owned()
                .expect("cleanup capacity"),
        );
        adapter.routes.insert(
            conn.clone(),
            Route {
                media: media.clone(),
                cancel: Arc::new(Notify::new()),
                stop_request: StopContactRequest {
                    instance_id: "instance".into(),
                    contact_id: "active-pending-contact".into(),
                },
                profile_id: ConnectProfileId::default(),
                starter: starter.clone(),
                cleanup_permit: permit,
                prepared: None,
                supervisor: Arc::new(AsyncMutex::new(None)),
            },
        );

        assert!(adapter.end(conn, EndReason::Normal).await.is_err());
        assert_eq!(
            starter.attempts.load(Ordering::SeqCst),
            STOP_CONTACT_ATTEMPTS
        );
        assert_eq!(default_starter.stops.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.pending_cleanup_count(), 1);
        assert_eq!(media.calls.closes.load(Ordering::SeqCst), 1);

        assert!(adapter
            .retry_pending_cleanup("active-pending-contact")
            .await
            .expect("retry succeeds"));
        assert_eq!(
            starter.attempts.load(Ordering::SeqCst),
            STOP_CONTACT_ATTEMPTS + 1
        );
        assert_eq!(default_starter.stops.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.pending_cleanup_count(), 0);
    }

    #[tokio::test]
    async fn typed_remote_media_end_surfaces_adapter_ended_event() {
        let (adapter, starter) = adapter_with_capture();
        let mut events = adapter.subscribe_events();
        let conn = ConnectionId::new();
        let media = MockMediaSession::new();
        let starter: Arc<dyn ConnectContactStarter> = starter;
        let route = route_with_mock_media(&adapter, media.clone(), starter);
        install_supervised_test_route(&adapter, conn.clone(), &route).await;
        media.terminal(ConnectMediaTerminalCause::RemoteEnded);

        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("Ended event timeout")
            .expect("adapter event stream remains open");
        match event {
            AdapterEvent::Ended {
                connection_id,
                reason,
            } => {
                assert_eq!(connection_id, conn);
                assert!(matches!(reason, EndReason::Normal));
            }
            other => panic!("expected remote Ended event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn typed_media_error_surfaces_adapter_failed_event_without_detail() {
        let (adapter, starter) = adapter_with_capture();
        let mut events = adapter.subscribe_events();
        let conn = ConnectionId::new();
        let media = MockMediaSession::new();
        let starter: Arc<dyn ConnectContactStarter> = starter;
        let route = route_with_mock_media(&adapter, media.clone(), starter);
        install_supervised_test_route(&adapter, conn.clone(), &route).await;
        media.terminal(ConnectMediaTerminalCause::RemoteError { status: Some(503) });

        let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
            .await
            .expect("Failed event timeout")
            .expect("adapter event stream remains open");
        match event {
            AdapterEvent::Failed {
                connection_id,
                detail,
            } => {
                assert_eq!(connection_id, conn);
                assert_eq!(detail, "Amazon media session failed");
            }
            other => panic!("expected remote Failed event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn local_cancellation_suppresses_remote_ended_event() {
        let (adapter, starter) = adapter_with_capture();
        let mut events = adapter.subscribe_events();
        let media = MockMediaSession::new();
        let starter: Arc<dyn ConnectContactStarter> = starter;
        let route = route_with_mock_media(&adapter, media.clone(), starter);
        let conn = ConnectionId::new();
        install_supervised_test_route(&adapter, conn.clone(), &route).await;
        tokio::task::yield_now().await;
        route.cancel.notify_waiters();
        tokio::task::yield_now().await;
        media.terminal(ConnectMediaTerminalCause::RemoteEnded);

        assert!(
            tokio::time::timeout(Duration::from_millis(50), events.recv())
                .await
                .is_err(),
            "local teardown must not loop back as a remote Ended event"
        );
        adapter.routes.remove(&conn);
    }
}
