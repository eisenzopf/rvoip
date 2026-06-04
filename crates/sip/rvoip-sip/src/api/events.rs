//! Typed application events emitted by `rvoip-sip`.
//!
//! [`Event`] is the common event contract used by [`StreamPeer`], per-call
//! [`SessionHandle`](crate::SessionHandle) receivers, and direct
//! [`UnifiedCoordinator`](crate::UnifiedCoordinator) subscribers. Events are
//! translated from lower-level dialog/media notifications into
//! application-facing call, registration, transfer, NOTIFY, and media events.
//! Helper methods provide typed views over compatibility fields such as REFER
//! transfer kind and NOTIFY subscription state.
//!
//! [`StreamPeer`]: crate::StreamPeer

use crate::api::dialog_package::{DialogInfo, DialogInfoDocument};
use crate::state_table::types::SessionId;
pub use rvoip_infra_common::events::cross_crate::{SipTraceConfig, SipTraceDirection};
use rvoip_sip_core::types::sdp::CryptoSuite;

/// Type alias for call ID (same as SessionId)
pub type CallId = SessionId;

/// Public SIP trace event emitted when [`SipTraceConfig::enabled`] is true.
///
/// `raw_message` is the rendered on-wire bytes (after optional header
/// redaction and body stripping). Pass it through
/// [`rvoip_sip_core::parse_message`] to get a typed
/// [`rvoip_sip_core::Message`] back if the consumer wants to inspect headers
/// programmatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipTrace {
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
    /// rvoip-sip session id after mapping, when known.
    pub session_id: Option<CallId>,
    /// Redacted, optionally body-stripped SIP message text.
    pub raw_message: String,
    /// Original rendered message byte length before redaction/body stripping/truncation.
    pub original_len: usize,
    /// Whether `raw_message` was truncated for bounded diagnostics.
    pub truncated: bool,
    /// Whether sensitive headers were redacted.
    pub redacted: bool,
}

/// Typed classification for REFER transfer requests.
///
/// The wire-facing `Event::ReferReceived::transfer_type` field remains a
/// string for compatibility. Use [`Event::transfer_kind`] when application
/// code wants a typed view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    /// Standard blind transfer REFER.
    Blind,
    /// REFER carrying attended-transfer context such as `Replaces`.
    Attended,
    /// Unrecognized or vendor-specific transfer flavor.
    Unknown,
}

/// Evidence that a transfer target actually progressed beyond REFER receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferTargetEvidence {
    /// A REFER `message/sipfrag` produced provisional target progress before
    /// the final successful sipfrag.
    ReferProgressThenFinal {
        /// Status code from the provisional progress sipfrag.
        progress_status_code: u16,
        /// Reason phrase from the provisional progress sipfrag.
        progress_reason: String,
        /// Status code from the final successful sipfrag.
        final_status_code: u16,
        /// Reason phrase from the final successful sipfrag.
        final_reason: String,
    },
    /// The target leg is local to this coordinator and reached answered state.
    LocalTargetLeg {
        /// Session identifier of the local target leg.
        call_id: CallId,
    },
    /// An RFC 4235 dialog-package NOTIFY reported matching target state.
    DialogPackage {
        /// Dialog state reported by the dialog-package NOTIFY.
        dialog: DialogInfo,
    },
}

impl TransferKind {
    /// Convert the raw transfer type field into a typed classification.
    pub fn from_header_value(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "blind" => Self::Blind,
            "attended" => Self::Attended,
            _ => Self::Unknown,
        }
    }

    /// Stable lowercase label for logs and UI display.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blind => "blind",
            Self::Attended => "attended",
            Self::Unknown => "unknown",
        }
    }
}

/// Parsed view of a `Subscription-State` header.
///
/// This intentionally preserves the raw header value while extracting the
/// common `state`, `expires`, and `reason` parameters. Use
/// [`Event::subscription_state`] to parse a NOTIFY event on demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionState {
    /// Primary state token, such as `active`, `pending`, or `terminated`.
    pub state: String,
    /// Parsed `expires` parameter, if present and numeric.
    pub expires: Option<u32>,
    /// Parsed `reason` parameter, if present.
    pub reason: Option<String>,
    /// Original header value.
    pub raw: String,
}

