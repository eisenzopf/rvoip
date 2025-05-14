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

use crate::error::{Error, Result};
use crate::transaction::{
    Transaction, TransactionAsync, TransactionState, TransactionKind, TransactionKey, TransactionEvent,
    InternalTransactionCommand, AtomicTransactionState,
};
use crate::timer::{TimerSettings, TimerFactory, TimerManager, TimerType};
use crate::server::{
    ServerTransaction, ServerTransactionData, CommonServerTransaction
};
use crate::transaction::logic::TransactionLogic;
use crate::transaction::runner::{run_transaction_loop, HasCommandSender, AsRefKey};
use crate::transaction::timer_utils;
use crate::transaction::validators;
use crate::transaction::common_logic;
use crate::utils;

/// Server INVITE transaction (RFC 3261 Section 17.2.1)
#[derive(Debug, Clone)]
pub struct ServerInviteTransaction {
    data: Arc<ServerTransactionData>,
    logic: Arc<ServerInviteLogic>,
}

/// Holds JoinHandles and dynamic state for timers specific to Server INVITE transactions.
#[derive(Default, Debug)]
struct ServerInviteTimerHandles {
    timer_g: Option<JoinHandle<()>>,
    current_timer_g_interval: Option<Duration>, // For backoff
    timer_h: Option<JoinHandle<()>>,
    timer_i: Option<JoinHandle<()>>,
}

/// Implements the TransactionLogic for Server INVITE transactions.
#[derive(Debug, Clone, Default)]
struct ServerInviteLogic {
    _data_marker: std::marker::PhantomData<ServerTransactionData>,
    timer_factory: TimerFactory,
}

impl ServerInviteLogic {
    // Helper method to start Timer G (retransmission timer) using timer utils
    async fn start_timer_g(
        &self,
        data: &Arc<ServerTransactionData>,
        timer_handles: &mut ServerInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        // Start Timer G (retransmission) with initial interval T1
        let initial_interval_g = timer_handles.current_timer_g_interval.unwrap_or(timer_config.t1);
        
        // Use timer_utils to start the timer
        let timer_manager = self.timer_factory.timer_manager();
        match timer_utils::start_transaction_timer(
            &timer_manager,
            tx_id,
            "G",
            TimerType::G,
            initial_interval_g,
            command_tx
        ).await {
            Ok(handle) => {
                timer_handles.timer_g = Some(handle);
                trace!(id=%tx_id, interval=?initial_interval_g, "Started Timer G for Completed state");
            },
            Err(e) => {
                error!(id=%tx_id, error=%e, "Failed to start Timer G");
            }
        }
    }
    
    // Helper method to start Timer H (transaction timeout for ACK) using timer utils
    async fn start_timer_h(
        &self,
        data: &Arc<ServerTransactionData>,
        timer_handles: &mut ServerInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        // Start Timer H
        let interval_h = timer_config.wait_time_h;
        
        // Use timer_utils to start the timer
        let timer_manager = self.timer_factory.timer_manager();
        match timer_utils::start_transaction_timer(
            &timer_manager,
            tx_id,
            "H",
            TimerType::H,
            interval_h,
            command_tx
        ).await {
            Ok(handle) => {
                timer_handles.timer_h = Some(handle);
                trace!(id=%tx_id, interval=?interval_h, "Started Timer H for Completed state");
            },
            Err(e) => {
                error!(id=%tx_id, error=%e, "Failed to start Timer H");
            }
        }
    }
    
    // Helper method to start Timer I (wait for retransmissions in Confirmed state) using timer utils with transition
    async fn start_timer_i(
        &self,
        data: &Arc<ServerTransactionData>,
        timer_handles: &mut ServerInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        // Start Timer I that automatically transitions to Terminated state when it fires
        let interval_i = timer_config.wait_time_i;
        
        // Use timer_utils to start the timer with transition
        let timer_manager = self.timer_factory.timer_manager();
        match timer_utils::start_timer_with_transition(
            &timer_manager,
            tx_id,
            "I",
            TimerType::I,
            interval_i,
            command_tx,
            TransactionState::Terminated
        ).await {
            Ok(handle) => {
                timer_handles.timer_i = Some(handle);
                trace!(id=%tx_id, interval=?interval_i, "Started Timer I for Confirmed state");
            },
            Err(e) => {
                error!(id=%tx_id, error=%e, "Failed to start Timer I");
            }
        }
    }

