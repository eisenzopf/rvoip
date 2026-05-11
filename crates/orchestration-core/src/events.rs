use crate::ids::*;
use crate::types::*;
use crate::voice_ai::TranscriptEvent;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use rvoip_infra_common::events::cross_crate::{
    OrchestrationCrossCrateEvent, RvoipCrossCrateEvent,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrchestrationEventEnvelope {
    pub event_id: EventId,
    pub sequence: u64,
    pub occurred_at: DateTime<Utc>,
    pub call_id: Option<CallId>,
    pub correlation_id: Option<String>,
    pub event: OrchestrationEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrchestrationEvent {
    InboundCallReceived {
        call_id: CallId,
        caller: CallerIdentity,
        to: String,
    },
    CallCreated {
        call: Call,
    },
    CallQueued {
        call_id: CallId,
        queue_id: QueueId,
    },
    CallDequeued {
        call_id: CallId,
        queue_id: QueueId,
    },
    QueueOverflowed {
        call_id: CallId,
        from_queue_id: QueueId,
        target: OverflowTarget,
        reason: String,
    },
    CallStatusChanged {
        call_id: CallId,
        from: CallStatus,
        to: CallStatus,
    },
    AgentStateChanged {
        agent_id: AgentId,
        from: AgentState,
        to: AgentState,
    },
    AgentReserved {
        call_id: CallId,
        agent_id: AgentId,
        offer_id: AgentOfferId,
    },
    AgentOfferAccepted {
        call_id: CallId,
        agent_id: AgentId,
        offer_id: AgentOfferId,
    },
    AgentOfferRejected {
        call_id: CallId,
        agent_id: AgentId,
        offer_id: AgentOfferId,
        reason: String,
    },
    AgentOfferTimedOut {
        call_id: CallId,
        agent_id: AgentId,
        offer_id: AgentOfferId,
    },
    AgentOfferFailed {
        call_id: CallId,
        agent_id: AgentId,
        offer_id: AgentOfferId,
        reason: String,
    },
    VoiceAiStarted {
        call_id: CallId,
        agent_id: AgentId,
    },
    VoiceAiTranscript {
        call_id: CallId,
        agent_id: AgentId,
        transcript: TranscriptEvent,
    },
    VoiceAiBargeIn {
        call_id: CallId,
        agent_id: AgentId,
    },
    VoiceAiEnded {
        call_id: CallId,
        agent_id: AgentId,
        reason: String,
    },
    BridgeStarted {
        call_id: CallId,
        bridge_id: BridgeId,
        caller_leg_id: CallLegId,
        agent_leg_id: CallLegId,
    },
    BridgeEnded {
        call_id: CallId,
        bridge_id: BridgeId,
        reason: String,
    },
    RecordingStarted {
        call_id: CallId,
        recording_id: RecordingId,
    },
    RecordingStopped {
        call_id: CallId,
        recording_id: RecordingId,
    },
    TransferRequested {
        call_id: CallId,
        from_agent_id: AgentId,
        target: TransferTarget,
    },
    TransferCompleted {
        call_id: CallId,
        target: TransferTarget,
    },
    CallEnded {
        call_id: CallId,
        reason: String,
    },
    CallFailed {
        call_id: CallId,
        reason: String,
    },
}

impl OrchestrationEvent {
    pub fn call_id(&self) -> Option<CallId> {
        match self {
            Self::InboundCallReceived { call_id, .. }
            | Self::CallQueued { call_id, .. }
            | Self::CallDequeued { call_id, .. }
            | Self::QueueOverflowed { call_id, .. }
            | Self::CallStatusChanged { call_id, .. }
            | Self::AgentReserved { call_id, .. }
            | Self::AgentOfferAccepted { call_id, .. }
            | Self::AgentOfferRejected { call_id, .. }
            | Self::AgentOfferTimedOut { call_id, .. }
            | Self::AgentOfferFailed { call_id, .. }
            | Self::VoiceAiStarted { call_id, .. }
            | Self::VoiceAiTranscript { call_id, .. }
            | Self::VoiceAiBargeIn { call_id, .. }
            | Self::VoiceAiEnded { call_id, .. }
            | Self::BridgeStarted { call_id, .. }
            | Self::BridgeEnded { call_id, .. }
            | Self::RecordingStarted { call_id, .. }
            | Self::RecordingStopped { call_id, .. }
            | Self::TransferRequested { call_id, .. }
            | Self::TransferCompleted { call_id, .. }
            | Self::CallEnded { call_id, .. }
            | Self::CallFailed { call_id, .. } => Some(call_id.clone()),
            Self::CallCreated { call } => Some(call.id.clone()),
            Self::AgentStateChanged { .. } => None,
        }
    }

    /// Stable per-variant tag used to route events to per-type broadcast
    /// channels. Matches the wire-format `event_type` strings registered
    /// with `GlobalEventCoordinator`.
    pub fn variant_kind(&self) -> &'static str {
        match self {
            Self::InboundCallReceived { .. } => "orchestration.inbound_call_received",
            Self::CallCreated { .. } => "orchestration.call_created",
            Self::CallQueued { .. } => "orchestration.call_queued",
            Self::CallDequeued { .. } => "orchestration.call_dequeued",
            Self::QueueOverflowed { .. } => "orchestration.queue_overflowed",
            Self::CallStatusChanged { .. } => "orchestration.call_status_changed",
            Self::AgentStateChanged { .. } => "orchestration.agent_state_changed",
            Self::AgentReserved { .. } => "orchestration.agent_reserved",
            Self::AgentOfferAccepted { .. } => "orchestration.agent_offer_accepted",
            Self::AgentOfferRejected { .. } => "orchestration.agent_offer_rejected",
            Self::AgentOfferTimedOut { .. } => "orchestration.agent_offer_timed_out",
            Self::AgentOfferFailed { .. } => "orchestration.agent_offer_failed",
            Self::VoiceAiStarted { .. } => "orchestration.voice_ai_started",
            Self::VoiceAiTranscript { .. } => "orchestration.voice_ai_transcript",
            Self::VoiceAiBargeIn { .. } => "orchestration.voice_ai_barge_in",
            Self::VoiceAiEnded { .. } => "orchestration.voice_ai_ended",
            Self::BridgeStarted { .. } => "orchestration.bridge_started",
            Self::BridgeEnded { .. } => "orchestration.bridge_ended",
            Self::RecordingStarted { .. } => "orchestration.recording_started",
            Self::RecordingStopped { .. } => "orchestration.recording_stopped",
            Self::TransferRequested { .. } => "orchestration.transfer_requested",
            Self::TransferCompleted { .. } => "orchestration.transfer_completed",
            Self::CallEnded { .. } => "orchestration.call_ended",
            Self::CallFailed { .. } => "orchestration.call_failed",
        }
    }

    /// Convert to the wire form published on `GlobalEventCoordinator` for
    /// cross-crate observers. Lossy by design — rich struct payloads
    /// (`Call`, `TranscriptEvent`) are flattened to primitive fields. In-crate
    /// subscribers should use the typed bus instead.
    pub fn to_cross_crate(&self) -> OrchestrationCrossCrateEvent {
        match self {
            Self::InboundCallReceived { call_id, caller, to } => {
                OrchestrationCrossCrateEvent::InboundCallReceived {
                    call_id: call_id.to_string(),
                    caller_uri: caller.uri.clone(),
                    to: to.clone(),
                }
            }
            Self::CallCreated { call } => OrchestrationCrossCrateEvent::CallCreated {
                call_id: call.id.to_string(),
            },
            Self::CallQueued { call_id, queue_id } => OrchestrationCrossCrateEvent::CallQueued {
                call_id: call_id.to_string(),
                queue_id: queue_id.to_string(),
            },
            Self::CallDequeued { call_id, queue_id } => {
                OrchestrationCrossCrateEvent::CallDequeued {
                    call_id: call_id.to_string(),
                    queue_id: queue_id.to_string(),
                }
            }
            Self::QueueOverflowed {
                call_id,
                from_queue_id,
                target,
                reason,
            } => OrchestrationCrossCrateEvent::QueueOverflowed {
                call_id: call_id.to_string(),
                from_queue_id: from_queue_id.to_string(),
                target: format!("{target:?}"),
                reason: reason.clone(),
            },
            Self::CallStatusChanged { call_id, from, to } => {
                OrchestrationCrossCrateEvent::CallStatusChanged {
                    call_id: call_id.to_string(),
                    from: format!("{from:?}"),
                    to: format!("{to:?}"),
                }
            }
            Self::AgentStateChanged { agent_id, from, to } => {
                OrchestrationCrossCrateEvent::AgentStateChanged {
                    agent_id: agent_id.to_string(),
                    from: format!("{from:?}"),
                    to: format!("{to:?}"),
                }
            }
            Self::AgentReserved {
                call_id,
                agent_id,
                offer_id,
            } => OrchestrationCrossCrateEvent::AgentReserved {
                call_id: call_id.to_string(),
                agent_id: agent_id.to_string(),
                offer_id: offer_id.to_string(),
            },
            Self::AgentOfferAccepted {
                call_id,
                agent_id,
                offer_id,
            } => OrchestrationCrossCrateEvent::AgentOfferAccepted {
                call_id: call_id.to_string(),
                agent_id: agent_id.to_string(),
                offer_id: offer_id.to_string(),
            },
            Self::AgentOfferRejected {
                call_id,
                agent_id,
                offer_id,
                reason,
            } => OrchestrationCrossCrateEvent::AgentOfferRejected {
                call_id: call_id.to_string(),
                agent_id: agent_id.to_string(),
                offer_id: offer_id.to_string(),
                reason: reason.clone(),
            },
            Self::AgentOfferTimedOut {
                call_id,
                agent_id,
                offer_id,
            } => OrchestrationCrossCrateEvent::AgentOfferTimedOut {
                call_id: call_id.to_string(),
                agent_id: agent_id.to_string(),
                offer_id: offer_id.to_string(),
            },
            Self::AgentOfferFailed {
                call_id,
                agent_id,
                offer_id,
                reason,
            } => OrchestrationCrossCrateEvent::AgentOfferFailed {
                call_id: call_id.to_string(),
                agent_id: agent_id.to_string(),
                offer_id: offer_id.to_string(),
                reason: reason.clone(),
            },
            Self::VoiceAiStarted { call_id, agent_id } => {
                OrchestrationCrossCrateEvent::VoiceAiStarted {
                    call_id: call_id.to_string(),
                    agent_id: agent_id.to_string(),
                }
            }
            Self::VoiceAiTranscript {
                call_id,
                agent_id,
                transcript,
            } => {
                let (text, is_final) = match transcript {
                    TranscriptEvent::Partial { text, .. } => (text.clone(), false),
                    TranscriptEvent::Final { text, .. } => (text.clone(), true),
                    TranscriptEvent::EndOfUtterance => (String::new(), true),
                    TranscriptEvent::Error { reason } => (reason.clone(), true),
                };
                OrchestrationCrossCrateEvent::VoiceAiTranscript {
                    call_id: call_id.to_string(),
                    agent_id: agent_id.to_string(),
                    text,
                    is_final,
                }
            }
            Self::VoiceAiBargeIn { call_id, agent_id } => {
                OrchestrationCrossCrateEvent::VoiceAiBargeIn {
                    call_id: call_id.to_string(),
                    agent_id: agent_id.to_string(),
                }
            }
            Self::VoiceAiEnded {
                call_id,
                agent_id,
                reason,
            } => OrchestrationCrossCrateEvent::VoiceAiEnded {
                call_id: call_id.to_string(),
                agent_id: agent_id.to_string(),
                reason: reason.clone(),
            },
            Self::BridgeStarted {
                call_id,
                bridge_id,
                caller_leg_id,
                agent_leg_id,
            } => OrchestrationCrossCrateEvent::BridgeStarted {
                call_id: call_id.to_string(),
                bridge_id: bridge_id.to_string(),
                caller_leg_id: caller_leg_id.to_string(),
                agent_leg_id: agent_leg_id.to_string(),
            },
            Self::BridgeEnded {
                call_id,
                bridge_id,
                reason,
            } => OrchestrationCrossCrateEvent::BridgeEnded {
                call_id: call_id.to_string(),
                bridge_id: bridge_id.to_string(),
                reason: reason.clone(),
            },
            Self::RecordingStarted {
                call_id,
                recording_id,
            } => OrchestrationCrossCrateEvent::RecordingStarted {
                call_id: call_id.to_string(),
                recording_id: recording_id.to_string(),
            },
            Self::RecordingStopped {
                call_id,
                recording_id,
            } => OrchestrationCrossCrateEvent::RecordingStopped {
                call_id: call_id.to_string(),
                recording_id: recording_id.to_string(),
            },
            Self::TransferRequested {
                call_id,
                from_agent_id,
                target,
            } => OrchestrationCrossCrateEvent::TransferRequested {
                call_id: call_id.to_string(),
                from_agent_id: from_agent_id.to_string(),
                target: format!("{target:?}"),
            },
            Self::TransferCompleted { call_id, target } => {
                OrchestrationCrossCrateEvent::TransferCompleted {
                    call_id: call_id.to_string(),
                    target: format!("{target:?}"),
                }
            }
            Self::CallEnded { call_id, reason } => OrchestrationCrossCrateEvent::CallEnded {
                call_id: call_id.to_string(),
                reason: reason.clone(),
            },
            Self::CallFailed { call_id, reason } => OrchestrationCrossCrateEvent::CallFailed {
                call_id: call_id.to_string(),
                reason: reason.clone(),
            },
        }
    }
}

/// Per-process orchestration event bus.
///
/// Backed by per-variant `tokio::broadcast` channels indexed by
/// `OrchestrationEvent::variant_kind()` so a slow consumer of one variant
/// (e.g. `VoiceAiTranscript`) does not lag a consumer of another (e.g.
/// `CallStatusChanged`). The legacy `subscribe()` method continues to return
/// a unified stream of every variant for backward compatibility.
///
/// When constructed with a `GlobalEventCoordinator`, every event is also
/// shadow-published as `RvoipCrossCrateEvent::Orchestration(..)` for
/// cross-crate observers (telemetry, future rvoip-harness).
#[derive(Clone)]
pub struct OrchestrationEventBus {
    /// Unified channel — every event lands here. Preserves the existing
    /// `subscribe()` API.
    fanout: broadcast::Sender<OrchestrationEventEnvelope>,
    /// Per-variant channels — created lazily on first `subscribe_kind` call.
    per_variant: Arc<DashMap<&'static str, broadcast::Sender<OrchestrationEventEnvelope>>>,
    /// Default capacity used for both `fanout` and any per-variant channel.
    capacity: usize,
    /// Monotonic sequence stamped onto every envelope. `Relaxed` ordering —
    /// downstream replay code that needs strict ordering should not rely on
    /// this counter alone. The previous `SeqCst` was a global memory barrier
    /// per emit; `Relaxed` removes that contention.
    sequence: Arc<AtomicU64>,
    /// Optional cross-crate fan-out via the platform-wide coordinator.
    coordinator: Option<Arc<GlobalEventCoordinator>>,
}

impl OrchestrationEventBus {
    pub fn new(capacity: usize) -> Self {
        let (fanout, _) = broadcast::channel(capacity);
        Self {
            fanout,
            per_variant: Arc::new(DashMap::new()),
            capacity,
            sequence: Arc::new(AtomicU64::new(0)),
            coordinator: None,
        }
    }

    /// Construct a bus that also shadow-publishes every event to
    /// `GlobalEventCoordinator` for cross-crate observers.
    pub fn with_coordinator(capacity: usize, coordinator: Arc<GlobalEventCoordinator>) -> Self {
        let (fanout, _) = broadcast::channel(capacity);
        Self {
            fanout,
            per_variant: Arc::new(DashMap::new()),
            capacity,
            sequence: Arc::new(AtomicU64::new(0)),
            coordinator: Some(coordinator),
        }
    }

    /// Subscribe to every event. Returns a unified stream — preserves the
    /// pre-Phase-0 API. Slow consumers of this subscription will lag the
    /// fan-out channel but do not affect per-variant subscribers.
    pub fn subscribe(&self) -> broadcast::Receiver<OrchestrationEventEnvelope> {
        self.fanout.subscribe()
    }

    /// Subscribe to a single variant only. Use this for hot consumers that
    /// only care about specific events — they get an independent broadcast
    /// channel and are not lagged by other variants.
    pub fn subscribe_kind(
        &self,
        kind: &'static str,
    ) -> broadcast::Receiver<OrchestrationEventEnvelope> {
        self.sender_for_kind(kind).subscribe()
    }

    fn sender_for_kind(
        &self,
        kind: &'static str,
    ) -> broadcast::Sender<OrchestrationEventEnvelope> {
        self.per_variant
            .entry(kind)
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(self.capacity);
                tx
            })
            .clone()
    }

    pub fn emit(&self, event: OrchestrationEvent) -> OrchestrationEventEnvelope {
        let envelope = OrchestrationEventEnvelope {
            event_id: EventId::new(),
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            occurred_at: Utc::now(),
            call_id: event.call_id(),
            correlation_id: None,
            event,
        };
        let _ = self.fanout.send(envelope.clone());

        let kind = envelope.event.variant_kind();
        if let Some(entry) = self.per_variant.get(kind) {
            let _ = entry.send(envelope.clone());
        }

        if let Some(coordinator) = &self.coordinator {
            let coord = coordinator.clone();
            let cross_crate = RvoipCrossCrateEvent::Orchestration(envelope.event.to_cross_crate());
            tokio::spawn(async move {
                let _ = coord.publish(Arc::new(cross_crate)).await;
            });
        }

        envelope
    }
}