impl SubscriptionState {
    /// Parse a raw `Subscription-State` header value.
    pub fn parse(raw: impl Into<String>) -> Self {
        let raw = raw.into();
        let mut parts = raw.split(';').map(str::trim);
        let state = parts.next().unwrap_or_default().to_string();
        let mut expires = None;
        let mut reason = None;

        for part in parts {
            if let Some(value) = part.strip_prefix("expires=") {
                expires = value.parse::<u32>().ok();
            } else if let Some(value) = part.strip_prefix("reason=") {
                reason = Some(value.to_string());
            }
        }

        Self {
            state,
            expires,
            reason,
            raw,
        }
    }
}

/// Media-security keying mechanism negotiated for a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSecurityKeying {
    /// SDP Security Descriptions (RFC 4568).
    Sdes,
}

/// RTP profile negotiated for protected media.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaSecurityProfile {
    /// Secure RTP Audio/Video Profile (`RTP/SAVP`).
    RtpSavp,
}

/// Current negotiated media-security state for a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaSecurityState {
    /// Keying mechanism used to derive SRTP contexts.
    pub keying: MediaSecurityKeying,
    /// Negotiated SDES crypto suite.
    pub suite: CryptoSuite,
    /// RTP profile used by the negotiated media stream.
    pub profile: MediaSecurityProfile,
    /// Whether SRTP send/receive contexts have been installed in media-core.
    pub contexts_installed: bool,
}

/// Typed session events delivered to applications.
///
/// These events are published by the state machine and adapters when SIP,
/// media, registration, or transfer activity occurs. Use
/// [`Event::call_id`] to route per-call events, or one of the `is_*`
/// helpers to classify events in generic event loops.
#[derive(Debug, Clone)]
pub enum Event {
    // ===== Call Lifecycle Events =====
    /// Incoming call received
    ///
    /// The state machine has already sent 180 Ringing when
    /// `Config::auto_180_ringing` is enabled. Developer must call
    /// `accept()` or `reject()` to complete the call handling.
    IncomingCall {
        /// Session identifier assigned to this incoming INVITE.
        call_id: CallId,
        /// Caller URI from the SIP `From` header.
        from: String,
        /// Called URI from the SIP `To` or request URI context.
        to: String,
        /// Remote SDP offer, if the INVITE contained one.
        sdp: Option<String>,
    },

    /// Call was answered (200 OK received for outgoing call)
    CallAnswered {
        /// Session identifier for the answered call.
        call_id: CallId,
        /// SDP answer received from the remote peer, if present.
        sdp: Option<String>,
    },

    /// Provisional call progress response received for an outgoing call.
    ///
    /// Emitted for SIP 1xx responses such as `180 Ringing` and
    /// `183 Session Progress`. The state machine still maintains
    /// `CallState::Ringing` / `CallState::EarlyMedia`, but applications can
    /// observe the actual response code, phrase, and early-media SDP here
    /// without polling state.
    CallProgress {
        /// Session identifier for the call.
        call_id: CallId,
        /// SIP provisional status code.
        status_code: u16,
        /// SIP reason phrase.
        reason: String,
        /// SDP body carried by the provisional response, if present.
        sdp: Option<String>,
    },

    /// Call ended (BYE sent/received)
    CallEnded {
        /// Session identifier for the ended call.
        call_id: CallId,
        /// Human-readable teardown reason.
        reason: String,
    },

    /// Call failed (4xx/5xx response or timeout)
    CallFailed {
        /// Session identifier for the failed call.
        call_id: CallId,
        /// SIP status code or synthesized failure code.
        status_code: u16,
        /// Human-readable failure reason.
        reason: String,
    },

    /// SIP_API_DESIGN_2 Phase A — typed inspection of every inbound 1xx
    /// provisional response. Carries an [`crate::api::incoming::IncomingResponse`] so B2BUA /
    /// SBC code can inspect `Contact:`, `Allow:`, `Supported:`,
    /// `Server:`, RFC 3262 reliability markers, and any custom headers
    /// the upstream sent before mirroring them to the downstream 1xx.
    /// Fires alongside the legacy [`Event::CallProgress`] variant; new
    /// code subscribes to the detailed form.
    CallProgressDetailed(crate::api::incoming::IncomingResponse),

