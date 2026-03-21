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
    
    /// Cleanup confirmation from a layer
    CleanupConfirmation {
        /// Dialog ID for the cleanup
        dialog_id: DialogId,
        
        /// Which layer is confirming cleanup
        layer: String, // "media", "client", etc.
    },
    
    /// PUBLISH request received (RFC 3903 event state publication)
    PublishReceived {
        /// Transaction ID for the PUBLISH
        transaction_id: TransactionKey,

        /// Event type from the Event header
        event_type: String,

        /// Body content (event state document)
        body: Option<Vec<u8>>,

        /// Content-Type of the body
        content_type: Option<String>,

        /// Source address
        source: SocketAddr,
    },

    /// PRACK request received (RFC 3262 reliable provisional response acknowledgement)
    PrackReceived {
        /// Dialog ID for the PRACK
        dialog_id: DialogId,

        /// Transaction ID for the PRACK
        transaction_id: TransactionKey,

        /// RAck header value (response-num cseq-num method)
        rack_value: String,

        /// The PRACK request
        request: Request,
    },

    /// MESSAGE request received (RFC 3428 instant messaging)
    MessageReceived {
        /// Transaction ID for the MESSAGE
        transaction_id: TransactionKey,

        /// Body content (message text)
        body: Option<Vec<u8>>,

        /// Content-Type of the body
        content_type: Option<String>,

        /// Source address
        source: SocketAddr,

        /// The original MESSAGE request
        request: Request,
    },

    /// Forked response detected (RFC 3261 Section 13.2.2.4)
    ///
    /// Emitted when a 1xx or 2xx response to a client INVITE arrives with a
    /// different To-tag than any existing dialog for this Call-ID, indicating
    /// that a downstream proxy forked the request.
    ForkedResponse {
        /// Call-ID shared by every fork in the group
        call_id: String,

        /// Dialog created (or matched) for this particular fork
        dialog_id: DialogId,

        /// Status code of the forked response (e.g. 180, 200)
        status_code: u16,
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
} 