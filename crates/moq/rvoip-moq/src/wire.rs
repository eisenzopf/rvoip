//! Draft-specific moq-rs adapter.
//!
//! Nothing in this module is re-exported. In particular, `moq_transport`
//! readers, writers, sessions, and errors cannot appear in rvoip-moq's public
//! signatures.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moq_transport::coding::{TrackName, TrackNamespace};
use moq_transport::data::ExtensionHeaders;
#[cfg(test)]
use moq_transport::serve::TrackReader;
use moq_transport::serve::{
    Subgroup, SubgroupsWriter, Tracks, TracksReader, TracksRequest, TracksWriter,
};
use moq_transport::session::{PublishNamespaceAcceptanceError, SessionTarget, Transport};
use rvoip_core_traits::broadcast::BroadcastSubstrate;
use tokio::task::JoinHandle;
use url::Url;

use crate::{
    LocAudioObject, MoqError, MoqNamespace, MoqRelayFailure, MoqRelayPeerIdentity,
    MoqRelaySubstratePolicy, AUDIO_TRACK, CATALOG_TRACK, EVENTS_TRACK, MOQT_NEGOTIATED_PROTOCOL,
};

pub(crate) struct WirePublication {
    tracks_reader: TracksReader,
    control: Mutex<Option<WireControl>>,
    lifecycle: Arc<WireLifecycle>,
    #[cfg(test)]
    fail_writes: Arc<AtomicBool>,
}

#[derive(Clone)]
pub(crate) struct WirePublicationHandle {
    tracks_reader: TracksReader,
}

struct WireControl {
    _tracks_writer: TracksWriter,
    _tracks_request: TracksRequest,
}

pub(crate) struct WireAudioWriter {
    audio: Option<SubgroupsWriter>,
    lifecycle: Arc<WireLifecycle>,
    #[cfg(test)]
    fail_writes: Arc<AtomicBool>,
}

pub(crate) struct WireCatalogWriter {
    catalog: Option<SubgroupsWriter>,
    lifecycle: Arc<WireLifecycle>,
    #[cfg(test)]
    fail_writes: Arc<AtomicBool>,
}

pub(crate) struct WireEventsWriter {
    events: Option<SubgroupsWriter>,
    lifecycle: Arc<WireLifecycle>,
    #[cfg(test)]
    fail_writes: Arc<AtomicBool>,
}

const WIRE_CREATED: u8 = 0;
const WIRE_LIVE_CATALOG: u8 = 1;
const WIRE_AUDIO_ENDED: u8 = 2;
const WIRE_TERMINAL_CATALOG: u8 = 3;
const WIRE_CATALOG_ENDED: u8 = 4;
const WIRE_HARD_CLOSED: u8 = 5;

struct WireLifecycle {
    state: AtomicU8,
    events_finished: AtomicBool,
}

impl WireLifecycle {
    fn transition(&self, expected: u8, next: u8) -> Result<(), MoqError> {
        self.state
            .compare_exchange(expected, next, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| MoqError::Wire("invalid MOQT publication write ordering".to_owned()))
    }

