//! Session coordination events
//!
//! Events sent from dialog-core to session-core for session management
//! coordination. This maintains the proper layer separation where dialog-core
//! handles SIP protocol operations and session-core handles session logic.

use std::{fmt, net::SocketAddr};

use crate::transaction::TransactionKey;
use rvoip_infra_common::events::cross_crate::OutboundRequestOutcome;
use rvoip_sip_core::types::refer_to::ReferTo;
use rvoip_sip_core::{Method, Request, Response, Uri};

use crate::dialog::DialogId;

/// Methods whose builder-option snapshot lifecycle is completed by the
/// generic exact outbound-request event.
///
/// BYE/CANCEL and out-of-dialog MESSAGE/OPTIONS/SUBSCRIBE retain their
/// existing method-specific completion paths. Emitting the generic event for
/// those methods would add protocol work to the call teardown hot path and
/// could clear state owned by a different lifecycle.
pub(crate) fn tracks_generic_outbound_request_completion(method: &Method) -> bool {
    matches!(
        method,
        Method::Info | Method::Refer | Method::Notify | Method::Update
    )
}

/// Events sent from dialog-core to session-core for coordination
#[derive(Clone)]
pub enum SessionCoordinationEvent {
    /// Incoming call that needs session creation
    IncomingCall {
        /// Dialog ID created for this call
        dialog_id: DialogId,

        /// Transaction ID for the INVITE
        transaction_id: TransactionKey,

        /// The original INVITE request
        request: Request,

        /// Source address of the INVITE
        source: SocketAddr,
    },

    /// Re-INVITE within an existing dialog
    ReInvite {
        /// Dialog ID for the re-INVITE
        dialog_id: DialogId,

        /// Transaction ID for the re-INVITE
        transaction_id: TransactionKey,

        /// The re-INVITE request
        request: Request,
    },

    /// Call has been answered (200 OK sent)
    CallAnswered {
        /// Dialog ID for the answered call
        dialog_id: DialogId,

        /// SDP answer that was sent
        session_answer: String,
    },

    /// Call is ringing (180 Ringing received)
    CallRinging {
        /// Dialog ID for the ringing call
        dialog_id: DialogId,
    },

    /// Call is terminating (Phase 1 - cleanup in progress)
    CallTerminating {
        /// Dialog ID for the terminating call
        dialog_id: DialogId,

        /// Reason for termination
        reason: String,
    },

    /// Inbound BYE received inside an established dialog. Dialog-core has
    /// already sent the 200 OK; session-core only needs to run BYE cleanup.
    ByeReceived {
        /// Dialog ID for the BYE
        dialog_id: DialogId,
    },

    /// Call has been terminated (Phase 2 - cleanup complete)
    CallTerminated {
        /// Dialog ID for the terminated call
        dialog_id: DialogId,

        /// Reason for termination
        reason: String,
    },

    /// Call has been cancelled (CANCEL received)
    CallCancelled {
        /// Dialog ID for the cancelled call
        dialog_id: DialogId,

        /// Reason for cancellation
        reason: String,
    },

    /// Response received for a transaction
    ResponseReceived {
        /// Dialog ID associated with the response
        dialog_id: DialogId,

        /// The SIP response received
        response: Response,

        /// Transaction ID that received the response
        transaction_id: TransactionKey,

        /// Exact Request-URI from the original outbound request when it is
        /// required for authentication. This is captured from transaction
        /// state, never reconstructed from dialog/session metadata.
        request_uri: Option<Uri>,
    },

    /// Exact terminal outcome for an outbound non-INVITE request that did not
    /// carry a response through [`Self::ResponseReceived`] (for example a
    /// timeout or transport failure).
    OutboundRequestCompleted {
        dialog_id: DialogId,
        transaction_id: TransactionKey,
        method: Method,
        outcome: OutboundRequestOutcome,
    },

    /// Registration request received
    RegistrationRequest {
        /// Transaction ID for the REGISTER
        transaction_id: TransactionKey,

        /// From URI from the REGISTER
        from_uri: Uri,

        /// Contact URI from the REGISTER
        contact_uri: Uri,

        /// Expires value (in seconds)
        expires: u32,
    },

    /// Dialog state change notification
    DialogStateChanged {
        /// Dialog ID that changed
        dialog_id: DialogId,

        /// New dialog state
        new_state: String,

        /// Previous state
        previous_state: String,
    },