    /// SIP_API_DESIGN_2 Phase A — typed inspection of the inbound 200 OK
    /// that established a call. Use for downstream 200 OK
    /// carry-through (Allow / Supported / Session-Expires).
    CallEstablishedDetailed(crate::api::incoming::IncomingResponse),

    /// SIP_API_DESIGN_2 Phase A — typed inspection of an inbound final
    /// failure response. Use to inspect `Retry-After:`, `Warning:`,
    /// RFC 3326 `Reason:`, and similar fields that the legacy
    /// [`Event::CallFailed`] discards.
    CallFailedDetailed(crate::api::incoming::IncomingResponse),

    /// Caller cancelled before the call was answered (RFC 3261 §15.1.2 —
    /// 487 Request Terminated following CANCEL). Distinct from `CallFailed`
    /// so UIs can render "missed call" rather than "call rejected".
    CallCancelled {
        /// Session identifier for the cancelled incoming call.
        call_id: CallId,
    },

    /// RFC 4028 session timer refresh succeeded (UPDATE or re-INVITE
    /// round-tripped). Emitted once per successful refresh — applications
    /// can use this to reset connection-health dashboards or log activity.
    SessionRefreshed {
        /// Session identifier for the refreshed dialog.
        call_id: CallId,
        /// Negotiated session expiration interval in seconds.
        expires_secs: u32,
    },

    /// RFC 4028 session-timer refresh failed; the dialog has been torn
    /// down with BYE (§10). Follow-up `CallEnded` will still fire.
    SessionRefreshFailed {
        /// Session identifier for the dialog whose refresh failed.
        call_id: CallId,
        /// Human-readable refresh failure reason.
        reason: String,
    },

    /// RFC 3261 §22.2 — server challenged our INVITE with 401/407 and we're
    /// about to retry with a digest authorization header. Informational; no
    /// action required from the app. If the retry fails (wrong credentials
    /// or retry cap exceeded), `CallFailed` follows.
    CallAuthRetrying {
        /// Session identifier for the challenged outgoing call.
        call_id: CallId,
        /// 401 or 407.
        status_code: u16,
        /// Digest realm the server asked us to authenticate against.
        realm: String,
    },

    // ===== Transfer Events =====
    /// REFER request received
    ///
    /// Callback handlers may accept or reject the REFER through their return
    /// value. Stream/unified users can call `accept_refer` or `reject_refer`;
    /// if they do nothing, rvoip-sip preserves the legacy behavior and
    /// accepts the REFER after a short grace period.
    ReferReceived {
        /// Session identifier for the dialog that received REFER.
        call_id: CallId,
        /// Raw `Refer-To` target URI.
        refer_to: String,
        /// Optional `Referred-By` header value.
        referred_by: Option<String>,
        /// Optional `Replaces` parameter/header value for attended transfer.
        replaces: Option<String>,
        /// Dialog-core transaction ID used to correlate REFER response/NOTIFY.
        transaction_id: String, // For NOTIFY correlation
        /// Raw transfer flavor. Prefer [`Event::transfer_kind`] for typed
        /// classification.
        transfer_type: String, // "blind" or "attended"
        /// SIP_API_DESIGN_2 Phase E: typed `IncomingRequest` view of
        /// the inbound REFER. Carries every header on the request
        /// (custom routing hints, Target-Dialog per RFC 4538, etc.).
        /// `None` for legacy publish sites that have not been migrated
        /// yet.
        request: Option<crate::api::incoming::IncomingRequest>,
    },

    /// Transfer accepted by recipient
    TransferAccepted {
        /// Session identifier for the call whose REFER was accepted.
        call_id: CallId,
        /// Target URI from the accepted REFER.
        refer_to: String,
    },

