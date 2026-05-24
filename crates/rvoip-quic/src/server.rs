//! `UctpQuicServer` — accept loop that consumes
//! `quinn::Connection`s from the [`rvoip_uctp::substrate::quinn`]
//! dispatcher and spins up one [`rvoip_uctp::state::UctpCoordinator`]
//! per peer.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use rvoip_core::adapter::{AdapterEvent, EndReason};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{
    Connection, ConnectionState, Direction, Transport, TransportHandle,
};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::stream::{MediaStream, MediaStreamHandle, StreamKind};
use rvoip_auth_core::BearerValidator;

use crate::adapter::Route;
use crate::media_stream::QuicDatagramMediaStream;

/// Default audio codec attached to new Connections at `InboundInvite`
/// time. Codec-renegotiation (replace this with the peer's chosen codec
/// after `connection.offer`/`answer`) is v0.x work.
fn default_audio_codec() -> CodecInfo {
    CodecInfo {
        name: "opus".into(),
        clock_rate_hz: 48000,
        channels: 1,
        fmtp: None,
    }
}
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::state::{UctpCoordinator, UctpSessionEvent, ENVELOPE_CHANNEL_CAP};
use rvoip_uctp::substrate::{envelope_reader, envelope_writer};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub struct UctpQuicServer {}

impl UctpQuicServer {
    /// Spawn the accept loop. Returns a handle that owns no state — the
    /// loop owns its own task. Adapter shutdown happens via dropping
    /// the dispatcher channel (`accept_rx`).
    pub fn start(
        mut accept_rx: mpsc::Receiver<quinn::Connection>,
        bearer: Arc<dyn BearerValidator>,
        events_tx: mpsc::Sender<AdapterEvent>,
        by_connection: Arc<DashMap<ConnectionId, String>>,
        by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
        routes: Arc<DashMap<ConnectionId, Route>>,
        _max_concurrent: usize,
        quinn_stats_interval: Duration,
        subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
        orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
        coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
    ) -> Arc<Self> {
        tokio::spawn(async move {
            while let Some(conn) = accept_rx.recv().await {
                let bearer = bearer.clone();
                let events_tx = events_tx.clone();
                let by_connection = Arc::clone(&by_connection);
                let by_uctp_sid = Arc::clone(&by_uctp_sid);
                let routes = Arc::clone(&routes);
                let subscription_handler = subscription_handler.clone();
                let orchestrator = orchestrator.clone();
                let caps = coordinator_caps.clone();
                tokio::spawn(spawn_peer_session(
                    conn,
                    bearer,
                    events_tx,
                    by_connection,
                    by_uctp_sid,
                    routes,
                    quinn_stats_interval,
                    subscription_handler,
                    orchestrator,
                    caps,
                ));
            }
            debug!("rvoip-quic::server: accept loop exiting");
        });
        Arc::new(Self {})
    }
}

/// Construct a synthetic `rvoip_core::Connection` from an inbound
/// `session.invite`. The orchestrator uses this as the "call coming in"
/// handle; transport-level handle wraps the `quinn::Connection` so
/// later adapter-method calls (`end`, `send_message`, …) can resolve
/// the right peer.
fn build_connection(
    quinn_conn: quinn::Connection,
    sid: SessionId,
    from: String,
) -> (ConnectionId, Connection) {
    let id = ConnectionId::new();
    let conn = Connection {
        id: id.clone(),
        session_id: sid,
        participant_id: ParticipantId::from_string(from),
        transport: Transport::Quic,
        direction: Direction::Inbound,
        state: ConnectionState::Connecting,
        capabilities: CapabilityDescriptor::default(),
        negotiated_codecs: NegotiatedCodecs::default(),
        streams: Vec::new(),
        messaging_enabled: false,
        transport_handle: TransportHandle(Arc::new(quinn_conn)),
        opened_at: Utc::now(),
        closed_at: None,
    };
    (id, conn)
}

