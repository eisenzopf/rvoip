//! TransactionManager constructor methods

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::{Mutex, mpsc};
use tracing::{debug, info, warn};

use rvoip_sip_transport::{Transport, TransportEvent};

use crate::transaction::error::{Error, Result};
use crate::transaction::timer::{TimerManager, TimerFactory, TimerSettings};
use crate::transaction::TransactionEvent;

use super::{TransactionManager, BoxedClientTransaction, BoxedServerTransaction};

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
    /// # use rvoip_dialog_core::transaction::TransactionManager;
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

        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

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
            shutdown_tx,
            transport_manager: None,
            aor_peer_routes: Arc::new(Mutex::new(HashMap::new())),
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
    /// # use rvoip_dialog_core::transaction::{TransactionManager, timer::TimerSettings};
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

        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

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
            shutdown_tx,
            transport_manager: None,
            aor_peer_routes: Arc::new(Mutex::new(HashMap::new())),
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
    /// # use rvoip_dialog_core::transaction::TransactionManager;
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
    /// # use rvoip_dialog_core::transaction::{TransactionManager, transport::TransportManager, transport::TransportManagerConfig};
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
        
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

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
            shutdown_tx,
            transport_manager: Some(Arc::new(transport_manager)),
            aor_peer_routes: Arc::new(Mutex::new(HashMap::new())),
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
    /// # use rvoip_dialog_core::transaction::{TransactionManager, timer::TimerSettings};
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
        
        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

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
            shutdown_tx,
            transport_manager: None,
            aor_peer_routes: Arc::new(Mutex::new(HashMap::new())),
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

        let (shutdown_tx, _) = tokio::sync::broadcast::channel(1);

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
            shutdown_tx,
            transport_manager: None,
            aor_peer_routes: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
