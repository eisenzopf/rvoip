use crate::adapter::{EndReason, TransferTarget};
use crate::identity::{AuthenticatedPrincipal, IdentityAssurance};
use crate::ids::{
    AiAttachmentId, BridgeId, ConnectionId, ConversationId, IdentityId, ListenerId, MessageId,
    ParticipantId, RecordingId, SessionId, StreamId, TenantId,
};
use crate::store::VconHandle;
use crate::stream::QualitySnapshot;
use crate::vcon::VconRef;
use crate::DataMessage;
use chrono::{DateTime, Utc};
use rvoip_infra_common::events::cross_crate::{RvoipCoreCrossCrateEvent, RvoipCrossCrateEvent};
use std::fmt;

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
#[derive(Clone)]
#[non_exhaustive]
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
        /// P9 — quality / accounting payload aggregated over the
        /// Session's lifetime. `None` when no quality samples landed
        /// (e.g. message-only Session).
        report: Option<SessionQualityReport>,
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
    /// A peer completed the UCTP auth handshake on this Connection.
    /// Fires immediately after `ConnectionInbound` for UCTP-family
    /// substrates; absent for substrates that don't model peer-level
    /// auth (SIP, WebRTC). The `participant_id` is the peer's claimed
    /// identifier from `auth.session`; `identity_id` is the
    /// server-issued binding. Plan §7 G1 / A3.
    ConnectionAuthenticated {
        connection_id: ConnectionId,
        identity_id: String,
        participant_id: String,
        assurance: IdentityAssurance,
        at: DateTime<Utc>,
    },
    /// Complete principal retained alongside the legacy authentication event.
    /// Authorization decisions should compare `principal.ownership_key()`.
    ConnectionPrincipalAuthenticated {
        connection_id: ConnectionId,
        participant_id: String,
        principal: AuthenticatedPrincipal,
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
    DataMessageReceived {
        connection_id: ConnectionId,
        message: DataMessage,
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
        /// Opaque reference to the persisted vCon document.
        ///
        /// v0 always emits `None`; the `rvoip-vcon` crate landing in v0.x
        /// populates `Some(VconRef::Local { uuid })` at session.ended. See
        /// UCTP plan §2.4 / §7 (vCon emission row).
        vcon_ref: Option<VconRef>,
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

    /// P12.6 — emitted after `Orchestrator::request_step_up` has
    /// pushed an `identity.step-up-request` to the peer's adapter. The
    /// consumer can use this as a positive signal that the request
    /// reached the wire. Carries the requested assurance for context.
    IdentityStepUpRequested {
        connection_id: ConnectionId,
        required: crate::capability::IdentityAssuranceRequirement,
        at: DateTime<Utc>,
    },

    /// P12.6 — peer sent an `identity.step-up-response`. Consumer
    /// resolves the `(method, credential)` pair to a
    /// [`crate::identity::Credential`] and calls
    /// [`crate::Orchestrator::complete_step_up`] to finish the
    /// round-trip (which emits `IdentityAssuranceChanged` on success).
    IdentityStepUpResponseReceived {
        connection_id: ConnectionId,
        method: String,
        credential: String,
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

    /// P5 — fired when ASR detects speech while a TTS playback is
    /// in flight. The orchestrator's AI loop cancels the current
    /// playback before firing this; downstream consumers can use it
    /// for analytics (barge-in count is one of the AI-quality
    /// signals PRD §11 calls out).
    BargeInDetected {
        connection_id: ConnectionId,
        ai_attachment_id: AiAttachmentId,
        at: DateTime<Utc>,
    },

    /// P8 — active-speaker advisory per CONVERSATION_PROTOCOL §6
    /// `stream.active-speaker`. Emitted (optionally) by the UCTP
    /// coordinator when audio-level extension data identifies a new
    /// dominant speaker. Pure advisory — subscribers may use it to
    /// drive UI focus; no media routing decisions are made off it.
    ActiveSpeakerChanged {
        session_id: SessionId,
        connection_id: ConnectionId,
        audio_level_dbov: i8,
        at: DateTime<Utc>,
    },
}

impl fmt::Debug for Event {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConversationOpened { .. } => formatter.write_str("ConversationOpened"),
            Self::ConversationClosed { .. } => formatter.write_str("ConversationClosed"),
            Self::SessionStarted { .. } => formatter.write_str("SessionStarted"),
            Self::SessionEnded { report, .. } => formatter
                .debug_struct("SessionEnded")
                .field("quality_report_present", &report.is_some())
                .finish(),
            Self::SessionFailed { detail, .. } => formatter
                .debug_struct("SessionFailed")
                .field("detail_present", &!detail.is_empty())
                .field("detail_bytes", &detail.len())
                .finish(),
            Self::ConnectionInbound { .. } => formatter.write_str("ConnectionInbound"),
            Self::ConnectionOutbound { .. } => formatter.write_str("ConnectionOutbound"),
            Self::ConnectionConnected { .. } => formatter.write_str("ConnectionConnected"),
            Self::ConnectionAuthenticated { .. } => formatter.write_str("ConnectionAuthenticated"),
            Self::ConnectionPrincipalAuthenticated { .. } => {
                formatter.write_str("ConnectionPrincipalAuthenticated")
            }
            Self::ConnectionProgress { kind, .. } => formatter
                .debug_struct("ConnectionProgress")
                .field("kind", kind)
                .finish(),
            Self::ConnectionEnded { .. } => formatter.write_str("ConnectionEnded"),
            Self::ConnectionFailed { detail, .. } => formatter
                .debug_struct("ConnectionFailed")
                .field("detail_present", &!detail.is_empty())
                .field("detail_bytes", &detail.len())
                .finish(),
            Self::ConnectionsBridged { .. } => formatter.write_str("ConnectionsBridged"),
            Self::ConnectionsUnbridged { .. } => formatter.write_str("ConnectionsUnbridged"),
            Self::ConnectionTransferred { .. } => formatter.write_str("ConnectionTransferred"),
            Self::ParticipantJoined { .. } => formatter.write_str("ParticipantJoined"),
            Self::ParticipantLeft { .. } => formatter.write_str("ParticipantLeft"),
            Self::AiAttached { provider_ref, .. } => formatter
                .debug_struct("AiAttached")
                .field("provider_ref_present", &!provider_ref.is_empty())
                .field("provider_ref_bytes", &provider_ref.len())
                .finish(),
            Self::AiDetached { .. } => formatter.write_str("AiDetached"),
            Self::ListenerAttached { .. } => formatter.write_str("ListenerAttached"),
            Self::ListenerDetached { .. } => formatter.write_str("ListenerDetached"),
            Self::MessageReceived { .. } => formatter.write_str("MessageReceived"),
            Self::DataMessageReceived { message, .. } => formatter
                .debug_struct("DataMessageReceived")
                .field("body_bytes", &message.bytes.len())
                .finish(),
            Self::MessageSent { .. } => formatter.write_str("MessageSent"),
            Self::MessageDelivered { .. } => formatter.write_str("MessageDelivered"),
            Self::MessageRead { .. } => formatter.write_str("MessageRead"),
            Self::DtmfReceived { digits, .. } => formatter
                .debug_struct("DtmfReceived")
                .field("digit_count", &digits.chars().count())
                .finish(),
            Self::TranscriptTurn {
                text,
                speaker,
                confidence,
                is_final,
                assigned_provider,
                ..
            } => formatter
                .debug_struct("TranscriptTurn")
                .field("speaker_present", &speaker.is_some())
                .field("text_bytes", &text.len())
                .field("confidence", confidence)
                .field("is_final", is_final)
                .field("assigned_provider_present", &assigned_provider.is_some())
                .finish(),
            Self::RecordingStarted { .. } => formatter.write_str("RecordingStarted"),
            Self::RecordingStopped { .. } => formatter.write_str("RecordingStopped"),
            Self::RecordingComplete { sink, vcon_ref, .. } => formatter
                .debug_struct("RecordingComplete")
                .field("sink_present", &!sink.is_empty())
                .field("sink_bytes", &sink.len())
                .field("vcon_ref_present", &vcon_ref.is_some())
                .finish(),
            Self::VconReady { .. } => formatter.write_str("VconReady"),
            Self::VconRedacted { .. } => formatter.write_str("VconRedacted"),
            Self::IdentityAssuranceChanged { identity_id, .. } => formatter
                .debug_struct("IdentityAssuranceChanged")
                .field("identity_present", &identity_id.is_some())
                .finish(),
            Self::IdentityStepUpRequested { .. } => formatter.write_str("IdentityStepUpRequested"),
            Self::IdentityStepUpResponseReceived {
                method, credential, ..
            } => formatter
                .debug_struct("IdentityStepUpResponseReceived")
                .field("method_present", &!method.is_empty())
                .field("method_bytes", &method.len())
                .field("credential_present", &!credential.is_empty())
                .field("credential_bytes", &credential.len())
                .finish(),
            Self::RegistrationChanged { .. } => formatter.write_str("RegistrationChanged"),
            Self::RegistrationHeartbeat { .. } => formatter.write_str("RegistrationHeartbeat"),
            Self::CapacityReport {
                tenant_id,
                active_connections,
                active_bridges,
                admission_in_use,
                ..
            } => formatter
                .debug_struct("CapacityReport")
                .field("tenant_present", &tenant_id.is_some())
                .field("active_connections", active_connections)
                .field("active_bridges", active_bridges)
                .field("admission_in_use", admission_in_use)
                .finish(),
            Self::UsageRecord { kind, units, .. } => formatter
                .debug_struct("UsageRecord")
                .field("kind", kind)
                .field("units", units)
                .finish(),
            Self::Anomaly {
                kind,
                connection_id,
                detail,
                ..
            } => formatter
                .debug_struct("Anomaly")
                .field("kind", kind)
                .field("connection_present", &connection_id.is_some())
                .field("detail_present", &!detail.is_empty())
                .field("detail_bytes", &detail.len())
                .finish(),
            Self::MediaQuality { .. } => formatter.write_str("MediaQuality"),
            Self::BargeInDetected { .. } => formatter.write_str("BargeInDetected"),
            Self::ActiveSpeakerChanged {
                audio_level_dbov, ..
            } => formatter
                .debug_struct("ActiveSpeakerChanged")
                .field("audio_level_dbov", audio_level_dbov)
                .finish(),
        }
    }
}

