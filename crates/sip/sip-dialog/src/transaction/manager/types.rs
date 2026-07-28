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
use std::time::{Duration, Instant};

use bytes::Bytes;
use rvoip_sip_core::{Request, Response};
use rvoip_sip_transport::TransportRoute;

use crate::transaction::TransactionKey;

/// Dialog identifiers used to route 2xx ACKs back to their INVITE server
/// transaction without scanning every active server transaction.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub(crate) struct ServerInviteDialogKey {
    pub(crate) call_id: String,
    pub(crate) from_tag: String,
    pub(crate) to_tag: Option<String>,
}

impl ServerInviteDialogKey {
    pub(crate) fn from_request(request: &Request) -> Option<Self> {
        Some(Self {
            call_id: request.call_id()?.value().to_string(),
            from_tag: request.from_tag()?,
            to_tag: request.to_tag(),
        })
    }

    pub(crate) fn ack_lookup_keys(request: &Request) -> Option<(Self, Option<Self>)> {
        let exact = Self::from_request(request)?;
        let fallback = exact.to_tag.as_ref().map(|_| Self {
            call_id: exact.call_id.clone(),
            from_tag: exact.from_tag.clone(),
            to_tag: None,
        });
        Some((exact, fallback))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ServerInviteAckIndexEntry {
    pub(crate) transaction_id: TransactionKey,
    pub(crate) expires_at: Option<Instant>,
    /// Monotonic identity assigned by the manager whenever this dialog-key
    /// binding changes. Retention deadlines carry the same generation so an
    /// old deadline cannot remove a newer binding for a reused dialog key.
    pub(crate) deadline_generation: u64,
    pub(crate) _admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
}

#[derive(Debug, Clone)]
pub(crate) struct Invite2xxResponseCacheEntry {
    pub(crate) response: Response,
    pub(crate) wire_bytes: Bytes,
    pub(crate) route: TransportRoute,
    pub(crate) created_at: Instant,
    pub(crate) acked_at: Option<Instant>,
    pub(crate) expires_at: Instant,
    pub(crate) next_retransmit_at: Instant,
    pub(crate) retransmit_interval: Duration,
    /// Identity of the exact retransmit/expiry deadline currently registered
    /// with the manager. A superseded deadline cannot mutate or remove a
    /// replacement cache entry that happens to reuse the same transaction key.
    pub(crate) deadline_generation: u64,
    pub(crate) _admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
}

impl Invite2xxResponseCacheEntry {
    pub(crate) fn is_expired(&self, now: Instant) -> bool {
        now >= self.expires_at
    }
}

impl ServerInviteAckIndexEntry {
    #[cfg(test)]
    pub(crate) fn active(transaction_id: TransactionKey) -> Self {
        Self {
            transaction_id,
            expires_at: None,
            deadline_generation: 0,
            _admission_owner: None,
        }
    }

    pub(crate) fn active_with_owner(
        transaction_id: TransactionKey,
        admission_owner: Option<crate::transaction::manager::TransactionAdmissionOwner>,
    ) -> Self {
        Self {
            transaction_id,
            expires_at: None,
            deadline_generation: 0,
            _admission_owner: admission_owner,
        }
    }

    pub(crate) fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|expires_at| now >= expires_at)
    }
}

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
