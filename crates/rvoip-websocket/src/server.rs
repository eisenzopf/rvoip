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
                tokio::spawn(async move {
                    let ws = match tokio_tungstenite::accept_async(tcp).await {
                        Ok(ws) => ws,
                        Err(e) => {
                            warn!(error = %e, %peer_addr, "rvoip-websocket: handshake failed");
                            return;
                        }
                    };
                    info!(%peer_addr, "rvoip-websocket: peer connected");
                    spawn_peer_session(ws, bearer, events_tx, by_connection, by_uctp_sid, routes)
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
) {
    let (mut sink, mut stream) = ws.split();

    let (in_tx, in_rx) = mpsc::channel::<UctpEnvelope>(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(ENVELOPE_CHANNEL_CAP);
    let (coord_events_tx, mut coord_events_rx) =
        mpsc::channel::<UctpSessionEvent>(ENVELOPE_CHANNEL_CAP);

    let route_out_tx = out_tx.clone();

    let _coord = UctpCoordinator::start("websocket", in_rx, out_tx, coord_events_tx, bearer);

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
            while let Some(event) = coord_events_rx.recv().await {
                let adapter_event = match event {
                    UctpSessionEvent::Authenticated { .. } => AdapterEvent::Native {
                        kind: "uctp.authenticated",
                        detail: "bearer".into(),
                    },
                    UctpSessionEvent::InboundInvite { sid, from, .. } => {
                        let (id, connection) = build_connection(sid.clone(), from);
                        // MediaStream population intentionally deferred for
                        // WebSocket: media rides a co-located WebRTC
                        // PeerConnection (see media_bridge.rs, stubbed
                        // pending webrtc-rs stable release). `Route.streams`
                        // and `Connection.streams` stay empty here; bridges
                        // to WS connections will return
                        // `AdmissionRejected("no audio stream")` until the
                        // WebRTC integration lands. The QUIC + WT adapters
                        // create default audio streams at this point — see
                        // their `server.rs` for the pattern to mirror once
                        // webrtc-rs is ready.
                        by_connection.insert(id.clone(), sid.to_string());
                        by_uctp_sid.insert(sid.to_string(), id.clone());
                        routes.insert(
                            id.clone(),
                            Route {
                                sid: sid.to_string(),
                                out_tx: route_out_tx.clone(),
                                streams: Arc::new(DashMap::new()),
                            },
                        );
                        AdapterEvent::InboundConnection { connection }
                    }
                    UctpSessionEvent::SessionConnected { sid } => {
                        match by_uctp_sid.get(sid.as_str()).map(|r| r.clone()) {
                            Some(connection_id) => AdapterEvent::Connected { connection_id },
                            None => AdapterEvent::Native {
                                kind: "uctp.session_connected_orphan",
                                detail: sid.to_string(),
                            },
                        }
                    }
                    UctpSessionEvent::ConnectionConnected { connid, .. } => {
                        AdapterEvent::Connected {
                            connection_id: connid,
                        }
                    }
                    UctpSessionEvent::ConnectionEnded { connid, reason, .. } => {
                        AdapterEvent::Ended {
                            connection_id: connid,
                            reason: EndReason::Failed { detail: reason },
                        }
                    }
                    UctpSessionEvent::SessionEnded { sid, reason } => {
                        match by_uctp_sid.remove(sid.as_str()) {
                            Some((_, connection_id)) => {
                                by_connection.remove(&connection_id);
                                routes.remove(&connection_id);
                                AdapterEvent::Ended {
                                    connection_id,
                                    reason: if reason == "cancelled" {
                                        EndReason::Cancelled
                                    } else {
                                        EndReason::Normal
                                    },
                                }
                            }
                            None => AdapterEvent::Native {
                                kind: "uctp.session_ended_orphan",
                                detail: format!("sid={} reason={}", sid, reason),
                            },
                        }
                    }
                    other => AdapterEvent::Native {
                        kind: "uctp.internal",
                        detail: format!("{:?}", other),
                    },
                };
                let _ = events_tx.send(adapter_event).await;
            }
        })
    };

    let _ = inbound_pump.await;
    let _ = outbound_pump.await;
    let _ = event_pump.await;
}
