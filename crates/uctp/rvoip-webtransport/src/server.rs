//! `UctpWtServer` — wraps a `quinn::Connection` accepted via ALPN `h3`,
//! drives the HTTP/3 + extended `CONNECT` upgrade to a
//! `web_transport_quinn::Session`, and spawns one
//! `rvoip_uctp::state::UctpCoordinator` per peer.

use std::sync::Arc;
use std::time::Duration;
use std::{collections::HashMap, num::NonZeroU16};

use chrono::Utc;
use dashmap::DashMap;
use futures::{SinkExt, StreamExt};
use rvoip_auth_core::BearerValidator;
use rvoip_core::adapter::{
    AdapterEvent, AdapterLifecycleSinkSlot, EndReason, OrchestratorAdapterEvent, TerminalDelivery,
};
use rvoip_core::capability::{CapabilityDescriptor, CodecInfo, NegotiatedCodecs};
use rvoip_core::connection::{Connection, ConnectionState, Direction, Transport, TransportHandle};
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId, StreamId};
use rvoip_core::stream::{MediaStream, StreamKind};

use crate::adapter::Route;
use crate::media_stream::WebTransportDatagramMediaStream;
use rvoip_uctp::envelope::UctpEnvelope;
use rvoip_uctp::state::{UctpCoordinator, UctpSessionEvent, ENVELOPE_CHANNEL_CAP};
use rvoip_uctp::substrate::{envelope_reader, envelope_writer};
use rvoip_uctp::CorrelationIdDiagnostic;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub struct UctpWtServer {}

impl UctpWtServer {
    pub(crate) fn start(
        mut accept_rx: mpsc::Receiver<quinn::Connection>,
        bearer: Arc<dyn BearerValidator>,
        events_tx: mpsc::Sender<OrchestratorAdapterEvent>,
        lifecycle_sink: AdapterLifecycleSinkSlot,
        by_connection: Arc<DashMap<ConnectionId, String>>,
        by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
        routes: Arc<DashMap<ConnectionId, Route>>,
        max_concurrent: usize,
        mount_path: String,
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
                            "transport" => "webtransport",
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
                let mount_path = mount_path.clone();
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
                    metrics::gauge!("uctp_active_connections", "transport" => "webtransport")
                        .increment(1.0);
                    spawn_peer_session(
                        conn,
                        bearer,
                        events_tx,
                        lifecycle_sink,
                        by_connection,
                        by_uctp_sid,
                        routes,
                        mount_path,
                        quinn_stats_interval,
                        subscription_handler,
                        session_binding_resolver,
                        orchestrator,
                        caps,
                        sig9421,
                    )
                    .await;
                    metrics::gauge!("uctp_active_connections", "transport" => "webtransport")
                        .decrement(1.0);
                });
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

struct BoundMediaBatch {
    wire_local_ids: Vec<u16>,
    local_ids: Vec<NonZeroU16>,
}

struct PreparedMediaBinding {
    local_id: NonZeroU16,
    stream: Arc<dyn MediaStream>,
}

async fn bind_media_batch(
    route: &Route,
    media_router: &Arc<rvoip_uctp::substrate::PeerMediaRouter>,
    core_session_id: &SessionId,
    core_connection_id: &ConnectionId,
    streams: Vec<rvoip_uctp::state::connection::AcceptedStream>,
) -> Result<BoundMediaBatch, rvoip_uctp::errors::UctpError> {
    if route.route_cancel.is_cancelled() {
        return Err(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
            "route-ending",
        ));
    }

    let mut prepared = Vec::with_capacity(streams.len());
    for stream in streams {
        match bind_one_media_stream(
            route,
            media_router,
            core_session_id,
            core_connection_id,
            stream,
        ) {
            Ok(binding) => prepared.push(binding),
            Err(error) => {
                rollback_prepared_media(media_router, prepared).await;
                return Err(error);
            }
        }
    }

    if route.route_cancel.is_cancelled() {
        rollback_prepared_media(media_router, prepared).await;
        return Err(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
            "route-ended-during-binding",
        ));
    }

    let wire_local_ids = prepared
        .iter()
        .map(|binding| binding.local_id.get())
        .collect();
    let local_ids = prepared.iter().map(|binding| binding.local_id).collect();
    for binding in prepared {
        route.streams.insert(binding.stream.id(), binding.stream);
    }
    Ok(BoundMediaBatch {
        wire_local_ids,
        local_ids,
    })
}

