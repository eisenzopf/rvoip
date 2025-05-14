mod common;
mod invite;
mod non_invite;
mod data;
mod cancel;
mod update;

pub use common::*;
pub use invite::ServerInviteTransaction;
pub use non_invite::ServerNonInviteTransaction;
pub use cancel::ServerCancelTransaction;
pub use update::ServerUpdateTransaction;
pub use data::{ServerTransactionData, CommandSender, CommandReceiver, CommonServerTransaction};

use async_trait::async_trait;
use std::net::SocketAddr;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::error::Result;
use crate::transaction::{Transaction, TransactionState, TransactionKey, TransactionAsync};
use rvoip_sip_core::prelude::*;

/// Common interface for server transactions
#[async_trait]
pub trait ServerTransaction: Transaction + TransactionAsync + Send + Sync + 'static {
    /// Process an incoming request associated with this transaction (e.g., retransmission, ACK, CANCEL).
    fn process_request(&self, request: Request) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;

    /// Send a response for this transaction. Initiates state transitions and timers.
    fn send_response(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
}

/// Extension trait for Transaction to safely downcast to ServerTransaction
pub trait TransactionExt {
    /// Try to downcast to a ServerTransaction reference
    fn as_server_transaction(&self) -> Option<&dyn ServerTransaction>;
}

impl<T: Transaction + ?Sized> TransactionExt for T {
    fn as_server_transaction(&self) -> Option<&dyn ServerTransaction> {
        use crate::transaction::TransactionKind;
        
        match self.kind() {
            TransactionKind::InviteServer | TransactionKind::NonInviteServer | TransactionKind::CancelServer | TransactionKind::UpdateServer => {
                // Get the Any representation and try downcasting
                self.as_any().downcast_ref::<Box<dyn ServerTransaction>>()
                    .map(|boxed| boxed.as_ref())
                    .or_else(|| {
                        // Try with specific implementations
                        use crate::server::{ServerInviteTransaction, ServerNonInviteTransaction, ServerCancelTransaction, ServerUpdateTransaction};
                        
                        if let Some(tx) = self.as_any().downcast_ref::<ServerInviteTransaction>() {
                            Some(tx as &dyn ServerTransaction)
                        } else if let Some(tx) = self.as_any().downcast_ref::<ServerNonInviteTransaction>() {
                            Some(tx as &dyn ServerTransaction)
                        } else if let Some(tx) = self.as_any().downcast_ref::<ServerCancelTransaction>() {
                            Some(tx as &dyn ServerTransaction)
                        } else if let Some(tx) = self.as_any().downcast_ref::<ServerUpdateTransaction>() {
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