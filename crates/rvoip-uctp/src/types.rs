//! `MessageType` — the UCTP envelope type catalog.
//!
//! Wire format is the dotted lowercase string from CONVERSATION_PROTOCOL.md
//! §6. Unknown types decode to [`MessageType::Unknown`] so forward-compat is
//! preserved (CONVERSATION_PROTOCOL.md §3.2: "unknown envelope types MUST
//! be tolerated").
//!
//! Note on serde: `MessageType` hand-implements `Serialize`/`Deserialize`
//! rather than using `#[serde(other)] Unknown`. The latter requires the
//! `Unknown` variant to be a unit variant, which would discard the wire
//! string for unknown types — losing the diagnostic value the spec §3.2
//! forward-compat rule depends on. Hand-rolled is more capable and only
//! a few lines longer.

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// All UCTP envelope types defined by the v0 wire spec.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MessageType {
    // --- Auth (§5) ---
    AuthHello,
    AuthChallenge,
    AuthResponse,
    AuthSession,
    AuthKeepalive,
    AuthBye,
    /// Mid-session bearer refresh (plan D4). Peer ships a freshly-issued
    /// token before the prior one expires; coordinator validates and
    /// updates [`PeerAuthState`]. On success, server replies with a
    /// fresh `auth.session` envelope carrying the new expires_at. A
    /// failed refresh does NOT revoke the existing session — the peer
    /// can retry until its current token actually expires.
    AuthRefresh,

    // --- Conversation (§7.1) ---
    ConversationCreate,
    ConversationOpened,
    ConversationClosed,
    ConversationList,

    // --- Session (§7.2–§7.3) ---
    SessionInvite,
    SessionAccept,
    SessionReject,
    SessionCancel,
    SessionEnd,
    SessionStarted,
    SessionEnded,
    SessionUpdate,
    SessionParticipantJoined,
    SessionParticipantLeft,

    // --- Connection / Stream (§7.4, §7.7) ---
    ConnectionOffer,
    ConnectionAnswer,
    ConnectionReady,
    ConnectionUpdate,
    ConnectionEnd,
    StreamOpened,
    StreamClosed,
    StreamSubscribe,
    StreamUnsubscribe,
    StreamActiveSpeaker,

    // --- Message (§9) ---
    MessageSend,
    MessageDelivered,
    MessageRead,
    MessageHistory,

    // --- Capabilities (§8) ---
    CapabilityAdvertise,

    // --- DTMF (§7.5) ---
    DtmfSend,
    DtmfReceived,

    // --- vCon (§7.6) ---
    RecordingVconReady,
    RecordingVconFetch,
    RecordingVconFetched,

    // --- Quality (§10.3) ---
    ConnectionQuality,

    // --- Identity (§5.6, §5.8) ---
    IdentityAssuranceChanged,
    IdentityStepUpRequest,
    IdentityStepUpResponse,

    // --- Errors / control (§11) ---
    Error,
    Ack,

    /// Any envelope type the local v0 implementation does not recognize.
    /// Carries the raw wire string so the receiver can log / route by name.
    Unknown(String),
}

impl MessageType {
    /// Returns the wire-format dotted string for this variant.
    pub fn as_wire_str(&self) -> &str {
        match self {
            MessageType::AuthHello => "auth.hello",
            MessageType::AuthChallenge => "auth.challenge",
            MessageType::AuthResponse => "auth.response",
            MessageType::AuthSession => "auth.session",
            MessageType::AuthKeepalive => "auth.keepalive",
            MessageType::AuthBye => "auth.bye",
            MessageType::AuthRefresh => "auth.refresh",
            MessageType::ConversationCreate => "conversation.create",
            MessageType::ConversationOpened => "conversation.opened",
            MessageType::ConversationClosed => "conversation.closed",
            MessageType::ConversationList => "conversation.list",
            MessageType::SessionInvite => "session.invite",
            MessageType::SessionAccept => "session.accept",
            MessageType::SessionReject => "session.reject",
            MessageType::SessionCancel => "session.cancel",
            MessageType::SessionEnd => "session.end",
            MessageType::SessionStarted => "session.started",
            MessageType::SessionEnded => "session.ended",
            MessageType::SessionUpdate => "session.update",
            MessageType::SessionParticipantJoined => "session.participant.joined",
            MessageType::SessionParticipantLeft => "session.participant.left",
            MessageType::ConnectionOffer => "connection.offer",
            MessageType::ConnectionAnswer => "connection.answer",
            MessageType::ConnectionReady => "connection.ready",
            MessageType::ConnectionUpdate => "connection.update",
            MessageType::ConnectionEnd => "connection.end",
            MessageType::StreamOpened => "stream.opened",
            MessageType::StreamClosed => "stream.closed",
            MessageType::StreamSubscribe => "stream.subscribe",
            MessageType::StreamUnsubscribe => "stream.unsubscribe",
            MessageType::StreamActiveSpeaker => "stream.active-speaker",
            MessageType::MessageSend => "message.send",
            MessageType::MessageDelivered => "message.delivered",
            MessageType::MessageRead => "message.read",
            MessageType::MessageHistory => "message.history",
            MessageType::CapabilityAdvertise => "capability.advertise",
            MessageType::DtmfSend => "dtmf.send",
            MessageType::DtmfReceived => "dtmf.received",
            MessageType::RecordingVconReady => "recording.vcon-ready",
            MessageType::RecordingVconFetch => "recording.vcon-fetch",
            MessageType::RecordingVconFetched => "recording.vcon-fetched",
            MessageType::ConnectionQuality => "connection.quality",
            MessageType::IdentityAssuranceChanged => "identity.assurance-changed",
            MessageType::IdentityStepUpRequest => "identity.step-up-request",
            MessageType::IdentityStepUpResponse => "identity.step-up-response",
            MessageType::Error => "error",
            MessageType::Ack => "ack",
            MessageType::Unknown(s) => s,
        }
    }