    /// Early media indication (1xx response with SDP)
    EarlyMedia {
        /// Dialog ID
        dialog_id: DialogId,

        /// SDP for early media
        sdp: String,
    },

    /// Call progress indication (non-SDP 1xx responses)
    CallProgress {
        /// Dialog ID
        dialog_id: DialogId,

        /// Status code received
        status_code: u16,

        /// Reason phrase
        reason_phrase: String,
    },

    /// Request failed (4xx, 5xx, 6xx responses)
    RequestFailed {
        /// Dialog ID (if available)
        dialog_id: Option<DialogId>,

        /// Transaction ID
        transaction_id: TransactionKey,

        /// Status code
        status_code: u16,

        /// Reason phrase
        reason_phrase: String,

        /// Original request method
        method: String,
    },

    /// Capability query (OPTIONS request)
    CapabilityQuery {
        /// Transaction ID for the OPTIONS
        transaction_id: TransactionKey,

        /// The OPTIONS request
        request: Request,

        /// Source address of the OPTIONS
        source: SocketAddr,
    },

    /// ACK sent for 2xx response (UAC side - RFC compliant media start point)
    AckSent {
        /// Dialog ID that sent the ACK
        dialog_id: DialogId,

        /// Transaction ID for the ACK
        transaction_id: TransactionKey,

        /// Final negotiated SDP if available
        negotiated_sdp: Option<String>,
    },

    /// ACK received for 2xx response (UAS side - RFC compliant media start point)
    AckReceived {
        /// Dialog ID that received the ACK
        dialog_id: DialogId,

        /// Transaction ID for the ACK
        transaction_id: TransactionKey,

        /// Final negotiated SDP if available
        negotiated_sdp: Option<String>,
    },

    /// RFC 4028 session-timer refresh succeeded (UPDATE or re-INVITE sent).
    SessionRefreshed {
        /// Dialog that was refreshed.
        dialog_id: DialogId,
        /// Negotiated session-expires interval in seconds.
        expires_secs: u32,
    },

    /// RFC 4028 session-timer refresh failed. The dialog has been torn
    /// down with BYE (§10). The session layer should emit CallEnded.
    SessionRefreshFailed {
        /// Dialog whose refresh failed.
        dialog_id: DialogId,
        /// Human-readable reason.
        reason: String,
    },

    /// Cleanup confirmation from a layer
    CleanupConfirmation {
        /// Dialog ID for the cleanup
        dialog_id: DialogId,

        /// Which layer is confirming cleanup
        layer: String, // "media", "client", etc.
    },

    /// Call transfer request received (REFER)
    TransferRequest {
        /// Dialog ID for the call being transferred
        dialog_id: DialogId,

        /// Transaction ID for the REFER request
        transaction_id: TransactionKey,

        /// The parsed Refer-To header (target of transfer)
        refer_to: ReferTo,

        /// Optional Referred-By header (who initiated transfer)
        referred_by: Option<String>,

        /// Optional Replaces header (for attended transfer)
        replaces: Option<String>,

        /// SIP_API_DESIGN_2 §7.5 — original inbound REFER bytes,
        /// preserved so the cross-crate `TransferRequested` variant
        /// can carry them through to session-core's
        /// `IncomingRequest` view (header-level access for
        /// `Target-Dialog`, custom X-* headers, etc.). `None` for
        /// synthesized/test publish paths.
        raw_request: Option<bytes::Bytes>,
    },

    /// RFC 5626 outbound flow has failed — the keep-alive ping either
    /// timed out waiting for a pong, saw a transport-level connection
    /// close, or hit an unrecoverable send error. The session layer
    /// should trigger a fresh REGISTER for the identified AoR so the
    /// UA re-establishes a flow without waiting for registration
    /// expiry (RFC 5626 §4.4.1 flow recovery).
    OutboundFlowFailed {
        /// AoR (To URI of the originating REGISTER, normalized) whose
        /// flow has failed.
        aor: String,
        /// RFC 5626 §4.2 `reg-id` of the failed flow.
        reg_id: u32,
        /// RFC 5626 §4.1 instance URN of the UA.
        instance: String,
        /// Underlying cause of the failure (for telemetry + debouncing).
        reason: FlowFailureReason,
    },
}

