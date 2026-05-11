use crate::adapter::{EndReason, TransferTarget};
use crate::ids::{
    AiAttachmentId, BridgeId, ConnectionId, ConversationId, IdentityId, ListenerId, MessageId,
    ParticipantId, RecordingId, SessionId, StreamId, TenantId,
};
use crate::store::VconHandle;
use crate::stream::QualitySnapshot;
use chrono::{DateTime, Utc};
use rvoip_infra_common::events::cross_crate::{
    RvoipCoreCrossCrateEvent, RvoipCrossCrateEvent,
};

/// Normalized event vocabulary emitted by rvoip-core. Adapters produce
/// `AdapterEvent`s, which are translated into these by the orchestrator.
///
/// In step 4 these will be wired through `infra-common`'s
/// `RvoipCrossCrateEvent::Core(RvoipCoreCrossCrateEvent)` variant. Steps 1-2
/// keep them as a plain enum so the surface is reviewable in isolation.
///
/// All variants carry timestamps; routing-relevant identifiers
/// (`tenant_id`/`conversation_id`/`session_id`/`connection_id`/`correlation_id`)
/// are added by the cross-crate envelope wrapper at publish time, per
/// INTERFACE_DESIGN §5.
#[derive(Clone, Debug)]
pub enum Event {
    // --- Conversation lifecycle ---
    ConversationOpened {
        conversation_id: ConversationId,
        at: DateTime<Utc>,
    },
    ConversationClosed {
        conversation_id: ConversationId,
        at: DateTime<Utc>,
    },

    // --- Session lifecycle ---
    SessionStarted {
        session_id: SessionId,
        conversation_id: ConversationId,
        at: DateTime<Utc>,
    },
    SessionEnded {
        session_id: SessionId,
        at: DateTime<Utc>,
    },
    SessionFailed {
        session_id: SessionId,
        detail: String,
        at: DateTime<Utc>,
    },

    // --- Connection lifecycle ---
    ConnectionInbound {
        connection_id: ConnectionId,
        at: DateTime<Utc>,
    },
    ConnectionOutbound {
        connection_id: ConnectionId,
        at: DateTime<Utc>,
    },
    ConnectionConnected {
        connection_id: ConnectionId,
        at: DateTime<Utc>,
    },
    /// Early states (per INTERFACE_DESIGN §5).
    ConnectionProgress {
        connection_id: ConnectionId,
        kind: ConnectionProgressKind,
        at: DateTime<Utc>,
    },
    ConnectionEnded {
        connection_id: ConnectionId,
        reason: EndReason,
        at: DateTime<Utc>,
    },
    ConnectionFailed {
        connection_id: ConnectionId,
        detail: String,
        at: DateTime<Utc>,
    },

    // --- Bridge lifecycle ---
    ConnectionsBridged {
        bridge_id: BridgeId,
        a: ConnectionId,
        b: ConnectionId,
        at: DateTime<Utc>,
    },
    ConnectionsUnbridged {
        bridge_id: BridgeId,
        at: DateTime<Utc>,
    },

    // --- Transfer ---
    ConnectionTransferred {
        connection_id: ConnectionId,
        target: TransferTarget,
        at: DateTime<Utc>,
    },

    // --- Participant lifecycle (per-Session) ---
    ParticipantJoined {
        session_id: SessionId,
        participant_id: ParticipantId,
        at: DateTime<Utc>,
    },
    ParticipantLeft {
        session_id: SessionId,
        participant_id: ParticipantId,
        at: DateTime<Utc>,
    },

    // --- AI / listener attach ---
    AiAttached {
        connection_id: ConnectionId,
        attachment_id: AiAttachmentId,
        provider_ref: String,
        at: DateTime<Utc>,
    },
    AiDetached {
        attachment_id: AiAttachmentId,
        at: DateTime<Utc>,
    },
    ListenerAttached {
        listener_id: ListenerId,
        at: DateTime<Utc>,
    },
    ListenerDetached {
        listener_id: ListenerId,
        at: DateTime<Utc>,
    },

