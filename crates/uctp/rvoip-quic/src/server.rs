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
use rvoip_core::stream::{MediaStream, StreamKind};

use crate::adapter::Route;
use crate::media_stream::QuicDatagramMediaStream;

use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::state::{UctpCoordinator, UctpSessionEvent, ENVELOPE_CHANNEL_CAP};
use rvoip_uctp::substrate::{
    envelope_reader, envelope_writer, PeerMediaConnectionKey, PeerMediaFanoutKey,
    PeerMediaRegistration, PeerMediaRouteKey, PeerMediaRouter,
};
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
        session_binding_resolver: Option<Arc<dyn rvoip_uctp::state::SessionBindingResolver>>,
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
                let session_binding_resolver = session_binding_resolver.clone();
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
                        session_binding_resolver,
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

struct BoundMediaBatch {
    local_ids: Vec<u16>,
    entries: Vec<(std::num::NonZeroU16, StreamId, Arc<QuicDatagramMediaStream>)>,
    route: Route,
}

impl BoundMediaBatch {
    async fn rollback(self) {
        for (local_id, stream_id, stream) in self.entries {
            self.route.media_router.remove_local_id(local_id);
            self.route.streams.remove(&stream_id);
            let _ = stream.close().await;
        }
    }
}

fn stream_kind(kind: &str) -> rvoip_uctp::Result<StreamKind> {
    match kind {
        "audio" => Ok(StreamKind::Audio),
        "video" => Ok(StreamKind::Video),
        "data" => Ok(StreamKind::Data),
        _ => Err(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
            "unsupported-stream-kind",
        )),
    }
}

fn stream_direction(direction: &str) -> rvoip_uctp::Result<Direction> {
    match direction {
        "recvonly" => Ok(Direction::Outbound),
        "sendonly" | "sendrecv" => Ok(Direction::Inbound),
        _ => Err(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
            "unsupported-stream-direction",
        )),
    }
}

fn map_media_router_error(
    error: rvoip_uctp::substrate::PeerMediaRouterError,
) -> rvoip_uctp::errors::UctpError {
    warn!(%error, "rvoip-quic: peer media binding failed");
    match error {
        rvoip_uctp::substrate::PeerMediaRouterError::LocalIdExhausted => {
            rvoip_uctp::errors::UctpError::StreamHandleExhausted
        }
        _ => rvoip_uctp::errors::UctpError::InvalidStreamBinding("peer-media-router-error"),
    }
}

