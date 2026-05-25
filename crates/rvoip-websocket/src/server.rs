//! `UctpWsServer` — TCP accept loop + WebSocket upgrade. One peer
//! coordinator per accepted socket; envelopes ride as text frames.

use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use rvoip_auth_core::BearerValidator;
use rvoip_core::adapter::{AdapterEvent, EndReason};
use rvoip_core::capability::{CapabilityDescriptor, NegotiatedCodecs};
use rvoip_core::connection::{
    Connection, ConnectionState, Direction, Transport, TransportHandle,
};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::state::{UctpCoordinator, UctpSessionEvent, ENVELOPE_CHANNEL_CAP};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

use crate::adapter::Route;

pub struct UctpWsServer;

impl UctpWsServer {
    pub fn start(
        listener: TcpListener,
        bearer: Arc<dyn BearerValidator>,
        events_tx: mpsc::Sender<AdapterEvent>,
        by_connection: Arc<DashMap<ConnectionId, String>>,
        by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
        routes: Arc<DashMap<ConnectionId, Route>>,
        _max_concurrent: usize,
        coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
    ) -> Arc<Self> {
        tokio::spawn(async move {
            loop {
                let (tcp, peer_addr) = match listener.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, "rvoip-websocket: accept failed");
                        continue;
                    }
                };
                let bearer = bearer.clone();
                let events_tx = events_tx.clone();
                let by_connection = Arc::clone(&by_connection);
                let by_uctp_sid = Arc::clone(&by_uctp_sid);
                let routes = Arc::clone(&routes);
                let caps = coordinator_caps.clone();
                tokio::spawn(async move {
                    let ws = match tokio_tungstenite::accept_async(tcp).await {
                        Ok(ws) => ws,
                        Err(e) => {
                            warn!(error = %e, %peer_addr, "rvoip-websocket: handshake failed");
                            return;
                        }
                    };
                    info!(%peer_addr, "rvoip-websocket: peer connected");
                    spawn_peer_session(
                        ws,
                        bearer,
                        events_tx,
                        by_connection,
                        by_uctp_sid,
                        routes,
                        caps,
                    )
                    .await;
                    info!(%peer_addr, "rvoip-websocket: peer disconnected");
                });
            }
        });
        Arc::new(Self)
    }
}

/// Synthesize a `rvoip_core::Connection` from an inbound session.invite.
fn build_connection(sid: SessionId, from: String) -> (ConnectionId, Connection) {
    let id = ConnectionId::new();
    let conn = Connection {
        id: id.clone(),
        session_id: sid,
        participant_id: ParticipantId::from_string(from),
        transport: Transport::WebSocket,
        direction: Direction::Inbound,
        state: ConnectionState::Connecting,
        capabilities: CapabilityDescriptor::default(),
        negotiated_codecs: NegotiatedCodecs::default(),
        streams: Vec::new(),
        messaging_enabled: false,
        // WS doesn't carry a quinn::Connection; TransportHandle wraps the
        // empty unit. The route map's out_tx is the actual handle code
        // uses for sending envelopes.
        transport_handle: TransportHandle(Arc::new(())),
        opened_at: Utc::now(),
        closed_at: None,
    };
    (id, conn)
}