    // Handle Timer G (retransmission in Completed state) trigger
    async fn handle_timer_g_trigger(
        &self,
        data: &Arc<ServerTransactionData>,
        current_state: TransactionState,
        timer_handles: &mut ServerInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        match current_state {
            TransactionState::Completed => {
                debug!(id=%tx_id, "Timer G triggered, retransmitting final response");
                
                // Retransmit the final response
                let response_guard = data.last_response.lock().await;
                if let Some(response) = &*response_guard {
                    if let Err(e) = data.transport.send_message(
                        Message::Response(response.clone()),
                        data.remote_addr
                    ).await {
                        error!(id=%tx_id, error=%e, "Failed to retransmit response");
                        common_logic::send_transport_error_event(tx_id, &data.events_tx).await;
                        return Ok(Some(TransactionState::Terminated));
                    }
                }
                drop(response_guard);
                
                // Update and restart Timer G with increased interval using the utility function
                let current_interval = timer_handles.current_timer_g_interval.unwrap_or(timer_config.t1);
                let new_interval = timer_utils::calculate_backoff_interval(current_interval, timer_config);
                timer_handles.current_timer_g_interval = Some(new_interval);
                
                // Start new Timer G with the increased interval
                self.start_timer_g(data, timer_handles, command_tx).await;
            },
            _ => {
                trace!(id=%tx_id, state=?current_state, "Timer G fired in invalid state, ignoring");
            }
        }
        
        Ok(None)
    }
    
    // Handle Timer H (wait for ACK in Completed state) trigger
    async fn handle_timer_h_trigger(
        &self,
        data: &Arc<ServerTransactionData>,
        current_state: TransactionState,
        _command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match current_state {
            TransactionState::Completed => {
                warn!(id=%tx_id, "Timer H (ACK Timeout) fired in Completed state");
                
                // Notify TU about timeout using common logic
                common_logic::send_transaction_timeout_event(tx_id, &data.events_tx).await;
                
                // Return state transition
                return Ok(Some(TransactionState::Terminated));
            },
            _ => {
                trace!(id=%tx_id, state=?current_state, "Timer H fired in invalid state, ignoring");
            }
        }
        
        Ok(None)
    }
    
    // Handle Timer I (wait for retransmissions in Confirmed state) trigger
    async fn handle_timer_i_trigger(
        &self,
        data: &Arc<ServerTransactionData>,
        current_state: TransactionState,
        _command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match current_state {
            TransactionState::Confirmed => {
                debug!(id=%tx_id, "Timer I fired in Confirmed state, terminating");
                // Timer I automatically transitions to Terminated, no need to return a state
                Ok(None)
            },
            _ => {
                trace!(id=%tx_id, state=?current_state, "Timer I fired in invalid state, ignoring");
                Ok(None)
            }
        }
    }
    
    // Process a retransmitted INVITE request
    async fn process_invite_retransmission(
        &self,
        data: &Arc<ServerTransactionData>,
        _request: Request,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match current_state {
            TransactionState::Proceeding => {
                debug!(id=%tx_id, "Received INVITE retransmission in Proceeding state");
                
                // Retransmit the last provisional response
                let last_response = data.last_response.lock().await;
                if let Some(response) = &*last_response {
                    if let Err(e) = data.transport.send_message(
                        Message::Response(response.clone()),
                        data.remote_addr
                    ).await {
                        error!(id=%tx_id, error=%e, "Failed to retransmit response");
                        return Ok(None);
                    }
                }
                
                // No state transition needed for INVITE retransmission
                Ok(None)
            },
            _ => {
                // INVITE retransmissions in other states are ignored
                trace!(id=%tx_id, state=?current_state, "Ignoring INVITE retransmission in state {:?}", current_state);
                Ok(None)
            }
        }
    }
    