    /// Parse a wire-format dotted string into a variant. Unrecognized
    /// strings produce [`MessageType::Unknown`] — never fails.
    pub fn from_wire_str(s: &str) -> Self {
        match s {
            "auth.hello" => MessageType::AuthHello,
            "auth.challenge" => MessageType::AuthChallenge,
            "auth.response" => MessageType::AuthResponse,
            "auth.session" => MessageType::AuthSession,
            "auth.keepalive" => MessageType::AuthKeepalive,
            "auth.bye" => MessageType::AuthBye,
            "auth.refresh" => MessageType::AuthRefresh,
            "conversation.create" => MessageType::ConversationCreate,
            "conversation.opened" => MessageType::ConversationOpened,
            "conversation.closed" => MessageType::ConversationClosed,
            "conversation.list" => MessageType::ConversationList,
            "session.invite" => MessageType::SessionInvite,
            "session.accept" => MessageType::SessionAccept,
            "session.reject" => MessageType::SessionReject,
            "session.cancel" => MessageType::SessionCancel,
            "session.end" => MessageType::SessionEnd,
            "session.started" => MessageType::SessionStarted,
            "session.ended" => MessageType::SessionEnded,
            "session.update" => MessageType::SessionUpdate,
            "session.participant.joined" => MessageType::SessionParticipantJoined,
            "session.participant.left" => MessageType::SessionParticipantLeft,
            "connection.offer" => MessageType::ConnectionOffer,
            "connection.answer" => MessageType::ConnectionAnswer,
            "connection.ready" => MessageType::ConnectionReady,
            "connection.update" => MessageType::ConnectionUpdate,
            "connection.end" => MessageType::ConnectionEnd,
            "stream.opened" => MessageType::StreamOpened,
            "stream.closed" => MessageType::StreamClosed,
            "stream.subscribe" => MessageType::StreamSubscribe,
            "stream.unsubscribe" => MessageType::StreamUnsubscribe,
            "stream.active-speaker" => MessageType::StreamActiveSpeaker,
            "message.send" => MessageType::MessageSend,
            "message.delivered" => MessageType::MessageDelivered,
            "message.read" => MessageType::MessageRead,
            "message.history" => MessageType::MessageHistory,
            "capability.advertise" => MessageType::CapabilityAdvertise,
            "dtmf.send" => MessageType::DtmfSend,
            "dtmf.received" => MessageType::DtmfReceived,
            "recording.vcon-ready" => MessageType::RecordingVconReady,
            "recording.vcon-fetch" => MessageType::RecordingVconFetch,
            "recording.vcon-fetched" => MessageType::RecordingVconFetched,
            "connection.quality" => MessageType::ConnectionQuality,
            "identity.assurance-changed" => MessageType::IdentityAssuranceChanged,
            "identity.step-up-request" => MessageType::IdentityStepUpRequest,
            "identity.step-up-response" => MessageType::IdentityStepUpResponse,
            "error" => MessageType::Error,
            "ack" => MessageType::Ack,
            other => MessageType::Unknown(other.to_string()),
        }
    }
}

impl fmt::Display for MessageType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

impl Serialize for MessageType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_wire_str())
    }
}

impl<'de> Deserialize<'de> for MessageType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = MessageType;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a UCTP envelope type string")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<MessageType, E> {
                Ok(MessageType::from_wire_str(v))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<MessageType, E> {
                Ok(MessageType::from_wire_str(&v))
            }
        }
        deserializer.deserialize_str(V)
    }
}