fn safe_method_label(method: &Method) -> &'static str {
    match method {
        Method::Invite => "INVITE",
        Method::Ack => "ACK",
        Method::Bye => "BYE",
        Method::Cancel => "CANCEL",
        Method::Register => "REGISTER",
        Method::Options => "OPTIONS",
        Method::Subscribe => "SUBSCRIBE",
        Method::Notify => "NOTIFY",
        Method::Update => "UPDATE",
        Method::Refer => "REFER",
        Method::Info => "INFO",
        Method::Message => "MESSAGE",
        Method::Prack => "PRACK",
        Method::Publish => "PUBLISH",
        Method::Extension(_) => "extension",
    }
}

struct SafeTransactionKeyDebug<'a>(&'a TransactionKey);

impl fmt::Debug for SafeTransactionKeyDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransactionKey")
            .field("method", &safe_method_label(&self.0.method))
            .field("is_server", &self.0.is_server)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for SessionCoordinationEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncomingCall {
                dialog_id,
                transaction_id,
                request,
                source,
            } => formatter
                .debug_struct("IncomingCall")
                .field("dialog_id", dialog_id)
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("request_method", &safe_method_label(&request.method))
                .field("request_header_count", &request.headers.len())
                .field("request_body_len", &request.body.len())
                .field("source", source)
                .finish(),
            Self::ReInvite {
                dialog_id,
                transaction_id,
                request,
            } => formatter
                .debug_struct("ReInvite")
                .field("dialog_id", dialog_id)
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("request_method", &safe_method_label(&request.method))
                .field("request_header_count", &request.headers.len())
                .field("request_body_len", &request.body.len())
                .finish(),
            Self::CallAnswered {
                dialog_id,
                session_answer,
            } => formatter
                .debug_struct("CallAnswered")
                .field("dialog_id", dialog_id)
                .field("session_answer_present", &!session_answer.is_empty())
                .field("session_answer_len", &session_answer.len())
                .finish(),
            Self::CallRinging { dialog_id } => formatter
                .debug_struct("CallRinging")
                .field("dialog_id", dialog_id)
                .finish(),
            Self::CallTerminating { dialog_id, reason } => formatter
                .debug_struct("CallTerminating")
                .field("dialog_id", dialog_id)
                .field("reason", &"[redacted]")
                .field("reason_len", &reason.len())
                .finish(),
            Self::ByeReceived { dialog_id } => formatter
                .debug_struct("ByeReceived")
                .field("dialog_id", dialog_id)
                .finish(),
            Self::CallTerminated { dialog_id, reason } => formatter
                .debug_struct("CallTerminated")
                .field("dialog_id", dialog_id)
                .field("reason", &"[redacted]")
                .field("reason_len", &reason.len())
                .finish(),
            Self::CallCancelled { dialog_id, reason } => formatter
                .debug_struct("CallCancelled")
                .field("dialog_id", dialog_id)
                .field("reason", &"[redacted]")
                .field("reason_len", &reason.len())
                .finish(),
            Self::ResponseReceived {
                dialog_id,
                response,
                transaction_id,
                request_uri,
            } => formatter
                .debug_struct("ResponseReceived")
                .field("dialog_id", dialog_id)
                .field("response_status", &response.status_code())
                .field("response_header_count", &response.headers.len())
                .field("response_body_len", &response.body.len())
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("request_uri_present", &request_uri.is_some())
                .finish(),
            Self::OutboundRequestCompleted {
                dialog_id,
                transaction_id,
                method,
                outcome,
            } => formatter
                .debug_struct("OutboundRequestCompleted")
                .field("dialog_id", dialog_id)
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("method", &safe_method_label(method))
                .field("outcome", outcome)
                .finish(),
            Self::RegistrationRequest {
                transaction_id,
                from_uri: _,
                contact_uri: _,
                expires,
            } => formatter
                .debug_struct("RegistrationRequest")
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("from_uri", &"[redacted]")
                .field("from_uri_present", &true)
                .field("contact_uri", &"[redacted]")
                .field("contact_uri_present", &true)
                .field("expires", expires)
                .finish(),
            Self::DialogStateChanged {
                dialog_id,
                new_state,
                previous_state,
            } => formatter
                .debug_struct("DialogStateChanged")
                .field("dialog_id", dialog_id)
                .field("new_state", &"[redacted]")
                .field("new_state_len", &new_state.len())
                .field("previous_state", &"[redacted]")
                .field("previous_state_len", &previous_state.len())
                .finish(),
            Self::EarlyMedia { dialog_id, sdp } => formatter
                .debug_struct("EarlyMedia")
                .field("dialog_id", dialog_id)
                .field("sdp_present", &!sdp.is_empty())
                .field("sdp_len", &sdp.len())
                .finish(),
            Self::CallProgress {
                dialog_id,
                status_code,
                reason_phrase,
            } => formatter
                .debug_struct("CallProgress")
                .field("dialog_id", dialog_id)
                .field("status_code", status_code)
                .field("reason_phrase", &"[redacted]")
                .field("reason_phrase_len", &reason_phrase.len())
                .finish(),
            Self::RequestFailed {
                dialog_id,
                transaction_id,
                status_code,
                reason_phrase,
                method,
            } => formatter
                .debug_struct("RequestFailed")
                .field("dialog_id", dialog_id)
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("status_code", status_code)
                .field("reason_phrase", &"[redacted]")
                .field("reason_phrase_len", &reason_phrase.len())
                .field("method", &"[redacted]")
                .field("method_len", &method.len())
                .finish(),
            Self::CapabilityQuery {
                transaction_id,
                request,
                source,
            } => formatter
                .debug_struct("CapabilityQuery")
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("request_method", &safe_method_label(&request.method))
                .field("request_header_count", &request.headers.len())
                .field("request_body_len", &request.body.len())
                .field("source", source)
                .finish(),
            Self::AckSent {
                dialog_id,
                transaction_id,
                negotiated_sdp,
            } => formatter
                .debug_struct("AckSent")
                .field("dialog_id", dialog_id)
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("negotiated_sdp_present", &negotiated_sdp.is_some())
                .field(
                    "negotiated_sdp_len",
                    &negotiated_sdp.as_ref().map(String::len),
                )
                .finish(),
            Self::AckReceived {
                dialog_id,
                transaction_id,
                negotiated_sdp,
            } => formatter
                .debug_struct("AckReceived")
                .field("dialog_id", dialog_id)
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("negotiated_sdp_present", &negotiated_sdp.is_some())
                .field(
                    "negotiated_sdp_len",
                    &negotiated_sdp.as_ref().map(String::len),
                )
                .finish(),
            Self::SessionRefreshed {
                dialog_id,
                expires_secs,
            } => formatter
                .debug_struct("SessionRefreshed")
                .field("dialog_id", dialog_id)
                .field("expires_secs", expires_secs)
                .finish(),
            Self::SessionRefreshFailed { dialog_id, reason } => formatter
                .debug_struct("SessionRefreshFailed")
                .field("dialog_id", dialog_id)
                .field("reason", &"[redacted]")
                .field("reason_len", &reason.len())
                .finish(),
            Self::CleanupConfirmation { dialog_id, layer } => formatter
                .debug_struct("CleanupConfirmation")
                .field("dialog_id", dialog_id)
                .field("layer", &"[redacted]")
                .field("layer_len", &layer.len())
                .finish(),
            Self::TransferRequest {
                dialog_id,
                transaction_id,
                refer_to: _,
                referred_by,
                replaces,
                raw_request,
            } => formatter
                .debug_struct("TransferRequest")
                .field("dialog_id", dialog_id)
                .field("transaction_id", &SafeTransactionKeyDebug(transaction_id))
                .field("refer_to", &"[redacted]")
                .field("refer_to_present", &true)
                .field("referred_by_present", &referred_by.is_some())
                .field("referred_by_len", &referred_by.as_ref().map(String::len))
                .field("replaces_present", &replaces.is_some())
                .field("replaces_len", &replaces.as_ref().map(String::len))
                .field("raw_request_present", &raw_request.is_some())
                .field(
                    "raw_request_len",
                    &raw_request.as_ref().map(bytes::Bytes::len),
                )
                .finish(),
            Self::OutboundFlowFailed {
                aor,
                reg_id,
                instance,
                reason,
            } => formatter
                .debug_struct("OutboundFlowFailed")
                .field("aor", &"[redacted]")
                .field("aor_len", &aor.len())
                .field("reg_id", reg_id)
                .field("instance", &"[redacted]")
                .field("instance_len", &instance.len())
                .field("reason", reason)
                .finish(),
        }
    }
}

