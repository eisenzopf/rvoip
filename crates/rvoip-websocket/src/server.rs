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
        #[cfg(feature = "wss")]
        tls: Option<Arc<rustls::ServerConfig>>,
    ) -> Arc<Self> {
        #[cfg(feature = "wss")]
        let tls_acceptor = tls.map(tokio_rustls::TlsAcceptor::from);

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
                #[cfg(feature = "wss")]
                let tls_acceptor = tls_acceptor.clone();
                tokio::spawn(async move {
                    #[cfg(feature = "wss")]
                    {
                        if let Some(acceptor) = tls_acceptor {
                            let tls_stream = match acceptor.accept(tcp).await {
                                Ok(s) => s,
                                Err(e) => {
                                    warn!(error = %e, %peer_addr, "rvoip-websocket: TLS handshake failed");
                                    return;
                                }
                            };
                            let ws = match tokio_tungstenite::accept_async(tls_stream).await {
                                Ok(ws) => ws,
                                Err(e) => {
                                    warn!(error = %e, %peer_addr, "rvoip-websocket: handshake failed (wss)");
                                    return;
                                }
                            };
                            info!(%peer_addr, "rvoip-websocket: peer connected over TLS");
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
                            info!(%peer_addr, "rvoip-websocket: peer disconnected (wss)");
                            return;
                        }
                    }

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

async fn spawn_peer_session<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    bearer: Arc<dyn BearerValidator>,
    events_tx: mpsc::Sender<AdapterEvent>,
    by_connection: Arc<DashMap<ConnectionId, String>>,
    by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
    routes: Arc<DashMap<ConnectionId, Route>>,
    coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
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
    // Gap plan §4.2 v1 punch list — capture the coordinator's
    // `Pending` correlator so per-Route adapter code can await
    // typed responses.
    let pending = _coord.pending();

    // Inbound: WS text frames → coordinator.
    //
    // Gap plan §2.4 envelope-level SDP interception (under `media-webrtc`):
    // when a `connection.offer` arrives, extract its `substrate_setup` and
    // apply it to the per-route answerer bridge (or queue if the bridge
    // hasn't finished constructing). The envelope still flows to the
    // coordinator unchanged so the normal §8.1 negotiation runs. After
    // the offer is applied, autonomously emit a `connection.answer`
    // envelope so the WS server can complete the SDP handshake without
    // requiring application code to drive it. The outbound pump below
    // fills in the answer's SDP from the bridge.
    let in_tx_for_pump = in_tx.clone();
    #[cfg(feature = "media-webrtc")]
    let routes_for_inbound = Arc::clone(&routes);
    #[cfg(feature = "media-webrtc")]
    let by_uctp_sid_for_inbound = Arc::clone(&by_uctp_sid);
    #[cfg(feature = "media-webrtc")]
    let route_out_tx_for_inbound = route_out_tx.clone();
    let inbound_pump = tokio::spawn(async move {
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    match serde_json::from_str::<UctpEnvelope>(&text) {
                        Ok(env) => {
                            #[cfg(feature = "media-webrtc")]
                            {
                                if env.msg_type
                                    == rvoip_uctp::types::MessageType::ConnectionOffer
                                {
                                    intercept_connection_offer(
                                        &env,
                                        &routes_for_inbound,
                                        &by_uctp_sid_for_inbound,
                                        &route_out_tx_for_inbound,
                                    )
                                    .await;
                                }
                            }
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
    //
    // Gap plan §2.4: under `media-webrtc`, before encoding a
    // `connection.answer` envelope on its way to the wire, inject the
    // local answerer SDP into the payload's `substrate_setup` field if
    // it isn't already set. This lets upper layers construct an answer
    // envelope without needing a handle to the WebRTC bridge.
    #[cfg(feature = "media-webrtc")]
    let routes_for_outbound = Arc::clone(&routes);
    #[cfg(feature = "media-webrtc")]
    let by_uctp_sid_for_outbound = Arc::clone(&by_uctp_sid);
    let outbound_pump = tokio::spawn(async move {
        while let Some(env) = out_rx.recv().await {
            #[cfg(feature = "media-webrtc")]
            let env = if env.msg_type == rvoip_uctp::types::MessageType::ConnectionAnswer {
                mutate_connection_answer(env, &routes_for_outbound, &by_uctp_sid_for_outbound).await
            } else {
                env
            };
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
                        #[cfg(feature = "media-webrtc")]
                        let pending_offer: Arc<
                            parking_lot::Mutex<
                                Option<rvoip_uctp::payloads::connection::WebRtcSubstrateSetup>,
                            >,
                        > = Arc::new(parking_lot::Mutex::new(None));
                        let route = Route {
                            sid: sid.to_string(),
                            out_tx: route_out_tx.clone(),
                            pending: Arc::clone(&pending),
                            streams: Arc::clone(&route_streams),
                            #[cfg(feature = "media-webrtc")]
                            bridge: Arc::clone(&bridge_slot),
                            #[cfg(feature = "media-webrtc")]
                            pending_offer: Arc::clone(&pending_offer),
                        };
                        routes.insert(id.clone(), route);

                        // Under `media-webrtc`, fire-and-forget the
                        // answerer-bridge construction + ready-watcher.
                        // Once `wait_connected` succeeds the watcher pushes
                        // the WebRtcMediaStream into `Route.streams` so that
                        // `adapter.streams(conn_id)` resolves to a real
                        // audio path for cross-transport bridging.
                        //
                        // The bridge-setup task also spawns the §4.1
                        // outbound trickle ICE pump, so it needs the
                        // route's out_tx + sid + connid for envelope
                        // construction.
                        #[cfg(feature = "media-webrtc")]
                        spawn_bridge_setup(
                            bridge_slot,
                            route_streams,
                            pending_offer,
                            route_out_tx.clone(),
                            sid.to_string(),
                            id.to_string(),
                        );

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
/// Gap plan §2.4 inbound interception (media-webrtc only).
///
/// Look up the per-route answerer bridge by `env.sid`, extract the
/// `WebRtcSubstrateSetup` from the offer's payload, and apply it (or
/// queue it on the route's `pending_offer` slot if the bridge isn't
/// constructed yet). Always returns; failures are logged. The envelope
/// itself is forwarded to the coordinator unchanged by the caller.
///
/// Also autonomously sends a `connection.answer` envelope on the
/// route's outbound channel so the SDP exchange completes without
/// requiring application code. The answer's SDP is filled in by the
/// outbound mutator below (see [`mutate_connection_answer`]).
#[cfg(feature = "media-webrtc")]
async fn intercept_connection_offer(
    env: &UctpEnvelope,
    routes: &Arc<DashMap<ConnectionId, Route>>,
    by_uctp_sid: &Arc<DashMap<String, ConnectionId>>,
    out_tx: &mpsc::Sender<UctpEnvelope>,
) {
    let Some(sid) = env.sid.clone() else {
        return;
    };
    let Some(connid) = by_uctp_sid.get(&sid).map(|r| r.clone()) else {
        // Race: offer arrived before InboundInvite was processed. The
        // envelope still goes to the coordinator; the coordinator
        // returns an error if no session is known. The per-route
        // queue isn't reachable from here without a route lookup, so
        // we just drop the interception path; the bridge_for variant
        // still works for tests that need precise timing.
        warn!(sid = %sid, "rvoip-websocket: connection.offer arrived before route was registered");
        return;
    };
    let Some(route) = routes.get(&connid).map(|r| r.clone()) else {
        return;
    };

    // Decode the payload. v0 envelopes carry substrate_setup as a JSON
    // value (it may be `{}` for non-webrtc substrates); we only
    // intercept when it deserializes as a WebRtcSubstrateSetup.
    let Ok(payload) = env.decode_payload::<rvoip_uctp::payloads::connection::ConnectionOffer>()
    else {
        return;
    };
    if payload.substrate != "websocket+webrtc" {
        return;
    }
    let Ok(setup) =
        serde_json::from_value::<rvoip_uctp::payloads::connection::WebRtcSubstrateSetup>(
            payload.substrate_setup,
        )
    else {
        return;
    };

    // Apply to the bridge if it's ready; otherwise queue.
    let bridge_opt = route.bridge.lock().clone();
    if let Some(bridge) = bridge_opt {
        if let Err(e) = bridge.set_remote_substrate_setup(setup).await {
            warn!(error = %e, "rvoip-websocket: set_remote_substrate_setup failed");
            return;
        }
    } else {
        *route.pending_offer.lock() = Some(setup);
        debug!(sid = %sid, "rvoip-websocket: queued connection.offer SDP; bridge not ready");
    }

    // Autonomously emit a connection.answer. The payload's
    // substrate_setup is left empty here; the outbound mutator below
    // fills it in once the bridge can produce a local answer.
    let answer = UctpEnvelope::new(
        rvoip_uctp::types::MessageType::ConnectionAnswer,
        serde_json::to_value(rvoip_uctp::payloads::connection::ConnectionAnswer {
            by_participant: "uctp-ws-server".into(),
            substrate: "websocket+webrtc".into(),
            capabilities: serde_json::Value::Object(Default::default()),
            streams_answered: Vec::new(),
            substrate_setup: serde_json::Value::Null,
        })
        .unwrap_or(serde_json::Value::Null),
    )
    .with_sid(sid)
    .with_connid(env.connid.clone().unwrap_or_default())
    .with_in_reply_to(env.id.clone());
    let _ = out_tx.send(answer).await;
}

/// Gap plan §2.4 outbound mutation (media-webrtc only).
///
/// For a `connection.answer` envelope on its way to the wire, populate
/// the payload's `substrate_setup` with the local answerer SDP if it
/// isn't already set. Other envelope types pass through untouched.
#[cfg(feature = "media-webrtc")]
async fn mutate_connection_answer(
    mut env: UctpEnvelope,
    routes: &Arc<DashMap<ConnectionId, Route>>,
    by_uctp_sid: &Arc<DashMap<String, ConnectionId>>,
) -> UctpEnvelope {
    let Some(sid) = env.sid.clone() else {
        return env;
    };
    let Some(connid) = by_uctp_sid.get(&sid).map(|r| r.clone()) else {
        return env;
    };
    let Some(route) = routes.get(&connid).map(|r| r.clone()) else {
        return env;
    };
    let bridge_opt = route.bridge.lock().clone();
    let Some(bridge) = bridge_opt else {
        return env;
    };

    // Only fill substrate_setup when it's null/empty — respect any
    // value an upstream layer already provided.
    let mut payload: rvoip_uctp::payloads::connection::ConnectionAnswer =
        match env.decode_payload() {
            Ok(p) => p,
            Err(_) => return env,
        };
    let already_set = match &payload.substrate_setup {
        serde_json::Value::Null => false,
        serde_json::Value::Object(map) => !map.is_empty(),
        _ => true,
    };
    if already_set {
        return env;
    }

    let setup = match bridge.local_substrate_setup().await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "rvoip-websocket: local_substrate_setup failed; sending answer without SDP");
            return env;
        }
    };
    payload.substrate_setup = match serde_json::to_value(setup) {
        Ok(v) => v,
        Err(_) => return env,
    };
    env.payload = match serde_json::to_value(payload) {
        Ok(v) => v,
        Err(_) => return env,
    };
    env
}

#[cfg(feature = "media-webrtc")]
fn spawn_bridge_setup(
    bridge_slot: Arc<
        parking_lot::Mutex<Option<Arc<crate::media_bridge::WebRtcMediaBridge>>>,
    >,
    route_streams: Arc<DashMap<rvoip_core::ids::StreamId, Arc<dyn rvoip_core::stream::MediaStream>>>,
    pending_offer: Arc<
        parking_lot::Mutex<
            Option<rvoip_uctp::payloads::connection::WebRtcSubstrateSetup>,
        >,
    >,
    route_out_tx: mpsc::Sender<UctpEnvelope>,
    sid: String,
    connid: String,
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

        // Gap plan §4.1 v1 punch-list — outbound trickle ICE pump.
        // Drain locally-gathered ICE candidates and forward them to
        // the peer as `connection.ice-candidate` envelopes until the
        // bridge's local ICE channel closes (gathering complete in
        // non-trickle mode or bridge teardown). Emit the empty-string
        // end-of-candidates marker on exit so the remote knows
        // gathering finished.
        spawn_trickle_ice_pump(
            Arc::clone(&bridge),
            route_out_tx.clone(),
            sid.clone(),
            connid.clone(),
        );

        // Gap plan §2.4 — drain any pending `connection.offer` SDP that
        // arrived before the bridge finished construction.
        let pending = pending_offer.lock().take();
        if let Some(setup) = pending {
            if let Err(e) = bridge.set_remote_substrate_setup(setup).await {
                warn!(error = %e, "rvoip-websocket: failed to apply queued remote offer");
            } else {
                debug!("rvoip-websocket: applied queued connection.offer SDP after bridge ready");
            }
        }

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

/// Gap plan §4.1 v1 punch-list — drains the bridge's
/// `next_local_ice_candidate` channel and forwards each candidate as a
/// `connection.ice-candidate` envelope on the route's outbound channel.
/// On channel close, emits the spec's end-of-candidates marker (empty
/// `candidate` string) and exits.
///
/// The pump is fire-and-forget — when the bridge drops, the underlying
/// `recv_local_ice` channel closes and the loop returns naturally. The
/// route's outbound channel may close first (peer disconnected); we
/// return on send error in that case too.
///
/// Exposed `pub` (with `#[doc(hidden)]`) so integration tests in
/// `tests/trickle_ice.rs` can exercise the pump in isolation against a
/// constructed bridge without spinning the full server.
#[doc(hidden)]
#[cfg(feature = "media-webrtc")]
pub fn spawn_trickle_ice_pump(
    bridge: Arc<crate::media_bridge::WebRtcMediaBridge>,
    out_tx: mpsc::Sender<UctpEnvelope>,
    sid: String,
    connid: String,
) {
    tokio::spawn(async move {
        loop {
            match bridge.next_local_ice_candidate().await {
                Some(init) => {
                    let payload = match serde_json::to_value(&init) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!(error = %e, "rvoip-websocket: trickle ICE: serialize candidate failed");
                            continue;
                        }
                    };
                    let env = UctpEnvelope::new(
                        rvoip_uctp::types::MessageType::ConnectionIceCandidate,
                        payload,
                    )
                    .with_sid(sid.clone())
                    .with_connid(connid.clone());
                    if out_tx.send(env).await.is_err() {
                        debug!(sid = %sid, "rvoip-websocket: trickle ICE: outbound closed; exiting pump");
                        return;
                    }
                }
                None => {
                    // Gathering complete (or bridge closed). Emit the
                    // end-of-candidates marker once, then exit.
                    // Single-stream audio bridges use sdp_mid="0" /
                    // m-line 0 in the answer SDP that
                    // `mutate_connection_answer` produces, so the EoC
                    // marker carries the same identifiers.
                    let eoc = rvoip_uctp::payloads::connection::IceCandidateInit::end_of_candidates("0", 0);
                    let env = UctpEnvelope::new(
                        rvoip_uctp::types::MessageType::ConnectionIceCandidate,
                        serde_json::to_value(&eoc).unwrap_or(serde_json::Value::Null),
                    )
                    .with_sid(sid.clone())
                    .with_connid(connid.clone());
                    let _ = out_tx.send(env).await;
                    debug!(sid = %sid, "rvoip-websocket: trickle ICE: gathering complete; pump exiting");
                    return;
                }
            }
        }
    });
}
