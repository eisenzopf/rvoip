//! Private draft-specific network adapter for managed catalog subscriptions.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use moq_transport::coding::{Location, TrackName, TrackNamespace};
use moq_transport::data::ObjectStatus;
use moq_transport::serve::{SubgroupsReader, Track, TrackReader, TrackReaderMode};
use moq_transport::session::{
    EndOfGroupState, Fetch, PublishedNamespace, Session, SessionError, SetupAuthorization,
    Subscribe, Subscriber, Transport,
};
use rvoip_core_traits::broadcast::BroadcastSubstrate;
use tokio::task::JoinHandle;

use crate::wire::{
    admit_relay_peer, canonical_session_target, map_peer_identity, native_substrate_policy,
    WireRelayClient,
};
use crate::{
    MoqCatalogSubscriberConfig, MoqCatalogValidationError, MoqEndOfGroupEvidence, MoqError,
    MoqRelayPeerIdentity, MoqRelaySubstratePolicy, MoqSubscriberCredential, CATALOG_TRACK,
    MOQT_NEGOTIATED_PROTOCOL,
};

const SESSION_CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub(crate) struct WireCatalogSubscriberClient {
    client: WireRelayClient,
}

impl WireCatalogSubscriberClient {
    pub(crate) fn bind(
        bind: SocketAddr,
        root_certificates: Vec<PathBuf>,
    ) -> Result<Self, MoqError> {
        Ok(Self {
            client: WireRelayClient::bind_server_authenticated(bind, root_certificates)?,
        })
    }

    pub(crate) async fn connect(
        &self,
        config: &MoqCatalogSubscriberConfig,
        credential: MoqSubscriberCredential,
    ) -> Result<WireCatalogSubscription, WireCatalogFailure> {
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| WireCatalogFailure::TaskFailed)?;
        let (target, substrate_policy) =
            canonical_session_target(&config.endpoint, config.substrate)
                .map_err(|_| WireCatalogFailure::ConnectFailed)?;
        let endpoint_uri = target.network_url().to_string();
        let connection = self
            .client
            .client()
            .connect_target(&target, native_substrate_policy(substrate_policy), None)
            .await
            .map_err(|_| WireCatalogFailure::ConnectFailed)?;

        let peer_identity = map_peer_identity(connection.peer_identity);
        admit_relay_peer(&peer_identity, self.client.requires_authenticated_peer())
            .map_err(|_| WireCatalogFailure::PeerUnauthenticated)?;
        require_negotiated_transport(config.substrate, connection.negotiated.substrate)?;
        if connection.negotiated.protocol != MOQT_NEGOTIATED_PROTOCOL {
            return Err(WireCatalogFailure::ProtocolMismatch);
        }

        let negotiated_protocol = connection.negotiated.protocol.to_owned();
        let substrate = map_substrate(connection.negotiated.substrate);
        let raw_session = connection.session.clone();
        let credential = credential.into_wire_bytes();
        let authorization = SetupAuthorization::new(credential.as_slice())
            .map_err(|_| WireCatalogFailure::SetupFailed)?;
        let (session, _publisher, mut subscriber) = Session::connect_with_authorization(
            connection.session,
            None,
            connection.negotiated,
            Some(authorization),
        )
        .await
        .map_err(|_| WireCatalogFailure::SetupFailed)?;
        drop(credential);

        let session_task = tokio::spawn(session.run());
        let mut pending = PendingCatalogSession::new(raw_session, session_task, runtime.clone());
        let mut published_namespace = tokio::select! {
            namespace = subscriber.published_namespace() => {
                namespace.ok_or(WireCatalogFailure::SessionEnded)?
            }
            result = pending.task_mut() => {
                pending.disarm_completed();
                return Err(map_session_result(result));
            }
        };
        let expected_namespace = TrackNamespace::from_utf8_path(config.namespace.as_str());
        if published_namespace.info.namespace != expected_namespace {
            return Err(WireCatalogFailure::InvalidTrack);
        }
        published_namespace
            .ok()
            .map_err(|_| WireCatalogFailure::SubscribeFailed)?;

