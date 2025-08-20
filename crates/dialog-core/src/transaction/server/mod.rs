/// # Server Transaction Module
///
/// This module implements the server-side transaction state machines according to 
/// [RFC 3261 Section 17.2](https://datatracker.ietf.org/doc/html/rfc3261#section-17.2).
/// 
/// ## SIP Server Transactions
///
/// Server transactions are created when a SIP element receives a request from a client.
/// They ensure proper handling of requests, responses, and retransmissions according
/// to the SIP protocol specifications.
///
/// ## Transaction Types
///
/// RFC 3261 defines two types of server transactions with different state machines:
///
/// 1. **INVITE Server Transactions** (Section 17.2.1): Used for handling session establishment requests.
///    - More complex due to the three-way handshake (INVITE, response, ACK)
///    - Uses a four-state machine: Proceeding, Completed, Confirmed, and Terminated
///    - Uses timers G, H, and I for retransmission and timeout management
///    - Must handle ACK specially in the Completed state
///
/// 2. **Non-INVITE Server Transactions** (Section 17.2.2): Used for all other request types.
///    - Simpler state machine with three states: Trying, Proceeding, and Completed
///    - Uses timer J for state management
///    - No special handling for ACK required
///
/// ## Implementation Details
///
/// Both transaction types share common infrastructure but implement different state machines:
///
/// - `ServerInviteTransaction`: Implements the INVITE server transaction state machine
/// - `ServerNonInviteTransaction`: Implements the non-INVITE server transaction state machine
/// - `ServerTransactionData`: Shared data structure for both transaction types
/// - `CommonServerTransaction`: Common behavior for server transactions
/// - `ServerTransaction`: Interface for all server transactions
///
/// ## Usage
///
/// Server transactions are typically created by the `TransactionManager` when it receives
/// a request from the network. It routes incoming messages to the appropriate transaction
/// and provides a clean API for the Transaction User (TU) to send responses.

mod common;
mod invite;
mod non_invite;
mod data;
pub mod builders;

pub use common::*;
pub use invite::ServerInviteTransaction;
pub use non_invite::ServerNonInviteTransaction;
pub use data::{ServerTransactionData, CommandSender, CommandReceiver, CommonServerTransaction};

use async_trait::async_trait;
use std::net::SocketAddr;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::transaction::error::Result;
use crate::transaction::{Transaction, TransactionState, TransactionKey, TransactionAsync};
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::json::ext::SipMessageJson;

