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
use rvoip_sip_core::builder::SimpleResponseBuilder;
use rvoip_sip_core::builder::ContentLengthBuilderExt;
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

/// Server CANCEL transaction (RFC 3261 Section 9.2)
/// CANCEL is a non-INVITE transaction from the server perspective, and follows
/// the same state machine as other non-INVITE transactions.
#[derive(Debug, Clone)]
pub struct ServerCancelTransaction {
    data: Arc<ServerTransactionData>,
    logic: Arc<ServerCancelLogic>,
    /// Reference to the original INVITE transaction that is being cancelled
    target_invite_tx_id: Option<TransactionKey>,
}

/// Holds JoinHandles and dynamic state for timers specific to Server CANCEL transactions.
/// CANCEL uses Timer J (same as non-INVITE transactions)
#[derive(Default, Debug)]
struct ServerCancelTimerHandles {
    timer_j: Option<JoinHandle<()>>,
}

/// Implements the TransactionLogic for Server CANCEL transactions.
#[derive(Debug, Clone, Default)]
struct ServerCancelLogic {
    _data_marker: std::marker::PhantomData<ServerTransactionData>,
    timer_factory: TimerFactory,
}

impl ServerCancelLogic {
    // Helper method to start Timer J (transaction completion timer)
    async fn start_timer_j(
        &self,
        data: &Arc<ServerTransactionData>,
        timer_handles: &mut ServerCancelTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        // Timer J (wait for retransmissions) duration
        let interval_j = timer_config.wait_time_k; // Same duration as client Timer K
        
        // Use timer_utils to start a timer with transition to Terminated state
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
    
    // Helper to send an automatic 200 OK response to a CANCEL request
    async fn send_ok_response(
        &self,
        data: &Arc<ServerTransactionData>,
    ) -> Result<()> {
        let tx_id = &data.id;
        // Get the original request to derive the response
        let request_guard = data.request.lock().await;
        
        // Create a 200 OK response to the CANCEL
        let builder = SimpleResponseBuilder::response_from_request(
            &request_guard,
            StatusCode::Ok,
            Some("OK"),
        );
        drop(request_guard); // Release lock
        
        let response = builder.build();
        
        // Store the response
        {
            let mut last_response = data.last_response.lock().await;
            *last_response = Some(response.clone());
        }
        
        // Send the response
        debug!(id=%tx_id, "Sending 200 OK response to CANCEL");
        if let Err(e) = data.transport.send_message(
            Message::Response(response.clone()),
            data.remote_addr
        ).await {
            error!(id=%tx_id, error=%e, "Failed to send 200 OK response to CANCEL");
            common_logic::send_transport_error_event(tx_id, &data.events_tx).await;
            return Err(Error::transport_error(e, "Failed to send 200 OK response to CANCEL"));
        }
        
        Ok(())
    }
}

#[async_trait::async_trait]
impl TransactionLogic<ServerTransactionData, ServerCancelTimerHandles> for ServerCancelLogic {
    fn kind(&self) -> TransactionKind {
        TransactionKind::CancelServer
    }

    fn initial_state(&self) -> TransactionState {
        TransactionState::Initial
    }

