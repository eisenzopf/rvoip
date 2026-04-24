//! Session coordination events
//!
//! Events sent from dialog-core to session-core for session management
//! coordination. This maintains the proper layer separation where dialog-core
//! handles SIP protocol operations and session-core handles session logic.

use std::net::SocketAddr;

use rvoip_sip_core::{Request, Response, Uri};
use rvoip_sip_core::types::refer_to::ReferTo;
use crate::transaction::TransactionKey;

use crate::dialog::DialogId;

/// Events sent from dialog-core to session-core for coordination
#[derive(Debug, Clone)]
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