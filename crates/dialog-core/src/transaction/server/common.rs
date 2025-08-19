/// # Common Server Transaction Functionality
///
/// This module provides common utility functions and traits used by both INVITE and non-INVITE
/// server transactions as defined in RFC 3261 Section 17.2.
///
/// The common functionality includes:
/// - Processing request retransmissions
/// - Sending responses
/// - Handling transaction state changes
/// - Managing server transaction data

use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, trace, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;

use async_trait::async_trait;

use crate::transaction::server::{ServerTransaction, ServerTransactionData};
use crate::transaction::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionState, TransactionKey, TransactionEvent,
    InternalTransactionCommand
};
use crate::transaction::timer::{TimerType, TimerSettings};
use crate::transaction::utils;

/// Common functionality for all server transaction types.
///
/// This trait defines utility methods shared by both INVITE and non-INVITE server
/// transactions to reduce code duplication and provide consistent behavior.
pub trait CommonServerTransaction {
    /// Returns the transaction data structure containing state and communication channels.
    ///
    /// # Returns
    ///
    /// A reference to the shared ServerTransactionData for this transaction.
    fn data(&self) -> &Arc<ServerTransactionData>;
    
    /// Common implementation for processing requests.
    ///
    /// This method handles the common logic for processing incoming requests to a server
    /// transaction. It forwards the request to the transaction's event loop for
    /// state-specific processing according to RFC 3261 Section 17.2.
    ///
    /// # Arguments
    ///
    /// * `request` - The SIP request to process
    ///
    /// # Returns
    ///
    /// A Future that resolves to Ok(()) if the request was processed successfully,
    /// or an Error if there was a problem.
    fn process_request_common(&self, request: Request) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            data.cmd_tx.send(InternalTransactionCommand::ProcessMessage(Message::Request(request))).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))
        })
    }
    
    /// Sends a command to this transaction's event loop.
    ///
    /// This helper method is used to send commands to the transaction's event loop
    /// for asynchronous processing. It's used by transaction-specific methods that
    /// need to interact with the transaction's internal state machine.
    ///
    /// # Arguments
    ///
    /// * `cmd` - The command to send to the transaction
    ///
    /// # Returns
    ///
    /// A Future that resolves to Ok(()) if the command was sent successfully,
    /// or an Error if there was a problem.
    fn send_command_common(&self, cmd: InternalTransactionCommand) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            data.cmd_tx.send(cmd).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))
        })
    }
    
    /// Returns the original request that initiated this transaction.
    ///
    /// This helper method is used to access the original request stored in the transaction data.
    /// It's useful for creating responses or handling retransmissions.
    ///
    /// # Returns
    ///
    /// A Future that resolves to the original SIP request.
    fn original_request_common(&self) -> Pin<Box<dyn Future<Output = Request> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            let request_guard = data.request.lock().await;
            request_guard.clone()
        })
    }
    
    /// Returns the last response sent by this transaction.
    ///
    /// This helper method is used to access the last response stored in the transaction data.
    /// It's particularly useful for handling request retransmissions, where the server
    /// should resend the last response.
    ///
    /// # Returns
    ///
    /// A Future that resolves to the last sent SIP response, or None if no response
    /// has been sent yet.
    fn last_response_common(&self) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            let response_guard = data.last_response.lock().await;
            response_guard.clone()
        })
    }
} 