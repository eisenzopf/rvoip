/// # Non-INVITE Client Transaction Implementation
///
/// This module implements the non-INVITE client transaction state machine as defined in
/// [RFC 3261 Section 17.1.2](https://datatracker.ietf.org/doc/html/rfc3261#section-17.1.2).
///
/// ## State Machine
///
/// The non-INVITE client transaction follows this state machine:
///
/// ```text
///                                  |Request from TU
///                                  |send request
///               Timer E            V
///               send request  +-----------+
///                   +---------|           |-------------------+
///                   |         |  Trying   |  Timer F fires    |
///                   +-------->|           |  or Transport Err.|
///                             +-----------+  inform TU        |
///                               |  |                          |
///                               |  |1xx                       |
///                               |  |from                      |
///                               |  |TL                        |
///           Timer E             |  |                          |
///           send request        |  |                          |
///               +---------------|--|---+                      |
///               |               |  |   |                      |
///               |               |  |   |                      |
///               |               |  |   |                      |
///               |               |  |   |                      |
///       +-------V------+        |  |   |                      |
///       |              |<-------+  |   |                      |
///       |  Proceeding  |          |   |                      |
///       |              |----------+   |    2xx from TL       |
///       +-------+------+              |    inform TU         |
///               |                     |                       |
///               |                     |                       |
///               | 300-699 from TL     |                       |
///               | inform TU           |                       |
///               |                     |                       |
///               |                     |                       |
///       +-------V------+          +---+---+                   |
///       |              |          |       |                   |
///       |  Completed   |          | Term. |<------------------+
///       |              |          |       |
///       +-------+------+          +-------+
///               |
///        Timer K|
///               |
///               V
///         +-----------+
///         |           |
///         | Terminated|
///         |           |
///         +-----------+
/// ```
///
/// ## Timers
///
/// Non-INVITE client transactions use the following timers:
///
/// - **Timer E**: Initial value T1, doubles on each retransmission (up to T2). Controls request retransmissions.
/// - **Timer F**: Typically 64*T1. Controls transaction timeout. When it fires, the transaction terminates with an error.
/// - **Timer K**: Typically 5s. Controls how long to wait in Completed state for response retransmissions.

use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, error, trace, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;

use crate::transaction::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionAsync, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand, AtomicTransactionState,
};
use crate::transaction::timer::{TimerSettings, TimerFactory, TimerManager, TimerType};
use crate::transaction::client::data::CommonClientTransaction;
use crate::transaction::client::{ClientTransaction, ClientTransactionData};
use crate::transaction::utils;
use crate::transaction::logic::TransactionLogic;
use crate::transaction::runner::{run_transaction_loop, HasCommandSender, AsRefKey};
use crate::transaction::timer_utils;
use crate::transaction::validators;
use crate::transaction::common_logic;

/// Client non-INVITE transaction implementation as defined in RFC 3261 Section 17.1.2.
///
/// This struct implements the state machine for client non-INVITE transactions, which are used
/// for all request methods except INVITE. Non-INVITE transactions have simpler behavior than
/// INVITE transactions, including:
///
/// - No ACK generation required (unlike INVITE transactions)
/// - Different timer requirements (Timers E, F, K instead of A, B, D)
/// - Different state transitions (e.g., non-INVITE transactions can go from Trying directly to Completed)
///
/// Key behaviors:
/// - In Trying state: Retransmits request periodically until response or timeout
/// - In Proceeding state: Continues retransmissions until final response
/// - In Completed state: Has received a final response, waiting for retransmissions
/// - In Terminated state: Transaction is finished
#[derive(Debug, Clone)]
pub struct ClientNonInviteTransaction {
    data: Arc<ClientTransactionData>,
    logic: Arc<ClientNonInviteLogic>,
}

/// Holds JoinHandles and dynamic state for timers specific to Client Non-INVITE transactions.
///
/// Used by the transaction runner to manage the various timers required by the
/// non-INVITE client transaction state machine as defined in RFC 3261.
#[derive(Default, Debug)]
struct ClientNonInviteTimerHandles {
    /// Handle for Timer E, which controls request retransmissions
    timer_e: Option<JoinHandle<()>>,
    
    /// Current interval for Timer E, which doubles after each firing (up to T2)
    current_timer_e_interval: Option<Duration>, // For backoff
    
    /// Handle for Timer F, which controls transaction timeout
    timer_f: Option<JoinHandle<()>>,
    
    /// Handle for Timer K, which controls how long to wait in Completed state
    timer_k: Option<JoinHandle<()>>,
}

/// Implements the TransactionLogic for Client Non-INVITE transactions.
///
/// This struct contains the core logic for the non-INVITE client transaction state machine,
/// implementing the behavior defined in RFC 3261 Section 17.1.2.
#[derive(Debug, Clone, Default)]
struct ClientNonInviteLogic {
    _data_marker: std::marker::PhantomData<ClientTransactionData>,
    timer_factory: TimerFactory,
}

impl ClientNonInviteLogic {
    /// Start Timer E (retransmission timer) using timer utils
    ///
    /// This method starts Timer E, which controls retransmissions of the request
    /// in the Trying and Proceeding states. The initial interval is T1, and it
    /// doubles on each retransmission up to T2.
    async fn start_timer_e(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientNonInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        // Start Timer E (retransmission) with initial interval T1
        let initial_interval_e = timer_config.t1;
        timer_handles.current_timer_e_interval = Some(initial_interval_e);
        
        // Use timer_utils to start the timer
        let timer_manager = self.timer_factory.timer_manager();
        match timer_utils::start_transaction_timer(
            &timer_manager,
            tx_id,
            "E",
            TimerType::E,
            initial_interval_e,
            command_tx
        ).await {
            Ok(handle) => {
                timer_handles.timer_e = Some(handle);
                trace!(id=%tx_id, interval=?initial_interval_e, "Started Timer E for Trying state");
            },
            Err(e) => {
                error!(id=%tx_id, error=%e, "Failed to start Timer E");
            }
        }
    }
    
