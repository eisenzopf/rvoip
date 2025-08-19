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
use crate::transaction::server::{
    ServerTransaction, ServerTransactionData, CommonServerTransaction
};
use crate::transaction::logic::TransactionLogic;
use crate::transaction::runner::{run_transaction_loop, HasCommandSender, AsRefKey};
use crate::transaction::timer_utils;
use crate::transaction::validators;
use crate::transaction::common_logic;
use crate::transaction::utils;

/// Server non-INVITE transaction (RFC 3261 Section 17.2.2)
#[derive(Debug, Clone)]
pub struct ServerNonInviteTransaction {
    data: Arc<ServerTransactionData>,
    logic: Arc<ServerNonInviteLogic>,
}

/// Holds JoinHandles and dynamic state for timers specific to Server Non-INVITE transactions.
#[derive(Default, Debug)]
struct ServerNonInviteTimerHandles {
    timer_j: Option<JoinHandle<()>>,
}

/// Implements the TransactionLogic for Server Non-INVITE transactions.
#[derive(Debug, Clone, Default)]
struct ServerNonInviteLogic {
    _data_marker: std::marker::PhantomData<ServerTransactionData>,
    timer_factory: TimerFactory,
}

impl ServerNonInviteLogic {
    // Helper method to start Timer J (wait for retransmissions) using timer utils with transition
    async fn start_timer_j(
        &self,
        data: &Arc<ServerTransactionData>,
        timer_handles: &mut ServerNonInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        // Start Timer J that automatically transitions to Terminated state when it fires
        let interval_j = timer_config.wait_time_j;
        
        // Use timer_utils to start the timer with transition
        let timer_manager = self.timer_factory.timer_manager();
        match timer_utils::start_timer_with_transition(
            &timer_manager,
            tx_id,
            "J",
            TimerType::J,
            interval_j,
            command_tx,
            TransactionState::Terminated
        ).await {
            Ok(handle) => {
                timer_handles.timer_j = Some(handle);
                trace!(id=%tx_id, interval=?interval_j, "Started Timer J for Completed state");
            },
            Err(e) => {
                error!(id=%tx_id, error=%e, "Failed to start Timer J");
            }
        }
    }
    
    // Handle Timer J (wait for retransmissions) trigger
    async fn handle_timer_j_trigger(
        &self,
        data: &Arc<ServerTransactionData>,
        current_state: TransactionState,
        _command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match current_state {
            TransactionState::Completed => {
                debug!(id=%tx_id, "Timer J fired in Completed state, terminating");
                // Timer J automatically transitions to Terminated, no need to return a state
                Ok(None)
            },
            _ => {
                trace!(id=%tx_id, state=?current_state, "Timer J fired in invalid state, ignoring");
                Ok(None)
            }
        }
    }
    
    // Process a retransmitted SIP request
    async fn process_request_retransmission(
        &self,
        data: &Arc<ServerTransactionData>,
        request: Request,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match current_state {
            TransactionState::Trying | TransactionState::Proceeding | TransactionState::Completed => {
                debug!(id=%tx_id, state=?current_state, "Received request retransmission");
                
                // If in Completed state, retransmit the last response
                if current_state == TransactionState::Completed {
                    let last_response = data.last_response.lock().await;
                    if let Some(response) = &*last_response {
                        if let Err(e) = data.transport.send_message(
                            Message::Response(response.clone()),
                            data.remote_addr
                        ).await {
                            error!(id=%tx_id, error=%e, "Failed to retransmit response");
                        }
                    }
                }
                
                // No state transition needed for request retransmission
                Ok(None)
            },
            _ => {
                // Requests in other states are ignored
                trace!(id=%tx_id, state=?current_state, "Ignoring request in state {:?}", current_state);
                Ok(None)
            }
        }
    }
}

#[async_trait::async_trait]
impl TransactionLogic<ServerTransactionData, ServerNonInviteTimerHandles> for ServerNonInviteLogic {
    fn kind(&self) -> TransactionKind {
        TransactionKind::NonInviteServer
    }

    fn initial_state(&self) -> TransactionState {
        TransactionState::Trying
    }

