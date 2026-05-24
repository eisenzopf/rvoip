//! `WebRtcAdapter` — `rvoip_core::ConnectionAdapter` for WebRTC interop.

use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
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
use rvoip_core::ids::{ConnectionId, StreamId};
use rvoip_core::message::Message;
use rvoip_core::stream::MediaStream;
use tokio::sync::mpsc;
use tracing::warn;
use webrtc::data_channel::DataChannel;

use crate::config::WebRtcConfig;
use crate::errors::{Result, WebRtcError};
use crate::media::{from_tracks, WebRtcMediaStream};
use crate::peer::{PeerRole, RvoipPeerConnection};
use crate::sdp::{negotiate_audio, parse_sdp, sdp_to_string};

pub const ADAPTER_EVENT_CAP: usize = 256;

#[derive(Clone)]
pub struct Route {
    pub peer: Arc<RvoipPeerConnection>,
    pub streams: Arc<DashMap<StreamId, Arc<WebRtcMediaStream>>>,
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub data_channel: Arc<DashMap<(), Arc<dyn DataChannel>>>,
    pub negotiated: NegotiatedCodecs,
    pub held: bool,
}

pub struct WebRtcAdapter {
    config: WebRtcConfig,
    routes: Arc<DashMap<ConnectionId, Route>>,
    events_tx: mpsc::Sender<AdapterEvent>,
    events_rx: StdMutex<Option<mpsc::Receiver<AdapterEvent>>>,
}

impl WebRtcAdapter {
    pub fn new(config: WebRtcConfig) -> Arc<Self> {
        let (events_tx, events_rx) = mpsc::channel(ADAPTER_EVENT_CAP);
        Arc::new(Self {
            config,
            routes: Arc::new(DashMap::new()),
            events_tx,
            events_rx: StdMutex::new(Some(events_rx)),
        })
    }

    pub fn routes(&self) -> &Arc<DashMap<ConnectionId, Route>> {
        &self.routes
    }

    fn try_send(&self, event: AdapterEvent) {
        if self.events_tx.try_send(event).is_err() {
            warn!("WebRtcAdapter event channel full or closed");
        }
    }