    fn timer_settings<'a>(data: &'a Arc<ServerTransactionData>) -> &'a TimerSettings {
        &data.timer_config
    }

    fn cancel_all_specific_timers(&self, timer_handles: &mut ServerCancelTimerHandles) {
        if let Some(handle) = timer_handles.timer_j.take() {
            handle.abort();
        }
    }

    async fn on_enter_state(
        &self,
        data: &Arc<ServerTransactionData>,
        new_state: TransactionState,
        previous_state: TransactionState,
        timer_handles: &mut ServerCancelTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>, 
    ) -> Result<()> {
        let tx_id = &data.id;

        match new_state {
            TransactionState::Trying => {
                // Send the event to transaction user first
                let request_guard = data.request.lock().await;
                common_logic::send_state_changed_event(
                    tx_id,
                    previous_state,
                    new_state,
                    &data.events_tx
                ).await;
                
                // Verify it's a CANCEL request
                if request_guard.method() != Method::Cancel {
                    error!(id=%tx_id, method=?request_guard.method(), "Non-CANCEL request in ServerCancelTransaction");
                    return Err(Error::Other("Non-CANCEL request in ServerCancelTransaction".to_string()));
                }
                
                // Notify transaction user about the CANCEL
                let _ = data.events_tx.send(TransactionEvent::NewRequest {
                    transaction_id: tx_id.clone(),
                    request: request_guard.clone(),
                    source: data.remote_addr,
                }).await;
                
                drop(request_guard);
                
                // Always respond with 200 OK automatically to CANCEL
                // as per RFC 3261 Section 9.2
                self.send_ok_response(data).await?;
                
                // Auto-transition to Completed state since we immediately send a final response
                let _ = command_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Completed)).await;
            },
            TransactionState::Completed => {
                // Start Timer J (wait for retransmissions)
                self.start_timer_j(data, timer_handles, command_tx).await;
            },
            TransactionState::Terminated => {
                trace!(id=%tx_id, "Entered Terminated state. Specific timers should have been cancelled by runner.");
                
                // Notify transaction user about termination
                common_logic::send_transaction_terminated_event(tx_id, &data.events_tx).await;
                
                // Unregister from timer manager when terminated
                let timer_manager = self.timer_factory.timer_manager();
                timer_utils::unregister_transaction(&timer_manager, tx_id).await;
            },
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
        timer_handles: &mut ServerCancelTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        
        if timer_name == "J" {
            // Clear the timer handle since it fired
            timer_handles.timer_j.take();
        }
        
        // Send timer triggered event
        common_logic::send_timer_triggered_event(tx_id, timer_name, &data.events_tx).await;
        
        match timer_name {
            "J" => {
                if current_state == TransactionState::Completed {
                    debug!(id=%tx_id, "Timer J fired in Completed state, transitioning to Terminated");
                    // Timer J automatically transitions to Terminated via start_timer_with_transition
                    Ok(None)
                } else {
                    trace!(id=%tx_id, state=?current_state, "Timer J fired in invalid state, ignoring");
                    Ok(None)
                }
            },
            _ => {
                warn!(id=%tx_id, timer_name=%timer_name, "Unknown timer triggered for ServerCancel");
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
        
        // Handle retransmitted CANCEL requests
        if let Message::Request(request) = message {
            if request.method() != Method::Cancel {
                warn!(id=%tx_id, method=?request.method(), "Received non-CANCEL request in ServerCancelTransaction");
                return Ok(None);
            }
            
            // If in Completed state, resend the 200 OK response
            if current_state == TransactionState::Completed {
                debug!(id=%tx_id, "Received retransmitted CANCEL request in Completed state, resending 200 OK");
                
                // Get the last response (should be 200 OK)
                let response_opt = {
                    let response_guard = data.last_response.lock().await;
                    response_guard.clone()
                };
                
                if let Some(response) = response_opt {
                    // Resend the 200 OK
                    if let Err(e) = data.transport.send_message(
                        Message::Response(response),
                        data.remote_addr
                    ).await {
                        error!(id=%tx_id, error=%e, "Failed to resend 200 OK response");
                        common_logic::send_transport_error_event(tx_id, &data.events_tx).await;
                    }
                } else {
                    error!(id=%tx_id, "No response to resend for retransmitted CANCEL");
                }
            }
        } else {
            warn!(id=%tx_id, "Received response in ServerCancelTransaction, ignoring");
        }
        
        Ok(None)
    }
}

