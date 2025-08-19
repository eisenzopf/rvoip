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
use crate::transaction::client::{
    ClientTransaction, 
    ClientInviteTransaction, 
    ClientNonInviteTransaction,
    TransactionExt,
};
use crate::transaction::server::{ServerTransaction, ServerInviteTransaction, ServerNonInviteTransaction};
use crate::transaction::timer::{Timer, TimerManager, TimerFactory, TimerSettings};
use crate::transaction::method::{cancel, update, ack};
use crate::transaction::utils::{transaction_key_from_message, generate_branch, create_ack_from_invite};
use crate::transaction::transport::{
    TransportCapabilities, TransportInfo, 
    NetworkInfoForSdp, WebSocketStatus, TransportCapabilitiesExt
};

// Type aliases without Sync requirement
type BoxedTransaction = Box<dyn Transaction + Send>;
/// Type alias for a boxed client transaction
type BoxedClientTransaction = Box<dyn ClientTransaction + Send>;
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
}

// Define RFC3261 Branch magic cookie
pub const RFC3261_BRANCH_MAGIC_COOKIE: &str = "z9hG4bK";

impl TransactionManager {
    /// Creates a new transaction manager with default settings.
    ///
    /// This async constructor sets up the transaction manager with default timer settings
    /// and starts the message processing loop. It is the preferred way to create a
    /// transaction manager in an async context.
    ///
    /// ## Transaction Manager Initialization
    ///
    /// The initialization process:
    /// 1. Sets up internal data structures for tracking transactions
    /// 2. Initializes the timer management system
    /// 3. Starts the message processing loop to handle transport events
    /// 4. Returns the manager and an event receiver for transaction events
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17: The transaction layer requires proper initialization
    /// - RFC 3261 Section 17.1.1.2 and 17.1.2.2: Timer initialization for retransmissions
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    /// * `transport_rx` - Channel for receiving transport events
    /// * `capacity` - Optional event queue capacity (defaults to 100)
    ///
    /// # Returns
    /// * `Result<(Self, mpsc::Receiver<TransactionEvent>)>` - The manager and event receiver
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use tokio::sync::mpsc;
    /// # use rvoip_sip_transport::{Transport, TransportEvent};
    /// # use crate::transaction::TransactionManager;
    /// # async fn example(transport: Arc<dyn Transport>) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a transport event channel
    /// let (transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(100);
    ///
    /// // Create transaction manager
    /// let (manager, event_rx) = TransactionManager::new(
    ///     transport,
    ///     transport_rx,
    ///     Some(200), // Buffer up to 200 events
    /// ).await?;
    ///
    /// // Now use the manager and listen for events
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        let events_capacity = capacity.unwrap_or(100);
        let (events_tx, events_rx) = mpsc::channel(events_capacity);
        
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        let subscriber_to_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_to_subscribers = Arc::new(Mutex::new(HashMap::new()));
        let next_subscriber_id = Arc::new(Mutex::new(0));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(Mutex::new(false));
        
        let timer_settings = TimerSettings::default();
        
        // Setup timer manager
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        
        // Create timer factory with the timer manager
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        let manager = Self {
            transport,
            client_transactions,
            server_transactions,
            transaction_destinations,
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
        };
        
        // Start the message processing loop
        manager.start_message_loop();
        
