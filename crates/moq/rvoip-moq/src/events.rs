use std::collections::VecDeque;

use rvoip_core_traits::broadcast::{
    BroadcastSanitizedEvent as MoqSanitizedEvent,
    BroadcastSanitizedEventKind as MoqSanitizedEventKind, MAX_BROADCAST_EVENT_JSON_INTEGER,
};
use serde::Serialize;

use crate::MoqError;

/// Reverse-DNS event type advertised by the MSF-01 event-timeline track.
pub const SANITIZED_EVENTS_EVENT_TYPE: &str = "io.rvoip.sanitized-call-events.v1";

/// Maximum number of events waiting to be packaged by one publisher.
pub const MAX_SANITIZED_EVENT_QUEUE_EVENTS: usize = 1_024;

/// Maximum accessible event history repeated in an independent MSF object.
pub const MAX_SANITIZED_EVENT_HISTORY_EVENTS: usize = 256;

/// Hard limit for one independent event-timeline JSON object.
pub const MAX_SANITIZED_EVENT_PAYLOAD_BYTES: usize = 64 * 1_024;

/// Explicit opt-in limits for the sanitized event-timeline track.
///
/// This type intentionally has no `Default` implementation: creating it is
/// the application-level decision that enables broadcast event disclosure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoqSanitizedEventsConfig {
    queue_events: usize,
    history_events: usize,
}

impl MoqSanitizedEventsConfig {
    pub fn new(
        queue_events: usize,
        history_events: usize,
    ) -> Result<Self, MoqSanitizedEventsConfigError> {
        if !(1..=MAX_SANITIZED_EVENT_QUEUE_EVENTS).contains(&queue_events) {
            return Err(MoqSanitizedEventsConfigError::InvalidQueueCapacity {
                maximum: MAX_SANITIZED_EVENT_QUEUE_EVENTS,
            });
        }
        if !(1..=MAX_SANITIZED_EVENT_HISTORY_EVENTS).contains(&history_events) {
            return Err(MoqSanitizedEventsConfigError::InvalidHistoryCapacity {
                maximum: MAX_SANITIZED_EVENT_HISTORY_EVENTS,
            });
        }
        Ok(Self {
            queue_events,
            history_events,
        })
    }

    pub const fn queue_events(&self) -> usize {
        self.queue_events
    }