async fn bind_media_stream_batch(
    wire_sid: &SessionId,
    wire_connid: &ConnectionId,
    accepted: Vec<rvoip_uctp::state::connection::AcceptedStream>,
    resource_bindings: &Arc<rvoip_uctp::state::PeerResourceBindings>,
    media_router: &Arc<PeerMediaRouter>,
    routes: &Arc<DashMap<ConnectionId, Route>>,
    conn: &quinn::Connection,
) -> rvoip_uctp::Result<BoundMediaBatch> {
    let core_session_id = resource_bindings.core_session(wire_sid).ok_or(
        rvoip_uctp::errors::UctpError::InvalidStreamBinding("session-binding-not-ready"),
    )?;
    let core_connection_id = resource_bindings
        .core_connection(wire_sid, wire_connid)
        .ok_or(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
            "connection-binding-not-ready",
        ))?;
    let route = routes
        .get(&core_connection_id)
        .map(|entry| entry.clone())
        .ok_or(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
            "adapter-route-not-ready",
        ))?;
    if route.route_cancel.is_cancelled() {
        return Err(rvoip_uctp::errors::UctpError::Closed);
    }

    let mut reservations = Vec::with_capacity(accepted.len());
    for _ in &accepted {
        match media_router.reserve() {
            Ok(reservation) => reservations.push(reservation),
            Err(error) => return Err(map_media_router_error(error)),
        }
    }

    let mut plans = Vec::with_capacity(accepted.len());
    for (stream, reservation) in accepted.into_iter().zip(reservations) {
        let Some(codec_name) = stream.chosen_codec.as_deref() else {
            return Err(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
                "missing-negotiated-codec",
            ));
        };
        let stream_id = StreamId::from_string(stream.strm_id);
        let concrete = QuicDatagramMediaStream::start_with_cancel(
            stream_id.clone(),
            stream_kind(&stream.kind)?,
            CodecInfo::from_name_with_defaults(codec_name),
            stream_direction(&stream.direction)?,
            reservation.local_id().get(),
            conn.clone(),
            reservation.cancellation_token(),
        );
        plans.push((reservation, stream_id, concrete));
    }

    let mut entries: Vec<(std::num::NonZeroU16, StreamId, Arc<QuicDatagramMediaStream>)> =
        Vec::with_capacity(plans.len());
    for (reservation, stream_id, stream) in plans {
        let local_id = reservation.local_id();
        let stream_dyn: Arc<dyn MediaStream> = stream.clone();
        let registration = PeerMediaRegistration::new(
            route.binding.owner().clone(),
            PeerMediaRouteKey::new(
                core_session_id.clone(),
                core_connection_id.clone(),
                stream_id.clone(),
            ),
            stream_dyn,
            stream.inbound_tx(),
        )
        .with_fanout(PeerMediaFanoutKey::new(
            core_session_id.clone(),
            core_connection_id.clone(),
            stream_id.clone(),
        ));
        if let Err(error) = reservation.commit(registration) {
            for (committed_id, committed_stream_id, committed_stream) in entries {
                media_router.remove_local_id(committed_id);
                route.streams.remove(&committed_stream_id);
                let _ = committed_stream.close().await;
            }
            let _ = stream.close().await;
            return Err(map_media_router_error(error));
        }
        route.streams.insert(stream_id.clone(), stream.clone());
        entries.push((local_id, stream_id, stream));
    }

    if route.route_cancel.is_cancelled() {
        BoundMediaBatch {
            local_ids: Vec::new(),
            entries,
            route,
        }
        .rollback()
        .await;
        return Err(rvoip_uctp::errors::UctpError::Closed);
    }

    Ok(BoundMediaBatch {
        local_ids: entries.iter().map(|(id, _, _)| id.get()).collect(),
        entries,
        route,
    })
}

async fn close_route_media(route: &Route) {
    route.route_cancel.cancel();
    let removed = route
        .media_router
        .remove_connection(&PeerMediaConnectionKey::new(
            route.core_session_id.clone(),
            route.core_connection_id.clone(),
        ));
    route.streams.clear();
    for binding in removed {
        let _ = binding.stream().clone().close().await;
    }
}

type WireConnectionKey = (SessionId, ConnectionId);