    // Process an ACK request
    async fn process_ack(
        &self,
        data: &Arc<ServerTransactionData>,
        request: Request,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match current_state {
            TransactionState::Completed => {
                debug!(id=%tx_id, "Received ACK in Completed state");
                
                // Notify TU about ACK
                let _ = data.events_tx.send(TransactionEvent::AckReceived {
                    transaction_id: tx_id.clone(),
                    request: request.clone(),
                }).await;
                
                // Transition to Confirmed state
                Ok(Some(TransactionState::Confirmed))
            },
            TransactionState::Confirmed => {
                // ACK retransmission, already in Confirmed state
                trace!(id=%tx_id, "Received duplicate ACK in Confirmed state, ignoring");
                Ok(None)
            },
            _ => {
                warn!(id=%tx_id, state=?current_state, "Received ACK in unexpected state");
                Ok(None)
            }
        }
    }
    
    // Process a CANCEL request
    async fn process_cancel(
        &self,
        data: &Arc<ServerTransactionData>,
        request: Request,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match current_state {
            TransactionState::Proceeding => {
                debug!(id=%tx_id, "Received CANCEL in Proceeding state");
                
                // Notify TU about CANCEL
                let _ = data.events_tx.send(TransactionEvent::CancelReceived {
                    transaction_id: tx_id.clone(),
                    cancel_request: request.clone(),
                }).await;
                
                // No state transition needed for CANCEL
                Ok(None)
            },
            _ => {
                trace!(id=%tx_id, state=?current_state, "Ignoring CANCEL in non-proceeding state");
                Ok(None)
            }
        }
    }
}

#[async_trait::async_trait]
impl TransactionLogic<ServerTransactionData, ServerInviteTimerHandles> for ServerInviteLogic {
    fn kind(&self) -> TransactionKind {
        TransactionKind::InviteServer
    }

    fn initial_state(&self) -> TransactionState {
        TransactionState::Proceeding
    }

