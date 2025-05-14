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
use crate::client::data::CommonClientTransaction;
use crate::client::{ClientTransaction, ClientTransactionData};
use crate::utils;
use crate::transaction::logic::TransactionLogic;
use crate::transaction::runner::{run_transaction_loop, HasCommandSender, AsRefKey};
use crate::transaction::timer_utils;
use crate::transaction::validators;
use crate::transaction::common_logic;

/// Client UPDATE transaction (RFC 3311)
/// UPDATE follows the non-INVITE transaction model but is specialized for updating 
/// session characteristics of an existing dialog without requiring a new INVITE
#[derive(Debug, Clone)]
pub struct ClientUpdateTransaction {
    data: Arc<ClientTransactionData>,
    logic: Arc<ClientUpdateLogic>,
}

/// Holds JoinHandles and dynamic state for timers specific to Client UPDATE transactions.
#[derive(Default, Debug)]
struct ClientUpdateTimerHandles {
    timer_e: Option<JoinHandle<()>>,
    current_timer_e_interval: Option<Duration>, // For backoff
    timer_f: Option<JoinHandle<()>>,
    timer_k: Option<JoinHandle<()>>,
}

/// Implements the TransactionLogic for Client UPDATE transactions.
#[derive(Debug, Clone, Default)]
struct ClientUpdateLogic {
    _data_marker: std::marker::PhantomData<ClientTransactionData>,
    timer_factory: TimerFactory,
}

