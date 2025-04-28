use async_trait::async_trait;
use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;
use std::{any::Any, fmt, net::SocketAddr, sync::Arc, time::Duration};

use crate::error::{Error, Result}; // Use crate's error type
use crate::utils; // Import utils for key generation

pub mod client;
pub mod server;

/// Defines whether a transaction is Invite or Non-Invite, Client or Server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransactionKind {
    InviteClient,
    NonInviteClient,
    InviteServer,
    NonInviteServer,
}

/// Uniquely identifies a transaction based on RFC 3261 rules.
/// Typically derived from the top Via branch, CSeq method, and potentially To/From tags.
// TODO: Define the actual structure and implementation for TransactionKey.
// It needs to be derivable from a Message and comparable.
// For now, using a simple String alias, but this should be a proper struct.
pub type TransactionKey = String;

// TODO: Define `TransactionId` if it's distinct from `TransactionKey`. Often they are the same.
pub type TransactionId = TransactionKey;


/// Represents events flowing *from* the transaction layer *to* the Transaction User (TU),
/// such as the Session layer or application logic.
#[derive(Debug, Clone)]
pub enum TransactionEvent {
    // --- Request Processing (Server Transactions) ---
    /// A new request has arrived that initiated a server transaction.
    /// The TU should process this request and eventually call `send_response` on the manager.
    NewRequest {
        transaction_id: TransactionKey,
        request: Request,
        source: SocketAddr,
    },
    /// An ACK was received for a non-2xx final response previously sent by an Invite Server Transaction.
    AckReceived {
        transaction_id: TransactionKey,
        ack_request: Request,
    },
    /// A CANCEL request was received for an existing Invite Server Transaction.
    /// The TU may need to stop processing the original INVITE.
    CancelReceived {
        transaction_id: TransactionKey,
        cancel_request: Request,
    },

    // --- Response Processing (Client Transactions) ---
    /// A provisional (1xx) response was received.
    ProvisionalResponse {
        transaction_id: TransactionKey,
        response: Response,
    },
    /// A successful final (2xx) response was received.
    /// For INVITE, the TU is responsible for sending the ACK via `manager.send_2xx_ack()`.
    SuccessResponse {
        transaction_id: TransactionKey,
        response: Response,
    },
    /// A failure final (3xx-6xx) response was received.
    FailureResponse {
        transaction_id: TransactionKey,
        response: Response,
    },

    // --- Response Sending (Server Transactions - Optional Info for TU) ---
    /// A provisional response was sent by the server transaction.
    ProvisionalResponseSent {
        transaction_id: TransactionKey,
        response: Response,
    },
    /// A final response was sent by the server transaction.
    FinalResponseSent {
        transaction_id: TransactionKey,
        response: Response,
    },

    // --- State and Error Events ---
    /// A transaction timed out (e.g., Timer B for INVITE client, Timer F for non-INVITE client).
    TransactionTimeout {
        transaction_id: TransactionKey,
    },
    /// An ACK was not received for a non-2xx final response in time (Timer H for INVITE server).
    AckTimeout {
        transaction_id: TransactionKey,
    },
    /// A transport error occurred related to this transaction. Transaction is likely terminated.
    TransportError {
        transaction_id: TransactionKey,
    },
    /// An internal error occurred within the transaction state machine.
    Error {
        transaction_id: Option<TransactionKey>,
        error: String, // Consider using crate::Error directly if clonable/suitable
    },
    /// A message was received that didn't match any existing transaction.
    StrayRequest {
        request: Request,
        source: SocketAddr,
    },
    StrayResponse {
        response: Response,
        source: SocketAddr,
    },
    /// Stray ACK (didn't match any server INVITE transaction). Usually ignored, but TU might want to know.
    StrayAck {
         request: Request,
         source: SocketAddr,
    },
     /// Stray CANCEL (didn't match any server INVITE transaction). 481 sent automatically.
    StrayCancel {
         request: Request,
         source: SocketAddr,
    },

    // --- Timer Events (Internal to Transaction Layer) ---
    /// Internal event used to trigger timer logic within the transaction.
    #[doc(hidden)] // Should not be exposed directly to TU
    TimerTriggered {
        transaction_id: TransactionKey,
        timer: String, // e.g., "A", "B", "G", "H", "I", "J", "K"
    },