    fn timer_settings<'a>(data: &'a Arc<ServerTransactionData>) -> &'a TimerSettings {
        &data.timer_config
    }

    fn cancel_all_specific_timers(&self, timer_handles: &mut ServerInviteTimerHandles) {
        if let Some(handle) = timer_handles.timer_g.take() {
            handle.abort();
        }
        if let Some(handle) = timer_handles.timer_h.take() {
            handle.abort();
        }
        if let Some(handle) = timer_handles.timer_i.take() {
            handle.abort();
        }
        // Reset current_timer_g_interval
        timer_handles.current_timer_g_interval = None;
    }

    async fn on_enter_state(
        &self,
        data: &Arc<ServerTransactionData>,
        new_state: TransactionState,
        _previous_state: TransactionState,
        timer_handles: &mut ServerInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<()> {
        let tx_id = &data.id;

        match new_state {
            TransactionState::Proceeding => {
                debug!(id=%tx_id, "Entered Proceeding state. No timers are started yet.");
            }
            TransactionState::Completed => {
                debug!(id=%tx_id, "Entered Completed state after sending final response.");
                // Start Timer G for retransmitting responses
                timer_handles.current_timer_g_interval = Some(data.timer_config.t1);
                self.start_timer_g(data, timer_handles, command_tx.clone()).await;
                
                // Start Timer H to guard against lost ACKs
                self.start_timer_h(data, timer_handles, command_tx).await;
            }
            TransactionState::Confirmed => {
                debug!(id=%tx_id, "Entered Confirmed state after receiving ACK.");
                // Start Timer I for final cleanup
                self.start_timer_i(data, timer_handles, command_tx).await;
            }
            TransactionState::Terminated => {
                trace!(id=%tx_id, "Entered Terminated state. Specific timers should have been cancelled by runner.");
                // Unregister from timer manager when terminated
                let timer_manager = self.timer_factory.timer_manager();
                timer_utils::unregister_transaction(&timer_manager, tx_id).await;
            }
            _ => {
                trace!(id=%tx_id, "Entered unhandled state {:?} in on_enter_state", new_state);
            }
        }
        Ok(())
    }

    async fn handle_timer(
        &self,
        data: &Arc<ServerTransactionData>,
        timer_name: &str,
        current_state: TransactionState,
        timer_handles: &mut ServerInviteTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        // Clear the timer handle since it fired
        match timer_name {
            "G" => { timer_handles.timer_g.take(); }
            "H" => { timer_handles.timer_h.take(); }
            "I" => { timer_handles.timer_i.take(); }
            _ => {}
        }
        
        // Send timer triggered event using common logic
        common_logic::send_timer_triggered_event(tx_id, timer_name, &data.events_tx).await;
        
        // Use the command_tx from data
        let self_command_tx = data.cmd_tx.clone();
        
        match timer_name {
            "G" => self.handle_timer_g_trigger(data, current_state, timer_handles, self_command_tx).await,
            "H" => self.handle_timer_h_trigger(data, current_state, self_command_tx).await,
            "I" => self.handle_timer_i_trigger(data, current_state, self_command_tx).await,
            _ => {
                warn!(id=%tx_id, timer_name=%timer_name, "Unknown timer triggered for ServerInvite");
                Ok(None)
            }
        }
    }

    async fn process_message(
        &self,
        data: &Arc<ServerTransactionData>,
        message: Message,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match message {
            Message::Request(request) => {
                let method = request.method();
                
                match method {
                    Method::Invite => self.process_invite_retransmission(data, request, current_state).await,
                    Method::Ack => self.process_ack(data, request, current_state).await,
                    Method::Cancel => self.process_cancel(data, request, current_state).await,
                    _ => {
                        warn!(id=%tx_id, method=%method, "Received unexpected request method");
                        Ok(None)
                    }
                }
            },
            Message::Response(_) => {
                warn!(id=%tx_id, "Server transaction received a Response, ignoring");
                Ok(None)
            }
        }
    }
}

impl ServerInviteTransaction {
    /// Create a new server INVITE transaction.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config_override: Option<TimerSettings>,
    ) -> Result<Self> {
        if request.method() != Method::Invite {
            return Err(Error::Other("Request must be INVITE for INVITE server transaction".to_string()));
        }

        let timer_config = timer_config_override.unwrap_or_default();
        let (cmd_tx, local_cmd_rx) = mpsc::channel(32);

        let data = Arc::new(ServerTransactionData {
            id: id.clone(),
            state: Arc::new(AtomicTransactionState::new(TransactionState::Proceeding)),
            request: Arc::new(Mutex::new(request.clone())),
            last_response: Arc::new(Mutex::new(None)),
            remote_addr,
            transport,
            events_tx,
            cmd_tx: cmd_tx.clone(),
            cmd_rx: Arc::new(Mutex::new(local_cmd_rx)),
            event_loop_handle: Arc::new(Mutex::new(None)),
            timer_config: timer_config.clone(),
        });

        let logic = Arc::new(ServerInviteLogic {
            _data_marker: std::marker::PhantomData,
            timer_factory: TimerFactory::new(Some(timer_config), Arc::new(TimerManager::new(None))),
        });

        let data_for_runner = data.clone();
        let logic_for_runner = logic.clone();
        
        // Spawn the generic event loop runner - get the receiver from the data first in a separate tokio task
        let event_loop_handle = tokio::spawn(async move {
            let mut cmd_rx_guard = data_for_runner.cmd_rx.lock().await;
            // Take the receiver out of the Mutex, replacing it with a dummy receiver
            let cmd_rx = std::mem::replace(&mut *cmd_rx_guard, mpsc::channel(1).1);
            // Drop the guard to release the lock
            drop(cmd_rx_guard);
            run_transaction_loop(data_for_runner, logic_for_runner, cmd_rx).await;
        });

        // Store the handle for cleanup
        if let Ok(mut handle_guard) = data.event_loop_handle.try_lock() {
            *handle_guard = Some(event_loop_handle);
        }
        
        Ok(Self { data, logic })
    }
}

