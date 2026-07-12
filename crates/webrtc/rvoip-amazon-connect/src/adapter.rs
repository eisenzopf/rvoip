//! [`AmazonConnectAdapter`] — `ConnectionAdapter` implementation that delivers
//! a call to an Amazon Connect agent over the Chime SDK WebRTC media plane.
//!
//! The natural entry point is [`AmazonConnectAdapter::originate_contact`], which
//! runs the full control + signaling + media establishment and returns a
//! connected [`ConnectionId`] ready to be bridged to the inbound leg via
//! `Orchestrator::bridge_connections`. The generic [`ConnectionAdapter::originate`]
//! delegates to it with no attributes.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use dashmap::DashMap;
use parking_lot::Mutex as SyncMutex;
use tokio::sync::{mpsc, Notify, OwnedSemaphorePermit, Semaphore};
use tracing::{debug, info, instrument, warn};

use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;

use rvoip_webrtc::media::{from_tracks_with_dtmf_events, WebRtcMediaStream};
use rvoip_webrtc::{PeerRole, RvoipPeerConnection, WebRtcConfig};

use crate::config::ConnectConfig;
use crate::control::{ConnectContactStarter, StartContactRequest, StopContactRequest};
use crate::errors::ConnectError;
use crate::signaling::ChimeSignalingClient;

/// Event channel depth (mirrors rvoip-webrtc's `ADAPTER_EVENT_CAP`).
pub const ADAPTER_EVENT_CAP: usize = 256;

/// Per-call override of the Amazon Connect contact target.
///
/// Every `None` field falls back to the adapter's [`ConnectConfig`], so a
/// default-constructed target reproduces the classic single-target behaviour.
/// This is the multi-tenant hook: one adapter (one credential chain, one
/// region) can place contacts into different Connect instances/flows per call.
#[derive(Clone, Debug, Default)]
pub struct ContactTarget {
    /// Amazon Connect instance id override.
    pub instance_id: Option<String>,
    /// Contact-flow id override.
    pub contact_flow_id: Option<String>,
    /// Display-name fallback override, used when the caller supplies no
    /// per-call display name.
    pub default_display_name: Option<String>,
}

/// One active Amazon Connect contact: the Chime peer connection, its signaling
/// session, and the bridged media stream(s).
#[derive(Clone)]
struct Route {
    peer: Arc<RvoipPeerConnection>,
    /// Owns the signaling websocket + keepalive; taken and shut down on `end`.
    chime: Arc<SyncMutex<Option<crate::signaling::ChimeSession>>>,
    streams: Arc<DashMap<StreamId, Arc<WebRtcMediaStream>>>,
    negotiated: NegotiatedCodecs,
    cancel: Arc<Notify>,
    failed_at: Arc<SyncMutex<Option<Instant>>>,
    /// Amazon Connect contact id (for correlation / logging).
    contact_id: String,
    /// Control-plane ownership retained until teardown.
    stop_request: StopContactRequest,
    cleanup_permit: Arc<OwnedSemaphorePermit>,
}

const MAX_OWNED_CONTACT_CLEANUPS: usize = 4_096;

struct PendingCleanupRecord {
    request: StopContactRequest,
    _permit: Arc<OwnedSemaphorePermit>,
}