async fn spawn_peer_session(
    conn: quinn::Connection,
    bearer: Arc<dyn BearerValidator>,
    events_tx: mpsc::Sender<AdapterEvent>,
    by_connection: Arc<DashMap<ConnectionId, String>>,
    by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
    routes: Arc<DashMap<ConnectionId, Route>>,
    quinn_stats_interval: Duration,
    subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
    orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
    coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
) {
    let peer_addr = conn.remote_address();
    info!(%peer_addr, "rvoip-quic: new connection");

    // The bidi stream the peer opens for signaling. The first accept_bi
    // is the signaling stream.
    let (send, recv) = match conn.accept_bi().await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "rvoip-quic: accept_bi failed");
            return;
        }
    };

    let mut reader = Box::pin(envelope_reader(recv));
    let mut writer = Box::pin(envelope_writer(send));

    let (in_tx, in_rx) = mpsc::channel::<UctpEnvelope>(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(ENVELOPE_CHANNEL_CAP);
    let (coord_events_tx, mut coord_events_rx) =
        mpsc::channel::<UctpSessionEvent>(ENVELOPE_CHANNEL_CAP);

    // Per-peer media-stream router. Each `MediaStream` we create for
    // this connection is pushed here, and a single
    // `spawn_datagram_reader` task (started on the first stream) reads
    // incoming QUIC datagrams off this `quinn::Connection`, looks the
    // matching stream up by `stream_local_id`, and forwards into the
    // stream's `inbound_tx`. Without this, the bridge's
    // `frames_in()` end never receives anything from the wire — the
    // outbound pump in `QuicDatagramMediaStream::start` already
    // handles the outgoing side via `conn.send_datagram`.
    let streams_router: Arc<parking_lot::RwLock<Vec<Arc<crate::media_stream::QuicDatagramMediaStream>>>> =
        Arc::new(parking_lot::RwLock::new(Vec::new()));
    let reader_spawned = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Clone the outbound sender BEFORE handing it to the coordinator so
    // the event translator can stash it under each new ConnectionId for
    // the adapter's `accept` / `reject` / `end` / `send_message` methods.
    let route_out_tx = out_tx.clone();

    // If a multi-party SubscriptionHandler was configured (MP2.6+),
    // construct the coordinator via `start_full` so stream.subscribe /
    // stream.unsubscribe envelopes route through it and stream.opened
    // auto-registers publishers. Otherwise the legacy `start` path
    // keeps the v0 503-reject behavior.
    let _coord = match subscription_handler {
        Some(handler) => UctpCoordinator::start_full_with_caps(
            "quic",
            in_rx,
            out_tx,
            coord_events_tx,
            bearer,
            Arc::new(rvoip_uctp::state::default_v0_descriptor()),
            handler,
            coordinator_caps,
        ),
        None => UctpCoordinator::start_full_with_caps(
            "quic",
            in_rx,
            out_tx,
            coord_events_tx,
            bearer,
            Arc::new(rvoip_uctp::state::default_v0_descriptor()),
            rvoip_uctp::state::rejecting_handler(),
            coordinator_caps,
        ),
    };

    // Inbound substrate → coordinator pump.
    let in_tx_for_pump = in_tx.clone();
    let inbound_pump = tokio::spawn(async move {
        while let Some(item) = reader.next().await {
            match item {
                Ok(env) => {
                    if in_tx_for_pump.send(env).await.is_err() {
                        return;
                    }
                }
                Err(e) => {
                    warn!(error = %e, "rvoip-quic: envelope read error");
                    return;
                }
            }
        }
    });

    // Outbound coordinator → substrate pump.
    let outbound_pump = tokio::spawn(async move {
        while let Some(env) = out_rx.recv().await {
            if let Err(e) = writer.send(env).await {
                warn!(error = %e, "rvoip-quic: envelope write error");
                return;
            }
        }
    });

    // Coordinator events → AdapterEvent translator.
    let event_pump = {
        let events_tx = events_tx.clone();
        let conn_for_translator = conn.clone();
        let by_connection = Arc::clone(&by_connection);
        let by_uctp_sid = Arc::clone(&by_uctp_sid);
        let routes = Arc::clone(&routes);
        let route_out_tx = route_out_tx.clone();
        let streams_router = Arc::clone(&streams_router);
        let reader_spawned = Arc::clone(&reader_spawned);
        tokio::spawn(async move {
            // Per-peer auth state. Set by `UctpSessionEvent::Authenticated`
            // (the coordinator's signal that the bearer handshake passed);
            // consumed by each subsequent `InboundInvite` so the synthetic
            // follow-up `AdapterEvent::Authenticated` it emits carries the
            // identity_id / participant_id / assurance triple tied to the
            // just-created Connection. See plan §7 G1 / A3.
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
                        // Native event preserved for adapter-level consumers
                        // (loopback tests, anything that subscribes directly
                        // to the adapter) that already watch for it.
                        Some(AdapterEvent::Native {
                            kind: "uctp.authenticated",
                            detail: "bearer".into(),
                        })
                    }
                    UctpSessionEvent::InboundInvite { sid, from, .. } => {
                        let (id, mut connection) =
                            build_connection(conn_for_translator.clone(), sid.clone(), from);
                        // Spin up a default audio stream so the orchestrator's
                        // `bridge_connections` finds something to bridge. v0
                        // uses `stream_local_id = 1` (first slot per
                        // CONVERSATION_PROTOCOL.md §10.1) and the Opus default
                        // codec; a future codec-renegotiation pass replaces
                        // this stream when the peer's `connection.offer`
                        // arrives.
                        let stream = QuicDatagramMediaStream::start(
                            StreamId::new(),
                            StreamKind::Audio,
                            default_audio_codec(),
                            Direction::Inbound,
                            1,
                            conn_for_translator.clone(),
                        );
                        // Register with the per-peer datagram-reader router
                        // BEFORE inserting into the connection-level map,
                        // so the reader (started below on first call) sees
                        // the stream.
                        streams_router.write().push(stream.clone());
                        if !reader_spawned.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            // Build the fanout context if an orchestrator
                            // is plumbed in (MP3b). The publisher is *this*
                            // connection; sid is the one we just learned
                            // from the inbound invite.
                            let fanout = orchestrator
                                .as_ref()
                                .map(|orch| crate::media_stream::FanoutContext {
                                    orchestrator: Arc::clone(orch),
                                    sid: sid.clone(),
                                    publisher_connid: id.clone(),
                                });
                            crate::media_stream::spawn_datagram_reader(
                                conn_for_translator.clone(),
                                Arc::clone(&streams_router),
                                fanout,
                            );
                        }
                        let stream_dyn: Arc<dyn MediaStream> = stream.clone();
                        connection
                            .streams
                            .push(MediaStreamHandle::new(stream_dyn.clone()));
                        let route_streams: Arc<DashMap<StreamId, Arc<dyn MediaStream>>> =
                            Arc::new(DashMap::new());
                        route_streams.insert(stream.id(), stream_dyn);
                        by_connection.insert(id.clone(), sid.to_string());
                        by_uctp_sid.insert(sid.to_string(), id.clone());
                        routes.insert(
                            id.clone(),
                            Route {
                                sid: sid.to_string(),
                                out_tx: route_out_tx.clone(),
                                streams: route_streams,
                                conn: conn_for_translator.clone(),
                                // Default audio stream claims local_id=1
                                // (see QuicDatagramMediaStream::start
                                // above); the allocator hands out 2,
                                // 3, ... for subsequent per-subscriber
                                // streams.
                                next_local_id: Arc::new(
                                    std::sync::atomic::AtomicU16::new(2),
                                ),
                                streams_router: Arc::clone(&streams_router),
                            },
                        );
                        // Send InboundConnection first so consumers
                        // creating a session see the Connection before
                        // the auth follow-up arrives.
                        let _ = events_tx
                            .send(AdapterEvent::InboundConnection { connection })
                            .await;
                        // Pair with a typed Authenticated event if we
                        // captured auth state earlier. A peer that
                        // somehow reached InboundInvite without auth
                        // (shouldn't happen post-A1, but be defensive)
                        // simply doesn't get the follow-up — the
                        // orchestrator sees the bare InboundConnection.
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
                        // Already sent both — skip the trailing send.
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
                        Some(AdapterEvent::Connected { connection_id: connid })
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
                                routes.remove(&connection_id);
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
                    UctpSessionEvent::ConnectionOpened { connid, .. }
                    | UctpSessionEvent::MediaFrame { connid, .. } => Some(AdapterEvent::Native {
                        kind: "uctp.internal",
                        detail: connid.to_string(),
                    }),
                    UctpSessionEvent::NegotiationFailed { sid, reason } => {
                        Some(AdapterEvent::Native {
                            kind: "uctp.negotiation_failed",
                            detail: format!("sid={} reason={}", sid, reason),
                        })
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
                };
                if let Some(ev) = adapter_event {
                    let _ = events_tx.send(ev).await;
                }
            }
        })
    };

    // Periodic quinn stats sampler. Lives in rvoip-uctp so QUIC and
    // WT adapters emit identical metric series for per-transport
    // comparison.
    let stats_pump = rvoip_uctp::substrate::spawn_stats_sampler(
        conn.clone(),
        "quic",
        quinn_stats_interval,
    );

    let _ = inbound_pump.await;
    let _ = outbound_pump.await;
    let _ = event_pump.await;
    stats_pump.abort();

    info!(%peer_addr, "rvoip-quic: connection closed");
}
