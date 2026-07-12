//! Transport-neutral one-to-many media publication contracts.
//!
//! The legacy [`BroadcastDescriptor`] remains the smallest common surface for
//! existing publishers.  The typed endpoint, protocol, lifecycle, health, and
//! drain descriptors let control planes manage UCTP and MOQT publishers
//! without importing either transport crate.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use tokio::sync::mpsc;

use crate::capability::CodecInfo;
use crate::error::{Result, RvoipError};
use crate::stream::MediaFrame;

/// Broadcast protocol family exposed by a publisher.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BroadcastTransport {
    /// RVoIP's authenticated UCTP protocol over QUIC or WebTransport.
    UctpQuic,
    /// Media over QUIC Transport, optionally through a relay path.
    Moqt,
}

/// Legacy publication descriptor retained for source compatibility.
///
/// New control-plane code should additionally query [`BroadcastPublisher::endpoint`]
/// and [`BroadcastPublisher::protocol`] for structured transport metadata.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BroadcastDescriptor {
    pub transport: BroadcastTransport,
    pub namespace: String,
    pub audio_track: String,
    pub catalog_track: Option<String>,
    pub protocol_version: String,
}

/// Largest integer represented exactly by interoperable JSON number parsers.
pub const MAX_BROADCAST_EVENT_JSON_INTEGER: u64 = (1_u64 << 53) - 1;

/// Fixed, transport-neutral lifecycle events allowed on sanitized broadcasts.
///
/// There is deliberately no custom/string variant, so call identifiers,
/// provider metadata, SIP headers, and application context cannot enter this
/// contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BroadcastSanitizedEventKind {
    CallConnecting,
    CallConnected,
    CallHeld,
    CallResumed,
    TransferStarted,
    TransferCompleted,
    TransferFailed,
    CallEnding,
    CallEnded,
}

/// One fixed-model sanitized event indexed by Unix wallclock milliseconds.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastSanitizedEvent {
    kind: BroadcastSanitizedEventKind,
    occurred_at_unix_millis: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BroadcastSanitizedEventWire {
    kind: BroadcastSanitizedEventKind,
    occurred_at_unix_millis: u64,
}

impl<'de> Deserialize<'de> for BroadcastSanitizedEvent {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BroadcastSanitizedEventWire::deserialize(deserializer)?;
        Self::at_unix_millis(wire.kind, wire.occurred_at_unix_millis)
            .map_err(serde::de::Error::custom)
    }
}

impl BroadcastSanitizedEvent {
    /// Construct an event only when its wallclock value is exactly safe in
    /// JSON implementations that represent numbers as IEEE-754 doubles.
    pub fn at_unix_millis(
        kind: BroadcastSanitizedEventKind,
        occurred_at_unix_millis: u64,
    ) -> std::result::Result<Self, BroadcastSanitizedEventError> {
        if occurred_at_unix_millis > MAX_BROADCAST_EVENT_JSON_INTEGER {
            return Err(BroadcastSanitizedEventError::TimestampOutOfRange {
                maximum: MAX_BROADCAST_EVENT_JSON_INTEGER,
                actual: occurred_at_unix_millis,
            });
        }
        Ok(Self {
            kind,
            occurred_at_unix_millis,
        })
    }

    pub fn now(
        kind: BroadcastSanitizedEventKind,
    ) -> std::result::Result<Self, BroadcastSanitizedEventError> {
        let occurred_at_unix_millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        Self::at_unix_millis(kind, occurred_at_unix_millis)
    }

    pub const fn kind(&self) -> BroadcastSanitizedEventKind {
        self.kind
    }

    pub const fn occurred_at_unix_millis(&self) -> u64 {
        self.occurred_at_unix_millis
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum BroadcastSanitizedEventError {
    #[error("sanitized broadcast event timestamp {actual} exceeds JSON-safe maximum {maximum}")]
    TimestampOutOfRange { maximum: u64, actual: u64 },
}

/// Bounded fixed-model event capability exposed by a publisher.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastSanitizedEventCapability {
    pub queue_capacity: u32,
    pub history_capacity: u32,
}

/// Transport-specific resource addressed by a broadcast endpoint.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BroadcastResource {
    /// A UCTP session and its receive-only media stream.
    Uctp {
        session_id: String,
        stream_id: String,
    },
    /// A MOQT namespace and its well-known publication tracks.
    Moqt {
        namespace: String,
        audio_track: String,
        catalog_track: Option<String>,
        events_track: Option<String>,
    },
}

