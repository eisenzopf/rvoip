use crate::state_table::{CallId, DialogId, MediaSessionId, SessionId};
use rvoip_sip_dialog::transaction::TransactionKey;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::history::{HistoryConfig, SessionHistory, TransitionRecord};
use crate::api::events::MediaSecurityState;
use crate::state_table::{ConditionUpdates, Role};
use crate::types::{CallState, MediaDirection};

/// Negotiated media configuration
#[derive(Clone, Serialize, Deserialize)]
pub struct NegotiatedConfig {
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u8,
}

impl fmt::Debug for NegotiatedConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NegotiatedConfig")
            .field("local_address_present", &true)
            .field("remote_address_present", &true)
            .field("codec_bytes", &self.codec.len())
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .finish()
    }
}

/// Kind of mid-dialog re-INVITE that was in flight when a 491 Request
/// Pending arrived — captured so `ScheduleReinviteRetry` can re-issue the
/// correct operation after the RFC 3261 §14.1 random backoff.
#[derive(Clone, PartialEq, Eq)]
pub enum PendingReinvite {
    Hold,
    Resume,
    /// Generic SDP update with a specific offer (codec change, etc.).
    SdpUpdate(String),
}

impl fmt::Debug for PendingReinvite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Hold => "Hold",
            Self::Resume => "Resume",
            Self::SdpUpdate(_) => "SdpUpdate",
        })
    }
}

/// Transfer state tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    None,
    TransferInitiated,
    TransferCompleted,
}

/// Complete state of a session.
///
/// `Debug` reports operational state, counts, and presence flags without
/// formatting retained SIP URIs, SDP, authentication material, headers, or
/// message bodies.
#[derive(Clone)]
pub struct SessionState {
    // Identity
    pub session_id: SessionId,
    pub role: Role,

    // Current state
    pub call_state: CallState,
    pub entered_state_at: Instant,

    // Readiness conditions (the 3 flags)
    pub dialog_established: bool,
    pub media_session_ready: bool,
    pub sdp_negotiated: bool,

    // Track if call established was triggered
    pub call_established_triggered: bool,

    // SDP data
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub negotiated_config: Option<NegotiatedConfig>,
    /// Negotiated media security, populated after SRTP contexts install.
    pub media_security: Option<MediaSecurityState>,
    /// Stable numeric SDP origin session id used in the `o=` line for
    /// every local offer/answer on this session.
    pub sdp_origin_session_id: String,
    /// Monotonic SDP origin version. Incremented for each locally generated
    /// SDP body that can be placed on the wire.
    pub sdp_origin_version: u64,
    /// Current local media direction from our perspective.
    pub local_media_direction: MediaDirection,
    /// Current remote offer direction from the peer's perspective.
    pub remote_media_direction: MediaDirection,

    // Related IDs
    pub dialog_id: Option<DialogId>,
    pub media_session_id: Option<MediaSessionId>,
    pub call_id: Option<CallId>,
    /// Inbound INVITE server transaction captured during UAS setup so the
    /// final 200 OK can avoid rediscovering the pending transaction.
    pub pending_inbound_invite_transaction_id: Option<TransactionKey>,
    /// Session-layer receive timestamp for Config-owned first-response timing.
    pub incoming_invite_received_at: Option<Instant>,

    // SIP URIs
    pub local_uri: Option<String>,  // From URI for UAC, To URI for UAS
    pub remote_uri: Option<String>, // To URI for UAC, From URI for UAS

    // Store last 200 OK response for ACK
    pub last_200_ok: Option<Vec<u8>>, // Serialized response

    // Bridging information (for peer-to-peer conferencing)
    pub bridged_to: Option<SessionId>, // Session this is bridged to

    // Conference information
    pub conference_mixer_id: Option<MediaSessionId>, // Mixer ID if hosting conference
    pub transfer_target: Option<String>,             // Target for transfers
    pub dtmf_digits: Option<String>,                 // DTMF digits to send

    // Rejection details captured from RejectCall event for use by SendRejectResponse
    pub reject_status: Option<u16>,
    pub reject_reason: Option<String>,
    /// SIP_API_DESIGN_2 §3.4 — application headers staged by
    /// [`RejectBuilder`](crate::api::respond::RejectBuilder) /
    /// [`AuthChallengeBuilder`](crate::api::respond::AuthChallengeBuilder)
    /// to ride on the wire 4xx-6xx response. Set by the builder
    /// before calling `reject_call`; consumed (and cleared) by
    /// `Action::SendRejectResponse`.
    pub reject_response_extras: Option<Vec<rvoip_sip_core::types::TypedHeader>>,