        let (subscribe, fetch, track_reader, fetch_cutoff) = {
            let setup = async {
                let (track_writer, track_reader) =
                    Track::new(expected_namespace, TrackName::from(CATALOG_TRACK)).produce();
                let (subscribe, fetch) = subscriber
                    .subscribe_joining(track_writer)
                    .await
                    .map_err(|_| WireCatalogFailure::SubscribeFailed)?;
                subscribe
                    .ok()
                    .await
                    .map_err(|_| WireCatalogFailure::SubscribeFailed)?;
                let fetch_ok = fetch
                    .ok()
                    .await
                    .map_err(|_| WireCatalogFailure::SubscribeFailed)?;

                Ok::<_, WireCatalogFailure>((subscribe, fetch, track_reader, fetch_ok.end_location))
            };
            tokio::pin!(setup);
            tokio::select! {
                result = &mut setup => result?,
                result = pending.task_mut() => {
                    pending.disarm_completed();
                    return Err(map_session_result(result));
                }
            }
        };
        let (raw_session, session_task) = pending.into_parts();

        Ok(WireCatalogSubscription {
            endpoint_uri,
            substrate,
            negotiated_protocol,
            peer_identity,
            subscriber,
            track_reader: Some(track_reader),
            groups: None,
            fetch_cutoff,
            fetch_drained: false,
            fetch: Some(fetch),
            subscribe: Some(subscribe),
            published_namespace: Some(published_namespace),
            raw_session: Some(raw_session),
            session_task: Some(session_task),
            runtime,
        })
    }
}

pub(crate) struct WireCatalogSubscription {
    pub(crate) endpoint_uri: String,
    pub(crate) substrate: BroadcastSubstrate,
    pub(crate) negotiated_protocol: String,
    pub(crate) peer_identity: MoqRelayPeerIdentity,
    subscriber: Subscriber,
    track_reader: Option<TrackReader>,
    groups: Option<SubgroupsReader>,
    fetch_cutoff: Location,
    fetch_drained: bool,
    fetch: Option<Fetch>,
    subscribe: Option<Subscribe>,
    published_namespace: Option<PublishedNamespace>,
    raw_session: Option<web_transport::Session>,
    session_task: Option<JoinHandle<Result<(), SessionError>>>,
    runtime: tokio::runtime::Handle,
}

