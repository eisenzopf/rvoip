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
use tokio::task::JoinHandle;
use url::Url;

use crate::{LocAudioObject, MoqError, MoqNamespace, MoqRelayFailure, AUDIO_TRACK, CATALOG_TRACK};

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
    session_task: Option<JoinHandle<MoqRelayFailure>>,
    publish_task: Option<JoinHandle<MoqRelayFailure>>,
    runtime: tokio::runtime::Handle,
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
    transport_stability_grace: Duration,
) -> Result<WireRelayPublication, MoqError> {
    let runtime =
        tokio::runtime::Handle::try_current().map_err(|_| MoqError::RuntimeUnavailable)?;
    let (wire, connection_id, transport) = client
        .client
        .connect(relay, None)
        .await
        .map_err(|_| MoqError::RelayFailure(MoqRelayFailure::ConnectFailed))?;
    let relay_path = match transport {
        moq_transport::session::Transport::RawQuic => "raw-quic",
        moq_transport::session::Transport::WebTransport => "webtransport",
    };
    let (session, mut publisher) = moq_transport::session::Publisher::connect(wire, transport)
        .await
        .map_err(|_| MoqError::RelayFailure(MoqRelayFailure::ConnectFailed))?;
    let session_task = tokio::spawn(async move {
        let _ = session.run().await;
        MoqRelayFailure::SessionEnded
    });
    let tracks = publication.tracks_reader.clone();
    let publish_task = tokio::spawn(async move {
        let _ = publisher.publish_namespace(tracks).await;
        MoqRelayFailure::PublicationEnded
    });
    let mut publication = WireRelayPublication {
        connection_id,
        relay_path,
        session_task: Some(session_task),
        publish_task: Some(publish_task),
        runtime,
    };

    // The pinned fork does not yet expose REQUEST_OK separately from the
    // long-running publish_namespace future. Treat a session plus publication
    // that both remain live for this explicit grace interval as transport
    // stable. This is not protocol readiness because REQUEST_OK remains
    // unobservable. An early completion is surfaced as failure.
    tokio::select! {
        failure = publication.terminated() => Err(MoqError::RelayFailure(failure)),
        () = tokio::time::sleep(transport_stability_grace) => Ok(publication),
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use moq_transport::coding::Value;
    use moq_transport::serve::TrackReaderMode;

    use super::*;
    use crate::{LOC_TIMESCALE_PROPERTY, LOC_TIMESTAMP_PROPERTY};

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