fn bind_one_media_stream(
    route: &Route,
    media_router: &Arc<rvoip_uctp::substrate::PeerMediaRouter>,
    core_session_id: &SessionId,
    core_connection_id: &ConnectionId,
    accepted: rvoip_uctp::state::connection::AcceptedStream,
) -> Result<PreparedMediaBinding, rvoip_uctp::errors::UctpError> {
    let kind = match accepted.kind.as_str() {
        "audio" => StreamKind::Audio,
        "video" => StreamKind::Video,
        "data" => StreamKind::Data,
        _ => {
            return Err(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
                "unsupported-stream-kind",
            ));
        }
    };
    let direction = match accepted.direction.as_str() {
        "recvonly" => Direction::Outbound,
        "sendonly" | "sendrecv" => Direction::Inbound,
        _ => {
            return Err(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
                "unsupported-stream-direction",
            ));
        }
    };
    let codec = accepted
        .chosen_codec
        .as_deref()
        .map(CodecInfo::from_name_with_defaults)
        .ok_or(rvoip_uctp::errors::UctpError::InvalidStreamBinding(
            "missing-negotiated-codec",
        ))?;
    let stream_id = StreamId::from_string(accepted.strm_id);
    let reservation = media_router.reserve().map_err(|error| {
        warn!(%error, "WebTransport peer media allocation rejected");
        match error {
            rvoip_uctp::substrate::PeerMediaRouterError::LocalIdExhausted => {
                rvoip_uctp::errors::UctpError::StreamHandleExhausted
            }
            _ => rvoip_uctp::errors::UctpError::InvalidStreamBinding("peer-media-router-rejected"),
        }
    })?;
    let local_id = reservation.local_id();
    let stream = WebTransportDatagramMediaStream::start_with_cancel(
        stream_id.clone(),
        kind,
        codec,
        direction,
        local_id.get(),
        route.session.clone(),
        reservation.cancellation_token(),
    );
    let stream_dyn: Arc<dyn MediaStream> = stream.clone();
    let mut registration = rvoip_uctp::substrate::PeerMediaRegistration::new(
        route.binding.owner().clone(),
        rvoip_uctp::substrate::PeerMediaRouteKey::new(
            core_session_id.clone(),
            core_connection_id.clone(),
            stream_id.clone(),
        ),
        Arc::clone(&stream_dyn),
        stream.inbound_tx(),
    );
    if accepted.direction != "recvonly" {
        registration = registration.with_fanout(rvoip_uctp::substrate::PeerMediaFanoutKey::new(
            core_session_id.clone(),
            core_connection_id.clone(),
            stream_id,
        ));
    }
    reservation.commit(registration).map_err(|error| {
        warn!(%error, "WebTransport peer media registration rejected");
        match error {
            rvoip_uctp::substrate::PeerMediaRouterError::LocalIdExhausted => {
                rvoip_uctp::errors::UctpError::StreamHandleExhausted
            }
            _ => rvoip_uctp::errors::UctpError::InvalidStreamBinding("peer-media-router-rejected"),
        }
    })?;
    Ok(PreparedMediaBinding {
        local_id,
        stream: stream_dyn,
    })
}

async fn rollback_prepared_media(
    media_router: &Arc<rvoip_uctp::substrate::PeerMediaRouter>,
    prepared: Vec<PreparedMediaBinding>,
) {
    for binding in prepared {
        media_router.remove_local_id(binding.local_id);
        let _ = binding.stream.close().await;
    }
}

