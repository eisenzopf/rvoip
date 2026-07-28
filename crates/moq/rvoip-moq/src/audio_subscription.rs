//! rvoip-owned contracts for receiving the canonical LOC-03 Opus track.
//!
//! Draft-specific MOQT readers are converted to [`ManagedLocAudioObject`]
//! inside the private wire adapter. This module validates those primitives and
//! exposes only bounded, transport-independent events and snapshots.

use std::fmt;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use chrono::{DateTime, Utc};
use tokio::sync::{broadcast, watch};

use crate::{
    validate_opus_20ms_mono, LocAudioObject, LocError, MoqCatalogSubscriberConfig,
    MoqCatalogSubscriberConfigError, MoqNamespace, AUDIO_TRACK, OPUS_SAMPLE_RATE,
};

/// Default maximum encoded size of one received LOC Opus object.
pub const DEFAULT_MAX_AUDIO_OBJECT_BYTES: usize = 64 * 1024;
/// Hard safety cap for one received LOC Opus object.
pub const MAX_AUDIO_OBJECT_BYTES: usize = 1024 * 1024;
/// Default number of received objects retained for each bounded receiver.
pub const DEFAULT_AUDIO_QUEUE_OBJECTS: usize = 128;
/// Hard cap for a managed receiver's object queue.
pub const MAX_AUDIO_QUEUE_OBJECTS: usize = 4_096;

/// Configuration for the managed canonical audio subscriber.
///
/// The catalog configuration remains the single source of truth for endpoint,
/// namespace, substrate, credentials, timeouts, and reconnect policy.
#[derive(Clone)]
pub struct MoqAudioSubscriberConfig {
    pub catalog: MoqCatalogSubscriberConfig,
    pub max_audio_object_bytes: usize,
    pub queue_objects: usize,
}

impl MoqAudioSubscriberConfig {
    pub fn new(endpoint: url::Url, namespace: MoqNamespace) -> Self {
        Self {
            catalog: MoqCatalogSubscriberConfig::new(endpoint, namespace),
            max_audio_object_bytes: DEFAULT_MAX_AUDIO_OBJECT_BYTES,
            queue_objects: DEFAULT_AUDIO_QUEUE_OBJECTS,
        }
    }

    pub fn validate(&self) -> Result<(), MoqAudioSubscriberConfigError> {
        self.catalog.validate()?;
        if !(1..=MAX_AUDIO_OBJECT_BYTES).contains(&self.max_audio_object_bytes) {
            return Err(MoqAudioSubscriberConfigError::InvalidObjectLimit {
                maximum: MAX_AUDIO_OBJECT_BYTES,
            });
        }
        if !(1..=MAX_AUDIO_QUEUE_OBJECTS).contains(&self.queue_objects) {
            return Err(MoqAudioSubscriberConfigError::InvalidQueueCapacity {
                maximum: MAX_AUDIO_QUEUE_OBJECTS,
            });
        }
        Ok(())
    }
}

impl fmt::Debug for MoqAudioSubscriberConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MoqAudioSubscriberConfig")
            .field("catalog", &self.catalog)
            .field("max_audio_object_bytes", &self.max_audio_object_bytes)
            .field("queue_objects", &self.queue_objects)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqAudioSubscriberConfigError {
    #[error(transparent)]
    Catalog(#[from] MoqCatalogSubscriberConfigError),
    #[error("audio object limit must be between 1 and {maximum} bytes")]
    InvalidObjectLimit { maximum: usize },
    #[error("audio receiver queue must contain between 1 and {maximum} objects")]
    InvalidQueueCapacity { maximum: usize },
}

/// Managed audio-track lifecycle. Catalog validation always precedes
/// `Subscribing` and `Live`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MoqAudioSubscriberLifecycle {
    AwaitingCatalog,
    Subscribing,
    Live,
    Reconnecting,
    Draining,
    Closed,
    Failed,
}

impl MoqAudioSubscriberLifecycle {
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Failed)
    }
}

/// Bounded, API-safe audio failure categories.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum MoqAudioSubscriberFailure {
    SubscribeFailed,
    InvalidObject,
    TransportEnded,
}

/// Latest managed audio-track state without any wire-engine handles.
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoqAudioSubscriptionSnapshot {
    pub namespace: MoqNamespace,
    pub track: String,
    pub lifecycle: MoqAudioSubscriberLifecycle,
    pub lifecycle_since: DateTime<Utc>,
    pub received_objects: u64,
    pub rejected_objects: u64,
    pub last_group_id: Option<u64>,
    pub last_timestamp: Option<u64>,
    pub last_received_at: Option<DateTime<Utc>>,
    pub reconnects: u32,
    pub queue_capacity: usize,
    pub failure: Option<MoqAudioSubscriberFailure>,
}

