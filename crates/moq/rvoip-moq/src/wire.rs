//! Draft-specific moq-rs adapter.
//!
//! Nothing in this module is re-exported. In particular, `moq_transport`
//! readers, writers, sessions, and errors cannot appear in rvoip-moq's public
//! signatures.

use std::io::BufReader;
use std::net::SocketAddr;
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use moq_transport::coding::{TrackName, TrackNamespace};
use moq_transport::data::ExtensionHeaders;
use moq_transport::serve::{
    Datagram, DatagramsWriter, Tracks, TracksReader, TracksRequest, TracksWriter,
};
use moq_transport::session::{PublishNamespaceAcceptanceError, SessionTarget, Transport};
use rvoip_core_traits::broadcast::BroadcastSubstrate;
use tokio::task::JoinHandle;
use url::Url;

use crate::{
    LocAudioObject, MoqError, MoqNamespace, MoqRelayFailure, MoqRelaySubstratePolicy, AUDIO_TRACK,
    CATALOG_TRACK, MOQT_NEGOTIATED_PROTOCOL,
};

pub(crate) struct WirePublication {
    tracks_reader: TracksReader,
    control: Mutex<Option<WireControl>>,
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
    audio: DatagramsWriter,
    // Retain the writer so the independent catalog remains available to late
    // subscribers for the lifetime of the audio publication.
    _catalog: DatagramsWriter,
    #[cfg(test)]
    fail_writes: Arc<AtomicBool>,
}

impl WirePublication {
    pub(crate) fn new(
        namespace: &MoqNamespace,
        catalog_payload: Vec<u8>,
    ) -> Result<(Self, WireAudioWriter), MoqError> {
        let wire_namespace = TrackNamespace::from_utf8_path(namespace.as_str());
        let (mut tracks_writer, tracks_request, tracks_reader) =
            Tracks::new(wire_namespace).produce();

        let audio_track = tracks_writer
            .create(TrackName::from(AUDIO_TRACK))
            .ok_or(MoqError::Closed)?;
        let audio = audio_track
            .datagrams()
            .map_err(|error| MoqError::Wire(error.to_string()))?;

        let catalog_track = tracks_writer
            .create(TrackName::from(CATALOG_TRACK))
            .ok_or(MoqError::Closed)?;
        let mut catalog = catalog_track
            .datagrams()
            .map_err(|error| MoqError::Wire(error.to_string()))?;
        catalog
            .write(Datagram {
                group_id: 0,
                object_id: 0,
                priority: 0,
                payload: catalog_payload.into(),
                extension_headers: ExtensionHeaders::new(),
            })
            .map_err(|error| MoqError::Wire(error.to_string()))?;
        #[cfg(test)]
        let fail_writes = Arc::new(AtomicBool::new(false));

        Ok((
            Self {
                tracks_reader,
                control: Mutex::new(Some(WireControl {
                    _tracks_writer: tracks_writer,
                    _tracks_request: tracks_request,
                })),
                #[cfg(test)]
                fail_writes: Arc::clone(&fail_writes),
            },
            WireAudioWriter {
                audio,
                _catalog: catalog,
                #[cfg(test)]
                fail_writes,
            },
        ))
    }

    pub(crate) fn close(&self) {
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
        self.audio
            .write(Datagram {
                group_id: object.group_id,
                object_id: object.object_id,
                priority: 0,
                payload: object.payload,
                extension_headers,
            })
            .map_err(|error| MoqError::Wire(error.to_string()))
    }
}

#[derive(Clone)]
pub(crate) struct WireRelayClient {
    // Keep only the client half. The endpoint's optional server contains an
    // accept-future set which is intentionally not Sync and is unnecessary for
    // an origin publishing to a relay.
    client: moq_native_ietf::quic::Client,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WireTlsMode {
    ProductionMutualTls,
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
        let roots = root_certificates;
        let client = if let (Some(certificate), Some(private_key)) =
            (client_certificate, client_private_key)
        {
            let certificates = rustls_pemfile::certs(&mut BufReader::new(
                std::fs::File::open(certificate).map_err(|_| {
                    MoqError::TlsConfiguration("client certificate could not be read")
                })?,
            ))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| MoqError::TlsConfiguration("client certificate is invalid"))?;
            let key = rustls_pemfile::private_key(&mut BufReader::new(
                std::fs::File::open(private_key).map_err(|_| {
                    MoqError::TlsConfiguration("client private key could not be read")
                })?,
            ))
            .map_err(|_| MoqError::TlsConfiguration("client private key is invalid"))?
            .ok_or(MoqError::TlsConfiguration("client private key is empty"))?;
            let mut root_store = rustls::RootCertStore::empty();
            if roots.is_empty() {
                for certificate in rustls_native_certs::load_native_certs().map_err(|_| {
                    MoqError::TlsConfiguration("system trust roots could not be loaded")
                })? {
                    root_store
                        .add(certificate)
                        .map_err(|_| MoqError::TlsConfiguration("system trust root is invalid"))?;
                }
            } else {
                for root in roots {
                    for certificate in rustls_pemfile::certs(&mut BufReader::new(
                        std::fs::File::open(root).map_err(|_| {
                            MoqError::TlsConfiguration("relay trust root could not be read")
                        })?,
                    )) {
                        root_store
                            .add(certificate.map_err(|_| {
                                MoqError::TlsConfiguration("relay trust root is invalid")
                            })?)
                            .map_err(|_| {
                                MoqError::TlsConfiguration("relay trust root is invalid")
                            })?;
                    }
                }
            }
            rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|_| MoqError::TlsConfiguration("TLS 1.3 is unavailable"))?
            .with_root_certificates(root_store)
            .with_client_auth_cert(certificates, key)
            .map_err(|_| MoqError::TlsConfiguration("client credentials are invalid"))?
        } else {
            #[cfg(feature = "insecure-development")]
            {
                moq_native_ietf::tls::Args {
                    root: roots,
                    disable_verify: disable_verification,
                    ..Default::default()
                }
                .load()
                .map_err(|_| MoqError::TlsConfiguration("relay trust roots could not be loaded"))?
                .client
            }
            #[cfg(not(feature = "insecure-development"))]
            {
                let _ = (roots, disable_verification);
                return Err(MoqError::TlsConfiguration(
                    "production relay connections require client credentials",
                ));
            }
        };
        let config = moq_native_ietf::quic::Config::new(
            bind,
            None,
            moq_native_ietf::tls::Config {
                client,
                server: None,
                fingerprints: Vec::new(),
            },
        )
        .map_err(|_| MoqError::TlsConfiguration("QUIC client configuration failed"))?;
        let endpoint = moq_native_ietf::quic::Endpoint::new(config)
            .map_err(|_| MoqError::TlsConfiguration("QUIC client bind failed"))?;
        Ok(Self {
            client: endpoint.client,
        })
    }
}