async fn rollback_media_bindings(
    route: &Route,
    media_router: &Arc<rvoip_uctp::substrate::PeerMediaRouter>,
    local_ids: &[NonZeroU16],
) {
    let removed = local_ids
        .iter()
        .filter_map(|local_id| media_router.remove_local_id(*local_id))
        .collect();
    close_media_bindings(route, removed).await;
}

async fn close_media_bindings(
    route: &Route,
    bindings: Vec<Arc<rvoip_uctp::substrate::PeerMediaBinding>>,
) {
    for binding in bindings {
        route.streams.remove(&binding.stream().id());
        let _ = Arc::clone(binding.stream()).close().await;
    }
}

async fn spawn_peer_session(
    conn: quinn::Connection,
    bearer: Arc<dyn BearerValidator>,
    events_tx: mpsc::Sender<OrchestratorAdapterEvent>,
    lifecycle_sink: AdapterLifecycleSinkSlot,
    by_connection: Arc<DashMap<ConnectionId, String>>,
    by_uctp_sid: Arc<DashMap<String, ConnectionId>>,
    routes: Arc<DashMap<ConnectionId, Route>>,
    mount_path: String,
    quinn_stats_interval: Duration,
    subscription_handler: Option<Arc<dyn rvoip_uctp::state::SubscriptionHandler>>,
    session_binding_resolver: Option<Arc<dyn rvoip_uctp::state::SessionBindingResolver>>,
    orchestrator: Option<Arc<rvoip_core::Orchestrator>>,
    coordinator_caps: rvoip_uctp::state::UctpCoordinatorCaps,
    sig9421: Option<rvoip_uctp::state::Sig9421Config>,
) {
    // Keep peer-supplied Session IDs in this authenticated peer's namespace.
    let _adapter_global_sid_index = by_uctp_sid;
    let by_uctp_sid: Arc<DashMap<String, ConnectionId>> = Arc::new(DashMap::new());
    let peer_addr = conn.remote_address();
    info!(%peer_addr, "rvoip-webtransport: new connection");

    // HTTP/3 + extended-CONNECT upgrade.
    let authentication_deadline = coordinator_caps.authentication_deadline;
    let request = match tokio::time::timeout(
        authentication_deadline,
        web_transport_quinn::Request::accept(conn.clone()),
    )
    .await
    {
        Ok(Ok(request)) => request,
        Ok(Err(e)) => {
            warn!(error = %e, "rvoip-webtransport: wt upgrade rejected");
            return;
        }
        Err(_) => {
            warn!(%peer_addr, "rvoip-webtransport: upgrade timed out");
            conn.close(quinn::VarInt::from_u32(0x102), b"upgrade timeout");
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

    let session = match tokio::time::timeout(authentication_deadline, request.ok()).await {
        Ok(Ok(session)) => session,
        Ok(Err(e)) => {
            warn!(error = %e, "rvoip-webtransport: failed to confirm WT session");
            return;
        }
        Err(_) => {
            warn!(%peer_addr, "rvoip-webtransport: session confirmation timed out");
            conn.close(quinn::VarInt::from_u32(0x102), b"session setup timeout");
            return;
        }
    };

    let (send, recv) =
        match tokio::time::timeout(authentication_deadline, session.accept_bi()).await {
            Ok(Ok(streams)) => streams,
            Ok(Err(e)) => {
                warn!(error = %e, "rvoip-webtransport: accept_bi failed");
                return;
            }
            Err(_) => {
                warn!(%peer_addr, "rvoip-webtransport: signaling stream setup timed out");
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

    let route_out_tx = out_tx.clone();

    // The UCTP media header contains only a peer-local u16 stream handle, so
    // every logical Session on this physical WT peer shares one allocator and
    // one exact routing table.
    let media_router = rvoip_uctp::substrate::PeerMediaRouter::new();
    let media_cancel = CancellationToken::new();

    let inner_subscription_handler =
        subscription_handler.unwrap_or_else(|| rvoip_uctp::state::rejecting_handler());
    let resolver = session_binding_resolver.unwrap_or_else(|| {
        rvoip_uctp::state::PeerScopedSessionResolver::new(format!(
            "wt-peer-{}",
            ConnectionId::new()
        ))
    });
    let resource_bindings = rvoip_uctp::state::PeerResourceBindings::new(resolver);
    let subscription_handler: Arc<dyn rvoip_uctp::state::SubscriptionHandler> =
        rvoip_uctp::state::BoundSubscriptionHandler::new(
            Arc::clone(&resource_bindings),
            inner_subscription_handler,
        );
    let drain_grace = coordinator_caps.signaling_send_timeout;
    let coord = if let Some(sig9421) = sig9421 {
        UctpCoordinator::start_full_with_sig9421(
            "webtransport",
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
            "webtransport",
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
        warn!(%error, "failed to install WebTransport coordinator resource authority");
        media_cancel.cancel();
        coord.abort().await;
        return;
    }
    coord.enable_external_media_binding();
    // Gap plan §4.2 v1 punch list — capture the coordinator's
    // `Pending` correlator so per-Route adapter code can await
    // typed responses.
    let pending = coord.pending();
    let media_reader = crate::media_stream::spawn_datagram_reader_with_cancel(
        session.clone(),
        Arc::clone(&media_router),
        orchestrator.clone(),
        media_cancel.clone(),
    );
    let auth_guard =
        rvoip_uctp::state::spawn_auth_lifecycle_guard(Arc::clone(&coord), authentication_deadline);

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
    drop(in_tx);

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
        let media_router = Arc::clone(&media_router);
        let media_cancel = media_cancel.clone();
        let resource_bindings = Arc::clone(&resource_bindings);
        let coord_for_translator = Arc::clone(&coord);
        let lifecycle_for_translator = lifecycle_sink.clone();
        tokio::spawn(async move {
            // Per-peer auth state; consumed by the InboundInvite arm to
            // emit a synthetic `AdapterEvent::Authenticated` carrying
            // the just-created Connection's id. Plan §7 G1 / A3.
            let mut latest_auth: Option<(
                String,
                String,
                rvoip_core::identity::IdentityAssurance,
                Option<rvoip_core::identity::AuthenticatedPrincipal>,
            )> = None;
            let mut wire_to_core = HashMap::<ConnectionId, ConnectionId>::new();
            let mut wire_media_bindings =
                HashMap::<(SessionId, ConnectionId), Vec<NonZeroU16>>::new();

            while let Some(event) = coord_events_rx.recv().await {
                let adapter_event: Option<AdapterEvent> = match event {
                    UctpSessionEvent::Authenticated {
                        identity_id,
                        participant_id,
                        assurance,
                    } => {
                        let Some(principal) = coord_for_translator.authenticated_principal() else {
                            warn!("authenticated event missing retained principal; closing peer");
                            media_cancel.cancel();
                            break;
                        };
                        if let Err(error) = resource_bindings.authenticate(principal.clone()) {
                            warn!(%error, "refusing WebTransport resource owner change");
                            media_cancel.cancel();
                            break;
                        }
                        let refresh_targets = routes
                            .iter()
                            .filter(|entry| {
                                entry.value().out_tx.same_channel(&route_out_tx)
                                    && entry.value().binding.is_owned_by(&principal)
                            })
                            .map(|entry| entry.key().clone())
                            .collect::<Vec<_>>();
                        let mut refresh_delivery_failed = false;
                        for connection_id in refresh_targets {
                            if !rvoip_uctp::state::try_deliver_orchestrator_event(
                                &events_tx,
                                OrchestratorAdapterEvent::Public(
                                    AdapterEvent::PrincipalAuthenticated {
                                        connection_id,
                                        participant_id: participant_id.clone(),
                                        principal: principal.clone(),
                                    },
                                ),
                                "webtransport",
                            ) {
                                media_cancel.cancel();
                                refresh_delivery_failed = true;
                                break;
                            }
                        }
                        if refresh_delivery_failed {
                            break;
                        }
                        latest_auth =
                            Some((identity_id, participant_id, assurance, Some(principal)));
                        Some(AdapterEvent::Native {
                            kind: "uctp.authenticated",
                            detail: "bearer".into(),
                        })
                    }
                    UctpSessionEvent::InboundInvite { sid, from, .. } => {
                        let Some(principal) = coord_for_translator.authenticated_principal() else {
                            warn!(sid = ?CorrelationIdDiagnostic::new(sid.as_str()), "authenticated invite missing retained principal; refusing route");
                            continue;
                        };
                        let Some((_, participant_id, _, Some(_))) = latest_auth.clone() else {
                            warn!(sid = ?CorrelationIdDiagnostic::new(sid.as_str()), "authenticated invite missing atomic handoff identity; refusing route");
                            continue;
                        };
                        let core_session_id = match resource_bindings.bind_session(&sid) {
                            Ok(core_session_id) => core_session_id,
                            Err(error) => {
                                warn!(wire_sid = ?CorrelationIdDiagnostic::new(sid.as_str()), %error, "refusing unauthorized Session binding");
                                continue;
                            }
                        };
                        let (id, connection) = build_connection(
                            conn_for_translator.clone(),
                            core_session_id.clone(),
                            from,
                        );
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
                                session: session_for_translator.clone(),
                                media_router: Arc::clone(&media_router),
                                route_cancel,
                                coordinator: Arc::clone(&coord_for_translator),
                            },
                        );
                        if !rvoip_uctp::state::try_deliver_orchestrator_event(
                            &events_tx,
                            OrchestratorAdapterEvent::AuthenticatedInboundConnection {
                                connection,
                                participant_id,
                                principal,
                            },
                            "webtransport",
                        ) {
                            if let Some((_, route)) = routes.remove(&id) {
                                route.route_cancel.cancel();
                                let removed = media_router.remove_session(&route.core_session_id);
                                close_media_bindings(&route, removed).await;
                            }
                            by_connection.remove(&id);
                            if by_uctp_sid
                                .get(sid.as_str())
                                .is_some_and(|mapped| *mapped == id)
                            {
                                by_uctp_sid.remove(sid.as_str());
                            }
                            resource_bindings.remove_session(&sid);
                            break;
                        }
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
                    UctpSessionEvent::BindMediaStreams {
                        sid,
                        connid,
                        streams,
                        reply,
                    } => {
                        let core_session_id = resource_bindings.core_session(&sid);
                        let core_connection_id = resource_bindings.core_connection(&sid, &connid);
                        let route = core_connection_id
                            .as_ref()
                            .and_then(|connection_id| routes.get(connection_id).map(|r| r.clone()));
                        match (core_session_id, core_connection_id, route) {
                            (Some(core_session_id), Some(core_connection_id), Some(route))
                                if route.core_session_id == core_session_id =>
                            {
                                match bind_media_batch(
                                    &route,
                                    &media_router,
                                    &core_session_id,
                                    &core_connection_id,
                                    streams,
                                )
                                .await
                                {
                                    Ok(batch) => {
                                        if reply.send(Ok(batch.wire_local_ids.clone())).is_ok() {
                                            wire_media_bindings
                                                .entry((sid, connid))
                                                .or_default()
                                                .extend(batch.local_ids);
                                        } else {
                                            rollback_media_bindings(
                                                &route,
                                                &media_router,
                                                &batch.local_ids,
                                            )
                                            .await;
                                        }
                                    }
                                    Err(error) => {
                                        let _ = reply.send(Err(error));
                                    }
                                }
                            }
                            _ => {
                                let _ = reply.send(Err(
                                    rvoip_uctp::errors::UctpError::InvalidStreamBinding(
                                        "wire-resource-binding-not-ready",
                                    ),
                                ));
                            }
                        }
                        None
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
                    UctpSessionEvent::ConnectionEnded {
                        sid,
                        connid,
                        reason,
                    } => match wire_to_core.get(&connid).cloned() {
                        Some(connection_id) => {
                            let has_sibling = wire_to_core
                                .iter()
                                .any(|(wire, core)| wire != &connid && core == &connection_id);
                            if has_sibling {
                                wire_to_core.remove(&connid);
                                resource_bindings.remove_connection(&sid, &connid);
                                if let Some(local_ids) =
                                    wire_media_bindings.remove(&(sid.clone(), connid.clone()))
                                {
                                    if let Some(route) =
                                        routes.get(&connection_id).map(|entry| entry.clone())
                                    {
                                        rollback_media_bindings(&route, &media_router, &local_ids)
                                            .await;
                                    }
                                }
                                Some(AdapterEvent::Native {
                                    kind: "uctp.connection_ended",
                                    detail: format!("connid={connid} reason={reason}"),
                                })
                            } else {
                                let terminal = AdapterEvent::Ended {
                                    connection_id: connection_id.clone(),
                                    reason: EndReason::Failed { detail: reason },
                                };
                                let removed_route = routes.remove(&connection_id);
                                wire_to_core.remove(&connid);
                                wire_media_bindings.remove(&(sid.clone(), connid.clone()));
                                resource_bindings.remove_connection(&sid, &connid);
                                let sid =
                                    by_connection.get(&connection_id).map(|entry| entry.clone());
                                by_connection.remove(&connection_id);
                                if let Some(sid) = sid {
                                    if by_uctp_sid
                                        .get(&sid)
                                        .is_some_and(|mapped| *mapped == connection_id)
                                    {
                                        by_uctp_sid.remove(&sid);
                                    }
                                }
                                if let Some((_, route)) = removed_route {
                                    route.route_cancel.cancel();
                                    let _ = lifecycle_for_translator
                                        .queue_or_deliver_orchestrator_terminal(
                                            &events_tx, terminal,
                                        )
                                        .await;
                                    let connection_key =
                                        rvoip_uctp::substrate::PeerMediaConnectionKey::new(
                                            route.core_session_id.clone(),
                                            connection_id.clone(),
                                        );
                                    let removed = media_router.remove_connection(&connection_key);
                                    if tokio::time::timeout(
                                        Duration::from_secs(2),
                                        close_media_bindings(&route, removed),
                                    )
                                    .await
                                    .is_err()
                                    {
                                        warn!(connection_id = ?CorrelationIdDiagnostic::new(connection_id.as_str()), "timed out closing WebTransport route media after terminal delivery");
                                    }
                                }
                                None
                            }
                        }
                        None => {
                            resource_bindings.remove_connection(&sid, &connid);
                            Some(AdapterEvent::Native {
                                kind: "uctp.connection_ended_orphan",
                                detail: connid.to_string(),
                            })
                        }
                    },
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
                                let removed_route = routes.remove(&connection_id);
                                wire_to_core.retain(|_, core| core != &connection_id);
                                wire_media_bindings.retain(|(wire_sid, _), _| wire_sid != &sid);
                                by_connection.remove(&connection_id);
                                by_uctp_sid.remove(sid.as_str());
                                let core_session_id = resource_bindings.core_session(&sid);
                                resource_bindings.remove_session(&sid);
                                if let Some((_, route)) = removed_route {
                                    route.route_cancel.cancel();
                                    let _ = lifecycle_for_translator
                                        .queue_or_deliver_orchestrator_terminal(
                                            &events_tx, terminal,
                                        )
                                        .await;
                                    let removed = core_session_id
                                        .as_ref()
                                        .map(|session_id| media_router.remove_session(session_id))
                                        .unwrap_or_default();
                                    if tokio::time::timeout(
                                        Duration::from_secs(2),
                                        close_media_bindings(&route, removed),
                                    )
                                    .await
                                    .is_err()
                                    {
                                        warn!(connection_id = ?CorrelationIdDiagnostic::new(connection_id.as_str()), "timed out closing WebTransport Session media after terminal delivery");
                                    }
                                }
                                None
                            }
                            None => {
                                resource_bindings.remove_session(&sid);
                                Some(AdapterEvent::Native {
                                    kind: "uctp.session_ended_orphan",
                                    detail: format!("sid={} reason={}", sid, reason),
                                })
                            }
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
                                            warn!(
                                                wire_connid = ?CorrelationIdDiagnostic::new(connid.as_str()),
                                                existing_core = ?CorrelationIdDiagnostic::new(existing.as_str()),
                                                attempted_core = ?CorrelationIdDiagnostic::new(core_connection_id.as_str()),
                                                "wire connection ID already belongs to another route"
                                            );
                                            Some(AdapterEvent::Native {
                                                kind: "uctp.connection_binding_rejected",
                                                detail: connid.to_string(),
                                            })
                                        }
                                        _ => {
                                            let resource_bound = resource_bindings.bind_connection(
                                                &sid,
                                                &connid,
                                                core_connection_id.clone(),
                                            );
                                            match resource_bound {
                                                Ok(()) => match binding.bind_wire_connection(
                                                    &principal,
                                                    connid.clone(),
                                                ) {
                                                    Ok(()) => {
                                                        wire_to_core.insert(
                                                            connid.clone(),
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
                                                        warn!(wire_connid = ?CorrelationIdDiagnostic::new(connid.as_str()), error = %error, "refusing UCTP connection binding");
                                                        Some(AdapterEvent::Native {
                                                            kind:
                                                                "uctp.connection_binding_rejected",
                                                            detail: connid.to_string(),
                                                        })
                                                    }
                                                },
                                                Err(error) => {
                                                    warn!(wire_connid = ?CorrelationIdDiagnostic::new(connid.as_str()), %error, "refusing wire-to-core Connection binding");
                                                    Some(AdapterEvent::Native {
                                                        kind: "uctp.connection_binding_rejected",
                                                        detail: connid.to_string(),
                                                    })
                                                }
                                            }
                                        }
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
                    if !rvoip_uctp::state::try_deliver_orchestrator_event(
                        &events_tx,
                        OrchestratorAdapterEvent::Public(ev),
                        "webtransport",
                    ) {
                        break;
                    }
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
    let mut media_reader = media_reader;
    if tokio::time::timeout(drain_grace, &mut media_reader)
        .await
        .is_err()
    {
        media_reader.abort();
        let _ = media_reader.await;
    }
    resource_bindings.clear();
    let stale_routes = routes
        .iter()
        .filter(|entry| entry.value().out_tx.same_channel(&route_out_tx))
        .map(|entry| (entry.key().clone(), entry.value().sid.clone()))
        .collect::<Vec<_>>();
    for (connection_id, sid) in stale_routes {
        let terminal = AdapterEvent::Ended {
            connection_id: connection_id.clone(),
            reason: EndReason::Failed {
                detail: "webtransport transport closed".into(),
            },
        };
        let Some((_, route)) = routes.remove(&connection_id) else {
            continue;
        };
        route.route_cancel.cancel();
        by_connection.remove(&connection_id);
        if by_uctp_sid
            .get(&sid)
            .is_some_and(|mapped| *mapped == connection_id)
        {
            by_uctp_sid.remove(&sid);
        }
        let delivery = lifecycle_sink
            .queue_or_deliver_orchestrator_terminal(&events_tx, terminal)
            .await;
        metrics::counter!(
            "uctp_terminal_delivery_total",
            "transport" => "webtransport",
            "outcome" => match delivery {
                TerminalDelivery::Queued => "queued",
                TerminalDelivery::Fallback => "fallback",
                TerminalDelivery::Undeliverable => "undeliverable",
            }
        )
        .increment(1);
        if delivery == TerminalDelivery::Undeliverable {
            warn!(connection_id = ?CorrelationIdDiagnostic::new(connection_id.as_str()), "terminal event undeliverable before adapter registration");
        }
    }
    let media_bindings = media_router.shutdown();
    for binding in media_bindings {
        let _ = tokio::time::timeout(Duration::from_secs(2), Arc::clone(binding.stream()).close())
            .await;
    }
    stats_pump.abort();

    info!(%peer_addr, "rvoip-webtransport: connection closed");
}
