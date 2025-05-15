/// # Common Client Transaction Functionality
///
/// This module provides common utility functions and traits used by both INVITE and non-INVITE
/// client transactions as defined in RFC 3261 Section 17.1.
///
/// The common functionality includes:
/// - Processing responses
/// - Sending commands to the transaction's event loop
/// - Accessing transaction state and data

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

use crate::client::{ClientTransaction, ClientTransactionData};
use crate::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand
};
use crate::timer::{TimerType, TimerSettings};
use crate::utils;

/// Common functionality for all client transaction types.
///
/// This trait defines utility methods shared by both INVITE and non-INVITE client
/// transactions to reduce code duplication and provide consistent behavior.
pub trait CommonClientTransaction {
    /// Returns the transaction data structure containing state and communication channels.
    ///
    /// # Returns
    ///
    /// A reference to the shared ClientTransactionData for this transaction.
    fn data(&self) -> &Arc<ClientTransactionData>;
    
    /// Processes a SIP response based on transaction kind and current state.
    ///
    /// This method handles the common logic for processing incoming responses to a client
    /// transaction. It stores the response in the transaction data and forwards it to the
    /// transaction's event loop for state-specific processing according to RFC 3261 Section 17.1.
    ///
    /// # Arguments
    ///
    /// * `response` - The SIP response to process
    ///
    /// # Returns
    ///
    /// A Future that resolves to Ok(()) if the response was processed successfully,
    /// or an Error if there was a problem.
    fn process_response_common(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            // Store the response
            {
                let mut last_response = data.last_response.lock().await;
                *last_response = Some(response.clone());
            }
            
            // Send a command to the transaction to process the response
            data.cmd_tx.send(InternalTransactionCommand::ProcessMessage(Message::Response(response))).await
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
    /// It's used by transaction-specific methods that need to reference the original request,
    /// such as when generating ACK requests for INVITE transactions.
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
    
    /// Returns the last response received by this transaction.
    ///
    /// This helper method is used to access the last response stored in the transaction data.
    /// It's used by transaction-specific methods that need to reference the last response.
    ///
    /// # Returns
    ///
    /// A Future that resolves to the last received SIP response, or None if no response
    /// has been received yet.
    fn last_response_common(&self) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            let response_guard = data.last_response.lock().await;
            response_guard.clone()
        })
    }
} 