    /// Start Timer F (transaction timeout) using timer utils
    ///
    /// This method starts Timer F, which controls the overall transaction timeout.
    /// If Timer F fires before a final response is received, the transaction fails
    /// with a timeout error.
    async fn start_timer_f(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientNonInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        // Start Timer F (transaction timeout)
        let interval_f = timer_config.transaction_timeout;
        
        // Use timer_utils to start the timer
        let timer_manager = self.timer_factory.timer_manager();
        match timer_utils::start_transaction_timer(
            &timer_manager,
            tx_id,
            "F",
            TimerType::F,
            interval_f,
            command_tx
        ).await {
            Ok(handle) => {
                timer_handles.timer_f = Some(handle);
                trace!(id=%tx_id, interval=?interval_f, "Started Timer F for Trying state");
            },
            Err(e) => {
                error!(id=%tx_id, error=%e, "Failed to start Timer F");
            }
        }
    }
    
    /// Start Timer K (wait for response retransmissions) using timer utils
    ///
    /// This method starts Timer K, which controls how long to wait in the Completed
    /// state for retransmissions of the final response. When Timer K fires, the
    /// transaction transitions to the Terminated state.
    async fn start_timer_k(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientNonInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        // Start Timer K that automatically transitions to Terminated state when it fires
        let interval_k = timer_config.wait_time_k;
        
        // Use timer_utils to start the timer with transition
        let timer_manager = self.timer_factory.timer_manager();
        match timer_utils::start_timer_with_transition(
            &timer_manager,
            tx_id,
            "K",
            TimerType::K,
            interval_k,
            command_tx,
            TransactionState::Terminated
        ).await {
            Ok(handle) => {
                timer_handles.timer_k = Some(handle);
                trace!(id=%tx_id, interval=?interval_k, "Started Timer K for Completed state");
            },
            Err(e) => {
                error!(id=%tx_id, error=%e, "Failed to start Timer K");
            }
        }
    }
    
