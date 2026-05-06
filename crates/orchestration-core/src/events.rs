use crate::ids::*;
use crate::types::*;
use crate::voice_ai::TranscriptEvent;
use chrono::{DateTime, Utc};
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
}

#[derive(Clone)]
pub struct OrchestrationEventBus {
    tx: broadcast::Sender<OrchestrationEventEnvelope>,
    sequence: Arc<AtomicU64>,
}

impl OrchestrationEventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self {
            tx,
            sequence: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OrchestrationEventEnvelope> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: OrchestrationEvent) -> OrchestrationEventEnvelope {
        let envelope = OrchestrationEventEnvelope {
            event_id: EventId::new(),
            sequence: self.sequence.fetch_add(1, Ordering::SeqCst) + 1,
            occurred_at: Utc::now(),
            call_id: event.call_id(),
            correlation_id: None,
            event,
        };
        let _ = self.tx.send(envelope.clone());
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
}