    pub const fn history_events(&self) -> usize {
        self.history_events
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum MoqSanitizedEventsConfigError {
    #[error("sanitized MOQT event queue capacity must be between 1 and {maximum}")]
    InvalidQueueCapacity { maximum: usize },
    #[error("sanitized MOQT event history capacity must be between 1 and {maximum}")]
    InvalidHistoryCapacity { maximum: usize },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SanitizedEventTimelineRecord {
    /// MSF-01 wallclock index in Unix milliseconds.
    t: u64,
    data: SanitizedEventData,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct SanitizedEventData {
    version: &'static str,
    sequence: u64,
    kind: MoqSanitizedEventKind,
}

pub(crate) struct MoqSanitizedEventTimeline {
    history: VecDeque<SanitizedEventTimelineRecord>,
    history_events: usize,
    last_sequence: Option<u64>,
}

impl MoqSanitizedEventTimeline {
    pub(crate) fn new(config: MoqSanitizedEventsConfig) -> Self {
        Self {
            history: VecDeque::with_capacity(config.history_events),
            history_events: config.history_events,
            last_sequence: None,
        }
    }

    /// Build the next independent MSF-01 event-timeline object.
    ///
    /// Every object contains the complete accessible bounded history, so a
    /// subscriber may join at any group boundary without prior objects.
    /// History is process-local by design. A restarted publisher begins with
    /// an empty accessible history while its persisted Group ID allocator
    /// keeps `sequence` monotonic across the restart.
    pub(crate) fn push(
        &mut self,
        event: MoqSanitizedEvent,
        sequence: u64,
    ) -> Result<Vec<u8>, MoqError> {
        if sequence > MAX_BROADCAST_EVENT_JSON_INTEGER {
            return Err(MoqError::SanitizedEventSequenceOutOfRange {
                maximum: MAX_BROADCAST_EVENT_JSON_INTEGER,
                actual: sequence,
            });
        }
        if self
            .last_sequence
            .is_some_and(|previous| sequence <= previous)
        {
            return Err(MoqError::SanitizedEventSequenceNotMonotonic);
        }
        let record = SanitizedEventTimelineRecord {
            t: event.occurred_at_unix_millis(),
            data: SanitizedEventData {
                version: SANITIZED_EVENTS_EVENT_TYPE,
                sequence,
                kind: event.kind(),
            },
        };
        let mut candidate = self.history.clone();
        if candidate.len() == self.history_events {
            candidate.pop_front();
        }
        candidate.push_back(record);
        let payload = serde_json::to_vec(&candidate).map_err(MoqError::SanitizedEventEncoding)?;
        if payload.len() > MAX_SANITIZED_EVENT_PAYLOAD_BYTES {
            return Err(MoqError::SanitizedEventPayloadTooLarge {
                maximum: MAX_SANITIZED_EVENT_PAYLOAD_BYTES,
                actual: payload.len(),
            });
        }
        self.history = candidate;
        self.last_sequence = Some(sequence);
        Ok(payload)
    }
}

pub(crate) fn sequence_for_group_id(group_id: u64) -> Result<u64, MoqError> {
    let sequence = group_id
        .checked_add(1)
        .ok_or(MoqError::SanitizedEventSequenceOutOfRange {
            maximum: MAX_BROADCAST_EVENT_JSON_INTEGER,
            actual: u64::MAX,
        })?;
    if sequence > MAX_BROADCAST_EVENT_JSON_INTEGER {
        return Err(MoqError::SanitizedEventSequenceOutOfRange {
            maximum: MAX_BROADCAST_EVENT_JSON_INTEGER,
            actual: sequence,
        });
    }
    Ok(sequence)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn configuration_is_explicit_and_strictly_bounded() {
        assert!(MoqSanitizedEventsConfig::new(1, 1).is_ok());
        assert!(MoqSanitizedEventsConfig::new(
            MAX_SANITIZED_EVENT_QUEUE_EVENTS,
            MAX_SANITIZED_EVENT_HISTORY_EVENTS
        )
        .is_ok());
        assert!(MoqSanitizedEventsConfig::new(0, 1).is_err());
        assert!(MoqSanitizedEventsConfig::new(1, 0).is_err());
        assert!(MoqSanitizedEventsConfig::new(MAX_SANITIZED_EVENT_QUEUE_EVENTS + 1, 1).is_err());
        assert!(MoqSanitizedEventsConfig::new(1, MAX_SANITIZED_EVENT_HISTORY_EVENTS + 1).is_err());
    }

    #[test]
    fn independent_objects_repeat_only_bounded_fixed_schema_history() {
        let config = MoqSanitizedEventsConfig::new(2, 2).unwrap();
        let mut timeline = MoqSanitizedEventTimeline::new(config);
        let first = timeline
            .push(
                MoqSanitizedEvent::at_unix_millis(MoqSanitizedEventKind::CallConnecting, 1_000)
                    .unwrap(),
                1,
            )
            .unwrap();
        let second = timeline
            .push(
                MoqSanitizedEvent::at_unix_millis(MoqSanitizedEventKind::CallConnected, 2_000)
                    .unwrap(),
                2,
            )
            .unwrap();
        let third = timeline
            .push(
                MoqSanitizedEvent::at_unix_millis(MoqSanitizedEventKind::CallEnded, 3_000).unwrap(),
                3,
            )
            .unwrap();

        let first: serde_json::Value = serde_json::from_slice(&first).unwrap();
        assert_eq!(first.as_array().unwrap().len(), 1);
        let second: serde_json::Value = serde_json::from_slice(&second).unwrap();
        assert_eq!(second.as_array().unwrap().len(), 2);
        let third: serde_json::Value = serde_json::from_slice(&third).unwrap();
        assert_eq!(third.as_array().unwrap().len(), 2);
        assert_eq!(third[0]["t"], 2_000);
        assert_eq!(third[0]["data"]["sequence"], 2);
        assert_eq!(third[1]["data"]["kind"], "call-ended");
        assert_eq!(third[1]["data"]["version"], SANITIZED_EVENTS_EVENT_TYPE);
        for record in third.as_array().unwrap() {
            assert_eq!(
                record
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["data", "t"])
            );
            assert_eq!(
                record["data"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["kind", "sequence", "version"])
            );
        }

        let encoded = serde_json::to_string(&third).unwrap();
        for forbidden in [
            "tenant",
            "broadcast",
            "call_id",
            "correlation",
            "provider",
            "sip",
            "header",
            "metadata",
        ] {
            assert!(!encoded.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn maximum_history_remains_under_the_payload_limit() {
        let config = MoqSanitizedEventsConfig::new(1, MAX_SANITIZED_EVENT_HISTORY_EVENTS).unwrap();
        let mut timeline = MoqSanitizedEventTimeline::new(config);
        let mut payload = Vec::new();
        for timestamp in 0..MAX_SANITIZED_EVENT_HISTORY_EVENTS as u64 {
            payload = timeline
                .push(
                    MoqSanitizedEvent::at_unix_millis(
                        MoqSanitizedEventKind::TransferCompleted,
                        timestamp,
                    )
                    .unwrap(),
                    timestamp + 1,
                )
                .unwrap();
        }
        assert!(payload.len() <= MAX_SANITIZED_EVENT_PAYLOAD_BYTES);
    }

    #[test]
    fn persisted_group_ids_keep_sequence_monotonic_while_restart_resets_history() {
        let config = MoqSanitizedEventsConfig::new(2, 2).unwrap();
        let mut first_process = MoqSanitizedEventTimeline::new(config);
        let first = first_process
            .push(
                MoqSanitizedEvent::at_unix_millis(MoqSanitizedEventKind::CallConnecting, 1_000)
                    .unwrap(),
                sequence_for_group_id(40).unwrap(),
            )
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&first).unwrap()[0]["data"]["sequence"],
            41
        );

        let mut restarted_process = MoqSanitizedEventTimeline::new(config);
        let after_restart = restarted_process
            .push(
                MoqSanitizedEvent::at_unix_millis(MoqSanitizedEventKind::CallConnected, 2_000)
                    .unwrap(),
                sequence_for_group_id(41).unwrap(),
            )
            .unwrap();
        let after_restart: serde_json::Value = serde_json::from_slice(&after_restart).unwrap();
        assert_eq!(after_restart.as_array().unwrap().len(), 1);
        assert_eq!(after_restart[0]["data"]["sequence"], 42);
    }

    #[test]
    fn timestamp_and_sequence_reject_non_json_safe_integers() {
        assert!(MoqSanitizedEvent::at_unix_millis(
            MoqSanitizedEventKind::CallConnected,
            MAX_BROADCAST_EVENT_JSON_INTEGER,
        )
        .is_ok());
        assert!(MoqSanitizedEvent::at_unix_millis(
            MoqSanitizedEventKind::CallConnected,
            MAX_BROADCAST_EVENT_JSON_INTEGER + 1,
        )
        .is_err());
        assert_eq!(
            sequence_for_group_id(MAX_BROADCAST_EVENT_JSON_INTEGER - 1).unwrap(),
            MAX_BROADCAST_EVENT_JSON_INTEGER
        );
        assert!(sequence_for_group_id(MAX_BROADCAST_EVENT_JSON_INTEGER).is_err());
    }
}