        Ok((manager, events_rx))
    }

    /// Creates a new transaction manager with custom timer configuration.
    ///
    /// This async constructor allows customizing the timer settings, which affect
    /// retransmission intervals and timeouts. This is useful for fine-tuning SIP
    /// transaction behavior in different network environments.
    ///
    /// ## Timer Configuration Importance
    ///
    /// SIP transactions rely heavily on timers for reliability:
    /// - Timer A, B: Control INVITE retransmissions and timeouts
    /// - Timer E, F: Control non-INVITE retransmissions and timeouts  
    /// - Timer G, H: Control INVITE response retransmissions
    /// - Timer I, J, K: Control various cleanup behaviors
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1.2: INVITE client transaction timers
    /// - RFC 3261 Section 17.1.2.2: Non-INVITE client transaction timers
    /// - RFC 3261 Section 17.2.1: INVITE server transaction timers
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction timers
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    /// * `transport_rx` - Channel for receiving transport events
    /// * `capacity` - Optional event queue capacity (defaults to 100)
    /// * `timer_settings` - Optional custom timer settings
    ///
    /// # Returns
    /// * `Result<(Self, mpsc::Receiver<TransactionEvent>)>` - The manager and event receiver
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # use tokio::sync::mpsc;
    /// # use rvoip_sip_transport::{Transport, TransportEvent};
    /// # use crate::transaction::{TransactionManager, timer::TimerSettings};
    /// # async fn example(transport: Arc<dyn Transport>) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create custom timer settings for high-latency networks
    /// let mut timer_settings = TimerSettings::default();
    /// timer_settings.t1 = Duration::from_millis(1000); // Increase base timer
    ///
    /// // Create a transport event channel
    /// let (transport_tx, transport_rx) = mpsc::channel::<TransportEvent>(100);
    ///
    /// // Create transaction manager with custom settings
    /// let (manager, event_rx) = TransactionManager::new_with_config(
    ///     transport,
    ///     transport_rx,
    ///     Some(200),
    ///     Some(timer_settings),
    /// ).await?;
    ///
    /// // Now use the manager with custom timer behavior
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new_with_config(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
        timer_settings: Option<TimerSettings>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        let events_capacity = capacity.unwrap_or(100);
        let (events_tx, events_rx) = mpsc::channel(events_capacity);
        
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        let subscriber_to_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_to_subscribers = Arc::new(Mutex::new(HashMap::new()));
        let next_subscriber_id = Arc::new(Mutex::new(0));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(Mutex::new(false));
        
        // Create timer settings
        let timer_settings = timer_settings.unwrap_or_default();
        
        // Create the timer manager with custom config
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        let manager = Self {
            transport,
            client_transactions,
            server_transactions,
            transaction_destinations,
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
        };
        
        // Start the message processing loop
        manager.start_message_loop();
        
        Ok((manager, events_rx))
    }

    /// Creates a transaction manager synchronously (without async).
    ///
    /// This constructor is provided for contexts where async initialization
    /// isn't possible. It creates a minimal transaction manager with dummy
    /// channels that will need to be properly connected later.
    ///
    /// Note: Using the async `new()` method is preferred in async contexts.
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    ///
    /// # Returns
    /// * `Self` - A transaction manager instance
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use rvoip_sip_transport::Transport;
    /// # use crate::transaction::TransactionManager;
    /// # fn example(transport: Arc<dyn Transport>) {
    /// // Create a transaction manager without async
    /// let manager = TransactionManager::new_sync(transport);
    /// 
    /// // Manager can now be used or passed to an async context
    /// # }
    /// ```
    pub fn new_sync(transport: Arc<dyn Transport>) -> Self {
        Self::with_config(transport, None)
    }
    
    /// Creates a new TransactionManager that uses a TransportManager for SIP transport.
    ///
    /// This method integrates the transaction layer with the transport manager, allowing
    /// for advanced transport capabilities such as multiple transport types, failover,
    /// and transport selection based on destination.
    ///
    /// # Arguments
    /// * `transport_manager` - The TransportManager to use for sending messages
    /// * `transport_rx` - Channel for receiving transport events
    /// * `capacity` - Optional event queue capacity (defaults to 100)
    ///
    /// # Returns
    /// * `Result<(Self, mpsc::Receiver<TransactionEvent>)>` - The manager and event receiver
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use std::net::SocketAddr;
    /// # use tokio::sync::mpsc;
    /// # use crate::transaction::{TransactionManager, transport::TransportManager, transport::TransportManagerConfig};
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a transport manager configuration
    /// let config = TransportManagerConfig {
    ///    bind_addresses: vec!["127.0.0.1:5060".parse().unwrap()],
    ///    enable_udp: true,
    ///    enable_tcp: true,
    ///    ..Default::default()
    /// };
    ///
    /// // Create and initialize the transport manager
    /// let (mut transport_manager, transport_rx) = TransportManager::new(config).await?;
    /// transport_manager.initialize().await?;
    ///
    /// // Create transaction manager with the transport manager
    /// let (transaction_manager, event_rx) = TransactionManager::with_transport_manager(
    ///     transport_manager,
    ///     transport_rx,
    ///     Some(100),
    /// ).await?;
    ///
    /// // Now use the transaction manager
    /// # Ok(())
    /// # }
    /// ```
    pub async fn with_transport_manager(
        transport_manager: crate::transaction::transport::TransportManager,
        transport_rx: mpsc::Receiver<TransportEvent>,
        capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransactionEvent>)> {
        // Get the default transport from the manager
        let default_transport = transport_manager.default_transport().await
            .ok_or_else(|| Error::Transport("No default transport available from TransportManager".into()))?;
        
        // Create the transaction manager using the default transport and event channel
        let events_capacity = capacity.unwrap_or(100);
        let (events_tx, events_rx) = mpsc::channel(events_capacity);
        
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        let subscriber_to_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_to_subscribers = Arc::new(Mutex::new(HashMap::new()));
        let next_subscriber_id = Arc::new(Mutex::new(0));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(Mutex::new(false));
        
        let timer_settings = TimerSettings::default();
        
        // Setup timer manager
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        
        // Create timer factory with the timer manager
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        let manager = Self {
            transport: default_transport,
            client_transactions,
            server_transactions,
            transaction_destinations,
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
        };
        
        // Start the message processing loop
        manager.start_message_loop();
        
        Ok((manager, events_rx))
    }
    
    /// Creates a transaction manager with custom timer configuration (sync version).
    ///
    /// This synchronous constructor allows customizing the timer settings
    /// in contexts where async initialization isn't possible.
    ///
    /// ## Timer Configuration
    ///
    /// The custom timer settings allow tuning:
    /// - T1: Base retransmission interval (default 500ms)
    /// - T2: Maximum retransmission interval (default 4s)
    /// - T4: Maximum duration a message remains in the network (default 5s)
    /// - TD: Wait time for response retransmissions (default 32s)
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    /// * `timer_settings_opt` - Optional custom timer settings
    ///
    /// # Returns
    /// * `Self` - A transaction manager instance
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use std::time::Duration;
    /// # use rvoip_sip_transport::Transport;
    /// # use crate::transaction::{TransactionManager, timer::TimerSettings};
    /// # fn example(transport: Arc<dyn Transport>) {
    /// // Create custom timer settings
    /// let mut timer_settings = TimerSettings::default();
    /// timer_settings.t1 = Duration::from_millis(1000);
    /// 
    /// // Create transaction manager with custom settings
    /// let manager = TransactionManager::with_config(
    ///     transport,
    ///     Some(timer_settings)
    /// );
    /// # }
    /// ```
    pub fn with_config(transport: Arc<dyn Transport>, timer_settings_opt: Option<TimerSettings>) -> Self {
        let (events_tx, _) = mpsc::channel(100); // Dummy receiver, will be ignored
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        let subscriber_to_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_to_subscribers = Arc::new(Mutex::new(HashMap::new()));
        let next_subscriber_id = Arc::new(Mutex::new(0));
        let (_, transport_rx) = mpsc::channel(100); // Dummy channel
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(Mutex::new(false));
        
        // Create timer settings
        let timer_settings = timer_settings_opt.unwrap_or_default();
        
        // Create the timer manager
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        Self {
            transport,
            client_transactions,
            server_transactions,
            transaction_destinations,
            events_tx,
            event_subscribers,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx,
            running,
            timer_settings,
            timer_manager,
            timer_factory,
        }
    }

    /// Creates a minimal transaction manager for testing purposes.
    ///
    /// This constructor creates a transaction manager with the minimal
    /// required components for testing. It doesn't start message loops
    /// or perform other initialization that might complicate testing.
    ///
    /// # Arguments
    /// * `transport` - The transport layer to use for sending messages
    /// * `transport_rx` - Channel for receiving transport events
    ///
    /// # Returns
    /// * `Self` - A transaction manager instance configured for testing
    pub fn dummy(
        transport: Arc<dyn Transport>,
        transport_rx: mpsc::Receiver<TransportEvent>,
    ) -> Self {
        // Setup basic channels
        let (events_tx, _) = mpsc::channel(10);
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        
        // Transaction registries
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        
        // Setup timer manager
        let timer_settings = TimerSettings::default();
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        // Initialize running state
        let running = Arc::new(Mutex::new(false));
        
        // Track destinations
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        
        // Initialize subscriber-related fields
        let subscriber_to_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_to_subscribers = Arc::new(Mutex::new(HashMap::new()));
        let next_subscriber_id = Arc::new(Mutex::new(0));
        
        Self {
            transport,
            events_tx,
            event_subscribers,
            client_transactions,
            server_transactions,
            timer_factory,
            timer_manager,
            timer_settings,
            running,
            transaction_destinations,
            subscriber_to_transactions,
            transaction_to_subscribers,
            next_subscriber_id,
            transport_rx: Arc::new(Mutex::new(transport_rx)),
        }
    }

    /// Sends a request through a client transaction.
    ///
    /// This method initiates a client transaction by sending its request
    /// according to the transaction state machine rules in RFC 3261.
    /// It triggers the transition from Initial to Calling (for INVITE)
    /// or Trying (for non-INVITE) state.
    ///
    /// ## Transaction State Transition
    ///
    /// This method triggers the following state transitions:
    /// - INVITE client: Initial → Calling
    /// - Non-INVITE client: Initial → Trying
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1.2: INVITE client transaction initiation
    /// - RFC 3261 Section 17.1.2.2: Non-INVITE client transaction initiation 
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the client transaction to send
    ///
    /// # Returns
    /// * `Result<()>` - Success or error if the transaction cannot be sent
    ///
    /// # Example
    /// ```no_run
    /// # use std::sync::Arc;
    /// # use std::net::SocketAddr;
    /// # use std::str::FromStr;
    /// # use rvoip_sip_core::Request;
    /// # use crate::transaction::TransactionManager;
    /// # use crate::transaction::transaction::TransactionKey;
    /// # async fn example(manager: &TransactionManager, request: Request) -> Result<(), Box<dyn std::error::Error>> {
    /// let destination = SocketAddr::from_str("192.168.1.100:5060")?;
    ///
    /// // First, create a client transaction
    /// let tx_id = manager.create_client_transaction(request, destination).await?;
    ///
    /// // Then, send the request through the transaction
    /// manager.send_request(&tx_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_request(&self, transaction_id: &TransactionKey) -> Result<()> {
        debug!(%transaction_id, "TransactionManager::send_request - sending request");
        
        // We need to get the transaction and clone only when needed
        let mut locked_txs = self.client_transactions.lock().await;
        
        // Check if transaction exists
        if !locked_txs.contains_key(transaction_id) {
            debug!(%transaction_id, "TransactionManager::send_request - transaction not found");
            return Err(Error::transaction_not_found(transaction_id.clone(), "send_request - transaction not found"));
        }
        
        // Get a reference to the transaction to determine its type
        let tx = locked_txs.get_mut(transaction_id).unwrap();
        debug!(%transaction_id, kind=?tx.kind(), state=?tx.state(), "TransactionManager::send_request - found transaction");
        
        // Remember initial state to detect quick state transitions
        let initial_state = tx.state();
        
        // First subscribe to events BEFORE initiating the transaction
        // so we don't miss any events that happen during initiation
        let mut event_rx = self.subscribe();
        
        // Use the TransactionExt trait to safely downcast
        use crate::transaction::client::TransactionExt;
        
        if let Some(client_tx) = tx.as_client_transaction() {
            debug!(%transaction_id, "TransactionManager::send_request - initiating client transaction");
            
            // Issue the initiate command
            let result = client_tx.initiate().await;
            debug!(%transaction_id, success=?result.is_ok(), "TransactionManager::send_request - initiate result");
            
            // If initiate() returned an error, return it immediately
            if let Err(e) = result {
                debug!(%transaction_id, error=%e, "TransactionManager::send_request - initiate failed immediately");
                return Err(e);
            }
            
            // Check transaction state immediately after initiate
            let current_state = tx.state();
            if current_state == TransactionState::Terminated {
                // Transaction terminated immediately - likely due to transport error
                debug!(%transaction_id, "Transaction terminated immediately during initiate - likely transport error");
                return Err(Error::transport_error(
                    rvoip_sip_transport::Error::ProtocolError("Transaction terminated immediately".into()),
                    "Failed to send request - transaction terminated immediately"
                ));
            }
            
            // Release lock to allow transaction processing
            drop(locked_txs);
            
            // Now wait for a short time to catch any asynchronous errors
            // We'll use a timeout to avoid hanging if no events are received
            let timeout_duration = tokio::time::Duration::from_millis(100);
            
            match tokio::time::timeout(timeout_duration, async {
                // Wait for events until timeout
                while let Some(event) = event_rx.recv().await {
                    match event {
                        TransactionEvent::TransportError { transaction_id: tx_id, .. } if tx_id == *transaction_id => {
                            debug!(%transaction_id, "Received TransportError event");
                            return Err(Error::transport_error(
                                rvoip_sip_transport::Error::ProtocolError("Transport error during request send".into()),
                                "Failed to send request - transport error"
                            ));
                        },
                        TransactionEvent::StateChanged { transaction_id: tx_id, previous_state, new_state } 
                            if tx_id == *transaction_id => {
                            debug!(%transaction_id, previous=?previous_state, new=?new_state, "Transaction state changed");
                            
                            // If transaction moved directly to Terminated state
                            if new_state == TransactionState::Terminated && 
                               (previous_state == TransactionState::Initial || 
                                previous_state == TransactionState::Calling || 
                                previous_state == TransactionState::Trying) {
                                
                                debug!(%transaction_id, "Transaction moved to Terminated state - likely transport error");
                                return Err(Error::transport_error(
                                    rvoip_sip_transport::Error::ProtocolError("Transaction terminated unexpectedly".into()),
                                    "Failed to send request - transaction terminated"
                                ));
                            }
                        },
                        _ => {} // Ignore other events
                    }
                }
                
                // Check final transaction state
                let locked_txs = self.client_transactions.lock().await;
                if let Some(tx) = locked_txs.get(transaction_id) {
                    let final_state = tx.state();
                    if final_state == TransactionState::Terminated {
                        debug!(%transaction_id, "Transaction is terminated after events processed");
                        return Err(Error::transport_error(
                            rvoip_sip_transport::Error::ProtocolError("Transaction terminated after processing".into()),
                            "Failed to send request - transaction terminated"
                        ));
                    }
                } else {
                    // Transaction was removed
                    debug!(%transaction_id, "Transaction was removed - likely due to termination");
                    return Err(Error::transport_error(
                        rvoip_sip_transport::Error::ProtocolError("Transaction was removed".into()),
                        "Failed to send request - transaction removed"
                    ));
                }
                
                Ok(())
            }).await {
                // Timeout occurred
                Err(_) => {
                    // Check one more time if the transaction still exists or has terminated
                    let locked_txs = self.client_transactions.lock().await;
                    if let Some(tx) = locked_txs.get(transaction_id) {
                        let final_state = tx.state();
                        if final_state == TransactionState::Terminated {
                            debug!(%transaction_id, "Transaction terminated after timeout");
                            return Err(Error::transport_error(
                                rvoip_sip_transport::Error::ProtocolError("Transaction terminated after timeout".into()),
                                "Failed to send request - transaction terminated"
                            ));
                        }
                        
                        // If we still have a transaction and it's not terminated, assume it's okay
                        debug!(%transaction_id, state=?final_state, "Transaction still exists and is not terminated after timeout");
                        Ok(())
                    } else {
                        // Transaction was removed
                        debug!(%transaction_id, "Transaction was removed after timeout");
                        Err(Error::transport_error(
                            rvoip_sip_transport::Error::ProtocolError("Transaction was removed after timeout".into()),
                            "Failed to send request - transaction removed"
                        ))
                    }
                },
                // Got a result from the event processing
                Ok(result) => result,
            }
        } else {
            debug!(%transaction_id, "TransactionManager::send_request - failed to downcast to client transaction");
            Err(Error::Other("Failed to downcast to client transaction".to_string()))
        }
    }

    /// Sends a response through a server transaction.
    ///
    /// This method sends a SIP response through an existing server transaction,
    /// which will handle retransmissions and state transitions according to
    /// RFC 3261 rules.
    ///
    /// ## Transaction State Transitions
    ///
    /// This method can trigger the following state transitions:
    /// - INVITE server with provisional response: Proceeding → Proceeding
    /// - INVITE server with final response: Proceeding → Completed  
    /// - Non-INVITE server with provisional response: Trying/Proceeding → Proceeding
    /// - Non-INVITE server with final response: Trying/Proceeding → Completed
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.2.1: INVITE server transaction response handling
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction response handling
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the server transaction
    /// * `response` - The SIP response to send
    ///
    /// # Returns
    /// * `Result<()>` - Success or error if the response cannot be sent
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_sip_core::{Response, StatusCode};
    /// # use rvoip_sip_core::builder::SimpleResponseBuilder;
    /// # use crate::transaction::TransactionManager;
    /// # use crate::transaction::transaction::TransactionKey;
    /// # async fn example(
    /// #    manager: &TransactionManager,
    /// #    tx_id: &TransactionKey,
    /// #    request: &rvoip_sip_core::Request
    /// # ) -> Result<(), Box<dyn std::error::Error>> {
    /// // Create a 200 OK response
    /// let response = SimpleResponseBuilder::response_from_request(
    ///     request,
    ///     StatusCode::Ok,
    ///     Some("OK")
    /// ).build();
    ///
    /// // Send the response through the transaction
    /// manager.send_response(tx_id, response).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn send_response(&self, transaction_id: &TransactionKey, response: Response) -> Result<()> {
        // We need to get the transaction and clone only when needed
        let mut locked_txs = self.server_transactions.lock().await;
        
        // Check if transaction exists
        if !locked_txs.contains_key(transaction_id) {
            return Err(Error::transaction_not_found(transaction_id.clone(), "send_response - transaction not found"));
        }
        
        // Get a reference to the transaction to determine its type
        let tx = locked_txs.get_mut(transaction_id).unwrap();
        
        // Use the TransactionExt trait to safely downcast
        use crate::transaction::server::TransactionExt;
        
        if let Some(server_tx) = tx.as_server_transaction() {
            server_tx.send_response(response).await
        } else {
            Err(Error::Other("Failed to downcast to server transaction".to_string()))
        }
    }

    /// Checks if a transaction with the given ID exists.
    ///
    /// This method looks for the transaction in both client and server
    /// transaction collections. It's useful for verifying that a transaction
    /// exists before attempting operations on it.
    ///
    /// # Arguments
    /// * `transaction_id` - The transaction ID to check
    ///
    /// # Returns
    /// * `bool` - True if the transaction exists, false otherwise
    ///
    /// # Example
    /// ```no_run
    /// # use crate::transaction::TransactionManager;
    /// # use crate::transaction::transaction::TransactionKey;
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) {
    /// if manager.transaction_exists(tx_id).await {
    ///     println!("Transaction {} exists", tx_id);
    /// } else {
    ///     println!("Transaction {} not found", tx_id);
    /// }
    /// # }
    /// ```
    pub async fn transaction_exists(&self, transaction_id: &TransactionKey) -> bool {
        let client_exists = {
            let client_txs = self.client_transactions.lock().await;
            client_txs.contains_key(transaction_id)
        };
        
        if client_exists {
            return true;
        }
        
        let server_exists = {
            let server_txs = self.server_transactions.lock().await;
            server_txs.contains_key(transaction_id)
        };
        
        server_exists
    }

    /// Gets the current state of a transaction.
    ///
    /// This method retrieves the current state of a transaction according to
    /// the state machines defined in RFC 3261. The state determines what
    /// operations are valid and how the transaction will respond to messages.
    ///
    /// ## Transaction States
    ///
    /// The possible states are:
    /// - **Initial**: Transaction created but not started
    /// - **Calling**: INVITE client waiting for response
    /// - **Trying**: Non-INVITE client waiting for response
    /// - **Proceeding**: Received provisional response, waiting for final
    /// - **Completed**: Received final response, waiting for reliability
    /// - **Terminated**: Transaction is done
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17.1.1: INVITE client transaction states
    /// - RFC 3261 Section 17.1.2: Non-INVITE client transaction states
    /// - RFC 3261 Section 17.2.1: INVITE server transaction states
    /// - RFC 3261 Section 17.2.2: Non-INVITE server transaction states
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the transaction
    ///
    /// # Returns
    /// * `Result<TransactionState>` - The transaction state or error if not found
    ///
    /// # Example
    /// ```no_run
    /// # use crate::transaction::TransactionManager;
    /// # use crate::transaction::transaction::{TransactionKey, TransactionState};
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) -> Result<(), Box<dyn std::error::Error>> {
    /// let state = manager.transaction_state(tx_id).await?;
    /// 
    /// match state {
    ///     TransactionState::Proceeding => println!("Transaction is in Proceeding state"),
    ///     TransactionState::Completed => println!("Transaction is in Completed state"),
    ///     TransactionState::Terminated => println!("Transaction is terminated"),
    ///     _ => println!("Transaction is in state: {:?}", state),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transaction_state(&self, transaction_id: &TransactionKey) -> Result<TransactionState> {
        // Try client transactions first
        {
            let client_txs = self.client_transactions.lock().await;
            if let Some(tx) = client_txs.get(transaction_id) {
                return Ok(tx.state());
            }
        }
        
        // Try server transactions
        {
            let server_txs = self.server_transactions.lock().await;
            if let Some(tx) = server_txs.get(transaction_id) {
                return Ok(tx.state());
            }
        }
        
        // Transaction not found
        Err(Error::transaction_not_found(transaction_id.clone(), "transaction_state - transaction not found"))
    }

    /// Gets the transaction type (kind) for the specified transaction.
    ///
    /// This method returns the type of transaction as defined in RFC 3261:
    /// - INVITE client transaction (ICT)
    /// - Non-INVITE client transaction (NICT)
    /// - INVITE server transaction (IST) 
    /// - Non-INVITE server transaction (NIST)
    ///
    /// Knowing the transaction kind is important because each type follows
    /// different state machines and behavior rules.
    ///
    /// ## RFC References
    /// - RFC 3261 Section 17: Four transaction types with different state machines
    /// - RFC 3261 Section 17.1: Client transaction types
    /// - RFC 3261 Section 17.2: Server transaction types
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the transaction
    ///
    /// # Returns
    /// * `Result<TransactionKind>` - The transaction kind or error if not found
    ///
    /// # Example
    /// ```no_run
    /// # use crate::transaction::TransactionManager;
    /// # use crate::transaction::transaction::{TransactionKey, TransactionKind};
    /// # async fn example(manager: &TransactionManager, tx_id: &TransactionKey) -> Result<(), Box<dyn std::error::Error>> {
    /// let kind = manager.transaction_kind(tx_id).await?;
    /// 
    /// match kind {
    ///     TransactionKind::InviteClient => println!("This is an INVITE client transaction"),
    ///     TransactionKind::NonInviteClient => println!("This is a non-INVITE client transaction"),
    ///     TransactionKind::InviteServer => println!("This is an INVITE server transaction"),
    ///     TransactionKind::NonInviteServer => println!("This is a non-INVITE server transaction"),
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn transaction_kind(&self, transaction_id: &TransactionKey) -> Result<TransactionKind> {
        let client_txs = self.client_transactions.lock().await;
        if let Some(tx) = client_txs.get(transaction_id) {
            return Ok(tx.kind());
        }

        let server_txs = self.server_transactions.lock().await;
        if let Some(tx) = server_txs.get(transaction_id) {
            return Ok(tx.kind());
        }

        Err(Error::transaction_not_found(transaction_id.clone(), "transaction kind lookup failed"))
    }

    /// Gets a list of all active transaction IDs.
    ///
    /// This method returns separate lists of client and server transaction IDs,
    /// which can be useful for monitoring, debugging, or cleanup operations.
    ///
    /// # Returns
    /// * `(Vec<TransactionKey>, Vec<TransactionKey>)` - Client and server transaction IDs
    ///
    /// # Example
    /// ```no_run
    /// # use crate::transaction::TransactionManager;
    /// # async fn example(manager: &TransactionManager) {
    /// let (client_txs, server_txs) = manager.active_transactions().await;
    /// 
    /// println!("Active client transactions: {}", client_txs.len());
    /// println!("Active server transactions: {}", server_txs.len());
    /// 
    /// // Process each transaction ID
    /// for tx_id in client_txs {
    ///     println!("Client transaction: {}", tx_id);
    /// }
    /// # }
    /// ```
    pub async fn active_transactions(&self) -> (Vec<TransactionKey>, Vec<TransactionKey>) {
        let client_txs = self.client_transactions.lock().await;
        let server_txs = self.server_transactions.lock().await;

        (
            client_txs.keys().cloned().collect(),
            server_txs.keys().cloned().collect(),
        )
    }

    /// Gets a reference to the transport layer used by this transaction manager.
    ///
    /// This method provides access to the underlying transport layer,
    /// which can be useful for operations outside the transaction layer.
    ///
    /// # Returns
    /// * `Arc<dyn Transport>` - The transport layer
    pub fn transport(&self) -> Arc<dyn Transport> {
        self.transport.clone()
    }

    /// Subscribe to events from all transactions.
    ///
    /// This method creates a new subscription to all transaction events.
    /// The returned receiver will get all events regardless of which transaction
    /// generated them. This is useful for monitoring or logging all transaction activity.
    ///
    /// # Returns
    /// * `mpsc::Receiver<TransactionEvent>` - The event receiver
    pub fn subscribe(&self) -> mpsc::Receiver<TransactionEvent> {
        let (tx, rx) = mpsc::channel(100);
        
        std::mem::drop(tokio::spawn({
            let subscribers = self.event_subscribers.clone();
            let next_subscriber_id = self.next_subscriber_id.clone();
            
            async move {
                let mut subscriber_id = next_subscriber_id.lock().await;
                let id = *subscriber_id;
                *subscriber_id += 1;
                
                // Add to global subscribers list
                let mut subs = subscribers.lock().await;
                subs.push(tx);
                
                debug!("Added global subscriber with ID {}", id);
            }
        }));
        
        rx
    }
    
    /// Subscribe to events from a specific transaction.
    ///
    /// This method creates a subscription to events from a single transaction.
    /// The returned receiver will only get events from the specified transaction,
    /// reducing noise and unnecessary event processing.
    ///
    /// # Arguments
    /// * `transaction_id` - The ID of the transaction to subscribe to
    ///
    /// # Returns
    /// * `Result<mpsc::Receiver<TransactionEvent>>` - The event receiver
    pub async fn subscribe_to_transaction(&self, transaction_id: &TransactionKey) -> Result<mpsc::Receiver<TransactionEvent>> {
        // Validate that transaction exists
        if !self.transaction_exists(transaction_id).await {
            return Err(Error::transaction_not_found(transaction_id.clone(), "subscribe_to_transaction - transaction not found"));
        }
        
        let (tx, rx) = mpsc::channel(100);
        
        // Register the subscription
        let subscriber_id = {
            let mut next_id = self.next_subscriber_id.lock().await;
            let id = *next_id;
            *next_id += 1;
            id
        };
        
        // Add to global subscribers list
        {
            let mut subs = self.event_subscribers.lock().await;
            subs.push(tx);
        }
        
        // Add to transaction-specific mapping
        {
            let mut tx_to_subs = self.transaction_to_subscribers.lock().await;
            
            // Create entry if it doesn't exist
            let subscriber_list = tx_to_subs.entry(transaction_id.clone())
                .or_insert_with(Vec::new);
            
            // Add this subscriber
            subscriber_list.push(subscriber_id);
        }
        
        // Add to subscriber-to-transactions mapping
        {
            let mut sub_to_txs = self.subscriber_to_transactions.lock().await;
            
            // Create entry if it doesn't exist
            let transaction_list = sub_to_txs.entry(subscriber_id)
                .or_insert_with(Vec::new);
            
            // Add this transaction
            transaction_list.push(transaction_id.clone());
        }
        
        debug!(%transaction_id, subscriber_id, "Added transaction-specific subscriber");
        
        Ok(rx)
    }
    
    /// Subscribe to events from multiple transactions.
    ///
    /// This method creates a subscription to events from multiple transactions.
    /// The returned receiver will only get events from the specified transactions.
    ///
    /// # Arguments
    /// * `transaction_ids` - The IDs of the transactions to subscribe to
    ///
    /// # Returns
    /// * `Result<mpsc::Receiver<TransactionEvent>>` - The event receiver
    pub async fn subscribe_to_transactions(&self, transaction_ids: &[TransactionKey]) -> Result<mpsc::Receiver<TransactionEvent>> {
        // Validate that all transactions exist
        for tx_id in transaction_ids {
            if !self.transaction_exists(tx_id).await {
                return Err(Error::transaction_not_found(tx_id.clone(), "subscribe_to_transactions - transaction not found"));
            }
        }
        
        let (tx, rx) = mpsc::channel(100);
        
        // Register the subscription
        let subscriber_id = {
            let mut next_id = self.next_subscriber_id.lock().await;
            let id = *next_id;
            *next_id += 1;
            id
        };
        
        // Add to global subscribers list
        {
            let mut subs = self.event_subscribers.lock().await;
            subs.push(tx);
        }
        
        // Add to transaction-specific mapping
        {
            let mut tx_to_subs = self.transaction_to_subscribers.lock().await;
            
            for tx_id in transaction_ids {
                // Create entry if it doesn't exist
                let subscriber_list = tx_to_subs.entry(tx_id.clone())
                    .or_insert_with(Vec::new);
                
                // Add this subscriber
                subscriber_list.push(subscriber_id);
            }
        }
        
        // Add to subscriber-to-transactions mapping
        {
            let mut sub_to_txs = self.subscriber_to_transactions.lock().await;
            
            // Create entry if it doesn't exist
            let transaction_list = sub_to_txs.entry(subscriber_id)
                .or_insert_with(Vec::new);
            
            // Add these transactions
            for tx_id in transaction_ids {
                transaction_list.push(tx_id.clone());
            }
        }
        
        debug!(subscriber_id, transaction_count = transaction_ids.len(), "Added multi-transaction subscriber");
        
        Ok(rx)
    }

    /// Shutdown the transaction manager
    pub async fn shutdown(&self) {
        {
             let mut running = self.running.lock().await;
             *running = false;
        } // Release lock

        // Clear all transactions
        self.client_transactions.lock().await.clear();
        self.server_transactions.lock().await.clear();
        self.transaction_destinations.lock().await.clear();

        debug!("Transaction manager shutdown");
    }

    /// Broadcasts a transaction event to all subscribers.
    ///
    /// This method is responsible for delivering transaction events to subscribers.
    /// It implements event filtering based on subscriber preferences, ensuring
    /// subscribers only receive events they're interested in.
    ///
    /// # Arguments
    /// * `event` - The transaction event to broadcast
    /// * `primary_tx` - The primary event channel
    /// * `subscribers` - Additional event subscribers
    /// * `transaction_to_subscribers` - Maps transactions to interested subscribers
    /// * `manager` - Optional manager for processing termination events
    async fn broadcast_event(
        event: TransactionEvent,
        primary_tx: &mpsc::Sender<TransactionEvent>,
        subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
        transaction_to_subscribers: Option<&Arc<Mutex<HashMap<TransactionKey, Vec<usize>>>>>,
        manager: Option<TransactionManager>,
    ) {
        // Extract transaction ID from the event for filtering
        let transaction_id = match &event {
            TransactionEvent::StateChanged { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::SuccessResponse { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::FailureResponse { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::ProvisionalResponse { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::TransactionTerminated { transaction_id } => Some(transaction_id),
            TransactionEvent::TimerTriggered { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::AckReceived { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::CancelReceived { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::InviteRequest { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::NonInviteRequest { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::AckRequest { transaction_id, .. } => Some(transaction_id),
            TransactionEvent::CancelRequest { transaction_id, .. } => Some(transaction_id),
            // These events don't have a specific transaction ID
            TransactionEvent::StrayResponse { .. } => None,
            TransactionEvent::StrayAck { .. } => None,
            TransactionEvent::StrayCancel { .. } => None,
            TransactionEvent::StrayAckRequest { .. } => None,
            // Add other event types for completeness
            _ => None,
        };

        // Get list of interested subscribers for this transaction
        let interested_subscribers = if let (Some(tx_id), Some(tx_to_subs_map)) = (transaction_id, transaction_to_subscribers) {
            let tx_to_subs = tx_to_subs_map.lock().await;
            tx_to_subs.get(tx_id).cloned().unwrap_or_default()
        } else {
            Vec::new() // No specific subscribers for this transaction or global event
        };
        
        // Send to primary channel if it's available
        if let Err(e) = primary_tx.send(event.clone()).await {
            warn!("Failed to send event to primary channel: {}", e);
        }
        
        // Send to interested subscribers only
        let mut subs = subscribers.lock().await;
        
        // If we have transaction-specific subscribers, filter events
        if let Some(tx_to_subs_map) = transaction_to_subscribers {
            for (idx, sub) in subs.iter().enumerate() {
                // Send to this subscriber if:
                // 1. It's a global event (no transaction ID)
                // 2. This subscriber is interested in this transaction
                // 3. There are no interested subscribers specified (backward compatibility)
                let should_send = transaction_id.is_none() || 
                                 interested_subscribers.contains(&idx) ||
                                 interested_subscribers.is_empty();
                
                if should_send {
                    if let Err(e) = sub.send(event.clone()).await {
                        warn!("Failed to send event to subscriber {}: {}", idx, e);
                    }
                }
            }
        } else {
            // No transaction filtering, send to all (backward compatibility)
            for (idx, sub) in subs.iter().enumerate() {
                if let Err(e) = sub.send(event.clone()).await {
                    warn!("Failed to send event to subscriber {}: {}", idx, e);
                }
            }
        }
        
        // Special handling for transaction termination
        if let TransactionEvent::TransactionTerminated { transaction_id } = &event {
            if let Some(manager_instance) = manager {
                // Process the termination in a separate task to avoid deadlocks
                let tx_id = transaction_id.clone();
                let manager_clone = manager_instance.clone();
                tokio::spawn(async move {
                    manager_clone.process_transaction_terminated(&tx_id).await;
                });
            }
        }
    }

    /// Handle transaction termination event and clean up terminated transactions
    async fn process_transaction_terminated(&self, transaction_id: &TransactionKey) {
        debug!(%transaction_id, "Processing transaction termination");
        
        let mut terminated = false;
        
        // Try to remove from client transactions
        {
            let mut client_txs = self.client_transactions.lock().await;
            if let Some(tx) = client_txs.remove(transaction_id) {
                debug!(%transaction_id, "Removed terminated client transaction");
                terminated = true;
            }
        }
        
        // Try to remove from server transactions regardless of whether it was found in client transactions
        // This is a defensive approach in case the transaction was somehow duplicated
        {
            let mut server_txs = self.server_transactions.lock().await;
            if let Some(tx) = server_txs.remove(transaction_id) {
                debug!(%transaction_id, "Removed terminated server transaction");
                terminated = true;
            }
        }
        
        // Always remove from destinations map if present
        {
            let mut destinations = self.transaction_destinations.lock().await;
            if destinations.remove(transaction_id).is_some() {
                debug!(%transaction_id, "Removed transaction from destinations map");
            }
        }
        
        // **CRITICAL FIX**: Clean up subscriber mappings to prevent memory leak
        {
            let mut tx_to_subs = self.transaction_to_subscribers.lock().await;
            if let Some(subscriber_ids) = tx_to_subs.remove(transaction_id) {
                debug!(%transaction_id, subscriber_count = subscriber_ids.len(), "Removed transaction from subscriber mappings");
                
                // Also clean up reverse mappings
                drop(tx_to_subs); // Release lock before acquiring another
                let mut sub_to_txs = self.subscriber_to_transactions.lock().await;
                
                for subscriber_id in subscriber_ids {
                    if let Some(tx_list) = sub_to_txs.get_mut(&subscriber_id) {
                        tx_list.retain(|tx_id| tx_id != transaction_id);
                        
                        // If subscriber has no more transactions, remove it entirely
                        if tx_list.is_empty() {
                            sub_to_txs.remove(&subscriber_id);
                            debug!(%transaction_id, subscriber_id, "Removed empty subscriber mapping");
                        }
                    }
                }
            }
        }
        
        // Unregister from timer manager (defensive - it should auto-unregister)
        self.timer_manager.unregister_transaction(transaction_id).await;
        debug!(%transaction_id, "Unregistered transaction from timer manager");
        
        if terminated {
            debug!(%transaction_id, "Successfully cleaned up terminated transaction");
        } else {
            warn!(%transaction_id, "Transaction not found for termination - may have been already removed");
        }
        
        // Run cleanup to catch any other terminated transactions
        // This is a defensive measure to prevent resource leaks
        // We use spawn to avoid blocking and make this a background task
        let manager_clone = self.clone();
        tokio::spawn(async move {
            match manager_clone.cleanup_terminated_transactions().await {
                Ok(count) if count > 0 => {
                    debug!("Cleaned up {} additional terminated transactions", count);
                },
                Err(e) => {
                    error!("Error in background cleanup of terminated transactions: {}", e);
                },
                _ => {}
            }
        });
    }

    /// Start the message processing loop for handling incoming transport events
    fn start_message_loop(&self) {
        let transport_arc = self.transport.clone();
        let client_transactions = self.client_transactions.clone();
        let server_transactions = self.server_transactions.clone();
        let events_tx = self.events_tx.clone();
        let transport_rx = self.transport_rx.clone();
        let event_subscribers = self.event_subscribers.clone();
        let running = self.running.clone();
        let manager_arc = self.clone();

        tokio::spawn(async move {
            debug!("Starting transaction message loop");
            
            // Create a separate channel to receive events from transactions
            let (internal_tx, mut internal_rx) = mpsc::channel(100);
            
            // Set running flag
            let mut running_guard = running.lock().await;
            *running_guard = true;
            drop(running_guard);
            
            // Get the transport receiver
            let mut receiver = transport_rx.lock().await;
            
            // Run the message processing loop
            loop {
                // Check if we should continue running
                let running_guard = running.lock().await;
                let is_running = *running_guard;
                drop(running_guard);
                
                if !is_running {
                    debug!("Transaction manager stopping message loop");
                    break;
                }
                
                // Use tokio::select to wait for a message from either the transport or internal channel
                tokio::select! {
                    Some(message_event) = receiver.recv() => {
                        if let Err(e) = manager_arc.handle_transport_event(message_event).await {
                            error!("Error handling transport message: {}", e);
                        }
                    }
                    Some(transaction_event) = internal_rx.recv() => {
                        // Handle transaction events, particularly termination events
                        Self::broadcast_event(
                            transaction_event, 
                            &events_tx, 
                            &event_subscribers,
                            Some(&manager_arc.transaction_to_subscribers),
                            Some(manager_arc.clone()),
                        ).await;
                    }
                    else => {
                        // Both channels have been closed; exit loop
                        debug!("All message channels closed, exiting transaction message loop");
                        break;
                    }
                }
            }
            
            debug!("Transaction message loop exited");
        });
    }

    /// Helper function to get timer settings for a request
    fn timer_settings_for_request(&self, request: &Request) -> Option<TimerSettings> {
        // In the future, we could customize timer settings based on request properties
        // For now, just return a clone of the default settings
        Some(self.timer_settings.clone())
    }

    /// Create a client transaction for sending a SIP request
    /// The caller is responsible for calling send_request() to initiate the transaction.
    pub async fn create_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        debug!(method=%request.method(), destination=%destination, "Creating client transaction");
        
        // Debug the Via headers in the request
        println!("Request Via headers before transaction creation:");
        for (i, via) in request.via_headers().iter().enumerate() {
            println!("  Via[{}]: {}", i, via);
        }
        
        // Extract branch parameter from the top Via header or generate a new one
        let branch = match request.first_via() {
            Some(via) => {
                match via.branch() {
                    Some(b) => b.to_string(),
                    None => {
                        // Generate a branch parameter if none exists
                        format!("{}{}", RFC3261_BRANCH_MAGIC_COOKIE, uuid::Uuid::new_v4().as_simple())
                    }
                }
            },
            None => {
                // No Via header - should not happen, but we'll handle it by generating a branch
                // and a Via header will be added by the transaction
                format!("{}{}", RFC3261_BRANCH_MAGIC_COOKIE, uuid::Uuid::new_v4().as_simple())
            }
        };
        
        // We'll create the transaction key directly
        let key = TransactionKey::new(branch.clone(), request.method().clone(), false);
        
        // For CANCEL method, make sure we don't add a new Via header if one already exists
        // This is already checked in create_cancel_request, but we'll verify here as well
        let mut modified_request = request.clone();
        
        // For CANCEL requests, the Via header should be preserved exactly as it was created
        // No need to add or modify it
        if request.method() == Method::Cancel {
            // Since CANCEL already has a Via header with the correct branch from create_cancel_request,
            // we don't need to modify it further
            println!("CANCEL request detected - not adding Via header");
        } else {
            // For other methods, ensure the request has a Via header with our branch
            // Create a Via header with the branch parameter
            let local_addr = self.transport.local_addr()
                .map_err(|e| Error::transport_error(e, "Failed to get local address for Via header"))?;
            
            let via_header = handlers::create_via_header(&local_addr, &branch)?;
            
            // Check if there's already a Via header
            if request.first_via().is_some() {
                // Replace it to ensure the branch is correct
                modified_request.headers.retain(|h| !matches!(h, TypedHeader::Via(_)));
                modified_request = modified_request.with_header(via_header);
            } else {
                // Add a new Via header
                modified_request = modified_request.with_header(via_header);
            }
        }
        
        println!("Request Via headers after potential modification:");
        for (i, via) in modified_request.via_headers().iter().enumerate() {
            println!("  Via[{}]: {}", i, via);
        }
        
        // Create the appropriate transaction based on the request method
        let transaction: Box<dyn ClientTransaction + Send> = match modified_request.method() {
            Method::Invite => {
                println!("Creating ClientInviteTransaction: {}", key);
                let tx = ClientInviteTransaction::new(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request)
                )?;
                println!("Created ClientInviteTransaction: {}", key);
                Box::new(tx)
            },
            Method::Cancel => {
                // Validate the CANCEL request
                if let Err(e) = cancel::validate_cancel_request(&modified_request) {
                    warn!(method = %modified_request.method(), error = %e, "Creating transaction for CANCEL with possible validation issues");
                }
                
                let tx = ClientNonInviteTransaction::new(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request)
                )?;
                Box::new(tx)
            },
            Method::Update => {
                // Validate the UPDATE request
                if let Err(e) = update::validate_update_request(&modified_request) {
                    warn!(method = %modified_request.method(), error = %e, "Creating transaction for UPDATE with possible validation issues");
                }
                
                let tx = ClientNonInviteTransaction::new(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request)
                )?;
                Box::new(tx)
            },
            _ => {
                let tx = ClientNonInviteTransaction::new(
                    key.clone(),
                    modified_request.clone(),
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    self.timer_settings_for_request(&modified_request)
                )?;
                Box::new(tx)
            }
        };
        
        // Store the transaction
        {
            let mut client_txs = self.client_transactions.lock().await;
            client_txs.insert(key.clone(), transaction);
        }
        
        // Store the destination
        {
            let mut dest_map = self.transaction_destinations.lock().await;
            dest_map.insert(key.clone(), destination);
        }
        
        debug!(id=%key, "Created client transaction");
        
        if request.method() == Method::Cancel {
            debug!(id=%key, original_id=%branch, "Created CANCEL transaction");
        }
        
        Ok(key)
    }

    /// Creates and sends an ACK request for a 2xx response to an INVITE.
    pub async fn send_ack_for_2xx(
        &self,
        invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> Result<()> {
        // Create the ACK request
        let ack_request = self.create_ack_for_2xx(invite_tx_id, response).await?;
        
        // Try to get a destination from the Contact header first
        let destination = if let Some(TypedHeader::Contact(contact)) = response.header(&HeaderName::Contact) {
            if let Some(contact_addr) = contact.addresses().next() {
                // Try to parse the URI as a socket address
                if let Some(addr) = utils::socket_addr_from_uri(&contact_addr.uri) {
                    Some(addr)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        
        // If we couldn't get a destination from the Contact header, use the original destination
        let destination = if let Some(dest) = destination {
            dest
        } else {
            // Fall back to the original destination
            let dest_map = self.transaction_destinations.lock().await;
            match dest_map.get(invite_tx_id) {
                Some(addr) => *addr,
                None => return Err(Error::Other(format!("Destination for transaction {:?} not found", invite_tx_id))),
            }
        };
        
        // Send the ACK directly without creating a transaction
        self.transport.send_message(Message::Request(ack_request), destination).await
            .map_err(|e| Error::transport_error(e, "Failed to send ACK"))?;
        
        Ok(())
    }
    
    /// Find transaction by message.
    ///
    /// This method tries to find a transaction that matches the given message.
    /// For requests, it looks for server transactions.
    /// For responses, it looks for client transactions.
    ///
    /// # Arguments
    /// * `message` - The message to match
    ///
    /// # Returns
    /// * `Result<Option<TransactionKey>>` - The matching transaction key if found
    pub async fn find_transaction_by_message(&self, message: &Message) -> Result<Option<TransactionKey>> {
        match message {
            Message::Request(req) => {
                // For requests, look for server transactions
                let server_txs = self.server_transactions.lock().await;
                for (tx_id, tx) in server_txs.iter() {
                    if tx.matches(message) {
                        return Ok(Some(tx_id.clone()));
                    }
                }
                Ok(None)
            },
            Message::Response(resp) => {
                // For responses, look for client transactions
                let client_txs = self.client_transactions.lock().await;
                for (tx_id, tx) in client_txs.iter() {
                    if tx.matches(message) {
                        return Ok(Some(tx_id.clone()));
                    }
                }
                Ok(None)
            }
        }
    }
    
    /// Find the matching INVITE transaction for a CANCEL request.
    ///
    /// # Arguments
    /// * `cancel_request` - The CANCEL request
    ///
    /// # Returns
    /// * `Result<Option<TransactionKey>>` - The matching INVITE transaction key if found
    pub async fn find_invite_transaction_for_cancel(&self, cancel_request: &Request) -> Result<Option<TransactionKey>> {
        if cancel_request.method() != Method::Cancel {
            return Err(Error::Other("Not a CANCEL request".to_string()));
        }
        
        // Get all client transactions
        let client_txs = self.client_transactions.lock().await;
        let invite_tx_keys: Vec<TransactionKey> = client_txs.keys()
            .filter(|k| *k.method() == Method::Invite && !k.is_server)
            .cloned()
            .collect();
        drop(client_txs);
        
        // Use the utility to find the matching INVITE transaction
        let tx_id = crate::transaction::method::cancel::find_invite_transaction_for_cancel(
            cancel_request, 
            invite_tx_keys
        );
        
        Ok(tx_id)
    }

    /// Creates an ACK request for a 2xx response to an INVITE.
    pub async fn create_ack_for_2xx(
        &self,
        invite_tx_id: &TransactionKey,
        response: &Response,
    ) -> Result<Request> {
        // Verify this is an INVITE client transaction
        if *invite_tx_id.method() != Method::Invite || invite_tx_id.is_server {
            return Err(Error::Other("Can only create ACK for INVITE client transactions".to_string()));
        }
        
        // Get the original INVITE request
        let invite_request = utils::get_transaction_request(
            &self.client_transactions, 
            invite_tx_id
        ).await?;
        
        // Get the local address for the Via header
        let local_addr = self.transport.local_addr()
            .map_err(|e| Error::transport_error(e, "Failed to get local address"))?;
        
        // Create the ACK request using our utility
        let ack_request = crate::transaction::method::ack::create_ack_for_2xx(&invite_request, response, &local_addr)?;
        
        Ok(ack_request)
    }

    /// Create a server transaction from an incoming request.
    /// 
    /// This is called when a new request is received from the transport layer.
    /// It creates an appropriate transaction based on the request method.
    pub async fn create_server_transaction(
        &self,
        request: Request,
        remote_addr: SocketAddr,
    ) -> Result<Arc<dyn ServerTransaction>> {
        // Extract branch parameter from the top Via header
        let branch = match request.first_via() {
            Some(via) => {
                match via.branch() {
                    Some(b) => b.to_string(),
                    None => return Err(Error::Other("Missing branch parameter in Via header".to_string())),
                }
            },
            None => return Err(Error::Other("Missing Via header in request".to_string())),
        };
        
        // Create the transaction key directly with is_server: true
        let key = TransactionKey::new(branch, request.method().clone(), true);
        
        // Check if this is a retransmission of an existing transaction
        {
            let server_txs = self.server_transactions.lock().await;
            if server_txs.contains_key(&key) {
                // This is a retransmission, get the existing transaction
                let transaction = server_txs.get(&key).unwrap().clone();
                drop(server_txs); // Release lock
                
                // Process the request in the existing transaction
                transaction.process_request(request.clone()).await?;
                
                debug!(id=%key, method=%request.method(), "Processed retransmitted request in existing transaction");
                return Ok(transaction);
            }
        }
        
        // Create a new transaction based on the request method
        let transaction: Arc<dyn ServerTransaction> = match request.method() {
            Method::Invite => {
                let tx = Arc::new(ServerInviteTransaction::new(
                    key.clone(),
                    request.clone(),
                    remote_addr,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    None, // No timer override
                )?);
                
                info!(id=%tx.id(), method=%request.method(), "Created new ServerInviteTransaction");
                tx
            },
            Method::Cancel => {
                // Validate the CANCEL request
                if let Err(e) = cancel::validate_cancel_request(&request) {
                    warn!(method = %request.method(), error = %e, "Creating transaction for CANCEL with possible validation issues");
                }
                
                // For CANCEL, try to find the target INVITE transaction
                let mut target_invite_tx_id = None;
                
                // Look for a matching INVITE transaction using the method utility
                let client_txs = self.client_transactions.lock().await;
                let invite_tx_keys: Vec<TransactionKey> = client_txs.keys()
                    .filter(|k| k.method() == &Method::Invite && !k.is_server)
                    .cloned()
                    .collect();
                drop(client_txs);
                
                if let Some(invite_tx_id) = cancel::find_matching_invite_transaction(&request, invite_tx_keys) {
                    target_invite_tx_id = Some(invite_tx_id);
                    debug!(method=%request.method(), "Found matching INVITE transaction for CANCEL");
                } else {
                    debug!(method=%request.method(), "No matching INVITE transaction found for CANCEL");
                }
                
                // Create a non-INVITE server transaction for CANCEL
                let tx = Arc::new(ServerNonInviteTransaction::new(
                    key.clone(),
                    request.clone(),
                    remote_addr,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    None, // No timer override
                )?);
                
                info!(id=%tx.id(), method=%request.method(), "Created new ServerNonInviteTransaction for CANCEL");
                
                // If we found a matching INVITE transaction, notify the TU
                if let Some(invite_tx_id) = target_invite_tx_id {
                    self.events_tx.send(TransactionEvent::CancelRequest {
                        transaction_id: tx.id().clone(),
                        target_transaction_id: invite_tx_id,
                        request: request.clone(),
                        source: remote_addr,
                    }).await.ok();
                }
                
                tx
            },
            Method::Update => {
                // Validate the UPDATE request
                if let Err(e) = update::validate_update_request(&request) {
                    warn!(method = %request.method(), error = %e, "Creating transaction for UPDATE with possible validation issues");
                }
                
                // Create a non-INVITE server transaction for UPDATE
                let tx = Arc::new(ServerNonInviteTransaction::new(
                    key.clone(),
                    request.clone(),
                    remote_addr,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    None, // No timer override
                )?);
                
                info!(id=%tx.id(), method=%request.method(), "Created new ServerNonInviteTransaction for UPDATE");
                tx
            },
            _ => {
                let tx = Arc::new(ServerNonInviteTransaction::new(
                    key.clone(),
                    request.clone(),
                    remote_addr,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    None, // No timer override
                )?);
                
                info!(id=%tx.id(), method=%request.method(), "Created new ServerNonInviteTransaction");
                tx
            }
        };
        
        // Store the transaction
        {
            let mut server_txs = self.server_transactions.lock().await;
            server_txs.insert(transaction.id().clone(), transaction.clone());
        }
        
        // Start the transaction in Trying state (for non-INVITE) or Proceeding (for INVITE)
        let initial_state = match transaction.kind() {
            TransactionKind::InviteServer => TransactionState::Proceeding,
            _ => TransactionState::Trying,
        };
        
        // Transition to the initial active state
        if let Err(e) = transaction.send_command(InternalTransactionCommand::TransitionTo(initial_state)).await {
            error!(id=%transaction.id(), error=%e, "Failed to initialize new server transaction");
            return Err(e);
        }
        
        Ok(transaction)
    }

    /// Cancel an active INVITE client transaction
    ///
    /// Creates a CANCEL request based on the original INVITE and creates
    /// a new client transaction to send it.
    ///
    /// Returns the transaction ID of the new CANCEL transaction.
    pub async fn cancel_invite_transaction(
        &self,
        invite_tx_id: &TransactionKey,
    ) -> Result<TransactionKey> {
        debug!(id=%invite_tx_id, "Canceling invite transaction");
        
        // Check that this is an INVITE client transaction
        if invite_tx_id.method() != &Method::Invite || invite_tx_id.is_server() {
            return Err(Error::Other(format!(
                "Transaction {} is not an INVITE client transaction", invite_tx_id
            )));
        }
        
        // Get the original INVITE request 
        let invite_request = utils::get_transaction_request(
            &self.client_transactions,
            invite_tx_id
        ).await?;
        
        debug!(id=%invite_tx_id, "Got INVITE request for cancellation");
        
        // Create a CANCEL request from the INVITE
        let local_addr = self.transport.local_addr()
            .map_err(|e| Error::transport_error(e, "Failed to get local address"))?;
        
        // Use the method utility to create the CANCEL request
        let cancel_request = cancel::create_cancel_request(&invite_request, &local_addr)?;
        
        // Log and validate the CANCEL request to help with debugging
        if let Err(e) = cancel::validate_cancel_request(&cancel_request) {
            warn!(method = %cancel_request.method(), error = %e, "CANCEL request validation issue - proceeding anyway");
        }
        
        // Get the destination for the CANCEL request (same as the INVITE)
        let destination = {
            let dest_map = self.transaction_destinations.lock().await;
            match dest_map.get(invite_tx_id) {
                Some(addr) => *addr,
                None => return Err(Error::Other(format!(
                    "No destination found for transaction {}", invite_tx_id
                ))),
            }
        };
        
        // Create a transaction for the CANCEL request
        let cancel_tx_id = self.create_client_transaction(
            cancel_request,
            destination,
        ).await?;
        
        debug!(id=%cancel_tx_id, original_id=%invite_tx_id, "Created CANCEL transaction");
        
        // Send the CANCEL request immediately
        self.send_request(&cancel_tx_id).await?;
        
        Ok(cancel_tx_id)
    }

    /// Creates a client transaction for a non-INVITE request.
    ///
    /// # Arguments
    /// * `request` - The non-INVITE request to send
    /// * `destination` - The destination address to send the request to
    ///
    /// # Returns
    /// * `Result<TransactionKey>` - The transaction ID on success, or an error
    pub async fn create_non_invite_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        if request.method() == Method::Invite {
            return Err(Error::Other("Cannot create non-INVITE transaction for INVITE request".to_string()));
        }
        
        self.create_client_transaction(request, destination).await
    }

    /// Creates a client transaction for an INVITE request.
    ///
    /// # Arguments
    /// * `request` - The INVITE request to send
    /// * `destination` - The destination address to send the request to
    ///
    /// # Returns
    /// * `Result<TransactionKey>` - The transaction ID on success, or an error 
    pub async fn create_invite_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        if request.method() != Method::Invite {
            return Err(Error::Other("Cannot create INVITE transaction for non-INVITE request".to_string()));
        }
        
        self.create_client_transaction(request, destination).await
    }

    /// Get information about available transport types and their capabilities
    /// 
    /// This method returns information about which transport types are available
    /// and their capabilities. This is useful for session-level components that
    /// need to know what transport options are available.
    pub fn get_transport_capabilities(&self) -> TransportCapabilities {
        TransportCapabilities {
            supports_udp: self.transport.supports_udp(),
            supports_tcp: self.transport.supports_tcp(),
            supports_tls: self.transport.supports_tls(),
            supports_ws: self.transport.supports_ws(),
            supports_wss: self.transport.supports_wss(),
            local_addr: self.transport.local_addr().ok(),
            default_transport: self.transport.default_transport_type(),
        }
    }

    /// Get detailed information about a specific transport type
    /// 
    /// This method returns detailed information about a specific transport type,
    /// such as connection status, local address, etc.
    pub fn get_transport_info(&self, transport_type: TransportType) -> Option<TransportInfo> {
        if !self.transport.supports_transport(transport_type) {
            return None;
        }

        Some(TransportInfo {
            transport_type,
            is_connected: self.transport.is_transport_connected(transport_type),
            local_addr: self.transport.get_transport_local_addr(transport_type).ok(),
            connection_count: self.transport.get_connection_count(transport_type),
        })
    }

    /// Check if a specific transport type is available
    pub fn is_transport_available(&self, transport_type: TransportType) -> bool {
        self.transport.supports_transport(transport_type)
    }

    /// Get network information for SDP generation
    /// 
    /// This method returns network information that can be used for SDP generation,
    /// such as the local IP address and ports for different media types.
    pub fn get_network_info_for_sdp(&self) -> NetworkInfoForSdp {
        NetworkInfoForSdp {
            local_ip: self.transport.local_addr()
                .map(|addr| addr.ip())
                .unwrap_or_else(|_| std::net::IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1))),
            rtp_port_range: (10000, 20000), // Default port range, could be configurable
        }
    }

    /// Get the best transport type for a given URI
    /// 
    /// This method analyzes a URI and returns the best transport type to use
    /// based on the URI scheme and available transports.
    pub fn get_best_transport_for_uri(&self, uri: &rvoip_sip_core::Uri) -> TransportType {
        // Determine the best transport based on the URI scheme
        let scheme = uri.scheme().to_string();
        
        match scheme.as_str() {
            "sips" => {
                if self.transport.supports_tls() {
                    TransportType::Tls
                } else {
                    // Fallback to another secure transport if TLS is not available
                    if self.transport.supports_wss() {
                        TransportType::Wss
                    } else {
                        // Last resort: use any available transport
                        self.transport.default_transport_type()
                    }
                }
            },
            "ws" => {
                if self.transport.supports_ws() {
                    TransportType::Ws
                } else {
                    self.transport.default_transport_type()
                }
            },
            "wss" => {
                if self.transport.supports_wss() {
                    TransportType::Wss
                } else if self.transport.supports_tls() {
                    TransportType::Tls
                } else {
                    self.transport.default_transport_type()
                }
            },
            // Default for "sip:" and any other schemes
            _ => self.transport.default_transport_type()
        }
    }

    /// Get WebSocket connection status if available
    /// 
    /// This method returns information about WebSocket connections if WebSocket
    /// transport is supported and enabled.
    pub fn get_websocket_status(&self) -> Option<WebSocketStatus> {
        if !self.transport.supports_ws() && !self.transport.supports_wss() {
            return None;
        }

        Some(WebSocketStatus {
            ws_connections: self.transport.get_connection_count(TransportType::Ws),
            wss_connections: self.transport.get_connection_count(TransportType::Wss),
            has_active_connection: self.transport.is_transport_connected(TransportType::Ws) || 
                                   self.transport.is_transport_connected(TransportType::Wss),
        })
    }
}

impl fmt::Debug for TransactionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Avoid trying to print the Mutex contents directly or requiring Debug on contents
        f.debug_struct("TransactionManager")
            .field("transport", &"Arc<dyn Transport>")
            .field("client_transactions", &"Arc<Mutex<HashMap<...>>>") // Indicate map exists
            .field("server_transactions", &"Arc<Mutex<HashMap<...>>>")
            .field("transaction_destinations", &"Arc<Mutex<HashMap<...>>>")
            .field("events_tx", &self.events_tx) // Sender might be Debug
            .field("event_subscribers", &"Arc<Mutex<Vec<Sender>>>")
            .field("transport_rx", &"Arc<Mutex<Receiver>>")
            .field("running", &self.running)
            .field("timer_settings", &self.timer_settings)
            .field("timer_manager", &"Arc<TimerManager>")
            .field("timer_factory", &"TimerFactory")
            .finish()
    } 
}