//! MOQT broadcast publishing for rvoip.
//!
//! The crate deliberately exposes rvoip-owned types while using
//! `moq-transport` internally. Draft churn is contained here instead of
//! leaking through Bridgefu or the core media graph.

use std::io::BufReader;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use moq_transport::coding::{TrackName, TrackNamespace};
use moq_transport::data::ExtensionHeaders;
use moq_transport::serve::{Datagram, Tracks, TracksReader, TracksRequest, TracksWriter};
use rvoip_core::broadcast::{BroadcastDescriptor, BroadcastPublisher, BroadcastTransport};
use rvoip_core::capability::CodecInfo;
use rvoip_core::error::{Result as RvoipResult, RvoipError};
use rvoip_core::stream::MediaFrame;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use url::Url;

pub const MOQT_DRAFT: &str = "draft-ietf-moq-transport-16";
pub const TARGET_MOQT_DRAFT: &str = "draft-ietf-moq-transport-19";
pub const MSF_DRAFT: &str = "draft-ietf-moq-msf-01";
pub const LOC_DRAFT: &str = "draft-ietf-moq-loc-03";
pub const AUDIO_TRACK: &str = "audio/main";
pub const CATALOG_TRACK: &str = "catalog";

/// LOC-03 provisional MOQT Object Property identifiers.
const LOC_TIMESTAMP_PROPERTY: u64 = 0x06;
const LOC_TIMESCALE_PROPERTY: u64 = 0x08;

#[derive(Clone, Debug)]
pub struct MoqPublisherConfig {
    pub tenant_id: String,
    pub broadcast_id: String,
    pub bitrate: u32,
    pub language: Option<String>,
    pub queue_frames: usize,
}