impl CommonServerTransaction for ServerInviteTransaction {
    fn data(&self) -> &Arc<ServerTransactionData> {
        &self.data
    }
}

impl Transaction for ServerInviteTransaction {
    fn id(&self) -> &TransactionKey {
        &self.data.id
    }

    fn kind(&self) -> TransactionKind {
        TransactionKind::InviteServer
    }

    fn state(&self) -> TransactionState {
        self.data.state.get()
    }
    
    fn remote_addr(&self) -> SocketAddr {
        self.data.remote_addr
    }
    
    fn matches(&self, message: &Message) -> bool {
        utils::transaction_key_from_message(message).map(|key| key == self.data.id).unwrap_or(false)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl TransactionAsync for ServerInviteTransaction {
    fn process_event<'a>(
        &'a self,
        event_type: &'a str,
        message: Option<Message>
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event_type {
                "request" => {
                    if let Some(Message::Request(request)) = message {
                        self.process_request(request).await
                    } else {
                        Err(Error::Other("Expected Request message".to_string()))
                    }
                },
                "response" => {
                    if let Some(Message::Response(response)) = message {
                        self.send_response(response).await
                    } else {
                        Err(Error::Other("Expected Response message".to_string()))
                    }
                },
                _ => Err(Error::Other(format!("Unhandled event type: {}", event_type))),
            }
        })
    }

    fn send_command<'a>(
        &'a self,
        cmd: InternalTransactionCommand
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let data = self.data.clone();
        
        Box::pin(async move {
            data.cmd_tx.send(cmd).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))
        })
    }

    fn original_request<'a>(
        &'a self
    ) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + 'a>> {
        Box::pin(async move {
            Some(self.data.request.lock().await.clone())
        })
    }

    fn last_response<'a>(
        &'a self
    ) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + 'a>> {
        Box::pin(async move {
            self.data.last_response.lock().await.clone()
        })
    }
}

impl ServerTransaction for ServerInviteTransaction {
    fn process_request(&self, request: Request) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        
        Box::pin(async move {
            data.cmd_tx.send(InternalTransactionCommand::ProcessMessage(Message::Request(request))).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))?;
            
