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
use rvoip_sip_core::types::MaxForwards;
use rvoip_sip_core::types::ContentLength;
use rvoip_sip_core::types::via::ViaHeader;
use rvoip_sip_core::types::param::Param;
use rvoip_sip_core::types::CSeq;
use rvoip_sip_transport::Transport;

use crate::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionAsync, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand, AtomicTransactionState
};
use crate::timer::{TimerSettings, TimerFactory, TimerManager, TimerType};
use crate::client::{ClientTransaction, ClientTransactionData, CommonClientTransaction};
use crate::utils;
use crate::transaction::logic::TransactionLogic;
use crate::transaction::runner::{run_transaction_loop, HasCommandSender, AsRefKey};
// Add imports for utility modules
use crate::transaction::timer_utils;
use crate::transaction::validators;
use crate::transaction::common_logic;

/// Client CANCEL transaction (RFC 3261 Section 9.1)
/// CANCEL is a non-INVITE transaction that is used to cancel an INVITE transaction
/// It follows non-INVITE transaction state machine but has special rules for construction
#[derive(Debug, Clone)]
pub struct ClientCancelTransaction {
    data: Arc<ClientTransactionData>,
    logic: Arc<ClientCancelLogic>,
    /// Reference to the original INVITE transaction that this CANCEL is cancelling
    original_invite_tx_id: Option<TransactionKey>,
}

/// Holds JoinHandles and dynamic state for timers specific to Client CANCEL transactions.
/// CANCEL uses the same timer model as non-INVITE transactions (E, F, K)
#[derive(Default, Debug)]
struct ClientCancelTimerHandles {
    timer_e: Option<JoinHandle<()>>,
    current_timer_e_interval: Option<Duration>, // For backoff
    timer_f: Option<JoinHandle<()>>,
    timer_k: Option<JoinHandle<()>>,
}

/// Implements the TransactionLogic for Client CANCEL transactions.
#[derive(Debug, Clone, Default)]
struct ClientCancelLogic {
    _data_marker: std::marker::PhantomData<ClientTransactionData>,
    timer_factory: TimerFactory,
}