impl WireCatalogSubscription {
    pub(crate) async fn next_object(
        &mut self,
        max_catalog_bytes: usize,
    ) -> Result<Option<WireCatalogObject>, WireCatalogFailure> {
        if !self.fetch_drained {
            let fetched = tokio::select! {
                result = self.fetch.as_mut().expect("catalog FETCH handle missing").next_object() => result,
                result = session_task(self.session_task.as_mut()) => {
                    self.session_task.take();
                    return Err(map_session_result(result));
                }
                unexpected = self.subscriber.published_namespace() => {
                    drop(unexpected);
                    return Err(WireCatalogFailure::InvalidTrack);
                }
            };
            if let Some(fetched) = fetched {
                if fetched.payload.len() > max_catalog_bytes {
                    return Err(WireCatalogFailure::PayloadTooLarge);
                }
                return Ok(Some(WireCatalogObject {
                    group_id: fetched.location.group_id,
                    subgroup_id: fetched.subgroup_id,
                    object_id: fetched.location.object_id,
                    first_object: fetched.location.object_id == 0,
                    end_of_group: map_group_end(fetched.group_end),
                    extension_header_count: fetched.properties.0.len(),
                    declared_payload_len: fetched.payload.len() as u64,
                    payload: fetched.payload,
                }));
            }
            let completion = tokio::select! {
                result = self.fetch.as_ref().expect("catalog FETCH handle missing").completed() => result,
                result = session_task(self.session_task.as_mut()) => {
                    self.session_task.take();
                    return Err(map_session_result(result));
                }
                unexpected = self.subscriber.published_namespace() => {
                    drop(unexpected);
                    return Err(WireCatalogFailure::InvalidTrack);
                }
            };
            completion.map_err(|_| WireCatalogFailure::SubscribeFailed)?;
            self.fetch_drained = true;
        }

        if self.groups.is_none() {
            let mode = tokio::select! {
                result = self.track_reader.as_ref().expect("catalog Track reader missing").mode() => result,
                result = session_task(self.session_task.as_mut()) => {
                    self.session_task.take();
                    return Err(map_session_result(result));
                }
                unexpected = self.subscriber.published_namespace() => {
                    drop(unexpected);
                    return Err(WireCatalogFailure::InvalidTrack);
                }
            }
            .map_err(|_| WireCatalogFailure::InvalidTrack)?;
            self.groups = match mode {
                TrackReaderMode::Subgroups(groups) => Some(groups),
                _ => return Err(WireCatalogFailure::InvalidTrack),
            };
            self.track_reader.take();
        }

        loop {
            let subgroup = tokio::select! {
                result = self.groups.as_mut().expect("catalog subgroup reader missing").next() => {
                    result.map_err(|_| WireCatalogFailure::StreamEnded)?
                }
                result = session_task(self.session_task.as_mut()) => {
                    self.session_task.take();
                    return Err(map_session_result(result));
                }
                unexpected = self.subscriber.published_namespace() => {
                    drop(unexpected);
                    return Err(WireCatalogFailure::InvalidTrack);
                }
            };
            let Some(mut subgroup) = subgroup else {
                return Ok(None);
            };
            let Some(mut object) = subgroup
                .next()
                .await
                .map_err(|_| WireCatalogFailure::StreamEnded)?
            else {
                return Err(WireCatalogFailure::InvalidCatalog(
                    MoqCatalogValidationError::InvalidObject,
                ));
            };
            if object.status != ObjectStatus::NormalObject {
                return Err(WireCatalogFailure::InvalidCatalog(
                    MoqCatalogValidationError::InvalidObject,
                ));
            }
            if object.size > max_catalog_bytes {
                return Err(WireCatalogFailure::PayloadTooLarge);
            }
            let declared_payload_len = u64::try_from(object.size).unwrap_or(u64::MAX);
            let extension_header_count = object.extension_headers.0.len();
            let location = Location::new(subgroup.group_id, object.object_id);
            let payload = object
                .read_all()
                .await
                .map_err(|_| WireCatalogFailure::StreamEnded)?;
            if subgroup
                .next()
                .await
                .map_err(|_| WireCatalogFailure::StreamEnded)?
                .is_some()
            {
                return Err(WireCatalogFailure::InvalidCatalog(
                    MoqCatalogValidationError::InvalidObject,
                ));
            }

            // Joining FETCH also inserts fetched Objects into the merged
            // Track. They were already delivered above with explicit
            // UnknownFromFetch evidence, so discard only coordinates covered
            // by the immutable FETCH cutoff. Live Objects after the cutoff
            // retain their actual subgroup boundary flags.
            if location <= self.fetch_cutoff {
                continue;
            }
            return Ok(Some(WireCatalogObject {
                group_id: subgroup.group_id,
                subgroup_id: subgroup.subgroup_id,
                object_id: object.object_id,
                first_object: subgroup.first_object,
                end_of_group: if subgroup.end_of_group {
                    MoqEndOfGroupEvidence::Signaled
                } else {
                    MoqEndOfGroupEvidence::NotSignaled
                },
                extension_header_count,
                declared_payload_len,
                payload,
            }));
        }
    }

    pub(crate) async fn close(mut self, reason: &'static str) {
        self.fetch.take();
        self.subscribe.take();
        self.published_namespace.take();
        if let Some(raw_session) = self.raw_session.take() {
            raw_session.close(0, reason);
        }
        finish_session_task(&mut self.session_task).await;
    }
}

impl Drop for WireCatalogSubscription {
    fn drop(&mut self) {
        self.fetch.take();
        self.subscribe.take();
        self.published_namespace.take();
        if let Some(raw_session) = self.raw_session.take() {
            raw_session.close(0, "rvoip catalog subscriber dropped");
        }
        let Some(task) = self.session_task.take() else {
            return;
        };
        task.abort();
        let _cleanup = self.runtime.spawn(async move {
            let _ = task.await;
        });
    }
}

