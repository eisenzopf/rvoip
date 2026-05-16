//! Cross-Crate Event Definitions
//!
//! Defines all events that cross crate boundaries, enabling event-driven
//! communication between session-core, dialog-core, media-core, etc.

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::events::types::{Event, EventPriority};
use crate::planes::routing::RoutableEvent;
use crate::planes::PlaneType;
use std::any::Any;

/// Event type identifier for cross-crate events
pub type EventTypeId = &'static str;

/// All cross-crate events in the RVOIP system
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RvoipCrossCrateEvent {
    /// Session-core to dialog-core events
    SessionToDialog(SessionToDialogEvent),

    /// Dialog-core to session-core events
    DialogToSession(DialogToSessionEvent),

    /// Session-core to media-core events
    SessionToMedia(SessionToMediaEvent),

    /// Media-core to session-core events
    MediaToSession(MediaToSessionEvent),

    /// Dialog-core to sip-transport events
    DialogToTransport(DialogToTransportEvent),

    /// Sip-transport to dialog-core events
    TransportToDialog(TransportToDialogEvent),

    /// Sip-transport/dialog transport boundary to session-core diagnostics
    TransportToSession(SipTraceEvent),

    /// Media-core to rtp-core events
    MediaToRtp(MediaToRtpEvent),

    /// Rtp-core to media-core events
    RtpToMedia(RtpToMediaEvent),

    /// Orchestration-plane events (orchestration-core / future rvoip-core).
    /// Per-fine-grained-variant `event_type` so subscribers get separate
    /// per-type broadcast channels in `GlobalEventCoordinator`.
    Orchestration(OrchestrationCrossCrateEvent),

    /// rvoip-core spine events (cross-transport `Connection*` / `Bridge*` /
    /// `Conversation*` / `Session*` vocabulary). Lives on its own variant so
    /// the rvoip-core spine doesn't piggy-back on the legacy `Orchestration`
    /// variant (which is workforce-flavored and disappears with
    /// orchestration-core in PRD §13.3 step 7). Per-fine-grained-variant
    /// `event_type` per the same pattern as `Orchestration`.
    Core(RvoipCoreCrossCrateEvent),
}

/// Trait for cross-crate events
pub trait CrossCrateEvent: Send + Sync + std::fmt::Debug {
    fn event_type(&self) -> EventTypeId;
    fn source_plane(&self) -> PlaneType;
    fn target_plane(&self) -> PlaneType;
    fn priority(&self) -> EventPriority;

    /// Convert to Any for downcasting (trait-based approach)
    fn as_any(&self) -> &dyn Any;
}

impl CrossCrateEvent for RvoipCrossCrateEvent {
    fn event_type(&self) -> EventTypeId {
        match self {
            RvoipCrossCrateEvent::SessionToDialog(_) => "session_to_dialog",
            RvoipCrossCrateEvent::DialogToSession(_) => "dialog_to_session",
            RvoipCrossCrateEvent::SessionToMedia(_) => "session_to_media",
            RvoipCrossCrateEvent::MediaToSession(_) => "media_to_session",
            RvoipCrossCrateEvent::DialogToTransport(_) => "dialog_to_transport",
            RvoipCrossCrateEvent::TransportToDialog(_) => "transport_to_dialog",
            RvoipCrossCrateEvent::TransportToSession(_) => "transport_to_session",
            RvoipCrossCrateEvent::MediaToRtp(_) => "media_to_rtp",
            RvoipCrossCrateEvent::RtpToMedia(_) => "rtp_to_media",
            RvoipCrossCrateEvent::Orchestration(inner) => inner.event_type(),
            RvoipCrossCrateEvent::Core(inner) => inner.event_type(),
        }
    }

