use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use rvoip_core_traits::broadcast::{
    BroadcastDescriptor, BroadcastEndpoint, BroadcastProtocolDescriptor, BroadcastProtocolFamily,
    BroadcastPublisher, BroadcastResource, BroadcastTransport,
};
use rvoip_core_traits::capability::CodecInfo;
use rvoip_core_traits::error::Result as RvoipResult;
use rvoip_core_traits::stream::MediaFrame;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use url::Url;

use crate::wire::{self, WirePublication, WireRelayClient, WireRelayPublication};
use crate::{
    LocError, LocOpusPacketizer, MoqCompatibility, MoqError, MoqNamespace, MoqProtocolVersion,
    MsfCatalog, AUDIO_TRACK, CATALOG_TRACK, LOC_DRAFT, MOQT_DRAFT, MSF_DRAFT,
};

#[derive(Clone, Debug)]
pub struct MoqPublisherConfig {
    pub tenant_id: String,
    pub broadcast_id: String,
    pub bitrate: u32,
    pub language: Option<String>,
    pub queue_frames: usize,
}

impl MoqPublisherConfig {
    /// Exact namespace representation retained for source compatibility.
    /// Validation is performed by [`Self::try_namespace`] and publisher
    /// construction; no sanitization or normalization occurs.
    pub fn namespace(&self) -> String {
        format!("{}/{}", self.tenant_id, self.broadcast_id)
    }

    pub fn try_namespace(&self) -> Result<MoqNamespace, MoqError> {
        Ok(MoqNamespace::new(
            self.tenant_id.clone(),
            self.broadcast_id.clone(),
        )?)
    }
}

/// MediaGraph-compatible MOQT publisher with an rvoip-owned public surface.
pub struct MoqBroadcastPublisher {
    config: MoqPublisherConfig,
    namespace: MoqNamespace,
    frame_tx: mpsc::Sender<MediaFrame>,
    wire: WirePublication,
    task: AbortHandle,
}

impl MoqBroadcastPublisher {
    pub fn new(config: MoqPublisherConfig) -> Result<Arc<Self>, MoqError> {
        let namespace = config.try_namespace()?;
        MoqCompatibility::PINNED.require(MoqProtocolVersion::PINNED)?;
        let catalog = MsfCatalog::opus_audio(
            &namespace,
            config.bitrate,
            config.language.clone(),
            unix_time_millis(),
        )?;
        let catalog_payload = catalog.to_json_bytes()?;
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| MoqError::RuntimeUnavailable)?;
        let (wire, mut audio) = WirePublication::new(&namespace, catalog_payload)?;

        let (frame_tx, mut frame_rx) = mpsc::channel::<MediaFrame>(config.queue_frames.max(1));
        let task = runtime.spawn(async move {
            let mut packetizer = LocOpusPacketizer::new();
            while let Some(frame) = frame_rx.recv().await {
                let packetized = match packetizer.packetize(&frame) {
                    Ok(packetized) => packetized,
                    Err(error) => {
                        metrics::counter!(
                            "rvoip_moq_invalid_frames_total",
                            "reason" => loc_error_label(&error)
                        )
                        .increment(1);
                        tracing::warn!(%error, "dropping frame outside the MOQT LOC profile");
                        continue;
                    }
                };
                if let Some(discontinuity) = packetized.discontinuity {
                    metrics::counter!("rvoip_moq_timestamp_discontinuities_total").increment(1);
                    tracing::warn!(
                        expected_rtp_timestamp = discontinuity.expected_rtp_timestamp,
                        actual_rtp_timestamp = discontinuity.actual_rtp_timestamp,
                        loc_timestamp = packetized.object.timestamp,
                        "publishing the first valid frame after an RTP timestamp discontinuity"
                    );
                }
                if let Err(error) = audio.write(packetized.object) {
                    tracing::debug!(%error, "MOQT audio track closed");
                    break;
                }
                metrics::counter!("rvoip_moq_objects_total", "track" => "audio").increment(1);
            }
        });

