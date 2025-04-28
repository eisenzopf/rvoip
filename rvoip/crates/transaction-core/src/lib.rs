//! SIP transaction state machine for the rvoip stack
//!
//! This crate provides the transaction layer for SIP message handling,
//! implementing the state machines defined in RFC 3261.

// Declare modules
mod error;
mod manager;
pub mod transaction;
pub mod utils;

// Re-export core types
pub use error::{Error, Result};
pub use manager::TransactionManager;
pub use transaction::{
    client::{ClientInviteTransaction, ClientNonInviteTransaction, ClientTransaction},
    server::{ServerInviteTransaction, ServerNonInviteTransaction, ServerTransaction},
    Transaction, TransactionEvent, TransactionKey, TransactionKind, TransactionState,
    // TransactionType is now TransactionKind
};

// --- Optional Simplified API (Keep or remove based on design) ---

use rvoip_sip_core::prelude::{Request, Response, StatusCode};
use rvoip_sip_transport::Transport;
use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;

/// Error type alias (consider removing if `Error` is clear enough)
// pub type TransactionError = Error;

/// Options for transaction manager (if needed at this level)
#[derive(Debug, Clone)]
pub struct TransactionConfig {
    /// SIP Timer T1 (RTT estimate) - RFC 3261 default 500ms
    pub t1: std::time::Duration,
    // Other timers can be derived or configured within specific transactions if needed.
    // e.g., T2 = 4s, T4 = 5s
    // INVITE Client: Timer_B = 64*T1, Timer_D = >32s (unreliable)
    // INVITE Server: Timer_H = 64*T1, Timer_I = T4 (unreliable)
    // Non-INVITE Client: Timer_F = 64*T1, Timer_K = T4 (unreliable)
    // Non-INVITE Server: Timer_J = 64*T1 (unreliable)
}

impl Default for TransactionConfig {
    fn default() -> Self {
        TransactionConfig {
            t1: std::time::Duration::from_millis(500),
        }
    }
}

// The simplified API below might be removed in favor of directly using TransactionManager

/// A simple handle to a client transaction (Placeholder)
pub struct ClientTransactionHandle {
    pub key: TransactionKey,
    // Potentially hold a way to receive the final response, e.g., a oneshot::Receiver
}

/// A simple handle to a server transaction (Placeholder)
pub struct ServerTransactionHandle {
    pub key: TransactionKey,
    // Potentially hold a way to send responses, e.g., Arc<TransactionManager>
}

/// Simplified interface trait (Placeholder)
#[async_trait]
pub trait TransactionFacade: Clone + Send + Sync + 'static {
    async fn create_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<ClientTransactionHandle>;

    async fn send_client_request(&self, handle: &ClientTransactionHandle) -> Result<()>;

    // Add methods for server transactions if this facade is kept
}


/// Re-export of common types and functions for easier use.
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::manager::TransactionManager;
    pub use crate::transaction::{
        client::{ClientInviteTransaction, ClientNonInviteTransaction, ClientTransaction},
        server::{ServerInviteTransaction, ServerNonInviteTransaction, ServerTransaction},
        Transaction, TransactionEvent, TransactionKey, TransactionKind, TransactionState,
    };
    // pub use crate::TransactionConfig; // Only if config is part of public API
    // pub use crate::TransactionFacade; // Only if facade pattern is kept
}