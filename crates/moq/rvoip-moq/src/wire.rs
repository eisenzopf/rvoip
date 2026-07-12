//! Draft-specific moq-rs adapter.
//!
//! Nothing in this module is re-exported. In particular, `moq_transport`
//! readers, writers, sessions, and errors cannot appear in rvoip-moq's public
//! signatures.

use std::io::BufReader;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use moq_transport::coding::{TrackName, TrackNamespace};
use moq_transport::data::ExtensionHeaders;
use moq_transport::serve::{
    Datagram, DatagramsWriter, Tracks, TracksReader, TracksRequest, TracksWriter,
};
use tokio::task::AbortHandle;
use url::Url;

use crate::{LocAudioObject, MoqError, MoqNamespace, AUDIO_TRACK, CATALOG_TRACK};

pub(crate) struct WirePublication {
    tracks_reader: TracksReader,
    control: Mutex<Option<WireControl>>,
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

        Ok((
            Self {
                tracks_reader,
                control: Mutex::new(Some(WireControl {
                    _tracks_writer: tracks_writer,
                    _tracks_request: tracks_request,
                })),
            },
            WireAudioWriter {
                audio,
                _catalog: catalog,
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
}

impl WireAudioWriter {
    pub(crate) fn write(&mut self, object: LocAudioObject) -> Result<(), MoqError> {
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

pub(crate) struct WireRelayClient {
    // Keep only the client half. The endpoint's optional server contains an
    // accept-future set which is intentionally not Sync and is unnecessary for
    // an origin publishing to a relay.
    client: moq_native_ietf::quic::Client,
}

impl WireRelayClient {
    pub(crate) fn bind(
        bind: SocketAddr,
        root_certificates: Vec<PathBuf>,
        client_certificate: Option<PathBuf>,
        client_private_key: Option<PathBuf>,
        disable_verification: bool,
    ) -> Result<Self, MoqError> {
        if client_certificate.is_some() != client_private_key.is_some() {
            return Err(MoqError::InvalidConfig(
                "MOQT client certificate and private key must be configured together",
            ));
        }
        let roots = root_certificates;
        let native_tls = moq_native_ietf::tls::Args {
            root: roots.clone(),
            disable_verify: disable_verification,
            ..Default::default()
        }
        .load()
        .map_err(|error| MoqError::Relay(error.to_string()))?;
        let client = if let (Some(certificate), Some(private_key)) =
            (client_certificate, client_private_key)
        {
            if disable_verification {
                return Err(MoqError::InvalidConfig(
                    "MOQT mTLS may not disable server certificate verification",
                ));
            }
            let certificates = rustls_pemfile::certs(&mut BufReader::new(
                std::fs::File::open(certificate)
                    .map_err(|error| MoqError::Relay(error.to_string()))?,
            ))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| MoqError::Relay(error.to_string()))?;
            let key = rustls_pemfile::private_key(&mut BufReader::new(
                std::fs::File::open(private_key)
                    .map_err(|error| MoqError::Relay(error.to_string()))?,
            ))
            .map_err(|error| MoqError::Relay(error.to_string()))?
            .ok_or(MoqError::InvalidConfig("MOQT client private key is empty"))?;
            let mut root_store = rustls::RootCertStore::empty();
            if roots.is_empty() {
                for certificate in rustls_native_certs::load_native_certs()
                    .map_err(|error| MoqError::Relay(error.to_string()))?
                {
                    root_store
                        .add(certificate)
                        .map_err(|error| MoqError::Relay(error.to_string()))?;
                }
            } else {
                for root in roots {
                    for certificate in rustls_pemfile::certs(&mut BufReader::new(
                        std::fs::File::open(root)
                            .map_err(|error| MoqError::Relay(error.to_string()))?,
                    )) {
                        root_store
                            .add(certificate.map_err(|error| MoqError::Relay(error.to_string()))?)
                            .map_err(|error| MoqError::Relay(error.to_string()))?;
                    }
                }
            }
            rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|error| MoqError::Relay(error.to_string()))?
            .with_root_certificates(root_store)
            .with_client_auth_cert(certificates, key)
            .map_err(|error| MoqError::Relay(error.to_string()))?
        } else {
            native_tls.client
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
        .map_err(|error| MoqError::Relay(error.to_string()))?;
        let endpoint = moq_native_ietf::quic::Endpoint::new(config)
            .map_err(|error| MoqError::Relay(error.to_string()))?;
        Ok(Self {
            client: endpoint.client,
        })
    }
}

pub(crate) struct WireRelayPublication {
    pub(crate) connection_id: String,
    pub(crate) relay_path: &'static str,
    session_task: AbortHandle,
    publish_task: AbortHandle,
}

impl Drop for WireRelayPublication {
    fn drop(&mut self) {
        self.publish_task.abort();
        self.session_task.abort();
    }
}

pub(crate) async fn publish_to_relay(
    publication: &WirePublication,
    client: &WireRelayClient,
    relay: &Url,
) -> Result<WireRelayPublication, MoqError> {
    let (wire, connection_id, transport) = client
        .client
        .connect(relay, None)
        .await
        .map_err(|error| MoqError::Relay(error.to_string()))?;
    let relay_path = match transport {
        moq_transport::session::Transport::RawQuic => "raw-quic",
        moq_transport::session::Transport::WebTransport => "webtransport",
    };
    let (session, mut publisher) = moq_transport::session::Publisher::connect(wire, transport)
        .await
        .map_err(|error| MoqError::Relay(error.to_string()))?;
    let session_task = tokio::spawn(async move {
        if let Err(error) = session.run().await {
            tracing::warn!(%error, "MOQT relay session ended");
        }
    });
    let tracks = publication.tracks();
    let publish_task = tokio::spawn(async move {
        if let Err(error) = publisher.publish_namespace(tracks).await {
            tracing::warn!(%error, "MOQT namespace publication ended");
        }
    });
    Ok(WireRelayPublication {
        connection_id,
        relay_path,
        session_task: session_task.abort_handle(),
        publish_task: publish_task.abort_handle(),
    })
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
}