    // --- Messaging ---
    MessageReceived {
        message_id: MessageId,
        conversation_id: ConversationId,
        at: DateTime<Utc>,
    },
    MessageSent {
        message_id: MessageId,
        conversation_id: ConversationId,
        at: DateTime<Utc>,
    },
    MessageDelivered {
        message_id: MessageId,
        at: DateTime<Utc>,
    },
    MessageRead {
        message_id: MessageId,
        at: DateTime<Utc>,
    },

    // --- DTMF ---
    DtmfReceived {
        connection_id: ConnectionId,
        digits: String,
        at: DateTime<Utc>,
    },

    // --- Transcription / recording ---
    TranscriptTurn {
        stream_id: StreamId,
        speaker: Option<ParticipantId>,
        text: String,
        confidence: f32,
        is_final: bool,
        assigned_provider: Option<String>,
        at: DateTime<Utc>,
    },
    RecordingStarted {
        recording_id: RecordingId,
        at: DateTime<Utc>,
    },
    RecordingStopped {
        recording_id: RecordingId,
        at: DateTime<Utc>,
    },
    RecordingComplete {
        recording_id: RecordingId,
        sink: String,
        at: DateTime<Utc>,
    },

    // --- vCon ---
    VconReady {
        session_id: SessionId,
        handle: VconHandle,
        at: DateTime<Utc>,
    },
    VconRedacted {
        session_id: SessionId,
        old: VconHandle,
        new: VconHandle,
        at: DateTime<Utc>,
    },

    // --- Identity ---
    IdentityAssuranceChanged {
        connection_id: ConnectionId,
        identity_id: Option<IdentityId>,
        at: DateTime<Utc>,
    },

    // --- Registration (emitted by adapters that include a registrar) ---
    RegistrationChanged {
        aor: String,
        at: DateTime<Utc>,
    },
    RegistrationHeartbeat {
        aor: String,
        at: DateTime<Utc>,
    },