    fn build_connection(
        &self,
        conn_id: ConnectionId,
        direction: Direction,
        negotiated: NegotiatedCodecs,
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
            transport_handle: TransportHandle(Arc::new(())),
            opened_at: Utc::now(),
            closed_at: None,
        }
    }

    fn route(&self, conn: &ConnectionId) -> Result<Route> {
        self.routes
            .get(conn)
            .map(|e| e.value().clone())
            .ok_or(WebRtcError::ConnectionNotFound)
    }

    fn insert_route(&self, conn_id: ConnectionId, route: Route) {
        Self::spawn_route_watchers(
            self.routes.clone(),
            self.events_tx.clone(),
            conn_id.clone(),
            route.peer.clone(),
        );
        self.routes.insert(conn_id, route);
    }

    fn spawn_route_watchers(
        routes: Arc<DashMap<ConnectionId, Route>>,
        events_tx: mpsc::Sender<AdapterEvent>,
        conn_id: ConnectionId,
        peer: Arc<RvoipPeerConnection>,
    ) {
        let routes_track = Arc::clone(&routes);
        let conn_track = conn_id.clone();
        let peer_track = Arc::clone(&peer);
        tokio::spawn(async move {
            loop {
                if !routes_track.contains_key(&conn_track) {
                    break;
                }
                if let Some(track) = peer_track.try_recv_remote_track().await {
                    if let Some(route) = routes_track.get(&conn_track) {
                        for entry in route.streams.iter() {
                            entry.value().attach_remote(track.clone());
                        }
                    }
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });

        tokio::spawn(async move {
            peer.wait_failed().await;
            if routes.remove(&conn_id).is_some() {
                let _ = events_tx
                    .send(AdapterEvent::Failed {
                        connection_id: conn_id,
                        detail: "peer connection failed".into(),
                    })
                    .await;
            }
        });
    }

    /// Apply a remote SDP answer to an outbound (offerer) connection.
    pub async fn apply_remote_answer(
        &self,
        conn: ConnectionId,
        answer_sdp: &str,
    ) -> Result<()> {
        let route = self.route(&conn)?;
        route.peer.set_remote_answer(answer_sdp).await?;
        Ok(())
    }

    /// Handle an inbound SDP offer — creates answerer PC and emits `InboundConnection`.
    pub async fn apply_remote_offer(&self, offer_sdp: &str) -> Result<ConnectionId> {
        let conn_id = ConnectionId::new();
        let peer = RvoipPeerConnection::new(&self.config, PeerRole::Answerer).await?;
        let answer_sdp = peer.accept_offer_and_gather(offer_sdp).await?;

        let negotiated = negotiate_audio(&self.config.capabilities, &self.config.capabilities)?;

        let route = Route {
            peer: Arc::clone(&peer),
            streams: Arc::new(DashMap::new()),
            local_sdp: Some(answer_sdp),
            remote_sdp: Some(offer_sdp.to_owned()),
            data_channel: Arc::new(DashMap::new()),
            negotiated: negotiated.clone(),
            held: false,
        };
        self.insert_route(conn_id.clone(), route);

        let connection = self.build_connection(conn_id.clone(), Direction::Inbound, negotiated);
        self.try_send(AdapterEvent::InboundConnection { connection });

        Ok(conn_id)
    }

    pub fn local_sdp(&self, conn: &ConnectionId) -> Result<String> {
        self.route(conn)?
            .local_sdp
            .clone()
            .ok_or_else(|| WebRtcError::Sdp("no local SDP".into()))
    }

    async fn ensure_media_streams(&self, conn: &ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        if !route.streams.is_empty() {
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
            .ok_or_else(|| RvoipError::Adapter("no local audio track".into()))?;

        let remote = route
            .peer
            .wait_remote_track(Duration::from_millis(500))
            .await
            .or(route.peer.try_recv_remote_track().await);

        let stream_id = StreamId::new();
        let has_remote = remote.is_some();
        let media = from_tracks(stream_id.clone(), codec, local, remote);
        if has_remote {
            media.enable_webrtc_stats(Arc::clone(route.peer.peer_connection()));
        }
        route.streams.insert(stream_id, media);
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

    /// WHIP ICE restart: apply a new offer on an inbound (answerer) connection.
    pub async fn apply_ice_restart_offer(
        &self,
        conn: ConnectionId,
        offer_sdp: &str,
    ) -> Result<String> {
        let route = self.route(&conn)?;
        if route.peer.role() != PeerRole::Answerer {
            return Err(WebRtcError::Adapter(
                "WHIP ICE restart requires an inbound (answerer) connection".into(),
            ));
        }
        let answer = route.peer.renegotiate_as_answerer(offer_sdp).await?;
        if let Some(mut route_mut) = self.routes.get_mut(&conn) {
            route_mut.local_sdp = Some(answer.clone());
            route_mut.remote_sdp = Some(offer_sdp.to_owned());
        }
        Ok(answer)
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

    async fn originate(&self, request: OriginateRequest) -> RvoipResult<ConnectionHandle> {
        let conn_id = ConnectionId::new();
        let peer = RvoipPeerConnection::new(&self.config, PeerRole::Offerer)
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let offer_sdp = peer
            .create_offer_and_gather()
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let negotiated = negotiate_audio(&request.capabilities, &self.config.capabilities)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let route = Route {
            peer,
            streams: Arc::new(DashMap::new()),
            local_sdp: Some(offer_sdp),
            remote_sdp: None,
            data_channel: Arc::new(DashMap::new()),
            negotiated: negotiated.clone(),
            held: false,
        };
        self.insert_route(conn_id.clone(), route);

        let mut connection = self.build_connection(conn_id, Direction::Outbound, negotiated);
        connection.session_id = request.session_id;
        connection.participant_id = request.participant_id;

        Ok(ConnectionHandle { connection })
    }

    async fn accept(&self, conn: ConnectionId) -> RvoipResult<()> {
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        route
            .peer
            .wait_connected(Duration::from_secs(self.config.gather_timeout_secs + 10))
            .await
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        self.ensure_media_streams(&conn).await?;
        self.try_send(AdapterEvent::Connected {
            connection_id: conn,
        });
        Ok(())
    }

    async fn reject(&self, conn: ConnectionId, _reason: RejectReason) -> RvoipResult<()> {
        if let Ok(route) = self.route(&conn) {
            route.peer.close().await.ok();
        }
        self.routes.remove(&conn);
        self.try_send(AdapterEvent::Failed {
            connection_id: conn,
            detail: "rejected".into(),
        });
        Ok(())
    }

    async fn end(&self, conn: ConnectionId, reason: EndReason) -> RvoipResult<()> {
        if let Ok(route) = self.route(&conn) {
            route.peer.close().await.ok();
        }
        self.routes.remove(&conn);
        self.try_send(AdapterEvent::Ended {
            connection_id: conn,
            reason,
        });
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
        let route = self
            .route(&conn)
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;

        let dc = if let Some(entry) = route.data_channel.get(&()) {
            entry.value().clone()
        } else {
            let dc = tokio::time::timeout(
                Duration::from_secs(2),
                route.peer.peer_connection().create_data_channel("rvoip-messages", None),
            )
            .await
            .map_err(|_| RvoipError::Adapter("create_data_channel timed out".into()))?
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
            route.data_channel.insert((), Arc::clone(&dc));
            dc
        };

        let body = String::from_utf8_lossy(&message.body).into_owned();
        tokio::time::timeout(Duration::from_secs(2), dc.send_text(&body))
            .await
            .map_err(|_| RvoipError::Adapter("data channel send timed out".into()))?
            .map_err(|e| RvoipError::Adapter(format!("{e}")))?;
        Ok(())
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
        self.events_rx
            .lock()
            .unwrap()
            .take()
            .expect("subscribe_events called twice on WebRtcAdapter")
    }

    fn capabilities(&self) -> CapabilityDescriptor {
        self.config.capabilities.clone()
    }

    async fn verify_request_signature(
        &self,
        _conn: ConnectionId,
        _signature: SignatureHeaders,
    ) -> RvoipResult<IdentityAssurance> {
        Ok(IdentityAssurance::Anonymous)
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
