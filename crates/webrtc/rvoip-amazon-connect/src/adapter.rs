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
use tokio::sync::{mpsc, Notify};
use tracing::{debug, info, instrument, warn};

use rvoip_core::adapter::{
    AdapterEvent, AdapterKind, ConnectionAdapter, ConnectionHandle, EndReason, OriginateRequest,
    RejectReason, SignatureHeaders, TransferTarget,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{
    Connection, ConnectionState, Direction, Transport, TransportHandle,
};
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;

use rvoip_webrtc::media::{from_tracks_with_dtmf_events, WebRtcMediaStream};
use rvoip_webrtc::{PeerRole, RvoipPeerConnection, WebRtcConfig};

use crate::config::ConnectConfig;
use crate::control::{ConnectContactStarter, StartContactRequest};
use crate::errors::ConnectError;
use crate::signaling::ChimeSignalingClient;

/// Event channel depth (mirrors rvoip-webrtc's `ADAPTER_EVENT_CAP`).
pub const ADAPTER_EVENT_CAP: usize = 256;

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
        let (conn_id, _negotiated) = self
            .establish(
                attributes,
                display_name,
                description,
                SessionId::new(),
                ParticipantId::new(),
            )
            .await?;
        Ok(conn_id)
    }

    /// Drive control → signaling → media. Inserts the route and emits
    /// `Connected` on success; increments the failure counter otherwise.
    async fn establish(
        &self,
        attributes: BTreeMap<String, String>,
        display_name: Option<String>,
        description: Option<String>,
        _session_id: SessionId,
        _participant_id: ParticipantId,
    ) -> crate::errors::Result<(ConnectionId, NegotiatedCodecs)> {
        match self
            .establish_inner(attributes, display_name, description)
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
        attributes: BTreeMap<String, String>,
        display_name: Option<String>,
        description: Option<String>,
    ) -> crate::errors::Result<(ConnectionId, NegotiatedCodecs)> {
        // 1. Control plane: StartWebRTCContact (attributes drive the screen pop).
        let request = StartContactRequest {
            instance_id: self.config.instance_id.clone(),
            contact_flow_id: self.config.contact_flow_id.clone(),
            display_name: display_name.unwrap_or_else(|| self.config.default_display_name.clone()),
            attributes,
            description,
            client_token: None,
        };
        let connection_data = self.starter.start_webrtc_contact(request).await?;
        self.contacts_started.fetch_add(1, Ordering::Relaxed);
        info!(
            contact_id = %connection_data.contact_id,
            meeting_id = %connection_data.meeting_id,
            "started Amazon Connect WebRTC contact"
        );

        // 2. Chime signaling JOIN → JOIN_ACK (yields TURN credentials).
        let join = ChimeSignalingClient::join(&connection_data, self.config.signaling_timeout)
            .await?;

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
        let route = Route {
            peer: Arc::clone(&peer),
            chime: Arc::new(SyncMutex::new(Some(session))),
            streams: Arc::new(DashMap::new()),
            negotiated: negotiated.clone(),
            cancel: Arc::clone(&cancel),
            failed_at: Arc::new(SyncMutex::new(None)),
            contact_id: connection_data.contact_id.clone(),
        };
        self.seed_media_stream(&conn_id, &route).await;
        self.spawn_fail_watcher(conn_id.clone(), &route);
        self.spawn_chime_end_watcher(conn_id.clone(), chime_ended, Arc::clone(&cancel));
        self.routes.insert(conn_id.clone(), route);

        self.try_send(AdapterEvent::Connected {
            connection_id: conn_id.clone(),
        });
        Ok((conn_id, negotiated))
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

    async fn teardown(&self, conn: &ConnectionId) {
        if let Some((_, route)) = self.routes.remove(conn) {
            route.cancel.notify_waiters();
            // Take the session out from under the (non-Send) parking_lot guard
            // before awaiting, so the future stays Send.
            let session = route.chime.lock().take();
            if let Some(session) = session {
                session.shutdown().await;
            }
            route.peer.close().await.ok();
            info!(conn = %conn, contact_id = %route.contact_id, "ended Amazon Connect contact");
        }
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
                BTreeMap::new(),
                None,
                None,
                request.session_id.clone(),
                request.participant_id.clone(),
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
        Ok(ConnectionHandle { connection })
    }

    async fn accept(&self, _conn: ConnectionId) -> RvoipResult<()> {
        // Connect contacts are established (media up) by the time the
        // ConnectionId exists, so accept is a no-op success.
        Ok(())
    }

    async fn reject(&self, conn: ConnectionId, _reason: RejectReason) -> RvoipResult<()> {
        self.teardown(&conn).await;
        self.try_send(AdapterEvent::Failed {
            connection_id: conn,
            detail: "rejected".into(),
        });
        Ok(())
    }

    #[instrument(skip(self), fields(conn = %conn, reason = ?reason))]
    async fn end(&self, conn: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        self.teardown(&conn).await;
        self.try_send(AdapterEvent::Ended {
            connection_id: conn,
            reason,
        });
        Ok(())
    }

    async fn hold(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self.route(&conn).map_err(RvoipError::from)?;
        route.peer.hold_audio().await.map_err(|e| {
            RvoipError::Adapter(format!("hold: {e}"))
        })
    }

    async fn resume(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self.route(&conn).map_err(RvoipError::from)?;
        route.peer.resume_audio().await.map_err(|e| {
            RvoipError::Adapter(format!("resume: {e}"))
        })
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