    // --- Observability ---
    CapacityReport {
        tenant_id: Option<TenantId>,
        active_connections: u64,
        active_bridges: u64,
        admission_in_use: u64,
        at: DateTime<Utc>,
    },
    UsageRecord {
        tenant_id: TenantId,
        kind: UsageKind,
        units: u64,
        at: DateTime<Utc>,
    },
    Anomaly {
        kind: AnomalyKind,
        connection_id: Option<ConnectionId>,
        detail: String,
        at: DateTime<Utc>,
    },
    MediaQuality {
        connection_id: ConnectionId,
        snapshot: QualitySnapshot,
        at: DateTime<Utc>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionProgressKind {
    Trying,
    Ringing,
    Busy,
    NoAnswer,
    AnsweringMachine,
    HumanAnswered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UsageKind {
    SessionSeconds,
    RecordingSeconds,
    TranscriptionSeconds,
    BridgedMinutes,
    MessagesSent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnomalyKind {
    QualityDrop,
    PossibleFraud,
    UnexpectedTeardown,
    AssuranceMismatch,
}

impl Event {
    /// Translate the rich in-process event to the primitive-payload wire form
    /// published through `infra-common::GlobalEventCoordinator`. The
    /// `RvoipCrossCrateEvent::Core` variant lives in infra-common per the
    /// CARVE_PLAN events.rs commitment.
    pub fn to_cross_crate(&self) -> RvoipCrossCrateEvent {
        use Event::*;
        let inner = match self {
            ConversationOpened { conversation_id, .. } => {
                RvoipCoreCrossCrateEvent::ConversationOpened {
                    conversation_id: conversation_id.to_string(),
                }
            }
            ConversationClosed { conversation_id, .. } => {
                RvoipCoreCrossCrateEvent::ConversationClosed {
                    conversation_id: conversation_id.to_string(),
                }
            }
            SessionStarted {
                session_id,
                conversation_id,
                ..
            } => RvoipCoreCrossCrateEvent::SessionStarted {
                session_id: session_id.to_string(),
                conversation_id: conversation_id.to_string(),
            },
            SessionEnded { session_id, .. } => RvoipCoreCrossCrateEvent::SessionEnded {
                session_id: session_id.to_string(),
            },
            SessionFailed {
                session_id, detail, ..
            } => RvoipCoreCrossCrateEvent::SessionFailed {
                session_id: session_id.to_string(),
                detail: detail.clone(),
            },
            ConnectionInbound { connection_id, .. } => {
                RvoipCoreCrossCrateEvent::ConnectionInbound {
                    connection_id: connection_id.to_string(),
                }
            }
            ConnectionOutbound { connection_id, .. } => {
                RvoipCoreCrossCrateEvent::ConnectionOutbound {
                    connection_id: connection_id.to_string(),
                }
            }
            ConnectionConnected { connection_id, .. } => {
                RvoipCoreCrossCrateEvent::ConnectionConnected {
                    connection_id: connection_id.to_string(),
                }
            }
            ConnectionProgress {
                connection_id,
                kind,
                ..
            } => RvoipCoreCrossCrateEvent::ConnectionProgress {
                connection_id: connection_id.to_string(),
                kind: format!("{:?}", kind),
            },
            ConnectionEnded {
                connection_id,
                reason,
                ..
            } => RvoipCoreCrossCrateEvent::ConnectionEnded {
                connection_id: connection_id.to_string(),
                reason: format!("{:?}", reason),
            },
            ConnectionFailed {
                connection_id,
                detail,
                ..
            } => RvoipCoreCrossCrateEvent::ConnectionFailed {
                connection_id: connection_id.to_string(),
                detail: detail.clone(),
            },
            ConnectionsBridged {
                bridge_id, a, b, ..
            } => RvoipCoreCrossCrateEvent::ConnectionsBridged {
                bridge_id: bridge_id.to_string(),
                a: a.to_string(),
                b: b.to_string(),
            },
            ConnectionsUnbridged { bridge_id, .. } => {
                RvoipCoreCrossCrateEvent::ConnectionsUnbridged {
                    bridge_id: bridge_id.to_string(),
                }
            }
            ConnectionTransferred {
                connection_id,
                target,
                ..
            } => RvoipCoreCrossCrateEvent::ConnectionTransferred {
                connection_id: connection_id.to_string(),
                target: format!("{:?}", target),
            },
            ParticipantJoined {
                session_id,
                participant_id,
                ..
            } => RvoipCoreCrossCrateEvent::ParticipantJoined {
                session_id: session_id.to_string(),
                participant_id: participant_id.to_string(),
            },
            ParticipantLeft {
                session_id,
                participant_id,
                ..
            } => RvoipCoreCrossCrateEvent::ParticipantLeft {
                session_id: session_id.to_string(),
                participant_id: participant_id.to_string(),
            },
            AiAttached {
                connection_id,
                attachment_id,
                provider_ref,
                ..
            } => RvoipCoreCrossCrateEvent::AiAttached {
                connection_id: connection_id.to_string(),
                attachment_id: attachment_id.to_string(),
                provider_ref: provider_ref.clone(),
            },
            AiDetached { attachment_id, .. } => RvoipCoreCrossCrateEvent::AiDetached {
                attachment_id: attachment_id.to_string(),
            },
            ListenerAttached { listener_id, .. } => {
                RvoipCoreCrossCrateEvent::ListenerAttached {
                    listener_id: listener_id.to_string(),
                }
            }
            ListenerDetached { listener_id, .. } => {
                RvoipCoreCrossCrateEvent::ListenerDetached {
                    listener_id: listener_id.to_string(),
                }
            }
            MessageReceived {
                message_id,
                conversation_id,
                ..
            } => RvoipCoreCrossCrateEvent::MessageReceived {
                message_id: message_id.to_string(),
                conversation_id: conversation_id.to_string(),
            },
            MessageSent {
                message_id,
                conversation_id,
                ..
            } => RvoipCoreCrossCrateEvent::MessageSent {
                message_id: message_id.to_string(),
                conversation_id: conversation_id.to_string(),
            },
            MessageDelivered { message_id, .. } => RvoipCoreCrossCrateEvent::MessageDelivered {
                message_id: message_id.to_string(),
            },
            MessageRead { message_id, .. } => RvoipCoreCrossCrateEvent::MessageRead {
                message_id: message_id.to_string(),
            },
            DtmfReceived {
                connection_id,
                digits,
                ..
            } => RvoipCoreCrossCrateEvent::DtmfReceived {
                connection_id: connection_id.to_string(),
                digits: digits.clone(),
            },
            TranscriptTurn {
                stream_id,
                speaker,
                text,
                confidence,
                is_final,
                assigned_provider,
                ..
            } => RvoipCoreCrossCrateEvent::TranscriptTurn {
                stream_id: stream_id.to_string(),
                speaker: speaker.as_ref().map(|p| p.to_string()),
                text: text.clone(),
                confidence: *confidence,
                is_final: *is_final,
                assigned_provider: assigned_provider.clone(),
            },
            RecordingStarted { recording_id, .. } => {
                RvoipCoreCrossCrateEvent::RecordingStarted {
                    recording_id: recording_id.to_string(),
                }
            }
            RecordingStopped { recording_id, .. } => {
                RvoipCoreCrossCrateEvent::RecordingStopped {
                    recording_id: recording_id.to_string(),
                }
            }
            RecordingComplete {
                recording_id, sink, ..
            } => RvoipCoreCrossCrateEvent::RecordingComplete {
                recording_id: recording_id.to_string(),
                sink: sink.clone(),
            },
            VconReady {
                session_id, handle, ..
            } => RvoipCoreCrossCrateEvent::VconReady {
                session_id: session_id.to_string(),
                handle_url: handle.url.clone(),
                content_hash: handle.content_hash.clone(),
            },
            VconRedacted {
                session_id, old, new, ..
            } => RvoipCoreCrossCrateEvent::VconRedacted {
                session_id: session_id.to_string(),
                old_url: old.url.clone(),
                new_url: new.url.clone(),
            },
            IdentityAssuranceChanged {
                connection_id,
                identity_id,
                ..
            } => RvoipCoreCrossCrateEvent::IdentityAssuranceChanged {
                connection_id: connection_id.to_string(),
                identity_id: identity_id.as_ref().map(|i| i.to_string()),
            },
            RegistrationChanged { aor, .. } => RvoipCoreCrossCrateEvent::RegistrationChanged {
                aor: aor.clone(),
            },
            RegistrationHeartbeat { aor, .. } => {
                RvoipCoreCrossCrateEvent::RegistrationHeartbeat { aor: aor.clone() }
            }
            CapacityReport {
                tenant_id,
                active_connections,
                active_bridges,
                admission_in_use,
                ..
            } => RvoipCoreCrossCrateEvent::CapacityReport {
                tenant_id: tenant_id.as_ref().map(|t| t.to_string()),
                active_connections: *active_connections,
                active_bridges: *active_bridges,
                admission_in_use: *admission_in_use,
            },
            UsageRecord {
                tenant_id,
                kind,
                units,
                ..
            } => RvoipCoreCrossCrateEvent::UsageRecord {
                tenant_id: tenant_id.to_string(),
                kind: format!("{:?}", kind),
                units: *units,
            },
            Anomaly {
                kind,
                connection_id,
                detail,
                ..
            } => RvoipCoreCrossCrateEvent::Anomaly {
                kind: format!("{:?}", kind),
                connection_id: connection_id.as_ref().map(|c| c.to_string()),
                detail: detail.clone(),
            },
            MediaQuality {
                connection_id,
                snapshot,
                ..
            } => RvoipCoreCrossCrateEvent::MediaQuality {
                connection_id: connection_id.to_string(),
                jitter_ms: snapshot.jitter_ms,
                packet_loss_pct: snapshot.packet_loss_pct,
                mos: snapshot.mos,
            },
        };
        RvoipCrossCrateEvent::Core(inner)
    }
}