async fn spawn_peer_session(
    ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
    bearer: Arc<dyn BearerValidator>,
    events_tx: mpsc::Sender<AdapterEvent>,
    by_connection: Arc<DashMap<ConnectionId, String>>,
    by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
    routes: Arc<DashMap<ConnectionId, Route>>,
    coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
) {
    let (mut sink, mut stream) = ws.split();

    let (in_tx, in_rx) = mpsc::channel::<UctpEnvelope>(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(ENVELOPE_CHANNEL_CAP);
    let (coord_events_tx, mut coord_events_rx) =
        mpsc::channel::<UctpSessionEvent>(ENVELOPE_CHANNEL_CAP);

    let route_out_tx = out_tx.clone();

    let _coord = UctpCoordinator::start_full_with_caps(
        "websocket",
        in_rx,
        out_tx,
        coord_events_tx,
        bearer,
        Arc::new(rvoip_uctp::state::default_v0_descriptor()),
        rvoip_uctp::state::rejecting_handler(),
        coordinator_caps,
    );

    // Inbound: WS text frames → coordinator.
    let in_tx_for_pump = in_tx.clone();
    let inbound_pump = tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    match serde_json::from_str::<UctpEnvelope>(&text) {
                        Ok(env) => {
                            if in_tx_for_pump.send(env).await.is_err() {
                                return;
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "rvoip-websocket: malformed envelope; dropping");
                        }
                    }
                }
                Ok(Message::Close(_)) => {
                    debug!("rvoip-websocket: peer sent close");
                    return;
                }
                Ok(_) => {
                    // Ignore Binary / Ping / Pong for v0 — signaling is text-only.
                }
                Err(e) => {
                    warn!(error = %e, "rvoip-websocket: read error");
                    return;
                }
            }
        }
    });

    // Outbound: coordinator → WS text frames.
    let outbound_pump = tokio::spawn(async move {
        while let Some(env) = out_rx.recv().await {
            let text = match serde_json::to_string(&env) {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "rvoip-websocket: encode failed");
                    continue;
                }
            };
            if let Err(e) = sink.send(Message::Text(text.into())).await {
                warn!(error = %e, "rvoip-websocket: write error");
                return;
            }
        }
    });

    // Coordinator events → AdapterEvent translator.
    let event_pump = {
        let events_tx = events_tx.clone();
        let by_connection = Arc::clone(&by_connection);
        let by_uctp_sid = Arc::clone(&by_uctp_sid);
        let routes = Arc::clone(&routes);
        let route_out_tx = route_out_tx.clone();
        tokio::spawn(async move {
            // Per-peer auth state; consumed by InboundInvite to emit a
            // synthetic `AdapterEvent::Authenticated` follow-up. Plan
            // §7 G1 / A3.
            let mut latest_auth: Option<(
                String,
                String,
                rvoip_core::identity::IdentityAssurance,
            )> = None;

            while let Some(event) = coord_events_rx.recv().await {
                let adapter_event: Option<AdapterEvent> = match event {
                    UctpSessionEvent::Authenticated {
                        identity_id,
                        participant_id,
                        assurance,
                    } => {
                        latest_auth = Some((identity_id, participant_id, assurance));
                        Some(AdapterEvent::Native {
                            kind: "uctp.authenticated",
                            detail: "bearer".into(),
                        })
                    }
                    UctpSessionEvent::InboundInvite { sid, from, .. } => {
                        let (id, connection) = build_connection(sid.clone(), from);
                        by_connection.insert(id.clone(), sid.to_string());
                        by_uctp_sid.insert(sid.to_string(), id.clone());

                        // Per-Connection routing record. Under `media-webrtc`
                        // the `bridge` slot is initialized empty and populated
                        // asynchronously by `spawn_bridge_setup` below — we
                        // can't `.await` `WebRtcMediaBridge::new_answerer()`
                        // inline because that would stall envelope dispatch.
                        let route_streams = Arc::new(DashMap::new());
                        #[cfg(feature = "media-webrtc")]
                        let bridge_slot: Arc<
                            parking_lot::Mutex<
                                Option<Arc<crate::media_bridge::WebRtcMediaBridge>>,
                            >,
                        > = Arc::new(parking_lot::Mutex::new(None));
                        let route = Route {
                            sid: sid.to_string(),
                            out_tx: route_out_tx.clone(),
                            streams: Arc::clone(&route_streams),
                            #[cfg(feature = "media-webrtc")]
                            bridge: Arc::clone(&bridge_slot),
                        };
                        routes.insert(id.clone(), route);

                        // Under `media-webrtc`, fire-and-forget the
                        // answerer-bridge construction + ready-watcher.
                        // Once `wait_connected` succeeds the watcher pushes
                        // the WebRtcMediaStream into `Route.streams` so that
                        // `adapter.streams(conn_id)` resolves to a real
                        // audio path for cross-transport bridging.
                        #[cfg(feature = "media-webrtc")]
                        spawn_bridge_setup(bridge_slot, route_streams);

                        let _ = events_tx
                            .send(AdapterEvent::InboundConnection { connection })
                            .await;
                        if let Some((identity_id, participant_id, assurance)) =
                            latest_auth.clone()
                        {
                            let _ = events_tx
                                .send(AdapterEvent::Authenticated {
                                    connection_id: id,
                                    identity_id,
                                    participant_id,
                                    assurance,
                                })
                                .await;
                        }
                        None
                    }
                    UctpSessionEvent::SessionConnected { sid } => {
                        match by_uctp_sid.get(sid.as_str()).map(|r| r.clone()) {
                            Some(connection_id) => {
                                Some(AdapterEvent::Connected { connection_id })
                            }
                            None => Some(AdapterEvent::Native {
                                kind: "uctp.session_connected_orphan",
                                detail: sid.to_string(),
                            }),
                        }
                    }
                    UctpSessionEvent::ConnectionConnected { connid, .. } => {
                        Some(AdapterEvent::Connected {
                            connection_id: connid,
                        })
                    }
                    UctpSessionEvent::ConnectionEnded { connid, reason, .. } => {
                        Some(AdapterEvent::Ended {
                            connection_id: connid,
                            reason: EndReason::Failed { detail: reason },
                        })
                    }
                    UctpSessionEvent::SessionEnded { sid, reason } => {
                        match by_uctp_sid.remove(sid.as_str()) {
                            Some((_, connection_id)) => {
                                by_connection.remove(&connection_id);
                                // Close + drop the per-Connection bridge
                                // before dropping the Route. Removing the
                                // Route drops the bridge Arc, but proactive
                                // close() releases the WebRTC PeerConnection
                                // (DTLS, ICE agents) cleanly rather than
                                // waiting on Drop.
                                #[cfg(feature = "media-webrtc")]
                                {
                                    let removed = routes.remove(&connection_id);
                                    let bridge_opt = removed.and_then(|(_, route)| {
                                        let guard = route.bridge.lock();
                                        guard.clone()
                                    });
                                    if let Some(bridge) = bridge_opt {
                                        tokio::spawn(async move {
                                            let _ = bridge.close().await;
                                        });
                                    }
                                }
                                #[cfg(not(feature = "media-webrtc"))]
                                {
                                    routes.remove(&connection_id);
                                }
                                Some(AdapterEvent::Ended {
                                    connection_id,
                                    reason: if reason == "cancelled" {
                                        EndReason::Cancelled
                                    } else {
                                        EndReason::Normal
                                    },
                                })
                            }
                            None => Some(AdapterEvent::Native {
                                kind: "uctp.session_ended_orphan",
                                detail: format!("sid={} reason={}", sid, reason),
                            }),
                        }
                    }
                    UctpSessionEvent::Dtmf {
                        connid,
                        digits,
                        duration_ms,
                        method: _,
                    } => Some(AdapterEvent::Dtmf {
                        connection_id: connid,
                        digits,
                        duration_ms,
                    }),
                    UctpSessionEvent::Quality {
                        connid,
                        strm_id: _,
                        snapshot,
                        rtt_ms: _,
                        bitrate_bps: _,
                    } => Some(AdapterEvent::Quality {
                        connection_id: connid,
                        snapshot,
                    }),
                    other => Some(AdapterEvent::Native {
                        kind: "uctp.internal",
                        detail: format!("{:?}", other),
                    }),
                };
                if let Some(ev) = adapter_event {
                    let _ = events_tx.send(ev).await;
                }
            }
        })
    };

    let _ = inbound_pump.await;
    let _ = outbound_pump.await;
    let _ = event_pump.await;
}