/// One validated LOC-03 Opus object and its local receipt time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoqReceivedAudioObject {
    pub object: LocAudioObject,
    pub received_at: DateTime<Utc>,
}

/// Error from a bounded audio-object receiver.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum MoqAudioReceiveError {
    #[error("audio receiver fell behind by {skipped} objects")]
    Lagged { skipped: u64 },
    #[error("audio subscription closed")]
    Closed,
}

/// Bounded receiver for validated LOC audio objects.
///
/// Slow consumers receive an explicit `Lagged` error; they never exert
/// backpressure on the MOQT session or other listeners.
pub struct MoqAudioObjectReceiver {
    receiver: broadcast::Receiver<Arc<MoqReceivedAudioObject>>,
}

impl MoqAudioObjectReceiver {
    pub async fn recv(&mut self) -> Result<Arc<MoqReceivedAudioObject>, MoqAudioReceiveError> {
        self.receiver.recv().await.map_err(|error| match error {
            broadcast::error::RecvError::Lagged(skipped) => {
                MoqAudioReceiveError::Lagged { skipped }
            }
            broadcast::error::RecvError::Closed => MoqAudioReceiveError::Closed,
        })
    }

    /// Create another receiver at the current live edge.
    pub fn resubscribe(&self) -> Self {
        Self {
            receiver: self.receiver.resubscribe(),
        }
    }
}