    /// Terminal successful REFER NOTIFY received.
    ///
    /// This means the REFER subscription reported a final 2xx sipfrag for the
    /// referenced INVITE. It does not prove that a replacement call later
    /// remained up or was torn down.
    ReferCompleted {
        /// Session identifier for the REFER subscription/dialog.
        call_id: CallId,
        /// Transfer target URI, when known.
        target: String,
        /// Final 2xx status code from the sipfrag.
        status_code: u16,
        /// Final reason phrase from the sipfrag.
        reason: String,
    },

    /// Transfer failed
    TransferFailed {
        /// Session identifier for the failed transfer.
        call_id: CallId,
        /// Human-readable failure reason.
        reason: String,
        /// SIP status code reported by REFER/NOTIFY handling.
        status_code: u16,
    },

    /// REFER progress update from a `message/sipfrag` NOTIFY.
    ReferProgress {
        /// Session identifier for the REFER subscription/dialog.
        call_id: CallId,
        /// SIP status code from the progress NOTIFY sipfrag.
        status_code: u16,
        /// Reason phrase from the progress NOTIFY sipfrag.
        reason: String,
    },

    /// Parsed REFER NOTIFY status surfaced before derived transfer events.
    ///
    /// This preserves the PBX-specific REFER subscription report so
    /// applications can distinguish an immediate terminal NOTIFY from real
    /// target progress.
    ReferNotify {
        /// Session identifier for the REFER subscription/dialog.
        call_id: CallId,
        /// SIP status code parsed from the `message/sipfrag` body.
        status_code: u16,
        /// Reason phrase parsed from the `message/sipfrag` body.
        reason: String,
        /// Parsed `Subscription-State`, if the NOTIFY carried one.
        subscription_state: Option<SubscriptionState>,
        /// Raw NOTIFY body, if any.
        body: Option<String>,
    },

    /// Evidence that the transfer target answered.
    TransferTargetAnswered {
        /// Session identifier of the transferring (REFER-issuing) call.
        transfer_call_id: CallId,
        /// URI of the transfer target that answered.
        target_uri: String,
        /// How the target's answer was observed.
        evidence: TransferTargetEvidence,
    },

    /// RFC 4235 observed a replacement dialog that appears related to a transfer.
    TransferReplacementDialogObserved {
        /// Session identifier of the transferring call.
        transfer_call_id: CallId,
        /// The observed replacement dialog's state.
        dialog: DialogInfo,
    },

    /// RFC 4235 or local target-leg evidence observed replacement dialog teardown.
    TransferReplacementDialogTerminated {
        /// Session identifier of the transferring call.
        transfer_call_id: CallId,
        /// The replacement dialog's final state.
        dialog: DialogInfo,
        /// Teardown reason, when reported.
        reason: Option<String>,
    },

    // ===== Subscription / NOTIFY =====
    /// Inbound NOTIFY surfaced to the application (RFC 6665).
    ///
    /// Fires for every NOTIFY received on any event package — REFER
    /// progress, dialog, presence, message-summary, etc. The session
    /// layer does not interpret the body; if `event_package == "refer"`
    /// and `content_type` is `message/sipfrag`, `ReferNotify` plus the
    /// derived `ReferProgress` / `ReferCompleted` / `TransferFailed` events
    /// are also emitted with the parsed status line.
    NotifyReceived {
        /// Session identifier for the dialog that received NOTIFY.
        call_id: CallId,
        /// SIP `Event` package name.
        event_package: String,
        /// Raw `Subscription-State:` header value (unparsed).
        subscription_state: Option<String>,
        /// Raw `Content-Type:` header value.
        content_type: Option<String>,
        /// NOTIFY body, if any.
        body: Option<String>,
        /// SIP_API_DESIGN_2 Phase E: typed view of the inbound NOTIFY
        /// for B2BUA carry-through / generic header inspection. `None`
        /// for legacy publish sites that have not been migrated yet.
        request: Option<crate::api::incoming::IncomingRequest>,
    },

    /// SIP_API_DESIGN_2 Phase E — inbound in-dialog INFO (RFC 6086).
    /// Today's stack drops INFO at the dialog layer; this variant
    /// surfaces it to applications so SIP-INFO DTMF, fax flow control,
    /// and other application-layer signalling can be observed.
    InfoReceived {
        /// Session identifier for the dialog that received INFO.
        call_id: CallId,
        /// Typed `IncomingRequest` view (raw INFO bytes re-parsed by
        /// the receiving handler).
        request: crate::api::incoming::IncomingRequest,
    },