impl BroadcastResource {
    /// Protocol family implied by this resource shape.
    pub fn transport(&self) -> BroadcastTransport {
        match self {
            Self::Uctp { .. } => BroadcastTransport::UctpQuic,
            Self::Moqt { .. } => BroadcastTransport::Moqt,
        }
    }
}

/// Role of one address in a relay path.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BroadcastRelayRole {
    Origin,
    Relay,
    Edge,
}

/// One diagnosable hop from a publisher origin to its subscribers.
///
/// Hop URIs belong in APIs, logs, and traces. They must not be copied into
/// metric labels because their cardinality is deployment-dependent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BroadcastRelayHop {
    pub role: BroadcastRelayRole,
    pub uri: String,
}

/// Subscriber-facing endpoint and protocol resource.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BroadcastEndpoint {
    /// Public raw-QUIC or WebTransport URI, when the publisher has bound one.
    pub uri: Option<String>,
    pub resource: BroadcastResource,
    /// Ordered origin-to-edge path. Direct publications leave this empty.
    pub relay_path: Vec<BroadcastRelayHop>,
}

impl BroadcastEndpoint {
    /// Protocol family implied by the endpoint resource.
    pub fn transport(&self) -> BroadcastTransport {
        self.resource.transport()
    }
}

/// Stable protocol family used for aggregate metrics and compatibility checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BroadcastProtocolFamily {
    Uctp,
    Moqt,
}

/// Network substrate carrying the application broadcast protocol.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BroadcastSubstrate {
    RawQuic,
    WebTransport,
    WebSocket,
}

/// Protocol compatibility tuple used by a publication.
///
/// `transport_version` is the negotiated transport version. MOQT
/// implementations use `media_format_version` and `object_format_version` to
/// declare their configured MSF and LOC versions unless the selected transport
/// extension negotiates those values separately.
/// UCTP implementations use `transport_version` for UCTP and `media_profile`
/// for the full-RTP datagram profile.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BroadcastProtocolDescriptor {
    pub family: BroadcastProtocolFamily,
    pub substrate: Option<BroadcastSubstrate>,
    pub transport_version: String,
    pub media_format_version: Option<String>,
    pub object_format_version: Option<String>,
    pub media_profile: Option<String>,
}

/// Managed publisher lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BroadcastLifecycleState {
    Starting,
    Ready,
    Degraded,
    Reconnecting,
    Draining,
    Closed,
    Failed,
}

/// Lifecycle snapshot suitable for an API or diagnostic response.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BroadcastLifecycleDescriptor {
    pub state: BroadcastLifecycleState,
    /// Time at which the current state began, when tracked by the publisher.
    pub since: Option<DateTime<Utc>>,
}

/// Aggregate health state with a bounded metric-label vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BroadcastHealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Closed,
}

/// Bounded health reason codes. Resource identifiers intentionally do not
/// appear here so these values are safe to aggregate in metrics.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BroadcastHealthIssue {
    TransportUnavailable,
    RelayUnavailable,
    AuthenticationUnavailable,
    VersionMismatch,
    CapacityExhausted,
    MediaStalled,
    Reconnecting,
    Draining,
}

/// Point-in-time publisher health and bounded capacity data.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BroadcastHealthDescriptor {
    pub status: BroadcastHealthStatus,
    pub issues: Vec<BroadcastHealthIssue>,
    pub active_subscribers: Option<u32>,
    pub subscriber_capacity: Option<u32>,
    pub checked_at: DateTime<Utc>,
}

impl BroadcastHealthDescriptor {
    /// Healthy snapshot for publishers that do not yet expose richer health.
    pub fn healthy() -> Self {
        Self {
            status: BroadcastHealthStatus::Healthy,
            issues: Vec::new(),
            active_subscribers: None,
            subscriber_capacity: None,
            checked_at: Utc::now(),
        }
    }
}

/// Operator intent behind a drain operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BroadcastDrainReason {
    OperatorRequest,
    Shutdown,
    Reconfigure,
    Unhealthy,
}

/// Request to stop admitting listeners and finish by a fixed deadline.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BroadcastDrainRequest {
    pub reason: BroadcastDrainReason,
    pub deadline: DateTime<Utc>,
}

impl BroadcastDrainRequest {
    /// Request an immediate operator-initiated drain.
    pub fn immediate() -> Self {
        Self {
            reason: BroadcastDrainReason::OperatorRequest,
            deadline: Utc::now(),
        }
    }
}

