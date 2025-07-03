/// # Transaction Manager Type Definitions
///
/// This module defines specific types used by the TransactionManager to handle
/// SIP transactions according to RFC 3261. These types support the transaction
/// layer operations including timer management, message routing, and transaction
/// state tracking.
///
/// The SIP transaction layer plays a critical role in ensuring reliable message
/// delivery over potentially unreliable transport layers. These types provide
/// the necessary structure to implement the state machines and timer-based
/// behaviors that SIP transactions require.

use std::net::SocketAddr;
use std::time::Instant;

/// Transaction timer data for transaction layer timers (A-K)
///
/// SIP transactions employ a variety of timers to ensure reliability and
/// proper state transitions. RFC 3261 Sections 17.1.1.2, 17.1.2.2, 17.2.1,
/// and 17.2.2 define the timers for each transaction type.
///
/// This structure associates timer data with specific transactions to enable
/// the timer manager to properly track and fire the timers.
///
/// ## SIP Transaction Timers
/// 
/// - Timer A: INVITE retransmission interval, for UDP only
/// - Timer B: INVITE transaction timeout
/// - Timer C: Proxy INVITE transaction timeout 
/// - Timer D: Wait time for response retransmissions
/// - Timer E: Non-INVITE retransmission interval, for UDP only
/// - Timer F: Non-INVITE transaction timeout
/// - Timer G: INVITE response retransmission interval
/// - Timer H: Wait time for ACK receipt
/// - Timer I: Wait time for ACK retransmissions
/// - Timer J: Wait time for retransmissions of non-INVITE requests
/// - Timer K: Wait time for response retransmissions
pub struct TransactionTimer {
    /// Transaction ID that owns this timer
    pub transaction_id: String,
    /// When the timer should fire
    pub expiry: Instant,
}

/// Represents a stray request that doesn't match any transaction
///
/// According to RFC 3261 Section 17.1.3 and 17.2.3, incoming requests should be
/// matched to existing transactions. When no match is found, the request is considered
/// "stray" and needs to be either handled specially (for methods like ACK and CANCEL)
/// or processed as a new transaction.
///
/// This structure encapsulates a stray request and its source for processing by
/// the TransactionManager.
#[derive(Debug, Clone)]
pub struct StrayRequest {
    /// The SIP request that couldn't be matched to any transaction
    pub request: rvoip_sip_core::Request,
    /// The source address from which the request was received
    pub source: SocketAddr,
} 