impl Default for OrchestrationEventBus {
    fn default() -> Self {
        Self::new(1024)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn event_bus_assigns_sequence_and_call_correlation() {
        let bus = OrchestrationEventBus::new(8);
        let mut rx = bus.subscribe();
        let call_id = CallId::new();

        let first = bus.emit(OrchestrationEvent::CallQueued {
            call_id: call_id.clone(),
            queue_id: QueueId::from("support"),
        });
        let second = bus.emit(OrchestrationEvent::CallEnded {
            call_id: call_id.clone(),
            reason: "done".to_string(),
        });

        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, 2);
        assert_eq!(first.call_id, Some(call_id.clone()));
        assert_eq!(second.call_id, Some(call_id.clone()));

        assert_eq!(rx.recv().await.unwrap().sequence, 1);
        assert_eq!(rx.recv().await.unwrap().sequence, 2);
    }

    #[tokio::test]
    async fn subscribe_kind_only_receives_matching_variant() {
        let bus = OrchestrationEventBus::new(8);
        let mut queued_rx = bus.subscribe_kind("orchestration.call_queued");
        let mut ended_rx = bus.subscribe_kind("orchestration.call_ended");
        let call_id = CallId::new();

        bus.emit(OrchestrationEvent::CallQueued {
            call_id: call_id.clone(),
            queue_id: QueueId::from("support"),
        });
        bus.emit(OrchestrationEvent::CallEnded {
            call_id: call_id.clone(),
            reason: "done".to_string(),
        });

        let from_queued = queued_rx.recv().await.unwrap();
        assert!(matches!(
            from_queued.event,
            OrchestrationEvent::CallQueued { .. }
        ));
        assert!(queued_rx.try_recv().is_err());

        let from_ended = ended_rx.recv().await.unwrap();
        assert!(matches!(
            from_ended.event,
            OrchestrationEvent::CallEnded { .. }
        ));
        assert!(ended_rx.try_recv().is_err());
    }
}
