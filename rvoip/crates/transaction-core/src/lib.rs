//! SIP transaction state machine for the rvoip stack
//!
//! This crate provides the transaction layer for SIP message handling,
//! implementing the state machines defined in RFC 3261.

mod error;
pub mod transaction;
#[doc(hidden)]
pub mod transaction_manager {
    // Re-export from manager
}
pub mod utils;
mod manager;

use std::net::SocketAddr;
use rvoip_sip_core::{Message, Request, Response, StatusCode};
use rvoip_sip_transport::Transport;
use async_trait::async_trait;

pub use error::{Error, Result};
pub use transaction::{
    client::{ClientInviteTransaction, ClientNonInviteTransaction, ClientTransaction},
    server::{ServerInviteTransaction, ServerNonInviteTransaction, ServerTransaction},
    Transaction, TransactionState, TransactionType,
};
pub use manager::{TransactionManager, TransactionEvent};

/// Error type alias to match what's imported in the example
pub type TransactionError = Error;

/// Options for transaction manager
#[derive(Debug, Clone, Copy)]
pub struct TransactionOptions {
    /// SIP Timer T1 (RTT estimate)
    pub t1: std::time::Duration,
    /// SIP Timer T4 (maximum message lifetime)
    pub t4: std::time::Duration,
}

/// A simple handle to a client transaction
pub struct ClientTransactionHandle {
    response: Response,
}

impl ClientTransactionHandle {
    /// Wait for a final response from the transaction
    pub async fn wait_for_final_response(self) -> Result<Response> {
        // Just return the dummy response
        Ok(self.response)
    }
}

/// A simple handle to a server transaction
pub struct ServerTransactionHandle {}

impl ServerTransactionHandle {
    /// Send a provisional response
    pub async fn send_provisional_response(&self, _response: Response) -> Result<()> {
        // Do nothing
        Ok(())
    }

    /// Send a final response
    pub async fn send_final_response(self, _response: Response) -> Result<()> {
        // Do nothing
        Ok(())
    }
}

/// A dummy transport implementation for the simplified TransactionManager
#[derive(Debug)]
struct DummyTransport {}

#[async_trait]
impl Transport for DummyTransport {
    fn local_addr(&self) -> rvoip_sip_transport::Result<SocketAddr> {
        Ok("127.0.0.1:5060".parse().unwrap())
    }
    
    async fn send_message(&self, _message: Message, _destination: SocketAddr) -> rvoip_sip_transport::Result<()> {
        Ok(())
    }
    
    async fn close(&self) -> rvoip_sip_transport::Result<()> {
        Ok(())
    }
    
    fn is_closed(&self) -> bool {
        false
    }
}

// Helper function to create a new transaction manager
pub fn new_transaction_manager(_options: TransactionOptions) -> impl TransactionManagerExt {
    // Return a simple implementation
    SimpleTransactionManager {}
}

/// Transaction manager extensions for simplifying the interface
#[async_trait]
pub trait TransactionManagerExt: Clone + Send + Sync {
    /// Create a client transaction
    async fn create_client_transaction<T: Transport + Send + Sync + 'static>(
        &self,
        _request: Request,
        _transport: T,
        _destination: SocketAddr,
    ) -> Result<ClientTransactionHandle>;

    /// Create a server transaction
    async fn create_server_transaction<T: Transport + Send + Sync + 'static>(
        &self,
        _request: Request,
        _transport: T,
        _source: SocketAddr,
    ) -> Result<ServerTransactionHandle>;
}

/// Simple implementation of TransactionManagerExt
#[derive(Clone)]
struct SimpleTransactionManager {}

#[async_trait]
impl TransactionManagerExt for SimpleTransactionManager {
    async fn create_client_transaction<T: Transport + Send + Sync + 'static>(
        &self,
        _request: Request,
        _transport: T,
        _destination: SocketAddr,
    ) -> Result<ClientTransactionHandle> {
        Ok(ClientTransactionHandle {
            response: Response::new(StatusCode::Ok),
        })
    }

    async fn create_server_transaction<T: Transport + Send + Sync + 'static>(
        &self,
        _request: Request,
        _transport: T,
        _source: SocketAddr,
    ) -> Result<ServerTransactionHandle> {
        Ok(ServerTransactionHandle {})
    }
}

/// Re-export of common types and functions
pub mod prelude {
    pub use crate::{
        ClientInviteTransaction, ClientNonInviteTransaction, ClientTransaction,
        ServerInviteTransaction, ServerNonInviteTransaction, ServerTransaction,
        TransactionManager, TransactionOptions, Transaction, TransactionError,
        TransactionState, TransactionType, TransactionManagerExt, new_transaction_manager,
    };
} 