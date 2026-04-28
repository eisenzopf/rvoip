use crate::state_table::{CallId, DialogId, MediaSessionId, SessionId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::history::{HistoryConfig, SessionHistory, TransitionRecord};
use crate::state_table::{ConditionUpdates, Role};
use crate::types::{CallState, MediaDirection};

/// Negotiated media configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiatedConfig {
    pub local_addr: SocketAddr,
    pub remote_addr: SocketAddr,
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u8,
}

/// Kind of mid-dialog re-INVITE that was in flight when a 491 Request
/// Pending arrived — captured so `ScheduleReinviteRetry` can re-issue the
/// correct operation after the RFC 3261 §14.1 random backoff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingReinvite {
    Hold,
    Resume,
    /// Generic SDP update with a specific offer (codec change, etc.).
    SdpUpdate(String),
}

/// Transfer state tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    None,
    TransferInitiated,
    TransferCompleted,
}

/// Complete state of a session
#[derive(Debug, Clone)]
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
    /// Optional `P-Asserted-Identity` URI (RFC 3325 §9.1) to attach to the
    /// outgoing INVITE for this session. When `Some`, the `SendINVITE` action
    /// routes through `dialog_adapter.send_invite_with_extra_headers` so the
    /// header lands on the very first wire transmission. Carrier trunks
    /// commonly require this for caller-ID assertion.
    pub pai_uri: Option<String>,
    pub is_registered: bool, // Whether registration is complete
    pub auth_challenge: Option<crate::auth::DigestChallenge>, // Cached authentication challenge from 401
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
            sdp_origin_session_id,
            sdp_origin_version: 0,
            local_media_direction: MediaDirection::SendRecv,
            remote_media_direction: MediaDirection::SendRecv,
            dialog_id: None,
            media_session_id: None,
            call_id: None,
            local_uri: None,
            remote_uri: None,
            last_200_ok: None,
            bridged_to: None,
            conference_mixer_id: None,
            transfer_target: None,
            dtmf_digits: None,
            reject_status: None,
            reject_reason: None,
            redirect_response_status: None,
            redirect_response_contacts: Vec::new(),
            early_media_sdp: None,
            pending_auth: None,
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
            pai_uri: None,
            is_registered: false,
            auth_challenge: None,
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
mod digest_nc_tests {
    use super::*;
    use crate::state_table::{Role, SessionId};

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