    /// SIP_API_DESIGN_2 Phase E — inbound in-dialog MESSAGE
    /// (RFC 3428). Distinct from the out-of-dialog `MessageDelivered`
    /// confirmation — this is *receiving* a MESSAGE.
    MessageReceived {
        /// Session identifier for the dialog that received MESSAGE.
        call_id: CallId,
        /// Typed `IncomingRequest` view.
        request: crate::api::incoming::IncomingRequest,
    },

    /// SIP_API_DESIGN_2 Phase E — inbound OPTIONS (RFC 3261 §11).
    /// `call_id` is `None` when the OPTIONS arrived out-of-dialog
    /// (capability query against the AOR).
    OptionsReceived {
        /// Session identifier for the dialog that received OPTIONS,
        /// when one exists.
        call_id: Option<CallId>,
        /// Typed `IncomingRequest` view.
        request: crate::api::incoming::IncomingRequest,
    },

    /// SIP_API_DESIGN_2 Phase E — inbound UPDATE (RFC 3311). This
    /// fires alongside the legacy hold/resume state transitions that
    /// run inside the state machine; subscribe to this variant for
    /// header-level inspection (Session-Expires, RFC 6086 INFO over
    /// UPDATE, custom X-* hints).
    UpdateReceived {
        /// Session identifier for the dialog that received UPDATE.
        call_id: CallId,
        /// Typed `IncomingRequest` view.
        request: crate::api::incoming::IncomingRequest,
    },

    /// SIP_API_DESIGN_2 Phase D — inbound REGISTER (RFC 3261 §10).
    /// Surfaces the typed `IncomingRegister` view so registrar
    /// applications can author the response via `accept_builder` /
    /// `challenge_builder` / `reject_builder` with Service-Route /
    /// Path / P-Associated-URI under their full control.
    IncomingRegister {
        /// Typed `IncomingRegister` view of the inbound REGISTER.
        register: crate::api::incoming::IncomingRegister,
    },

    /// Parsed RFC 4235 dialog-package NOTIFY.
    DialogPackageNotify {
        /// Session identifier of the dialog-package subscription.
        subscription_id: CallId,
        /// `entity` attribute of the dialog-info document, when present.
        entity: Option<String>,
        /// `version` attribute of the dialog-info document, when present.
        version: Option<u32>,
        /// Per-dialog states reported by this NOTIFY.
        dialogs: Vec<DialogInfo>,
        /// The full parsed dialog-info document.
        document: DialogInfoDocument,
    },

    /// Derived per-dialog state transition from an RFC 4235 NOTIFY.
    DialogStateChanged {
        /// Session identifier of the dialog-package subscription.
        subscription_id: CallId,
        /// The dialog whose state changed.
        dialog: DialogInfo,
    },

    // ===== Call State Events =====
    /// Local hold was accepted by the remote peer.
    ///
    /// Emitted after the hold re-INVITE/answer exchange succeeds.
    CallOnHold {
        /// Session identifier for the held call.
        call_id: CallId,
    },

    /// Local resume was accepted by the remote peer.
    ///
    /// Emitted after the resume re-INVITE/answer exchange succeeds.
    CallResumed {
        /// Session identifier for the resumed call.
        call_id: CallId,
    },

    /// The remote peer placed this call on hold with a mid-call offer.
    RemoteCallOnHold {
        /// Session identifier for the remotely held call.
        call_id: CallId,
    },

    /// The remote peer resumed this call with a mid-call offer.
    RemoteCallResumed {
        /// Session identifier for the remotely resumed call.
        call_id: CallId,
    },

    /// Call was muted locally
    CallMuted {
        /// Session identifier for the muted call.
        call_id: CallId,
    },

    /// Call was unmuted locally
    CallUnmuted {
        /// Session identifier for the unmuted call.
        call_id: CallId,
    },

    // ===== Media Events =====
    /// DTMF digit received
    DtmfReceived {
        /// Session identifier for the call that received DTMF.
        call_id: CallId,
        /// Received digit.
        digit: char,
    },

