//! `UctpQuicServer` — accept loop that consumes
//! `quinn::Connection`s from the [`rvoip_uctp::substrate::quinn`]
//! dispatcher and spins up one [`rvoip_uctp::state::UctpCoordinator`]
//! per peer.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use rvoip_auth_core::BearerValidator;
use rvoip_core::adapter::{AdapterEvent, AdapterLifecycleSinkSlot, EndReason, TerminalDelivery};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::stream::{MediaStream, MediaStreamHandle, StreamKind};

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
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub struct UctpQuicServer {}

impl UctpQuicServer {
    /// Spawn the accept loop. Returns a handle that owns no state — the
    /// loop owns its own task. Adapter shutdown happens via dropping
    /// the dispatcher channel (`accept_rx`).
    pub(crate) fn start(
        mut accept_rx: mpsc::Receiver<quinn::Connection>,
        bearer: Arc<dyn BearerValidator>,
        events_tx: mpsc::Sender<AdapterEvent>,
        lifecycle_sink: AdapterLifecycleSinkSlot,
        by_connection: Arc<DashMap<ConnectionId, String>>,
        by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
        routes: Arc<DashMap<ConnectionId, Route>>,
        max_concurrent: usize,
        quinn_stats_interval: Duration,
        subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
        orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
        coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
        sig9421: Option<rvoip_uctp::state::Sig9421Config>,
    ) -> Arc<Self> {
        tokio::spawn(async move {
            let connection_slots = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
            while let Some(conn) = accept_rx.recv().await {
                let permit = match Arc::clone(&connection_slots).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        metrics::counter!(
                            "uctp_connections_rejected_total",
                            "transport" => "quic",
                            "reason" => "capacity"
                        )
                        .increment(1);
                        conn.close(
                            quinn::VarInt::from_u32(0x100),
                            b"connection capacity reached",
                        );
                        continue;
                    }
                };
                let bearer = bearer.clone();
                let events_tx = events_tx.clone();
                let lifecycle_sink = lifecycle_sink.clone();
                let by_connection = Arc::clone(&by_connection);
                let by_uctp_sid = Arc::clone(&by_uctp_sid);
                let routes = Arc::clone(&routes);
                let subscription_handler = subscription_handler.clone();
                let orchestrator = orchestrator.clone();
                let caps = coordinator_caps.clone();
                let sig9421 = sig9421.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    metrics::gauge!("uctp_active_connections", "transport" => "quic")
                        .increment(1.0);
                    spawn_peer_session(
                        conn,
                        bearer,
                        events_tx,
                        lifecycle_sink,
                        by_connection,
                        by_uctp_sid,
                        routes,
                        quinn_stats_interval,
                        subscription_handler,
                        orchestrator,
                        caps,
                        sig9421,
                    )
                    .await;
                    metrics::gauge!("uctp_active_connections", "transport" => "quic")
                        .decrement(1.0);
                });
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
    lifecycle_sink: AdapterLifecycleSinkSlot,
    by_connection: Arc<DashMap<ConnectionId, String>>,
    by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
    routes: Arc<DashMap<ConnectionId, Route>>,
    quinn_stats_interval: Duration,
    subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
    orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
    coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
    sig9421: Option<rvoip_uctp::state::Sig9421Config>,
) {
    // Wire Session IDs are peer-controlled and need only be unique within one
    // authenticated substrate peer. Never resolve them through the adapter-
    // global map, where another tenant could choose the same value.
    let _adapter_global_sid_index = by_uctp_sid;
    let by_uctp_sid: Arc<DashMap<String, ConnectionId>> = Arc::new(DashMap::new());
    let peer_addr = conn.remote_address();
    info!(%peer_addr, "rvoip-quic: new connection");

    // The bidi stream the peer opens for signaling. The first accept_bi
    // is the signaling stream.
    let authentication_deadline = coordinator_caps.authentication_deadline;
    let (send, recv) = match tokio::time::timeout(authentication_deadline, conn.accept_bi()).await {
        Ok(Ok(streams)) => streams,
        Ok(Err(e)) => {
            warn!(error = %e, "rvoip-quic: accept_bi failed");
            return;
        }
        Err(_) => {
            warn!(%peer_addr, "rvoip-quic: signaling stream setup timed out");
            conn.close(quinn::VarInt::from_u32(0x102), b"signaling setup timeout");
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
    let streams_router: Arc<
        parking_lot::RwLock<Vec<Arc<crate::media_stream::QuicDatagramMediaStream>>>,
    > = Arc::new(parking_lot::RwLock::new(Vec::new()));
    let reader_spawned = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let media_cancel = CancellationToken::new();
    let media_reader = Arc::new(parking_lot::Mutex::new(None));

    // Clone the outbound sender BEFORE handing it to the coordinator so
    // the event translator can stash it under each new ConnectionId for
    // the adapter's `accept` / `reject` / `end` / `send_message` methods.
    let route_out_tx = out_tx.clone();

    // If a multi-party SubscriptionHandler was configured (MP2.6+),
    // construct the coordinator via `start_full` so stream.subscribe /
    // stream.unsubscribe envelopes route through it and stream.opened
    // auto-registers publishers. Otherwise the legacy `start` path
    // keeps the v0 503-reject behavior.
    let subscription_handler =
        subscription_handler.unwrap_or_else(|| rvoip_uctp::state::rejecting_handler());
    let subscription_handler: Arc<dyn rvoip_uctp::state::SubscriptionHandler> =
        rvoip_uctp::state::NamespacedSubscriptionHandler::new(
            ConnectionId::new().to_string(),
            subscription_handler,
        );
    let drain_grace = coordinator_caps.signaling_send_timeout;
    let coord = if let Some(sig9421) = sig9421 {
        UctpCoordinator::start_full_with_sig9421(
            "quic",
            in_rx,
            out_tx,
            coord_events_tx,
            bearer,
            sig9421.verifier,
            sig9421.policy,
            Arc::new(rvoip_uctp::state::default_v0_descriptor()),
            subscription_handler,
            coordinator_caps,
        )
    } else {
        UctpCoordinator::start_full_with_caps(
            "quic",
            in_rx,
            out_tx,
            coord_events_tx,
            bearer,
            Arc::new(rvoip_uctp::state::default_v0_descriptor()),
            subscription_handler,
            coordinator_caps,
        )
    };
    // Gap plan §4.2 v1 punch list — capture the coordinator's
    // `Pending` correlator so per-Route adapter code can await
    // typed responses (`renegotiate_media`, future correlated
    // ops). Cloned into every `Route` built below.
    let pending = coord.pending();
    let auth_guard =
        rvoip_uctp::state::spawn_auth_lifecycle_guard(Arc::clone(&coord), authentication_deadline);

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
    // The pump owns the sole ingress sender. Dropping this local copy ensures
    // EOF on the substrate closes the coordinator input instead of leaving a
    // hidden sender alive for the remainder of the peer task.
    drop(in_tx);

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
        let media_cancel = media_cancel.clone();
        let media_reader = Arc::clone(&media_reader);
        let coord_for_translator = Arc::clone(&coord);
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
                Option<rvoip_core::identity::AuthenticatedPrincipal>,
            )> = None;
            let mut wire_to_core = std::collections::HashMap::<ConnectionId, ConnectionId>::new();

            while let Some(event) = coord_events_rx.recv().await {
                let adapter_event: Option<AdapterEvent> = match event {
                    UctpSessionEvent::Authenticated {
                        identity_id,
                        participant_id,
                        assurance,
                    } => {
                        latest_auth = Some((
                            identity_id,
                            participant_id,
                            assurance,
                            coord_for_translator.authenticated_principal(),
                        ));
                        // Native event preserved for adapter-level consumers
                        // (loopback tests, anything that subscribes directly
                        // to the adapter) that already watch for it.
                        Some(AdapterEvent::Native {
                            kind: "uctp.authenticated",
                            detail: "bearer".into(),
                        })
                    }
                    UctpSessionEvent::InboundInvite { sid, from, .. } => {
                        let Some(principal) = coord_for_translator.authenticated_principal() else {
                            warn!(sid = %sid, "authenticated invite missing retained principal; refusing route");
                            continue;
                        };
                        let (id, mut connection) =
                            build_connection(conn_for_translator.clone(), sid.clone(), from);
                        // Spin up a default audio stream so the orchestrator's
                        // `bridge_connections` finds something to bridge. v0
                        // uses `stream_local_id = 1` (first slot per
                        // CONVERSATION_PROTOCOL.md §10.1) and the Opus default
                        // codec; a future codec-renegotiation pass replaces
                        // this stream when the peer's `connection.offer`
                        // arrives.
                        let route_cancel = media_cancel.child_token();
                        let stream = QuicDatagramMediaStream::start_with_cancel(
                            StreamId::new(),
                            StreamKind::Audio,
                            default_audio_codec(),
                            Direction::Inbound,
                            1,
                            conn_for_translator.clone(),
                            route_cancel.clone(),
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
                            let fanout = orchestrator.as_ref().map(|orch| {
                                crate::media_stream::FanoutContext {
                                    orchestrator: Arc::clone(orch),
                                    sid: sid.clone(),
                                    publisher_connid: id.clone(),
                                }
                            });
                            let reader = crate::media_stream::spawn_datagram_reader_with_cancel(
                                conn_for_translator.clone(),
                                Arc::clone(&streams_router),
                                fanout,
                                media_cancel.clone(),
                            );
                            *media_reader.lock() = Some(reader);
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
                                binding: rvoip_uctp::adapter_helpers::AuthenticatedConnectionBinding::new(&principal),
                                out_tx: route_out_tx.clone(),
                                pending: Arc::clone(&pending),
                                streams: route_streams,
                                conn: conn_for_translator.clone(),
                                // Default audio stream claims local_id=1
                                // (see QuicDatagramMediaStream::start
                                // above); the allocator hands out 2,
                                // 3, ... for subsequent per-subscriber
                                // streams.
                                next_local_id: Arc::new(std::sync::atomic::AtomicU16::new(2)),
                                streams_router: Arc::clone(&streams_router),
                                route_cancel,
                            },
                        );
                        // Send InboundConnection first so consumers
                        // creating a session see the Connection before
                        // the auth follow-up arrives.
                        if !rvoip_uctp::state::try_deliver_adapter_event(
                            &events_tx,
                            AdapterEvent::InboundConnection { connection },
                            "quic",
                        ) {
                            break;
                        }
                        // Pair with a typed Authenticated event if we
                        // captured auth state earlier. A peer that
                        // somehow reached InboundInvite without auth
                        // (shouldn't happen post-A1, but be defensive)
                        // simply doesn't get the follow-up — the
                        // orchestrator sees the bare InboundConnection.
                        if let Some((identity_id, participant_id, assurance, principal)) =
                            latest_auth.clone()
                        {
                            let event = match principal {
                                Some(principal) => AdapterEvent::PrincipalAuthenticated {
                                    connection_id: id,
                                    participant_id,
                                    principal,
                                },
                                None => AdapterEvent::Authenticated {
                                    connection_id: id,
                                    identity_id,
                                    participant_id,
                                    assurance,
                                },
                            };
                            if !rvoip_uctp::state::try_deliver_adapter_event(
                                &events_tx, event, "quic",
                            ) {
                                break;
                            }
                        }
                        // Already sent both — skip the trailing send.
                        None
                    }
                    UctpSessionEvent::SessionConnected { sid } => {
                        match by_uctp_sid.get(sid.as_str()).map(|r| r.clone()) {
                            Some(connection_id) => Some(AdapterEvent::Connected { connection_id }),
                            None => Some(AdapterEvent::Native {
                                kind: "uctp.session_connected_orphan",
                                detail: sid.to_string(),
                            }),
                        }
                    }
                    UctpSessionEvent::ConnectionConnected { connid, .. } => wire_to_core
                        .get(&connid)
                        .cloned()
                        .map(|connection_id| AdapterEvent::Connected { connection_id })
                        .or_else(|| {
                            Some(AdapterEvent::Native {
                                kind: "uctp.connection_connected_orphan",
                                detail: connid.to_string(),
                            })
                        }),
                    UctpSessionEvent::ConnectionEnded { connid, reason, .. } => {
                        match wire_to_core.get(&connid).cloned() {
                            Some(connection_id) => {
                                let has_sibling = wire_to_core
                                    .iter()
                                    .any(|(wire, core)| wire != &connid && core == &connection_id);
                                if has_sibling {
                                    wire_to_core.remove(&connid);
                                    Some(AdapterEvent::Native {
                                        kind: "uctp.connection_ended",
                                        detail: format!("connid={connid} reason={reason}"),
                                    })
                                } else {
                                    let terminal = AdapterEvent::Ended {
                                        connection_id: connection_id.clone(),
                                        reason: EndReason::Failed { detail: reason },
                                    };
                                    if events_tx.try_send(terminal).is_err() {
                                        warn!(%connid, "terminal adapter event backpressured; preserving route for peer cleanup");
                                        break;
                                    }
                                    wire_to_core.remove(&connid);
                                    let sid = by_connection
                                        .get(&connection_id)
                                        .map(|entry| entry.clone());
                                    by_connection.remove(&connection_id);
                                    if let Some(sid) = sid {
                                        if by_uctp_sid
                                            .get(&sid)
                                            .is_some_and(|mapped| *mapped == connection_id)
                                        {
                                            by_uctp_sid.remove(&sid);
                                        }
                                    }
                                    if let Some((_, route)) = routes.remove(&connection_id) {
                                        route.route_cancel.cancel();
                                        let streams = route
                                            .streams
                                            .iter()
                                            .map(|entry| entry.value().clone())
                                            .collect::<Vec<_>>();
                                        let stream_ids = streams
                                            .iter()
                                            .map(|stream| stream.id())
                                            .collect::<std::collections::HashSet<_>>();
                                        streams_router
                                            .write()
                                            .retain(|stream| !stream_ids.contains(&stream.id()));
                                        for stream in streams {
                                            let _ = stream.close().await;
                                        }
                                    }
                                    None
                                }
                            }
                            None => Some(AdapterEvent::Native {
                                kind: "uctp.connection_ended_orphan",
                                detail: connid.to_string(),
                            }),
                        }
                    }
                    UctpSessionEvent::SessionEnded { sid, reason } => {
                        match by_uctp_sid.get(sid.as_str()).map(|entry| entry.clone()) {
                            Some(connection_id) => {
                                let terminal = AdapterEvent::Ended {
                                    connection_id: connection_id.clone(),
                                    reason: if reason == "cancelled" {
                                        EndReason::Cancelled
                                    } else {
                                        EndReason::Normal
                                    },
                                };
                                if events_tx.try_send(terminal).is_err() {
                                    warn!(%sid, "terminal adapter event backpressured; preserving route for peer cleanup");
                                    break;
                                }
                                wire_to_core.retain(|_, core| core != &connection_id);
                                by_connection.remove(&connection_id);
                                by_uctp_sid.remove(sid.as_str());
                                if let Some((_, route)) = routes.remove(&connection_id) {
                                    route.route_cancel.cancel();
                                    let streams = route
                                        .streams
                                        .iter()
                                        .map(|entry| entry.value().clone())
                                        .collect::<Vec<_>>();
                                    let stream_ids = streams
                                        .iter()
                                        .map(|stream| stream.id())
                                        .collect::<std::collections::HashSet<_>>();
                                    streams_router
                                        .write()
                                        .retain(|stream| !stream_ids.contains(&stream.id()));
                                    for stream in streams {
                                        let _ = stream.close().await;
                                    }
                                }
                                None
                            }
                            None => Some(AdapterEvent::Native {
                                kind: "uctp.session_ended_orphan",
                                detail: format!("sid={} reason={}", sid, reason),
                            }),
                        }
                    }
                    UctpSessionEvent::ConnectionOpened { sid, connid, .. } => {
                        let core_connection_id =
                            by_uctp_sid.get(sid.as_str()).map(|entry| entry.clone());
                        let principal = coord_for_translator.authenticated_principal();
                        match (core_connection_id, principal) {
                            (Some(core_connection_id), Some(principal)) => {
                                let binding = routes
                                    .get(&core_connection_id)
                                    .map(|route| route.binding.clone());
                                match binding {
                                    Some(binding) => match wire_to_core.get(&connid) {
                                        Some(existing) if existing != &core_connection_id => {
                                            warn!(wire_connid = %connid, existing_core = %existing, attempted_core = %core_connection_id, "wire connection ID already belongs to another route");
                                            Some(AdapterEvent::Native {
                                                kind: "uctp.connection_binding_rejected",
                                                detail: connid.to_string(),
                                            })
                                        }
                                        _ => match binding
                                            .bind_wire_connection(&principal, connid.clone())
                                        {
                                            Ok(()) => {
                                                wire_to_core
                                                    .insert(connid.clone(), core_connection_id);
                                                Some(AdapterEvent::Native {
                                                    kind: "uctp.connection_bound",
                                                    detail: connid.to_string(),
                                                })
                                            }
                                            Err(error) => {
                                                warn!(wire_connid = %connid, error = %error, "refusing UCTP connection binding");
                                                Some(AdapterEvent::Native {
                                                    kind: "uctp.connection_binding_rejected",
                                                    detail: connid.to_string(),
                                                })
                                            }
                                        },
                                    },
                                    None => Some(AdapterEvent::Native {
                                        kind: "uctp.connection_opened_orphan",
                                        detail: connid.to_string(),
                                    }),
                                }
                            }
                            _ => Some(AdapterEvent::Native {
                                kind: "uctp.connection_opened_orphan",
                                detail: connid.to_string(),
                            }),
                        }
                    }
                    UctpSessionEvent::MediaFrame { connid, .. } => Some(AdapterEvent::Native {
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
                    } => {
                        wire_to_core
                            .get(&connid)
                            .cloned()
                            .map(|connection_id| AdapterEvent::Dtmf {
                                connection_id,
                                digits,
                                duration_ms,
                            })
                    }
                    UctpSessionEvent::DataMessage { connid, message } => wire_to_core
                        .get(&connid)
                        .cloned()
                        .map(|connection_id| AdapterEvent::DataMessage {
                            connection_id,
                            message,
                        }),
                    UctpSessionEvent::Quality {
                        connid,
                        strm_id: _,
                        snapshot,
                        rtt_ms: _,
                        bitrate_bps: _,
                    } => wire_to_core.get(&connid).cloned().map(|connection_id| {
                        AdapterEvent::Quality {
                            connection_id,
                            snapshot,
                        }
                    }),
                    UctpSessionEvent::StepUpResponse {
                        connid,
                        method,
                        credential,
                    } => connid.and_then(|wire_connection_id| {
                        wire_to_core
                            .get(&wire_connection_id)
                            .cloned()
                            .map(|connection_id| AdapterEvent::StepUpResponse {
                                connection_id,
                                method,
                                credential,
                            })
                    }),
                    _ => Some(AdapterEvent::Native {
                        kind: "uctp.internal",
                        detail: "unmapped UCTP session event".into(),
                    }),
                };
                if let Some(ev) = adapter_event {
                    if !rvoip_uctp::state::try_deliver_adapter_event(&events_tx, ev, "quic") {
                        break;
                    }
                }
            }
        })
    };

    // Periodic quinn stats sampler. Lives in rvoip-uctp so QUIC and
    // WT adapters emit identical metric series for per-transport
    // comparison.
    let stats_pump =
        rvoip_uctp::substrate::spawn_stats_sampler(conn.clone(), "quic", quinn_stats_interval);
    let media_guard_token = media_cancel.clone();
    let media_guard = tokio::spawn(async move {
        media_guard_token.cancelled().await;
    });

    let _ = rvoip_uctp::state::supervise_peer_tasks_with_media_cancel(
        Arc::clone(&coord),
        vec![
            inbound_pump,
            outbound_pump,
            event_pump,
            auth_guard,
            media_guard,
        ],
        drain_grace,
        media_cancel.clone(),
    )
    .await;
    media_cancel.cancel();
    conn.close(quinn::VarInt::from_u32(0), b"UCTP peer session ended");
    let media_reader_task = media_reader.lock().take();
    if let Some(mut task) = media_reader_task {
        if tokio::time::timeout(drain_grace, &mut task).await.is_err() {
            task.abort();
            let _ = task.await;
        }
    }
    let media_streams = streams_router.write().drain(..).collect::<Vec<_>>();
    for stream in media_streams {
        let _ = stream.close().await;
    }
    let stale_routes = routes
        .iter()
        .filter(|entry| entry.value().out_tx.same_channel(&route_out_tx))
        .map(|entry| (entry.key().clone(), entry.value().sid.clone()))
        .collect::<Vec<_>>();
    for (connection_id, sid) in stale_routes {
        let terminal = AdapterEvent::Ended {
            connection_id: connection_id.clone(),
            reason: EndReason::Failed {
                detail: "quic transport closed".into(),
            },
        };
        routes.remove(&connection_id);
        by_connection.remove(&connection_id);
        if by_uctp_sid
            .get(&sid)
            .is_some_and(|mapped| *mapped == connection_id)
        {
            by_uctp_sid.remove(&sid);
        }
        let delivery = lifecycle_sink
            .queue_or_deliver_terminal(&events_tx, terminal)
            .await;
        metrics::counter!(
            "uctp_terminal_delivery_total",
            "transport" => "quic",
            "outcome" => match delivery {
                TerminalDelivery::Queued => "queued",
                TerminalDelivery::Fallback => "fallback",
                TerminalDelivery::Undeliverable => "undeliverable",
            }
        )
        .increment(1);
        if delivery == TerminalDelivery::Undeliverable {
            warn!(%connection_id, "terminal event undeliverable before adapter registration");
        }
    }
    stats_pump.abort();

    info!(%peer_addr, "rvoip-quic: connection closed");
}