    /// Handle initial request sending in Trying state
    ///
    /// This method is called when the transaction enters the Trying state.
    /// It sends the initial request and starts Timers E and F according to RFC 3261 Section 17.1.2.2.
    async fn handle_trying_state(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientNonInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<()> {
        let tx_id = &data.id;
        
        // Send the initial request
        debug!(id=%tx_id, "ClientNonInviteLogic: Sending initial request in Trying state");
        let request_guard = data.request.lock().await;
        if let Err(e) = data.transport.send_message(
            Message::Request(request_guard.clone()),
            data.remote_addr
        ).await {
            error!(id=%tx_id, error=%e, "Failed to send initial request from Trying state");
            common_logic::send_transport_error_event(tx_id, &data.events_tx).await;
            // If send fails, command a transition to Terminated
            let _ = command_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
            return Err(Error::transport_error(e, "Failed to send initial request"));
        }
        drop(request_guard); // Release lock

        // Start timers for Trying state
        self.start_timer_e(data, timer_handles, command_tx.clone()).await;
        self.start_timer_f(data, timer_handles, command_tx).await;
        
        Ok(())
    }

    /// Handle Timer E (retransmission) trigger
    ///
    /// This method is called when Timer E fires. According to RFC 3261 Section 17.1.2.2,
    /// when Timer E fires in the Trying or Proceeding state, the client should retransmit
    /// the request and restart Timer E with a doubled interval (capped by T2).
    async fn handle_timer_e_trigger(
        &self,
        data: &Arc<ClientTransactionData>,
        current_state: TransactionState,
        timer_handles: &mut ClientNonInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        match current_state {
            TransactionState::Trying | TransactionState::Proceeding => {
                debug!(id=%tx_id, "Timer E triggered, retransmitting request");
                
                // Retransmit the request
                let request_guard = data.request.lock().await;
                if let Err(e) = data.transport.send_message(
                    Message::Request(request_guard.clone()),
                    data.remote_addr
                ).await {
                    error!(id=%tx_id, error=%e, "Failed to retransmit request");
                    common_logic::send_transport_error_event(tx_id, &data.events_tx).await;
                    return Ok(Some(TransactionState::Terminated));
                }
                
                // Update and restart Timer E with increased interval using the utility function
                let current_interval = timer_handles.current_timer_e_interval.unwrap_or(timer_config.t1);
                let new_interval = timer_utils::calculate_backoff_interval(current_interval, timer_config);
                timer_handles.current_timer_e_interval = Some(new_interval);
                
                // Start new Timer E with the increased interval
                let timer_manager = self.timer_factory.timer_manager();
                match timer_utils::start_transaction_timer(
                    &timer_manager,
                    tx_id,
                    "E",
                    TimerType::E,
                    new_interval,
                    command_tx
                ).await {
                    Ok(handle) => {
                        timer_handles.timer_e = Some(handle);
                        trace!(id=%tx_id, interval=?new_interval, "Restarted Timer E with backoff");
                    },
                    Err(e) => {
                        error!(id=%tx_id, error=%e, "Failed to restart Timer E");
                    }
                }
            },
            _ => {
                trace!(id=%tx_id, state=?current_state, "Timer E fired in invalid state, ignoring");
            }
        }
        
        Ok(None)
    }
    
    /// Handle Timer F (transaction timeout) trigger
    ///
    /// This method is called when Timer F fires. According to RFC 3261 Section 17.1.2.2,
    /// when Timer F fires in the Trying or Proceeding state, the client should inform
    /// the TU that the transaction has timed out and transition to the Terminated state.
    async fn handle_timer_f_trigger(
        &self,
        data: &Arc<ClientTransactionData>,
        current_state: TransactionState,
        _command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match current_state {
            TransactionState::Trying | TransactionState::Proceeding => {
                warn!(id=%tx_id, "Timer F (Timeout) fired in state {:?}", current_state);
                
                // Notify TU about timeout using common logic
                common_logic::send_transaction_timeout_event(tx_id, &data.events_tx).await;
                
                // Return state transition
                return Ok(Some(TransactionState::Terminated));
            },
            _ => {
                trace!(id=%tx_id, state=?current_state, "Timer F fired in invalid state, ignoring");
            }
        }
        
        Ok(None)
    }
    
    /// Handle Timer K (wait for retransmissions) trigger
    ///
    /// This method is called when Timer K fires. According to RFC 3261 Section 17.1.2.2,
    /// when Timer K fires in the Completed state, the client should transition to the
    /// Terminated state. Timer K ensures that any retransmissions of the final response
    /// are properly received before the transaction terminates.
    async fn handle_timer_k_trigger(
        &self,
        data: &Arc<ClientTransactionData>,
        current_state: TransactionState,
        _command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match current_state {
            TransactionState::Completed => {
                debug!(id=%tx_id, "Timer K fired in Completed state, terminating");
                // Timer K automatically transitions to Terminated, no need to return a state
                Ok(None)
            },
            _ => {
                trace!(id=%tx_id, state=?current_state, "Timer K fired in invalid state, ignoring");
                Ok(None)
            }
        }
    }

    /// Process a response and determine state transition
    ///
    /// This method handles incoming responses according to RFC 3261 Section 17.1.2.2
    /// and manages Timer E cancellation for proper retransmission control.
    async fn process_response(
        &self,
        data: &Arc<ClientTransactionData>,
        response: Response,
        current_state: TransactionState,
        timer_handles: &mut ClientNonInviteTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        debug!(id=%tx_id, status=%response.status(), state=?current_state, "🔍 DEBUG: process_response called");
        
        // Get the original method from the request to validate the response
        let request_guard = data.request.lock().await;
        let original_method = validators::get_method_from_request(&request_guard);
        drop(request_guard);
        
        // Validate that the response matches our transaction
        if let Err(e) = validators::validate_response_matches_transaction(&response, tx_id, &original_method) {
            warn!(id=%tx_id, error=%e, "Response validation failed");
            return Ok(None);
        }
        
        // Get status information for timer management
        let status = response.status();
        let is_provisional = status.is_provisional();
        let is_final = !is_provisional;
        
        debug!(id=%tx_id, status=%status, is_provisional=%is_provisional, is_final=%is_final, state=?current_state, 
               "🔍 DEBUG: Response classification");
        
        match current_state {
            TransactionState::Trying | TransactionState::Proceeding => {
                debug!(id=%tx_id, "🔍 DEBUG: In Trying/Proceeding state");
                
                // **RFC 3261 COMPLIANCE**: Cancel Timer E for final responses
                // Section 17.1.2.2: "When a final response is received, the client
                // transaction enters the Completed state after possibly generating an ACK"
                if is_final {
                    debug!(id=%tx_id, "🔍 DEBUG: This is a final response, checking Timer E");
                    
                    // Cancel Timer E (retransmission timer) for final responses
                    if let Some(handle) = timer_handles.timer_e.take() {
                        handle.abort();
                        debug!(id=%tx_id, status=%status, "✅ Cancelled Timer E (final response received)");
                    } else {
                        debug!(id=%tx_id, "🔍 DEBUG: No Timer E handle found to cancel");
                    }
                    // Reset the interval tracking
                    timer_handles.current_timer_e_interval = None;
                } else {
                    debug!(id=%tx_id, "🔍 DEBUG: This is a provisional response, keeping Timer E running");
                }
                
                // Note: Timer F (transaction timeout) is left running until state transition
                // It will be cancelled by the runner's cancel_all_specific_timers during state change
            },
            _ => {
                // In other states, no timer changes needed for response processing
                debug!(id=%tx_id, state=?current_state, "🔍 DEBUG: In non-active state, no timer changes");
            }
        }
        
        // Use the common_logic handler which works for both INVITE and non-INVITE transactions
        // For non-INVITE transactions, is_invite is false
        let new_state = common_logic::handle_response_by_status(
            tx_id, 
            response.clone(), 
            current_state, 
            &data.events_tx,
            false, // non-INVITE
            data.remote_addr
        ).await;
        
        debug!(id=%tx_id, old_state=?current_state, new_state=?new_state, "🔍 DEBUG: State transition result");
        
        Ok(new_state)
    }
}

#[async_trait::async_trait]
impl TransactionLogic<ClientTransactionData, ClientNonInviteTimerHandles> for ClientNonInviteLogic {
    fn kind(&self) -> TransactionKind {
        TransactionKind::NonInviteClient
    }

    fn initial_state(&self) -> TransactionState {
        TransactionState::Initial
    }