    /// Media quality changed
    MediaQualityChanged {
        /// Session identifier for the media stream.
        call_id: CallId,
        /// Packet loss percentage, rounded to an integer.
        packet_loss_percent: u32,
        /// Jitter in milliseconds, rounded to an integer.
        jitter_ms: u32,
    },

    /// SRTP media security was negotiated and installed.
    MediaSecurityNegotiated {
        /// Session identifier for the protected media stream.
        call_id: CallId,
        /// Keying mechanism used to derive SRTP contexts.
        keying: MediaSecurityKeying,
        /// Negotiated SDES crypto suite.
        suite: CryptoSuite,
        /// RTP profile used by the negotiated media stream.
        profile: MediaSecurityProfile,
        /// Whether SRTP send/receive contexts have been installed in media-core.
        contexts_installed: bool,
    },

    // ===== Registration Events =====
    /// Registration successful.
    ///
    /// `expires` is the registrar-accepted expiry, not necessarily the value
    /// requested by the application. Use
    /// [`UnifiedCoordinator::registration_info`](crate::UnifiedCoordinator::registration_info)
    /// for refresh timing, Service-Route, GRUU, and failure metadata.
    RegistrationSuccess {
        /// Registrar URI used for the REGISTER.
        registrar: String,
        /// Expiration interval accepted by the registrar.
        expires: u32,
        /// Contact URI that was registered.
        contact: String,
    },

    /// Registration failed.
    ///
    /// Final failure after any supported retry path, such as auth retry
    /// or 423 Interval Too Brief retry.
    RegistrationFailed {
        /// Registrar URI used for the failed REGISTER.
        registrar: String,
        /// SIP status code returned by the registrar.
        status_code: u16,
        /// Human-readable failure reason.
        reason: String,
    },

    /// Unregistration successful.
    ///
    /// Automatic refresh for the registration has been aborted.
    UnregistrationSuccess {
        /// Registrar URI used for the unregistration.
        registrar: String,
    },

    /// Unregistration failed.
    UnregistrationFailed {
        /// Registrar URI used for the failed unregistration.
        registrar: String,
        /// Human-readable failure reason.
        reason: String,
    },

    // ===== Diagnostics Events =====
    /// SIP message observed at the transport boundary.
    SipTrace(SipTrace),

    // ===== Error Events =====
    /// Network error occurred
    NetworkError {
        /// Session identifier, if the transport error can be tied to one call.
        call_id: Option<CallId>,
        /// Human-readable error text.
        error: String,
    },

    /// Authentication required (401/407 response)
    AuthenticationRequired {
        /// Session identifier for the challenged request.
        call_id: CallId,
        /// Digest-auth realm from the challenge.
        realm: String,
    },
}

impl Event {
    /// Get the call ID associated with this event (if any)
    pub fn call_id(&self) -> Option<&CallId> {
        match self {
            Event::IncomingCall { call_id, .. }
            | Event::CallAnswered { call_id, .. }
            | Event::CallProgress { call_id, .. }
            | Event::CallEnded { call_id, .. }
            | Event::CallFailed { call_id, .. }
            | Event::CallCancelled { call_id, .. }
            | Event::SessionRefreshed { call_id, .. }
            | Event::SessionRefreshFailed { call_id, .. }
            | Event::CallAuthRetrying { call_id, .. }
            | Event::ReferReceived { call_id, .. }
            | Event::TransferAccepted { call_id, .. }
            | Event::TransferFailed { call_id, .. }
            | Event::ReferProgress { call_id, .. }
            | Event::ReferNotify { call_id, .. }
            | Event::ReferCompleted { call_id, .. }
            | Event::CallOnHold { call_id, .. }
            | Event::CallResumed { call_id, .. }
            | Event::RemoteCallOnHold { call_id, .. }
            | Event::RemoteCallResumed { call_id, .. }
            | Event::CallMuted { call_id, .. }
            | Event::CallUnmuted { call_id, .. }
            | Event::DtmfReceived { call_id, .. }
            | Event::MediaQualityChanged { call_id, .. }
            | Event::MediaSecurityNegotiated { call_id, .. }
            | Event::NotifyReceived { call_id, .. }
            | Event::AuthenticationRequired { call_id, .. } => Some(call_id),
            Event::TransferTargetAnswered {
                transfer_call_id, ..
            }
            | Event::TransferReplacementDialogObserved {
                transfer_call_id, ..
            }
            | Event::TransferReplacementDialogTerminated {
                transfer_call_id, ..
            } => Some(transfer_call_id),
            Event::DialogPackageNotify {
                subscription_id, ..
            }
            | Event::DialogStateChanged {
                subscription_id, ..
            } => Some(subscription_id),
            Event::SipTrace(trace) => trace.session_id.as_ref(),
            Event::NetworkError { call_id, .. } => call_id.as_ref(),
            Event::CallProgressDetailed(r)
            | Event::CallEstablishedDetailed(r)
            | Event::CallFailedDetailed(r) => Some(&r.call_id),
            Event::InfoReceived { call_id, .. }
            | Event::MessageReceived { call_id, .. }
            | Event::UpdateReceived { call_id, .. } => Some(call_id),
            Event::OptionsReceived { call_id, .. } => call_id.as_ref(),
            // Registration events don't have call_id
            Event::RegistrationSuccess { .. }
            | Event::RegistrationFailed { .. }
            | Event::UnregistrationSuccess { .. }
            | Event::UnregistrationFailed { .. }
            | Event::IncomingRegister { .. } => None,
        }
    }