/// Transport-neutral object emitted by the private draft-specific adapter.
pub(crate) struct ManagedLocAudioObject {
    pub(crate) namespace: String,
    pub(crate) track: String,
    pub(crate) group_id: u64,
    pub(crate) subgroup_id: u64,
    pub(crate) object_id: u64,
    pub(crate) first_object: bool,
    pub(crate) end_of_group: bool,
    pub(crate) extension_header_count: usize,
    pub(crate) timestamp: Option<u64>,
    pub(crate) timescale: Option<u64>,
    pub(crate) declared_payload_len: u64,
    pub(crate) payload: Bytes,
    pub(crate) received_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum MoqAudioValidationError {
    #[error("audio object namespace mismatch")]
    NamespaceMismatch,
    #[error("audio object track mismatch")]
    TrackMismatch,
    #[error("canonical LOC audio requires subgroup zero")]
    InvalidSubgroup,
    #[error("canonical LOC audio requires Object zero")]
    InvalidObject,
    #[error("canonical LOC audio requires one complete object per group")]
    InvalidGroupBoundary,
    #[error("canonical LOC audio requires exactly Timestamp and Timescale properties")]
    InvalidProperties,
    #[error("LOC Timescale must be 48000")]
    InvalidTimescale,
    #[error("audio object declared payload length differs from the delivered payload")]
    PayloadLengthMismatch,
    #[error("audio object exceeds the configured payload limit")]
    PayloadTooLarge,
    #[error("audio group coordinate did not advance")]
    CoordinateRegression,
    #[error("LOC timestamp did not advance")]
    TimestampRegression,
    #[error(transparent)]
    InvalidOpus(#[from] LocError),
    #[error("audio object counter overflowed")]
    CounterOverflow,
}

struct MoqAudioStateMachine {
    namespace: MoqNamespace,
    max_audio_object_bytes: usize,
    last_group_id: Option<u64>,
    last_timestamp: Option<u64>,
    accepted: u64,
}

impl MoqAudioStateMachine {
    fn new(config: &MoqAudioSubscriberConfig) -> Self {
        Self {
            namespace: config.catalog.namespace.clone(),
            max_audio_object_bytes: config.max_audio_object_bytes,
            last_group_id: None,
            last_timestamp: None,
            accepted: 0,
        }
    }

    fn apply(
        &mut self,
        object: ManagedLocAudioObject,
    ) -> Result<MoqReceivedAudioObject, MoqAudioValidationError> {
        if object.namespace != self.namespace.as_str() {
            return Err(MoqAudioValidationError::NamespaceMismatch);
        }
        if object.track != AUDIO_TRACK {
            return Err(MoqAudioValidationError::TrackMismatch);
        }
        if object.subgroup_id != 0 {
            return Err(MoqAudioValidationError::InvalidSubgroup);
        }
        if object.object_id != 0 {
            return Err(MoqAudioValidationError::InvalidObject);
        }
        if !object.first_object || !object.end_of_group {
            return Err(MoqAudioValidationError::InvalidGroupBoundary);
        }
        if object.extension_header_count != 2 {
            return Err(MoqAudioValidationError::InvalidProperties);
        }
        let timestamp = object
            .timestamp
            .ok_or(MoqAudioValidationError::InvalidProperties)?;
        let timescale = object
            .timescale
            .ok_or(MoqAudioValidationError::InvalidProperties)?;
        if timescale != u64::from(OPUS_SAMPLE_RATE) {
            return Err(MoqAudioValidationError::InvalidTimescale);
        }
        if object.declared_payload_len != u64::try_from(object.payload.len()).unwrap_or(u64::MAX) {
            return Err(MoqAudioValidationError::PayloadLengthMismatch);
        }
        if object.payload.len() > self.max_audio_object_bytes {
            return Err(MoqAudioValidationError::PayloadTooLarge);
        }
        if self
            .last_group_id
            .is_some_and(|previous| object.group_id <= previous)
        {
            return Err(MoqAudioValidationError::CoordinateRegression);
        }
        if self
            .last_timestamp
            .is_some_and(|previous| timestamp <= previous)
        {
            return Err(MoqAudioValidationError::TimestampRegression);
        }
        validate_opus_20ms_mono(&object.payload)?;
        self.accepted = self
            .accepted
            .checked_add(1)
            .ok_or(MoqAudioValidationError::CounterOverflow)?;
        self.last_group_id = Some(object.group_id);
        self.last_timestamp = Some(timestamp);
        Ok(MoqReceivedAudioObject {
            object: LocAudioObject {
                group_id: object.group_id,
                object_id: object.object_id,
                timestamp,
                timescale: OPUS_SAMPLE_RATE,
                payload: object.payload,
            },
            received_at: object.received_at,
        })
    }
}

pub(crate) struct AudioSubscriberStatus {
    snapshot: watch::Sender<MoqAudioSubscriptionSnapshot>,
    objects: broadcast::Sender<Arc<MoqReceivedAudioObject>>,
    validator: Mutex<MoqAudioStateMachine>,
}

impl AudioSubscriberStatus {
    pub(crate) fn new(config: &MoqAudioSubscriberConfig) -> Arc<Self> {
        let now = Utc::now();
        let (snapshot, _) = watch::channel(MoqAudioSubscriptionSnapshot {
            namespace: config.catalog.namespace.clone(),
            track: AUDIO_TRACK.to_owned(),
            lifecycle: MoqAudioSubscriberLifecycle::AwaitingCatalog,
            lifecycle_since: now,
            received_objects: 0,
            rejected_objects: 0,
            last_group_id: None,
            last_timestamp: None,
            last_received_at: None,
            reconnects: 0,
            queue_capacity: config.queue_objects,
            failure: None,
        });
        let (objects, _) = broadcast::channel(config.queue_objects);
        Arc::new(Self {
            snapshot,
            objects,
            validator: Mutex::new(MoqAudioStateMachine::new(config)),
        })
    }

    pub(crate) fn snapshot(&self) -> MoqAudioSubscriptionSnapshot {
        self.snapshot.borrow().clone()
    }

    pub(crate) fn updates(&self) -> watch::Receiver<MoqAudioSubscriptionSnapshot> {
        self.snapshot.subscribe()
    }

    pub(crate) async fn wait_terminal(&self) -> MoqAudioSubscriptionSnapshot {
        let mut receiver = self.updates();
        loop {
            let snapshot = receiver.borrow().clone();
            if snapshot.lifecycle.is_terminal() {
                return snapshot;
            }
            if receiver.changed().await.is_err() {
                return snapshot;
            }
        }
    }

    pub(crate) fn objects(&self) -> MoqAudioObjectReceiver {
        MoqAudioObjectReceiver {
            receiver: self.objects.subscribe(),
        }
    }

    pub(crate) fn subscribing(&self) {
        self.transition(MoqAudioSubscriberLifecycle::Subscribing, None);
    }

    pub(crate) fn live(&self) {
        self.transition(MoqAudioSubscriberLifecycle::Live, None);
    }

    pub(crate) fn reconnecting(&self, reconnects: u32) {
        self.transition(MoqAudioSubscriberLifecycle::Reconnecting, None);
        self.snapshot.send_modify(|snapshot| {
            snapshot.reconnects = reconnects;
        });
    }

    pub(crate) fn draining(&self) {
        self.transition(MoqAudioSubscriberLifecycle::Draining, None);
    }

    pub(crate) fn closed(&self) {
        self.transition(MoqAudioSubscriberLifecycle::Closed, None);
    }

    pub(crate) fn subscribe_failed(&self) {
        self.transition(
            MoqAudioSubscriberLifecycle::Failed,
            Some(MoqAudioSubscriberFailure::SubscribeFailed),
        );
    }

    pub(crate) fn transport_ended(&self) {
        if !self.snapshot().lifecycle.is_terminal() {
            self.transition(
                MoqAudioSubscriberLifecycle::Failed,
                Some(MoqAudioSubscriberFailure::TransportEnded),
            );
        }
    }

    pub(crate) fn invalid_object(&self) {
        self.snapshot.send_modify(|snapshot| {
            snapshot.rejected_objects = snapshot.rejected_objects.saturating_add(1);
            snapshot.lifecycle = MoqAudioSubscriberLifecycle::Failed;
            snapshot.lifecycle_since = Utc::now();
            snapshot.failure = Some(MoqAudioSubscriberFailure::InvalidObject);
        });
        metrics::counter!("rvoip_moq_subscriber_audio_rejected_total").increment(1);
    }

    pub(crate) fn accept(
        &self,
        object: ManagedLocAudioObject,
    ) -> Result<(), MoqAudioValidationError> {
        let received = match self
            .validator
            .lock()
            .expect("MOQT audio validator lock poisoned")
            .apply(object)
        {
            Ok(received) => received,
            Err(error) => {
                self.invalid_object();
                return Err(error);
            }
        };
        let event = Arc::new(received);
        self.snapshot.send_modify(|snapshot| {
            snapshot.lifecycle = MoqAudioSubscriberLifecycle::Live;
            snapshot.failure = None;
            snapshot.received_objects = snapshot.received_objects.saturating_add(1);
            snapshot.last_group_id = Some(event.object.group_id);
            snapshot.last_timestamp = Some(event.object.timestamp);
            snapshot.last_received_at = Some(event.received_at);
        });
        let _ = self.objects.send(event);
        metrics::counter!("rvoip_moq_subscriber_audio_objects_total").increment(1);
        Ok(())
    }

    pub(crate) fn fatal_invalid_object(&self) -> bool {
        self.snapshot().failure == Some(MoqAudioSubscriberFailure::InvalidObject)
    }

    fn transition(
        &self,
        lifecycle: MoqAudioSubscriberLifecycle,
        failure: Option<MoqAudioSubscriberFailure>,
    ) {
        self.snapshot.send_modify(|snapshot| {
            if snapshot.lifecycle.is_terminal() {
                return;
            }
            snapshot.lifecycle = lifecycle;
            snapshot.lifecycle_since = Utc::now();
            snapshot.failure = failure;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> MoqAudioSubscriberConfig {
        MoqAudioSubscriberConfig::new(
            url::Url::parse("moqt://relay.example/tenant/broadcast").unwrap(),
            MoqNamespace::new("tenant", "broadcast").unwrap(),
        )
    }

    fn object(group_id: u64, timestamp: u64) -> ManagedLocAudioObject {
        ManagedLocAudioObject {
            namespace: "tenant/broadcast".to_owned(),
            track: AUDIO_TRACK.to_owned(),
            group_id,
            subgroup_id: 0,
            object_id: 0,
            first_object: true,
            end_of_group: true,
            extension_header_count: 2,
            timestamp: Some(timestamp),
            timescale: Some(48_000),
            declared_payload_len: 2,
            payload: Bytes::from_static(&[0x78, 0x00]),
            received_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn bounded_receiver_exposes_only_validated_loc_objects() {
        let status = AudioSubscriberStatus::new(&config());
        let mut receiver = status.objects();
        status.subscribing();
        status.live();
        status.accept(object(7, 960)).unwrap();
        let received = receiver.recv().await.unwrap();
        assert_eq!(received.object.group_id, 7);
        assert_eq!(received.object.timestamp, 960);
        assert_eq!(received.object.timescale, 48_000);
        assert_eq!(status.snapshot().received_objects, 1);
    }

    #[test]
    fn rejects_noncanonical_properties_and_coordinate_reuse() {
        let status = AudioSubscriberStatus::new(&config());
        let mut invalid = object(0, 0);
        invalid.extension_header_count = 3;
        assert_eq!(
            status.accept(invalid).unwrap_err(),
            MoqAudioValidationError::InvalidProperties
        );

        let status = AudioSubscriberStatus::new(&config());
        status.accept(object(0, 0)).unwrap();
        assert_eq!(
            status.accept(object(0, 960)).unwrap_err(),
            MoqAudioValidationError::CoordinateRegression
        );
        assert_eq!(status.snapshot().rejected_objects, 1);
    }

    #[test]
    fn validates_audio_limits_without_weakening_catalog_policy() {
        let mut config = config();
        config.queue_objects = 0;
        assert_eq!(
            config.validate().unwrap_err(),
            MoqAudioSubscriberConfigError::InvalidQueueCapacity {
                maximum: MAX_AUDIO_QUEUE_OBJECTS
            }
        );
        config.queue_objects = 1;
        config.max_audio_object_bytes = MAX_AUDIO_OBJECT_BYTES + 1;
        assert_eq!(
            config.validate().unwrap_err(),
            MoqAudioSubscriberConfigError::InvalidObjectLimit {
                maximum: MAX_AUDIO_OBJECT_BYTES
            }
        );
    }
}