        Ok(Arc::new(Self {
            config,
            namespace,
            frame_tx,
            wire,
            task: task.abort_handle(),
        }))
    }

    pub fn namespace(&self) -> &MoqNamespace {
        &self.namespace
    }

    pub fn config(&self) -> &MoqPublisherConfig {
        &self.config
    }

    pub const fn protocol_version(&self) -> MoqProtocolVersion {
        MoqProtocolVersion::PINNED
    }

    /// Announce this publisher to an external raw-QUIC or WebTransport MOQT
    /// relay. The handle closes both protocol tasks when dropped.
    pub async fn publish_to_relay(
        &self,
        client: &MoqRelayClient,
        relay: &Url,
    ) -> Result<MoqRelayPublication, MoqError> {
        let wire = wire::publish_to_relay(&self.wire, &client.wire, relay).await?;
        metrics::counter!(
            "rvoip_moq_relay_publications_total",
            "path" => wire.relay_path
        )
        .increment(1);
        Ok(MoqRelayPublication {
            connection_id: wire.connection_id.clone(),
            relay_path: wire.relay_path,
            _wire: wire,
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

/// Reusable MOQT relay client. Client certificate and private key enable
/// origin-to-relay mTLS.
pub struct MoqRelayClient {
    wire: WireRelayClient,
}

impl MoqRelayClient {
    pub fn bind(bind: SocketAddr, tls: MoqRelayTlsConfig) -> Result<Self, MoqError> {
        Ok(Self {
            wire: WireRelayClient::bind(
                bind,
                tls.root_certificates,
                tls.client_certificate,
                tls.client_private_key,
                tls.disable_verification,
            )?,
        })
    }
}

/// Running publication to one relay.
pub struct MoqRelayPublication {
    pub connection_id: String,
    pub relay_path: &'static str,
    _wire: WireRelayPublication,
}

#[async_trait]
impl BroadcastPublisher for MoqBroadcastPublisher {
    fn descriptor(&self) -> BroadcastDescriptor {
        BroadcastDescriptor {
            transport: BroadcastTransport::Moqt,
            namespace: self.namespace.to_string(),
            audio_track: AUDIO_TRACK.into(),
            catalog_track: Some(CATALOG_TRACK.into()),
            protocol_version: MoqProtocolVersion::PINNED.to_string(),
        }
    }

    fn codec(&self) -> CodecInfo {
        CodecInfo::from_name_with_defaults("opus")
    }

    fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
        self.frame_tx.clone()
    }

    fn endpoint(&self) -> BroadcastEndpoint {
        BroadcastEndpoint {
            uri: None,
            resource: BroadcastResource::Moqt {
                namespace: self.namespace.to_string(),
                audio_track: AUDIO_TRACK.into(),
                catalog_track: Some(CATALOG_TRACK.into()),
                events_track: None,
            },
            relay_path: Vec::new(),
        }
    }

    fn protocol(&self) -> BroadcastProtocolDescriptor {
        BroadcastProtocolDescriptor {
            family: BroadcastProtocolFamily::Moqt,
            // The publisher can be attached to either raw QUIC or
            // WebTransport; a concrete relay publication reports the path.
            substrate: None,
            transport_version: MOQT_DRAFT.into(),
            media_format_version: Some(MSF_DRAFT.into()),
            object_format_version: Some(LOC_DRAFT.into()),
            media_profile: Some("opus/48000/1; frame-duration=20ms".into()),
        }
    }

    async fn close(self: Arc<Self>) -> RvoipResult<()> {
        self.task.abort();
        self.wire.close();
        Ok(())
    }
}

fn loc_error_label(error: &LocError) -> &'static str {
    match error {
        LocError::NotAudio => "not_audio",
        LocError::EmptyPacket => "empty_packet",
        LocError::StereoPacket => "stereo",
        LocError::MissingFrameCount | LocError::InvalidFrameCount { .. } => "frame_count",
        LocError::PacketDuration { .. } => "duration",
        LocError::TimestampOverflow => "timestamp_overflow",
        LocError::GroupIdExhausted => "group_id_exhausted",
    }
}

fn unix_time_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use chrono::Utc;
    use rvoip_core_traits::broadcast::BroadcastPublisher;
    use rvoip_core_traits::ids::StreamId;
    use rvoip_core_traits::stream::StreamKind;

    use super::*;

    fn config() -> MoqPublisherConfig {
        MoqPublisherConfig {
            tenant_id: "tenant-a".into(),
            broadcast_id: "broadcast-1".into(),
            bitrate: 24_000,
            language: Some("en".into()),
            queue_frames: 10,
        }
    }

    #[tokio::test]
    async fn publishes_through_the_transport_neutral_contract() {
        let publisher = MoqBroadcastPublisher::new(config()).unwrap();
        assert_eq!(publisher.namespace().as_str(), "tenant-a/broadcast-1");
        assert_eq!(
            publisher.descriptor().protocol_version,
            "draft-ietf-moq-transport-19; draft-ietf-moq-msf-01; draft-ietf-moq-loc-03"
        );
        assert_eq!(publisher.protocol().transport_version, MOQT_DRAFT);
        assert_eq!(
            publisher.protocol().media_format_version.as_deref(),
            Some(MSF_DRAFT)
        );
        assert_eq!(
            publisher.protocol().object_format_version.as_deref(),
            Some(LOC_DRAFT)
        );

        publisher
            .frames_out()
            .send(MediaFrame {
                stream_id: StreamId::new(),
                kind: StreamKind::Audio,
                payload: Bytes::from_static(&[0x78, 0x00]),
                timestamp_rtp: 960,
                captured_at: Utc::now(),
                payload_type: Some(111),
            })
            .await
            .unwrap();
        publisher.close().await.unwrap();
    }

    #[tokio::test]
    async fn rejects_namespaces_instead_of_collapsing_them() {
        let mut invalid = config();
        invalid.tenant_id = "tenant/a".into();
        assert!(matches!(
            MoqBroadcastPublisher::new(invalid),
            Err(MoqError::Namespace(_))
        ));
    }

    #[test]
    fn publisher_and_relay_public_types_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<MoqBroadcastPublisher>();
        assert_send_sync::<MoqRelayClient>();
    }

    #[test]
    fn construction_without_a_runtime_returns_an_explicit_error() {
        assert!(matches!(
            MoqBroadcastPublisher::new(config()),
            Err(MoqError::RuntimeUnavailable)
        ));
    }
}
