use std::net::SocketAddr;
use async_trait::async_trait;

use crate::error::Result;
use crate::call::types::CallState;

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
    
    /// Update call state in the registry
    async fn update_call_state(&self, call_id: &str, previous_state: CallState, new_state: CallState) -> Result<()>;
    
    /// Update dialog information for a call
    async fn update_dialog_info(&self, 
        call_id: &str,
        dialog_id: Option<String>,
        dialog_state: Option<String>,
        local_tag: Option<String>,
        remote_tag: Option<String>,
        local_seq: Option<u32>,
        remote_seq: Option<u32>,
        route_set: Option<Vec<String>>,
        remote_target: Option<String>,
        secure: Option<bool>
    ) -> Result<()>;
} 