    /// Check if this is a call lifecycle event
    pub fn is_call_event(&self) -> bool {
        matches!(
            self,
            Event::IncomingCall { .. }
                | Event::CallAnswered { .. }
                | Event::CallProgress { .. }
                | Event::CallEnded { .. }
                | Event::CallFailed { .. }
                | Event::CallCancelled { .. }
                | Event::CallProgressDetailed(_)
                | Event::CallEstablishedDetailed(_)
                | Event::CallFailedDetailed(_)
                | Event::InfoReceived { .. }
                | Event::MessageReceived { .. }
                | Event::OptionsReceived { .. }
                | Event::UpdateReceived { .. }
        )
    }

    /// Check if this is a call state/control event
    pub fn is_call_state_event(&self) -> bool {
        matches!(
            self,
            Event::CallOnHold { .. }
                | Event::CallResumed { .. }
                | Event::RemoteCallOnHold { .. }
                | Event::RemoteCallResumed { .. }
                | Event::CallMuted { .. }
                | Event::CallUnmuted { .. }
        )
    }

    /// Check if this is a transfer-related event
    pub fn is_transfer_event(&self) -> bool {
        matches!(
            self,
            Event::ReferReceived { .. }
                | Event::TransferAccepted { .. }
                | Event::ReferCompleted { .. }
                | Event::TransferFailed { .. }
                | Event::ReferProgress { .. }
                | Event::ReferNotify { .. }
                | Event::TransferTargetAnswered { .. }
                | Event::TransferReplacementDialogObserved { .. }
                | Event::TransferReplacementDialogTerminated { .. }
        )
    }

    /// Check if this is a media-related event
    pub fn is_media_event(&self) -> bool {
        matches!(
            self,
            Event::DtmfReceived { .. }
                | Event::MediaQualityChanged { .. }
                | Event::MediaSecurityNegotiated { .. }
        )
    }

    /// Typed transfer kind for `ReferReceived`.
    ///
    /// Returns `None` for non-REFER events.
    pub fn transfer_kind(&self) -> Option<TransferKind> {
        match self {
            Event::ReferReceived { transfer_type, .. } => {
                Some(TransferKind::from_header_value(transfer_type))
            }
            _ => None,
        }
    }

    /// Parsed `Subscription-State` for `NotifyReceived`.
    ///
    /// Returns `None` when the event is not NOTIFY or the header was absent.
    pub fn subscription_state(&self) -> Option<SubscriptionState> {
        match self {
            Event::NotifyReceived {
                subscription_state: Some(raw),
                ..
            } => Some(SubscriptionState::parse(raw.clone())),
            Event::ReferNotify {
                subscription_state: Some(parsed),
                ..
            } => Some(parsed.clone()),
            _ => None,
        }
    }
}