/// Progress of a drain operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BroadcastDrainState {
    Draining,
    Drained,
    DeadlineExceeded,
}

/// Result snapshot for a drain operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BroadcastDrainDescriptor {
    pub state: BroadcastDrainState,
    pub reason: BroadcastDrainReason,
    pub started_at: DateTime<Utc>,
    pub deadline: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub remaining_subscribers: u32,
}

impl BroadcastDescriptor {
    /// Convert the legacy resource fields into the typed endpoint contract.
    pub fn endpoint(&self) -> BroadcastEndpoint {
        let resource = match self.transport {
            BroadcastTransport::UctpQuic => BroadcastResource::Uctp {
                session_id: self.namespace.clone(),
                stream_id: self.audio_track.clone(),
            },
            BroadcastTransport::Moqt => BroadcastResource::Moqt {
                namespace: self.namespace.clone(),
                audio_track: self.audio_track.clone(),
                catalog_track: self.catalog_track.clone(),
                events_track: None,
            },
        };
        BroadcastEndpoint {
            uri: None,
            resource,
            relay_path: Vec::new(),
        }
    }

    /// Convert the legacy version string into a typed protocol descriptor.
    /// Publishers with a multi-part version tuple should override
    /// [`BroadcastPublisher::protocol`].
    pub fn protocol(&self) -> BroadcastProtocolDescriptor {
        BroadcastProtocolDescriptor {
            family: match self.transport {
                BroadcastTransport::UctpQuic => BroadcastProtocolFamily::Uctp,
                BroadcastTransport::Moqt => BroadcastProtocolFamily::Moqt,
            },
            substrate: None,
            transport_version: self.protocol_version.clone(),
            media_format_version: None,
            object_format_version: None,
            media_profile: None,
        }
    }
}

/// Object-safe lifecycle and media contract shared by broadcast publishers.
///
/// Existing implementors only need the legacy required methods. The richer
/// management methods have conservative defaults and can be overridden as a
/// transport grows managed origin, relay, reconnect, and drain support.
#[async_trait]
pub trait BroadcastPublisher: Send + Sync {
    fn descriptor(&self) -> BroadcastDescriptor;
    fn codec(&self) -> CodecInfo;
    fn frames_out(&self) -> mpsc::Sender<MediaFrame>;

    /// Optional fixed-model event capability. Legacy and media-only
    /// publishers return `None`.
    fn sanitized_event_capability(&self) -> Option<BroadcastSanitizedEventCapability> {
        None
    }

    /// Nonblocking admission of one fixed-model sanitized event.
    fn try_publish_sanitized_event(&self, _event: BroadcastSanitizedEvent) -> Result<()> {
        Err(RvoipError::NotImplemented(
            "sanitized broadcast event publication",
        ))
    }

    /// Subscriber-facing endpoint. Defaults to the legacy descriptor fields.
    fn endpoint(&self) -> BroadcastEndpoint {
        self.descriptor().endpoint()
    }

    /// Transport version plus configured/declared media profiles.
    /// Defaults to the legacy free-form version string.
    fn protocol(&self) -> BroadcastProtocolDescriptor {
        self.descriptor().protocol()
    }

    /// Current managed lifecycle. Legacy publishers report ready.
    fn lifecycle(&self) -> BroadcastLifecycleDescriptor {
        BroadcastLifecycleDescriptor {
            state: BroadcastLifecycleState::Ready,
            since: None,
        }
    }

    /// Current aggregate health. Legacy publishers default to healthy/unknown.
    fn health(&self) -> BroadcastHealthDescriptor {
        BroadcastHealthDescriptor::healthy()
    }

    /// Stop listener admission and close by the requested deadline.
    ///
    /// Legacy publishers close immediately. Managed publishers can override
    /// this method to wait for listeners or relay publications to leave.
    async fn drain(
        self: Arc<Self>,
        request: BroadcastDrainRequest,
    ) -> Result<BroadcastDrainDescriptor> {
        let started_at = Utc::now();
        let missed_deadline = started_at > request.deadline;
        self.close().await?;
        Ok(BroadcastDrainDescriptor {
            state: if missed_deadline {
                BroadcastDrainState::DeadlineExceeded
            } else {
                BroadcastDrainState::Drained
            },
            reason: request.reason,
            started_at,
            deadline: request.deadline,
            completed_at: Some(Utc::now()),
            remaining_subscribers: 0,
        })
    }

