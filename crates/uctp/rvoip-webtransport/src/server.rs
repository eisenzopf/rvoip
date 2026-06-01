//! `UctpWtServer` — wraps a `quinn::Connection` accepted via ALPN `h3`,
//! drives the HTTP/3 + extended `CONNECT` upgrade to a
//! `web_transport_quinn::Session`, and spawns one
//! `rvoip_uctp::state::UctpCoordinator` per peer.

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
use crate::media_stream::WebTransportDatagramMediaStream;

/// Default audio codec attached to new Connections at `InboundInvite`
/// time. Codec-renegotiation is v0.x work.
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

pub struct UctpWtServer {}

impl UctpWtServer {
    pub(crate) fn start(
        mut accept_rx: mpsc::Receiver<quinn::Connection>,
        bearer: Arc<dyn BearerValidator>,
        events_tx: mpsc::Sender<AdapterEvent>,
        by_connection: Arc<DashMap<ConnectionId, String>>,
        by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
        routes: Arc<DashMap<ConnectionId, Route>>,
        _max_concurrent: usize,
        mount_path: String,
        quinn_stats_interval: Duration,
        subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
        orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
        coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
    ) -> Arc<Self> {
        tokio::spawn(async move {
            while let Some(conn) = accept_rx.recv().await {
                let bearer = bearer.clone();
                let events_tx = events_tx.clone();
                let mount_path = mount_path.clone();
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
                    mount_path,
                    quinn_stats_interval,
                    subscription_handler,
                    orchestrator,
                    caps,
                ));
            }
            debug!("rvoip-webtransport::server: accept loop exiting");
        });
        Arc::new(Self {})
    }
}

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
        transport: Transport::WebTransport,
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
    mount_path: String,
    quinn_stats_interval: Duration,
    subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
    orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
    coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
) {
    let peer_addr = conn.remote_address();
    info!(%peer_addr, "rvoip-webtransport: new connection");

    // HTTP/3 + extended-CONNECT upgrade.
    let request = match web_transport_quinn::Request::accept(conn.clone()).await {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "rvoip-webtransport: wt upgrade rejected");
            return;
        }
    };

    // web-transport-quinn 0.11+: `url` is a field, not a method.
    // The graceful `close(StatusCode)` API was removed; for now we
    // drop the request on path mismatch, which manifests as a
    // closed CONNECT stream on the client side.
    if !request.url.path().eq(&mount_path) {
        warn!(
            requested = %request.url.path(),
            expected = %mount_path,
            "rvoip-webtransport: mount path mismatch; closing"
        );
        return;
    }

    let session = match request.ok().await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "rvoip-webtransport: failed to confirm WT session");
            return;
        }
    };

    let (send, recv) = match session.accept_bi().await {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "rvoip-webtransport: accept_bi failed");
            return;
        }
    };

    let mut reader = Box::pin(envelope_reader(recv));
    let mut writer = Box::pin(envelope_writer(send));

    let (in_tx, in_rx) = mpsc::channel::<UctpEnvelope>(ENVELOPE_CHANNEL_CAP);
    let (out_tx, mut out_rx) = mpsc::channel::<UctpEnvelope>(ENVELOPE_CHANNEL_CAP);
    let (coord_events_tx, mut coord_events_rx) =
        mpsc::channel::<UctpSessionEvent>(ENVELOPE_CHANNEL_CAP);

    let route_out_tx = out_tx.clone();

    // Per-peer media-stream router + first-call-spawns-reader semantics.
    // Parallel to rvoip-quic/src/server.rs — without this the bridge's
    // `frames_in()` end never receives anything from the wire.
    let streams_router: Arc<
        parking_lot::RwLock<Vec<Arc<crate::media_stream::WebTransportDatagramMediaStream>>>,
    > = Arc::new(parking_lot::RwLock::new(Vec::new()));
    let reader_spawned = Arc::new(std::sync::atomic::AtomicBool::new(false));

    let _coord = match subscription_handler {
        Some(handler) => UctpCoordinator::start_full_with_caps(
            "webtransport",
            in_rx,
            out_tx,
            coord_events_tx,
            bearer,
            Arc::new(rvoip_uctp::state::default_v0_descriptor()),
            handler,
            coordinator_caps,
        ),
        None => UctpCoordinator::start_full_with_caps(
            "webtransport",
            in_rx,
            out_tx,
            coord_events_tx,
            bearer,
            Arc::new(rvoip_uctp::state::default_v0_descriptor()),
            rvoip_uctp::state::rejecting_handler(),
            coordinator_caps,
        ),
    };
    // Gap plan §4.2 v1 punch list — capture the coordinator's
    // `Pending` correlator so per-Route adapter code can await
    // typed responses.
    let pending = _coord.pending();

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
                    warn!(error = %e, "rvoip-webtransport: envelope read error");
                    return;
                }
            }
        }
    });

    let outbound_pump = tokio::spawn(async move {
        while let Some(env) = out_rx.recv().await {
            if let Err(e) = writer.send(env).await {
                warn!(error = %e, "rvoip-webtransport: envelope write error");
                return;
            }
        }
    });

    let event_pump = {
        let events_tx = events_tx.clone();
        let conn_for_translator = conn.clone();
        let session_for_translator = session.clone();
        let by_connection = Arc::clone(&by_connection);
        let by_uctp_sid = Arc::clone(&by_uctp_sid);
        let routes = Arc::clone(&routes);
        let route_out_tx = route_out_tx.clone();
        let streams_router = Arc::clone(&streams_router);
        let reader_spawned = Arc::clone(&reader_spawned);
        tokio::spawn(async move {
            // Per-peer auth state; consumed by the InboundInvite arm to
            // emit a synthetic `AdapterEvent::Authenticated` carrying
            // the just-created Connection's id. Plan §7 G1 / A3.
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
                        let (id, mut connection) =
                            build_connection(conn_for_translator.clone(), sid.clone(), from);
                        // Default audio stream — see rvoip-quic/server.rs for
                        // the rationale on `InboundInvite`-time creation +
                        // `stream_local_id = 1`. Codec replacement on
                        // negotiation lands in v0.x.
                        let stream = WebTransportDatagramMediaStream::start(
                            StreamId::new(),
                            StreamKind::Audio,
                            default_audio_codec(),
                            Direction::Inbound,
                            1,
                            session_for_translator.clone(),
                        );
                        streams_router.write().push(stream.clone());
                        if !reader_spawned.swap(true, std::sync::atomic::Ordering::SeqCst) {
                            let fanout = orchestrator
                                .as_ref()
                                .map(|orch| crate::media_stream::FanoutContext {
                                    orchestrator: Arc::clone(orch),
                                    sid: sid.clone(),
                                    publisher_connid: id.clone(),
                                });
                            crate::media_stream::spawn_datagram_reader(
                                session_for_translator.clone(),
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
                                pending: Arc::clone(&pending),
                                streams: route_streams,
                                session: session_for_translator.clone(),
                                next_local_id: Arc::new(
                                    std::sync::atomic::AtomicU16::new(2),
                                ),
                                streams_router: Arc::clone(&streams_router),
                            },
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
                    UctpSessionEvent::StepUpResponse {
                        connid,
                        method,
                        credential,
                    } => connid.map(|c| AdapterEvent::StepUpResponse {
                        connection_id: c,
                        method,
                        credential,
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

    // Periodic quinn stats sampler. Shared helper in rvoip-uctp so the
    // QUIC and WT adapters emit identical metric series.
    let stats_pump = rvoip_uctp::substrate::spawn_stats_sampler(
        conn.clone(),
        "webtransport",
        quinn_stats_interval,
    );

    let _ = inbound_pump.await;
    let _ = outbound_pump.await;
    let _ = event_pump.await;
    stats_pump.abort();

    info!(%peer_addr, "rvoip-webtransport: connection closed");
}