/// Spawn the per-Connection WebRTC answerer-bridge setup task.
///
/// The construction (`WebRtcMediaBridge::new_answerer`) does ICE prep + DTLS
/// material gathering and may take ~50–200 ms — too long to await inline in
/// the event-translator loop. We spawn it off, store the resulting Arc in
/// the route's `bridge_slot`, and then spawn a ready-watcher that calls
/// `wait_connected` (30s deadline) and on success pushes the bridge's
/// `WebRtcMediaStream` into `Route.streams`.
///
/// Two paths advance the bridge to "connected":
/// 1. Test / application code drives the SDP exchange via
///    [`crate::UctpWsAdapter::bridge_for`] (direct access), calling
///    `set_remote_substrate_setup` + `local_substrate_setup` against a
///    peer-side offerer bridge.
/// 2. (v0.x) Server intercepts `connection.offer`/`connection.answer`
///    envelopes and drives the exchange transparently. Not implemented in
///    v0 — see plan G1 simplification notes.
#[cfg(feature = "media-webrtc")]
fn spawn_bridge_setup(
    bridge_slot: Arc<
        parking_lot::Mutex<Option<Arc<crate::media_bridge::WebRtcMediaBridge>>>,
    >,
    route_streams: Arc<DashMap<rvoip_core::ids::StreamId, Arc<dyn rvoip_core::stream::MediaStream>>>,
) {
    use std::time::Duration;
    tokio::spawn(async move {
        let bridge = match crate::media_bridge::WebRtcMediaBridge::new_answerer().await {
            Ok(b) => Arc::new(b),
            Err(e) => {
                warn!(error = %e, "rvoip-websocket: WebRTC answerer bridge construction failed");
                return;
            }
        };
        *bridge_slot.lock() = Some(Arc::clone(&bridge));

        // Ready-watcher: wait_connected and surface the media stream.
        let bridge_for_watcher = Arc::clone(&bridge);
        tokio::spawn(async move {
            if let Err(e) = bridge_for_watcher
                .wait_connected(Duration::from_secs(30))
                .await
            {
                debug!(error = %e, "rvoip-websocket: bridge wait_connected timed out / failed");
                return;
            }
            if let Some(stream) = bridge_for_watcher.media_stream() {
                let id = rvoip_core::stream::MediaStream::id(stream.as_ref());
                route_streams.insert(id, stream as Arc<dyn rvoip_core::stream::MediaStream>);
                debug!("rvoip-websocket: WebRTC bridge connected; stream registered");
            }
        });
    });
}