    async fn close(self: Arc<Self>) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    struct LegacyPublisher {
        closed: AtomicBool,
        frame_tx: mpsc::Sender<MediaFrame>,
    }

    #[async_trait]
    impl BroadcastPublisher for LegacyPublisher {
        fn descriptor(&self) -> BroadcastDescriptor {
            BroadcastDescriptor {
                transport: BroadcastTransport::UctpQuic,
                namespace: "session-1".into(),
                audio_track: "stream-2".into(),
                catalog_track: None,
                protocol_version: "uctp/0.2; rtp-datagram/1".into(),
            }
        }

        fn codec(&self) -> CodecInfo {
            CodecInfo::from_name_with_defaults("opus")
        }

        fn frames_out(&self) -> mpsc::Sender<MediaFrame> {
            self.frame_tx.clone()
        }

        async fn close(self: Arc<Self>) -> Result<()> {
            self.closed.store(true, Ordering::Release);
            Ok(())
        }
    }

    #[tokio::test]
    async fn legacy_implementor_gets_typed_defaults_and_object_safe_drain() {
        let (frame_tx, _) = mpsc::channel(1);
        let publisher: Arc<dyn BroadcastPublisher> = Arc::new(LegacyPublisher {
            closed: AtomicBool::new(false),
            frame_tx,
        });

        assert_eq!(
            publisher.endpoint().resource,
            BroadcastResource::Uctp {
                session_id: "session-1".into(),
                stream_id: "stream-2".into(),
            }
        );
        assert_eq!(publisher.protocol().family, BroadcastProtocolFamily::Uctp);
        assert_eq!(publisher.lifecycle().state, BroadcastLifecycleState::Ready);
        assert_eq!(publisher.health().status, BroadcastHealthStatus::Healthy);
        assert_eq!(publisher.sanitized_event_capability(), None);
        assert!(matches!(
            publisher.try_publish_sanitized_event(
                BroadcastSanitizedEvent::at_unix_millis(
                    BroadcastSanitizedEventKind::CallConnected,
                    1_000,
                )
                .unwrap(),
            ),
            Err(RvoipError::NotImplemented(_))
        ));

        let drained = Arc::clone(&publisher)
            .drain(BroadcastDrainRequest {
                reason: BroadcastDrainReason::Shutdown,
                deadline: Utc::now() + chrono::Duration::seconds(1),
            })
            .await
            .unwrap();
        assert_eq!(drained.state, BroadcastDrainState::Drained);
    }

    #[test]
    fn moqt_legacy_descriptor_maps_to_typed_tracks() {
        let endpoint = BroadcastDescriptor {
            transport: BroadcastTransport::Moqt,
            namespace: "tenant/broadcast".into(),
            audio_track: "audio/main".into(),
            catalog_track: Some("catalog".into()),
            protocol_version: "draft-19".into(),
        }
        .endpoint();

        assert_eq!(endpoint.transport(), BroadcastTransport::Moqt);
        assert!(matches!(
            endpoint.resource,
            BroadcastResource::Moqt {
                events_track: None,
                ..
            }
        ));
    }

    #[test]
    fn sanitized_event_model_is_fixed_and_json_safe() {
        let event = BroadcastSanitizedEvent::at_unix_millis(
            BroadcastSanitizedEventKind::CallConnected,
            MAX_BROADCAST_EVENT_JSON_INTEGER,
        )
        .unwrap();
        assert_eq!(
            event.occurred_at_unix_millis(),
            MAX_BROADCAST_EVENT_JSON_INTEGER
        );
        assert!(matches!(
            BroadcastSanitizedEvent::at_unix_millis(
                BroadcastSanitizedEventKind::CallConnected,
                MAX_BROADCAST_EVENT_JSON_INTEGER + 1,
            ),
            Err(BroadcastSanitizedEventError::TimestampOutOfRange { .. })
        ));
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            serde_json::json!({
                "kind": "call-connected",
                "occurredAtUnixMillis": MAX_BROADCAST_EVENT_JSON_INTEGER,
            })
        );
        assert!(
            serde_json::from_value::<BroadcastSanitizedEvent>(serde_json::json!({
                "kind": "call-connected",
                "occurredAtUnixMillis": MAX_BROADCAST_EVENT_JSON_INTEGER + 1,
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<BroadcastSanitizedEvent>(serde_json::json!({
                "kind": "call-connected",
                "occurredAtUnixMillis": 1_000,
                "metadata": "forbidden",
            }))
            .is_err()
        );
    }
}