    fn timer_settings<'a>(data: &'a Arc<ClientTransactionData>) -> &'a TimerSettings {
        &data.timer_config
    }

    fn cancel_all_specific_timers(&self, timer_handles: &mut ClientNonInviteTimerHandles) {
        if let Some(handle) = timer_handles.timer_e.take() {
            handle.abort();
        }
        if let Some(handle) = timer_handles.timer_f.take() {
            handle.abort();
        }
        if let Some(handle) = timer_handles.timer_k.take() {
            handle.abort();
        }
        // Resetting current_timer_e_interval here might be good practice
        timer_handles.current_timer_e_interval = None;
    }

    async fn on_enter_state(
        &self,
        data: &Arc<ClientTransactionData>,
        new_state: TransactionState,
        previous_state: TransactionState,
        timer_handles: &mut ClientNonInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>, // This is the runner's command_tx
    ) -> Result<()> {
        let tx_id = &data.id;

        match new_state {
            TransactionState::Trying => {
                self.handle_trying_state(data, timer_handles, command_tx).await?;
            }
            TransactionState::Proceeding => {
                trace!(id=%tx_id, "Entered Proceeding state. Timers E & F continue.");
                // Timer E continues with its current backoff interval.
                // Timer F continues. No new timers are started specifically for entering Proceeding.
            }
            TransactionState::Completed => {
                // Start Timer K (wait for response retransmissions)
                self.start_timer_k(data, timer_handles, command_tx).await;
            }
            TransactionState::Terminated => {
                trace!(id=%tx_id, "Entered Terminated state. Specific timers should have been cancelled by runner.");
                // Unregister from timer manager when terminated
                let timer_manager = self.timer_factory.timer_manager();
                timer_utils::unregister_transaction(&timer_manager, tx_id).await;
            }
            _ => { // Initial state, or others not directly part of the main flow.
                trace!(id=%tx_id, "Entered unhandled state {:?} in on_enter_state", new_state);
            }
        }
        Ok(())
    }

    // Original handle_timer method required by the trait
    async fn handle_timer(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_name: &str,
        current_state: TransactionState,
        timer_handles: &mut ClientNonInviteTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        if timer_name == "E" {
            // Clear the timer handle since it fired
            timer_handles.timer_e.take();
        }
        
        // Send timer triggered event using common logic
        common_logic::send_timer_triggered_event(tx_id, timer_name, &data.events_tx).await;
        
        // Use the command_tx from data to set up retransmission timers
        let self_command_tx = data.cmd_tx.clone();
        
        match timer_name {
            "E" => self.handle_timer_e_trigger(data, current_state, timer_handles, self_command_tx).await,
            "F" => self.handle_timer_f_trigger(data, current_state, self_command_tx).await,
            "K" => self.handle_timer_k_trigger(data, current_state, self_command_tx).await,
            _ => {
                warn!(id=%tx_id, timer_name=%timer_name, "Unknown timer triggered for ClientNonInvite");
                Ok(None)
            }
        }
    }

    async fn process_message(
        &self,
        data: &Arc<ClientTransactionData>,
        message: Message,
        current_state: TransactionState,
        timer_handles: &mut ClientNonInviteTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        debug!(id=%tx_id, state=?current_state, "🔍 DEBUG: process_message called with message type: {}", 
               if message.is_response() { "Response" } else { "Request" });
        
        // Use the validators utility to extract and validate the response
        match validators::extract_response(&message, tx_id) {
            Ok(response) => {
                debug!(id=%tx_id, status=%response.status(), "🔍 DEBUG: Extracted response, storing and processing");
                
                // Store the response
                {
                    let mut last_response = data.last_response.lock().await;
                    *last_response = Some(response.clone());
                }
                
                // Use our helper for response processing with real timer handles
                let result = self.process_response(data, response, current_state, timer_handles).await;
                debug!(id=%tx_id, "🔍 DEBUG: process_response completed");
                result
            },
            Err(e) => {
                warn!(id=%tx_id, error=%e, "Received non-response message");
                Ok(None)
            }
        }
    }
}

impl ClientNonInviteTransaction {
    /// Create a new client non-INVITE transaction.
    ///
    /// This method creates a new non-INVITE client transaction with the specified parameters.
    /// It initializes the transaction data and spawns the transaction runner task.
    ///
    /// Non-INVITE transactions are used for all request methods except INVITE, including:
    /// - REGISTER: For user registration with a SIP registrar
    /// - OPTIONS: For querying capabilities of a SIP UA or server
    /// - BYE: For terminating a session
    /// - CANCEL: For canceling a pending INVITE
    /// - And others (SUBSCRIBE, NOTIFY, INFO, etc.)
    ///
    /// # Arguments
    ///
    /// * `id` - The unique identifier for this transaction
    /// * `request` - The non-INVITE request that initiates this transaction
    /// * `remote_addr` - The address to which the request should be sent
    /// * `transport` - The transport layer to use for sending messages
    /// * `events_tx` - The channel for sending events to the Transaction User
    /// * `timer_config_override` - Optional custom timer settings
    ///
    /// # Returns
    ///
    /// A Result containing the new ClientNonInviteTransaction or an error
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config_override: Option<TimerSettings>,
    ) -> Result<Self> {
        let timer_config = timer_config_override.unwrap_or_default();
        let (cmd_tx, local_cmd_rx) = mpsc::channel(32); // Renamed cmd_rx to local_cmd_rx to avoid conflict after ClientTransactionData change

        let data = Arc::new(ClientTransactionData {
            id: id.clone(),
            state: Arc::new(AtomicTransactionState::new(TransactionState::Initial)),
            request: Arc::new(Mutex::new(request.clone())),
            last_response: Arc::new(Mutex::new(None)),
            remote_addr,
            transport,
            events_tx,
            cmd_tx: cmd_tx.clone(), // For the transaction itself to send commands to its loop
            // cmd_rx is no longer stored here; it's passed directly to the spawned loop
            event_loop_handle: Arc::new(Mutex::new(None)),
            timer_config: timer_config.clone(),
        });

        let logic = Arc::new(ClientNonInviteLogic {
            _data_marker: std::marker::PhantomData,
            timer_factory: TimerFactory::new(Some(timer_config), Arc::new(TimerManager::new(None))),
        });

        let data_for_runner = data.clone();
        let logic_for_runner = logic.clone();

        // Spawn the generic event loop runner
        let event_loop_handle = tokio::spawn(async move {
            // local_cmd_rx is moved into the loop here
            run_transaction_loop(data_for_runner, logic_for_runner, local_cmd_rx).await;
        });

        // Store the handle for cleanup
        if let Ok(mut handle_guard) = data.event_loop_handle.try_lock() {
            *handle_guard = Some(event_loop_handle);
        }
        
        Ok(Self { data, logic })
    }
}

