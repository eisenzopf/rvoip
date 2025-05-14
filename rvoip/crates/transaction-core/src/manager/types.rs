use std::net::SocketAddr;
use std::time::Instant;

/// Transaction timer data
pub struct TransactionTimer {
    /// Transaction ID
    pub transaction_id: String,
    /// When the timer should fire
    pub expiry: Instant,
}

// Define StrayRequest variant if it doesn't exist in TransactionEvent
#[derive(Debug, Clone)]
pub struct StrayRequest {
    pub request: rvoip_sip_core::Request,
    pub source: SocketAddr,
} 