/// P9 — per-Session quality + accounting report carried on
/// `Event::SessionEnded`. Mirrors PRD §10.2.
#[derive(Clone, Debug, Default)]
pub struct SessionQualityReport {
    pub mos: Option<f32>,
    pub packet_loss_pct: f32,
    pub jitter_ms: f32,
    pub rtt_ms: Option<f32>,
    pub codec: Option<String>,
    pub bitrate_bps: Option<u32>,
    pub talk_pct: Option<f32>,
    pub silence_pct: Option<f32>,
    pub pdd_ms: Option<u32>,
    pub ring_time_ms: Option<u32>,
    pub setup_time_ms: Option<u32>,
    pub hangup_reason: Option<String>,
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
            ConversationOpened {
                conversation_id, ..
            } => RvoipCoreCrossCrateEvent::ConversationOpened {
                conversation_id: conversation_id.to_string(),
            },
            ConversationClosed {
                conversation_id, ..
            } => RvoipCoreCrossCrateEvent::ConversationClosed {
                conversation_id: conversation_id.to_string(),
            },
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
            ConnectionAuthenticated { connection_id, .. } => {
                // Authentication context is tenant-sensitive. The global
                // cross-crate bus is not tenant-authorized, so it receives
                // only a lifecycle marker; rich identity stays on the typed
                // in-process event and connection route.
                RvoipCoreCrossCrateEvent::IdentityAssuranceChanged {
                    connection_id: connection_id.to_string(),
                    identity_id: None,
                }
            }
            ConnectionPrincipalAuthenticated { connection_id, .. } => {
                RvoipCoreCrossCrateEvent::IdentityAssuranceChanged {
                    connection_id: connection_id.to_string(),
                    identity_id: None,
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
            ListenerAttached { listener_id, .. } => RvoipCoreCrossCrateEvent::ListenerAttached {
                listener_id: listener_id.to_string(),
            },
            ListenerDetached { listener_id, .. } => RvoipCoreCrossCrateEvent::ListenerDetached {
                listener_id: listener_id.to_string(),
            },
            MessageReceived {
                message_id,
                conversation_id,
                ..
            } => RvoipCoreCrossCrateEvent::MessageReceived {
                message_id: message_id.to_string(),
                conversation_id: conversation_id.to_string(),
            },
            DataMessageReceived {
                connection_id,
                message,
                ..
            } => RvoipCoreCrossCrateEvent::DataMessageReceived {
                connection_id: connection_id.to_string(),
                body_size: message.bytes.len(),
                reliability: format!("{:?}", message.reliability),
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
            RecordingStarted { recording_id, .. } => RvoipCoreCrossCrateEvent::RecordingStarted {
                recording_id: recording_id.to_string(),
            },
            RecordingStopped { recording_id, .. } => RvoipCoreCrossCrateEvent::RecordingStopped {
                recording_id: recording_id.to_string(),
            },
            RecordingComplete {
                recording_id,
                sink,
                vcon_ref: _,
                ..
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
                session_id,
                old,
                new,
                ..
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
            RegistrationChanged { aor, .. } => {
                RvoipCoreCrossCrateEvent::RegistrationChanged { aor: aor.clone() }
            }
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
            BargeInDetected { connection_id, .. } => {
                // No dedicated cross-crate variant yet; surface as
                // IdentityAssuranceChanged with None identity so
                // downstream services notice an event on the bus.
                RvoipCoreCrossCrateEvent::IdentityAssuranceChanged {
                    connection_id: connection_id.to_string(),
                    identity_id: None,
                }
            }
            ActiveSpeakerChanged { connection_id, .. } => {
                // No dedicated wire variant yet — surface as MediaQuality
                // with zero loss so downstream crates that don't know
                // about ActiveSpeaker still see *something* on the bus.
                RvoipCoreCrossCrateEvent::MediaQuality {
                    connection_id: connection_id.to_string(),
                    jitter_ms: 0.0,
                    packet_loss_pct: 0.0,
                    mos: None,
                }
            }
            IdentityStepUpRequested { connection_id, .. }
            | IdentityStepUpResponseReceived { connection_id, .. } => {
                // P12.6 — no dedicated cross-crate variant yet; surface
                // as IdentityAssuranceChanged with None identity_id so
                // downstream services see the round-trip on the bus.
                // The actual assurance change still emits a separate
                // IdentityAssuranceChanged event when the consumer calls
                // `complete_step_up`.
                RvoipCoreCrossCrateEvent::IdentityAssuranceChanged {
                    connection_id: connection_id.to_string(),
                    identity_id: None,
                }
            }
        };
        RvoipCrossCrateEvent::Core(inner)
    }
}

#[cfg(test)]
mod credential_diagnostic_tests {
    use super::*;

    #[test]
    fn enclosing_step_up_event_redacts_credential_and_keeps_live_value() {
        const CANARY: &str = "core-event-credential-canary\r\nAuthorization: exposed";
        let event = Event::IdentityStepUpResponseReceived {
            connection_id: ConnectionId::new(),
            method: "bearer".into(),
            credential: CANARY.into(),
            at: Utc::now(),
        };
        let rendered = format!("{event:?}");
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
        match event {
            Event::IdentityStepUpResponseReceived { credential, .. } => {
                assert_eq!(credential, CANARY)
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