impl ClientTransaction for ClientNonInviteTransaction {
    fn initiate(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        let kind = self.kind(); // Get kind for the error message
        let tx_id = self.data.id.clone(); // Get ID for logging
        
        Box::pin(async move {
            println!("ClientNonInviteTransaction::initiate called for {}", tx_id);
            let current_state = data.state.get();
            println!("Current state is {:?}", current_state);
            
            if current_state != TransactionState::Initial {
                println!("Invalid state transition: {:?} -> Trying", current_state);
                // Corrected Error::invalid_state_transition call
                return Err(Error::invalid_state_transition(
                    kind, // Pass the correct kind
                    current_state,
                    TransactionState::Trying,
                    Some(data.id.clone()), // Pass Option<TransactionKey>
                ));
            }

            println!("Sending TransitionTo(Trying) command for {}", tx_id);
            match data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Trying)).await {
                Ok(_) => {
                    println!("Successfully sent TransitionTo command for {}", tx_id);
                    // Wait a small amount of time to allow the transaction runner to process the command
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    
                    // Verify state change
                    let new_state = data.state.get();
                    println!("State after sending command: {:?}", new_state);
                    if new_state != TransactionState::Trying {
                        println!("WARNING: State didn't change to Trying, still: {:?}", new_state);
                    }
                    
                    Ok(())
                },
                Err(e) => {
                    println!("Failed to send command: {}", e);
                    Err(Error::Other(format!("Failed to send command: {}", e)))
                }
            }
        })
    }

    fn process_response(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        Box::pin(async move {
            trace!(id=%data.id, method=%response.status(), "Received response");
            
            data.cmd_tx.send(InternalTransactionCommand::ProcessMessage(Message::Response(response))).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))?;
            
            Ok(())
        })
    }

    // Implement the missing original_request method
    fn original_request(&self) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + '_>> {
        let request_arc = self.data.request.clone();
        Box::pin(async move {
            let req = request_arc.lock().await;
            Some(req.clone()) // Clone the request out of the Mutex guard
        })
    }

    // Add the required last_response implementation for ClientTransaction
    fn last_response<'a>(&'a self) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + 'a>> {
        // Create a future that just returns the last response
        let last_response = self.data.last_response.clone();
        Box::pin(async move {
            last_response.lock().await.clone()
        })
    }
}

impl Transaction for ClientNonInviteTransaction {
    fn id(&self) -> &TransactionKey {
        &self.data.id
    }

    fn kind(&self) -> TransactionKind {
        TransactionKind::NonInviteClient
    }

    fn state(&self) -> TransactionState {
        self.data.state.get()
    }
    
    fn remote_addr(&self) -> SocketAddr {
        self.data.remote_addr
    }
    