impl ClientCancelLogic {
    // Helper method to start Timer E (retransmission timer) using timer utils
    async fn start_timer_e(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientCancelTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        // Start Timer E (retransmission) with initial interval T1
        let initial_interval_e = timer_handles.current_timer_e_interval.unwrap_or(timer_config.t1);
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
    
    // Helper method to start Timer F (transaction timeout) using timer utils
    async fn start_timer_f(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientCancelTimerHandles,
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
    
    // Helper method to start Timer K (wait for response retransmissions) using timer utils with transition
    async fn start_timer_k(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientCancelTimerHandles,
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
    
    // Helper method to handle initial request sending in Trying state
    async fn handle_trying_state(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientCancelTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<()> {
        let tx_id = &data.id;
        
        // Send the initial CANCEL request
        debug!(id=%tx_id, "ClientCancelLogic: Sending initial CANCEL request in Trying state");
        let request_guard = data.request.lock().await;
        if let Err(e) = data.transport.send_message(
            Message::Request(request_guard.clone()),
            data.remote_addr
        ).await {
            error!(id=%tx_id, error=%e, "Failed to send initial CANCEL request from Trying state");
            common_logic::send_transport_error_event(tx_id, &data.events_tx).await;
            // If send fails, command a transition to Terminated
            let _ = command_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
            return Err(Error::transport_error(e, "Failed to send initial CANCEL request"));
        }
        drop(request_guard); // Release lock

        // Start timers for Trying state
        self.start_timer_e(data, timer_handles, command_tx.clone()).await;
        self.start_timer_f(data, timer_handles, command_tx).await;
        
        Ok(())
    }

    // Handle Timer E (retransmission) trigger
    async fn handle_timer_e_trigger(
        &self,
        data: &Arc<ClientTransactionData>,
        current_state: TransactionState,
        timer_handles: &mut ClientCancelTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        match current_state {
            TransactionState::Trying | TransactionState::Proceeding => {
                debug!(id=%tx_id, "Timer E triggered, retransmitting CANCEL request");
                
                // Retransmit the CANCEL request
                let request_guard = data.request.lock().await;
                if let Err(e) = data.transport.send_message(
                    Message::Request(request_guard.clone()),
                    data.remote_addr
                ).await {
                    error!(id=%tx_id, error=%e, "Failed to retransmit CANCEL request");
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
    
    // Handle Timer F (transaction timeout) trigger
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
    
    // Handle Timer K (wait for retransmissions) trigger
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
    
    // Process a SIP response
    async fn process_response(
        &self,
        data: &Arc<ClientTransactionData>,
        response: Response,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        // Get the original method from the request to validate the response
        let request_guard = data.request.lock().await;
        let original_method = validators::get_method_from_request(&request_guard);
        drop(request_guard);
        
        // Validate that the response matches our transaction
        if let Err(e) = validators::validate_response_matches_transaction(&response, tx_id, &original_method) {
            warn!(id=%tx_id, error=%e, "Response validation failed");
            return Ok(None);
        }
        
        // For CANCEL, handle responses according to RFC 3261 Section 9.1
        // RFC states: "If the original request was an INVITE, the CANCEL request cannot affect the processing 
        // of the INVITE until an INVITE response has been received."
        
        // Get status information
        let (is_provisional, is_success, is_failure) = validators::categorize_response_status(&response);
        
        // Use the common logic handler - CANCEL follows non-INVITE transaction rules
        let new_state = common_logic::handle_response_by_status(
            tx_id, 
            response.clone(), 
            current_state, 
            &data.events_tx,
            false // CANCEL is non-INVITE for state transitions
        ).await;
        
        Ok(new_state)
    }
}

#[async_trait::async_trait]
impl TransactionLogic<ClientTransactionData, ClientCancelTimerHandles> for ClientCancelLogic {
    fn kind(&self) -> TransactionKind {
        TransactionKind::CancelClient
    }

    fn initial_state(&self) -> TransactionState {
        TransactionState::Initial
    }

    fn timer_settings<'a>(data: &'a Arc<ClientTransactionData>) -> &'a TimerSettings {
        &data.timer_config
    }

    fn cancel_all_specific_timers(&self, timer_handles: &mut ClientCancelTimerHandles) {
        if let Some(handle) = timer_handles.timer_e.take() {
            handle.abort();
        }
        if let Some(handle) = timer_handles.timer_f.take() {
            handle.abort();
        }
        if let Some(handle) = timer_handles.timer_k.take() {
            handle.abort();
        }
        timer_handles.current_timer_e_interval = None;
    }

    async fn on_enter_state(
        &self,
        data: &Arc<ClientTransactionData>,
        new_state: TransactionState,
        previous_state: TransactionState,
        timer_handles: &mut ClientCancelTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>, 
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

    async fn handle_timer(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_name: &str,
        current_state: TransactionState,
        timer_handles: &mut ClientCancelTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        if timer_name == "E" {
            // Clear the timer handle since it fired
            timer_handles.timer_e.take();
        }
        
        // Send timer triggered event using common logic
        common_logic::send_timer_triggered_event(tx_id, timer_name, &data.events_tx).await;
        
        // Use the command_tx from data to set up timers
        let self_command_tx = data.cmd_tx.clone();
        
        match timer_name {
            "E" => self.handle_timer_e_trigger(data, current_state, timer_handles, self_command_tx).await,
            "F" => self.handle_timer_f_trigger(data, current_state, self_command_tx).await,
            "K" => self.handle_timer_k_trigger(data, current_state, self_command_tx).await,
            _ => {
                warn!(id=%tx_id, timer_name=%timer_name, "Unknown timer triggered for ClientCancel");
                Ok(None)
            }
        }
    }

    async fn process_message(
        &self,
        data: &Arc<ClientTransactionData>,
        message: Message,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        // Use the validators utility to extract and validate the response
        match validators::extract_response(&message, tx_id) {
            Ok(response) => {
                // Store the response
                {
                    let mut last_response = data.last_response.lock().await;
                    *last_response = Some(response.clone());
                }
                
                // Process the response
                self.process_response(data, response, current_state).await
            },
            Err(e) => {
                warn!(id=%tx_id, error=%e, "Received non-response message");
                Ok(None)
            }
        }
    }
}

impl ClientCancelTransaction {
    /// Create a new client CANCEL transaction for an existing INVITE transaction.
    /// 
    /// # Arguments
    /// * `invite_request` - The original INVITE request being cancelled
    /// * `invite_tx_id` - The transaction ID of the INVITE transaction
    /// * `remote_addr` - The remote address to send the CANCEL to
    /// * `transport` - The transport to use for sending
    /// * `events_tx` - Event channel to the transaction user
    /// * `timer_config_override` - Optional custom timer settings
    pub fn new(
        invite_request: Request,
        invite_tx_id: TransactionKey,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config_override: Option<TimerSettings>,
    ) -> Result<Self> {
        if invite_request.method() != Method::Invite {
            return Err(Error::Other("Cannot create CANCEL for non-INVITE request".to_string()));
        }

        // Create CANCEL request from INVITE
        let cancel_request = Self::create_cancel_from_invite(&invite_request)?;
        
        // Generate a transaction key for this CANCEL
        let cancel_tx_id = TransactionKey::from_request(&cancel_request)
            .ok_or_else(|| Error::Other("Failed to create transaction key from CANCEL request".to_string()))?;

        let timer_config = timer_config_override.unwrap_or_default();
        let (cmd_tx, local_cmd_rx) = mpsc::channel(32);
        
        let data = Arc::new(ClientTransactionData {
            id: cancel_tx_id.clone(),
            state: Arc::new(AtomicTransactionState::new(TransactionState::Initial)),
            request: Arc::new(Mutex::new(cancel_request.clone())),
            last_response: Arc::new(Mutex::new(None)),
            remote_addr,
            transport,
            events_tx,
            cmd_tx: cmd_tx.clone(), // For the transaction itself to send commands to its loop
            event_loop_handle: Arc::new(Mutex::new(None)),
            timer_config: timer_config.clone(),
        });

        let logic = Arc::new(ClientCancelLogic {
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
        
        Ok(Self { 
            data, 
            logic,
            original_invite_tx_id: Some(invite_tx_id),
        })
    }

    /// Helper method to create a CANCEL request from an INVITE request
    pub fn create_cancel_from_invite(invite: &Request) -> Result<Request> {
        if invite.method() != Method::Invite {
            return Err(Error::Other("Cannot create CANCEL for non-INVITE request".to_string()));
        }

        // According to RFC 3261 Section 9.1:
        // The following fields in the CANCEL request MUST be identical to those in the
        // request being cancelled: Call-ID, To, From, and CSeq-num (not method).
        // Max-Forwards might be different and SHOULD be 70.
        // Route, Request-URI are the same.
        // CANCEL gets its own Via branch parameter, however.

        // Create a new request with the same URI as the original
        let mut cancel_request = Request::new(Method::Cancel, invite.uri().clone());
        
        // Copy headers one by one
        if let Some(header) = invite.header(&HeaderName::CallId) {
            cancel_request = cancel_request.with_header(header.clone());
        }
        
        if let Some(header) = invite.header(&HeaderName::From) {
            cancel_request = cancel_request.with_header(header.clone());
        }
        
        if let Some(header) = invite.header(&HeaderName::To) {
            cancel_request = cancel_request.with_header(header.clone());
        }
        
        if let Some(header) = invite.header(&HeaderName::Route) {
            cancel_request = cancel_request.with_header(header.clone());
        }
        
        // Update CSeq by extracting number from original and using CANCEL method
        if let Some(TypedHeader::CSeq(cseq)) = invite.header(&HeaderName::CSeq) {
            let sequence_num = cseq.sequence();
            cancel_request = cancel_request.with_header(TypedHeader::CSeq(CSeq::new(sequence_num, Method::Cancel)));
        }
        
        // Set Max-Forwards to 70
        cancel_request = cancel_request.with_header(TypedHeader::MaxForwards(MaxForwards::new(70)));
        
        // Generate a new Via header with a fresh branch parameter
        let branch = format!("z9hG4bK.{}", uuid::Uuid::new_v4().as_simple());
        if let Some(TypedHeader::Via(via_vec)) = invite.header(&HeaderName::Via) {
            if !via_vec.0.is_empty() {
                // Clone the via vector
                let mut new_via_vec = via_vec.clone();
                if let Some(first_via) = new_via_vec.0.get_mut(0) {
                    // Create a new Via with updated branch parameter
                    // First, copy all params except the branch
                    let mut new_params = Vec::new();
                    
                    // Copy all params except branch
                    for param in &first_via.params {
                        if !matches!(param, &Param::Branch(_)) {
                            new_params.push(param.clone());
                        }
                    }
                    
                    // Add our new branch param
                    new_params.push(Param::branch(branch));
                    
                    // Update the via params
                    first_via.params = new_params;
                }
                cancel_request = cancel_request.with_header(TypedHeader::Via(new_via_vec));
            }
        }
        
        // Set content length to 0
        cancel_request = cancel_request.with_header(TypedHeader::ContentLength(ContentLength::new(0)));
        
        Ok(cancel_request)
    }

    /// Get the transaction ID of the original INVITE transaction
    pub fn original_invite_tx_id(&self) -> Option<&TransactionKey> {
        self.original_invite_tx_id.as_ref()
    }
}

impl ClientTransaction for ClientCancelTransaction {
    fn initiate(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        let kind = self.kind();
        
        Box::pin(async move {
            let current_state = data.state.get();
            
            if current_state != TransactionState::Initial {
                return Err(Error::invalid_state_transition(
                    kind,
                    current_state,
                    TransactionState::Trying,
                    Some(data.id.clone()),
                ));
            }

            data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Trying)).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))?;
            Ok(())
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

    // Implement the original_request method
    fn original_request(&self) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + '_>> {
        let request_arc = self.data.request.clone();
        Box::pin(async move {
            let req = request_arc.lock().await;
            Some(req.clone()) // Clone the request out of the Mutex guard
        })
    }
}

impl Transaction for ClientCancelTransaction {
    fn id(&self) -> &TransactionKey {
        &self.data.id
    }

    fn kind(&self) -> TransactionKind {
        TransactionKind::CancelClient
    }

    fn state(&self) -> TransactionState {
        self.data.state.get()
    }
    
    fn remote_addr(&self) -> SocketAddr {
        self.data.remote_addr
    }
    
    fn matches(&self, message: &Message) -> bool {
        // Follow the same approach as in non_invite.rs, checking:
        // 1. Top Via branch parameter matches transaction ID's branch
        // 2. CSeq method matches original request's method (CANCEL)
        if !message.is_response() { return false; }
        
        let response = match message {
            Message::Response(r) => r,
            _ => return false,
        };

        // Check Via headers
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

        // Check CSeq method
        if let Some(TypedHeader::CSeq(cseq_header)) = response.header(&HeaderName::CSeq) {
            if cseq_header.method != Method::Cancel {
                return false;
            }
        } else {
            return false; // No CSeq header or not of TypedHeader::CSeq type
        }
        
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl TransactionAsync for ClientCancelTransaction {
    fn process_event<'a>(
        &'a self,
        event_type: &'a str, // e.g. "response" from TransactionManager when it routes a message
        message: Option<Message>
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event_type {
                "response" => {
                    if let Some(msg) = message {
                        self.data.cmd_tx.send(InternalTransactionCommand::ProcessMessage(msg)).await
                            .map_err(|e| Error::Other(format!("Failed to send ProcessMessage command: {}", e)))?;
                    } else {
                        return Err(Error::Other("Expected Message for 'response' event type".to_string()));
                    }
                },
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

impl CommonClientTransaction for ClientCancelTransaction {
    fn data(&self) -> &Arc<ClientTransactionData> {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::str::FromStr;
    use tokio::sync::Notify;
    use tokio::time::{timeout, Duration};
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use rvoip_sip_core::types::status::StatusCode;
    use crate::client::ClientTransaction;

    // Mock Transport for testing
    #[derive(Debug)]
    struct UnitTestMockTransport {
        sent_messages: Arc<Mutex<VecDeque<(Message, SocketAddr)>>>,
        local_addr: SocketAddr,
        // Notifier for when a message is sent
        message_sent_notifier: Arc<Notify>,
    }

    impl UnitTestMockTransport {
        fn new(local_addr_str: &str) -> Self {
            UnitTestMockTransport {
                sent_messages: Arc::new(Mutex::new(VecDeque::new())),
                local_addr: SocketAddr::from_str(local_addr_str).unwrap_or_else(|_| {
                    SocketAddr::from_str("127.0.0.1:5060").unwrap()
                }),
                message_sent_notifier: Arc::new(Notify::new()),
            }
        }

        async fn get_sent_message(&self) -> Option<(Message, SocketAddr)> {
            let mut queue = self.sent_messages.lock().await;
            queue.pop_front()
        }

        async fn wait_for_message_sent(&self, duration: Duration) -> std::result::Result<(), tokio::time::error::Elapsed> {
            timeout(duration, self.message_sent_notifier.notified()).await
        }
    }

    #[async_trait::async_trait]
    impl Transport for UnitTestMockTransport {
        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        async fn send_message(&self, message: Message, destination: SocketAddr) -> std::result::Result<(), rvoip_sip_transport::Error> {
            let mut queue = self.sent_messages.lock().await;
            queue.push_back((message, destination));
            self.message_sent_notifier.notify_one();
            Ok(())
        }

        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    // Helper to build a test INVITE request
    fn build_invite_request(target_uri_str: &str) -> Request {
        // First create the builder
        let builder_result = SimpleRequestBuilder::new(Method::Invite, target_uri_str);
        let builder = match builder_result {
            Ok(builder) => builder,
            Err(e) => panic!("Failed to create request builder: {}", e),
        };
        
        // Then build the request with all required headers
        builder
            .from("Alice", "sip:test@test.com", Some("fromtag"))
            .to("Bob", "sip:bob@target.com", None)
            .call_id("test-call-id")
            .cseq(101)
            .via("SIP/2.0/UDP", "test.example.com", Some("z9hG4bK.originalbranchvalue"))
            .content_type("application/sdp")
            .body(b"v=0\r\no=- 1234 1234 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\n".to_vec())
            .build()
    }

    // Helper to build a response to a CANCEL request
    fn build_simple_response(status_code: StatusCode, original_request: &Request) -> Response {
        // Create a response builder using the original request
        SimpleResponseBuilder::response_from_request(
            original_request,
            status_code,
            Some(status_code.reason_phrase())
        ).build()
    }

    // Test environment setup
    struct TestSetup {
        transaction: ClientCancelTransaction,
        mock_transport: Arc<UnitTestMockTransport>,
        tu_events_rx: mpsc::Receiver<TransactionEvent>,
        // Original INVITE transaction ID
        invite_tx_id: TransactionKey,
    }

    // Helper to create test environment
    async fn setup_test_environment(target_uri_str: &str) -> TestSetup {
        let invite_request = build_invite_request(target_uri_str);
        
        // Create a transaction ID for the original INVITE
        let invite_tx_id = TransactionKey::from_request(&invite_request)
            .expect("Failed to create transaction key from INVITE request");
            
        let mock_transport = Arc::new(UnitTestMockTransport::new("127.0.0.1:5060"));
        let (tu_events_tx, tu_events_rx) = mpsc::channel(32);
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        
        // Create the CANCEL transaction
        let transaction = ClientCancelTransaction::new(
            invite_request,
            invite_tx_id.clone(),
            remote_addr,
            mock_transport.clone(),
            tu_events_tx,
            None,
        ).expect("Failed to create CANCEL transaction");
        
        TestSetup {
            transaction,
            mock_transport,
            tu_events_rx,
            invite_tx_id,
        }
    }

    #[tokio::test]
    async fn test_cancel_client_creation() {
        let setup = setup_test_environment("sip:bob@target.com").await;
        
        // Check initial state
        assert_eq!(setup.transaction.state(), TransactionState::Initial);
        
        // Verify transaction kind
        assert_eq!(setup.transaction.kind(), TransactionKind::CancelClient);
        
        // Verify the original INVITE transaction ID is stored
        assert!(setup.transaction.original_invite_tx_id().is_some());
        assert_eq!(setup.transaction.original_invite_tx_id().unwrap(), &setup.invite_tx_id);
        
        // Request should be available and be a CANCEL method
        let req = ClientTransaction::original_request(&setup.transaction).await.expect("Request should be available");
        assert_eq!(req.method(), Method::Cancel);
        
        // Verify key CANCEL request fields
        if let Some(TypedHeader::CSeq(cseq)) = req.header(&HeaderName::CSeq) {
            assert_eq!(cseq.method, Method::Cancel);
            assert_eq!(cseq.sequence(), 101); // Same as original INVITE
        } else {
            panic!("CSeq header not found in CANCEL request");
        }
        
        // Verify other headers carried from INVITE
        assert_eq!(req.header(&HeaderName::CallId), build_invite_request("sip:bob@target.com").header(&HeaderName::CallId));
        assert_eq!(req.header(&HeaderName::From), build_invite_request("sip:bob@target.com").header(&HeaderName::From));
        assert_eq!(req.header(&HeaderName::To), build_invite_request("sip:bob@target.com").header(&HeaderName::To));
        
        // Verify ContentLength is 0 and body is empty
        if let Some(TypedHeader::ContentLength(len)) = req.header(&HeaderName::ContentLength) {
            // Check that ContentLength is 0
            assert_eq!(len.0, 0);
        } else {
            panic!("ContentLength header not found in CANCEL request");
        }
        assert!(req.body().is_empty());
    }

    #[tokio::test]
    async fn test_cancel_client_initiate() {
        let mut setup = setup_test_environment("sip:bob@target.com").await;
        
        // Initiate the transaction
        setup.transaction.initiate().await.expect("Failed to initiate transaction");
        
        // Wait briefly for the message to be sent
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Timed out waiting for message");
        
        // Check that state transitioned to Trying
        assert_eq!(setup.transaction.state(), TransactionState::Trying);
        
        // Verify a message was sent
        let sent_message = setup.mock_transport.get_sent_message().await.expect("No message was sent");
        
        // Verify it's a CANCEL request
        match sent_message.0 {
            Message::Request(req) => {
                assert_eq!(req.method(), Method::Cancel);
            },
            _ => panic!("Expected Request message"),
        }
        
        // Verify transaction events
        let event = setup.tu_events_rx.recv().await.expect("No event received");
        match event {
            TransactionEvent::StateChanged { previous_state, new_state, .. } => {
                assert_eq!(previous_state, TransactionState::Initial);
                assert_eq!(new_state, TransactionState::Trying);
            },
            _ => panic!("Expected StateChanged event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_cancel_client_provisional_response() {
        let mut setup = setup_test_environment("sip:bob@target.com").await;
        
        // Initiate the transaction
        setup.transaction.initiate().await.expect("Failed to initiate transaction");
        
        // Wait for the transaction to send the request
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Timed out waiting for message");
        let sent_message = setup.mock_transport.get_sent_message().await.expect("No message was sent");
        
        // Extract the request to build a response
        let request = match sent_message.0 {
            Message::Request(req) => req,
            _ => panic!("Expected Request message"),
        };
        
        // Create and process a 100 Trying response
        let response = build_simple_response(StatusCode::Trying, &request);
        setup.transaction.process_response(response).await.expect("Failed to process response");
        
        // Wait for the state to change
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Verify state changed to Proceeding
        assert_eq!(setup.transaction.state(), TransactionState::Proceeding);
        
        // Verify transaction events
        // First should be Initial -> Trying
        let event = setup.tu_events_rx.recv().await.expect("No event received");
        match event {
            TransactionEvent::StateChanged { previous_state, new_state, .. } => {
                assert_eq!(previous_state, TransactionState::Initial);
                assert_eq!(new_state, TransactionState::Trying);
            },
            _ => panic!("Expected StateChanged event, got {:?}", event),
        }
        
        // Then should be a ProvisionalResponse event
        let event = setup.tu_events_rx.recv().await.expect("No event received");
        match event {
            TransactionEvent::ProvisionalResponse { response, .. } => {
                assert_eq!(response.status(), StatusCode::Trying);
            },
            _ => panic!("Expected ProvisionalResponse event, got {:?}", event),
        }
        
        // Followed by Trying -> Proceeding state change
        let event = setup.tu_events_rx.recv().await.expect("No event received");
        match event {
            TransactionEvent::StateChanged { previous_state, new_state, .. } => {
                assert_eq!(previous_state, TransactionState::Trying);
                assert_eq!(new_state, TransactionState::Proceeding);
            },
            _ => panic!("Expected StateChanged event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_cancel_client_success_response() {
        // Create custom timer settings for testing with a very short wait_time_k
        let timer_settings = TimerSettings {
            t1: Duration::from_millis(10),
            t2: Duration::from_millis(40),
            transaction_timeout: Duration::from_millis(500),
            wait_time_k: Duration::from_millis(50), // Very short Timer K to test termination
            wait_time_d: Duration::from_millis(100),
            ..Default::default()
        };
        
        // Create test request and transaction directly instead of using setup
        let invite_request = build_invite_request("sip:bob@target.com");
        let invite_tx_id = TransactionKey::from_request(&invite_request)
            .expect("Failed to create transaction key from INVITE request");
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        let mock_transport = Arc::new(UnitTestMockTransport::new("127.0.0.1:5060"));
        
        let (tu_events_tx, mut tu_events_rx) = mpsc::channel(32);
        let transaction = ClientCancelTransaction::new(
            invite_request,
            invite_tx_id,
            remote_addr,
            mock_transport.clone(),
            tu_events_tx,
            Some(timer_settings),
        ).expect("Failed to create CANCEL transaction");
        
        // Initiate the transaction
        transaction.initiate().await.expect("Failed to initiate transaction");
        
        // Wait for the transaction to send the request
        mock_transport.wait_for_message_sent(Duration::from_millis(100)).await
            .expect("Timed out waiting for message");
        let sent_message = mock_transport.get_sent_message().await
            .expect("No message was sent");
        
        // Extract the request to build a response
        let request = match sent_message.0 {
            Message::Request(req) => req,
            _ => panic!("Expected Request message"),
        };
        
        // Create and process a 200 OK response
        let response = build_simple_response(StatusCode::Ok, &request);
        transaction.process_response(response).await.expect("Failed to process response");
        
        // Verify state changed to Completed
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(transaction.state(), TransactionState::Completed);
        
        // Verify we received the expected events
        let mut got_initial_to_trying = false;
        let mut got_success_response = false;
        let mut got_trying_to_completed = false;
        
        // Process events until we run out or see all expected ones
        for _ in 0..10 {  // Process at most 10 events to avoid infinite loops
            match tu_events_rx.try_recv() {
                Ok(event) => {
                    match event {
                        TransactionEvent::StateChanged { previous_state, new_state, .. } => {
                            if previous_state == TransactionState::Initial && new_state == TransactionState::Trying {
                                got_initial_to_trying = true;
                            } else if previous_state == TransactionState::Trying && new_state == TransactionState::Completed {
                                got_trying_to_completed = true;
                            }
                        },
                        TransactionEvent::SuccessResponse { .. } => {
                            got_success_response = true;
                        },
                        _ => {}
                    }
                },
                Err(_) => break,  // No more events
            }
        }
        
        assert!(got_initial_to_trying, "Expected Initial->Trying state change event");
        assert!(got_success_response, "Expected SuccessResponse event");
        assert!(got_trying_to_completed, "Expected Trying->Completed state change event");
        
        // Wait for Timer K to fire and transition to Terminated state
        // This should take around 50ms but we'll give it up to 500ms
        let mut terminated = false;
        for i in 0..10 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if transaction.state() == TransactionState::Terminated {
                terminated = true;
                break;
            }
            // Print debug info if we're waiting for a while
            if i > 5 {
                debug!("Still waiting for transaction to reach Terminated state, current state: {:?}", transaction.state());
            }
        }
        
        assert!(terminated, "Transaction did not reach Terminated state after Timer K");
        
        // Check for the state change to Terminated event
        let mut got_completed_to_terminated = false;
        for _ in 0..5 {
            match tu_events_rx.try_recv() {
                Ok(event) => {
                    if let TransactionEvent::StateChanged { previous_state, new_state, .. } = event {
                        if previous_state == TransactionState::Completed && new_state == TransactionState::Terminated {
                            got_completed_to_terminated = true;
                            break;
                        }
                    }
                },
                Err(_) => break,
            }
        }
        
        assert!(got_completed_to_terminated, "Did not receive Completed->Terminated state change event");
    }

    #[tokio::test]
    async fn test_cancel_client_failure_response() {
        let mut setup = setup_test_environment("sip:bob@target.com").await;
        
        // Initiate the transaction
        setup.transaction.initiate().await.expect("Failed to initiate transaction");
        
        // Wait for the transaction to send the request
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Timed out waiting for message");
        let sent_message = setup.mock_transport.get_sent_message().await.expect("No message was sent");
        
        // Extract the request to build a response
        let request = match sent_message.0 {
            Message::Request(req) => req,
            _ => panic!("Expected Request message"),
        };
        
        // Create and process a 481 Call/Transaction Does Not Exist
        let response = build_simple_response(StatusCode::CallOrTransactionDoesNotExist, &request);
        setup.transaction.process_response(response).await.expect("Failed to process response");
        
        // Wait for the state to change
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Verify state changed to Completed
        assert_eq!(setup.transaction.state(), TransactionState::Completed);
        
        // Verify transaction events
        // First should be Initial -> Trying
        let event = setup.tu_events_rx.recv().await.expect("No event received");
        match event {
            TransactionEvent::StateChanged { previous_state, new_state, .. } => {
                assert_eq!(previous_state, TransactionState::Initial);
                assert_eq!(new_state, TransactionState::Trying);
            },
            _ => panic!("Expected StateChanged event, got {:?}", event),
        }
        
        // Then should be a FailureResponse event
        let event = setup.tu_events_rx.recv().await.expect("No event received");
        match event {
            TransactionEvent::FailureResponse { response, .. } => {
                assert_eq!(response.status(), StatusCode::CallOrTransactionDoesNotExist);
            },
            _ => panic!("Expected FailureResponse event, got {:?}", event),
        }
        
        // Followed by Trying -> Completed state change
        let event = setup.tu_events_rx.recv().await.expect("No event received");
        match event {
            TransactionEvent::StateChanged { previous_state, new_state, .. } => {
                assert_eq!(previous_state, TransactionState::Trying);
                assert_eq!(new_state, TransactionState::Completed);
            },
            _ => panic!("Expected StateChanged event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_cancel_client_timer_f_timeout() {
        let mut setup = setup_test_environment("sip:bob@target.com").await;
        
        // Override the timer settings to use very short timers for testing
        let timer_settings = TimerSettings {
            t1: Duration::from_millis(10),
            t2: Duration::from_millis(40),
            transaction_timeout: Duration::from_millis(50), // Very short Timer F
            wait_time_k: Duration::from_millis(100),
            wait_time_d: Duration::from_millis(100),
            ..Default::default()
        };
        
        // Create a new transaction with short timers
        let invite_request = build_invite_request("sip:bob@target.com");
        let invite_tx_id = TransactionKey::from_request(&invite_request)
            .expect("Failed to create transaction key from INVITE request");
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        
        let (tu_events_tx, mut tu_events_rx) = mpsc::channel(32);
        let transaction = ClientCancelTransaction::new(
            invite_request,
            invite_tx_id,
            remote_addr,
            setup.mock_transport.clone(),
            tu_events_tx,
            Some(timer_settings),
        ).expect("Failed to create CANCEL transaction");
        
        // Initiate the transaction
        transaction.initiate().await.expect("Failed to initiate transaction");
        
        // Wait for Timer F to fire (transaction timeout)
        tokio::time::sleep(Duration::from_millis(200)).await;
        
        // Verify state is Terminated after timeout
        assert_eq!(transaction.state(), TransactionState::Terminated);
        
        // Verify transaction events including timeout event
        let mut saw_timeout_event = false;
        while let Ok(event) = tu_events_rx.try_recv() {
            match event {
                TransactionEvent::TransactionTimeout { .. } => {
                    saw_timeout_event = true;
                },
                _ => {}
            }
        }
        
        assert!(saw_timeout_event, "Should have received a TransactionTimeout event");
    }

    #[tokio::test]
    async fn test_cancel_client_retransmission() {
        let mut setup = setup_test_environment("sip:bob@target.com").await;
        
        // Override the timer settings to use very short timers for testing
        let timer_settings = TimerSettings {
            t1: Duration::from_millis(10),
            t2: Duration::from_millis(40),
            transaction_timeout: Duration::from_millis(500),
            wait_time_k: Duration::from_millis(100),
            wait_time_d: Duration::from_millis(100),
            ..Default::default()
        };
        
        // Create a new transaction with short timers
        let invite_request = build_invite_request("sip:bob@target.com");
        let invite_tx_id = TransactionKey::from_request(&invite_request)
            .expect("Failed to create transaction key from INVITE request");
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        
        let (tu_events_tx, tu_events_rx) = mpsc::channel(32);
        let transaction = ClientCancelTransaction::new(
            invite_request,
            invite_tx_id,
            remote_addr,
            setup.mock_transport.clone(),
            tu_events_tx,
            Some(timer_settings),
        ).expect("Failed to create CANCEL transaction");
        
        // Initiate the transaction
        transaction.initiate().await.expect("Failed to initiate transaction");
        
        // Wait for initial message
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Timed out waiting for initial message");
        let _ = setup.mock_transport.get_sent_message().await.expect("No initial message was sent");
        
        // Wait for a retransmission after ~20ms (2*T1)
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Timed out waiting for first retransmission");
        let retransmission1 = setup.mock_transport.get_sent_message().await.expect("No first retransmission was sent");
        
        // And another retransmission after ~40ms (2*2*T1)
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Timed out waiting for second retransmission");
        let retransmission2 = setup.mock_transport.get_sent_message().await.expect("No second retransmission was sent");
        
        // Verify they are CANCEL messages
        match retransmission1.0 {
            Message::Request(req) => assert_eq!(req.method(), Method::Cancel),
            _ => panic!("Expected Request message for retransmission"),
        }
        
        match retransmission2.0 {
            Message::Request(req) => assert_eq!(req.method(), Method::Cancel),
            _ => panic!("Expected Request message for retransmission"),
        }
    }

    #[tokio::test]
    async fn test_invalid_request_for_cancel() {
        // Try to create a CANCEL from a non-INVITE request
        let builder_result = SimpleRequestBuilder::new(Method::Options, "sip:bob@target.com");
        let builder = match builder_result {
            Ok(builder) => builder,
            Err(e) => panic!("Failed to create request builder: {}", e),
        };
        
        let options_request = builder
            .from("Alice", "sip:test@test.com", Some("fromtag"))
            .to("Bob", "sip:bob@target.com", None)
            .call_id("test-call-id")
            .cseq(102)
            .via("SIP/2.0/UDP", "test.example.com", Some("z9hG4bK.somebranchvalue"))
            .build();
            
        let invite_tx_id = TransactionKey::new("z9hG4bK.somebranchvalue".to_string(), Method::Cancel, false);
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        let mock_transport = Arc::new(UnitTestMockTransport::new("127.0.0.1:5060"));
        let (tu_events_tx, _) = mpsc::channel(32);
        
        // This should fail
        let result = ClientCancelTransaction::new(
            options_request,
            invite_tx_id,
            remote_addr,
            mock_transport,
            tu_events_tx,
            None,
        );
        
        assert!(result.is_err(), "Should fail to create CANCEL from non-INVITE request");
    }
} 