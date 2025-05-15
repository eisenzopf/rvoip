mod common;
mod invite;
mod non_invite;
mod data;

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

/// Common interface for client transactions
pub trait ClientTransaction: Transaction + Send + Sync + 'static {
    /// Initiate the transaction by sending the first request.
    /// This starts timers E/F for non-INVITE or A/B for INVITE.
    fn initiate(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Process an incoming response for this transaction.
    fn process_response(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Get original request
    fn original_request<'a>(&'a self) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + 'a>>;
    
    /// Get the last response received by this transaction
    fn last_response<'a>(&'a self) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + 'a>>;
}

/// Extension trait for Transaction to safely downcast to ClientTransaction
pub trait TransactionExt {
    /// Try to downcast to a ClientTransaction reference
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