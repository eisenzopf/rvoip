/// # Transaction Manager for SIP Protocol
///
/// This module implements the TransactionManager, which is the central component
/// of the RFC 3261 SIP transaction layer. It manages all four transaction types
/// defined in the specification:
///
/// - INVITE client transactions (ICT)
/// - Non-INVITE client transactions (NICT)
/// - INVITE server transactions (IST)
/// - Non-INVITE server transactions (NIST)
///
/// ## Transaction Layer Architecture
///
/// In the SIP protocol stack, the transaction layer sits between the transport layer
/// and the Transaction User (TU) layer, fulfilling RFC 3261 Section 17:
///
/// ```text
/// +---------------------------+
/// |  Transaction User (TU)    |  <- Dialogs, call control, etc.
/// |  (UAC, UAS, Proxy)        |
/// +---------------------------+
///              ↑ ↓
///              | |  Transaction events, requests, responses
///              ↓ ↑
/// +---------------------------+
/// |  Transaction Layer        |  <- This module
/// |  (Manager + Transactions) |
/// +---------------------------+
///              ↑ ↓
///              | |  Messages, transport events
///              ↓ ↑
/// +---------------------------+
/// |  Transport Layer          |  <- UDP, TCP, etc.
/// +---------------------------+
/// ```
///
/// ## Transaction Manager Responsibilities
/// 
/// The TransactionManager's primary responsibilities include:
///
/// 1. **Transaction Creation**: Creating the appropriate transaction type for client or server operations
/// 2. **Message Matching**: Matching incoming messages to existing transactions 
/// 3. **State Machine Management**: Managing transaction state transitions
/// 4. **Timer Coordination**: Managing transaction timers (A, B, C, D, E, F, G, H, I, J, K)
/// 5. **Message Retransmission**: Handling reliable delivery over unreliable transports
/// 6. **Event Distribution**: Notifying the TU of transaction events
/// 7. **Special Method Handling**: Processing special methods like ACK and CANCEL
///
/// ## Transaction Matching Rules (RFC 3261 Section 17.1.3 and 17.2.3)
///
/// The manager implements these matching rules to route incoming messages:
///
/// - For client transactions (responses):
///   - Match the branch parameter in the top Via header
///   - Match the sent-by value in the top Via header
///   - Match the method in the CSeq header
///
/// - For server transactions (requests):
///   - Match the branch parameter in the top Via header
///   - Match the sent-by value in the top Via header
///   - Match the request method (except for ACK)
///
/// ## Transaction State Machines
///
/// The TransactionManager orchestrates four distinct state machines defined in RFC 3261:
///
/// ```text
///       INVITE Client Transaction          Non-INVITE Client Transaction
///             (Section 17.1.1)                  (Section 17.1.2)
///
///               |INVITE                           |Request
///               V                                 V
///    +-------+                         +-------+
///    |Calling|-------------+           |Trying |------------+
///    +-------+             |           +-------+            |
///        |                 |               |                |
///        |1xx              |               |1xx             |
///        |                 |               |                |
///        V                 |               V                |
///    +----------+          |           +----------+         |
///    |Proceeding|          |           |Proceeding|         |
///    +----------+          |           +----------+         |
///        |                 |               |                |
///        |200-699          |               |200-699         |
///        |                 |               |                |
///        V                 |               V                |
///    +---------+           |           +---------+          |
///    |Completed|<----------+           |Completed|<---------+
///    +---------+                       +---------+
///        |                                 |
///        |                                 |
///        V                                 V
///    +-----------+                    +-----------+
///    |Terminated |                    |Terminated |
///    +-----------+                    +-----------+
///
///
///       INVITE Server Transaction          Non-INVITE Server Transaction
///             (Section 17.2.1)                  (Section 17.2.2)
///
///               |INVITE                           |Request
///               V                                 V
///    +----------+                        +----------+
///    |Proceeding|--+                     |  Trying  |
///    +----------+  |                     +----------+
///        |         |                         |
///        |1xx      |1xx                      |
///        |         |                         |
///        |         v                         v
///        |      +----------+              +----------+
///        |      |Proceeding|---+          |Proceeding|---+
///        |      +----------+   |          +----------+   |
///        |         |           |              |          |
///        |         |2xx        |              |2xx       |
///        |         |           |              |          |
///        v         v           |              v          |
///    +----------+  |           |          +----------+   |
///    |Completed |<-+-----------+          |Completed |<--+
///    +----------+                         +----------+
///        |                                    |
///        |                                    |
///        V                                    V
///    +-----------+                       +-----------+
///    |Terminated |                       |Terminated |
///    +-----------+                       +-----------+
/// ```
///
/// ## Special Method Handling
///
/// The TransactionManager implements special handling for:
///
/// - **ACK for non-2xx responses**: Automatically generated by the transaction layer (RFC 3261 17.1.1.3)
/// - **ACK for 2xx responses**: Generated by the TU, not by the transaction layer
/// - **CANCEL**: Requires matching to an existing INVITE transaction (RFC 3261 Section 9.1)
/// - **UPDATE**: Follows RFC 3311 rules for in-dialog requests
///
/// ## Event Flow Between Layers
///
/// The TransactionManager facilitates communication between the Transport layer and the TU:
///
/// ```text
///   TU (Transaction User)
///      ↑        ↓
///      |        | - Requests to send
///      |        | - Responses to send
///      |        | - Commands (e.g., CANCEL)
/// Events|        |
///      |        |
///      ↓        ↑
///   Transaction Manager
///      ↑        ↓
///      |        | - Outgoing messages
///      |        |
/// Events|        |
///      |        |
///      ↓        ↑
///   Transport Layer
/// ```