    fn timer_settings<'a>(data: &'a Arc<ServerTransactionData>) -> &'a TimerSettings {
        &data.timer_config
    }

    fn cancel_all_specific_timers(&self, timer_handles: &mut ServerNonInviteTimerHandles) {
        if let Some(handle) = timer_handles.timer_j.take() {
            handle.abort();
        }
    }

    async fn on_enter_state(
        &self,
        data: &Arc<ServerTransactionData>,
        new_state: TransactionState,
        previous_state: TransactionState,
        timer_handles: &mut ServerNonInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<()> {
        let tx_id = &data.id;

        match new_state {
            TransactionState::Trying => {
                trace!(id=%tx_id, "Entered Trying state. No timers are started yet until a response is sent.");
            }
            TransactionState::Proceeding => {
                debug!(id=%tx_id, "Entered Proceeding state after sending provisional response.");
                // No timers are started in Proceeding state for non-INVITE server transactions
            }
            TransactionState::Completed => {
                debug!(id=%tx_id, "Entered Completed state after sending final response.");
                // Start Timer J
                self.start_timer_j(data, timer_handles, command_tx).await;
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
        timer_handles: &mut ServerNonInviteTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        if timer_name == "J" {
            // Clear the timer handle since it fired
            timer_handles.timer_j.take();
        }
        
        // Send timer triggered event using common logic
        common_logic::send_timer_triggered_event(tx_id, timer_name, &data.events_tx).await;
        
        // Use the command_tx from data
        let self_command_tx = data.cmd_tx.clone();
        
        match timer_name {
            "J" => self.handle_timer_j_trigger(data, current_state, self_command_tx).await,
            _ => {
                warn!(id=%tx_id, timer_name=%timer_name, "Unknown timer triggered for ServerNonInvite");
                Ok(None)
            }
        }
    }

    async fn process_message(
        &self,
        data: &Arc<ServerTransactionData>,
        message: Message,
        current_state: TransactionState,
        timer_handles: &mut ServerNonInviteTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        match message {
            Message::Request(request) => {
                self.process_request_retransmission(data, request, current_state).await
            },
            Message::Response(_) => {
                warn!(id=%tx_id, "Server transaction received a Response, ignoring");
                Ok(None)
            }
        }
    }
}

impl ServerNonInviteTransaction {
    /// Create a new server non-INVITE transaction.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config_override: Option<TimerSettings>,
    ) -> Result<Self> {
        if request.method() == Method::Invite || request.method() == Method::Ack {
            return Err(Error::Other("Request must not be INVITE or ACK for non-INVITE server transaction".to_string()));
        }

        let timer_config = timer_config_override.unwrap_or_default();
        let (cmd_tx, local_cmd_rx) = mpsc::channel(32);

        let data = Arc::new(ServerTransactionData {
            id: id.clone(),
            state: Arc::new(AtomicTransactionState::new(TransactionState::Trying)),
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

        let logic = Arc::new(ServerNonInviteLogic {
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

impl CommonServerTransaction for ServerNonInviteTransaction {
    fn data(&self) -> &Arc<ServerTransactionData> {
        &self.data
    }
}

impl Transaction for ServerNonInviteTransaction {
    fn id(&self) -> &TransactionKey {
        &self.data.id
    }

    fn kind(&self) -> TransactionKind {
        TransactionKind::NonInviteServer
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

impl TransactionAsync for ServerNonInviteTransaction {
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

impl ServerTransaction for ServerNonInviteTransaction {
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
            
            // State transitions
            if current_state == TransactionState::Trying {
                if is_provisional {
                    // 1xx -> Proceeding
                    debug!(id=%data.id, "Sent provisional response, transitioning to Proceeding");
                    data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Proceeding)).await
                        .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
                } else {
                    // Final response -> Completed
                    debug!(id=%data.id, "Sent final response, transitioning to Completed");
                    data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Completed)).await
                        .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
                }
            } else if current_state == TransactionState::Proceeding {
                if !is_provisional {
                    // Final response -> Completed
                    debug!(id=%data.id, "Sent final response, transitioning to Completed");
                    data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Completed)).await
                        .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
                }
            }
            
            Ok(())
        })
    }

    // Add the required last_response implementation for ServerTransaction
    fn last_response(&self) -> Option<Response> {
        // Return the last response from the last_response field
        // We use try_lock() instead of lock() to avoid blocking
        // If the lock is already held, we return None
        self.data.last_response.try_lock().ok()?.clone()
    }
    
    // Implement the synchronous original request accessor
    fn original_request_sync(&self) -> Option<Request> {
        // Try to get the original request from the data structure's request field
        // We use try_lock() to avoid blocking if the lock is held
        self.data.request.try_lock().ok().map(|req| req.clone())
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
        transaction: ServerNonInviteTransaction,
        mock_transport: Arc<UnitTestMockTransport>,
        tu_events_rx: mpsc::Receiver<TransactionEvent>,
    }

    async fn setup_test_environment(
        request_method: Method,
        target_uri_str: &str,
    ) -> TestSetup {
        let local_addr = "127.0.0.1:5090";
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        let mock_transport = Arc::new(UnitTestMockTransport::new(local_addr));
        let (tu_events_tx, tu_events_rx) = mpsc::channel(100);

        let req_uri = Uri::from_str(target_uri_str).unwrap();
        let builder = SimpleRequestBuilder::new(request_method, &req_uri.to_string())
            .expect("Failed to create SimpleRequestBuilder")
            .from("Alice", "sip:alice@atlanta.com", Some("fromtag"))
            .to("Bob", "sip:bob@target.com", None)
            .call_id("callid-noninvite-server-test")
            .cseq(1);
        
        let via_branch = format!("z9hG4bK.{}", uuid::Uuid::new_v4().as_simple());
        let builder = builder.via(remote_addr.to_string().as_str(), "UDP", Some(&via_branch));

        let request = builder.build();
        
        let tx_key = TransactionKey::from_request(&request).expect("Failed to create tx key from request");

        let settings = TimerSettings {
            t1: Duration::from_millis(50),
            transaction_timeout: Duration::from_millis(200),
            wait_time_j: Duration::from_millis(100),
            ..Default::default()
        };

        let transaction = ServerNonInviteTransaction::new(
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
    async fn test_server_noninvite_creation() {
        let setup = setup_test_environment(Method::Register, "sip:registrar.example.com").await;
        assert_eq!(setup.transaction.state(), TransactionState::Trying);
        assert!(setup.transaction.data.event_loop_handle.lock().await.is_some());
    }

    #[tokio::test]
    async fn test_server_noninvite_send_provisional_response() {
        let mut setup = setup_test_environment(Method::Register, "sip:registrar.example.com").await;
        
        // Create a provisional response
        let original_request = setup.transaction.data.request.lock().await.clone();
        let prov_response = build_simple_response(StatusCode::Trying, &original_request);
        
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
                assert_eq!(resp.status_code(), StatusCode::Trying.as_u16());
            }
            assert_eq!(dest, setup.transaction.remote_addr());
        }
        
        // Check for state transition event
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { transaction_id, previous_state, new_state })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(previous_state, TransactionState::Trying);
                assert_eq!(new_state, TransactionState::Proceeding);
            },
            Ok(Some(other_event)) => panic!("Unexpected event: {:?}", other_event),
            _ => panic!("Expected StateChanged event"),
        }
        
        // Check state
        assert_eq!(setup.transaction.state(), TransactionState::Proceeding);
    }

    #[tokio::test]
    async fn test_server_noninvite_send_final_response() {
        let mut setup = setup_test_environment(Method::Register, "sip:registrar.example.com").await;
        
        // Create a final response
        let original_request = setup.transaction.data.request.lock().await.clone();
        let final_response = build_simple_response(StatusCode::Ok, &original_request);
        
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
                assert_eq!(resp.status_code(), StatusCode::Ok.as_u16());
            }
            assert_eq!(dest, setup.transaction.remote_addr());
        }
        
        // Check for state transition event
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { transaction_id, previous_state, new_state })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(previous_state, TransactionState::Trying);
                assert_eq!(new_state, TransactionState::Completed);
            },
            Ok(Some(other_event)) => panic!("Unexpected event: {:?}", other_event),
            _ => panic!("Expected StateChanged event"),
        }
        
        // Check state
        assert_eq!(setup.transaction.state(), TransactionState::Completed);
        
        // Wait for Timer J to fire and transition to Terminated
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(setup.transaction.state(), TransactionState::Terminated);
    }

    #[tokio::test]
    async fn test_server_noninvite_retransmit_final_response() {
        let mut setup = setup_test_environment(Method::Register, "sip:registrar.example.com").await;
        
        // Create and send final response
        let original_request = setup.transaction.data.request.lock().await.clone();
        let final_response = build_simple_response(StatusCode::Ok, &original_request);
        setup.transaction.send_response(final_response.clone()).await.expect("send_response failed");
        
        // Wait for response to be sent and state to change to Completed
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Response should be sent quickly");
        setup.mock_transport.get_sent_message().await;
        
        // Wait for state transition
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { new_state, .. })) => {
                assert_eq!(new_state, TransactionState::Completed);
            },
            _ => panic!("Expected StateChanged event"),
        }
        
        // Process a retransmitted request
        setup.transaction.process_request(original_request.clone()).await.expect("process_request failed");
        
        // Verify that the response was retransmitted
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Response should be retransmitted");
        let retrans_msg_info = setup.mock_transport.get_sent_message().await;
        assert!(retrans_msg_info.is_some(), "Response should have been retransmitted");
        if let Some((msg, _)) = retrans_msg_info {
            assert!(msg.is_response());
            if let Message::Response(resp) = msg {
                assert_eq!(resp.status_code(), StatusCode::Ok.as_u16());
            }
        }
    }
} 