            Ok(())
        })
    }
    
    fn send_response(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        
        Box::pin(async move {
            let status = response.status();
            let is_provisional = status.is_provisional();
            let is_success = status.is_success();
            let current_state = data.state.get();
            
            // Store this response
            {
                let mut response_guard = data.last_response.lock().await;
                *response_guard = Some(response.clone());
            }
            
            // Always send the response
            data.transport.send_message(Message::Response(response.clone()), data.remote_addr)
                .await
                .map_err(|e| Error::transport_error(e, "Failed to send response"))?;
            
            // For preliminary responses in Proceeding state, stay in Proceeding
            if is_provisional && current_state == TransactionState::Proceeding {
                // Stays in Proceeding state, no state change
                trace!(id=%data.id, "Sent provisional response, staying in Proceeding state");
                return Ok(());
            }
            
            // For 2xx responses, directly terminate the transaction
            if is_success {
                debug!(id=%data.id, "Sent 2xx response, transitioning to Terminated");
                
                // TU level will handle reliable delivery of 2xx responses
                data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await
                    .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
                
                return Ok(());
            }
            
            // For >= 300 responses, transition to Completed
            if !is_provisional && !is_success && current_state == TransactionState::Proceeding {
                debug!(id=%data.id, "Sent >= 300 response, transitioning to Completed");
                
                data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Completed)).await
                    .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
            }
            
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use tokio::sync::Notify;
    use tokio::time::timeout as TokioTimeout;
    use std::collections::VecDeque;
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use rvoip_sip_core::types::status::StatusCode;

    #[derive(Debug, Clone)]
    struct UnitTestMockTransport {
        sent_messages: Arc<Mutex<VecDeque<(Message, SocketAddr)>>>,
        local_addr: SocketAddr,
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

        async fn wait_for_message_sent(&self, duration: Duration) -> std::result::Result<(), tokio::time::error::Elapsed> {
            TokioTimeout(duration, self.message_sent_notifier.notified()).await
        }
    }

    #[async_trait::async_trait]
    impl Transport for UnitTestMockTransport {
        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        async fn send_message(&self, message: Message, destination: SocketAddr) -> std::result::Result<(), rvoip_sip_transport::Error> {
            self.sent_messages.lock().await.push_back((message.clone(), destination));
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

    struct TestSetup {
        transaction: ServerInviteTransaction,
        mock_transport: Arc<UnitTestMockTransport>,
        tu_events_rx: mpsc::Receiver<TransactionEvent>,
    }

    async fn setup_test_environment() -> TestSetup {
        let local_addr = "127.0.0.1:5090";
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        let mock_transport = Arc::new(UnitTestMockTransport::new(local_addr));
        let (tu_events_tx, tu_events_rx) = mpsc::channel(100);

        let builder = SimpleRequestBuilder::new(Method::Invite, "sip:bob@target.com")
            .expect("Failed to create SimpleRequestBuilder")
            .from("Alice", "sip:alice@atlanta.com", Some("fromtag"))
            .to("Bob", "sip:bob@target.com", None)
            .call_id("callid-invite-server-test")
            .cseq(1);
        
        let via_branch = format!("z9hG4bK.{}", uuid::Uuid::new_v4().as_simple());
        let builder = builder.via(remote_addr.to_string().as_str(), "UDP", Some(&via_branch));

        let request = builder.build();
        
        let tx_key = TransactionKey::from_request(&request).expect("Failed to create tx key from request");

        let settings = TimerSettings {
            t1: Duration::from_millis(50),
            t2: Duration::from_millis(100),
            transaction_timeout: Duration::from_millis(200),
            wait_time_h: Duration::from_millis(100),
            wait_time_i: Duration::from_millis(100),
            ..Default::default()
        };

        let transaction = ServerInviteTransaction::new(
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
    
    fn build_simple_response(status_code: StatusCode, original_request: &Request) -> Response {
        SimpleResponseBuilder::response_from_request(
            original_request,
            status_code,
            Some(status_code.reason_phrase())
        ).build()
    }

    #[tokio::test]
    async fn test_server_invite_creation() {
        let setup = setup_test_environment().await;
        assert_eq!(setup.transaction.state(), TransactionState::Proceeding);
        assert!(setup.transaction.data.event_loop_handle.lock().await.is_some());
    }

    #[tokio::test]
    async fn test_server_invite_send_provisional_response() {
        let mut setup = setup_test_environment().await;
        
        // Create a provisional response
        let original_request = setup.transaction.data.request.lock().await.clone();
        let prov_response = build_simple_response(StatusCode::Ringing, &original_request);
        
        // Send the response
        setup.transaction.send_response(prov_response.clone()).await.expect("send_response failed");
        
        // Wait for the response to be sent
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Response should be sent quickly");
        
        // Check sent message
        let sent_msg_info = setup.mock_transport.get_sent_message().await;
        assert!(sent_msg_info.is_some(), "Response should have been sent");
        if let Some((msg, dest)) = sent_msg_info {
            assert!(msg.is_response());
            if let Message::Response(resp) = msg {
                assert_eq!(resp.status_code(), StatusCode::Ringing.as_u16());
            }
            assert_eq!(dest, setup.transaction.remote_addr());
        }
        
        // We should stay in Proceeding state for provisional responses
        assert_eq!(setup.transaction.state(), TransactionState::Proceeding);
    }

    #[tokio::test]
    async fn test_server_invite_send_final_error_response() {
        let mut setup = setup_test_environment().await;
        
        // Create a final response
        let original_request = setup.transaction.data.request.lock().await.clone();
        let final_response = build_simple_response(StatusCode::NotFound, &original_request);
        
        // Send the response
        setup.transaction.send_response(final_response.clone()).await.expect("send_response failed");
        
        // Wait for the response to be sent
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Response should be sent quickly");
        
        // Check sent message
        let sent_msg_info = setup.mock_transport.get_sent_message().await;
        assert!(sent_msg_info.is_some(), "Response should have been sent");
        if let Some((msg, dest)) = sent_msg_info {
            assert!(msg.is_response());
            if let Message::Response(resp) = msg {
                assert_eq!(resp.status_code(), StatusCode::NotFound.as_u16());
            }
            assert_eq!(dest, setup.transaction.remote_addr());
        }
        
        // Check for state transition event
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { transaction_id, previous_state, new_state })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(previous_state, TransactionState::Proceeding);
                assert_eq!(new_state, TransactionState::Completed);
            },
            Ok(Some(other_event)) => panic!("Unexpected event: {:?}", other_event),
            _ => panic!("Expected StateChanged event"),
        }
        
        // Check state
        assert_eq!(setup.transaction.state(), TransactionState::Completed);
        
        // Wait for Timer G to trigger a retransmission
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Response should be retransmitted");
        let retrans_msg_info = setup.mock_transport.get_sent_message().await;
        assert!(retrans_msg_info.is_some(), "Response should have been retransmitted");
    }

    #[tokio::test]
    async fn test_server_invite_send_success_response() {
        let mut setup = setup_test_environment().await;
        
        // Create a 2xx response
        let original_request = setup.transaction.data.request.lock().await.clone();
        let success_response = build_simple_response(StatusCode::Ok, &original_request);
        
        // Send the response
        setup.transaction.send_response(success_response.clone()).await.expect("send_response failed");
        
        // Wait for the response to be sent
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Response should be sent quickly");
        
        // Check for state transition event
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { transaction_id, previous_state, new_state })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(previous_state, TransactionState::Proceeding);
                assert_eq!(new_state, TransactionState::Terminated);
            },
            Ok(Some(other_event)) => panic!("Unexpected event: {:?}", other_event),
            _ => panic!("Expected StateChanged event"),
        }
        
        // Check state
        assert_eq!(setup.transaction.state(), TransactionState::Terminated);
    }

    #[tokio::test]
    async fn test_server_invite_ack_handling() {
        let mut setup = setup_test_environment().await;
        
        // Create and send a final error response
        let original_request = setup.transaction.data.request.lock().await.clone();
        let final_response = build_simple_response(StatusCode::NotFound, &original_request);
        setup.transaction.send_response(final_response.clone()).await.expect("send_response failed");
        
        // Wait for state transition to Completed
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { new_state, .. })) => {
                assert_eq!(new_state, TransactionState::Completed);
            },
            _ => panic!("Expected StateChanged event"),
        }
        
        // Create an ACK request
        let ack_request = SimpleRequestBuilder::new(Method::Ack, "sip:bob@target.com").unwrap()
            .from("Alice", "sip:alice@atlanta.com", Some("fromtag"))
            .to("Bob", "sip:bob@target.com", None)
            .call_id("callid-invite-server-test")
            .cseq(1)
            .via(setup.transaction.remote_addr().to_string().as_str(), "UDP", 
                 Some(setup.transaction.id().branch.as_str()))
            .build();
        
        // Send the ACK
        setup.transaction.process_request(ack_request.clone()).await.expect("process_request failed");
        
        // Check for AckReceived event
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::AckReceived { transaction_id, request })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(request.method(), Method::Ack);
            },
            Ok(Some(other_event)) => panic!("Unexpected event: {:?}", other_event),
            _ => panic!("Expected AckReceived event"),
        }
        
        // Check for state transition to Confirmed
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { transaction_id, previous_state, new_state })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(previous_state, TransactionState::Completed);
                assert_eq!(new_state, TransactionState::Confirmed);
            },
            Ok(Some(other_event)) => panic!("Unexpected event: {:?}", other_event),
            _ => panic!("Expected StateChanged event"),
        }
        
        // Check state
        assert_eq!(setup.transaction.state(), TransactionState::Confirmed);
        
        // Wait for Timer I to fire and transition to Terminated
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(setup.transaction.state(), TransactionState::Terminated);
    }
} 