/// Common interface for server transactions, implementing the behavior defined in RFC 3261 Section 17.2.
///
/// This trait defines operations that both INVITE and non-INVITE server transactions must support.
/// It encapsulates the functionality required to process requests, send responses, and track state
/// according to the SIP specification.
#[async_trait]
pub trait ServerTransaction: Transaction + TransactionAsync + CommonServerTransaction + Send + Sync + 'static {
    /// Processes an incoming request associated with this transaction.
    ///
    /// This handles various types of requests that may arrive for this transaction:
    /// - For INVITE server transactions: ACK requests or CANCEL requests
    /// - For non-INVITE server transactions: Retransmissions of the original request
    ///
    /// # Arguments
    ///
    /// * `request` - The SIP request to process
    ///
    /// # Returns
    ///
    /// A Future that resolves to Ok(()) if the request was processed successfully,
    /// or an Error if there was a problem.
    fn process_request(&self, request: Request) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Sends a response for this transaction, triggering appropriate state transitions.
    ///
    /// According to RFC 3261 Sections 17.2.1 and 17.2.2, sending responses triggers 
    /// specific state transitions based on the response status code:
    ///
    /// - For INVITE server transactions:
    ///   - 1xx responses keep the transaction in Proceeding state
    ///   - 2xx responses cause transition to Terminated state
    ///   - 3xx-6xx responses cause transition to Completed state
    ///
    /// - For non-INVITE server transactions:
    ///   - In Trying state, 1xx responses cause transition to Proceeding state
    ///   - In Trying or Proceeding state, final responses cause transition to Completed state
    ///
    /// # Arguments
    ///
    /// * `response` - The SIP response to send
    ///
    /// # Returns
    ///
    /// A Future that resolves to Ok(()) if the response was sent successfully,
    /// or an Error if there was a problem.
    fn send_response(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Returns the last response sent by this transaction.
    ///
    /// This can be used to handle retransmissions of requests, where the server
    /// should resend the last response without passing the request to the TU.
    ///
    /// # Returns
    ///
    /// The last SIP response sent by this transaction, or None if no response has been sent.
    fn last_response(&self) -> Option<Response>;
    
    /// Gets the Call-ID from the original request that created this transaction.
    ///
    /// Call-ID is a critical dialog identifier used to match ACK with its INVITE.
    /// According to RFC 3261 section 8.1.1.4, Call-ID must be identical for all
    /// requests and responses in a dialog, including the ACK for a final response.
    ///
    /// # Returns
    /// 
    /// Some(call_id) if the transaction has an original request with a Call-ID header,
    /// None otherwise.
    fn original_request_call_id(&self) -> Option<String> {
        if let Some(req) = self.original_request_sync() {
            req.call_id().map(|hdr| hdr.value().to_string())
        } else {
            None
        }
    }
    
    /// Gets the From tag from the original request that created this transaction.
    ///
    /// From tag is part of the dialog identifiers used to match ACK with its INVITE.
    /// According to RFC 3261 section 8.1.1.7, the From tag must be identical for all
    /// requests and responses in a dialog (including ACK and CANCEL).
    ///
    /// # Returns
    /// 
    /// Some(from_tag) if the transaction has an original request with a From tag,
    /// None otherwise.
    fn original_request_from_tag(&self) -> Option<String> {
        if let Some(req) = self.original_request_sync() {
            req.from_tag()
        } else {
            None
        }
    }
    
    /// Gets the To tag from the original request that created this transaction.
    ///
    /// To tag may be part of the dialog identifiers used to match ACK with its INVITE.
    /// In early dialogs, the original INVITE may not have a To tag, but subsequent
    /// ACKs for final responses will include the To tag from the response.
    ///
    /// # Returns
    /// 
    /// Some(to_tag) if the transaction has an original request with a To tag,
    /// None otherwise.
    fn original_request_to_tag(&self) -> Option<String> {
        if let Some(req) = self.original_request_sync() {
            req.to_tag()
        } else {
            None
        }
    }
    
    /// Synchronous accessor for the original request if it's available without async operations.
    /// This is an internal helper method that should be implemented by transaction types
    /// that can provide synchronous access to the original request.
    ///
    /// # Returns
    /// 
    /// Some(Request) if the transaction has cached the original request,
    /// None if it would require an async operation to retrieve.
    fn original_request_sync(&self) -> Option<Request> {
        None
    }
}

/// Extension trait for Transaction to safely downcast to ServerTransaction.
///
/// This trait provides a convenience method for downcasting any Transaction object
/// to a ServerTransaction reference, making it easier to work with transaction-specific
/// functionality without unsafe code.
pub trait TransactionExt {
    /// Attempts to downcast to a ServerTransaction reference.
    ///
    /// # Returns
    ///
    /// Some(&dyn ServerTransaction) if the transaction is a server transaction,
    /// None otherwise.
    fn as_server_transaction(&self) -> Option<&dyn ServerTransaction>;
}

impl<T: Transaction + ?Sized> TransactionExt for T {
    fn as_server_transaction(&self) -> Option<&dyn ServerTransaction> {
        use crate::transaction::TransactionKind;
        
        match self.kind() {
            TransactionKind::InviteServer | TransactionKind::NonInviteServer => {
                // Get the Any representation and try downcasting
                self.as_any().downcast_ref::<Box<dyn ServerTransaction>>()
                    .map(|boxed| boxed.as_ref())
                    .or_else(|| {
                        // Try with specific implementations
                        use crate::transaction::server::{ServerInviteTransaction, ServerNonInviteTransaction};
                        
                        if let Some(tx) = self.as_any().downcast_ref::<ServerInviteTransaction>() {
                            Some(tx as &dyn ServerTransaction)
                        } else if let Some(tx) = self.as_any().downcast_ref::<ServerNonInviteTransaction>() {
                            Some(tx as &dyn ServerTransaction)
                        } else {
                            None
                        }
                    })
            },
            _ => None,
        }
    }
} 