pub(crate) struct WireRelayPublication {
    pub(crate) connection_id: String,
    pub(crate) relay_path: &'static str,
    pub(crate) endpoint_uri: String,
    pub(crate) substrate: BroadcastSubstrate,
    pub(crate) negotiated_protocol: String,
    session_task: Option<JoinHandle<MoqRelayFailure>>,
    publish_task: Option<JoinHandle<MoqRelayFailure>>,
    runtime: tokio::runtime::Handle,
}

struct PendingSessionTask {
    task: Option<JoinHandle<MoqRelayFailure>>,
    runtime: tokio::runtime::Handle,
}

impl PendingSessionTask {
    fn new(task: JoinHandle<MoqRelayFailure>, runtime: tokio::runtime::Handle) -> Self {
        Self {
            task: Some(task),
            runtime,
        }
    }

    fn task_mut(&mut self) -> &mut JoinHandle<MoqRelayFailure> {
        self.task
            .as_mut()
            .expect("pending MOQT session task already consumed")
    }

    fn take(&mut self) -> JoinHandle<MoqRelayFailure> {
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
    pub(crate) async fn terminated(&mut self) -> MoqRelayFailure {
        enum Completed {
            Session(Result<MoqRelayFailure, tokio::task::JoinError>),
            Publication(Result<MoqRelayFailure, tokio::task::JoinError>),
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
                result.unwrap_or(MoqRelayFailure::TaskFailed)
            }
            Completed::Publication(result) => {
                self.publish_task.take();
                abort_and_join_wire_task(&mut self.session_task).await;
                result.unwrap_or(MoqRelayFailure::TaskFailed)
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

async fn abort_and_join_wire_task(task: &mut Option<JoinHandle<MoqRelayFailure>>) {
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
        MoqRelayFailure::SessionEnded
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
            .unwrap_or(MoqRelayFailure::TaskFailed);
        drop(publish);
        return Err(MoqError::RelayFailure(failure));
    }
    let publish_task = tokio::spawn(async move {
        let _ = publish.serve(tracks).await;
        MoqRelayFailure::PublicationEnded
    });
    Ok(WireRelayPublication {
        connection_id,
        relay_path,
        endpoint_uri,
        substrate,
        negotiated_protocol,
        session_task: Some(pending_session.take()),
        publish_task: Some(publish_task),
        runtime,
    })
}

fn canonical_session_target(
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

fn native_substrate_policy(
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
    use bytes::Bytes;
    use moq_transport::coding::ReasonPhrase;
    use moq_transport::coding::Value;
    use moq_transport::serve::TrackReaderMode;
    use moq_transport::session::PublishNamespaceRejection;

    use super::*;
    use crate::{LOC_TIMESCALE_PROPERTY, LOC_TIMESTAMP_PROPERTY};

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
            std::future::pending::<MoqRelayFailure>().await
        });
        started_rx.await.unwrap();

        let pending = PendingSessionTask::new(task, tokio::runtime::Handle::current());
        drop(pending);

        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("dropping a pending connection must abort its session task")
            .expect("session task drop signal must remain connected");
    }

    #[tokio::test]
    async fn maps_rvoip_loc_properties_to_the_expected_wire_ids() {
        let namespace = MoqNamespace::new("tenant", "broadcast").unwrap();
        let (publication, mut audio_writer) =
            WirePublication::new(&namespace, br#"{"version":"draft-01"}"#.to_vec()).unwrap();
        let wire_namespace = publication.tracks_reader.info.namespace.clone();
        let mut tracks = publication.tracks();
        let audio = tracks
            .get_track_reader(&wire_namespace, TrackName::from(AUDIO_TRACK))
            .unwrap();
        let mut audio = match audio.mode().await.unwrap() {
            TrackReaderMode::Datagrams(reader) => reader,
            _ => panic!("audio track must use datagrams"),
        };

        audio_writer
            .write(LocAudioObject {
                group_id: 4,
                object_id: 0,
                timestamp: 960,
                timescale: 48_000,
                payload: Bytes::from_static(&[0x78, 0x00]),
            })
            .unwrap();
        let object = audio.read().await.unwrap().unwrap();
        assert_eq!((object.group_id, object.object_id), (4, 0));
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
    }

    #[cfg(feature = "insecure-development")]
    #[test]
    fn production_and_development_tls_postures_are_explicit() {
        let bind = "127.0.0.1:0".parse().unwrap();
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
