//! [`AmazonConnectAdapter`] — `ConnectionAdapter` implementation that delivers
//! a call to an Amazon Connect agent over the Chime SDK WebRTC media plane.
//!
//! The natural entry point is [`AmazonConnectAdapter::originate_contact`], which
//! runs the full control + signaling + media establishment and returns a
//! connected [`ConnectionId`] ready to be bridged to the inbound leg via
//! `Orchestrator::bridge_connections`. The generic [`ConnectionAdapter::originate`]
//! now admits only an exact typed context and remains I/O-dormant until the
//! staged lifecycle lands; the legacy wrapper remains behavior-compatible.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex as SyncMutex;
use tokio::sync::{mpsc, Notify, OwnedSemaphorePermit, Semaphore};
use tracing::{info, instrument, warn};

use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::Transport;
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
    /// Exact account/region starter selected for both Start and Stop.
    starter: Arc<dyn ConnectContactStarter>,
    cleanup_permit: Arc<OwnedSemaphorePermit>,
}

const MAX_OWNED_CONTACT_CLEANUPS: usize = 4_096;

struct PendingCleanupRecord {
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
    starter: Arc<dyn ConnectContactStarter>,
    request: Option<StopContactRequest>,
    permit: Arc<OwnedSemaphorePermit>,
    pending: PendingCleanupMap,
}

impl StartedContactGuard {
    fn new(
        starter: Arc<dyn ConnectContactStarter>,
        request: StopContactRequest,
        permit: Arc<OwnedSemaphorePermit>,
        pending: PendingCleanupMap,
    ) -> Self {
        Self {
            starter,
            request: Some(request),
            permit,
            pending,
        }
    }

    fn disarm(&mut self) -> Option<StopContactRequest> {
        self.request.take()
    }

    fn request(&self) -> Option<StopContactRequest> {
        self.request.clone()
    }

    async fn stop_now(&mut self) -> crate::errors::Result<()> {
        let Some(request) = self.request.take() else {
            return Ok(());
        };
        match stop_contact_with_retry(&self.starter, request.clone()).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.pending.insert(
                    PendingCleanupKey::from_request(&request),
                    PendingCleanupRecord {
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
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = stop_contact_with_retry(&starter, request.clone()).await {
                    pending.insert(
                        PendingCleanupKey::from_request(&request),
                        PendingCleanupRecord {
                            request,
                            starter,
                            _permit: permit,
                        },
                    );
                    warn!(%error, "failed to stop Connect contact during setup cleanup");
                }
            });
        } else {
            pending.insert(
                PendingCleanupKey::from_request(&request),
                PendingCleanupRecord {
                    request: request.clone(),
                    starter: Arc::clone(&self.starter),
                    _permit: permit,
                },
            );
            warn!("cannot stop Connect contact: no Tokio runtime during cleanup");
        }
    }
}