    fn hard_close(&self) {
        let mut current = self.state.load(Ordering::Acquire);
        while current != WIRE_CATALOG_ENDED && current != WIRE_HARD_CLOSED {
            match self.state.compare_exchange_weak(
                current,
                WIRE_HARD_CLOSED,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }
}

impl WirePublication {
    pub(crate) fn new(
        namespace: &MoqNamespace,
    ) -> Result<(Self, WireCatalogWriter, WireAudioWriter), MoqError> {
        let (publication, catalog, audio, events) = Self::new_profile(namespace, false)?;
        debug_assert!(events.is_none());
        Ok((publication, catalog, audio))
    }

    pub(crate) fn new_with_sanitized_events(
        namespace: &MoqNamespace,
    ) -> Result<(Self, WireCatalogWriter, WireAudioWriter, WireEventsWriter), MoqError> {
        let (publication, catalog, audio, events) = Self::new_profile(namespace, true)?;
        Ok((
            publication,
            catalog,
            audio,
            events.expect("enabled event track must return its writer"),
        ))
    }

    fn new_profile(
        namespace: &MoqNamespace,
        sanitized_events: bool,
    ) -> Result<
        (
            Self,
            WireCatalogWriter,
            WireAudioWriter,
            Option<WireEventsWriter>,
        ),
        MoqError,
    > {
        let wire_namespace = TrackNamespace::from_utf8_path(namespace.as_str());
        let (mut tracks_writer, tracks_request, tracks_reader) =
            Tracks::new(wire_namespace).produce();

        let audio_track = tracks_writer
            .create(TrackName::from(AUDIO_TRACK))
            .ok_or(MoqError::Closed)?;
        let audio = audio_track
            .subgroups()
            .map_err(|error| MoqError::Wire(error.to_string()))?;

        let catalog_track = tracks_writer
            .create(TrackName::from(CATALOG_TRACK))
            .ok_or(MoqError::Closed)?;
        let catalog = catalog_track
            .subgroups()
            .map_err(|error| MoqError::Wire(error.to_string()))?;
        let events = if sanitized_events {
            let events_track = tracks_writer
                .create(TrackName::from(EVENTS_TRACK))
                .ok_or(MoqError::Closed)?;
            Some(
                events_track
                    .subgroups()
                    .map_err(|error| MoqError::Wire(error.to_string()))?,
            )
        } else {
            None
        };
        let lifecycle = Arc::new(WireLifecycle {
            state: AtomicU8::new(WIRE_CREATED),
            events_finished: AtomicBool::new(!sanitized_events),
        });
        #[cfg(test)]
        let fail_writes = Arc::new(AtomicBool::new(false));

        Ok((
            Self {
                tracks_reader,
                control: Mutex::new(Some(WireControl {
                    _tracks_writer: tracks_writer,
                    _tracks_request: tracks_request,
                })),
                lifecycle: Arc::clone(&lifecycle),
                #[cfg(test)]
                fail_writes: Arc::clone(&fail_writes),
            },
            WireCatalogWriter {
                catalog: Some(catalog),
                lifecycle: Arc::clone(&lifecycle),
                #[cfg(test)]
                fail_writes: Arc::clone(&fail_writes),
            },
            WireAudioWriter {
                audio: Some(audio),
                lifecycle: Arc::clone(&lifecycle),
                #[cfg(test)]
                fail_writes: Arc::clone(&fail_writes),
            },
            events.map(|events| WireEventsWriter {
                events: Some(events),
                lifecycle,
                #[cfg(test)]
                fail_writes,
            }),
        ))
    }

    pub(crate) fn close(&self) {
        self.lifecycle.hard_close();
        self.control
            .lock()
            .expect("MOQT wire control poisoned")
            .take();
    }

    fn tracks(&self) -> TracksReader {
        self.tracks_reader.clone()
    }

    pub(crate) fn tracks_handle(&self) -> WirePublicationHandle {
        WirePublicationHandle {
            tracks_reader: self.tracks(),
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_writes_for_test(&self) {
        self.fail_writes.store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn is_closed_for_test(&self) -> bool {
        self.control
            .lock()
            .expect("MOQT wire control poisoned")
            .is_none()
    }

    #[cfg(test)]
    pub(crate) fn is_cleanly_completed_for_test(&self) -> bool {
        self.lifecycle.state.load(Ordering::Acquire) == WIRE_CATALOG_ENDED
    }

    #[cfg(test)]
    fn tracks_for_test(&self) -> TracksReader {
        self.tracks()
    }

    #[cfg(test)]
    pub(crate) fn event_track_for_test(&self) -> Option<TrackReader> {
        let mut tracks = self.tracks();
        let namespace = tracks.info.namespace.clone();
        tracks.get_track_reader(&namespace, TrackName::from(EVENTS_TRACK))
    }
}

impl WireAudioWriter {
    pub(crate) fn write(&mut self, object: LocAudioObject) -> Result<(), MoqError> {
        #[cfg(test)]
        if self.fail_writes.load(Ordering::Acquire) {
            return Err(MoqError::Closed);
        }
        let mut extension_headers = ExtensionHeaders::new();
        for property in object.properties() {
            extension_headers.set_intvalue(property.id, property.value);
        }
        if object.object_id != 0 {
            return Err(MoqError::Wire(
                "canonical LOC publication requires Object ID 0".to_owned(),
            ));
        }
        if self.lifecycle.state.load(Ordering::Acquire) != WIRE_LIVE_CATALOG {
            return Err(MoqError::Wire(
                "audio cannot be published before the live catalog".to_owned(),
            ));
        }
        write_stream_object(
            self.audio.as_mut().ok_or(MoqError::Closed)?,
            object.group_id,
            object.payload,
            extension_headers,
        )
    }

    pub(crate) fn finish(mut self) -> Result<(), MoqError> {
        if !self.lifecycle.events_finished.load(Ordering::Acquire) {
            return Err(MoqError::Wire(
                "event track must end before the audio track".to_owned(),
            ));
        }
        drop(self.audio.take());
        self.lifecycle
            .transition(WIRE_LIVE_CATALOG, WIRE_AUDIO_ENDED)
    }
}

impl WireEventsWriter {
    pub(crate) fn write(&mut self, group_id: u64, payload: Vec<u8>) -> Result<(), MoqError> {
        #[cfg(test)]
        if self.fail_writes.load(Ordering::Acquire) {
            return Err(MoqError::Closed);
        }
        if self.lifecycle.state.load(Ordering::Acquire) != WIRE_LIVE_CATALOG {
            return Err(MoqError::Wire(
                "events cannot be published before the live catalog".to_owned(),
            ));
        }
        if self.lifecycle.events_finished.load(Ordering::Acquire) {
            return Err(MoqError::Closed);
        }
        write_stream_object(
            self.events.as_mut().ok_or(MoqError::Closed)?,
            group_id,
            payload.into(),
            ExtensionHeaders::new(),
        )
    }

    pub(crate) fn finish(mut self) -> Result<(), MoqError> {
        if self.lifecycle.state.load(Ordering::Acquire) != WIRE_LIVE_CATALOG {
            return Err(MoqError::Wire(
                "event track can only end after the live catalog".to_owned(),
            ));
        }
        self.lifecycle
            .events_finished
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| MoqError::Closed)?;
        drop(self.events.take());
        Ok(())
    }
}

impl WireCatalogWriter {
    pub(crate) fn write_live(&mut self, group_id: u64, payload: Vec<u8>) -> Result<(), MoqError> {
        #[cfg(test)]
        if self.fail_writes.load(Ordering::Acquire) {
            return Err(MoqError::Closed);
        }
        if self.lifecycle.state.load(Ordering::Acquire) != WIRE_CREATED {
            return Err(MoqError::Wire(
                "live catalog must be the first publication object".to_owned(),
            ));
        }
        write_stream_object(
            self.catalog.as_mut().ok_or(MoqError::Closed)?,
            group_id,
            payload.into(),
            ExtensionHeaders::new(),
        )?;
        self.lifecycle.transition(WIRE_CREATED, WIRE_LIVE_CATALOG)
    }

    pub(crate) fn write_terminal(
        &mut self,
        group_id: u64,
        payload: Vec<u8>,
    ) -> Result<(), MoqError> {
        #[cfg(test)]
        if self.fail_writes.load(Ordering::Acquire) {
            return Err(MoqError::Closed);
        }
        if self.lifecycle.state.load(Ordering::Acquire) != WIRE_AUDIO_ENDED {
            return Err(MoqError::Wire(
                "terminal catalog requires the audio track to end first".to_owned(),
            ));
        }
        write_stream_object(
            self.catalog.as_mut().ok_or(MoqError::Closed)?,
            group_id,
            payload.into(),
            ExtensionHeaders::new(),
        )?;
        self.lifecycle
            .transition(WIRE_AUDIO_ENDED, WIRE_TERMINAL_CATALOG)
    }

    pub(crate) fn finish(mut self) -> Result<(), MoqError> {
        drop(self.catalog.take());
        self.lifecycle
            .transition(WIRE_TERMINAL_CATALOG, WIRE_CATALOG_ENDED)
    }
}

fn write_stream_object(
    track: &mut SubgroupsWriter,
    group_id: u64,
    payload: bytes::Bytes,
    extension_headers: ExtensionHeaders,
) -> Result<(), MoqError> {
    let subgroup = Subgroup::new(group_id, 0, 0)
        .with_first_object(true)
        .with_end_of_group(true);
    let mut stream = track
        .create(subgroup)
        .map_err(|error| MoqError::Wire(error.to_string()))?;
    let mut object = stream
        .create(payload.len(), Some(extension_headers))
        .map_err(|error| MoqError::Wire(error.to_string()))?;
    object
        .write(payload)
        .map_err(|error| MoqError::Wire(error.to_string()))?;
    drop(object);
    drop(stream);
    Ok(())
}

#[derive(Clone)]
pub(crate) struct WireRelayClient {
    // Keep only the client half. The endpoint's optional server contains an
    // accept-future set which is intentionally not Sync and is unnecessary for
    // an origin publishing to a relay.
    client: moq_native_ietf::quic::Client,
    require_authenticated_peer: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WireTlsMode {
    ProductionMutualTls,
    ProductionServerAuthenticated,
    #[cfg(feature = "insecure-development")]
    DevelopmentServerAuthenticated,
    #[cfg(feature = "insecure-development")]
    DevelopmentInsecure,
}

impl WireRelayClient {
    pub(crate) fn bind(
        bind: SocketAddr,
        root_certificates: Vec<PathBuf>,
        client_certificate: Option<PathBuf>,
        client_private_key: Option<PathBuf>,
        disable_verification: bool,
        mode: WireTlsMode,
    ) -> Result<Self, MoqError> {
        if client_certificate.is_some() != client_private_key.is_some() {
            return Err(MoqError::TlsConfiguration(
                "MOQT client certificate and private key must be configured together",
            ));
        }
        match mode {
            WireTlsMode::ProductionMutualTls => {
                if client_certificate.is_none() {
                    return Err(MoqError::TlsConfiguration(
                        "production relay connections require a client certificate and private key",
                    ));
                }
                if disable_verification {
                    return Err(MoqError::TlsConfiguration(
                        "production relay connections require server certificate verification",
                    ));
                }
            }
            WireTlsMode::ProductionServerAuthenticated => {
                if client_certificate.is_some() {
                    return Err(MoqError::TlsConfiguration(
                        "server-authenticated subscriber connections cannot present client credentials",
                    ));
                }
                if disable_verification {
                    return Err(MoqError::TlsConfiguration(
                        "production subscriber connections require server certificate verification",
                    ));
                }
            }
            #[cfg(feature = "insecure-development")]
            WireTlsMode::DevelopmentServerAuthenticated => {
                if client_certificate.is_some() || disable_verification {
                    return Err(MoqError::TlsConfiguration(
                        "server-auth-only development mode cannot use client credentials or disable verification",
                    ));
                }
            }
            #[cfg(feature = "insecure-development")]
            WireTlsMode::DevelopmentInsecure => {
                if client_certificate.is_some() || !disable_verification {
                    return Err(MoqError::TlsConfiguration(
                        "insecure development mode requires disable_verification and no client credentials",
                    ));
                }
            }
        }
        let require_authenticated_peer = !disable_verification;
        let tls = moq_native_ietf::tls::Args {
            root: root_certificates,
            client_cert: client_certificate,
            client_key: client_private_key,
            disable_verify: disable_verification,
            ..Default::default()
        }
        .load()
        .map_err(|_| MoqError::TlsConfiguration("MOQT TLS configuration could not be loaded"))?;
        if tls.verifies_server_certificates() != require_authenticated_peer {
            return Err(MoqError::TlsConfiguration(
                "MOQT TLS verification posture does not match the requested mode",
            ));
        }
        let config = moq_native_ietf::quic::Config::new(bind, None, tls)
            .map_err(|_| MoqError::TlsConfiguration("QUIC client configuration failed"))?;
        let endpoint = moq_native_ietf::quic::Endpoint::new(config)
            .map_err(|_| MoqError::TlsConfiguration("QUIC client bind failed"))?;
        if endpoint.verifies_server_certificates() != require_authenticated_peer
            || endpoint.client_auth_mode() != moq_native_ietf::tls::ClientAuthMode::Disabled
            || endpoint.server.is_some()
            || endpoint.writes_per_connection_diagnostics()
            || endpoint.tls_key_logging_enabled()
        {
            return Err(MoqError::TlsConfiguration(
                "MOQT endpoint security evidence does not match the origin-client profile",
            ));
        }
        Ok(Self {
            client: endpoint.client,
            require_authenticated_peer,
        })
    }

    pub(crate) fn bind_server_authenticated(
        bind: SocketAddr,
        root_certificates: Vec<PathBuf>,
    ) -> Result<Self, MoqError> {
        Self::bind(
            bind,
            root_certificates,
            None,
            None,
            false,
            WireTlsMode::ProductionServerAuthenticated,
        )
    }

    pub(crate) fn client(&self) -> &moq_native_ietf::quic::Client {
        &self.client
    }

    pub(crate) const fn requires_authenticated_peer(&self) -> bool {
        self.require_authenticated_peer
    }
}

pub(crate) struct WireRelayPublication {
    pub(crate) connection_id: String,
    pub(crate) relay_path: &'static str,
    pub(crate) endpoint_uri: String,
    pub(crate) substrate: BroadcastSubstrate,
    pub(crate) negotiated_protocol: String,
    pub(crate) peer_identity: MoqRelayPeerIdentity,
    session_task: Option<JoinHandle<WireRelayTermination>>,
    publish_task: Option<JoinHandle<WireRelayTermination>>,
    runtime: tokio::runtime::Handle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WireRelayTermination {
    Completed,
    Failed(MoqRelayFailure),
}

struct PendingSessionTask {
    task: Option<JoinHandle<WireRelayTermination>>,
    runtime: tokio::runtime::Handle,
}

impl PendingSessionTask {
    fn new(task: JoinHandle<WireRelayTermination>, runtime: tokio::runtime::Handle) -> Self {
        Self {
            task: Some(task),
            runtime,
        }
    }

    fn task_mut(&mut self) -> &mut JoinHandle<WireRelayTermination> {
        self.task
            .as_mut()
            .expect("pending MOQT session task already consumed")
    }

    fn take(&mut self) -> JoinHandle<WireRelayTermination> {
        self.task
            .take()
            .expect("pending MOQT session task already consumed")
    }

    async fn abort_and_join(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
    }
}

impl Drop for PendingSessionTask {
    fn drop(&mut self) {
        let Some(task) = self.task.take() else {
            return;
        };
        task.abort();
        let _cleanup = self.runtime.spawn(async move {
            let _ = task.await;
        });
    }
}

impl Drop for WireRelayPublication {
    fn drop(&mut self) {
        let session = self.session_task.take();
        let publication = self.publish_task.take();
        if session.is_none() && publication.is_none() {
            return;
        }
        if let Some(task) = &session {
            task.abort();
        }
        if let Some(task) = &publication {
            task.abort();
        }
        let _cleanup = self.runtime.spawn(async move {
            if let Some(task) = session {
                let _ = task.await;
            }
            if let Some(task) = publication {
                let _ = task.await;
            }
        });
    }
}

impl WireRelayPublication {
    pub(crate) async fn terminated(&mut self) -> WireRelayTermination {
        enum Completed {
            Session(Result<WireRelayTermination, tokio::task::JoinError>),
            Publication(Result<WireRelayTermination, tokio::task::JoinError>),
        }
        let completed = {
            let session = self
                .session_task
                .as_mut()
                .expect("MOQT session task already consumed");
            let publication = self
                .publish_task
                .as_mut()
                .expect("MOQT publication task already consumed");
            tokio::select! {
                result = session => Completed::Session(result),
                result = publication => Completed::Publication(result),
            }
        };
        match completed {
            Completed::Session(result) => {
                self.session_task.take();
                abort_and_join_wire_task(&mut self.publish_task).await;
                result.unwrap_or(WireRelayTermination::Failed(MoqRelayFailure::TaskFailed))
            }
            Completed::Publication(result) => {
                self.publish_task.take();
                abort_and_join_wire_task(&mut self.session_task).await;
                result.unwrap_or(WireRelayTermination::Failed(MoqRelayFailure::TaskFailed))
            }
        }
    }

    pub(crate) async fn close(&mut self) {
        self.abort_and_join().await;
    }

    async fn abort_and_join(&mut self) {
        abort_and_join_wire_task(&mut self.publish_task).await;
        abort_and_join_wire_task(&mut self.session_task).await;
    }
}

async fn abort_and_join_wire_task(task: &mut Option<JoinHandle<WireRelayTermination>>) {
    if let Some(task) = task.take() {
        task.abort();
        let _ = task.await;
    }
}

pub(crate) async fn publish_to_relay(
    publication: &WirePublicationHandle,
    client: &WireRelayClient,
    relay: &Url,
    substrate_policy: MoqRelaySubstratePolicy,
    acceptance_timeout: Duration,
) -> Result<WireRelayPublication, MoqError> {
    let runtime =
        tokio::runtime::Handle::try_current().map_err(|_| MoqError::RuntimeUnavailable)?;
    let (target, substrate_policy) = canonical_session_target(relay, substrate_policy)?;
    let endpoint_uri = target.network_url().to_string();
    let substrate_policy = native_substrate_policy(substrate_policy);
    let connection = client
        .client
        .connect_target(&target, substrate_policy, None)
        .await
        .map_err(|_| MoqError::RelayFailure(MoqRelayFailure::ConnectFailed))?;
    let peer_identity = map_peer_identity(connection.peer_identity);
    admit_relay_peer(&peer_identity, client.require_authenticated_peer)?;
    let negotiated = connection.negotiated;
    if negotiated.protocol != MOQT_NEGOTIATED_PROTOCOL {
        return Err(MoqError::NegotiatedProtocolMismatch {
            expected: MOQT_NEGOTIATED_PROTOCOL,
            negotiated: negotiated.protocol.to_owned(),
        });
    }
    let (relay_path, substrate) = match negotiated.substrate {
        Transport::RawQuic => ("raw-quic", BroadcastSubstrate::RawQuic),
        Transport::WebTransport => ("webtransport", BroadcastSubstrate::WebTransport),
    };
    let negotiated_protocol = negotiated.protocol.to_owned();
    let connection_id = connection.connection_id;
    let (session, mut publisher) =
        moq_transport::session::Publisher::connect(connection.session, negotiated)
            .await
            .map_err(|_| MoqError::RelayFailure(MoqRelayFailure::ConnectFailed))?;
    let session_task = tokio::spawn(async move {
        let _ = session.run().await;
        WireRelayTermination::Failed(MoqRelayFailure::SessionEnded)
    });
    let mut pending_session = PendingSessionTask::new(session_task, runtime.clone());
    let tracks = publication.tracks_reader.clone();
    let publish = match publisher
        .publish_namespace_open(tracks.info.namespace.clone())
        .await
    {
        Ok(publish) => publish,
        Err(_) => {
            pending_session.abort_and_join().await;
            return Err(MoqError::RelayFailure(MoqRelayFailure::PublicationEnded));
        }
    };
    let accepted = tokio::select! {
        biased;
        result = publish.accepted_with_timeout(acceptance_timeout) => result,
        result = pending_session.task_mut() => {
            let _ = result;
            drop(pending_session.take());
            drop(publish);
            return Err(MoqError::RelayFailure(MoqRelayFailure::SessionEnded));
        }
    };
    if let Err(error) = accepted {
        drop(publish);
        pending_session.abort_and_join().await;
        return Err(map_publish_namespace_acceptance_error(error));
    }
    if pending_session.task_mut().is_finished() {
        let failure = pending_session
            .take()
            .await
            .unwrap_or(WireRelayTermination::Failed(MoqRelayFailure::TaskFailed));
        drop(publish);
        return Err(MoqError::RelayFailure(match failure {
            WireRelayTermination::Completed => MoqRelayFailure::PublicationEnded,
            WireRelayTermination::Failed(failure) => failure,
        }));
    }
    let publish_task = tokio::spawn(async move {
        match publish.serve(tracks).await {
            Ok(()) => WireRelayTermination::Completed,
            Err(_) => WireRelayTermination::Failed(MoqRelayFailure::PublicationEnded),
        }
    });
    Ok(WireRelayPublication {
        connection_id,
        relay_path,
        endpoint_uri,
        substrate,
        negotiated_protocol,
        peer_identity,
        session_task: Some(pending_session.take()),
        publish_task: Some(publish_task),
        runtime,
    })
}

pub(crate) fn canonical_session_target(
    relay: &Url,
    substrate_policy: MoqRelaySubstratePolicy,
) -> Result<(SessionTarget, MoqRelaySubstratePolicy), MoqError> {
    match relay.scheme() {
        "moqt" => Ok((
            SessionTarget::try_from_url(relay.clone()).map_err(|_| MoqError::InvalidRelayTarget)?,
            substrate_policy,
        )),
        "https" => {
            tracing::warn!(
                "https:// MOQT relay inputs are deprecated; use a canonical moqt:// target and an explicit WebTransport policy"
            );
            Ok((
                SessionTarget::from_webtransport_url(relay)
                    .map_err(|_| MoqError::InvalidRelayTarget)?,
                MoqRelaySubstratePolicy::WebTransport,
            ))
        }
        _ => Err(MoqError::InvalidRelayTarget),
    }
}

pub(crate) fn native_substrate_policy(
    substrate_policy: MoqRelaySubstratePolicy,
) -> moq_native_ietf::quic::SubstratePolicy {
    match substrate_policy {
        MoqRelaySubstratePolicy::Auto => moq_native_ietf::quic::SubstratePolicy::Auto,
        MoqRelaySubstratePolicy::RawQuic => moq_native_ietf::quic::SubstratePolicy::RawQuic,
        MoqRelaySubstratePolicy::WebTransport => {
            moq_native_ietf::quic::SubstratePolicy::WebTransport
        }
    }
}

pub(crate) fn map_peer_identity(
    identity: moq_native_ietf::tls::PeerIdentity,
) -> MoqRelayPeerIdentity {
    let certificate_fields = |identity: &moq_native_ietf::tls::CertificateIdentity| {
        (
            identity.leaf_sha256_hex(),
            identity.chain_len(),
            identity.total_der_bytes(),
        )
    };
    match identity {
        moq_native_ietf::tls::PeerIdentity::Anonymous => MoqRelayPeerIdentity::Anonymous,
        moq_native_ietf::tls::PeerIdentity::UnverifiedCertificate(identity) => {
            let (leaf_sha256, chain_len, total_der_bytes) = certificate_fields(&identity);
            MoqRelayPeerIdentity::UnverifiedCertificate {
                leaf_sha256,
                chain_len,
                total_der_bytes,
            }
        }
        moq_native_ietf::tls::PeerIdentity::Certificate(identity) => {
            let (leaf_sha256, chain_len, total_der_bytes) = certificate_fields(&identity);
            MoqRelayPeerIdentity::VerifiedCertificate {
                leaf_sha256,
                chain_len,
                total_der_bytes,
            }
        }
    }
}

pub(crate) fn admit_relay_peer(
    identity: &MoqRelayPeerIdentity,
    require_authenticated_peer: bool,
) -> Result<(), MoqError> {
    if require_authenticated_peer && !identity.is_authenticated() {
        return Err(MoqError::RelayPeerUnauthenticated);
    }
    Ok(())
}

fn map_publish_namespace_acceptance_error(error: PublishNamespaceAcceptanceError) -> MoqError {
    match error {
        PublishNamespaceAcceptanceError::Rejected(rejection) => {
            MoqError::PublishNamespaceRejected {
                error_code: rejection.error_code,
                retry_interval: rejection.retry_interval,
                reason: rejection.reason.0,
            }
        }
        PublishNamespaceAcceptanceError::TimedOut { timeout } => {
            MoqError::PublishNamespaceAcceptanceTimedOut { timeout }
        }
        PublishNamespaceAcceptanceError::ResponseStreamClosed => {
            MoqError::PublishNamespaceResponseStreamClosed
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bytes::Bytes;
    use moq_transport::coding::ReasonPhrase;
    use moq_transport::coding::Value;
    use moq_transport::serve::TrackReaderMode;
    use moq_transport::session::PublishNamespaceRejection;

    use super::*;
    use crate::{LOC_TIMESCALE_PROPERTY, LOC_TIMESTAMP_PROPERTY};

    struct TestPki {
        directory: PathBuf,
        certificate: PathBuf,
        private_key: PathBuf,
    }

    impl TestPki {
        fn new() -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir().join(format!(
                "rvoip-moq-opaque-tls-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir_all(&directory).unwrap();
            let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let certificate = directory.join("identity.pem");
            let private_key = directory.join("identity.key");
            std::fs::write(&certificate, generated.cert.pem()).unwrap();
            std::fs::write(&private_key, generated.signing_key.serialize_pem()).unwrap();
            Self {
                directory,
                certificate,
                private_key,
            }
        }

        fn certificate(&self) -> &Path {
            &self.certificate
        }

        fn private_key(&self) -> &Path {
            &self.private_key
        }
    }

    impl Drop for TestPki {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn opaque_native_tls_configuration_is_the_only_wire_construction_path() {
        let implementation = include_str!("wire.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap();
        assert!(implementation.contains("moq_native_ietf::tls::Args"));
        assert!(!implementation.contains("moq_native_ietf::tls::Config {"));
        assert!(!implementation.contains("rustls::ClientConfig"));
    }

    #[tokio::test]
    async fn opaque_tls_builder_preserves_the_production_mtls_posture() {
        let pki = TestPki::new();
        let client = WireRelayClient::bind(
            "127.0.0.1:0".parse().unwrap(),
            vec![pki.certificate().to_owned()],
            Some(pki.certificate().to_owned()),
            Some(pki.private_key().to_owned()),
            false,
            WireTlsMode::ProductionMutualTls,
        )
        .unwrap();
        assert!(client.require_authenticated_peer);
        assert!(client.client.verifies_server_certificates());
    }

    #[test]
    fn verified_tls_identity_is_required_outside_insecure_development() {
        let anonymous = MoqRelayPeerIdentity::Anonymous;
        let unverified = MoqRelayPeerIdentity::UnverifiedCertificate {
            leaf_sha256: "22".repeat(32),
            chain_len: 1,
            total_der_bytes: 512,
        };
        let verified = MoqRelayPeerIdentity::VerifiedCertificate {
            leaf_sha256: "33".repeat(32),
            chain_len: 1,
            total_der_bytes: 512,
        };

        assert!(matches!(
            admit_relay_peer(&anonymous, true),
            Err(MoqError::RelayPeerUnauthenticated)
        ));
        assert!(matches!(
            admit_relay_peer(&unverified, true),
            Err(MoqError::RelayPeerUnauthenticated)
        ));
        admit_relay_peer(&verified, true).unwrap();
        admit_relay_peer(&anonymous, false).unwrap();
        admit_relay_peer(&unverified, false).unwrap();
    }

    #[test]
    fn canonical_target_keeps_substrate_policy_separate() {
        let relay = Url::parse("moqt://Relay.Example:4443/live?q=1").unwrap();
        let (target, policy) =
            canonical_session_target(&relay, MoqRelaySubstratePolicy::Auto).unwrap();
        assert_eq!(target.to_string(), "moqt://relay.example:4443/live?q=1");
        assert_eq!(policy, MoqRelaySubstratePolicy::Auto);
        assert!(matches!(
            native_substrate_policy(policy),
            moq_native_ietf::quic::SubstratePolicy::Auto
        ));
    }

    #[test]
    fn legacy_https_target_is_canonicalized_and_forces_webtransport() {
        let relay = Url::parse("https://Relay.Example:4443/live?q=1").unwrap();
        let (target, policy) =
            canonical_session_target(&relay, MoqRelaySubstratePolicy::RawQuic).unwrap();
        assert_eq!(target.to_string(), "moqt://relay.example:4443/live?q=1");
        assert_eq!(policy, MoqRelaySubstratePolicy::WebTransport);
        assert!(matches!(
            canonical_session_target(
                &Url::parse("wss://relay.example/live").unwrap(),
                MoqRelaySubstratePolicy::Auto,
            ),
            Err(MoqError::InvalidRelayTarget)
        ));
    }

    #[test]
    fn canonical_targets_reject_userinfo_without_leaking_credentials() {
        for relay in [
            "moqt://user:password@relay.example/live",
            "https://user:password@relay.example/live",
        ] {
            let error = canonical_session_target(
                &Url::parse(relay).unwrap(),
                MoqRelaySubstratePolicy::Auto,
            )
            .unwrap_err();
            assert!(matches!(error, MoqError::InvalidRelayTarget));
            let diagnostic = format!("{error:?} {error}");
            assert!(!diagnostic.contains("user"));
            assert!(!diagnostic.contains("password"));
        }
    }

    #[test]
    fn namespace_rejection_retains_bounded_wire_details_in_rvoip_types() {
        let error = map_publish_namespace_acceptance_error(
            PublishNamespaceAcceptanceError::Rejected(PublishNamespaceRejection {
                error_code: 0x1,
                retry_interval: 250,
                reason: ReasonPhrase("denied".into()),
                redirect: None,
            }),
        );
        assert!(matches!(
            error,
            MoqError::PublishNamespaceRejected {
                error_code: 0x1,
                retry_interval: 250,
                reason,
            } if reason == "denied"
        ));
    }

    #[test]
    fn namespace_silence_maps_to_a_typed_acceptance_timeout() {
        let timeout = Duration::from_millis(25);
        assert!(matches!(
            map_publish_namespace_acceptance_error(
                PublishNamespaceAcceptanceError::TimedOut { timeout }
            ),
            MoqError::PublishNamespaceAcceptanceTimedOut { timeout: actual }
                if actual == timeout
        ));
    }

    #[test]
    fn namespace_response_disconnect_maps_to_a_distinct_error() {
        assert!(matches!(
            map_publish_namespace_acceptance_error(
                PublishNamespaceAcceptanceError::ResponseStreamClosed
            ),
            MoqError::PublishNamespaceResponseStreamClosed
        ));
    }

    #[tokio::test]
    async fn cancelled_acceptance_aborts_and_reaps_the_pending_session_task() {
        struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

        impl Drop for DropSignal {
            fn drop(&mut self) {
                if let Some(signal) = self.0.take() {
                    let _ = signal.send(());
                }
            }
        }

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (dropped_tx, dropped_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _drop_signal = DropSignal(Some(dropped_tx));
            let _ = started_tx.send(());
            std::future::pending::<WireRelayTermination>().await
        });
        started_rx.await.unwrap();

        let pending = PendingSessionTask::new(task, tokio::runtime::Handle::current());
        drop(pending);

        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("dropping a pending connection must abort its session task")
            .expect("session task drop signal must remain connected");
    }

    #[test]
    fn default_wire_publication_does_not_create_an_events_track() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let (publication, _catalog, _audio) = WirePublication::new(&namespace).unwrap();
        let wire_namespace = publication.tracks_reader.info.namespace.clone();
        let mut tracks = publication.tracks_for_test();

        assert!(tracks
            .get_track_reader(&wire_namespace, TrackName::from(EVENTS_TRACK))
            .is_none());
    }

    #[tokio::test]
    async fn opt_in_events_use_independent_object_zero_groups_and_end_before_catalog() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let (publication, mut catalog_writer, audio_writer, mut events_writer) =
            WirePublication::new_with_sanitized_events(&namespace).unwrap();
        let wire_namespace = publication.tracks_reader.info.namespace.clone();
        let mut tracks = publication.tracks_for_test();
        let events_track = tracks
            .get_track_reader(&wire_namespace, TrackName::from(EVENTS_TRACK))
            .expect("explicit opt-in must create the event track");

        catalog_writer.write_live(7, b"live".to_vec()).unwrap();
        let payload = br#"[{"t":1000,"data":{"version":"io.rvoip.sanitized-call-events.v1","sequence":1,"kind":"call-connected"}}]"#.to_vec();
        events_writer.write(11, payload.clone()).unwrap();

        let mut events = match events_track.mode().await.unwrap() {
            TrackReaderMode::Subgroups(reader) => reader,
            _ => panic!("event track must use subgroup streams"),
        };
        let mut group = events.next().await.unwrap().unwrap();
        assert_eq!((group.group_id, group.subgroup_id), (11, 0));
        assert!(group.first_object);
        assert!(group.end_of_group);
        let mut object = group.next().await.unwrap().unwrap();
        assert_eq!(object.object_id, 0);
        assert!(object.extension_headers.is_empty());
        assert_eq!(object.read_all().await.unwrap(), payload);
        assert!(group.next().await.unwrap().is_none());

        events_writer.finish().unwrap();
        tokio::time::timeout(Duration::from_millis(100), events_track.closed())
            .await
            .expect("event track must close before audio completion")
            .unwrap();
        audio_writer.finish().unwrap();
        catalog_writer
            .write_terminal(12, b"terminal".to_vec())
            .unwrap();
        catalog_writer.finish().unwrap();
        assert!(publication.is_cleanly_completed_for_test());
    }

    #[tokio::test]
    async fn msf_and_loc_objects_use_one_first_object_end_of_group_stream_each() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let (publication, mut catalog_writer, mut audio_writer) =
            WirePublication::new(&namespace).unwrap();
        let wire_namespace = publication.tracks_reader.info.namespace.clone();
        let mut tracks = publication.tracks_for_test();
        let audio_track = tracks
            .get_track_reader(&wire_namespace, TrackName::from(AUDIO_TRACK))
            .unwrap();
        let catalog_track = tracks
            .get_track_reader(&wire_namespace, TrackName::from(CATALOG_TRACK))
            .unwrap();

        let live_catalog = br#"{"version":"draft-01","tracks":[{}]}"#.to_vec();
        catalog_writer.write_live(7, live_catalog.clone()).unwrap();
        let mut catalog = match catalog_track.mode().await.unwrap() {
            TrackReaderMode::Subgroups(reader) => reader,
            _ => panic!("catalog track must use subgroup streams"),
        };
        let mut live_stream = catalog.next().await.unwrap().unwrap();
        assert_eq!((live_stream.group_id, live_stream.subgroup_id), (7, 0));
        assert!(live_stream.first_object);
        assert!(live_stream.end_of_group);
        let mut live_object = live_stream.next().await.unwrap().unwrap();
        assert_eq!(live_object.object_id, 0);
        assert_eq!(live_object.read_all().await.unwrap(), live_catalog);
        assert!(live_object.extension_headers.is_empty());
        assert!(live_stream.next().await.unwrap().is_none());

        audio_writer
            .write(LocAudioObject {
                group_id: 4,
                object_id: 0,
                timestamp: 960,
                timescale: 48_000,
                payload: Bytes::from_static(&[0x78, 0x00]),
            })
            .unwrap();
        let mut audio = match audio_track.mode().await.unwrap() {
            TrackReaderMode::Subgroups(reader) => reader,
            _ => panic!("audio track must use subgroup streams"),
        };
        let mut audio_stream = audio.next().await.unwrap().unwrap();
        assert_eq!((audio_stream.group_id, audio_stream.subgroup_id), (4, 0));
        assert!(audio_stream.first_object);
        assert!(audio_stream.end_of_group);
        let mut object = audio_stream.next().await.unwrap().unwrap();
        assert_eq!(object.object_id, 0);
        assert_eq!(
            object
                .extension_headers
                .get(LOC_TIMESTAMP_PROPERTY)
                .unwrap()
                .value,
            Value::IntValue(960)
        );
        assert_eq!(
            object
                .extension_headers
                .get(LOC_TIMESCALE_PROPERTY)
                .unwrap()
                .value,
            Value::IntValue(48_000)
        );
        // The obsolete LOC timestamp property ID must not be emitted.
        assert!(!object.extension_headers.has(0x06));
        assert_eq!(
            object.read_all().await.unwrap(),
            Bytes::from_static(&[0x78, 0x00])
        );
        assert!(audio_stream.next().await.unwrap().is_none());

        audio_writer.finish().unwrap();
        tokio::time::timeout(Duration::from_millis(100), audio_track.closed())
            .await
            .expect("audio track must end before terminal catalog")
            .unwrap();

        let terminal_catalog = br#"{"version":"draft-01","isComplete":true,"tracks":[]}"#.to_vec();
        catalog_writer
            .write_terminal(8, terminal_catalog.clone())
            .unwrap();
        let mut terminal_stream = catalog.next().await.unwrap().unwrap();
        assert_eq!(
            (terminal_stream.group_id, terminal_stream.subgroup_id),
            (8, 0)
        );
        assert!(terminal_stream.first_object);
        assert!(terminal_stream.end_of_group);
        let mut terminal_object = terminal_stream.next().await.unwrap().unwrap();
        assert_eq!(terminal_object.object_id, 0);
        assert_eq!(terminal_object.read_all().await.unwrap(), terminal_catalog);
        assert!(terminal_stream.next().await.unwrap().is_none());

        catalog_writer.finish().unwrap();
        tokio::time::timeout(Duration::from_millis(100), catalog_track.closed())
            .await
            .expect("catalog track must end after its terminal update")
            .unwrap();
        assert!(publication.is_cleanly_completed_for_test());
    }

    #[test]
    fn wire_writers_reject_out_of_order_terminal_completion() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let (_publication, mut catalog, mut audio) = WirePublication::new(&namespace).unwrap();

        assert!(audio
            .write(LocAudioObject {
                group_id: 0,
                object_id: 0,
                timestamp: 0,
                timescale: 48_000,
                payload: Bytes::from_static(&[0x78, 0x00]),
            })
            .is_err());
        catalog.write_live(0, b"live".to_vec()).unwrap();
        assert!(catalog.write_terminal(1, b"terminal".to_vec()).is_err());
        audio.finish().unwrap();
        catalog.write_terminal(1, b"terminal".to_vec()).unwrap();
        catalog.finish().unwrap();
    }

    #[test]
    fn maximum_group_id_is_safe_at_the_wire_publication_boundary() {
        let namespace = MoqNamespace::new("tenant", "maximum-group").unwrap();
        let (publication, mut catalog, mut audio) = WirePublication::new(&namespace).unwrap();
        catalog.write_live(0, b"live".to_vec()).unwrap();
        audio
            .write(LocAudioObject {
                group_id: u64::MAX,
                object_id: 0,
                timestamp: 0,
                timescale: 48_000,
                payload: Bytes::from_static(&[0x78, 0x00]),
            })
            .unwrap();
        audio.finish().unwrap();
        catalog
            .write_terminal(u64::MAX, b"terminal".to_vec())
            .unwrap();
        catalog.finish().unwrap();
        assert!(publication.is_cleanly_completed_for_test());
    }

    #[tokio::test]
    async fn production_catalog_subscriber_uses_verified_server_auth_without_a_client_identity() {
        let pki = TestPki::new();
        let subscriber = WireRelayClient::bind_server_authenticated(
            "127.0.0.1:0".parse().unwrap(),
            vec![pki.certificate().to_owned()],
        )
        .unwrap();
        assert!(subscriber.requires_authenticated_peer());
        assert!(subscriber.client().verifies_server_certificates());
    }

    #[cfg(feature = "insecure-development")]
    #[tokio::test]
    async fn production_and_development_tls_postures_are_explicit() {
        let bind = "127.0.0.1:0".parse().unwrap();
        let pki = TestPki::new();
        assert!(matches!(
            WireRelayClient::bind(
                bind,
                Vec::new(),
                None,
                None,
                false,
                WireTlsMode::ProductionMutualTls,
            ),
            Err(MoqError::TlsConfiguration(_))
        ));
        assert!(matches!(
            WireRelayClient::bind(
                bind,
                Vec::new(),
                Some(PathBuf::from("/secret/client.pem")),
                Some(PathBuf::from("/secret/client.key")),
                false,
                WireTlsMode::DevelopmentServerAuthenticated,
            ),
            Err(MoqError::TlsConfiguration(_))
        ));
        assert!(matches!(
            WireRelayClient::bind(
                bind,
                Vec::new(),
                None,
                None,
                false,
                WireTlsMode::DevelopmentInsecure,
            ),
            Err(MoqError::TlsConfiguration(_))
        ));

        let server_authenticated = WireRelayClient::bind(
            bind,
            vec![pki.certificate().to_owned()],
            None,
            None,
            false,
            WireTlsMode::DevelopmentServerAuthenticated,
        )
        .unwrap();
        assert!(server_authenticated.require_authenticated_peer);
        assert!(server_authenticated.client.verifies_server_certificates());

        let insecure = WireRelayClient::bind(
            bind,
            vec![pki.certificate().to_owned()],
            None,
            None,
            true,
            WireTlsMode::DevelopmentInsecure,
        )
        .unwrap();
        assert!(!insecure.require_authenticated_peer);
        assert!(!insecure.client.verifies_server_certificates());
    }

    #[test]
    fn tls_file_errors_do_not_expose_secret_paths() {
        let error = WireRelayClient::bind(
            "127.0.0.1:0".parse().unwrap(),
            vec![PathBuf::from("/secret/relay-ca.pem")],
            Some(PathBuf::from("/secret/client.pem")),
            Some(PathBuf::from("/secret/client.key")),
            false,
            WireTlsMode::ProductionMutualTls,
        )
        .err()
        .expect("missing credentials must fail");
        let rendered = error.to_string();
        assert!(!rendered.contains("/secret"));
        assert!(!rendered.contains("client.key"));
    }
}