/// Cause of an RFC 5626 outbound flow failure, reported alongside
/// [`SessionCoordinationEvent::OutboundFlowFailed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowFailureReason {
    /// Keep-alive ping (CRLFCRLF) was sent but no pong (CRLF) arrived
    /// within the configured pong-timeout window.
    PongTimeout,
    /// Transport layer reported the connection closed (TCP FIN/RST, TLS
    /// close_notify, etc.) before the flow could be explicitly torn
    /// down.
    ConnectionClosed,
    /// Transport-level send on the keep-alive ping itself returned an
    /// unrecoverable error (e.g. socket gone).
    SendError,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use rvoip_sip_core::{
        types::headers::{HeaderName, HeaderValue, TypedHeader},
        Method, StatusCode,
    };
    use std::str::FromStr;

    fn transaction_key(method: Method) -> TransactionKey {
        TransactionKey::new("z9hG4bK-debug-canary".into(), method, true)
    }

    #[test]
    fn coordination_debug_exposes_only_safe_signaling_metadata() {
        const URI_SECRET: &str = "coordination-uri-secret.example";
        const AUTH_SECRET: &str = "Bearer coordination-auth-secret";
        const BODY_SECRET: &str = "coordination-body-secret";
        const REASON_SECRET: &str = "coordination-reason-secret";
        const SDP_SECRET: &str = "v=0 s=coordination-sdp-secret";
        const RAW_SECRET: &str = "coordination-raw-request-secret";
        const METHOD_SECRET: &str = "coordination-extension-method-secret";
        const BRANCH_SECRET: &str = "z9hG4bK-coordination-branch-secret";
        let dialog_id = DialogId::new();

        let mut request = Request::new(
            Method::Invite,
            format!("sip:bob@{URI_SECRET}").parse().unwrap(),
        )
        .with_body(BODY_SECRET);
        request.headers.push(TypedHeader::Other(
            HeaderName::Authorization,
            HeaderValue::Raw(AUTH_SECRET.as_bytes().to_vec()),
        ));

        let debug_outputs = [
            format!(
                "{:?}",
                SessionCoordinationEvent::IncomingCall {
                    dialog_id: dialog_id.clone(),
                    transaction_id: transaction_key(Method::Invite),
                    request,
                    source: "127.0.0.1:5060".parse().unwrap(),
                }
            ),
            format!(
                "{:?}",
                SessionCoordinationEvent::ResponseReceived {
                    dialog_id: dialog_id.clone(),
                    response: Response::new(StatusCode::Ok)
                        .with_reason(REASON_SECRET)
                        .with_body(BODY_SECRET),
                    transaction_id: transaction_key(Method::Invite),
                    request_uri: Some(format!("sip:bob@{URI_SECRET}").parse().unwrap()),
                }
            ),
            format!(
                "{:?}",
                SessionCoordinationEvent::EarlyMedia {
                    dialog_id: dialog_id.clone(),
                    sdp: SDP_SECRET.into(),
                }
            ),
            format!(
                "{:?}",
                SessionCoordinationEvent::CallTerminating {
                    dialog_id: dialog_id.clone(),
                    reason: REASON_SECRET.into(),
                }
            ),
            format!(
                "{:?}",
                SessionCoordinationEvent::TransferRequest {
                    dialog_id,
                    transaction_id: transaction_key(Method::Refer),
                    refer_to: ReferTo::from_str(&format!("sip:transfer@{URI_SECRET}")).unwrap(),
                    referred_by: Some(format!("sip:referrer@{URI_SECRET}")),
                    replaces: Some(REASON_SECRET.into()),
                    raw_request: Some(Bytes::from_static(RAW_SECRET.as_bytes())),
                }
            ),
            format!(
                "{:?}",
                SessionCoordinationEvent::IncomingCall {
                    dialog_id: DialogId::new(),
                    transaction_id: TransactionKey::new(
                        BRANCH_SECRET.into(),
                        Method::Extension(METHOD_SECRET.into()),
                        true,
                    ),
                    request: Request::new(
                        Method::Extension(METHOD_SECRET.into()),
                        "sip:example.test".parse().unwrap(),
                    ),
                    source: "127.0.0.1:5060".parse().unwrap(),
                }
            ),
        ];

        for debug in &debug_outputs {
            for secret in [
                URI_SECRET,
                AUTH_SECRET,
                BODY_SECRET,
                REASON_SECRET,
                SDP_SECRET,
                RAW_SECRET,
                METHOD_SECRET,
                BRANCH_SECRET,
            ] {
                assert!(!debug.contains(secret));
            }
        }
        assert!(debug_outputs[0].contains("request_header_count"));
        assert!(debug_outputs[0].contains("request_body_len"));
        assert!(debug_outputs[1].contains("response_status"));
        assert!(debug_outputs[2].contains("sdp_len"));
        assert!(debug_outputs[4].contains("raw_request_len"));
        assert!(debug_outputs[5].contains("extension"));
    }
}