/// Lightweight runtime counters.
#[derive(Clone, Debug, Default)]
pub struct ConnectMetrics {
    pub contacts_started: u64,
    pub active_sessions: usize,
    pub failures: u64,
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
    events_tx: mpsc::Sender<AdapterEvent>,
    events_rx: SyncMutex<Option<mpsc::Receiver<AdapterEvent>>>,
    contacts_started: Arc<AtomicUsize>,
    failures: Arc<AtomicUsize>,
    cleanup_slots: Arc<Semaphore>,
    pending_cleanups: PendingCleanupMap,
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
            events_tx,
            events_rx: SyncMutex::new(Some(events_rx)),
            contacts_started: Arc::new(AtomicUsize::new(0)),
            failures: Arc::new(AtomicUsize::new(0)),
            cleanup_slots: Arc::new(Semaphore::new(MAX_OWNED_CONTACT_CLEANUPS)),
            pending_cleanups: Arc::new(DashMap::new()),
        })
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
        let Some((request, starter)) = self
            .pending_cleanups
            .get(&key)
            .map(|record| (record.request.clone(), Arc::clone(&record.starter)))
        else {
            return Ok(false);
        };
        stop_contact_with_retry(&starter, request).await?;
        self.pending_cleanups.remove(&key);
        Ok(true)
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
            Arc::clone(&starter),
            StopContactRequest {
                instance_id,
                contact_id: connection_data.contact_id.clone(),
            },
            Arc::clone(&cleanup_permit),
            Arc::clone(&self.pending_cleanups),
        );
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
                starter,
                cleanup_permit,
            };
            self.spawn_media_watchers(conn_id.clone(), &route);
            self.routes.insert(conn_id.clone(), route);
            let _route_owns_cleanup = cleanup.disarm();
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

    /// Forward the media seam's typed terminal and inbound DTMF events through
    /// the legacy adapter event contract. Retained task ownership lands in 5d;
    /// route cancellation still terminates both compatibility watchers.
    fn spawn_media_watchers(&self, conn: ConnectionId, route: &Route) {
        let mut terminal = route.media.subscribe_terminal();
        let cancel = Arc::clone(&route.cancel);
        let events_tx = self.events_tx.clone();
        let terminal_conn = conn.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.notified() => return,
                    changed = terminal.changed() => {
                        if changed.is_err() {
                            return;
                        }
                        let Some(cause) = *terminal.borrow_and_update() else {
                            continue;
                        };
                        let event = match cause {
                            ConnectMediaTerminalCause::RemoteEnded
                            | ConnectMediaTerminalCause::TransportClosed => {
                                AdapterEvent::Ended {
                                    connection_id: terminal_conn,
                                    reason: EndReason::Normal,
                                }
                            }
                            ConnectMediaTerminalCause::RemoteError { .. }
                            | ConnectMediaTerminalCause::TransportError
                            | ConnectMediaTerminalCause::PeerFailed => AdapterEvent::Failed {
                                connection_id: terminal_conn,
                                detail: "Amazon media session failed".into(),
                            },
                        };
                        let _ = events_tx.send(event).await;
                        return;
                    }
                }
            }
        });

        let Some(mut dtmf_rx) = route.media.take_dtmf_events() else {
            return;
        };
        let cancel = Arc::clone(&route.cancel);
        let events_tx = self.events_tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.notified() => return,
                    event = dtmf_rx.recv() => {
                        let Some(event) = event else { return };
                        if events_tx.send(AdapterEvent::Dtmf {
                            connection_id: conn.clone(),
                            digits: event.digit.to_string(),
                            duration_ms: event.duration_ms,
                        }).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
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

    async fn teardown(&self, conn: &ConnectionId) -> crate::errors::Result<()> {
        if let Some((_, route)) = self.routes.remove(conn) {
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
            if let Err(error) =
                stop_contact_with_retry(&route.starter, route.stop_request.clone()).await
            {
                self.pending_cleanups.insert(
                    PendingCleanupKey::from_request(&route.stop_request),
                    PendingCleanupRecord {
                        request: route.stop_request,
                        starter: Arc::clone(&route.starter),
                        _permit: Arc::clone(&route.cleanup_permit),
                    },
                );
                return Err(error);
            }
            info!(conn = %conn, "ended Amazon Connect contact");
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

    #[instrument(skip(self, request), fields(context_present = !request.context.is_empty()))]
    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        let (_context, _selected_starter) = self.resolve_generic_context(&request)?;
        Err(RvoipError::NotImplemented(
            "Amazon Connect staged outbound activation is not implemented",
        ))
    }

    async fn accept(&self, _conn: ConnectionId) -> RvoipResult<()> {
        // Connect contacts are established (media up) by the time the
        // ConnectionId exists, so accept is a no-op success.
        Ok(())
    }

    async fn reject(&self, conn: ConnectionId, _reason: RejectReason) -> RvoipResult<()> {
        self.teardown(&conn).await.map_err(RvoipError::from)?;
        self.try_send(AdapterEvent::Failed {
            connection_id: conn,
            detail: "rejected".into(),
        });
        Ok(())
    }

    #[instrument(skip(self), fields(conn = %conn, reason = ?reason))]
    async fn end(&self, conn: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        self.teardown(&conn).await.map_err(RvoipError::from)?;
        self.try_send(AdapterEvent::Ended {
            connection_id: conn,
            reason,
        });
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
    use crate::originate::{
        AmazonConnectOriginateContext, AmazonConnectTarget, ConnectClientToken,
    };
    use rvoip_core::connection::Direction;
    use std::future::pending;
    use tokio::sync::{oneshot, watch};

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
            .unwrap_err();
        assert!(matches!(staged, RvoipError::NotImplemented(_)));
        assert_eq!(starter.starts.load(Ordering::SeqCst), 0);
        assert_eq!(starter.stops.load(Ordering::SeqCst), 0);
        assert_eq!(adapter.metrics().contacts_started, 0);
        assert_eq!(adapter.metrics().active_sessions, 0);
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
            assert!(matches!(
                adapter
                    .originate(generic_request().with_context(typed_context(profile)))
                    .await,
                Err(RvoipError::NotImplemented(_))
            ));
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
            starter,
            cleanup_permit: Arc::new(
                Arc::clone(&adapter.cleanup_slots)
                    .try_acquire_owned()
                    .expect("cleanup capacity"),
            ),
        }
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
    async fn failed_stop_retains_bounded_ownership_until_retry_succeeds() {
        let starter = Arc::new(RecoveringStopStarter {
            attempts: AtomicUsize::new(0),
            transient_failures_remaining: AtomicUsize::new(STOP_CONTACT_ATTEMPTS),
        });
        let adapter =
            AmazonConnectAdapter::new(ConnectConfig::new("instance", "flow"), starter.clone());
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
            starter_trait,
            request,
            permit,
            Arc::clone(&adapter.pending_cleanups),
        );

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
        for (instance_id, starter) in [
            (
                "instance-a",
                profile_a.clone() as Arc<dyn ConnectContactStarter>,
            ),
            (
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
                starter: starter.clone(),
                cleanup_permit: permit,
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
        adapter.spawn_media_watchers(conn.clone(), &route);
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
        adapter.spawn_media_watchers(conn.clone(), &route);
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
        adapter.spawn_media_watchers(ConnectionId::new(), &route);
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
    }
}