    // RFC 3261 §8.1.3.4 / §21.3 — redirect details captured from a local
    // UAS-side RedirectCall event, used by `SendRedirectResponse`. The status
    // must be 3xx; contacts are the URIs we'll advertise in the `Contact:`
    // header so the UAC can pick one to follow.
    pub redirect_response_status: Option<u16>,
    pub redirect_response_contacts: Vec<String>,

    // Caller-supplied SDP for SendEarlyMedia. Consumed by PrepareEarlyMediaSDP
    // on the way to the reliable 183; None means "auto-negotiate from remote
    // offer" (the usual case for a call-flow-driven ringback).
    pub early_media_sdp: Option<String>,

    // RFC 3261 §22.2 — AuthRequired payload stashed here by the executor
    // (mirrors reject_status pattern). Consumed by StoreAuthChallenge and
    // SendINVITEWithAuth to pick `Authorization` vs `Proxy-Authorization`
    // based on status code. Carried as a tuple to keep the field count low.
    pub pending_auth: Option<(u16, String)>,

    // SIP_API_DESIGN_2 R2 — SIP method of the challenged request,
    // extracted from the AuthRequired event (originally from the
    // response's CSeq:). Consumed by `SendRequestWithAuth` to route
    // the retry to the right per-method dispatcher. Empty string means
    // method-agnostic (legacy publish path); the action falls back to
    // inspecting which `pending_*_options` stash is set.
    pub pending_auth_method: Option<String>,

    // Post-send outbound transport context for the challenged request, when
    // dialog-core could report it. Used by UAC auth retry policy so Basic and
    // Bearer decisions are based on the hop that actually carried the first
    // request.
    pub pending_auth_transport: Option<crate::auth::SipTransportSecurityContext>,

    // SIP_API_DESIGN_2 R2 — retry cap counter for the generic
    // `SendRequestWithAuth` action. Mirrors `invite_auth_retry_count`
    // but covers BYE / REFER / NOTIFY / INFO / UPDATE / MESSAGE /
    // OPTIONS / SUBSCRIBE. Capped at 1 (one retry total) to prevent
    // loops when credentials are wrong. The conflict guard ensures
    // only one method has an in-flight request per session so a single
    // counter is sufficient.
    pub request_auth_retry_count: u8,

    // RFC 3261 §22.2 — INVITE auth retry counter, capped at 1 (two attempts
    // total: initial + one authenticated retry). Prevents infinite loops when
    // the server keeps re-challenging with the same nonce.
    pub invite_auth_retry_count: u8,

    // 3xx redirect follow-up state (RFC 3261 §8.1.3.4)
    // Remaining redirect targets to try (first = highest priority); popped
    // from the front by RetryWithContact.
    pub redirect_targets: Vec<String>,
    // Number of redirects followed so far; RFC-recommended cap is 5.
    pub redirect_attempts: u8,

    // 491 Request Pending retry state (RFC 3261 §14.1). Remembers the kind
    // of re-INVITE that was in flight when a 491 was received, so we can
    // re-issue it after the random backoff.
    pub pending_reinvite: Option<PendingReinvite>,
    pub reinvite_retry_attempts: u8,

    // RFC 4028 §6 — 422 Session Interval Too Small retry state. Peer's
    // required `Min-SE` floor is stashed here by the 422 event handler; the
    // retry action reads it to build the bumped `Session-Expires`. Retry
    // counter is capped at 2 to avoid loops when a broken UAS keeps sending
    // 422 regardless of what we pick.
    pub session_timer_min_se: Option<u32>,
    pub session_timer_retry_count: u8,

    // Transfer tracking (blind transfer + REFER-with-Replaces primitive for
    // higher-layer attended-transfer orchestrators). Per-session state only;
    // linking two sessions (consultation + original) is an orchestration
    // concern that lives outside this crate.
    pub transfer_state: TransferState, // Current transfer state
    pub transfer_notify_dialog: Option<DialogId>, // Dialog to send NOTIFY messages to (for blind transfer)

    // Transfer coordination fields
    pub replaces_header: Option<String>, // Replaces header for attended transfer
    pub referred_by: Option<String>,     // Referred-By header from REFER request
    pub refer_transaction_id: Option<String>, // Transaction ID for REFER request (for sending response)
    pub is_transfer_call: bool, // Flag indicating this session is a result of a transfer
    pub transferor_session_id: Option<SessionId>, // Session ID of who sent us the REFER (for NOTIFY messages)
    pub transfer_target_progress_seen: bool, // Whether REFER NOTIFY reported target provisional progress
    pub transfer_target_last_progress: Option<(u16, String)>, // Last provisional target evidence
    pub pending_bye_reason: Option<(String, u16, Option<String>)>, // RFC 3326 Reason for next local BYE