impl MoqPublisherConfig {
    pub fn namespace(&self) -> String {
        format!(
            "{}/{}",
            sanitize_component(&self.tenant_id),
            sanitize_component(&self.broadcast_id)
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsfCatalog {
    pub version: String,
    pub generated_at: i64,
    pub tracks: Vec<MsfTrack>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MsfTrack {
    pub namespace: String,
    pub name: String,
    pub packaging: String,
    pub is_live: bool,
    pub codec: String,
    pub samplerate: u32,
    pub channel_config: String,
    pub max_bitrate: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

impl MsfCatalog {
    fn audio(config: &MoqPublisherConfig) -> Self {
        Self {
            version: "1".into(),
            generated_at: chrono_like_now_millis(),
            tracks: vec![MsfTrack {
                namespace: config.namespace(),
                name: AUDIO_TRACK.into(),
                packaging: "loc".into(),
                is_live: true,
                codec: "opus".into(),
                samplerate: 48_000,
                channel_config: "mono".into(),
                max_bitrate: config.bitrate,
                lang: config.language.clone(),
            }],
        }
    }
}

pub struct MoqBroadcastPublisher {
    config: MoqPublisherConfig,
    frame_tx: mpsc::Sender<MediaFrame>,
    tracks_reader: TracksReader,
    _tracks_writer: Mutex<Option<TracksWriter>>,
    _tracks_request: Mutex<Option<TracksRequest>>,
    task: AbortHandle,
}

impl MoqBroadcastPublisher {
    pub fn new(config: MoqPublisherConfig) -> Result<Arc<Self>, MoqError> {
        if config.tenant_id.trim().is_empty() || config.broadcast_id.trim().is_empty() {
            return Err(MoqError::InvalidConfig(
                "tenant_id and broadcast_id are required",
            ));
        }
        let namespace_path = config.namespace();
        let namespace = TrackNamespace::from_utf8_path(&namespace_path);
        let (mut tracks_writer, tracks_request, tracks_reader) =
            Tracks::new(namespace.clone()).produce();

        let audio_track = tracks_writer
            .create(TrackName::from(AUDIO_TRACK))
            .ok_or(MoqError::Closed)?;
        let mut audio = audio_track.datagrams().map_err(MoqError::Serve)?;

        let catalog_track = tracks_writer
            .create(TrackName::from(CATALOG_TRACK))
            .ok_or(MoqError::Closed)?;
        let mut catalog = catalog_track.datagrams().map_err(MoqError::Serve)?;
        let catalog_payload = serde_json::to_vec(&MsfCatalog::audio(&config))?;
        catalog.write(Datagram {
            group_id: 0,
            object_id: 0,
            priority: 0,
            payload: catalog_payload.into(),
            extension_headers: ExtensionHeaders::new(),
        })?;

        let (frame_tx, mut frame_rx) = mpsc::channel::<MediaFrame>(config.queue_frames.max(1));
        let task = tokio::spawn(async move {
            // Keep the catalog writer alive for late subscribers while audio
            // is being published.
            let _catalog = catalog;
            let mut group_id = 0u64;
            while let Some(frame) = frame_rx.recv().await {
                let mut properties = ExtensionHeaders::new();
                properties.set_intvalue(LOC_TIMESTAMP_PROPERTY, frame.timestamp_rtp as u64);
                properties.set_intvalue(LOC_TIMESCALE_PROPERTY, 48_000);
                if let Err(error) = audio.write(Datagram {
                    group_id,
                    object_id: 0,
                    priority: 0,
                    payload: frame.payload,
                    extension_headers: properties,
                }) {
                    tracing::debug!(%error, "MOQT audio track closed");
                    break;
                }
                metrics::counter!("rvoip_moq_objects_total", "track" => "audio").increment(1);
                group_id = group_id.wrapping_add(1);
            }
        });

        Ok(Arc::new(Self {
            config,
            frame_tx,
            tracks_reader,
            _tracks_writer: Mutex::new(Some(tracks_writer)),
            _tracks_request: Mutex::new(Some(tracks_request)),
            task: task.abort_handle(),
        }))
    }

    /// Reader consumed by a moq-transport publisher session or relay.
    pub fn tracks(&self) -> TracksReader {
        self.tracks_reader.clone()
    }

    pub fn namespace(&self) -> TrackNamespace {
        self.tracks_reader.info.namespace.clone()
    }

    /// Announce this publisher to an external raw-QUIC or WebTransport MOQT
    /// relay. The returned handle owns both protocol tasks and closes them on
    /// drop, making relay publication follow the broadcast lifecycle.
    pub async fn publish_to_relay(
        &self,
        client: &MoqRelayClient,
        relay: &Url,
    ) -> Result<MoqRelayPublication, MoqError> {
        let (wire, connection_id, transport) = client
            .endpoint
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
        let tracks = self.tracks();
        let publish_task = tokio::spawn(async move {
            if let Err(error) = publisher.publish_namespace(tracks).await {
                tracing::warn!(%error, "MOQT namespace publication ended");
            }
        });
        metrics::counter!("rvoip_moq_relay_publications_total", "path" => relay_path).increment(1);
        Ok(MoqRelayPublication {
            connection_id,
            relay_path,
            session_task: session_task.abort_handle(),
            publish_task: publish_task.abort_handle(),
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct MoqRelayTlsConfig {
    pub root_certificates: Vec<PathBuf>,
    pub client_certificate: Option<PathBuf>,
    pub client_private_key: Option<PathBuf>,
    pub disable_verification: bool,
}

/// Reusable MOQT relay client. `client_certificate` and
/// `client_private_key` enable origin-to-relay mTLS.
pub struct MoqRelayClient {
    endpoint: moq_native_ietf::quic::Endpoint,
}

impl MoqRelayClient {
    pub fn bind(bind: SocketAddr, tls: MoqRelayTlsConfig) -> Result<Self, MoqError> {
        if tls.client_certificate.is_some() != tls.client_private_key.is_some() {
            return Err(MoqError::InvalidConfig(
                "MOQT client certificate and private key must be configured together",
            ));
        }
        let roots = tls.root_certificates.clone();
        let native_tls = moq_native_ietf::tls::Args {
            root: roots.clone(),
            disable_verify: tls.disable_verification,
            ..Default::default()
        }
        .load()
        .map_err(|error| MoqError::Relay(error.to_string()))?;
        let client = if let (Some(certificate), Some(private_key)) =
            (tls.client_certificate, tls.client_private_key)
        {
            if tls.disable_verification {
                return Err(MoqError::InvalidConfig(
                    "MOQT mTLS may not disable server certificate verification",
                ));
            }
            let certificates = rustls_pemfile::certs(&mut BufReader::new(
                std::fs::File::open(certificate)
                    .map_err(|error| MoqError::Relay(error.to_string()))?,
            ))
            .collect::<std::result::Result<Vec<_>, _>>()
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
        Ok(Self { endpoint })
    }
}

/// Running publication to one relay.
pub struct MoqRelayPublication {
    pub connection_id: String,
    pub relay_path: &'static str,
    session_task: AbortHandle,
    publish_task: AbortHandle,
}

impl Drop for MoqRelayPublication {
    fn drop(&mut self) {
        self.publish_task.abort();
        self.session_task.abort();
    }
}

#[async_trait]
impl BroadcastPublisher for MoqBroadcastPublisher {
    fn descriptor(&self) -> BroadcastDescriptor {
        BroadcastDescriptor {
            transport: BroadcastTransport::Moqt,
            namespace: self.config.namespace(),
            audio_track: AUDIO_TRACK.into(),
            catalog_track: Some(CATALOG_TRACK.into()),
            protocol_version: format!(
                "{MOQT_DRAFT}; target={TARGET_MOQT_DRAFT}; {MSF_DRAFT}; {LOC_DRAFT}"
            ),
        }
    }

    fn codec(&self) -> CodecInfo {
        CodecInfo::from_name_with_defaults("opus")
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.frame_tx.clone()
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        self.task.abort();
        self._tracks_writer
            .lock()
            .expect("tracks writer poisoned")
            .take();
        self._tracks_request
            .lock()
            .expect("tracks request poisoned")
            .take();
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MoqError {
    #[error("invalid MOQT publisher configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("MOQT tracks are closed")]
    Closed,
    #[error("MOQT serve error: {0}")]
    Serve(#[from] moq_transport::serve::ServeError),
    #[error("MSF catalog encoding failed: {0}")]
    Catalog(#[from] serde_json::Error),
    #[error("MOQT relay error: {0}")]
    Relay(String),
}

impl From<MoqError> for RvoipError {
    fn from(error: MoqError) -> Self {
        RvoipError::Adapter(error.to_string())
    }
}

fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn chrono_like_now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use moq_transport::serve::TrackReaderMode;
    use rvoip_core::broadcast::BroadcastPublisher;
    use rvoip_core::ids::StreamId;
    use rvoip_core::stream::StreamKind;

    use super::*;

    fn config() -> MoqPublisherConfig {
        MoqPublisherConfig {
            tenant_id: "tenant/a".into(),
            broadcast_id: "broadcast-1".into(),
            bitrate: 24_000,
            language: Some("en".into()),
            queue_frames: 10,
        }
    }

    #[tokio::test]
    async fn publishes_loc_opus_objects_and_catalog() {
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        assert_eq!(publisher.descriptor().namespace, "tenant_a/broadcast-1");
        let namespace = publisher.namespace();
        let mut tracks = publisher.tracks();
        let audio = tracks
            .get_track_reader(&namespace, TrackName::from(AUDIO_TRACK))
            .unwrap();
        let mut audio = match audio.mode().await.unwrap() {
            TrackReaderMode::Datagrams(reader) => reader,
            _ => panic!("audio track must use datagrams"),
        };

        publisher
            .frames_out()
            .send(MediaFrame {
                stream_id: StreamId::new(),
                kind: StreamKind::Audio,
                payload: Bytes::from_static(b"opus"),
                timestamp_rtp: 960,
                captured_at: chrono::Utc::now(),
                payload_type: Some(111),
            })
            .await
            .unwrap();

        let object = audio.read().await.unwrap().unwrap();
        assert_eq!(object.group_id, 0);
        assert_eq!(object.object_id, 0);
        assert_eq!(object.payload, Bytes::from_static(b"opus"));
    }
}