impl ServerCancelTransaction {
    /// Create a new server-side CANCEL transaction from an incoming CANCEL request.
    /// 
    /// # Arguments
    /// * `request` - The incoming CANCEL request
    /// * `remote_addr` - The address the request came from
    /// * `transport` - The transport to use for sending responses
    /// * `events_tx` - Channel to send events to the transaction user
    /// * `target_invite_tx_id` - Optional transaction ID of the INVITE being cancelled
    /// * `timer_config_override` - Optional custom timer settings
    pub fn new(
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        target_invite_tx_id: Option<TransactionKey>,
        timer_config_override: Option<TimerSettings>,
    ) -> Result<Self> {
        if request.method() != Method::Cancel {
            return Err(Error::Other("Cannot create a CANCEL transaction from a non-CANCEL request".to_string()));
        }
        
        // Generate a transaction key from the request
        let transaction_id = TransactionKey::from_request(&request)
            .map(|mut key| {
                // Set the is_server flag to true since this is a server transaction
                key.is_server = true;
                key
            })
            .ok_or_else(|| Error::Other("Failed to generate transaction key from CANCEL request".to_string()))?;
            
        let timer_config = timer_config_override.unwrap_or_default();
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        
        // Create transaction data
        let data = Arc::new(ServerTransactionData {
            id: transaction_id.clone(),
            state: Arc::new(AtomicTransactionState::new(TransactionState::Initial)),
            request: Arc::new(Mutex::new(request.clone())),
            last_response: Arc::new(Mutex::new(None)),
            remote_addr,
            transport,
            events_tx,
            cmd_tx: cmd_tx.clone(),
            cmd_rx: Arc::new(Mutex::new(cmd_rx)),
            event_loop_handle: Arc::new(Mutex::new(None)),
            timer_config: timer_config.clone(),
        });
        
        // Create transaction logic
        let logic = Arc::new(ServerCancelLogic {
            _data_marker: std::marker::PhantomData,
            timer_factory: TimerFactory::new(Some(timer_config), Arc::new(TimerManager::new(None))),
        });
        
        // Set up and start the transaction event loop
        let data_for_runner = data.clone();
        let logic_for_runner = logic.clone();
        
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
        
        Ok(Self { 
            data, 
            logic,
            target_invite_tx_id,
        })
    }

    /// Get the transaction ID of the target INVITE transaction being cancelled
    pub fn target_invite_tx_id(&self) -> Option<&TransactionKey> {
        self.target_invite_tx_id.as_ref()
    }
}

impl ServerTransaction for ServerCancelTransaction {
    fn process_request(&self, request: Request) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        
        Box::pin(async move {
            if request.method() != Method::Cancel {
                return Err(Error::Other("Non-CANCEL request in ServerCancelTransaction".to_string()));
            }
            
            debug!(id=%data.id, method=%request.method(), "Received request");
            
            data.cmd_tx.send(InternalTransactionCommand::ProcessMessage(Message::Request(request))).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))?;
            
            Ok(())
        })
    }

    fn send_response(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        
        Box::pin(async move {
            // For CANCEL, responses are automatically sent, so this is unusual
            warn!(id=%data.id, status=%response.status(), "Manual send_response called for CANCEL transaction");
            
            // Store the response
            {
                let mut last_response = data.last_response.lock().await;
                *last_response = Some(response.clone());
            }
            
            // Send the response
            if let Err(e) = data.transport.send_message(
                Message::Response(response.clone()),
                data.remote_addr
            ).await {
                error!(id=%data.id, error=%e, "Failed to send response");
                common_logic::send_transport_error_event(&data.id, &data.events_tx).await;
                return Err(Error::transport_error(e, "Failed to send response"));
            }
            
            // CANCEL transaction automatically goes to Completed after final response
            data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Completed)).await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))?;
                
            Ok(())
        })
    }
}

impl CommonServerTransaction for ServerCancelTransaction {
    fn data(&self) -> &Arc<ServerTransactionData> {
        &self.data
    }
}

impl Transaction for ServerCancelTransaction {
    fn id(&self) -> &TransactionKey {
        &self.data.id
    }