    // ──────────────────────────────────────────────────────────────────
    // SIP_API_DESIGN_2 §7.3 — Pending-options stash lifecycle.
    //
    // Each `pending_<method>_options` slot is set by the matching
    // rvoip-sip builder's `.send()` immediately before the
    // `Action::Send<METHOD>WithOptions` is queued. The state-machine
    // handler reads, dispatches, and clears the slot back to `None`
    // when the transaction reaches a final response (success,
    // terminal failure, or hard timeout). Auth-retry re-reads the
    // same `Arc<XxxRequestOptions>` for the retry transaction; the
    // slot persists across retries until the final response.
    //
    // Set-once / consumed-once: a second `.send()` of the same
    // method on the same session while the slot is occupied returns
    // `Err(SessionError::Conflict { method })`. Different methods on
    // the same session are independent (different slots).
    //
    // On entry to `Terminated`, every `pending_*_options` is set to
    // `None`.
    // ──────────────────────────────────────────────────────────────────
    pub pending_invite_options:
        Option<std::sync::Arc<crate::api::send::outbound_call::OutboundCallOptionsSnapshot>>,
    pub pending_reinvite_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::ReInviteRequestOptions>>,
    pub pending_register_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::RegisterRequestOptions>>,
    pub pending_refer_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::ReferRequestOptions>>,
    pub pending_bye_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::ByeRequestOptions>>,
    pub pending_cancel_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::CancelRequestOptions>>,
    pub pending_notify_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::NotifyRequestOptions>>,
    pub pending_subscribe_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::SubscribeRequestOptions>>,
    pub pending_info_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::InfoRequestOptions>>,
    pub pending_update_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::UpdateRequestOptions>>,
    pub pending_message_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::MessageRequestOptions>>,
    pub pending_options_options:
        Option<std::sync::Arc<rvoip_sip_dialog::api::unified::OptionsRequestOptions>>,

    // Registration fields
    pub registrar_uri: Option<String>, // URI of the registrar server
    pub registration_expires: Option<u32>, // Registration expiry in seconds
    pub registration_contact: Option<String>, // Contact URI for registration
    pub registration_call_id: Option<String>, // Stable Call-ID for this registration lifecycle
    pub registration_cseq: u32,        // Last CSeq used for this registration lifecycle
    pub registration_accepted_expires: Option<u32>, // Registrar-accepted expiry in seconds
    pub registration_registered_at: Option<Instant>, // Time of last successful registration
    pub registration_next_refresh_at: Option<Instant>, // Scheduled automatic refresh time
    pub registration_last_failure: Option<String>, // Last registration failure summary
    pub registration_service_route: Option<Vec<String>>, // Registrar Service-Route URIs
    pub registration_pub_gruu: Option<String>, // Registrar-assigned public GRUU
    pub registration_temp_gruu: Option<String>, // Registrar-assigned temporary GRUU
    pub credentials: Option<crate::types::Credentials>, // User credentials for authentication
    pub auth: Option<crate::auth::SipClientAuth>, // General UAC auth for 401/407 retries
    /// Optional `P-Asserted-Identity` URI (RFC 3325 §9.1) to attach to the
    /// outgoing INVITE for this session. When `Some`, the `SendINVITE` action
    /// routes through `dialog_adapter.send_invite_with_extra_headers` so the
    /// header lands on the very first wire transmission. Carrier trunks
    /// commonly require this for caller-ID assertion.
    pub pai_uri: Option<String>,
    /// Caller-supplied extra typed headers to attach to the very first
    /// outgoing INVITE for this session. Populated by the
    /// `_with_headers` variants on the public API surfaces; consumed by
    /// `Action::SendINVITE`, which appends them to the `extras` vector after
    /// any synthesized `P-Asserted-Identity`. Empty by default — the
    /// outbound-proxy Route prepended inside
    /// `DialogAdapter::send_invite_with_extra_headers` still runs after this,
    /// so a configured outbound proxy stays first on the wire.
    pub extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    pub is_registered: bool, // Whether registration is complete
    pub auth_challenge: Option<crate::auth::DigestChallenge>, // Cached authentication challenge from 401
    pub auth_challenge_raw: Option<String>, // Cached raw challenge for non-Digest auth schemes
    /// Whether the latest cached challenge carried `stale=true`.
    pub auth_challenge_stale: bool,
    /// Previous nonce replaced by the latest cached challenge. Used to allow
    /// exactly one stale-nonce recovery retry with a fresh nonce.
    pub auth_challenge_replaces_nonce: Option<String>,
    pub registration_retry_count: u32, // Number of retries attempted (prevent infinite loops)

    // RFC 7616 §3.4.5 — per-(realm, nonce) digest nonce-count cursor.
    // Each successive request reusing the same nonce increments its
    // entry; when a fresh challenge with a new nonce arrives, a new
    // entry is inserted at 1. Carriers reject `nc` repeats as replays,
    // so this map is the difference between working and broken auth on
    // anything beyond the first 401 retry. Sessions are ephemeral —
    // the map is not persisted across process restart on purpose.
    pub digest_nc: HashMap<(String, String), u32>,

