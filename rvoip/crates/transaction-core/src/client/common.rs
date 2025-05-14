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

/// Common functionality for all client transaction types
pub trait CommonClientTransaction {
    /// Get the transaction data
    fn data(&self) -> &Arc<ClientTransactionData>;
    
    /// Process a response based on transaction kind and current state
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
    
    /// Helper to send a command to this transaction
    fn send_command_common(&self, cmd: InternalTransactionCommand) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            data.cmd_tx.send(cmd).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))
        })
    }
    
    /// Helper to get original request
    fn original_request_common(&self) -> Pin<Box<dyn Future<Output = Request> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            let request_guard = data.request.lock().await;
            request_guard.clone()
        })
    }
    
    /// Helper to get last response
    fn last_response_common(&self) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + '_>> {
        let data = self.data().clone();
        
        Box::pin(async move {
            let response_guard = data.last_response.lock().await;
            response_guard.clone()
        })
    }
} 