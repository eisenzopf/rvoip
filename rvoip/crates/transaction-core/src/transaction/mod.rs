use std::fmt;
use std::time::Duration;

use rvoip_sip_core::{Message, Method, Request, Response};
use rvoip_sip_transport::Transport;

pub mod client;
pub mod server;

/// SIP transaction type (client or server)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionType {
    /// Client transaction (sending request, receiving responses)
    Client,
    /// Server transaction (receiving request, sending responses)
    Server,
}

/// SIP transaction states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    // Common states
    /// Initial state
    Initial,
    /// Transaction is completed
    Completed,
    /// Transaction is terminated
    Terminated,
    
    // Client INVITE transaction states
    /// Calling state (INVITE sent)
    Calling,
    /// Proceeding state (provisional response received)
    Proceeding,
    
    // Server INVITE transaction states
    /// Proceeding state (request received, provisional response sent)
    ServerProceeding,
    /// Confirmed state (ACK received)
    Confirmed,
    
    // Non-INVITE transaction states
    /// Trying state (request received or sent)
    Trying,
}

/// SIP transaction interface
#[async_trait::async_trait]
pub trait Transaction: fmt::Debug + Send + Sync {
    /// Get the transaction ID
    fn id(&self) -> &str;
    
    /// Get the transaction type
    fn transaction_type(&self) -> TransactionType;
    
    /// Get the current transaction state
    fn state(&self) -> TransactionState;
    
    /// Get the original request that created this transaction
    fn original_request(&self) -> &Request;
    
    /// Get the last response (if any)
    fn last_response(&self) -> Option<&Response>;
    
    /// Process an incoming message
    async fn process_message(&mut self, message: Message) -> crate::Result<Option<Message>>;
    
    /// Check if this transaction matches the given message
    fn matches(&self, message: &Message) -> bool;
    
    /// Check if the transaction is completed
    fn is_completed(&self) -> bool {
        matches!(self.state(), 
            TransactionState::Completed | 
            TransactionState::Confirmed | 
            TransactionState::Terminated
        )
    }
    
    /// Check if the transaction is terminated
    fn is_terminated(&self) -> bool {
        self.state() == TransactionState::Terminated
    }
    
    /// Get the timeout duration for the current state
    fn timeout_duration(&self) -> Option<Duration>;
    
    /// Handle a timeout event
    async fn on_timeout(&mut self) -> crate::Result<Option<Message>>;
} 