    // Timestamps
    pub created_at: Instant,

    // Optional history tracking
    pub history: Option<SessionHistory>,
}

impl fmt::Debug for SessionState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let pending_option_count = [
            self.pending_invite_options.is_some(),
            self.pending_reinvite_options.is_some(),
            self.pending_register_options.is_some(),
            self.pending_refer_options.is_some(),
            self.pending_bye_options.is_some(),
            self.pending_cancel_options.is_some(),
            self.pending_notify_options.is_some(),
            self.pending_subscribe_options.is_some(),
            self.pending_info_options.is_some(),
            self.pending_update_options.is_some(),
            self.pending_message_options.is_some(),
            self.pending_options_options.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        let pending_reinvite = self.pending_reinvite.as_ref().map(|pending| match pending {
            PendingReinvite::Hold => "hold",
            PendingReinvite::Resume => "resume",
            PendingReinvite::SdpUpdate(_) => "sdp_update",
        });
        let pending_auth_status = self.pending_auth.as_ref().map(|(status, _)| *status);
        let pending_auth_transport_secure = self
            .pending_auth_transport
            .as_ref()
            .map(crate::auth::SipTransportSecurityContext::is_secure);
        let transfer_target_last_status = self
            .transfer_target_last_progress
            .as_ref()
            .map(|(status, _)| *status);
        let media_security_keying = self.media_security.as_ref().map(|state| state.keying);
        let media_security_suite = self.media_security.as_ref().map(|state| state.suite);
        let media_security_profile = self.media_security.as_ref().map(|state| state.profile);
        let media_security_contexts_installed = self
            .media_security
            .as_ref()
            .map(|state| state.contexts_installed);
        let history_total_transitions = self
            .history
            .as_ref()
            .map(|history| history.total_transitions);
        let history_total_errors = self.history.as_ref().map(|history| history.total_errors);

        formatter
            .debug_struct("SessionState")
            .field("session_id", &self.session_id)
            .field("role", &self.role)
            .field("call_state", &self.call_state)
            .field("dialog_established", &self.dialog_established)
            .field("media_session_ready", &self.media_session_ready)
            .field("sdp_negotiated", &self.sdp_negotiated)
            .field(
                "call_established_triggered",
                &self.call_established_triggered,
            )
            .field("local_sdp_present", &self.local_sdp.is_some())
            .field("remote_sdp_present", &self.remote_sdp.is_some())
            .field(
                "negotiated_config_present",
                &self.negotiated_config.is_some(),
            )
            .field("media_security_keying", &media_security_keying)
            .field("media_security_suite", &media_security_suite)
            .field("media_security_profile", &media_security_profile)
            .field(
                "media_security_contexts_installed",
                &media_security_contexts_installed,
            )
            .field("sdp_origin_version", &self.sdp_origin_version)
            .field("local_media_direction", &self.local_media_direction)
            .field("remote_media_direction", &self.remote_media_direction)
            .field("dialog_id_present", &self.dialog_id.is_some())
            .field("media_session_id_present", &self.media_session_id.is_some())
            .field("call_id_present", &self.call_id.is_some())
            .field(
                "pending_inbound_invite_transaction_present",
                &self.pending_inbound_invite_transaction_id.is_some(),
            )
            .field(
                "incoming_invite_received_at_present",
                &self.incoming_invite_received_at.is_some(),
            )
            .field("local_uri_present", &self.local_uri.is_some())
            .field("remote_uri_present", &self.remote_uri.is_some())
            .field(
                "last_200_ok_len",
                &self.last_200_ok.as_ref().map_or(0, Vec::len),
            )
            .field("bridged_to_present", &self.bridged_to.is_some())
            .field(
                "conference_mixer_present",
                &self.conference_mixer_id.is_some(),
            )
            .field("transfer_target_present", &self.transfer_target.is_some())
            .field("dtmf_digits_present", &self.dtmf_digits.is_some())
            .field("reject_status", &self.reject_status)
            .field("reject_reason_present", &self.reject_reason.is_some())
            .field(
                "reject_response_extra_count",
                &self.reject_response_extras.as_ref().map_or(0, Vec::len),
            )
            .field("redirect_response_status", &self.redirect_response_status)
            .field(
                "redirect_response_contact_count",
                &self.redirect_response_contacts.len(),
            )
            .field("early_media_sdp_present", &self.early_media_sdp.is_some())
            .field("pending_auth_status", &pending_auth_status)
            .field(
                "pending_auth_method_present",
                &self.pending_auth_method.is_some(),
            )
            .field(
                "pending_auth_transport_present",
                &self.pending_auth_transport.is_some(),
            )
            .field(
                "pending_auth_transport_secure",
                &pending_auth_transport_secure,
            )
            .field("request_auth_retry_count", &self.request_auth_retry_count)
            .field("invite_auth_retry_count", &self.invite_auth_retry_count)
            .field("redirect_target_count", &self.redirect_targets.len())
            .field("redirect_attempts", &self.redirect_attempts)
            .field("pending_reinvite", &pending_reinvite)
            .field("reinvite_retry_attempts", &self.reinvite_retry_attempts)
            .field("session_timer_min_se", &self.session_timer_min_se)
            .field("session_timer_retry_count", &self.session_timer_retry_count)
            .field("transfer_state", &self.transfer_state)
            .field(
                "transfer_notify_dialog_present",
                &self.transfer_notify_dialog.is_some(),
            )
            .field("replaces_header_present", &self.replaces_header.is_some())
            .field("referred_by_present", &self.referred_by.is_some())
            .field(
                "refer_transaction_id_present",
                &self.refer_transaction_id.is_some(),
            )
            .field("is_transfer_call", &self.is_transfer_call)
            .field(
                "transferor_session_id_present",
                &self.transferor_session_id.is_some(),
            )
            .field(
                "transfer_target_progress_seen",
                &self.transfer_target_progress_seen,
            )
            .field("transfer_target_last_status", &transfer_target_last_status)
            .field(
                "pending_bye_reason_present",
                &self.pending_bye_reason.is_some(),
            )
            .field("pending_option_count", &pending_option_count)
            .field(
                "pending_invite_options_present",
                &self.pending_invite_options.is_some(),
            )
            .field(
                "pending_reinvite_options_present",
                &self.pending_reinvite_options.is_some(),
            )
            .field(
                "pending_register_options_present",
                &self.pending_register_options.is_some(),
            )
            .field(
                "pending_refer_options_present",
                &self.pending_refer_options.is_some(),
            )
            .field(
                "pending_bye_options_present",
                &self.pending_bye_options.is_some(),
            )
            .field(
                "pending_cancel_options_present",
                &self.pending_cancel_options.is_some(),
            )
            .field(
                "pending_notify_options_present",
                &self.pending_notify_options.is_some(),
            )
            .field(
                "pending_subscribe_options_present",
                &self.pending_subscribe_options.is_some(),
            )
            .field(
                "pending_info_options_present",
                &self.pending_info_options.is_some(),
            )
            .field(
                "pending_update_options_present",
                &self.pending_update_options.is_some(),
            )
            .field(
                "pending_message_options_present",
                &self.pending_message_options.is_some(),
            )
            .field(
                "pending_options_options_present",
                &self.pending_options_options.is_some(),
            )
            .field("registrar_uri_present", &self.registrar_uri.is_some())
            .field("registration_expires", &self.registration_expires)
            .field(
                "registration_contact_present",
                &self.registration_contact.is_some(),
            )
            .field(
                "registration_call_id_present",
                &self.registration_call_id.is_some(),
            )
            .field("registration_cseq", &self.registration_cseq)
            .field(
                "registration_accepted_expires",
                &self.registration_accepted_expires,
            )
            .field(
                "registration_registered_at_present",
                &self.registration_registered_at.is_some(),
            )
            .field(
                "registration_next_refresh_at_present",
                &self.registration_next_refresh_at.is_some(),
            )
            .field(
                "registration_last_failure_present",
                &self.registration_last_failure.is_some(),
            )
            .field(
                "registration_service_route_count",
                &self.registration_service_route.as_ref().map_or(0, Vec::len),
            )
            .field(
                "registration_pub_gruu_present",
                &self.registration_pub_gruu.is_some(),
            )
            .field(
                "registration_temp_gruu_present",
                &self.registration_temp_gruu.is_some(),
            )
            .field("credentials_present", &self.credentials.is_some())
            .field("auth_present", &self.auth.is_some())
            .field("pai_uri_present", &self.pai_uri.is_some())
            .field("extra_header_count", &self.extra_headers.len())
            .field("is_registered", &self.is_registered)
            .field("auth_challenge_present", &self.auth_challenge.is_some())
            .field(
                "auth_challenge_raw_present",
                &self.auth_challenge_raw.is_some(),
            )
            .field("auth_challenge_stale", &self.auth_challenge_stale)
            .field(
                "auth_challenge_replaces_nonce_present",
                &self.auth_challenge_replaces_nonce.is_some(),
            )
            .field("registration_retry_count", &self.registration_retry_count)
            .field("digest_nonce_count", &self.digest_nc.len())
            .field("history_present", &self.history.is_some())
            .field("history_total_transitions", &history_total_transitions)
            .field("history_total_errors", &history_total_errors)
            .finish()
    }
}

