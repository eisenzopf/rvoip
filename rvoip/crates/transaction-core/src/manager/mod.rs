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

use crate::error::{self, Error, Result};
use crate::transaction::{
    Transaction, TransactionAsync, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand,
};
use crate::client::{
    ClientTransaction, 
    ClientInviteTransaction, 
    ClientNonInviteTransaction,
    TransactionExt,
};
use crate::server::{ServerTransaction, ServerInviteTransaction, ServerNonInviteTransaction};
use crate::timer::{Timer, TimerManager, TimerFactory, TimerSettings};
use crate::method::{cancel, update, ack};
use crate::utils::{transaction_key_from_message, generate_branch, extract_cseq, create_ack_from_invite};

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
    /// # use rvoip_transaction_core::TransactionManager;
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
    /// # use rvoip_transaction_core::{TransactionManager, timer::TimerSettings};
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
    /// # use rvoip_transaction_core::TransactionManager;
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
    /// # use rvoip_transaction_core::{TransactionManager, timer::TimerSettings};
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
        let (events_tx, _) = mpsc::channel(100);
        let (_, transport_rx) = mpsc::channel(100);
        
        let client_transactions = Arc::new(Mutex::new(HashMap::new()));
        let server_transactions = Arc::new(Mutex::new(HashMap::new()));
        let transaction_destinations = Arc::new(Mutex::new(HashMap::new()));
        let event_subscribers = Arc::new(Mutex::new(Vec::new()));
        let transport_rx = Arc::new(Mutex::new(transport_rx));
        let running = Arc::new(Mutex::new(false));
        
        // Create timer settings
        let timer_settings = timer_settings_opt.unwrap_or_else(TimerSettings::default);
        
        // Create the timer manager with custom config
        let timer_manager = Arc::new(TimerManager::new(Some(timer_settings.clone())));
        let timer_factory = TimerFactory::new(Some(timer_settings.clone()), timer_manager.clone());
        
        Self {
            transport,
            client_transactions,
            server_transactions,
            transaction_destinations,
            events_tx,
            event_subscribers,
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
    /// # use rvoip_transaction_core::TransactionManager;
    /// # use rvoip_transaction_core::transaction::TransactionKey;
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
        use crate::client::TransactionExt;
        
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
                    rvoip_sip_transport::Error::ConnectionFailed("Transaction terminated immediately".into()),
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
                                rvoip_sip_transport::Error::ConnectionFailed("Transport error during request send".into()),
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
                                    rvoip_sip_transport::Error::ConnectionFailed("Transaction terminated unexpectedly".into()),
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
                            rvoip_sip_transport::Error::ConnectionFailed("Transaction terminated after processing".into()),
                            "Failed to send request - transaction terminated"
                        ));
                    }
                } else {
                    // Transaction was removed
                    debug!(%transaction_id, "Transaction was removed - likely due to termination");
                    return Err(Error::transport_error(
                        rvoip_sip_transport::Error::ConnectionFailed("Transaction was removed".into()),
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
                                rvoip_sip_transport::Error::ConnectionFailed("Transaction terminated after timeout".into()),
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
                            rvoip_sip_transport::Error::ConnectionFailed("Transaction was removed after timeout".into()),
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
    /// # use rvoip_transaction_core::TransactionManager;
    /// # use rvoip_transaction_core::transaction::TransactionKey;
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
        use crate::server::TransactionExt;
        
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
    /// # use rvoip_transaction_core::TransactionManager;
    /// # use rvoip_transaction_core::transaction::TransactionKey;
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
    /// # use rvoip_transaction_core::TransactionManager;
    /// # use rvoip_transaction_core::transaction::{TransactionKey, TransactionState};
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
    /// # use rvoip_transaction_core::TransactionManager;
    /// # use rvoip_transaction_core::transaction::{TransactionKey, TransactionKind};
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
    /// # use rvoip_transaction_core::TransactionManager;
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

    /// Subscribes to transaction events.
    ///
    /// This method returns a receiver that will receive all transaction events
    /// emitted by this manager. This is the primary way for the application
    /// to be notified of transaction state changes, incoming messages, and
    /// other important events.
    ///
    /// ## Transaction Events
    ///
    /// The subscriber will receive events including:
    /// - Transaction state changes
    /// - Incoming requests
    /// - Response events (provisional and final)
    /// - Transaction timeouts
    /// - Special events for ACK and CANCEL
    ///
    /// # Returns
    /// * `mpsc::Receiver<TransactionEvent>` - Channel for receiving events
    ///
    /// # Example
    /// ```no_run
    /// # use rvoip_transaction_core::TransactionManager;
    /// # use rvoip_transaction_core::transaction::TransactionEvent;
    /// # use tokio::sync::mpsc;
    /// # async fn example(manager: &TransactionManager) {
    /// // Subscribe to transaction events
    /// let mut event_rx = manager.subscribe();
    ///
    /// // Process events in a loop
    /// tokio::spawn(async move {
    ///     while let Some(event) = event_rx.recv().await {
    ///         match event {
    ///             TransactionEvent::StateChanged { transaction_id, previous_state, new_state } => {
    ///                 println!("Transaction {} changed state: {:?} -> {:?}",
    ///                     transaction_id, previous_state, new_state);
    ///             },
    ///             TransactionEvent::SuccessResponse { transaction_id, response, .. } => {
    ///                 println!("Transaction {} received success response: {}",
    ///                     transaction_id, response.status());
    ///             },
    ///             _ => println!("Received event: {:?}", event),
    ///         }
    ///     }
    /// });
    /// # }
    /// ```
    pub fn subscribe(&self) -> mpsc::Receiver<TransactionEvent> {
        // Use a larger buffer to prevent backpressure
        let (tx, rx) = mpsc::channel(100);

        // Add logging and diagnostics
        debug!("New subscription to transaction events created");
        
        // Clone necessary variables for the async block
        let subscribers_clone = self.event_subscribers.clone();
        let tx_clone = tx.clone();
        
        // Add the sender asynchronously
        tokio::spawn(async move {
            // Add subscriber to the list immediately
            subscribers_clone.lock().await.push(tx_clone);
            
            // Periodically check if the channel is still open
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                if tx.is_closed() {
                    debug!("Event subscription channel is closed, will be removed on next broadcast");
                    break;
                }
            }
        });

        rx
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

    /// Broadcast events to primary and all subscriber channels
    async fn broadcast_event(
        event: TransactionEvent,
        primary_tx: &mpsc::Sender<TransactionEvent>,
        subscribers: &Arc<Mutex<Vec<mpsc::Sender<TransactionEvent>>>>,
        manager: Option<TransactionManager>, // Optional manager for processing termination events
    ) {
        // Create detailed logging about the event
        if let TransactionEvent::StateChanged { transaction_id, previous_state, new_state } = &event {
            debug!(%transaction_id, previous_state=?previous_state, new_state=?new_state, "Broadcasting state change event");
        } else if let TransactionEvent::TransactionTerminated { transaction_id } = &event {
            debug!(%transaction_id, "Broadcasting transaction termination event");
        } else {
            debug!(event_type=?std::mem::discriminant(&event), "Broadcasting event");
        }
        
        // Process termination events with the manager if provided
        if let TransactionEvent::TransactionTerminated { transaction_id } = &event {
            if let Some(manager) = manager.clone() {
                let transaction_id_clone = transaction_id.clone();
                // Process in the background to avoid blocking event distribution
                tokio::spawn(async move {
                    manager.process_transaction_terminated(&transaction_id_clone).await;
                });
            }
        }
        
        // First send to primary with retry
        for retry in 0..3 {
            match primary_tx.send(event.clone()).await {
                Ok(_) => {
                    debug!("Event sent to primary channel");
                    break;
                },
                Err(e) if retry < 2 => {
                    warn!("Failed to send event to primary channel, retrying: {}", e);
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                },
                Err(e) => {
                    error!("Failed to send event to primary channel after retries: {}", e);
                }
            }
        }
        
        // Send to all subscribers and collect any that failed
        let mut subscribers_guard = subscribers.lock().await;
        let mut closed_indices = Vec::new();
        
        for (idx, subscriber) in subscribers_guard.iter().enumerate() {
            if subscriber.is_closed() {
                closed_indices.push(idx);
                continue;
            }
            
            match subscriber.try_send(event.clone()) {
                Ok(_) => {
                    trace!("Event sent to subscriber");
                },
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Channel is full, try async send with small timeout
                    match tokio::time::timeout(tokio::time::Duration::from_millis(50), 
                                              subscriber.send(event.clone())).await {
                        Ok(Ok(_)) => trace!("Event sent to subscriber after waiting"),
                        Ok(Err(_)) => {
                            warn!("Subscriber channel closed after waiting");
                            closed_indices.push(idx);
                        },
                        Err(_) => {
                            warn!("Timed out sending to subscriber channel");
                            // Don't mark for removal since it might just be slow
                        }
                    }
                },
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    debug!("Subscriber channel closed, marking for removal");
                    closed_indices.push(idx);
                },
            }
        }
        
        // Remove any closed subscribers
        if !closed_indices.is_empty() {
            closed_indices.sort_unstable_by(|a, b| b.cmp(a)); // Sort in reverse order
            let indices_count = closed_indices.len();
            for idx in closed_indices {
                if idx < subscribers_guard.len() {
                    subscribers_guard.swap_remove(idx);
                }
            }
            debug!("Removed {} closed subscriber channels", indices_count);
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
                        if let Err(e) = handle_transport_message(
                            message_event,
                            &transport_arc,
                            &client_transactions,
                            &server_transactions,
                            &events_tx,
                            &event_subscribers,
                            &manager_arc,
                        ).await {
                            error!("Error handling transport message: {}", e);
                        }
                    }
                    Some(transaction_event) = internal_rx.recv() => {
                        // Handle transaction events, particularly termination events
                        Self::broadcast_event(
                            transaction_event, 
                            &events_tx, 
                            &event_subscribers,
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

    /// Create a client transaction for sending a SIP request
    /// The caller is responsible for calling send_request() to initiate the transaction.
    pub async fn create_client_transaction(
        &self,
        request: Request,
        destination: SocketAddr,
    ) -> Result<TransactionKey> {
        debug!(method = %request.method(), %destination, "Creating client transaction");
        
        // Check if we've already registered a transaction with that branch
        let branch = match request.first_via() {
            Some(via) => {
                match via.branch() {
                    Some(b) if b.starts_with(RFC3261_BRANCH_MAGIC_COOKIE) => b.to_string(),
                    _ => generate_branch(),
                }
            },
            None => generate_branch(),
        };
        
        let local_addr = self.transport.local_addr()
            .map_err(|e| Error::transport_error(e, "Failed to get local address"))?;
            
        // Create request with proper Via header
        let mut modified_request = request.clone();
        
        // Now add our Via header to the top of the stack
        let local_host = local_addr.ip().to_string();
        let local_port = local_addr.port();
        
        // Create a new Via header and insert it
        let mut via_params = vec![Param::branch(&branch)];
        
        // Add other params like rport if needed
        // Example: via_params.push(Param::other("rport".to_string(), None));
        
        let via = Via::new("SIP", "2.0", "UDP", &local_host, Some(local_port), via_params)
            .map_err(Error::SipCoreError)?;
        
        modified_request.headers.insert(0, TypedHeader::Via(via));
        
        // Create transaction key/ID directly with is_server: false
        let key = TransactionKey::new(branch, modified_request.method().clone(), false);
            
        // Store the destination so it can be used for ACKs outside the transaction
        {
            let mut dest_map = self.transaction_destinations.lock().await;
            dest_map.insert(key.clone(), destination);
        }
        
        // Create transaction based on request type
        match modified_request.method() {
            Method::Invite => {
                // Create an INVITE client transaction
                let transaction = ClientInviteTransaction::new(
                    key.clone(),
                    modified_request,
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    Some(self.timer_settings.clone()),
                )?;
                
                // Store the transaction
                let mut client_txs = self.client_transactions.lock().await;
                client_txs.insert(key.clone(), Box::new(transaction));
            },
            Method::Cancel | Method::Update | _ => {
                // For CANCEL, validate that it's properly formed using the utility
                if modified_request.method() == Method::Cancel {
                    // Validate but don't fail - just log a warning
                    if let Err(e) = cancel::validate_cancel_request(&modified_request) {
                        warn!(method = %modified_request.method(), error = %e, "Creating transaction for CANCEL with possible validation issues");
                    }
                }
                
                // For UPDATE, validate that it's properly formed using the utility
                if modified_request.method() == Method::Update {
                    // Validate but don't fail - just log a warning
                    if let Err(e) = update::validate_update_request(&modified_request) {
                        warn!(method = %modified_request.method(), error = %e, "Creating transaction for UPDATE with possible validation issues");
                    }
                }
                
                // Create a non-INVITE client transaction for all other methods
                let transaction = ClientNonInviteTransaction::new(
                    key.clone(),
                    modified_request,
                    destination,
                    self.transport.clone(),
                    self.events_tx.clone(),
                    Some(self.timer_settings.clone()),
                )?;
                
                // Store the transaction
                let mut client_txs = self.client_transactions.lock().await;
                client_txs.insert(key.clone(), Box::new(transaction));
            }
        }
        
        debug!(id=%key, "Created client transaction");
        
        Ok(key)
    }

    /// Send an ACK for a 2xx response
    pub async fn send_2xx_ack(
        &self,
        final_response: &Response,
    ) -> Result<()> {
        // Get Request-URI from Contact or To
        let request_uri = match final_response.header(&HeaderName::Contact) { 
            Some(contact) => {
                if let TypedHeader::Contact(contact_val) = contact {
                    contact_val.addresses().next().map(|a| a.uri.clone())
                } else { None }
            },
            None => None,
        }.or_else(|| {
            final_response.header(&HeaderName::To)
                .and_then(|to| if let TypedHeader::To(to_val) = to { 
                    Some(to_val.address().uri.clone()) 
                } else { None })
        }).ok_or_else(|| Error::Other("Cannot determine Request-URI for ACK from response".into()))?;

        let mut ack_builder = RequestBuilder::new(Method::Ack, &request_uri.to_string())?;

        if let Some(via) = final_response.first_via() {
            ack_builder = ack_builder.header(TypedHeader::Via(via.clone())); // Wrap
        } else {
            return Err(Error::Other("Cannot determine Via header for ACK from response".into()));
        }
        
        if let Some(from) = final_response.header(&HeaderName::From) {
            if let TypedHeader::From(from_val) = from {
                ack_builder = ack_builder.header(TypedHeader::From(from_val.clone())); // Wrap
            } else {
                return Err(Error::Other("Missing or invalid From header in response for ACK".into()));
            }
        } else {
            return Err(Error::Other("Missing From header in response for ACK".into()));
        }
        
        if let Some(to) = final_response.header(&HeaderName::To) {
            if let TypedHeader::To(to_val) = to {
                ack_builder = ack_builder.header(TypedHeader::To(to_val.clone())); // Wrap
            } else {
                return Err(Error::Other("Missing or invalid To header in response for ACK".into()));
            }
        } else {
            return Err(Error::Other("Missing To header in response for ACK".into()));
        }
        
        if let Some(call_id) = final_response.header(&HeaderName::CallId) {
            if let TypedHeader::CallId(call_id_val) = call_id {
                ack_builder = ack_builder.header(TypedHeader::CallId(call_id_val.clone())); // Wrap
            } else {
                return Err(Error::Other("Missing or invalid Call-ID header in response for ACK".into()));
            }
        } else {
            return Err(Error::Other("Missing Call-ID header in response for ACK".into()));
        }
        
        // Use utils::extract_cseq on the message
        if let Some((seq, _)) = extract_cseq(&Message::Response(final_response.clone())) {
            ack_builder = ack_builder.header(TypedHeader::CSeq(CSeq::new(seq, Method::Ack))); // Wrap
        } else {
            return Err(Error::Other("Missing or invalid CSeq header in response for ACK".into()));
        }

        ack_builder = ack_builder.header(TypedHeader::MaxForwards(MaxForwards::new(70))); // Wrap
        ack_builder = ack_builder.header(TypedHeader::ContentLength(ContentLength::new(0))); // Wrap

        let ack_request = ack_builder.build();

        let destination = handlers::determine_ack_destination(final_response).await
            .ok_or_else(|| Error::Other("Could not determine destination for ACK".into()))?;

        info!(%destination, "Sending ACK for 2xx response");

        self.transport.send_message(
            Message::Request(ack_request),
            destination
        ).await.map_err(|e| Error::transport_error(e, "Failed to send ACK for non-2xx response"))
    }

    // Test-only method to get server transactions for debugging
    #[cfg(test)]
    pub async fn get_server_transactions_for_test(&self) -> Vec<String> {
        let transactions = self.server_transactions.lock().await;
        transactions.keys().map(|k| k.to_string()).collect()
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
        let ack_request = crate::method::ack::create_ack_for_2xx(&invite_request, response, &local_addr)?;
        
        Ok(ack_request)
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
        let tx_id = crate::method::cancel::find_invite_transaction_for_cancel(
            cancel_request, 
            invite_tx_keys
        );
        
        Ok(tx_id)
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