     // --- Optional events for finer-grained state tracking ---
     // TransactionCreated { transaction_id: TransactionKey },
     // TransactionTerminated { transaction_id: TransactionKey },
     // StateChanged { transaction_id: TransactionKey, state: TransactionState },
}


/// SIP transaction states (aligned with RFC 3261 state machines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransactionState {
    // Common initial/final states
    Initial,    // Before any action
    Completed,  // Final response sent/received, waiting for termination timer/ACK
    Confirmed,  // ACK received (Server INVITE only)
    Terminated, // Transaction finished

    // Client States
    Calling,    // INVITE specific: Request sent, waiting 1xx/final
    Trying,     // Non-INVITE specific: Request sent, waiting 1xx/final
    Proceeding, // 1xx received, waiting final

    // Server States (Proceeding used for both INVITE/Non-INVITE after 1xx sent)
    // Trying,     // Non-INVITE specific: Request received, before 1xx sent
    // Proceeding, // Request received, 1xx sent (used instead of ServerProceeding)
}


/// Core SIP transaction trait. Defines common behavior for client and server transactions.
#[async_trait]
pub trait Transaction: fmt::Debug + Send + Sync + 'static {
    /// Get the transaction's unique key.
    fn id(&self) -> &TransactionKey;

    /// Get the kind of transaction (Invite/NonInvite, Client/Server).
    fn kind(&self) -> TransactionKind;

    /// Get the current state of the transaction machine.
    fn state(&self) -> TransactionState;

    /// Get the network transport used by this transaction.
    fn transport(&self) -> Arc<dyn Transport>;

     /// Get the remote address associated with this transaction.
     /// (Destination for client tx, Source for server tx).
     fn remote_addr(&self) -> SocketAddr;

    /// Process an event relevant to this transaction (e.g., incoming message, timer).
    /// This method is primarily for internal use by the TransactionManager.
    /// `event_type` might be "request", "response", "timer", "transport_err".
    async fn process_event(&mut self, event_type: &str, message: Option<Message>) -> Result<()>;

    /// Explicitly handle a timer event dispatched by the manager.
    async fn handle_timer(&mut self, timer_name: String) -> Result<()>;

    /// Check if this transaction matches the given message based on RFC 3261 rules.
    /// Primarily used by the TransactionManager for dispatching incoming messages.
    fn matches(&self, message: &Message) -> bool;

    /// Check if the transaction is in a completed or terminated state.
    fn is_finished(&self) -> bool {
        matches!(
            self.state(),
            TransactionState::Completed | TransactionState::Confirmed | TransactionState::Terminated
        )
    }

    /// Check if the transaction is specifically terminated.
    fn is_terminated(&self) -> bool {
        self.state() == TransactionState::Terminated
    }

    // --- Optional helper methods for accessing internal state ---
    /// Get the original request that initiated this transaction.
    fn original_request(&self) -> &Request;

    /// Get the last response sent or received by this transaction.
    fn last_response(&self) -> Option<&Response>;
}

// Implement `from_message` for TransactionKey (placeholder)
// In a real implementation, this would parse Via, CSeq etc.
impl TransactionKey {
    pub fn from_message(message: &Message) -> Result<Self> {
        // Placeholder: Use Via branch + CSeq method for uniqueness attempt
         let branch = utils::extract_branch(message)
             .ok_or_else(|| Error::Other("Missing branch in Via for key generation".to_string()))?;
         let method = match message {
             Message::Request(req) => req.method().clone(),
             // Use the helper function to extract CSeq from Response
             Message::Response(_) => utils::extract_cseq(message)
                                        .ok_or(Error::Other("Missing or invalid CSeq in Response".to_string()))?
                                        .1, // Get the Method part
         };

        // Include CSeq method for server transactions to differentiate INVITE/non-INVITE on same branch
        // Client transactions use CSeq method from original request implicitly via tx type.
        // Include top Via sent-by for server requests to disambiguate retransmissions arriving
        // at different transport listeners (though manager should ideally handle this).

        // TODO: Refine key generation according to RFC 3261 Section 17.1.3 and 17.2.3 rigorously.
        // For server transactions, key = branch + sent-by + method (excluding ACK/CANCEL?)
        // For client transactions, key = branch + sent-by + method (of original request)
        // This simple version might have collisions.

         Ok(format!("{}-{}", branch, method)) // Highly simplified!
    }
} 