pub(crate) struct WireCatalogObject {
    pub(crate) group_id: u64,
    pub(crate) subgroup_id: u64,
    pub(crate) object_id: u64,
    pub(crate) first_object: bool,
    pub(crate) end_of_group: MoqEndOfGroupEvidence,
    pub(crate) extension_header_count: usize,
    pub(crate) declared_payload_len: u64,
    pub(crate) payload: Bytes,
}

#[derive(Debug)]
pub(crate) enum WireCatalogFailure {
    ConnectFailed,
    PeerUnauthenticated,
    ProtocolMismatch,
    SetupFailed,
    SubscribeFailed,
    InvalidTrack,
    InvalidCatalog(MoqCatalogValidationError),
    PayloadTooLarge,
    StreamEnded,
    SessionEnded,
    TaskFailed,
}

struct PendingCatalogSession {
    raw_session: Option<web_transport::Session>,
    task: Option<JoinHandle<Result<(), SessionError>>>,
    runtime: tokio::runtime::Handle,
}

impl PendingCatalogSession {
    fn new(
        raw_session: web_transport::Session,
        task: JoinHandle<Result<(), SessionError>>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            raw_session: Some(raw_session),
            task: Some(task),
            runtime,
        }
    }

    fn task_mut(&mut self) -> &mut JoinHandle<Result<(), SessionError>> {
        self.task
            .as_mut()
            .expect("pending catalog session task already consumed")
    }

    fn disarm_completed(&mut self) {
        self.task.take();
    }

    fn into_parts(mut self) -> (web_transport::Session, JoinHandle<Result<(), SessionError>>) {
        let raw_session = self
            .raw_session
            .take()
            .expect("pending catalog raw session already consumed");
        let task = self
            .task
            .take()
            .expect("pending catalog session task already consumed");
        (raw_session, task)
    }
}

impl Drop for PendingCatalogSession {
    fn drop(&mut self) {
        if let Some(raw_session) = self.raw_session.take() {
            raw_session.close(0, "rvoip catalog setup cancelled");
        }
        let Some(task) = self.task.take() else {
            return;
        };
        task.abort();
        let _cleanup = self.runtime.spawn(async move {
            let _ = task.await;
        });
    }
}

fn require_negotiated_transport(
    expected: MoqRelaySubstratePolicy,
    actual: Transport,
) -> Result<(), WireCatalogFailure> {
    let matches = matches!(
        (expected, actual),
        (MoqRelaySubstratePolicy::RawQuic, Transport::RawQuic)
            | (
                MoqRelaySubstratePolicy::WebTransport,
                Transport::WebTransport
            )
    );
    if matches {
        Ok(())
    } else {
        Err(WireCatalogFailure::ProtocolMismatch)
    }
}

const fn map_substrate(substrate: Transport) -> BroadcastSubstrate {
    match substrate {
        Transport::RawQuic => BroadcastSubstrate::RawQuic,
        Transport::WebTransport => BroadcastSubstrate::WebTransport,
    }
}

const fn map_group_end(group_end: EndOfGroupState) -> MoqEndOfGroupEvidence {
    match group_end {
        EndOfGroupState::Signaled => MoqEndOfGroupEvidence::Signaled,
        EndOfGroupState::NotSignaled => MoqEndOfGroupEvidence::NotSignaled,
        EndOfGroupState::UnknownFromFetch => MoqEndOfGroupEvidence::UnknownFromFetch,
    }
}

async fn session_task(
    task: Option<&mut JoinHandle<Result<(), SessionError>>>,
) -> Result<Result<(), SessionError>, tokio::task::JoinError> {
    task.expect("catalog session task missing").await
}

fn map_session_result(
    result: Result<Result<(), SessionError>, tokio::task::JoinError>,
) -> WireCatalogFailure {
    match result {
        Ok(_) => WireCatalogFailure::SessionEnded,
        Err(_) => WireCatalogFailure::TaskFailed,
    }
}

async fn finish_session_task(task: &mut Option<JoinHandle<Result<(), SessionError>>>) {
    let Some(mut task) = task.take() else {
        return;
    };
    if tokio::time::timeout(SESSION_CLOSE_TIMEOUT, &mut task)
        .await
        .is_err()
    {
        task.abort();
        let _ = task.await;
    }
}