impl SessionState {
    /// Create a new session state
    pub fn new(session_id: SessionId, role: Role) -> Self {
        let now = Instant::now();
        let sdp_origin_session_id = stable_sdp_origin_session_id(&session_id.0);
        Self {
            session_id,
            role,
            call_state: CallState::Idle,
            entered_state_at: now,
            dialog_established: false,
            media_session_ready: false,
            sdp_negotiated: false,
            call_established_triggered: false,
            local_sdp: None,
            remote_sdp: None,
            negotiated_config: None,
            media_security: None,
            sdp_origin_session_id,
            sdp_origin_version: 0,
            local_media_direction: MediaDirection::SendRecv,
            remote_media_direction: MediaDirection::SendRecv,
            dialog_id: None,
            media_session_id: None,
            call_id: None,
            pending_inbound_invite_transaction_id: None,
            incoming_invite_received_at: None,
            local_uri: None,
            remote_uri: None,
            last_200_ok: None,
            bridged_to: None,
            conference_mixer_id: None,
            transfer_target: None,
            dtmf_digits: None,
            reject_status: None,
            reject_reason: None,
            reject_response_extras: None,
            redirect_response_status: None,
            redirect_response_contacts: Vec::new(),
            early_media_sdp: None,
            pending_auth: None,
            pending_auth_method: None,
            pending_auth_transport: None,
            request_auth_retry_count: 0,
            invite_auth_retry_count: 0,
            redirect_targets: Vec::new(),
            redirect_attempts: 0,
            pending_reinvite: None,
            reinvite_retry_attempts: 0,
            session_timer_min_se: None,
            session_timer_retry_count: 0,
            transfer_state: TransferState::None,
            transfer_notify_dialog: None,
            replaces_header: None,
            referred_by: None,
            refer_transaction_id: None,
            is_transfer_call: false,
            transferor_session_id: None,
            transfer_target_progress_seen: false,
            transfer_target_last_progress: None,
            pending_bye_reason: None,
            // SIP_API_DESIGN_2 §7.3 stash slots — none populated at
            // session creation.
            pending_invite_options: None,
            pending_reinvite_options: None,
            pending_register_options: None,
            pending_refer_options: None,
            pending_bye_options: None,
            pending_cancel_options: None,
            pending_notify_options: None,
            pending_subscribe_options: None,
            pending_info_options: None,
            pending_update_options: None,
            pending_message_options: None,
            pending_options_options: None,
            registrar_uri: None,
            registration_expires: None,
            registration_contact: None,
            registration_call_id: None,
            registration_cseq: 0,
            registration_accepted_expires: None,
            registration_registered_at: None,
            registration_next_refresh_at: None,
            registration_last_failure: None,
            registration_service_route: None,
            registration_pub_gruu: None,
            registration_temp_gruu: None,
            credentials: None,
            auth: None,
            pai_uri: None,
            extra_headers: Vec::new(),
            is_registered: false,
            auth_challenge: None,
            auth_challenge_raw: None,
            auth_challenge_stale: false,
            auth_challenge_replaces_nonce: None,
            registration_retry_count: 0,
            digest_nc: HashMap::new(),
            created_at: now,
            history: None,
        }
    }