impl ClientUpdateLogic {
    // Helper method to start Timer E (retransmission timer) using timer utils
    async fn start_timer_e(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientUpdateTimerHandles,
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
    
    // Helper method to start Timer F (transaction timeout) using timer utils
    async fn start_timer_f(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_handles: &mut ClientUpdateTimerHandles,
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
        timer_handles: &mut ClientUpdateTimerHandles,
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
        timer_handles: &mut ClientUpdateTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<()> {
        let tx_id = &data.id;
        
        // Send the initial request
        debug!(id=%tx_id, "ClientUpdateLogic: Sending initial UPDATE request in Trying state");
        let request_guard = data.request.lock().await;
        if let Err(e) = data.transport.send_message(
            Message::Request(request_guard.clone()),
            data.remote_addr
        ).await {
            error!(id=%tx_id, error=%e, "Failed to send initial UPDATE request from Trying state");
            common_logic::send_transport_error_event(tx_id, &data.events_tx).await;
            // If send fails, command a transition to Terminated
            let _ = command_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
            return Err(Error::transport_error(e, "Failed to send initial UPDATE request"));
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
        timer_handles: &mut ClientUpdateTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        
        match current_state {
            TransactionState::Trying | TransactionState::Proceeding => {
                debug!(id=%tx_id, "Timer E triggered, retransmitting UPDATE request");
                
                // Retransmit the request
                let request_guard = data.request.lock().await;
                if let Err(e) = data.transport.send_message(
                    Message::Request(request_guard.clone()),
                    data.remote_addr
                ).await {
                    error!(id=%tx_id, error=%e, "Failed to retransmit UPDATE request");
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

    // Process a SIP response using common logic
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
        
        // Use the common_logic handler which works for non-INVITE transactions
        let new_state = common_logic::handle_response_by_status(
            tx_id, 
            response.clone(), 
            current_state, 
            &data.events_tx,
            false // UPDATE follows non-INVITE pattern
        ).await;
        
        Ok(new_state)
    }
}

#[async_trait::async_trait]
impl TransactionLogic<ClientTransactionData, ClientUpdateTimerHandles> for ClientUpdateLogic {
    fn kind(&self) -> TransactionKind {
        TransactionKind::UpdateClient
    }

    fn initial_state(&self) -> TransactionState {
        TransactionState::Initial
    }

    fn timer_settings<'a>(data: &'a Arc<ClientTransactionData>) -> &'a TimerSettings {
        &data.timer_config
    }

    fn cancel_all_specific_timers(&self, timer_handles: &mut ClientUpdateTimerHandles) {
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
        timer_handles: &mut ClientUpdateTimerHandles,
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
        timer_handles: &mut ClientUpdateTimerHandles,
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
                warn!(id=%tx_id, timer_name=%timer_name, "Unknown timer triggered for ClientUpdate");
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
                
                // Use our helper for response processing
                self.process_response(data, response, current_state).await
            },
            Err(e) => {
                warn!(id=%tx_id, error=%e, "Received non-response message");
                Ok(None)
            }
        }
    }
}

impl ClientUpdateTransaction {
    /// Create a new client UPDATE transaction.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config_override: Option<TimerSettings>,
    ) -> Result<Self> {
        if request.method() != Method::Update {
            return Err(Error::Other("Request must be UPDATE for UPDATE client transaction".to_string()));
        }

        let timer_config = timer_config_override.unwrap_or_default();
        let (cmd_tx, local_cmd_rx) = mpsc::channel(32);

        let data = Arc::new(ClientTransactionData {
            id: id.clone(),
            state: Arc::new(AtomicTransactionState::new(TransactionState::Initial)),
            request: Arc::new(Mutex::new(request.clone())),
            last_response: Arc::new(Mutex::new(None)),
            remote_addr,
            transport,
            events_tx,
            cmd_tx: cmd_tx.clone(),
            event_loop_handle: Arc::new(Mutex::new(None)),
            timer_config: timer_config.clone(),
        });

        let logic = Arc::new(ClientUpdateLogic {
            _data_marker: std::marker::PhantomData,
            timer_factory: TimerFactory::new(Some(timer_config), Arc::new(TimerManager::new(None))),
        });

        let data_for_runner = data.clone();
        let logic_for_runner = logic.clone();

        // Spawn the generic event loop runner
        let event_loop_handle = tokio::spawn(async move {
            run_transaction_loop(data_for_runner, logic_for_runner, local_cmd_rx).await;
        });

        // Store the handle for cleanup
        if let Ok(mut handle_guard) = data.event_loop_handle.try_lock() {
            *handle_guard = Some(event_loop_handle);
        }
        
        Ok(Self { data, logic })
    }
}

impl ClientTransaction for ClientUpdateTransaction {
    fn initiate(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        let kind = self.kind(); // Get kind for the error message
        
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

    fn original_request(&self) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + '_>> {
        let request_arc = self.data.request.clone();
        Box::pin(async move {
            let req = request_arc.lock().await;
            Some(req.clone()) // Clone the request out of the Mutex guard
        })
    }
}

impl Transaction for ClientUpdateTransaction {
    fn id(&self) -> &TransactionKey {
        &self.data.id
    }

    fn kind(&self) -> TransactionKind {
        TransactionKind::UpdateClient
    }

    fn state(&self) -> TransactionState {
        self.data.state.get()
    }
    
    fn remote_addr(&self) -> SocketAddr {
        self.data.remote_addr
    }
    