    fn kind(&self) -> TransactionKind {
        TransactionKind::CancelServer
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

impl TransactionAsync for ServerCancelTransaction {
    fn process_event<'a>(
        &'a self,
        event_type: &'a str,
        message: Option<Message>
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event_type {
                "request" => {
                    if let Some(msg) = message {
                        if let Message::Request(req) = msg {
                            self.process_request(req).await?;
                        } else {
                            return Err(Error::Other("Expected Request message for 'request' event type".to_string()));
                        }
                    } else {
                        return Err(Error::Other("Expected Message for 'request' event type".to_string()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::str::FromStr;
    use tokio::sync::Notify;
    use tokio::time::{timeout, Duration};
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use rvoip_sip_core::types::status::StatusCode;
    
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

    // Helper to build a test CANCEL request
    fn build_cancel_request(target_uri_str: &str) -> Request {
        // Create the builder for a CANCEL request
        let builder_result = SimpleRequestBuilder::new(Method::Cancel, target_uri_str);
        let builder = match builder_result {
            Ok(builder) => builder,
            Err(e) => panic!("Failed to create request builder: {}", e),
        };
        
        // Build the request with required headers
        builder
            .from("Bob", "sip:bob@example.com", Some("a73kszlfl"))
            .to("Alice", "sip:alice@example.com", None)
            .call_id("a84b4c76e66710")
            .cseq(1)
            .via("SIP/2.0/UDP", "bob.example.com", Some("z9hG4bK776asdhds"))
            .max_forwards(70)
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }

    // Test environment setup
    struct TestSetup {
        transaction: ServerCancelTransaction,
        mock_transport: Arc<UnitTestMockTransport>,
        tu_events_rx: mpsc::Receiver<TransactionEvent>,
    }

    // Helper to create test environment
    async fn setup_test_environment(target_uri_str: &str) -> TestSetup {
        let cancel_request = build_cancel_request(target_uri_str);
        
        let mock_transport = Arc::new(UnitTestMockTransport::new("127.0.0.1:5060"));
        let (tu_events_tx, tu_events_rx) = mpsc::channel(32);
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        
        // Create the CANCEL transaction
        let transaction = ServerCancelTransaction::new(
            cancel_request,
            remote_addr,
            mock_transport.clone(),
            tu_events_tx,
            None, // No target INVITE transaction ID for this test
            None, // Use default timer settings
        ).expect("Failed to create ServerCancelTransaction");
        
        TestSetup {
            transaction,
            mock_transport,
            tu_events_rx,
        }
    }

    #[tokio::test]
    async fn test_cancel_server_creation() {
        let setup = setup_test_environment("sip:alice@example.com").await;
        
        // Check initial state
        assert_eq!(setup.transaction.state(), TransactionState::Initial);
        
        // Verify transaction kind
        assert_eq!(setup.transaction.kind(), TransactionKind::CancelServer);
        
        // Verify target invite transaction ID (none in this test)
        assert!(setup.transaction.target_invite_tx_id().is_none());
        
        // Request should be available and be a CANCEL method
        let req = setup.transaction.original_request().await.expect("Request should be available");
        assert_eq!(req.method(), Method::Cancel);
    }

    #[tokio::test]
    async fn test_cancel_server_automatic_response() {
        let mut setup = setup_test_environment("sip:alice@example.com").await;
        
        // Start the transaction by initiating a state transition to Trying 
        setup.transaction.send_command(InternalTransactionCommand::TransitionTo(TransactionState::Trying)).await
            .expect("Failed to send command");
        
        // Wait briefly for the transaction to process
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Should auto-respond with 200 OK and transition to Completed
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await
            .expect("Timed out waiting for 200 OK response");
        
        // Verify a response was sent
        let sent_message = setup.mock_transport.get_sent_message().await.expect("No response was sent");
        
        // Verify it's a 200 OK response
        match sent_message.0 {
            Message::Response(res) => {
                assert_eq!(res.status(), StatusCode::Ok);
            },
            _ => panic!("Expected Response message"),
        }
        
        // Verify state is now Completed
        assert_eq!(setup.transaction.state(), TransactionState::Completed);
        
        // Verify transaction events
        // Multiple events should have been received: 
        // 1. StateChanged from Initial to Trying
        // 2. RequestReceived
        // 3. StateChanged from Trying to Completed
        
        let mut events_received = 0;
        let mut saw_initial_to_trying = false;
        let mut saw_trying_to_completed = false;
        let mut saw_request_received = false;
        
        while let Ok(event) = setup.tu_events_rx.try_recv() {
            events_received += 1;
            match event {
                TransactionEvent::StateChanged { previous_state, new_state, .. } => {
                    if previous_state == TransactionState::Initial && new_state == TransactionState::Trying {
                        saw_initial_to_trying = true;
                    } else if previous_state == TransactionState::Trying && new_state == TransactionState::Completed {
                        saw_trying_to_completed = true;
                    }
                },
                TransactionEvent::NewRequest { .. } => {
                    saw_request_received = true;
                },
                _ => {},
            }
        }
        
        assert!(events_received >= 3, "Expected at least 3 events, got {}", events_received);
        assert!(saw_initial_to_trying, "Missing Initial->Trying state change event");
        assert!(saw_trying_to_completed, "Missing Trying->Completed state change event");
        assert!(saw_request_received, "Missing RequestReceived event");
    }

    #[tokio::test]
    async fn test_cancel_server_retransmission_handling() {
        let mut setup = setup_test_environment("sip:alice@example.com").await;
        
        // Initialize the transaction
        setup.transaction.send_command(InternalTransactionCommand::TransitionTo(TransactionState::Trying)).await
            .expect("Failed to send command");
        
        // Wait for processing and auto-response
        tokio::time::sleep(Duration::from_millis(50)).await;
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await
            .expect("Timed out waiting for initial 200 OK");
        let _ = setup.mock_transport.get_sent_message().await;
        
        // State should now be Completed
        assert_eq!(setup.transaction.state(), TransactionState::Completed);
        
        // Simulate a retransmission of the CANCEL request
        let retransmitted_request = build_cancel_request("sip:alice@example.com");
        setup.transaction.process_request(retransmitted_request).await
            .expect("Failed to process retransmitted request");
            
        // Should resend the 200 OK response
        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await
            .expect("Timed out waiting for retransmitted 200 OK");
        
        let sent_message = setup.mock_transport.get_sent_message().await.expect("No response was sent");
        
        // Verify it's a 200 OK response
        match sent_message.0 {
            Message::Response(res) => {
                assert_eq!(res.status(), StatusCode::Ok);
            },
            _ => panic!("Expected Response message"),
        }
        
        // State should still be Completed
        assert_eq!(setup.transaction.state(), TransactionState::Completed);
    }

    #[tokio::test]
    async fn test_cancel_server_timer_j() {
        // Create transaction with short Timer J
        let cancel_request = build_cancel_request("sip:alice@example.com");
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        let mock_transport = Arc::new(UnitTestMockTransport::new("127.0.0.1:5060"));
        let (tu_events_tx, mut tu_events_rx) = mpsc::channel(32);
        
        // Create custom timer settings with very short wait_time_k (used for Timer J)
        let timer_settings = TimerSettings {
            t1: Duration::from_millis(10),
            t2: Duration::from_millis(40),
            transaction_timeout: Duration::from_millis(500),
            wait_time_k: Duration::from_millis(50), // Very short Timer J
            wait_time_d: Duration::from_millis(100),
            ..Default::default()
        };
        
        let transaction = ServerCancelTransaction::new(
            cancel_request,
            remote_addr,
            mock_transport.clone(),
            tu_events_tx,
            None,
            Some(timer_settings),
        ).expect("Failed to create ServerCancelTransaction");
        
        // Initialize the transaction
        transaction.send_command(InternalTransactionCommand::TransitionTo(TransactionState::Trying)).await
            .expect("Failed to send command");
        
        // Wait for automatic 200 OK and transition to Completed
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Verify state is Completed
        assert_eq!(transaction.state(), TransactionState::Completed);
        
        // Wait for Timer J to fire (should take ~50ms)
        // Give it up to 200ms
        let mut terminated = false;
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            if transaction.state() == TransactionState::Terminated {
                terminated = true;
                break;
            }
        }
        
        assert!(terminated, "Transaction did not reach Terminated state after Timer J");
        
        // Verify we received the expected events including timer and terminated events
        let mut saw_timer_j = false;
        let mut saw_completed_to_terminated = false;
        let mut saw_transaction_terminated = false;
        
        while let Ok(event) = tu_events_rx.try_recv() {
            match event {
                TransactionEvent::TimerTriggered { timer, .. } => {
                    if timer == "J" {
                        saw_timer_j = true;
                    }
                },
                TransactionEvent::StateChanged { previous_state, new_state, .. } => {
                    if previous_state == TransactionState::Completed && new_state == TransactionState::Terminated {
                        saw_completed_to_terminated = true;
                    }
                },
                TransactionEvent::TransactionTerminated { .. } => {
                    saw_transaction_terminated = true;
                },
                _ => {},
            }
        }
        
        assert!(saw_timer_j, "Did not receive Timer J triggered event");
        assert!(saw_completed_to_terminated, "Did not receive Completed->Terminated state change event");
        assert!(saw_transaction_terminated, "Did not receive TransactionTerminated event");
    }
} 