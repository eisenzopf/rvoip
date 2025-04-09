use std::net::SocketAddr;
use async_trait::async_trait;

use crate::error::Result;

/// Interface for call registry methods needed by Call
#[async_trait]
pub trait CallRegistryInterface: std::fmt::Debug + Send + Sync {
    /// Log a transaction
    async fn log_transaction(&self, call_id: &str, transaction: crate::call_registry::TransactionRecord) -> Result<()>;
    
    /// Get transactions for a call
    async fn get_transactions(&self, call_id: &str) -> Result<Vec<crate::call_registry::TransactionRecord>>;
    
    /// Update transaction status
    async fn update_transaction_status(&self, call_id: &str, transaction_id: &str, status: &str, info: Option<String>) -> Result<()>;
    
    /// Get transaction destination (SocketAddr) from the registry, used for ACK fallback
    async fn get_transaction_destination(&self, call_id: &str) -> Result<Option<SocketAddr>>;
} 