    fn matches(&self, message: &Message) -> bool {
        // Key matching logic (typically branch, method for non-INVITE client)
        // This can be simplified using utils if response matching rules are consistent.
        // For a client transaction, it matches responses based on:
        // 1. Topmost Via header's branch parameter matching the transaction ID's branch.
        // 2. CSeq method matching the original request's CSeq method.
        // (For non-INVITE, CSeq number doesn't have to match strictly for responses unlike INVITE ACK)
        if !message.is_response() { return false; }
        
        let response = match message {
            Message::Response(r) => r,
            _ => return false,
        };

        if let Some(TypedHeader::Via(via_header_vec)) = response.header(&HeaderName::Via) {
            if let Some(via_header) = via_header_vec.0.first() {
                if via_header.branch() != Some(self.data.id.branch()) {
                    return false;
                }
            } else {
                return false; // No Via value in the vector
            }
        } else {
            return false; // No Via header or not of TypedHeader::Via type
        }

        // Clone the method from the reference to get an owned Method
        let original_request_method = self.data.id.method().clone();
        if let Some(TypedHeader::CSeq(cseq_header)) = response.header(&HeaderName::CSeq) {
            if cseq_header.method != original_request_method {
                return false;
            }
        } else {
            return false; // No CSeq header or not of TypedHeader::CSeq type
        }
        
        // Call-ID, From tag, To tag must also match for strictness, though branch is primary.
        // This simplified check assumes branch + CSeq method is sufficient for this context.
        // RFC 3261 Section 17.1.3 provides full matching rules.
        // The `utils::transaction_key_from_message` is more for *creating* keys.
        // Here we are *matching* an incoming response to an existing client transaction.
        // The ID of the transaction IS the key we are looking for.

        // A more robust check would compare relevant fields directly or reconstruct a key from response
        // and compare. For now, top Via branch and CSeq method matching is a good start.
        // The most crucial part is that the response's top Via branch matches our transaction ID's branch.
        // And the CSeq method also matches.
        
        // Let's refine using transaction_key_from_message if it's suitable for responses too.
        // utils::transaction_key_from_message is primarily for requests.
        // For responses, client matches on:
        // - top Via branch == original request's top Via branch (which is stored in tx.id.branch)
        // - sent-protocol in Via is the same
        // - sent-by in Via matches the remote_addr we sent to (or is a NATed version)
        // - CSeq method matches
        // - For non-INVITE, CSeq num matching is not required for responses.

        // Assuming self.data.id.branch is the branch we sent in the request's Via.
        // Assuming self.data.id.method is the method of the original request.
        true // If passed Via and CSeq checks above
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl TransactionAsync for ClientNonInviteTransaction {
    fn process_event<'a>(
        &'a self,
        event_type: &'a str, // e.g. "response" from TransactionManager when it routes a message
        message: Option<Message>
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // The TransactionManager, when it receives a message from transport and matches it
            // to this transaction, will call `process_event` with "response" and the message.
            // This should then send an InternalTransactionCommand::ProcessMessage to the runner.
            match event_type {
                "response" => { // This name is illustrative, TM would use a specific trigger
                    if let Some(msg) = message {
                        self.data.cmd_tx.send(InternalTransactionCommand::ProcessMessage(msg)).await
                            .map_err(|e| Error::Other(format!("Failed to send ProcessMessage command: {}", e)))?;
                    } else {
                        return Err(Error::Other("Expected Message for 'response' event type".to_string()));
                    }
                },
                // Other event types if the TU or manager needs to directly interact via this generic method.
                // For now, direct commands are preferred.
                _ => return Err(Error::Other(format!("Unhandled event type in TransactionAsync::process_event: {}", event_type))),
            }
            Ok(())
        })
    }

    fn send_command<'a>(
        &'a self,
        cmd: InternalTransactionCommand
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let cmd_tx = self.data.cmd_tx.clone();
        Box::pin(async move {
            cmd_tx.send(cmd).await
                .map_err(|e| Error::Other(format!("Failed to send command via TransactionAsync: {}", e)))
        })
    }

    fn original_request<'a>(
        &'a self
    ) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + 'a>> {
        let request_mutex = self.data.request.clone();
        Box::pin(async move {
            Some(request_mutex.lock().await.clone())
        })
    }

    fn last_response<'a>(
        &'a self
    ) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + 'a>> {
        let response_mutex = self.data.last_response.clone();
        Box::pin(async move {
            response_mutex.lock().await.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::runner::{AsRefState, AsRefKey, HasTransactionEvents, HasTransport, HasCommandSender}; // For ClientTransactionData
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder}; // Added SimpleResponseBuilder
    use rvoip_sip_core::types::status::StatusCode;
    use rvoip_sip_core::Response as SipCoreResponse;
    // use rvoip_sip_transport::TransportEvent as TransportLayerEvent; // This was unused
    use std::collections::VecDeque;
    use std::str::FromStr;
    use tokio::sync::Notify;
    use tokio::time::timeout as TokioTimeout;


    // A simple mock transport for these unit tests
    #[derive(Debug, Clone)]
    struct UnitTestMockTransport {
        sent_messages: Arc<Mutex<VecDeque<(Message, SocketAddr)>>>,
        local_addr: SocketAddr,
        // Notifier for when a message is sent, to help synchronize tests
        message_sent_notifier: Arc<Notify>,
    }

    impl UnitTestMockTransport {
        fn new(local_addr_str: &str) -> Self {
            Self {
                sent_messages: Arc::new(Mutex::new(VecDeque::new())),
                local_addr: SocketAddr::from_str(local_addr_str).unwrap(),
                message_sent_notifier: Arc::new(Notify::new()),
            }
        }

        async fn get_sent_message(&self) -> Option<(Message, SocketAddr)> {
            self.sent_messages.lock().await.pop_front()
        }

        // Return type changed to std::result::Result
        async fn wait_for_message_sent(&self, duration: Duration) -> std::result::Result<(), tokio::time::error::Elapsed> {
            TokioTimeout(duration, self.message_sent_notifier.notified()).await
        }
    }

    #[async_trait::async_trait]
    impl Transport for UnitTestMockTransport {
        // Return type changed to std::result::Result<_, rvoip_sip_transport::Error>
        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        // Return type changed
        async fn send_message(&self, message: Message, destination: SocketAddr) -> std::result::Result<(), rvoip_sip_transport::Error> {
            self.sent_messages.lock().await.push_back((message.clone(), destination));
            self.message_sent_notifier.notify_one(); // Notify that a message was "sent"
            Ok(())
        }

        // Return type changed
        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    struct TestSetup {
        transaction: ClientNonInviteTransaction,
        mock_transport: Arc<UnitTestMockTransport>,
        tu_events_rx: mpsc::Receiver<TransactionEvent>,
    }

    async fn setup_test_environment(
        request_method: Method,
        target_uri_str: &str, // Changed to target_uri_str
    ) -> TestSetup {
        let local_addr = "127.0.0.1:5090";
        let mock_transport = Arc::new(UnitTestMockTransport::new(local_addr));
        let (tu_events_tx, tu_events_rx) = mpsc::channel(100);

        let req_uri = Uri::from_str(target_uri_str).unwrap();
        let builder = SimpleRequestBuilder::new(request_method, &req_uri.to_string())
            .expect("Failed to create SimpleRequestBuilder")
            .from("Alice", "sip:test@test.com", Some("fromtag"))
            .to("Bob", "sip:bob@target.com", None)
            .call_id("callid-noninvite-test")
            .cseq(1); // Remove the method parameter
        
        let via_branch = format!("z9hG4bK.{}", uuid::Uuid::new_v4().as_simple());
        let builder = builder.via(mock_transport.local_addr.to_string().as_str(), "UDP", Some(&via_branch));

        let request = builder.build();
        
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        // Corrected TransactionKey::from_request call
        let tx_key = TransactionKey::from_request(&request).expect("Failed to create tx key from request");

        let settings = TimerSettings {
            t1: Duration::from_millis(50),
            transaction_timeout: Duration::from_millis(200),
            wait_time_k: Duration::from_millis(100),
            ..Default::default()
        };

        let transaction = ClientNonInviteTransaction::new(
            tx_key,
            request,
            remote_addr,
            mock_transport.clone() as Arc<dyn Transport>,
            tu_events_tx,
            Some(settings),
        ).unwrap();

        TestSetup {
            transaction,
            mock_transport,
            tu_events_rx,
        }
    }
    
    fn build_simple_response(status_code: StatusCode, original_request: &Request) -> SipCoreResponse {
        let response_builder = SimpleResponseBuilder::response_from_request(
            original_request,
            status_code,
            Some(status_code.reason_phrase())
        );
        
        let response_builder = if original_request.to().unwrap().tag().is_none() {
             response_builder.to(
                original_request.to().unwrap().address().display_name().unwrap_or_default(),
                &original_request.to().unwrap().address().uri().to_string(),
                Some("totag-server")
            )
        } else {
            response_builder
        };
        
        response_builder.build()
    }


    #[tokio::test]
    async fn test_non_invite_client_creation_and_initial_state() {
        let setup = setup_test_environment(Method::Options, "sip:bob@target.com").await;
        assert_eq!(setup.transaction.state(), TransactionState::Initial);
        assert!(setup.transaction.data.event_loop_handle.lock().await.is_some());
    }

    #[tokio::test]
    async fn test_non_invite_client_initiate_sends_request_and_starts_timers() {
        let mut setup = setup_test_environment(Method::Options, "sip:bob@target.com").await;
        
        setup.transaction.initiate().await.expect("initiate should succeed");

        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Message should be sent quickly");

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(setup.transaction.state(), TransactionState::Trying, "State should be Trying after initiate");

        let sent_msg_info = setup.mock_transport.get_sent_message().await;
        assert!(sent_msg_info.is_some(), "Request should have been sent");
        if let Some((msg, dest)) = sent_msg_info {
            assert!(msg.is_request());
            assert_eq!(msg.method(), Some(Method::Options));
            assert_eq!(dest, setup.transaction.remote_addr());
        }
        
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Timer E retransmission failed to occur");
        let retransmitted_msg_info = setup.mock_transport.get_sent_message().await;
        assert!(retransmitted_msg_info.is_some(), "Request should have been retransmitted by Timer E");
         if let Some((msg, _)) = retransmitted_msg_info {
            assert!(msg.is_request());
            assert_eq!(msg.method(), Some(Method::Options));
        }
    }

    #[tokio::test]
    async fn test_non_invite_client_provisional_response() {
        let mut setup = setup_test_environment(Method::Options, "sip:bob@target.com").await;
        setup.transaction.initiate().await.expect("initiate failed");
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.unwrap();
        setup.mock_transport.get_sent_message().await;

        // Wait for and ignore the StateChanged event
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { .. })) => {
                // Expected StateChanged event, continue
            },
            Ok(Some(other_event)) => panic!("Unexpected first event: {:?}", other_event),
            _ => panic!("Expected StateChanged event"),
        }

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(setup.transaction.state(), TransactionState::Trying);

        let original_request_clone = setup.transaction.data.request.lock().await.clone();
        let prov_response = build_simple_response(StatusCode::Ringing, &original_request_clone);
        
        setup.transaction.process_response(prov_response.clone()).await.expect("process_response failed");

        // Wait for ProvisionalResponse event
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::ProvisionalResponse { transaction_id, response, .. })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(response.status_code(), StatusCode::Ringing.as_u16());
            },
            Ok(Some(other_event)) => panic!("Unexpected event: {:?}", other_event),
            Ok(None) => panic!("Event channel closed"),
            Err(_) => panic!("Timeout waiting for ProvisionalResponse event"),
        }
        
        // Check for StateChanged from Trying to Proceeding
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { transaction_id, previous_state, new_state })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(previous_state, TransactionState::Trying);
                assert_eq!(new_state, TransactionState::Proceeding);
            },
            Ok(Some(other_event)) => panic!("Unexpected event: {:?}", other_event),
            _ => panic!("Expected StateChanged event"),
        }
        
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(setup.transaction.state(), TransactionState::Proceeding, "State should be Proceeding");

        // No need to check for immediate message, Timer E is no longer applicable in the Proceeding state
    }

    #[tokio::test]
    async fn test_non_invite_client_final_success_response() {
        let mut setup = setup_test_environment(Method::Options, "sip:bob@target.com").await;
        setup.transaction.initiate().await.expect("initiate failed");
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.unwrap();
        setup.mock_transport.get_sent_message().await;

        // Wait for and ignore the StateChanged event
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { .. })) => {
                // Expected StateChanged event, continue
            },
            Ok(Some(other_event)) => panic!("Unexpected first event: {:?}", other_event),
            _ => panic!("Expected StateChanged event"),
        }

        let original_request_clone = setup.transaction.data.request.lock().await.clone();
        let success_response = build_simple_response(StatusCode::Ok, &original_request_clone);
        
        setup.transaction.process_response(success_response.clone()).await.expect("process_response failed");

        // Success response can come before or after state change, so collect events and check them all
        let mut success_response_received = false;
        let mut trying_to_completed_received = false;
        let mut completed_to_terminated_received = false;
        let mut transaction_terminated_received = false;

        // Collect all events until we get terminated or timeout
        for _ in 0..5 {  // Give it 5 iterations max
            match TokioTimeout(Duration::from_millis(150), setup.tu_events_rx.recv()).await {
                Ok(Some(TransactionEvent::SuccessResponse { transaction_id, response, .. })) => {
                    assert_eq!(transaction_id, *setup.transaction.id());
                    assert_eq!(response.status_code(), StatusCode::Ok.as_u16());
                    success_response_received = true;
                },
                Ok(Some(TransactionEvent::StateChanged { transaction_id, previous_state, new_state })) => {
                    assert_eq!(transaction_id, *setup.transaction.id());
                    if previous_state == TransactionState::Trying && new_state == TransactionState::Completed {
                        trying_to_completed_received = true;
                    } else if previous_state == TransactionState::Completed && new_state == TransactionState::Terminated {
                        completed_to_terminated_received = true;
                    } else {
                        panic!("Unexpected state transition: {:?} -> {:?}", previous_state, new_state);
                    }
                },
                Ok(Some(TransactionEvent::TransactionTerminated { transaction_id, .. })) => {
                    assert_eq!(transaction_id, *setup.transaction.id());
                    transaction_terminated_received = true;
                    break;  // We got the terminal event, can stop waiting
                },
                Ok(Some(TransactionEvent::TimerTriggered { .. })) => {
                    // Timer events can happen, ignore them
                    continue;
                },
                Ok(Some(other_event)) => panic!("Unexpected event: {:?}", other_event),
                Ok(None) => panic!("Event channel closed"),
                Err(_) => {
                    // If we timed out but already got the necessary events, we're good
                    if success_response_received && trying_to_completed_received && 
                       (completed_to_terminated_received || transaction_terminated_received) {
                        break;
                    } else {
                        // Otherwise, keep waiting
                        continue;
                    }
                }
            }
            
            // If we got all the necessary events, we can stop waiting
            if success_response_received && trying_to_completed_received && 
               completed_to_terminated_received && transaction_terminated_received {
                break;
            }
        }

        // Check that we got all the expected events
        assert!(success_response_received, "SuccessResponse event not received");
        assert!(trying_to_completed_received, "StateChanged Trying->Completed event not received");
        
        // The transaction should reach Terminated state
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(setup.transaction.state(), TransactionState::Terminated, "State should be Terminated after Timer K");
    }
    
    #[tokio::test]
    async fn test_non_invite_client_timer_f_timeout() {
        let mut setup = setup_test_environment(Method::Options, "sip:bob@target.com").await;
        setup.transaction.initiate().await.expect("initiate failed");

        // Wait for and ignore the StateChanged event
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { .. })) => {
                // Expected StateChanged event, continue
            },
            Ok(Some(other_event)) => panic!("Unexpected first event: {:?}", other_event),
            _ => panic!("Expected StateChanged event"),
        }

        let mut timeout_event_received = false;
        let mut terminated_event_received = false;
        let mut timer_f_received = false;

        // Loop to catch multiple events, specifically TimerTriggered for E/F, then TransactionTimeout, then TransactionTerminated.
        // Increased loop count and timeout to be more robust for E retransmissions before F.
        for _ in 0..6 { 
            match TokioTimeout(Duration::from_millis(150), setup.tu_events_rx.recv()).await { // Increased timeout per event
                Ok(Some(TransactionEvent::TransactionTimeout { transaction_id, .. })) => {
                    assert_eq!(transaction_id, *setup.transaction.id());
                    timeout_event_received = true;
                },
                Ok(Some(TransactionEvent::TransactionTerminated { transaction_id, .. })) => {
                    assert_eq!(transaction_id, *setup.transaction.id());
                    terminated_event_received = true;
                },
                Ok(Some(TransactionEvent::TimerTriggered { ref timer, .. })) => { // Used ref timer
                    if timer == "E" { 
                        debug!("Timer E triggered during F timeout test, continuing...");
                        continue; 
                    } else if timer == "F" {
                        timer_f_received = true;
                        continue;
                    }
                    panic!("Unexpected TimerTriggered event: {:?}", timer);
                },
                Ok(Some(TransactionEvent::StateChanged { .. })) => {
                    // State transitions can happen, ignore them
                    continue;
                },
                Ok(Some(other_event)) => {
                    panic!("Unexpected event: {:?}", other_event);
                },
                Ok(None) => panic!("Event channel closed prematurely"),
                Err(_) => { // Timeout from TokioTimeout
                    // This timeout is for a single recv() call. If we haven't gotten both target events, continue test waiting.
                    if !timeout_event_received || !terminated_event_received {
                        debug!("TokioTimeout while waiting for F events, may be normal if timers are still running");
                        // Continue to next iteration of the loop if not all events are received.
                    } else {
                        break; // Both events received, or one timed out after the other was received.
                    }
                }
            }
            if timeout_event_received && terminated_event_received { break; }
        }
        
        assert!(timeout_event_received, "TransactionTimeout event not received");
        assert!(terminated_event_received, "TransactionTerminated event not received");
        
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(setup.transaction.state(), TransactionState::Terminated, "State should be Terminated after Timer F");
    }
} 