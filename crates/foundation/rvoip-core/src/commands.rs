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
use std::fmt;

/// Consumer-facing commands dispatched into the [`crate::Orchestrator`]. Each
/// carries `tenant_id` and `correlation_id` for tracing/quotas.
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

impl fmt::Debug for Command {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let variant = match self {
            Self::OpenConversation { .. } => "OpenConversation",
            Self::CloseConversation { .. } => "CloseConversation",
            Self::StartSession { .. } => "StartSession",
            Self::EndSession { .. } => "EndSession",
            Self::JoinSession { .. } => "JoinSession",
            Self::LeaveSession { .. } => "LeaveSession",
            Self::RouteInboundConnection { .. } => "RouteInboundConnection",
            Self::OriginateConnection { .. } => "OriginateConnection",
            Self::EndConnection { .. } => "EndConnection",
            Self::BridgeConnections { .. } => "BridgeConnections",
            Self::UnbridgeConnections { .. } => "UnbridgeConnections",
            Self::TransferConnection { .. } => "TransferConnection",
            Self::Hold { .. } => "Hold",
            Self::Resume { .. } => "Resume",
            Self::Mute { .. } => "Mute",
            Self::Unmute { .. } => "Unmute",
            Self::SendMessage { .. } => "SendMessage",
            Self::SendDtmf { .. } => "SendDtmf",
            Self::PlayAudio { .. } => "PlayAudio",
            Self::RenegotiateMedia { .. } => "RenegotiateMedia",
            Self::AttachAi { .. } => "AttachAi",
            Self::AttachListener { .. } => "AttachListener",
            Self::Detach { .. } => "Detach",
            Self::StartRecording { .. } => "StartRecording",
            Self::StopRecording { .. } => "StopRecording",
            Self::PauseRecording { .. } => "PauseRecording",
            Self::ResumeRecording { .. } => "ResumeRecording",
            Self::StartTranscription { .. } => "StartTranscription",
            Self::StopTranscription { .. } => "StopTranscription",
        };
        formatter
            .debug_struct("Command")
            .field("variant", &variant)
            .finish()
    }
}

#[derive(Clone)]
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

impl fmt::Debug for InboundAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Accept { .. } => "InboundAction::Accept",
            Self::Reject { .. } => "InboundAction::Reject",
            Self::BridgeTo { .. } => "InboundAction::BridgeTo",
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MuteDirection {
    Send,
    Receive,
    Both,
}

#[derive(Clone)]
pub enum AudioSource {
    Url(String),
    TtsRequest {
        provider_ref: String,
        text: String,
        voice: Option<String>,
    },
}

impl fmt::Debug for AudioSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Url(url) => formatter
                .debug_struct("AudioSource::Url")
                .field("value_present", &!url.is_empty())
                .finish(),
            Self::TtsRequest {
                provider_ref,
                text,
                voice,
            } => formatter
                .debug_struct("AudioSource::TtsRequest")
                .field("provider_present", &!provider_ref.is_empty())
                .field("text_bytes", &text.len())
                .field("voice_present", &voice.is_some())
                .finish(),
        }
    }
}

#[derive(Clone)]
pub enum ListenerTarget {
    Connection(ConnectionId),
    Session(SessionId),
}

impl fmt::Debug for ListenerTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Connection(_) => "ListenerTarget::Connection",
            Self::Session(_) => "ListenerTarget::Session",
        })
    }
}

#[derive(Clone)]
pub enum ListenerSink {
    File { path: String },
    Url(String),
    Channel,
}

impl fmt::Debug for ListenerSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::File { .. } => "ListenerSink::File",
            Self::Url(_) => "ListenerSink::Url",
            Self::Channel => "ListenerSink::Channel",
        })
    }
}

#[derive(Clone)]
pub enum AttachmentRef {
    Ai(AiAttachmentId),
    Listener(ListenerId),
    Recording(RecordingId),
}

impl fmt::Debug for AttachmentRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Ai(_) => "AttachmentRef::Ai",
            Self::Listener(_) => "AttachmentRef::Listener",
            Self::Recording(_) => "AttachmentRef::Recording",
        })
    }
}

#[derive(Clone)]
pub enum RecordingTarget {
    Connection(ConnectionId),
    Session(SessionId),
}

impl fmt::Debug for RecordingTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Connection(_) => "RecordingTarget::Connection",
            Self::Session(_) => "RecordingTarget::Session",
        })
    }
}

#[derive(Clone)]
pub enum RecordingSink {
    File { path: String },
    Url(String),
}

impl fmt::Debug for RecordingSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::File { .. } => "RecordingSink::File",
            Self::Url(_) => "RecordingSink::Url",
        })
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn command_and_nested_payload_debug_never_render_caller_values() {
        let secret = "credential-canary-value";
        let connection_id = ConnectionId::new();
        let commands = [
            Command::SendDtmf {
                connection_id: connection_id.clone(),
                digits: secret.into(),
                duration_ms: 100,
                correlation_id: secret.into(),
            },
            Command::PlayAudio {
                connection_id,
                source: AudioSource::TtsRequest {
                    provider_ref: secret.into(),
                    text: secret.into(),
                    voice: Some(secret.into()),
                },
                correlation_id: secret.into(),
            },
        ];

        for command in commands {
            let debug = format!("{command:?}");
            assert!(!debug.contains(secret));
            assert!(debug.contains("variant"));
        }

        for debug in [
            format!("{:?}", AudioSource::Url(secret.into())),
            format!(
                "{:?}",
                ListenerSink::File {
                    path: secret.into()
                }
            ),
            format!("{:?}", ListenerSink::Url(secret.into())),
            format!(
                "{:?}",
                RecordingSink::File {
                    path: secret.into()
                }
            ),
            format!("{:?}", RecordingSink::Url(secret.into())),
        ] {
            assert!(!debug.contains(secret));
        }
    }
}