    fn source_plane(&self) -> PlaneType {
        match self {
            RvoipCrossCrateEvent::SessionToDialog(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::DialogToSession(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::SessionToMedia(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::MediaToSession(_) => PlaneType::Media,
            RvoipCrossCrateEvent::DialogToTransport(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::TransportToDialog(_) => PlaneType::Transport,
            RvoipCrossCrateEvent::TransportToSession(_) => PlaneType::Transport,
            RvoipCrossCrateEvent::MediaToRtp(_) => PlaneType::Media,
            RvoipCrossCrateEvent::RtpToMedia(_) => PlaneType::Transport,
            RvoipCrossCrateEvent::Orchestration(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::Core(_) => PlaneType::Signaling,
        }
    }

    fn target_plane(&self) -> PlaneType {
        match self {
            RvoipCrossCrateEvent::SessionToDialog(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::DialogToSession(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::SessionToMedia(_) => PlaneType::Media,
            RvoipCrossCrateEvent::MediaToSession(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::DialogToTransport(_) => PlaneType::Transport,
            RvoipCrossCrateEvent::TransportToDialog(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::TransportToSession(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::MediaToRtp(_) => PlaneType::Transport,
            RvoipCrossCrateEvent::RtpToMedia(_) => PlaneType::Media,
            RvoipCrossCrateEvent::Orchestration(_) => PlaneType::Signaling,
            RvoipCrossCrateEvent::Core(_) => PlaneType::Signaling,
        }
    }

    fn priority(&self) -> EventPriority {
        match self {
            RvoipCrossCrateEvent::SessionToDialog(_) => EventPriority::High,
            RvoipCrossCrateEvent::DialogToSession(_) => EventPriority::High,
            RvoipCrossCrateEvent::SessionToMedia(_) => EventPriority::High,
            RvoipCrossCrateEvent::MediaToSession(_) => EventPriority::Normal,
            RvoipCrossCrateEvent::DialogToTransport(_) => EventPriority::High,
            RvoipCrossCrateEvent::TransportToDialog(_) => EventPriority::High,
            RvoipCrossCrateEvent::TransportToSession(_) => EventPriority::Normal,
            RvoipCrossCrateEvent::MediaToRtp(_) => EventPriority::Normal,
            RvoipCrossCrateEvent::RtpToMedia(_) => EventPriority::Normal,
            RvoipCrossCrateEvent::Orchestration(_) => EventPriority::Normal,
            RvoipCrossCrateEvent::Core(_) => EventPriority::Normal,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Event for RvoipCrossCrateEvent {
    fn event_type() -> &'static str {
        "rvoip_cross_crate_event"
    }

    fn priority() -> EventPriority {
        EventPriority::High // Cross-crate events are high priority by default
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl RoutableEvent for RvoipCrossCrateEvent {
    fn event_type(&self) -> &'static str {
        CrossCrateEvent::event_type(self)
    }

    fn session_id(&self) -> Option<&str> {
        // Extract session ID from the event if present
        match self {
            RvoipCrossCrateEvent::SessionToDialog(event) => match event {
                SessionToDialogEvent::InitiateCall { session_id, .. } => Some(session_id),
                SessionToDialogEvent::TerminateSession { session_id, .. } => Some(session_id),
                SessionToDialogEvent::HoldSession { session_id, .. } => Some(session_id),
                SessionToDialogEvent::ResumeSession { session_id, .. } => Some(session_id),
                SessionToDialogEvent::TransferCall { session_id, .. } => Some(session_id),
                SessionToDialogEvent::SendDtmf { session_id, .. } => Some(session_id),
                SessionToDialogEvent::StoreDialogMapping { session_id, .. } => Some(session_id),
                SessionToDialogEvent::ReferResponse { .. } => None, // No session_id in ReferResponse
                SessionToDialogEvent::SendRegisterResponse { .. } => None, // Transaction-based, no session_id
            },
            RvoipCrossCrateEvent::DialogToSession(event) => match event {
                DialogToSessionEvent::IncomingCall { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallStateChanged { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallProgress { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallEstablished { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallTerminated { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallCancelled { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SessionRefreshed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SessionRefreshFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::AuthRequired { session_id, .. } => Some(session_id),
                DialogToSessionEvent::CallRedirected { session_id, .. } => Some(session_id),
                DialogToSessionEvent::ReinviteGlare { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SessionIntervalTooSmall { session_id, .. } => {
                    Some(session_id)
                }
                DialogToSessionEvent::DtmfReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::DialogError { session_id, .. } => Some(session_id),
                DialogToSessionEvent::DialogCreated { .. } => None, // No session_id in DialogCreated
                DialogToSessionEvent::DialogStateChanged { session_id, .. } => Some(session_id),
                DialogToSessionEvent::ReinviteReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::TransferRequested { session_id, .. } => Some(session_id),
                DialogToSessionEvent::AckReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::RegistrationSuccess { session_id, .. } => Some(session_id),
                DialogToSessionEvent::RegistrationFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SubscriptionAccepted { session_id, .. } => Some(session_id),
                DialogToSessionEvent::SubscriptionFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::NotifyReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::MessageDelivered { session_id, .. } => Some(session_id),
                DialogToSessionEvent::MessageFailed { session_id, .. } => Some(session_id),
                DialogToSessionEvent::IncomingRegister { .. } => None, // No session_id yet for incoming REGISTER
                DialogToSessionEvent::OutboundFlowFailed { .. } => None, // Flow-level, not session-level
                DialogToSessionEvent::InfoReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::MessageReceived { session_id, .. } => Some(session_id),
                DialogToSessionEvent::OptionsReceived { session_id, .. } => {
                    if session_id.is_empty() {
                        None
                    } else {
                        Some(session_id)
                    }
                }
            },
            RvoipCrossCrateEvent::SessionToMedia(event) => match event {
                SessionToMediaEvent::StartMediaStream { session_id, .. } => Some(session_id),
                SessionToMediaEvent::StopMediaStream { session_id, .. } => Some(session_id),
                SessionToMediaEvent::UpdateMediaStream { session_id, .. } => Some(session_id),
                SessionToMediaEvent::HoldMedia { session_id, .. } => Some(session_id),
                SessionToMediaEvent::ResumeMedia { session_id, .. } => Some(session_id),
                SessionToMediaEvent::StartRecording { session_id, .. } => Some(session_id),
                SessionToMediaEvent::StopRecording { session_id, .. } => Some(session_id),
                SessionToMediaEvent::PlayAudio { session_id, .. } => Some(session_id),
                SessionToMediaEvent::StopAudio { session_id, .. } => Some(session_id),
            },
            RvoipCrossCrateEvent::MediaToSession(event) => match event {
                MediaToSessionEvent::MediaStreamStarted { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaStreamStopped { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaQualityUpdate { session_id, .. } => Some(session_id),
                MediaToSessionEvent::RecordingStarted { session_id, .. } => Some(session_id),
                MediaToSessionEvent::RecordingStopped { session_id, .. } => Some(session_id),
                MediaToSessionEvent::AudioPlaybackFinished { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaError { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaFlowEstablished { session_id, .. } => Some(session_id),
                MediaToSessionEvent::MediaQualityDegraded { session_id, .. } => Some(session_id),
                MediaToSessionEvent::DtmfDetected { session_id, .. } => Some(session_id),
                MediaToSessionEvent::RtpTimeout { session_id, .. } => Some(session_id),
                MediaToSessionEvent::PacketLossThresholdExceeded { session_id, .. } => {
                    Some(session_id)
                }
            },
            RvoipCrossCrateEvent::DialogToTransport(_) => None, // Transport events don't have session context
            RvoipCrossCrateEvent::TransportToDialog(_) => None,
            RvoipCrossCrateEvent::TransportToSession(event) => event.session_id.as_deref(),
            RvoipCrossCrateEvent::MediaToRtp(event) => match event {
                MediaToRtpEvent::StartRtpStream { session_id, .. } => Some(session_id),
                MediaToRtpEvent::StopRtpStream { session_id, .. } => Some(session_id),
                MediaToRtpEvent::SendRtpPacket { session_id, .. } => Some(session_id),
                MediaToRtpEvent::UpdateRtpStream { session_id, .. } => Some(session_id),
            },
            RvoipCrossCrateEvent::RtpToMedia(event) => match event {
                RtpToMediaEvent::RtpStreamStarted { session_id, .. } => Some(session_id),
                RtpToMediaEvent::RtpStreamStopped { session_id, .. } => Some(session_id),
                RtpToMediaEvent::RtpPacketReceived { session_id, .. } => Some(session_id),
                RtpToMediaEvent::RtpStatisticsUpdate { session_id, .. } => Some(session_id),
                RtpToMediaEvent::RtpError { session_id, .. } => Some(session_id),
            },
            // Orchestration events use call_id, not SIP session_id, so no
            // session-bound routing is offered. Subscribers route by event_type.
            RvoipCrossCrateEvent::Orchestration(_) => None,
            // rvoip-core spine events use cross-transport ConnectionId /
            // SessionId / ConversationId vocabulary, not the SIP session_id
            // dispatched by RoutableEvent. Subscribers route by event_type.
            RvoipCrossCrateEvent::Core(_) => None,
        }
    }
}

// =============================================================================
// SIP TRACE EVENTS
// =============================================================================

/// Direction of a traced SIP message at the transport boundary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SipTraceDirection {
    /// Message received from a remote peer.
    Inbound,
    /// Message sent to a remote peer.
    Outbound,
}

impl SipTraceDirection {
    /// Compact arrow used by CLIs and logs.
    pub fn arrow(&self) -> &'static str {
        match self {
            Self::Inbound => "<",
            Self::Outbound => ">",
        }
    }
}

/// Runtime policy for SIP trace emission.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct SipTraceConfig {
    /// Whether SIP trace events should be emitted.
    pub enabled: bool,
    /// Suggested in-memory capacity for consumers that keep a trace ring.
    pub capacity: usize,
    /// Whether authentication-bearing SIP headers should be redacted.
    pub redact_sensitive_headers: bool,
    /// Whether message bodies, including SDP, should be included.
    pub include_body: bool,
}

impl SipTraceConfig {
    /// Default bounded trace capacity.
    pub const DEFAULT_CAPACITY: usize = 256;

    /// Create an enabled trace config with default redaction.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }
}

impl Default for SipTraceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            capacity: Self::DEFAULT_CAPACITY,
            redact_sensitive_headers: true,
            include_body: true,
        }
    }
}

/// SIP message observed at the transport boundary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SipTraceEvent {
    /// Coordinator-specific owner id used to filter global trace events.
    pub owner_id: String,
    /// Inbound or outbound at the local transport boundary.
    pub direction: SipTraceDirection,
    /// Transport flavour, for example `UDP`, `TCP`, or `TLS`.
    pub transport: String,
    /// Local socket address.
    pub local_addr: String,
    /// Remote socket address.
    pub remote_addr: String,
    /// Milliseconds since Unix epoch when the trace event was created.
    pub timestamp_unix_millis: u64,
    /// SIP start line, for example `INVITE sip:bob@example.com SIP/2.0`.
    pub start_line: String,
    /// Wire-level SIP `Call-ID` header value when present.
    pub sip_call_id: Option<String>,
    /// Session-core session id after mapping, when known.
    pub session_id: Option<String>,
    /// Redacted, optionally body-stripped SIP message text.
    pub raw_message: String,
    /// Original rendered message byte length before redaction/body stripping/truncation.
    pub original_len: usize,
    /// Whether `raw_message` was truncated for bounded diagnostics.
    pub truncated: bool,
    /// Whether sensitive headers were redacted.
    pub redacted: bool,
}

/// Maximum rendered SIP message bytes kept in one trace event.
pub const SIP_TRACE_MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// Apply trace policy to a rendered SIP message.
pub fn format_sip_trace_message(raw: &str, config: &SipTraceConfig) -> (String, bool) {
    let original_len = raw.len();
    let mut message = normalize_line_endings(raw);

    if config.redact_sensitive_headers {
        message = redact_sip_message(&message);
    }

    if !config.include_body {
        message = strip_sip_body(&message);
    }

    let (message, truncated) = truncate_at_char_boundary(&message, SIP_TRACE_MAX_MESSAGE_BYTES);
    (
        format_truncation(message, original_len, truncated),
        truncated,
    )
}

/// Redact auth-bearing SIP headers while preserving all other lines.
pub fn redact_sip_message(raw: &str) -> String {
    let mut in_headers = true;
    let mut redacted = Vec::new();

    for line in raw.lines() {
        let trimmed = line.trim_end_matches('\r');
        if in_headers && trimmed.is_empty() {
            in_headers = false;
            redacted.push(String::new());
            continue;
        }

        if in_headers {
            if let Some((name, _value)) = trimmed.split_once(':') {
                if is_sensitive_sip_header(name) {
                    redacted.push(format!("{}: <redacted>", name.trim()));
                    continue;
                }
            }
        }

        redacted.push(trimmed.to_string());
    }

    redacted.join("\n")
}

fn normalize_line_endings(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\r', "\n")
}

fn strip_sip_body(raw: &str) -> String {
    if let Some((headers, body)) = raw.split_once("\n\n") {
        if body.is_empty() {
            headers.to_string()
        } else {
            format!("{headers}\n\n<body omitted>")
        }
    } else {
        raw.to_string()
    }
}

fn is_sensitive_sip_header(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "www-authenticate" | "proxy-authenticate"
    )
}

fn truncate_at_char_boundary(raw: &str, max_bytes: usize) -> (String, bool) {
    if raw.len() <= max_bytes {
        return (raw.to_string(), false);
    }

    let mut end = max_bytes;
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    (raw[..end].to_string(), true)
}

fn format_truncation(mut message: String, original_len: usize, truncated: bool) -> String {
    if truncated {
        message.push_str(&format!(
            "\n\n<truncated: original message was {original_len} bytes>"
        ));
    }
    message
}

// =============================================================================
// SESSION-CORE ↔ DIALOG-CORE EVENTS
// =============================================================================

/// Events sent from session-core to dialog-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SessionToDialogEvent {
    /// Request to initiate a new call
    InitiateCall {
        session_id: String,
        from: String,
        to: String,
        sdp_offer: Option<String>,
        headers: HashMap<String, String>,
    },

    /// Request to terminate a session
    TerminateSession { session_id: String, reason: String },

    /// Request to hold a session
    HoldSession { session_id: String },

    /// Request to resume a session from hold
    ResumeSession {
        session_id: String,
        sdp_offer: Option<String>,
    },

    /// Request to transfer a call
    TransferCall {
        session_id: String,
        target: String,
        transfer_type: TransferType,
    },

    /// Send DTMF tones
    SendDtmf { session_id: String, tones: String },

    /// Store dialog mapping (response to DialogCreated)
    StoreDialogMapping {
        session_id: String,
        dialog_id: String,
    },

    /// Response to REFER request (Accept/Reject decision)
    ReferResponse {
        transaction_id: String,
        accept: bool,
        status_code: u16,
        reason: String,
    },

    /// Send REGISTER response (401/200) - server-side
    SendRegisterResponse {
        transaction_id: String,
        status_code: u16,
        reason: String,
        www_authenticate: Option<String>, // For 401 challenge
        contact: Option<String>,          // For 200 OK
        expires: Option<u32>,             // For 200 OK
        /// SIP_API_DESIGN_2 Phase D — RFC 3261 §20.23 `Min-Expires` for
        /// 423 Interval Too Brief responses.
        min_expires: Option<u32>,
        /// SIP_API_DESIGN_2 Phase D — RFC 3608 `Service-Route` URIs
        /// returned on REGISTER 2xx; out-of-dialog requests within the
        /// registration binding SHOULD pre-load these as Route headers.
        service_route: Vec<String>,
        /// SIP_API_DESIGN_2 Phase D — RFC 3327 `Path` echo flag. When
        /// true, the registrar echoes any `Path:` headers seen on the
        /// inbound REGISTER back on the 2xx so subsequent re-targeted
        /// requests reach the UA through the same waypoints.
        path_echo: bool,
        /// SIP_API_DESIGN_2 Phase D — RFC 3455 `P-Associated-URI` list
        /// returned on REGISTER 2xx so the UA learns the additional
        /// AORs the registrar has provisioned for the same subscriber.
        associated_uri: Vec<String>,
        /// SIP_API_DESIGN_2 Phase D — additional application-staged
        /// headers as `(name, value)` wire-format tuples. The
        /// receiving dialog-core handler reconstructs `TypedHeader`s
        /// via sip-core; infra-common stays SIP-agnostic by carrying
        /// only strings here.
        extra_headers: Vec<(String, String)>,
    },
}

/// Events sent from dialog-core to session-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DialogToSessionEvent {
    /// Incoming call notification
    IncomingCall {
        session_id: String,
        call_id: String,
        from: String,
        to: String,
        sdp_offer: Option<String>,
        headers: HashMap<String, String>,
        /// Transaction ID for sending responses
        transaction_id: String,
        /// Source address for responses
        source_addr: String,
        /// SIP_API_DESIGN_2 Phase A: original inbound INVITE bytes,
        /// preserved end-to-end so `IncomingCall::raw_request()` can
        /// expose the parsed `Arc<Request>`. `infra-common` stays
        /// SIP-agnostic by carrying `Bytes`, not the typed Request.
        /// `None` for legacy publish sites that haven't been migrated
        /// yet.
        #[serde(skip)]
        raw_request: Option<Arc<Bytes>>,
    },

    /// Call state change notification
    CallStateChanged {
        session_id: String,
        new_state: CallState,
        reason: Option<String>,
    },

    /// Provisional 1xx call progress response received for an outgoing call.
    CallProgress {
        session_id: String,
        /// SIP provisional status code.
        status_code: u16,
        /// SIP reason phrase.
        reason_phrase: String,
        /// SDP body carried by the provisional response, if present.
        sdp: Option<String>,
        /// SIP_API_DESIGN_2 Phase A: original inbound response bytes
        /// so B2BUA callers can build an `IncomingResponse` view
        /// (`Allow` / `Supported` / `Server` carry-through to the
        /// downstream 183).
        #[serde(skip)]
        raw_response: Option<Arc<Bytes>>,
    },

    /// Call successfully established
    CallEstablished {
        session_id: String,
        sdp_answer: Option<String>,
        /// SIP_API_DESIGN_2 Phase A: original inbound 200 OK bytes
        /// for downstream carry-through.
        #[serde(skip)]
        raw_response: Option<Arc<Bytes>>,
    },

    /// Call terminated notification
    CallTerminated {
        session_id: String,
        reason: TerminationReason,
    },

    /// Final failure response received for an outgoing request
    /// (3xx redirect, 4xx client error, 5xx server error, 6xx global failure).
    /// RFC 3261 §8.1.3 — the UAC transaction is complete on a final response.
    CallFailed {
        session_id: String,
        status_code: u16,
        reason_phrase: String,
        /// SIP_API_DESIGN_2 Phase A: original inbound failure response
        /// bytes so applications can inspect `Retry-After`, `Warning`,
        /// `Reason`, and friends via `IncomingResponse::raw_response()`.
        #[serde(skip)]
        raw_response: Option<Arc<Bytes>>,
    },

    /// Caller cancelled before the call was answered (RFC 3261 §15.1.2 —
    /// 487 Request Terminated after CANCEL). Distinct from CallFailed so
    /// applications can render "missed call" UX rather than "call failed".
    CallCancelled { session_id: String },

    /// RFC 4028 session-timer refresh succeeded (UPDATE or re-INVITE
    /// completed round-trip). Emitted once per successful refresh.
    SessionRefreshed {
        session_id: String,
        expires_secs: u32,
    },

    /// RFC 4028 session-timer refresh failed; the dialog has been torn
    /// down with BYE (§10). A subsequent CallTerminated will also fire.
    SessionRefreshFailed { session_id: String, reason: String },

    /// RFC 3261 §22.2 — server challenged the UAC request. Emitted on any
    /// 401 Unauthorized or 407 Proxy Authentication Required that carries a
    /// parseable challenge header. Method-agnostic: INVITE, REGISTER, and
    /// future auth-challenged requests all route through this variant. If
    /// the caller has credentials on file, session-core computes the
    /// digest response and retries; otherwise this converts to a final
    /// `CallFailed` / `RegistrationFailed` at the app level.
    AuthRequired {
        session_id: String,
        /// 401 or 407.
        status_code: u16,
        /// Raw challenge header value (e.g. `Digest realm="...", nonce="..."`).
        /// Passed verbatim to `auth-core::DigestAuthenticator::parse_challenge`.
        challenge: String,
        /// Pre-extracted realm, convenience for logging / app-level routing.
        /// Authoritative parse is still done by auth-core.
        realm: Option<String>,
        /// SIP method of the challenged request, extracted from the
        /// response `CSeq:` header (`"INVITE"`, `"REGISTER"`, `"BYE"`,
        /// `"SUBSCRIBE"`, …). Empty string for legacy publish paths
        /// that haven't been updated to populate this field; the
        /// session-side handler treats `""` as "method-agnostic" and
        /// falls back to inspecting which `pending_*_options` stash
        /// is set on the session. Populated by
        /// `rvoip-sip-dialog/src/events/event_hub.rs` for the canonical
        /// 401/407 response path.
        method: String,
    },

    /// 3xx redirect response received (RFC 3261 §8.1.3.4 / §21.3). The UAC
    /// SHOULD retry the INVITE against the first URI in `targets`. `q_values`
    /// carries the relative priority from Contact headers (RFC 3261 §20.10);
    /// each entry defaults to 1.0 when the server omits it.
    CallRedirected {
        session_id: String,
        status_code: u16,
        targets: Vec<String>,
        q_values: Vec<f32>,
    },

    /// 491 Request Pending for a mid-dialog request (RFC 3261 §14.1). The
    /// UAC SHOULD wait a random interval and retry. Emitted only for
    /// re-INVITEs (and UPDATEs) — call-setup INVITEs fall through the
    /// generic CallFailed path.
    ReinviteGlare { session_id: String },

    /// RFC 4028 §6 — 422 Session Interval Too Small on INVITE. The UAS
    /// requires a longer session interval than the UAC offered; its
    /// `Min-SE:` header (extracted into `min_se_secs`) carries the required
    /// floor. The UAC should resend the INVITE with a `Session-Expires`
    /// bumped to at least `min_se_secs`.
    ///
    /// session-core handles this transparently with a two-retry cap
    /// mirroring the 423 REGISTER-retry pattern. If the response is
    /// missing a parseable `Min-SE` header the event falls through to
    /// generic `CallFailed`.
    SessionIntervalTooSmall {
        session_id: String,
        /// Required minimum session interval, in seconds, parsed from the
        /// server's `Min-SE:` header.
        min_se_secs: u32,
    },

    /// DTMF tones received
    DtmfReceived { session_id: String, tones: String },

    /// Dialog error occurred
    DialogError {
        session_id: String,
        error: String,
        error_code: Option<u32>,
    },

    /// Dialog created (for session-core to track)
    DialogCreated { dialog_id: String, call_id: String },

    /// Dialog state changed
    DialogStateChanged {
        session_id: String,
        old_state: DialogState,
        new_state: DialogState,
    },

    /// Re-INVITE or UPDATE received (mid-dialog request). `method` is the
    /// uppercase SIP method string ("INVITE" or "UPDATE"); session-core
    /// uses it to dispatch to the correct state-table event.
    ReinviteReceived {
        session_id: String,
        sdp: Option<String>,
        method: String,
        /// SIP_API_DESIGN_2 Phase E: original inbound re-INVITE / UPDATE
        /// bytes so applications can build an `IncomingRequest` view
        /// for B2BUA carry-through to the downstream leg.
        #[serde(skip)]
        raw_request: Option<Arc<Bytes>>,
    },

    /// Transfer requested
    TransferRequested {
        session_id: String,
        refer_to: String,
        transfer_type: TransferType,
        transaction_id: String,
        /// Optional Referred-By header value from the REFER request.
        referred_by: Option<String>,
        /// Optional Replaces value, either from a Replaces header or the
        /// Refer-To URI for attended-transfer primitives.
        replaces: Option<String>,
        /// SIP_API_DESIGN_2 Phase E: original inbound REFER bytes so
        /// applications can inspect History-Info / Diversion / custom
        /// headers via the `IncomingRequest` view.
        #[serde(skip)]
        raw_request: Option<Arc<Bytes>>,
    },

    /// ACK received (for UAS state transitions)
    AckReceived {
        session_id: String,
        sdp: Option<String>,
    },

    /// Registration successful
    RegistrationSuccess { session_id: String },

    /// Registration failed
    RegistrationFailed {
        session_id: String,
        status_code: u16,
    },

    /// Subscription accepted
    SubscriptionAccepted { session_id: String },

    /// Subscription failed
    SubscriptionFailed {
        session_id: String,
        status_code: u16,
    },

    /// NOTIFY received.
    ///
    /// Published by dialog-core after validating an inbound NOTIFY and
    /// sending the 200 OK (RFC 6665). Session-core uses this to surface
    /// `Event::NotifyReceived` on the public event stream; for REFER
    /// subscriptions (`event_package == "refer"`) with a
    /// `message/sipfrag` body it also parses the sipfrag status line
    /// into `Event::TransferProgress` / `TransferCompleted` / `TransferFailed`
    /// so the transferor (including b2bua wrappers) can observe the
    /// transferee's progress.
    NotifyReceived {
        session_id: String,
        event_package: String,
        /// Raw `Subscription-State:` header value (unparsed), e.g.
        /// `"active;expires=3600"` or `"terminated;reason=noresource"`.
        subscription_state: Option<String>,
        /// Raw `Content-Type:` header value, e.g. `"message/sipfrag"`.
        content_type: Option<String>,
        body: Option<String>,
        /// SIP_API_DESIGN_2 Phase E: original inbound NOTIFY bytes
        /// for `IncomingRequest`-style typed inspection.
        #[serde(skip)]
        raw_request: Option<Arc<Bytes>>,
    },

    /// SIP_API_DESIGN_2 Phase E — in-dialog INFO (RFC 6086) received.
    /// Today's stack drops inbound INFO at the dialog-core layer; this
    /// variant bridges it to session-core so applications can wire
    /// SIP-INFO DTMF, fax flow control, and other mid-dialog
    /// signalling through a typed `IncomingRequest`.
    InfoReceived {
        session_id: String,
        /// Raw inbound INFO bytes; subscribers reconstruct an
        /// `Arc<Request>` via `parse_message`.
        #[serde(skip)]
        raw_request: Option<Arc<Bytes>>,
    },

    /// SIP_API_DESIGN_2 Phase E — in-dialog MESSAGE (RFC 3428)
    /// received.
    MessageReceived {
        session_id: String,
        #[serde(skip)]
        raw_request: Option<Arc<Bytes>>,
    },

    /// SIP_API_DESIGN_2 Phase E — OPTIONS received. May arrive
    /// in-dialog (keep-alive probe on an established call) or
    /// out-of-dialog (capability query against the AOR); `session_id`
    /// is empty when out-of-dialog.
    OptionsReceived {
        session_id: String,
        #[serde(skip)]
        raw_request: Option<Arc<Bytes>>,
    },

    /// MESSAGE delivered
    MessageDelivered { session_id: String },

    /// MESSAGE delivery failed
    MessageFailed {
        session_id: String,
        status_code: u16,
    },

    /// Incoming REGISTER request (server-side)
    IncomingRegister {
        transaction_id: String,
        from_uri: String,
        to_uri: String,
        contact_uri: String,
        expires: u32,
        authorization: Option<String>, // Authorization header if present
        call_id: String,
        /// SIP_API_DESIGN_2 Phase A: original inbound REGISTER bytes,
        /// preserved so registrar surfaces can build an
        /// `IncomingRegister::raw_request()` view. `None` for legacy
        /// publish sites until migration.
        #[serde(skip)]
        raw_request: Option<Arc<Bytes>>,
    },

    /// RFC 5626 outbound flow has failed — the keep-alive ping either
    /// timed out, saw a transport-level connection close, or hit an
    /// unrecoverable send error. Session-core debounces this event per
    /// AoR and triggers a fresh REGISTER to re-establish the flow
    /// without waiting for registration expiry (§4.4.1 flow recovery).
    ///
    /// Flow-level, not session-level: `session_id()` returns `None`.
    OutboundFlowFailed {
        /// AoR (To URI of the REGISTER that established the flow,
        /// normalized to string form).
        aor: String,
        /// RFC 5626 §4.2 `reg-id` of the failed flow.
        reg_id: u32,
        /// RFC 5626 §4.1 instance URN of the UA.
        instance: String,
        /// Human-readable failure cause (`"PongTimeout"`,
        /// `"ConnectionClosed"`, or `"SendError"`) — used for
        /// telemetry and log correlation.
        reason: String,
    },
}

// =============================================================================
// SESSION-CORE ↔ MEDIA-CORE EVENTS
// =============================================================================

/// Events sent from session-core to media-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SessionToMediaEvent {
    /// Start media stream for session
    StartMediaStream {
        session_id: String,
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
        media_config: MediaStreamConfig,
    },

    /// Stop media stream for session
    StopMediaStream { session_id: String },

    /// Update media stream configuration
    UpdateMediaStream {
        session_id: String,
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
    },

    /// Hold media stream
    HoldMedia { session_id: String },

    /// Resume media stream
    ResumeMedia { session_id: String },

    /// Start recording
    StartRecording {
        session_id: String,
        file_path: String,
        format: RecordingFormat,
    },

    /// Stop recording
    StopRecording { session_id: String },

    /// Play audio file
    PlayAudio {
        session_id: String,
        file_path: String,
        loop_count: Option<u32>,
    },

    /// Stop audio playback
    StopAudio { session_id: String },
}

/// Events sent from media-core to session-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MediaToSessionEvent {
    /// Media stream started successfully
    MediaStreamStarted {
        session_id: String,
        local_port: u16,
        codec: String,
    },

    /// Media stream stopped
    MediaStreamStopped { session_id: String, reason: String },

    /// Media quality update
    MediaQualityUpdate {
        session_id: String,
        quality_metrics: MediaQualityMetrics,
    },

    /// Recording started
    RecordingStarted {
        session_id: String,
        file_path: String,
    },

    /// Recording stopped
    RecordingStopped {
        session_id: String,
        file_path: String,
        duration_ms: u64,
    },

    /// Audio playback finished
    AudioPlaybackFinished { session_id: String },

    /// Media error occurred
    MediaError {
        session_id: String,
        error: String,
        error_code: Option<u32>,
    },

    /// Media flow established
    MediaFlowEstablished { session_id: String },

    /// Media quality degraded
    MediaQualityDegraded {
        session_id: String,
        metrics: MediaQualityMetrics,
        severity: QualitySeverity,
    },

    /// DTMF detected
    DtmfDetected {
        session_id: String,
        digit: char,
        duration_ms: u32,
    },

    /// RTP timeout
    RtpTimeout {
        session_id: String,
        last_packet_time: u64,
    },

    /// Packet loss threshold exceeded
    PacketLossThresholdExceeded {
        session_id: String,
        loss_percentage: f32,
    },
}

// =============================================================================
// DIALOG-CORE ↔ SIP-TRANSPORT EVENTS
// =============================================================================

/// Events sent from dialog-core to sip-transport
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DialogToTransportEvent {
    /// Send SIP message
    SendSipMessage {
        destination: String,
        method: String,
        headers: HashMap<String, String>,
        body: Option<String>,
        transaction_id: Option<String>,
    },

    /// Send SIP response
    SendSipResponse {
        transaction_id: String,
        status_code: u16,
        reason_phrase: String,
        headers: HashMap<String, String>,
        body: Option<String>,
    },

    /// Register SIP endpoint
    RegisterEndpoint {
        uri: String,
        expires: Option<u32>,
        contact: Option<String>,
    },

    /// Unregister SIP endpoint
    UnregisterEndpoint { uri: String },
}

/// Events sent from sip-transport to dialog-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TransportToDialogEvent {
    /// SIP message received
    SipMessageReceived {
        source: String,
        method: String,
        headers: HashMap<String, String>,
        body: Option<String>,
        transaction_id: String,
    },

    /// SIP response received
    SipResponseReceived {
        transaction_id: String,
        status_code: u16,
        reason_phrase: String,
        headers: HashMap<String, String>,
        body: Option<String>,
    },

    /// Transport error occurred
    TransportError {
        error: String,
        transaction_id: Option<String>,
    },

    /// Registration status update
    RegistrationStatusUpdate {
        uri: String,
        status: RegistrationStatus,
        expires: Option<u32>,
    },
}

// =============================================================================
// MEDIA-CORE ↔ RTP-CORE EVENTS
// =============================================================================

/// Events sent from media-core to rtp-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum MediaToRtpEvent {
    /// Start RTP stream
    StartRtpStream {
        session_id: String,
        local_port: u16,
        remote_address: String,
        remote_port: u16,
        payload_type: u8,
        codec: String,
    },

    /// Stop RTP stream
    StopRtpStream { session_id: String },

    /// Send RTP packet
    SendRtpPacket {
        session_id: String,
        payload: Vec<u8>,
        timestamp: u32,
        sequence_number: u16,
    },

    /// Update RTP stream parameters
    UpdateRtpStream {
        session_id: String,
        remote_address: Option<String>,
        remote_port: Option<u16>,
    },
}

/// Events sent from rtp-core to media-core
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RtpToMediaEvent {
    /// RTP stream started
    RtpStreamStarted { session_id: String, local_port: u16 },

    /// RTP stream stopped
    RtpStreamStopped { session_id: String, reason: String },

    /// RTP packet received
    RtpPacketReceived {
        session_id: String,
        payload: Vec<u8>,
        timestamp: u32,
        sequence_number: u16,
        payload_type: u8,
    },

    /// RTP statistics update
    RtpStatisticsUpdate {
        session_id: String,
        stats: RtpStatistics,
    },

    /// RTP error occurred
    RtpError { session_id: String, error: String },
}

// =============================================================================
// SUPPORTING TYPES
// =============================================================================

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CallState {
    Initiating,
    Ringing,
    Active,
    OnHold,
    Transferring,
    Terminating,
    Terminated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TerminationReason {
    LocalHangup,
    RemoteHangup,
    Rejected(String),
    Error(String),
    Timeout,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DialogState {
    Initial,
    Early,
    Confirmed,
    Recovering,
    Terminated,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum QualitySeverity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TransferType {
    Blind,
    Attended,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaStreamConfig {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub enable_dtx: bool,
    pub enable_fec: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RecordingFormat {
    Wav,
    Mp3,
    Flac,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MediaQualityMetrics {
    pub mos_score: f64,
    pub packet_loss: f64,
    pub jitter_ms: f64,
    pub delay_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RegistrationStatus {
    Registered,
    Unregistered,
    Failed(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RtpStatistics {
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub packet_loss_rate: f64,
    pub jitter_ms: f64,
}

/// Helper functions for creating cross-crate events
impl RvoipCrossCrateEvent {
    /// Create a session to dialog initiate call event
    pub fn initiate_call(
        session_id: String,
        from: String,
        to: String,
        sdp_offer: Option<String>,
    ) -> Self {
        RvoipCrossCrateEvent::SessionToDialog(SessionToDialogEvent::InitiateCall {
            session_id,
            from,
            to,
            sdp_offer,
            headers: HashMap::new(),
        })
    }

    /// Create an incoming call event
    pub fn incoming_call(
        session_id: String,
        call_id: String,
        from: String,
        to: String,
        sdp_offer: Option<String>,
    ) -> Self {
        RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::IncomingCall {
            session_id,
            call_id,
            from,
            to,
            sdp_offer,
            headers: HashMap::new(),
            transaction_id: String::new(), // Must be set by caller
            source_addr: String::new(),    // Must be set by caller
            raw_request: None,
        })
    }

    /// Create a call state changed event
    pub fn call_state_changed(
        session_id: String,
        new_state: CallState,
        reason: Option<String>,
    ) -> Self {
        RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallStateChanged {
            session_id,
            new_state,
            reason,
        })
    }

    /// Create a provisional call progress event.
    pub fn call_progress(
        session_id: String,
        status_code: u16,
        reason_phrase: String,
        sdp: Option<String>,
    ) -> Self {
        RvoipCrossCrateEvent::DialogToSession(DialogToSessionEvent::CallProgress {
            session_id,
            status_code,
            reason_phrase,
            sdp,
            raw_response: None,
        })
    }

    /// Create a start media stream event
    pub fn start_media_stream(
        session_id: String,
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
        config: MediaStreamConfig,
    ) -> Self {
        RvoipCrossCrateEvent::SessionToMedia(SessionToMediaEvent::StartMediaStream {
            session_id,
            local_sdp,
            remote_sdp,
            media_config: config,
        })
    }
}

// =============================================================================
// ORCHESTRATION-PLANE EVENTS
// =============================================================================

/// Wire-format orchestration events for cross-crate observability.
///
/// Mirrors `orchestration-core::OrchestrationEvent` with primitive payloads
/// (string IDs, no rich struct payloads) so the wire format does not pull
/// orchestration-core types into infra-common. Each variant maps to a
/// distinct `event_type()` string so `GlobalEventCoordinator` allocates a
/// separate broadcast channel per variant — a slow consumer of one variant
/// does not lag consumers of another.
///
/// In-process subscribers within orchestration-core continue to use the
/// rich, typed `OrchestrationEvent` API; this wire form exists for
/// cross-crate observers (logging sinks, future rvoip-harness, telemetry).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum OrchestrationCrossCrateEvent {
    InboundCallReceived {
        call_id: String,
        caller_uri: String,
        to: String,
    },
    CallCreated {
        call_id: String,
    },
    CallQueued {
        call_id: String,
        queue_id: String,
    },
    CallDequeued {
        call_id: String,
        queue_id: String,
    },
    QueueOverflowed {
        call_id: String,
        from_queue_id: String,
        target: String,
        reason: String,
    },
    CallStatusChanged {
        call_id: String,
        from: String,
        to: String,
    },
    AgentStateChanged {
        agent_id: String,
        from: String,
        to: String,
    },
    AgentReserved {
        call_id: String,
        agent_id: String,
        offer_id: String,
    },
    AgentOfferAccepted {
        call_id: String,
        agent_id: String,
        offer_id: String,
    },
    AgentOfferRejected {
        call_id: String,
        agent_id: String,
        offer_id: String,
        reason: String,
    },
    AgentOfferTimedOut {
        call_id: String,
        agent_id: String,
        offer_id: String,
    },
    AgentOfferFailed {
        call_id: String,
        agent_id: String,
        offer_id: String,
        reason: String,
    },
    VoiceAiStarted {
        call_id: String,
        agent_id: String,
    },
    VoiceAiTranscript {
        call_id: String,
        agent_id: String,
        text: String,
        is_final: bool,
    },
    VoiceAiBargeIn {
        call_id: String,
        agent_id: String,
    },
    VoiceAiEnded {
        call_id: String,
        agent_id: String,
        reason: String,
    },
    BridgeStarted {
        call_id: String,
        bridge_id: String,
        caller_leg_id: String,
        agent_leg_id: String,
    },
    BridgeEnded {
        call_id: String,
        bridge_id: String,
        reason: String,
    },
    RecordingStarted {
        call_id: String,
        recording_id: String,
    },
    RecordingStopped {
        call_id: String,
        recording_id: String,
    },
    TransferRequested {
        call_id: String,
        from_agent_id: String,
        target: String,
    },
    TransferCompleted {
        call_id: String,
        target: String,
    },
    CallEnded {
        call_id: String,
        reason: String,
    },
    CallFailed {
        call_id: String,
        reason: String,
    },
}

impl OrchestrationCrossCrateEvent {
    /// Per-variant event type string, used by `GlobalEventCoordinator` to
    /// allocate a separate broadcast channel per variant.
    pub fn event_type(&self) -> EventTypeId {
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

    /// All orchestration event-type strings, in declaration order. Used by
    /// `EventTypeRegistry::register_builtin_types` to register every variant.
    pub const ALL_EVENT_TYPES: &'static [EventTypeId] = &[
        "orchestration.inbound_call_received",
        "orchestration.call_created",
        "orchestration.call_queued",
        "orchestration.call_dequeued",
        "orchestration.queue_overflowed",
        "orchestration.call_status_changed",
        "orchestration.agent_state_changed",
        "orchestration.agent_reserved",
        "orchestration.agent_offer_accepted",
        "orchestration.agent_offer_rejected",
        "orchestration.agent_offer_timed_out",
        "orchestration.agent_offer_failed",
        "orchestration.voice_ai_started",
        "orchestration.voice_ai_transcript",
        "orchestration.voice_ai_barge_in",
        "orchestration.voice_ai_ended",
        "orchestration.bridge_started",
        "orchestration.bridge_ended",
        "orchestration.recording_started",
        "orchestration.recording_stopped",
        "orchestration.transfer_requested",
        "orchestration.transfer_completed",
        "orchestration.call_ended",
        "orchestration.call_failed",
    ];
}

// =============================================================================
// RVOIP-CORE SPINE EVENTS
// =============================================================================

/// Wire-format rvoip-core spine events for cross-crate observability.
///
/// Mirrors `rvoip_core::events::Event` with primitive payloads (string IDs,
/// no rich struct payloads) so the wire format does not pull rvoip-core types
/// into infra-common. Each variant maps to a distinct `event_type()` string
/// so `GlobalEventCoordinator` allocates a separate broadcast channel per
/// variant (a slow consumer of one variant does not lag others).
///
/// In-process subscribers within rvoip-core continue to use the rich, typed
/// `Event` API; this wire form exists for cross-crate observers (logging
/// sinks, harness, telemetry, the rvoip facade).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RvoipCoreCrossCrateEvent {
    // --- Conversation lifecycle ---
    ConversationOpened { conversation_id: String },
    ConversationClosed { conversation_id: String },

    // --- Session lifecycle ---
    SessionStarted { session_id: String, conversation_id: String },
    SessionEnded { session_id: String },
    SessionFailed { session_id: String, detail: String },

    // --- Connection lifecycle ---
    ConnectionInbound { connection_id: String },
    ConnectionOutbound { connection_id: String },
    ConnectionConnected { connection_id: String },
    ConnectionProgress { connection_id: String, kind: String },
    ConnectionEnded { connection_id: String, reason: String },
    ConnectionFailed { connection_id: String, detail: String },

    // --- Bridge lifecycle ---
    ConnectionsBridged { bridge_id: String, a: String, b: String },
    ConnectionsUnbridged { bridge_id: String },

    // --- Transfer ---
    ConnectionTransferred { connection_id: String, target: String },

    // --- Participant lifecycle ---
    ParticipantJoined { session_id: String, participant_id: String },
    ParticipantLeft { session_id: String, participant_id: String },

    // --- AI / listener attach ---
    AiAttached {
        connection_id: String,
        attachment_id: String,
        provider_ref: String,
    },
    AiDetached { attachment_id: String },
    ListenerAttached { listener_id: String },
    ListenerDetached { listener_id: String },

    // --- Messaging ---
    MessageReceived { message_id: String, conversation_id: String },
    MessageSent { message_id: String, conversation_id: String },
    MessageDelivered { message_id: String },
    MessageRead { message_id: String },

    // --- DTMF ---
    DtmfReceived { connection_id: String, digits: String },

    // --- Transcription / recording ---
    TranscriptTurn {
        stream_id: String,
        speaker: Option<String>,
        text: String,
        confidence: f32,
        is_final: bool,
        assigned_provider: Option<String>,
    },
    RecordingStarted { recording_id: String },
    RecordingStopped { recording_id: String },
    RecordingComplete { recording_id: String, sink: String },

    // --- vCon ---
    VconReady { session_id: String, handle_url: String, content_hash: String },
    VconRedacted {
        session_id: String,
        old_url: String,
        new_url: String,
    },

    // --- Identity ---
    IdentityAssuranceChanged {
        connection_id: String,
        identity_id: Option<String>,
    },

    // --- Registration ---
    RegistrationChanged { aor: String },
    RegistrationHeartbeat { aor: String },

    // --- Observability ---
    CapacityReport {
        tenant_id: Option<String>,
        active_connections: u64,
        active_bridges: u64,
        admission_in_use: u64,
    },
    UsageRecord { tenant_id: String, kind: String, units: u64 },
    Anomaly {
        kind: String,
        connection_id: Option<String>,
        detail: String,
    },
    MediaQuality {
        connection_id: String,
        jitter_ms: f32,
        packet_loss_pct: f32,
        mos: Option<f32>,
    },
}

impl RvoipCoreCrossCrateEvent {
    /// Per-variant event type string, used by `GlobalEventCoordinator` to
    /// allocate a separate broadcast channel per variant.
    pub fn event_type(&self) -> EventTypeId {
        match self {
            Self::ConversationOpened { .. } => "rvoip_core.conversation_opened",
            Self::ConversationClosed { .. } => "rvoip_core.conversation_closed",
            Self::SessionStarted { .. } => "rvoip_core.session_started",
            Self::SessionEnded { .. } => "rvoip_core.session_ended",
            Self::SessionFailed { .. } => "rvoip_core.session_failed",
            Self::ConnectionInbound { .. } => "rvoip_core.connection_inbound",
            Self::ConnectionOutbound { .. } => "rvoip_core.connection_outbound",
            Self::ConnectionConnected { .. } => "rvoip_core.connection_connected",
            Self::ConnectionProgress { .. } => "rvoip_core.connection_progress",
            Self::ConnectionEnded { .. } => "rvoip_core.connection_ended",
            Self::ConnectionFailed { .. } => "rvoip_core.connection_failed",
            Self::ConnectionsBridged { .. } => "rvoip_core.connections_bridged",
            Self::ConnectionsUnbridged { .. } => "rvoip_core.connections_unbridged",
            Self::ConnectionTransferred { .. } => "rvoip_core.connection_transferred",
            Self::ParticipantJoined { .. } => "rvoip_core.participant_joined",
            Self::ParticipantLeft { .. } => "rvoip_core.participant_left",
            Self::AiAttached { .. } => "rvoip_core.ai_attached",
            Self::AiDetached { .. } => "rvoip_core.ai_detached",
            Self::ListenerAttached { .. } => "rvoip_core.listener_attached",
            Self::ListenerDetached { .. } => "rvoip_core.listener_detached",
            Self::MessageReceived { .. } => "rvoip_core.message_received",
            Self::MessageSent { .. } => "rvoip_core.message_sent",
            Self::MessageDelivered { .. } => "rvoip_core.message_delivered",
            Self::MessageRead { .. } => "rvoip_core.message_read",
            Self::DtmfReceived { .. } => "rvoip_core.dtmf_received",
            Self::TranscriptTurn { .. } => "rvoip_core.transcript_turn",
            Self::RecordingStarted { .. } => "rvoip_core.recording_started",
            Self::RecordingStopped { .. } => "rvoip_core.recording_stopped",
            Self::RecordingComplete { .. } => "rvoip_core.recording_complete",
            Self::VconReady { .. } => "rvoip_core.vcon_ready",
            Self::VconRedacted { .. } => "rvoip_core.vcon_redacted",
            Self::IdentityAssuranceChanged { .. } => "rvoip_core.identity_assurance_changed",
            Self::RegistrationChanged { .. } => "rvoip_core.registration_changed",
            Self::RegistrationHeartbeat { .. } => "rvoip_core.registration_heartbeat",
            Self::CapacityReport { .. } => "rvoip_core.capacity_report",
            Self::UsageRecord { .. } => "rvoip_core.usage_record",
            Self::Anomaly { .. } => "rvoip_core.anomaly",
            Self::MediaQuality { .. } => "rvoip_core.media_quality",
        }
    }

    /// All rvoip-core event-type strings, in declaration order. Used by
    /// `EventTypeRegistry::register_builtin_types` to register every variant.
    pub const ALL_EVENT_TYPES: &'static [EventTypeId] = &[
        "rvoip_core.conversation_opened",
        "rvoip_core.conversation_closed",
        "rvoip_core.session_started",
        "rvoip_core.session_ended",
        "rvoip_core.session_failed",
        "rvoip_core.connection_inbound",
        "rvoip_core.connection_outbound",
        "rvoip_core.connection_connected",
        "rvoip_core.connection_progress",
        "rvoip_core.connection_ended",
        "rvoip_core.connection_failed",
        "rvoip_core.connections_bridged",
        "rvoip_core.connections_unbridged",
        "rvoip_core.connection_transferred",
        "rvoip_core.participant_joined",
        "rvoip_core.participant_left",
        "rvoip_core.ai_attached",
        "rvoip_core.ai_detached",
        "rvoip_core.listener_attached",
        "rvoip_core.listener_detached",
        "rvoip_core.message_received",
        "rvoip_core.message_sent",
        "rvoip_core.message_delivered",
        "rvoip_core.message_read",
        "rvoip_core.dtmf_received",
        "rvoip_core.transcript_turn",
        "rvoip_core.recording_started",
        "rvoip_core.recording_stopped",
        "rvoip_core.recording_complete",
        "rvoip_core.vcon_ready",
        "rvoip_core.vcon_redacted",
        "rvoip_core.identity_assurance_changed",
        "rvoip_core.registration_changed",
        "rvoip_core.registration_heartbeat",
        "rvoip_core.capacity_report",
        "rvoip_core.usage_record",
        "rvoip_core.anomaly",
        "rvoip_core.media_quality",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_identification() {
        let event = RvoipCrossCrateEvent::initiate_call(
            "test_session".to_string(),
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
            None,
        );

        assert_eq!(CrossCrateEvent::event_type(&event), "session_to_dialog");
        assert_eq!(event.source_plane(), PlaneType::Signaling);
        assert_eq!(event.target_plane(), PlaneType::Signaling);
        assert_eq!(event.priority(), EventPriority::High);
    }

    #[test]
    fn test_event_serialization() {
        let event = RvoipCrossCrateEvent::call_state_changed(
            "test_session".to_string(),
            CallState::Active,
            None,
        );

        // Test that events can be serialized and deserialized
        let serialized = serde_json::to_string(&event).unwrap();
        let deserialized: RvoipCrossCrateEvent = serde_json::from_str(&serialized).unwrap();

        assert_eq!(
            CrossCrateEvent::event_type(&deserialized),
            CrossCrateEvent::event_type(&event)
        );
    }

    #[test]
    fn sip_trace_redacts_authorization_headers() {
        let raw = concat!(
            "INVITE sip:bob@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 127.0.0.1:5060\r\n",
            "Authorization: Digest username=\"alice\", response=\"secret\"\r\n",
            "Proxy-Authorization: Digest username=\"alice\", response=\"proxy-secret\"\r\n",
            "Call-ID: call-1\r\n",
            "\r\n",
            "body"
        );

        let redacted = redact_sip_message(raw);

        assert!(redacted.contains("Authorization: <redacted>"));
        assert!(redacted.contains("Proxy-Authorization: <redacted>"));
        assert!(redacted.contains("Via: SIP/2.0/UDP 127.0.0.1:5060"));
        assert!(redacted.contains("body"));
        assert!(!redacted.contains("secret"));
        assert!(!redacted.contains("proxy-secret"));
    }

    #[test]
    fn sip_trace_format_respects_no_redact_and_no_body() {
        let raw = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Authorization: Digest response=\"secret\"\r\n",
            "\r\n",
            "private body"
        );
        let config = SipTraceConfig {
            enabled: true,
            capacity: 4,
            redact_sensitive_headers: false,
            include_body: false,
        };

        let (message, truncated) = format_sip_trace_message(raw, &config);

        assert!(!truncated);
        assert!(message.contains("Authorization: Digest response=\"secret\""));
        assert!(!message.contains("private body"));
    }
}
