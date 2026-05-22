use crate::adapter::{EndReason, OriginateRequest, RejectReason, TransferTarget};
use crate::capability::CapabilityDescriptor;
use crate::conversation::ConversationPolicy;
use crate::ids::{
    AiAttachmentId, BridgeId, ConnectionId, ConversationId, ListenerId, ParticipantId, RecordingId,
    SessionId, TenantId,
};
use crate::message::Message;
use crate::participant::{ParticipantKind, ParticipantRole};
use crate::session::SessionMedium;
use std::collections::HashMap;

/// Consumer-facing commands dispatched into the [`crate::Orchestrator`]. Each
/// carries `tenant_id` and `correlation_id` for tracing/quotas.
#[derive(Debug)]
pub enum Command {
    // --- Conversation lifecycle ---
    OpenConversation {
        tenant_id: TenantId,
        policy: ConversationPolicy,
        metadata: HashMap<String, String>,
        correlation_id: String,
    },
    CloseConversation {
        conversation_id: ConversationId,
        force: bool,
        correlation_id: String,
    },

    // --- Session lifecycle ---
    StartSession {
        conversation_id: ConversationId,
        medium: SessionMedium,
        invitees: Vec<ParticipantId>,
        correlation_id: String,
    },
    EndSession {
        session_id: SessionId,
        reason: EndReason,
        correlation_id: String,
    },
    JoinSession {
        session_id: SessionId,
        participant_id: ParticipantId,
        kind: ParticipantKind,
        role: ParticipantRole,
        correlation_id: String,
    },
    LeaveSession {
        session_id: SessionId,
        participant_id: ParticipantId,
        correlation_id: String,
    },

    // --- Connection lifecycle ---
    RouteInboundConnection {
        connection_id: ConnectionId,
        action: InboundAction,
        correlation_id: String,
    },
    OriginateConnection {
        request: OriginateRequest,
        correlation_id: String,
    },
    EndConnection {
        connection_id: ConnectionId,
        reason: EndReason,
        correlation_id: String,
    },

    // --- Bridging ---
    BridgeConnections {
        a: ConnectionId,
        b: ConnectionId,
        correlation_id: String,
    },
    UnbridgeConnections {
        bridge_id: BridgeId,
        correlation_id: String,
    },

    // --- Transfer ---
    TransferConnection {
        connection_id: ConnectionId,
        target: TransferTarget,
        correlation_id: String,
    },

    // --- Per-Connection media control ---
    Hold {
        connection_id: ConnectionId,
        correlation_id: String,
    },
    Resume {
        connection_id: ConnectionId,
        correlation_id: String,
    },
    Mute {
        connection_id: ConnectionId,
        direction: MuteDirection,
        correlation_id: String,
    },
    Unmute {
        connection_id: ConnectionId,
        direction: MuteDirection,
        correlation_id: String,
    },
    SendMessage {
        connection_id: ConnectionId,
        message: Message,
        correlation_id: String,
    },
    SendDtmf {
        connection_id: ConnectionId,
        digits: String,
        duration_ms: u32,
        correlation_id: String,
    },
    PlayAudio {
        connection_id: ConnectionId,
        source: AudioSource,
        correlation_id: String,
    },
    RenegotiateMedia {
        connection_id: ConnectionId,
        capabilities: CapabilityDescriptor,
        correlation_id: String,
    },

    // --- AI / listener attach (AI runtime + recorder taps live in
    //     consumer / rvoip-harness; rvoip-core just carries the command) ---
    AttachAi {
        connection_id: ConnectionId,
        provider_ref: String,
        config: HashMap<String, String>,
        correlation_id: String,
    },
    AttachListener {
        target: ListenerTarget,
        sink: ListenerSink,
        correlation_id: String,
    },
    Detach {
        attachment: AttachmentRef,
        correlation_id: String,
    },

    // --- Recording / transcription ---
    StartRecording {
        target: RecordingTarget,
        sink: RecordingSink,
        correlation_id: String,
    },
    StopRecording {
        recording_id: RecordingId,
        correlation_id: String,
    },
    PauseRecording {
        recording_id: RecordingId,
        correlation_id: String,
    },
    ResumeRecording {
        recording_id: RecordingId,
        correlation_id: String,
    },
    StartTranscription {
        target: RecordingTarget,
        provider_ref: String,
        correlation_id: String,
    },
    StopTranscription {
        target: RecordingTarget,
        correlation_id: String,
    },
}

#[derive(Clone, Debug)]
pub enum InboundAction {
    Accept {
        session_id: SessionId,
        participant_id: ParticipantId,
    },
    Reject {
        reason: RejectReason,
    },
    /// Originate an outbound leg and bridge to the inbound (gateway pattern).
    BridgeTo {
        session_id: SessionId,
        outbound: OriginateRequest,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MuteDirection {
    Send,
    Receive,
    Both,
}

#[derive(Clone, Debug)]
pub enum AudioSource {
    Url(String),
    TtsRequest {
        provider_ref: String,
        text: String,
        voice: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum ListenerTarget {
    Connection(ConnectionId),
    Session(SessionId),
}

#[derive(Clone, Debug)]
pub enum ListenerSink {
    File { path: String },
    Url(String),
    Channel,
}

#[derive(Clone, Debug)]
pub enum AttachmentRef {
    Ai(AiAttachmentId),
    Listener(ListenerId),
    Recording(RecordingId),
}

#[derive(Clone, Debug)]
pub enum RecordingTarget {
    Connection(ConnectionId),
    Session(SessionId),
}

#[derive(Clone, Debug)]
pub enum RecordingSink {
    File { path: String },
    Url(String),
}