type PendingCleanupMap = Arc<DashMap<String, PendingCleanupRecord>>;

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
                    request.contact_id.clone(),
                    PendingCleanupRecord {
                        request,
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
            Err(ConnectError::TransientControl(detail)) if attempt < STOP_CONTACT_ATTEMPTS => {
                warn!(attempt, %detail, "transient StopContact failure; retrying");
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
                        request.contact_id.clone(),
                        PendingCleanupRecord {
                            request,
                            _permit: permit,
                        },
                    );
                    warn!(%error, "failed to stop Connect contact during setup cleanup");
                }
            });
        } else {
            pending.insert(
                request.contact_id.clone(),
                PendingCleanupRecord {
                    request: request.clone(),
                    _permit: permit,
                },
            );
            warn!(
                contact_id = %request.contact_id,
                "cannot stop Connect contact: no Tokio runtime during cleanup"
            );
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

/// Amazon Connect interop adapter.
pub struct AmazonConnectAdapter {
    config: ConnectConfig,
    /// WebRTC peer/media settings (ICE servers are overridden per-contact from
    /// the Chime JOIN_ACK TURN credentials).
    webrtc: WebRtcConfig,
    starter: Arc<dyn ConnectContactStarter>,
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
            routes: Arc::new(DashMap::new()),
            events_tx,
            events_rx: SyncMutex::new(Some(events_rx)),
            contacts_started: Arc::new(AtomicUsize::new(0)),
            failures: Arc::new(AtomicUsize::new(0)),
            cleanup_slots: Arc::new(Semaphore::new(MAX_OWNED_CONTACT_CLEANUPS)),
            pending_cleanups: Arc::new(DashMap::new()),
        })
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
        self.routes.get(conn).map(|route| {
            route
                .streams
                .iter()
                .map(|e| Arc::clone(e.value()) as Arc<dyn MediaStream>)
                .collect()
        })
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
        let Some(request) = self
            .pending_cleanups
            .get(contact_id)
            .map(|record| record.request.clone())
        else {
            return Ok(false);
        };
        stop_contact_with_retry(&self.starter, request).await?;
        self.pending_cleanups.remove(contact_id);
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
            .establish_inner(target, attributes, display_name, description, observer)
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
        let connection_data = self.starter.start_webrtc_contact(request).await?;
        self.contacts_started.fetch_add(1, Ordering::Relaxed);
        let mut cleanup = StartedContactGuard::new(
            Arc::clone(&self.starter),
            StopContactRequest {
                instance_id,
                contact_id: connection_data.contact_id.clone(),
            },
            Arc::clone(&cleanup_permit),
            Arc::clone(&self.pending_cleanups),
        );
        if let Some(observer) = observer.as_ref() {
            observer(ContactSetupStage::ContactStarted);
        }
        info!(
            contact_id = %connection_data.contact_id,
            meeting_id = %connection_data.meeting_id,
            "started Amazon Connect WebRTC contact"
        );

        let outcome = async {
            // 2. Chime signaling JOIN → JOIN_ACK (yields TURN credentials).
            let join =
                ChimeSignalingClient::join(&connection_data, self.config.signaling_timeout).await?;

            // 3. Build the offerer peer connection seeded with the meeting's TURN
            //    servers, then generate the SDP offer.
            let mut webrtc = self.webrtc.clone();
            let mut ice = webrtc.ice_servers.clone();
            ice.extend(join.ice_servers());
            webrtc.ice_servers = ice;

            let peer = RvoipPeerConnection::new(&webrtc, PeerRole::Offerer).await?;
            peer.add_local_audio_track().await?;
            let offer_sdp = peer.create_offer_and_gather().await?;

            // 4. SUBSCRIBE(offer) → SUBSCRIBE_ACK(answer); session keeps the socket.
            let (answer_sdp, mut session) = join
                .subscribe(
                    offer_sdp,
                    self.config.signaling_timeout,
                    self.config.keepalive_interval,
                )
                .await?;
            // Take the "Chime ended on its own" signal so we can surface a
            // reverse-direction `Ended` (e.g. agent hangup) before storing the session.
            let chime_ended = session.take_ended_signal();
            peer.set_remote_answer(&answer_sdp).await?;

            // 5. Wait for DTLS/ICE to come up.
            peer.wait_connected(self.config.media_connect_timeout)
                .await?;

            // 6. Seed the bridgeable audio media stream.
            let conn_id = ConnectionId::new();
            let negotiated = NegotiatedCodecs::default();
            let cancel = Arc::new(Notify::new());
            let Some(stop_request) = cleanup.request() else {
                return Err(ConnectError::Control(
                    "started contact cleanup ownership was lost".into(),
                ));
            };
            let route = Route {
                peer: Arc::clone(&peer),
                chime: Arc::new(SyncMutex::new(Some(session))),
                streams: Arc::new(DashMap::new()),
                negotiated: negotiated.clone(),
                cancel: Arc::clone(&cancel),
                failed_at: Arc::new(SyncMutex::new(None)),
                contact_id: connection_data.contact_id.clone(),
                stop_request,
                cleanup_permit,
            };
            self.seed_media_stream(&conn_id, &route).await;
            self.spawn_fail_watcher(conn_id.clone(), &route);
            self.spawn_chime_end_watcher(conn_id.clone(), chime_ended, Arc::clone(&cancel));
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

    /// Build the audio `WebRtcMediaStream` (outbound from our mic track,
    /// inbound from the agent's track). Inbound DTMF is surfaced as
    /// `AdapterEvent::Dtmf`.
    async fn seed_media_stream(&self, conn: &ConnectionId, route: &Route) {
        let codec = route.negotiated.audio.clone().unwrap_or_else(opus_codec);
        let Some(local) = route.peer.local_audio_track() else {
            debug!(conn = %conn, "no local audio track; media stream not seeded");
            return;
        };
        let Some(local_ssrc) = route.peer.local_audio_ssrc() else {
            return;
        };
        let payload_type = payload_type_for_audio_codec(&codec);
        let remote = route
            .peer
            .wait_remote_track(Duration::from_millis(500))
            .await;

        let (dtmf_tx, mut dtmf_rx) =
            mpsc::channel::<rvoip_webrtc::media::dtmf::DecodedDtmfEvent>(32);
        let events_tx = self.events_tx.clone();
        let conn_for_dtmf = conn.clone();
        tokio::spawn(async move {
            while let Some(event) = dtmf_rx.recv().await {
                let _ = events_tx
                    .send(AdapterEvent::Dtmf {
                        connection_id: conn_for_dtmf.clone(),
                        digits: event.digit.to_string(),
                        duration_ms: event.duration_ms,
                    })
                    .await;
            }
        });

        let stream_id = StreamId::new();
        let media = from_tracks_with_dtmf_events(
            stream_id.clone(),
            codec,
            local,
            local_ssrc,
            payload_type,
            remote,
            Some(dtmf_tx),
        );
        route.streams.insert(stream_id, media);
    }

    /// Watch for peer-connection failure and surface it as `AdapterEvent::Failed`.
    fn spawn_fail_watcher(&self, conn: ConnectionId, route: &Route) {
        let peer = Arc::clone(&route.peer);
        let failed_at = Arc::clone(&route.failed_at);
        let cancel = Arc::clone(&route.cancel);
        let events_tx = self.events_tx.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel.notified() => {}
                _ = peer.wait_failed() => {
                    *failed_at.lock() = Some(Instant::now());
                    let _ = events_tx
                        .send(AdapterEvent::Failed {
                            connection_id: conn,
                            detail: "chime peer connection failed".into(),
                        })
                        .await;
                }
            }
        });
    }

    /// Watch for the Chime signaling session ending on its own (agent hangup /
    /// socket close) and surface it as `AdapterEvent::Ended` so the bridge can
    /// hang up the far (e.g. SIP) leg. The `ended_rx` resolves to `Err` when we
    /// tore down locally (sender dropped), in which case we stay quiet.
    fn spawn_chime_end_watcher(
        &self,
        conn: ConnectionId,
        ended_rx: Option<tokio::sync::oneshot::Receiver<()>>,
        cancel: Arc<Notify>,
    ) {
        let Some(ended_rx) = ended_rx else { return };
        let events_tx = self.events_tx.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel.notified() => {}
                r = ended_rx => {
                    if r.is_ok() {
                        info!(conn = %conn, "chime/agent leg ended; surfacing Ended");
                        let _ = events_tx
                            .send(AdapterEvent::Ended {
                                connection_id: conn,
                                reason: EndReason::Normal,
                            })
                            .await;
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
            // Take the session out from under the (non-Send) parking_lot guard
            // before awaiting, so the future stays Send.
            let session = route.chime.lock().take();
            if let Some(session) = session {
                session.shutdown().await;
            }
            route.peer.close().await.ok();
            if let Err(error) =
                stop_contact_with_retry(&self.starter, route.stop_request.clone()).await
            {
                self.pending_cleanups.insert(
                    route.stop_request.contact_id.clone(),
                    PendingCleanupRecord {
                        request: route.stop_request,
                        _permit: Arc::clone(&route.cleanup_permit),
                    },
                );
                return Err(error);
            }
            info!(conn = %conn, contact_id = %route.contact_id, "ended Amazon Connect contact");
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

    #[instrument(skip(self, request), fields(instance = %self.config.instance_id))]
    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        let (conn_id, negotiated) = self
            .establish(
                ContactTarget::default(),
                BTreeMap::new(),
                None,
                None,
                request.session_id.clone(),
                request.participant_id.clone(),
                None,
            )
            .await
            .map_err(RvoipError::from)?;

        let connection = Connection {
            id: conn_id,
            session_id: request.session_id,
            participant_id: request.participant_id,
            transport: Transport::AmazonConnect,
            direction: Direction::Outbound,
            state: ConnectionState::Connected,
            capabilities: self.webrtc.capabilities.clone(),
            negotiated_codecs: negotiated,
            streams: vec![],
            messaging_enabled: false,
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: chrono::Utc::now(),
            closed_at: None,
        };
        Ok(ConnectionHandle::new(connection))
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
            .peer
            .hold_audio()
            .await
            .map_err(|e| RvoipError::Adapter(format!("hold: {e}")))
    }

    async fn resume(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self.route(&conn).map_err(RvoipError::from)?;
        route
            .peer
            .resume_audio()
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
        let route = self.route(&conn).map_err(RvoipError::from)?;
        rvoip_webrtc::media::dtmf::send_dtmf(&route.peer, digits, duration_ms)
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
    use std::future::pending;
    use tokio::sync::oneshot;

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
    async fn active_route_teardown_retains_failed_stop_until_retry_succeeds() {
        let starter = Arc::new(RecoveringStopStarter {
            attempts: AtomicUsize::new(0),
            transient_failures_remaining: AtomicUsize::new(STOP_CONTACT_ATTEMPTS),
        });
        let adapter =
            AmazonConnectAdapter::new(ConnectConfig::new("instance", "flow"), starter.clone());
        let peer = RvoipPeerConnection::new(&WebRtcConfig::default(), PeerRole::Offerer)
            .await
            .expect("test peer");
        let conn = ConnectionId::new();
        let permit = Arc::new(
            Arc::clone(&adapter.cleanup_slots)
                .try_acquire_owned()
                .expect("cleanup capacity"),
        );
        adapter.routes.insert(
            conn.clone(),
            Route {
                peer,
                chime: Arc::new(SyncMutex::new(None)),
                streams: Arc::new(DashMap::new()),
                negotiated: NegotiatedCodecs::default(),
                cancel: Arc::new(Notify::new()),
                failed_at: Arc::new(SyncMutex::new(None)),
                contact_id: "active-pending-contact".into(),
                stop_request: StopContactRequest {
                    instance_id: "instance".into(),
                    contact_id: "active-pending-contact".into(),
                },
                cleanup_permit: permit,
            },
        );

        assert!(adapter.end(conn, EndReason::Normal).await.is_err());
        assert_eq!(
            starter.attempts.load(Ordering::SeqCst),
            STOP_CONTACT_ATTEMPTS
        );
        assert_eq!(adapter.pending_cleanup_count(), 1);

        assert!(adapter
            .retry_pending_cleanup("active-pending-contact")
            .await
            .expect("retry succeeds"));
        assert_eq!(
            starter.attempts.load(Ordering::SeqCst),
            STOP_CONTACT_ATTEMPTS + 1
        );
        assert_eq!(adapter.pending_cleanup_count(), 0);
    }

    #[tokio::test]
    async fn remote_chime_end_surfaces_adapter_ended_event() {
        let (adapter, _starter) = adapter_with_capture();
        let mut events = adapter.subscribe_events();
        let conn = ConnectionId::new();
        let (remote_ended_tx, remote_ended_rx) = oneshot::channel();

        adapter.spawn_chime_end_watcher(
            conn.clone(),
            Some(remote_ended_rx),
            Arc::new(Notify::new()),
        );
        remote_ended_tx.send(()).expect("watcher is alive");

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
    async fn local_cancellation_suppresses_remote_ended_event() {
        let (adapter, _starter) = adapter_with_capture();
        let mut events = adapter.subscribe_events();
        let cancel = Arc::new(Notify::new());
        let (remote_ended_tx, remote_ended_rx) = oneshot::channel::<()>();

        adapter.spawn_chime_end_watcher(
            ConnectionId::new(),
            Some(remote_ended_rx),
            Arc::clone(&cancel),
        );
        cancel.notify_one();
        drop(remote_ended_tx);

        assert!(
            tokio::time::timeout(Duration::from_millis(50), events.recv())
                .await
                .is_err(),
            "local teardown must not loop back as a remote Ended event"
        );
    }
}

/// Default Opus codec descriptor (Chime audio is Opus).
fn opus_codec() -> CodecInfo {
    CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48000,
        channels: 2,
        fmtp: None,
    }
}

/// Map a negotiated audio codec to its RTP payload type (matches the codec
/// table rvoip-webrtc's media engine registers).
fn payload_type_for_audio_codec(codec: &CodecInfo) -> u8 {
    let name = codec.name.to_ascii_lowercase();
    if name.contains("pcmu") {
        0
    } else if name.contains("pcma") {
        8
    } else {
        111 // Opus
    }
}