    /// Create with history tracking enabled
    pub fn with_history(session_id: SessionId, role: Role, config: HistoryConfig) -> Self {
        let mut state = Self::new(session_id, role);
        state.history = Some(SessionHistory::new(config));
        state
    }

    /// Record a transition in history
    pub fn record_transition(&mut self, record: TransitionRecord) {
        if let Some(ref mut history) = self.history {
            history.record_transition(record);
        }
    }

    /// Transition to a new state
    pub fn transition_to(&mut self, new_state: CallState) {
        if let Some(ref mut history) = self.history {
            use crate::session_store::TransitionRecord;
            use crate::state_table::EventType;
            let now = Instant::now();
            let record = TransitionRecord {
                timestamp: now,
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                sequence: 0, // Will be set by history
                from_state: self.call_state,
                event: EventType::MediaEvent("transition_to".to_string()),
                to_state: Some(new_state),
                guards_evaluated: vec![],
                actions_executed: vec![],
                duration_ms: 0,
                errors: vec![],
                events_published: vec![],
            };
            history.record_transition(record);
        }
        self.call_state = new_state;
        self.entered_state_at = Instant::now();
    }

    /// Apply condition updates from a transition
    pub fn apply_condition_updates(&mut self, updates: &ConditionUpdates) {
        if let Some(value) = updates.dialog_established {
            self.dialog_established = value;
        }
        if let Some(value) = updates.media_session_ready {
            self.media_session_ready = value;
        }
        if let Some(value) = updates.sdp_negotiated {
            self.sdp_negotiated = value;
        }
    }