mod handlers;
mod types;
pub mod utils;
mod functions;
mod constructors;
mod operations;
mod creation;
#[cfg(test)]
mod tests;

pub use types::*;
pub use handlers::*;
pub use utils::*;
use functions::*;

use std::collections::HashMap;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::str::FromStr;
use std::pin::Pin;
use std::future::Future;

use tokio::sync::{Mutex, mpsc};
use tokio::time::sleep;
use tracing::{debug, error, info, warn, trace};
use async_trait::async_trait;

use rvoip_sip_core::prelude::*;
use rvoip_sip_core::{Host, TypedHeader};
use rvoip_sip_transport::{Transport, TransportEvent};
use rvoip_sip_transport::transport::TransportType;

use crate::transaction::error::{self, Error, Result};
use crate::transaction::{
    Transaction, TransactionAsync, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand,
};
use crate::transaction::state::TransactionLifecycle;
use crate::transaction::client::{
    ClientTransaction, 
    ClientInviteTransaction, 
    ClientNonInviteTransaction,
    TransactionExt,
    CommonClientTransaction,
};
use crate::transaction::runner::HasLifecycle;
use crate::transaction::server::{ServerTransaction, ServerInviteTransaction, ServerNonInviteTransaction, CommonServerTransaction};
use crate::transaction::timer::{Timer, TimerManager, TimerFactory, TimerSettings};
use crate::transaction::method::{cancel, update, ack};
use crate::transaction::utils::{transaction_key_from_message, generate_branch, create_ack_from_invite};
use crate::transaction::transport::{
    TransportCapabilities, TransportInfo, 
    NetworkInfoForSdp, WebSocketStatus, TransportCapabilitiesExt
};

// Type aliases without Sync requirement
type BoxedTransaction = Box<dyn Transaction + Send>;
/// Type alias for a shared client transaction (Arc enables clone-under-lock without removing)
type BoxedClientTransaction = Arc<dyn ClientTransaction + Send + Sync>;
/// Type alias for an Arc wrapped server transaction
type BoxedServerTransaction = Arc<dyn ServerTransaction>;

/// Defines the public API for the RFC 3261 SIP Transaction Manager.
///
/// The TransactionManager coordinates all SIP transaction activities, including
/// creation, processing of messages, and event delivery.
///
/// It implements the four core transaction types defined in RFC 3261:
/// - INVITE client transactions (ICT)
/// - Non-INVITE client transactions (NICT)
/// - INVITE server transactions (IST)
/// - Non-INVITE server transactions (NIST)
///
/// Special methods (CANCEL, ACK for 2xx, UPDATE) are handled through utility
/// functions that work with these four core transaction types.
#[derive(Clone)]
pub struct TransactionManager {
    /// Transport to use for messages
    transport: Arc<dyn Transport>,
    /// Active client transactions
    client_transactions: Arc<Mutex<HashMap<TransactionKey, BoxedClientTransaction>>>,
    /// Active server transactions
    server_transactions: Arc<Mutex<HashMap<TransactionKey, Arc<dyn ServerTransaction>>>>,
    /// Transaction destinations - maps transaction IDs to their destinations
    transaction_destinations: Arc<Mutex<HashMap<TransactionKey, SocketAddr>>>,
    /// Event sender
    events_tx: mpsc::Sender<TransactionEvent>,
    /// Additional event subscribers
    event_subscribers: Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
    /// Maps subscribers to transactions they're interested in
    subscriber_to_transactions: Arc<Mutex<HashMap<usize, Vec<TransactionKey>>>>,
    /// Maps transactions to subscribers interested in them
    transaction_to_subscribers: Arc<Mutex<HashMap<TransactionKey, Vec<usize>>>>,
    /// Subscriber counter for assigning unique IDs
    next_subscriber_id: Arc<Mutex<usize>>,
    /// Transport message channel
    transport_rx: Arc<Mutex<mpsc::Receiver<TransportEvent>>>,
    /// Running flag
    running: Arc<Mutex<bool>>,
    /// Timer configuration
    timer_settings: TimerSettings,
    /// Centralized timer manager
    timer_manager: Arc<TimerManager>,
    /// Timer factory
    timer_factory: TimerFactory,
    /// Broadcast shutdown signal for spawned tasks
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
}

// Define RFC3261 Branch magic cookie
pub const RFC3261_BRANCH_MAGIC_COOKIE: &str = "z9hG4bK";