fn resolve_unscoped_wire_connection(
    bindings: &std::collections::HashMap<WireConnectionKey, ConnectionId>,
    wire_connection_id: &ConnectionId,
) -> Option<ConnectionId> {
    let mut matches = bindings
        .iter()
        .filter(|((_, candidate), _)| candidate == wire_connection_id)
        .map(|(_, core)| core.clone());
    let first = matches.next()?;
    if matches.any(|candidate| candidate != first) {
        return None;
    }
    Some(first)
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
    session_binding_resolver: Option<Arc<dyn rvoip_uctp::state::SessionBindingResolver>>,
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

    // Exactly one stream-local-ID namespace and one datagram reader exist for
    // this physical peer, regardless of how many UCTP Sessions it multiplexes.
    let media_router = PeerMediaRouter::new();
    let media_cancel = CancellationToken::new();
    let mut media_reader = crate::media_stream::spawn_datagram_reader_with_cancel(
        conn.clone(),
        Arc::clone(&media_router),
        orchestrator,
        media_cancel.clone(),
    );

    // Clone the outbound sender BEFORE handing it to the coordinator so
    // the event translator can stash it under each new ConnectionId for
    // the adapter's `accept` / `reject` / `end` / `send_message` methods.
    let route_out_tx = out_tx.clone();

    // All wire resource IDs pass through authenticated peer bindings before
    // the shared publisher/subscriber handler sees them.
    let resolver = session_binding_resolver.unwrap_or_else(|| {
        rvoip_uctp::state::PeerScopedSessionResolver::new(ConnectionId::new().to_string())
    });
    let resource_bindings = rvoip_uctp::state::PeerResourceBindings::new(resolver);
    let subscription_handler =
        subscription_handler.unwrap_or_else(|| rvoip_uctp::state::rejecting_handler());
    let subscription_handler: Arc<dyn rvoip_uctp::state::SubscriptionHandler> =
        rvoip_uctp::state::BoundSubscriptionHandler::new(
            Arc::clone(&resource_bindings),
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
    if let Err(error) = coord.set_resource_bindings(Arc::clone(&resource_bindings)) {
        warn!(%error, "rvoip-quic: failed to install coordinator resource authority");
        media_cancel.cancel();
        media_reader.abort();
        let _ = media_reader.await;
        coord.abort().await;
        return;
    }
    coord.enable_external_media_binding();
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
        let media_cancel = media_cancel.clone();
        let media_router = Arc::clone(&media_router);
        let resource_bindings = Arc::clone(&resource_bindings);
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
            let mut wire_to_core =
                std::collections::HashMap::<WireConnectionKey, ConnectionId>::new();

            while let Some(event) = coord_events_rx.recv().await {
                let adapter_event: Option<AdapterEvent> = match event {
                    UctpSessionEvent::Authenticated {
                        identity_id,
                        participant_id,
                        assurance,
                    } => {
                        let principal = coord_for_translator.authenticated_principal();
                        let Some(principal_for_bindings) = principal.clone() else {
                            warn!("rvoip-quic: authenticated event missing retained principal");
                            media_cancel.cancel();
                            break;
                        };
                        if let Err(error) = resource_bindings.authenticate(principal_for_bindings) {
                            warn!(%error, "rvoip-quic: refusing authenticated peer resource binding");
                            media_cancel.cancel();
                            break;
                        }
                        latest_auth = Some((identity_id, participant_id, assurance, principal));
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
                        let core_session_id = match resource_bindings.bind_session(&sid) {
                            Ok(session_id) => session_id,
                            Err(error) => {
                                warn!(%sid, %error, "rvoip-quic: refusing unauthorized Session binding");
                                let rejection = UctpEnvelope::new(
                                    rvoip_uctp::types::MessageType::SessionReject,
                                    serde_json::to_value(
                                        rvoip_uctp::payloads::session::SessionReject {
                                            by: "system:rvoip".into(),
                                            reason_code: error.code,
                                            reason: error.reason,
                                        },
                                    )
                                    .expect("SessionReject is serializable"),
                                )
                                .with_sid(sid.to_string());
                                let _ = route_out_tx.try_send(rejection);
                                continue;
                            }
                        };
                        let (id, connection) = build_connection(
                            conn_for_translator.clone(),
                            core_session_id.clone(),
                            from,
                        );
                        // Media is created only after capability negotiation
                        // asks for an external all-or-nothing binding.
                        let route_cancel = media_cancel.child_token();
                        let route_streams: Arc<DashMap<StreamId, Arc<dyn MediaStream>>> =
                            Arc::new(DashMap::new());
                        by_connection.insert(id.clone(), sid.to_string());
                        by_uctp_sid.insert(sid.to_string(), id.clone());
                        routes.insert(
                            id.clone(),
                            Route {
                                sid: sid.to_string(),
                                core_session_id,
                                core_connection_id: id.clone(),
                                binding: rvoip_uctp::adapter_helpers::AuthenticatedConnectionBinding::new(&principal),
                                out_tx: route_out_tx.clone(),
                                pending: Arc::clone(&pending),
                                streams: route_streams,
                                conn: conn_for_translator.clone(),
                                media_router: Arc::clone(&media_router),
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
                    UctpSessionEvent::ConnectionConnected { sid, connid } => wire_to_core
                        .get(&(sid, connid.clone()))
                        .cloned()
                        .map(|connection_id| AdapterEvent::Connected { connection_id })
                        .or_else(|| {
                            Some(AdapterEvent::Native {
                                kind: "uctp.connection_connected_orphan",
                                detail: connid.to_string(),
                            })
                        }),
                    UctpSessionEvent::ConnectionEnded {
                        sid,
                        connid,
                        reason,
                    } => {
                        resource_bindings.remove_connection(&sid, &connid);
                        let wire_key = (sid.clone(), connid.clone());
                        match wire_to_core.get(&wire_key).cloned() {
                            Some(connection_id) => {
                                let has_sibling = wire_to_core.iter().any(|(wire, core)| {
                                    wire != &wire_key && core == &connection_id
                                });
                                if has_sibling {
                                    wire_to_core.remove(&wire_key);
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
                                    wire_to_core.remove(&wire_key);
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
                                        close_route_media(&route).await;
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
                        let core_session_id = resource_bindings.core_session(&sid);
                        resource_bindings.remove_session(&sid);
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
                                    close_route_media(&route).await;
                                } else if let Some(core_session_id) = core_session_id {
                                    let removed = media_router.remove_session(&core_session_id);
                                    for binding in removed {
                                        let _ = binding.stream().clone().close().await;
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
                                    Some(binding) => match wire_to_core
                                        .get(&(sid.clone(), connid.clone()))
                                    {
                                        Some(existing) if existing != &core_connection_id => {
                                            warn!(wire_connid = %connid, existing_core = %existing, attempted_core = %core_connection_id, "wire connection ID already belongs to another route");
                                            Some(AdapterEvent::Native {
                                                kind: "uctp.connection_binding_rejected",
                                                detail: connid.to_string(),
                                            })
                                        }
                                        _ => match resource_bindings.bind_connection(
                                            &sid,
                                            &connid,
                                            core_connection_id.clone(),
                                        ) {
                                            Ok(()) => match binding
                                                .bind_wire_connection(&principal, connid.clone())
                                            {
                                                Ok(()) => {
                                                    wire_to_core.insert(
                                                        (sid.clone(), connid.clone()),
                                                        core_connection_id,
                                                    );
                                                    Some(AdapterEvent::Native {
                                                        kind: "uctp.connection_bound",
                                                        detail: connid.to_string(),
                                                    })
                                                }
                                                Err(error) => {
                                                    resource_bindings
                                                        .remove_connection(&sid, &connid);
                                                    warn!(wire_connid = %connid, error = %error, "refusing UCTP connection binding");
                                                    Some(AdapterEvent::Native {
                                                        kind: "uctp.connection_binding_rejected",
                                                        detail: connid.to_string(),
                                                    })
                                                }
                                            },
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
                    UctpSessionEvent::BindMediaStreams {
                        sid,
                        connid,
                        streams,
                        reply,
                    } => {
                        match bind_media_stream_batch(
                            &sid,
                            &connid,
                            streams,
                            &resource_bindings,
                            &media_router,
                            &routes,
                            &conn_for_translator,
                        )
                        .await
                        {
                            Ok(batch) => {
                                if reply.send(Ok(batch.local_ids.clone())).is_err() {
                                    batch.rollback().await;
                                }
                            }
                            Err(error) => {
                                let _ = reply.send(Err(error));
                            }
                        }
                        None
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
                    } => resolve_unscoped_wire_connection(&wire_to_core, &connid).map(
                        |connection_id| AdapterEvent::Dtmf {
                            connection_id,
                            digits,
                            duration_ms,
                        },
                    ),
                    UctpSessionEvent::DataMessage { connid, message } => {
                        resolve_unscoped_wire_connection(&wire_to_core, &connid).map(
                            |connection_id| AdapterEvent::DataMessage {
                                connection_id,
                                message,
                            },
                        )
                    }
                    UctpSessionEvent::Quality {
                        connid,
                        strm_id: _,
                        snapshot,
                        rtt_ms: _,
                        bitrate_bps: _,
                    } => resolve_unscoped_wire_connection(&wire_to_core, &connid).map(
                        |connection_id| AdapterEvent::Quality {
                            connection_id,
                            snapshot,
                        },
                    ),
                    UctpSessionEvent::StepUpResponse {
                        connid,
                        method,
                        credential,
                    } => connid.and_then(|wire_connection_id| {
                        resolve_unscoped_wire_connection(&wire_to_core, &wire_connection_id).map(
                            |connection_id| AdapterEvent::StepUpResponse {
                                connection_id,
                                method,
                                credential,
                            },
                        )
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
    if tokio::time::timeout(drain_grace, &mut media_reader)
        .await
        .is_err()
    {
        media_reader.abort();
        let _ = media_reader.await;
    }
    resource_bindings.clear();
    let media_bindings = media_router.shutdown();
    for binding in media_bindings {
        let _ = binding.stream().clone().close().await;
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
        if let Some((_, route)) = routes.remove(&connection_id) {
            close_route_media(&route).await;
        }
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