    /// Check if all readiness conditions are met
    pub fn all_conditions_met(&self) -> bool {
        self.dialog_established && self.media_session_ready && self.sdp_negotiated
    }

    /// Get time spent in current state
    pub fn time_in_state(&self) -> std::time::Duration {
        Instant::now() - self.entered_state_at
    }

    /// Get total session duration
    pub fn session_duration(&self) -> std::time::Duration {
        Instant::now() - self.created_at
    }
}

fn stable_sdp_origin_session_id(raw_id: &str) -> String {
    let candidate = raw_id
        .strip_prefix("session-")
        .or_else(|| raw_id.strip_prefix("media-session-"))
        .unwrap_or(raw_id);

    if !candidate.is_empty() && candidate.bytes().all(|b| b.is_ascii_digit()) {
        return candidate.to_string();
    }

    if let Ok(uuid) = uuid::Uuid::parse_str(candidate) {
        let bytes = uuid.as_u128().to_be_bytes();
        let low = u64::from_be_bytes(bytes[8..16].try_into().expect("uuid low bytes"));
        return low.max(1).to_string();
    }

    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in raw_id.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash.max(1).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::send::outbound_call::{
        OutboundCallOptionsSnapshot, PaiOverride, ProxyOverride,
    };
    use crate::auth::SipClientAuth;
    use crate::state_table::{Role, SessionId};
    use crate::types::Credentials;
    use rvoip_sip_core::types::{headers::HeaderValue, HeaderName, TypedHeader};
    use std::sync::Arc;

    const SECRET: &str = "session-state-secret-canary";
    const SECRET_HEADER_NAME: &str = "X-Session-State-Secret-Canary";

    fn secret_header() -> TypedHeader {
        TypedHeader::Other(
            HeaderName::Other(SECRET_HEADER_NAME.into()),
            HeaderValue::Raw(SECRET.as_bytes().to_vec()),
        )
    }

    #[test]
    fn pending_reinvite_debug_redacts_sdp_update_body() {
        let debug = format!("{:?}", PendingReinvite::SdpUpdate(SECRET.into()));

        assert_eq!(debug, "SdpUpdate");
        assert!(!debug.contains(SECRET));
    }

    #[test]
    fn session_state_debug_redacts_retained_values() {
        let mut session =
            SessionState::new(SessionId::from_string("session-visible-id"), Role::UAC);
        session.local_sdp = Some(format!("v=0\r\na={SECRET}"));
        session.remote_sdp = Some(format!("v=0\r\na={SECRET}"));
        session.sdp_origin_session_id = SECRET.into();
        session.call_id = Some(SECRET.into());
        session.local_uri = Some(format!("sip:{SECRET}@local.invalid"));
        session.remote_uri = Some(format!("sip:{SECRET}@remote.invalid"));
        session.last_200_ok = Some(SECRET.as_bytes().to_vec());
        session.transfer_target = Some(format!("sip:{SECRET}@transfer.invalid"));
        session.dtmf_digits = Some(SECRET.into());
        session.reject_reason = Some(SECRET.into());
        session.reject_response_extras = Some(vec![secret_header()]);
        session.redirect_response_contacts = vec![format!("sip:{SECRET}@redirect.invalid")];
        session.early_media_sdp = Some(format!("v=0\r\na={SECRET}"));
        session.pending_auth = Some((401, format!("Digest {SECRET}")));
        session.pending_auth_method = Some(SECRET.into());
        session.redirect_targets = vec![format!("sip:{SECRET}@retry.invalid")];
        session.pending_reinvite = Some(PendingReinvite::SdpUpdate(format!("v=0\r\na={SECRET}")));
        session.replaces_header = Some(SECRET.into());
        session.referred_by = Some(SECRET.into());
        session.refer_transaction_id = Some(SECRET.into());
        session.transfer_target_last_progress = Some((183, SECRET.into()));
        session.pending_bye_reason = Some((SECRET.into(), 500, Some(SECRET.into())));
        session.pending_invite_options = Some(Arc::new(OutboundCallOptionsSnapshot {
            from: Some(format!("sip:{SECRET}@from.invalid")),
            to: format!("sip:{SECRET}@target.invalid"),
            sdp: Some(format!("v=0\r\na={SECRET}")),
            credentials: Some(Credentials::new(SECRET, SECRET)),
            auth: Some(SipClientAuth::bearer_token(SECRET)),
            pai_override: PaiOverride::Use(format!("sip:{SECRET}@pai.invalid")),
            contact_uri: Some(format!("sip:{SECRET}@contact.invalid")),
            outbound_proxy_override: ProxyOverride::Use(format!("sip:{SECRET}@proxy.invalid")),
            subject: Some(SECRET.into()),
            from_display: Some(SECRET.into()),
            precomputed_auth: Some(format!("Bearer {SECRET}")),
            transfer_leg: Some(SECRET.into()),
            supported_100rel: true,
            extra_headers: vec![secret_header()],
            topology_hiding: true,
        }));
        session.pending_register_options = Some(Arc::new(
            rvoip_sip_dialog::api::unified::RegisterRequestOptions {
                registrar_uri: format!("sip:{SECRET}@registrar.invalid"),
                aor_uri: format!("sip:{SECRET}@aor.invalid"),
                contact_uri: format!("sip:{SECRET}@contact.invalid"),
                authorization: Some(format!("Bearer {SECRET}")),
                proxy_authorization: Some(format!("Digest {SECRET}")),
                call_id: Some(SECRET.into()),
                extra_headers: vec![secret_header()],
                ..Default::default()
            },
        ));
        session.registrar_uri = Some(format!("sip:{SECRET}@registrar.invalid"));
        session.registration_contact = Some(format!("sip:{SECRET}@contact.invalid"));
        session.registration_call_id = Some(SECRET.into());
        session.registration_last_failure = Some(SECRET.into());
        session.registration_service_route = Some(vec![format!("sip:{SECRET}@route.invalid")]);
        session.registration_pub_gruu = Some(format!("sip:{SECRET}@pub-gruu.invalid"));
        session.registration_temp_gruu = Some(format!("sip:{SECRET}@temp-gruu.invalid"));
        session.credentials = Some(Credentials::new(SECRET, SECRET));
        session.auth = Some(SipClientAuth::bearer_token(SECRET));
        session.pai_uri = Some(format!("sip:{SECRET}@pai.invalid"));
        session.extra_headers = vec![secret_header()];
        session.auth_challenge_raw = Some(format!("Digest {SECRET}"));
        session.auth_challenge_replaces_nonce = Some(SECRET.into());
        session.digest_nc.insert((SECRET.into(), SECRET.into()), 3);

        let debug = format!("{session:?}");

        assert!(!debug.contains(SECRET), "secret escaped through {debug}");
        assert!(
            !debug.contains(SECRET_HEADER_NAME),
            "header name escaped through {debug}"
        );
        assert!(debug.contains("call_state: Idle"));
        assert!(debug.contains("local_sdp_present: true"));
        assert!(debug.contains("pending_auth_status: Some(401)"));
        assert!(debug.contains("pending_reinvite: Some(\"sdp_update\")"));
        assert!(debug.contains("pending_option_count: 2"));
        assert!(debug.contains("credentials_present: true"));
        assert!(debug.contains("auth_present: true"));
        assert!(debug.contains("extra_header_count: 1"));
        assert!(debug.contains("digest_nonce_count: 1"));
    }

    /// RFC 7616 §3.4.5 — repeated requests reusing the same nonce
    /// must carry monotonically incrementing `nc`. The exact idiom
    /// used at both call sites (`SendINVITEWithAuth` and REGISTER
    /// auth) is exercised here to guard against drift.
    #[test]
    fn digest_nc_increments_for_same_realm_nonce() {
        let mut session = SessionState::new(SessionId::new(), Role::UAC);
        let key = ("example.com".to_string(), "shared-nonce".to_string());

        let first = *session
            .digest_nc
            .entry(key.clone())
            .and_modify(|n| *n += 1)
            .or_insert(1);
        let second = *session
            .digest_nc
            .entry(key.clone())
            .and_modify(|n| *n += 1)
            .or_insert(1);
        let third = *session
            .digest_nc
            .entry(key.clone())
            .and_modify(|n| *n += 1)
            .or_insert(1);

        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_eq!(third, 3);
    }

    /// A fresh challenge with a new nonce gets its own counter space.
    /// The old entry stays in the map but is never read again — the
    /// session's `auth_challenge` field has been overwritten with the
    /// new nonce, so subsequent compute calls use the new key.
    #[test]
    fn digest_nc_keys_independent_per_nonce() {
        let mut session = SessionState::new(SessionId::new(), Role::UAC);
        let key_a = ("example.com".to_string(), "nonce-A".to_string());
        let key_b = ("example.com".to_string(), "nonce-B".to_string());

        for _ in 0..5 {
            session
                .digest_nc
                .entry(key_a.clone())
                .and_modify(|n| *n += 1)
                .or_insert(1);
        }

        let first_b = *session
            .digest_nc
            .entry(key_b.clone())
            .and_modify(|n| *n += 1)
            .or_insert(1);
        assert_eq!(first_b, 1, "fresh nonce starts a new counter");
        assert_eq!(*session.digest_nc.get(&key_a).unwrap(), 5);
    }
}