    fn matches(&self, message: &Message) -> bool {
        // For a client transaction, it matches responses based on:
        // 1. Topmost Via header's branch parameter matching the transaction ID's branch.
        // 2. CSeq method matching the original request's CSeq method.
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

        // Check CSeq method
        let original_request_method = self.data.id.method().clone();
        if let Some(TypedHeader::CSeq(cseq_header)) = response.header(&HeaderName::CSeq) {
            if cseq_header.method != original_request_method {
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

impl TransactionAsync for ClientUpdateTransaction {
    fn process_event<'a>(
        &'a self,
        event_type: &'a str,
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

impl CommonClientTransaction for ClientUpdateTransaction {
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
    use tokio::time::timeout as TokioTimeout;
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use rvoip_sip_core::types::status::StatusCode;
    use rvoip_sip_core::Response as SipCoreResponse;

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
            self.message_sent_notifier.notify_one(); // Notify that a message was "sent"
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
        transaction: ClientUpdateTransaction,
        mock_transport: Arc<UnitTestMockTransport>,
        tu_events_rx: mpsc::Receiver<TransactionEvent>,
    }

    async fn setup_test_environment(target_uri_str: &str) -> TestSetup {
        let local_addr = "127.0.0.1:5090";
        let mock_transport = Arc::new(UnitTestMockTransport::new(local_addr));
        let (tu_events_tx, tu_events_rx) = mpsc::channel(100);

        let req_uri = Uri::from_str(target_uri_str).unwrap();
        let builder = SimpleRequestBuilder::new(Method::Update, &req_uri.to_string())
            .expect("Failed to create SimpleRequestBuilder")
            .from("Alice", "sip:test@test.com", Some("fromtag"))
            .to("Bob", "sip:bob@target.com", Some("totag"))
            .call_id("callid-update-test")
            .cseq(1);
        
        let via_branch = format!("z9hG4bK.{}", uuid::Uuid::new_v4().as_simple());
        let builder = builder.via(mock_transport.local_addr.to_string().as_str(), "UDP", Some(&via_branch));

        let request = builder.build();
        
        let remote_addr = SocketAddr::from_str("127.0.0.1:5070").unwrap();
        let tx_key = TransactionKey::from_request(&request).expect("Failed to create tx key from request");

        let settings = TimerSettings {
            t1: Duration::from_millis(50),
            transaction_timeout: Duration::from_millis(200),
            wait_time_k: Duration::from_millis(100),
            ..Default::default()
        };

        let transaction = ClientUpdateTransaction::new(
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
        SimpleResponseBuilder::response_from_request(
            original_request,
            status_code,
            Some(status_code.reason_phrase())
        ).build()
    }

    #[tokio::test]
    async fn test_update_client_creation_and_initial_state() {
        let setup = setup_test_environment("sip:bob@target.com").await;
        assert_eq!(setup.transaction.state(), TransactionState::Initial);
        assert!(setup.transaction.data.event_loop_handle.lock().await.is_some());
    }

    #[tokio::test]
    async fn test_update_client_initiate_sends_request() {
        let mut setup = setup_test_environment("sip:bob@target.com").await;
        
        setup.transaction.initiate().await.expect("initiate should succeed");

        setup.mock_transport.wait_for_message_sent(Duration::from_millis(100)).await.expect("Message should be sent quickly");

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(setup.transaction.state(), TransactionState::Trying, "State should be Trying after initiate");

        let sent_msg_info = setup.mock_transport.get_sent_message().await;
        assert!(sent_msg_info.is_some(), "Request should have been sent");
        if let Some((msg, dest)) = sent_msg_info {
            assert!(msg.is_request());
            assert_eq!(msg.method(), Some(Method::Update));
            assert_eq!(dest, setup.transaction.remote_addr());
        }
    }

    #[tokio::test]
    async fn test_update_client_success_response() {
        let mut setup = setup_test_environment("sip:bob@target.com").await;
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

        // Success response should lead to Completed state and then Terminated after Timer K
        let mut success_response_received = false;
        let mut trying_to_completed_received = false;

        // Collect all events until we get terminated or timeout
        for _ in 0..5 {
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
                    }
                },
                Ok(Some(TransactionEvent::TimerTriggered { .. })) => {
                    // Timer events can happen, ignore them
                    continue;
                },
                Ok(Some(_)) => {
                    // Other events can happen, ignore them
                    continue;
                },
                Ok(None) => panic!("Event channel closed"),
                Err(_) => {
                    // If we timed out but already got the necessary events, we're good
                    if success_response_received && trying_to_completed_received {
                        break;
                    }
                    continue;
                }
            }
            
            // If we got all the necessary events, we can stop waiting
            if success_response_received && trying_to_completed_received {
                break;
            }
        }

        // Check that we got all the expected events
        assert!(success_response_received, "SuccessResponse event not received");
        assert!(trying_to_completed_received, "StateChanged Trying->Completed event not received");
        
        // Wait for Timer K to fire and transition to Terminated state
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(setup.transaction.state(), TransactionState::Terminated, "State should be Terminated after Timer K");
    }
} 