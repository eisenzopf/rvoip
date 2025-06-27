/// # Client Transaction Module
///
/// This module implements the client-side transaction state machines according to 
/// [RFC 3261 Section 17.1](https://datatracker.ietf.org/doc/html/rfc3261#section-17.1).
/// 
/// ## SIP Client Transactions
///
/// Client transactions are initiated by the Transaction User (TU) when it wants to send a request.
/// They ensure reliable delivery of requests, handle retransmissions, and deliver responses
/// back to the TU.
///
/// ## Transaction Types
///
/// RFC 3261 defines two types of client transactions with different state machines:
///
/// 1. **INVITE Client Transactions** (Section 17.1.1): Used for session establishment.
///    - More complex due to the three-way handshake required (INVITE, response, ACK)
///    - Special handling for ACK generation on non-2xx responses
///    - Unique timer requirements (Timers A, B, D)
///
/// 2. **Non-INVITE Client Transactions** (Section 17.1.2): Used for all other request types.
///    - Simpler state machine
///    - No ACK generation
///    - Different timer requirements (Timers E, F, K)
///
/// ## Implementation Details
///
/// Both transaction types share common infrastructure but implement different state machines:
///
/// - `ClientInviteTransaction`: Implements the INVITE client transaction state machine
/// - `ClientNonInviteTransaction`: Implements the non-INVITE client transaction state machine
/// - `ClientTransactionData`: Shared data structure for both transaction types
/// - `CommonClientTransaction`: Common behavior for client transactions
/// - `ClientTransaction`: Interface for all client transactions
///
/// ## Usage
///
/// Client transactions are typically created and managed by the `TransactionManager`, which routes
/// incoming messages to the appropriate transaction and exposes a clean API for the TU.

mod common;
mod invite;
mod non_invite;
mod data;
pub mod builders;

pub use common::*;
pub use invite::ClientInviteTransaction;
pub use non_invite::ClientNonInviteTransaction;
pub use data::{ClientTransactionData, CommandSender, CommandReceiver, CommonClientTransaction};

use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use std::future::Future;
use std::pin::Pin;

use crate::error::Result;
use crate::transaction::{Transaction, TransactionState, TransactionKey, TransactionAsync};
use rvoip_sip_core::prelude::*;
use rvoip_sip_core::Request;

/// Common interface for client transactions, implementing the behavior defined in RFC 3261 Section 17.1.
///
/// This trait defines operations that both INVITE and non-INVITE client transactions must support.
/// It encapsulates the functionality required to initiate transactions, process responses,
/// and track state according to the SIP specification.
pub trait ClientTransaction: Transaction + Send + Sync + 'static {
    /// Initiates the transaction by sending the first request.
    ///
    /// For INVITE transactions, this starts Timers A/B and moves the transaction to the Calling state.
    /// For non-INVITE transactions, this starts Timers E/F and moves the transaction to the Trying state.
    ///
    /// # Returns
    /// 
    /// A Future that resolves to Ok(()) if the transaction was initiated successfully,
    /// or an Error if there was a problem.
    fn initiate(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Processes an incoming response for this transaction.
    ///
    /// This method is called when a response is received that matches this transaction
    /// according to the rules in RFC 3261 Section 17.1.3.
    ///
    /// # Arguments
    ///
    /// * `response` - The SIP response to process
    ///
    /// # Returns
    ///
    /// A Future that resolves to Ok(()) if the response was processed successfully,
    /// or an Error if there was a problem.
    fn process_response(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Returns the original request that initiated this transaction.
    ///
    /// # Returns
    ///
    /// A Future that resolves to the original SIP request, or None if it's not available.
    fn original_request<'a>(&'a self) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + 'a>>;
    
    /// Returns the last response received by this transaction.
    ///
    /// # Returns
    ///
    /// A Future that resolves to the last received SIP response, or None if no response
    /// has been received yet.
    fn last_response<'a>(&'a self) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + 'a>>;
}

/// Extension trait for Transaction to safely downcast to ClientTransaction.
///
/// This trait provides a convenience method for downcasting any Transaction object
/// to a ClientTransaction reference, making it easier to work with transaction-specific
/// functionality without unsafe code.
pub trait TransactionExt {
    /// Attempts to downcast to a ClientTransaction reference.
    ///
    /// # Returns
    ///
    /// Some(&dyn ClientTransaction) if the transaction is a client transaction,
    /// None otherwise.
    fn as_client_transaction(&self) -> Option<&dyn ClientTransaction>;
}

impl<T: Transaction + ?Sized> TransactionExt for T {
    fn as_client_transaction(&self) -> Option<&dyn ClientTransaction> {
        use crate::transaction::TransactionKind;
        
        match self.kind() {
            TransactionKind::InviteClient | TransactionKind::NonInviteClient => {
                // Get the Any representation and try downcasting
                self.as_any().downcast_ref::<Box<dyn ClientTransaction>>()
                    .map(|boxed| boxed.as_ref())
                    .or_else(|| {
                        // Try with specific implementations
                        use crate::client::{ClientInviteTransaction, ClientNonInviteTransaction};
                        
                        if let Some(tx) = self.as_any().downcast_ref::<ClientInviteTransaction>() {
                            Some(tx as &dyn ClientTransaction)
                        } else if let Some(tx) = self.as_any().downcast_ref::<ClientNonInviteTransaction>() {
                            Some(tx as &dyn ClientTransaction)
                        } else {
                            None
                        }
                    })
            },
            _ => None,
        }
    }
} 