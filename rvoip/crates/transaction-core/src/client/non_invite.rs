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
use crate::timer::{TimerSettings, TimerFactory, TimerManager};
use crate::client::data::CommonClientTransaction;
use crate::client::{ClientTransaction, ClientTransactionData};
use crate::utils;
use crate::transaction::logic::TransactionLogic;
use crate::transaction::runner::{run_transaction_loop, HasCommandSender, AsRefKey};

/// Client non-INVITE transaction (RFC 3261 Section 17.1.2)
#[derive(Debug, Clone)]
pub struct ClientNonInviteTransaction {
    data: Arc<ClientTransactionData>,
    logic: Arc<ClientNonInviteLogic>,
}

/// Holds JoinHandles and dynamic state for timers specific to Client Non-INVITE transactions.
#[derive(Default, Debug)]
struct ClientNonInviteTimerHandles {
    timer_e: Option<JoinHandle<()>>,
    current_timer_e_interval: Option<Duration>, // For backoff
    timer_f: Option<JoinHandle<()>>,
    timer_k: Option<JoinHandle<()>>,
}

/// Implements the TransactionLogic for Client Non-INVITE transactions.
#[derive(Debug, Clone, Default)]
struct ClientNonInviteLogic {
    _data_marker: std::marker::PhantomData<ClientTransactionData>,
    timer_factory: TimerFactory,
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
        // The trace log needs access to data.id, which is not passed here.
        // Logging for cancellation will be better handled in the runner or by the caller of this.
        // trace!(id=%data.id.branch, "Cancelled CNI specific timers E, F, K");
    }

    async fn on_enter_state(
        &self,
        data: &Arc<ClientTransactionData>,
        new_state: TransactionState,
        _previous_state: TransactionState,
        timer_handles: &mut ClientNonInviteTimerHandles,
        command_tx: mpsc::Sender<InternalTransactionCommand>, // This is the runner's command_tx
    ) -> Result<()> {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;

        match new_state {
            TransactionState::Trying => {
                // Send the initial request
                debug!(id=%tx_id, "ClientNonInviteLogic: Sending initial request in Trying state");
                let request_guard = data.request.lock().await;
                if let Err(e) = data.transport.send_message(
                    Message::Request(request_guard.clone()),
                    data.remote_addr
                ).await {
                    error!(id=%tx_id, error=%e, "Failed to send initial request from Trying state");
                    let _ = data.events_tx.send(TransactionEvent::TransportError { transaction_id: tx_id.clone() }).await;
                    // If send fails, command a transition to Terminated
                    let _ = command_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                    return Err(Error::transport_error(e, "Failed to send initial request"));
                }
                drop(request_guard); // Release lock

                // Start Timer E (retransmission) with initial interval T1
                let initial_interval_e = timer_config.t1;
                timer_handles.current_timer_e_interval = Some(initial_interval_e);
                let cmd_tx_clone_e = command_tx.clone();
                let tx_id_clone_e = tx_id.clone();
                timer_handles.timer_e = Some(tokio::spawn(async move {
                    tokio::time::sleep(initial_interval_e).await;
                    debug!(id=%tx_id_clone_e, "Timer E (initial) fired");
                    let _ = cmd_tx_clone_e.send(InternalTransactionCommand::Timer("E".to_string())).await;
                }));
                trace!(id=%tx_id, interval=?initial_interval_e, "Started Timer E for Trying state");

                // Start Timer F (transaction timeout)
                let interval_f = timer_config.transaction_timeout;
                let cmd_tx_clone_f = command_tx.clone();
                let tx_id_clone_f = tx_id.clone();
                timer_handles.timer_f = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval_f).await;
                    debug!(id=%tx_id_clone_f, "Timer F fired");
                    let _ = cmd_tx_clone_f.send(InternalTransactionCommand::Timer("F".to_string())).await;
                }));
                trace!(id=%tx_id, interval=?interval_f, "Started Timer F for Trying state");
            }
            TransactionState::Proceeding => {
                trace!(id=%tx_id, "Entered Proceeding state. Timers E & F continue.");
                // Timer E continues with its current backoff interval.
                // Timer F continues. No new timers are started specifically for entering Proceeding.
            }
            TransactionState::Completed => {
                // Start Timer K (wait for response retransmissions)
                let interval_k = timer_config.wait_time_k;
                let cmd_tx_clone_k = command_tx.clone(); // command_tx of the runner
                let tx_id_clone_k = tx_id.clone();
                timer_handles.timer_k = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval_k).await;
                    debug!(id=%tx_id_clone_k, "Timer K fired");
                    // Timer K's task directly commands termination
                    let _ = cmd_tx_clone_k.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                }));
                trace!(id=%tx_id, interval=?interval_k, "Started Timer K for Completed state");
            }
            TransactionState::Terminated => {
                trace!(id=%tx_id, "Entered Terminated state. Specific timers should have been cancelled by runner.");
            }
            _ => { // Initial state, or others not directly part of the main flow.
                trace!(id=%tx_id, "Entered unhandled state {:?} in on_enter_state", new_state);
            }
        }
        Ok(())
    }

    async fn process_message(
        &self,
        data: &Arc<ClientTransactionData>,
        message: Message,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>> {
        let response = match message {
            Message::Response(r) => r,
            Message::Request(_) => {
                warn!(id=%data.id, "Client transaction received a Request, ignoring.");
                return Ok(None); // No state change
            }
        };

        // Let's change our approach to avoid dereferencing 
        // We know original_request_method is a reference, get a cloned Method
        let original_request_method = {
            let req = data.request.lock().await;
            req.method().clone() // Get an owned Method, not a reference
        };

        // Validate response (e.g. CSeq method match, Via branch match)
        // Simplified validation for now
        if let Some(TypedHeader::Via(via_header_vec)) = response.header(&HeaderName::Via) {
            if let Some(via_header) = via_header_vec.0.first() {
                if let Some(branch) = via_header.branch() {
                    if branch != data.id.branch.as_str() {
                        warn!(id=%data.id, received_branch=?via_header.branch(), expected_branch=%data.id.branch, "Received response with mismatched Via branch. Ignoring.");
                        return Ok(None);
                    }
                } else {
                    warn!(id=%data.id, "Received response Via without branch. Ignoring.");
                    return Ok(None);
                }
            } else {
                warn!(id=%data.id, "Received response without Via value. Ignoring.");
                return Ok(None);
            }
        } else {
            warn!(id=%data.id, "Received response without Via header. Ignoring.");
            return Ok(None);
        }

        if let Some(TypedHeader::CSeq(cseq_header)) = response.header(&HeaderName::CSeq) {
            if cseq_header.method != original_request_method {
                warn!(id=%data.id, received_cseq_method=?cseq_header.method, expected_method=?original_request_method, "Received response with mismatched CSeq method. Ignoring.");
                return Ok(None);
            }
        } else {
            warn!(id=%data.id, "Received response without CSeq header. Ignoring.");
            return Ok(None);
        }

        let status_code = response.status();

        match current_state {
            TransactionState::Initial => {
                // Should not happen as we send request in Trying
                Ok(None)
            }
            TransactionState::Trying | TransactionState::Proceeding => {
                if status_code.is_provisional() { // 1xx
                    let _ = data.events_tx.send(TransactionEvent::ProvisionalResponse {
                        transaction_id: data.id.clone(),
                        response: response.clone(),
                        // remote_addr: data.remote_addr, // Or from response source if different
                    }).await;
                    *data.last_response.lock().await = Some(response);
                    if current_state == TransactionState::Trying {
                        return Ok(Some(TransactionState::Proceeding));
                    }
                    Ok(None)
                } else if status_code.is_success() { // 2xx
                    let _ = data.events_tx.send(TransactionEvent::SuccessResponse {
                        transaction_id: data.id.clone(),
                        response: response.clone(),
                        // remote_addr: data.remote_addr,
                    }).await;
                    *data.last_response.lock().await = Some(response);
                    Ok(Some(TransactionState::Completed))
                } else { // 3xx-6xx
                    let _ = data.events_tx.send(TransactionEvent::FailureResponse {
                        transaction_id: data.id.clone(),
                        response: response.clone(),
                        // remote_addr: data.remote_addr,
                        // is_timeout: false,
                    }).await;
                    *data.last_response.lock().await = Some(response);
                    Ok(Some(TransactionState::Completed))
                }
            }
            TransactionState::Completed => {
                // Retransmission of response from TU for some reason, or late message. Ignore for non-INVITE client.
                debug!(id=%data.id, "Received message in Completed state, ignoring.");
                Ok(None)
            }
            TransactionState::Terminated => {
                debug!(id=%data.id, "Received message in Terminated state, ignoring.");
                Ok(None)
            }
            _ => Ok(None), // Other states like Calling for INVITE
        }
    }

    async fn handle_timer(
        &self,
        data: &Arc<ClientTransactionData>,
        timer_name: &str,
        current_state: TransactionState,
        timer_handles: &mut ClientNonInviteTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;
        let timer_config = &data.timer_config;
        let self_command_tx = data.cmd_tx.clone(); // For restarting timers

        match timer_name {
            "E" => {
                timer_handles.timer_e.take(); // Timer E fired, clear its handle.
                let _ = data.events_tx.send(TransactionEvent::TimerTriggered { transaction_id: tx_id.clone(), timer: "E".to_string() }).await;

                if current_state == TransactionState::Trying || current_state == TransactionState::Proceeding {
                    debug!(id=%tx_id, "Timer E triggered, retransmitting request");
                    
                    // Retransmit the request
                    let request_guard = data.request.lock().await;
                    if let Err(e) = data.transport.send_message(
                        Message::Request(request_guard.clone()),
                        data.remote_addr
                    ).await {
                        error!(id=%tx_id, error=%e, "Failed to retransmit request on Timer E");
                        let _ = data.events_tx.send(TransactionEvent::TransportError{transaction_id: tx_id.clone()}).await;
                        return Ok(Some(TransactionState::Terminated));
                    }
                    drop(request_guard);

                    // Calculate next Timer E interval and restart Timer E
                    let current_e_interval = timer_handles.current_timer_e_interval.unwrap_or(timer_config.t1);
                    let next_interval_e = std::cmp::min(current_e_interval * 2, timer_config.t2);
                    timer_handles.current_timer_e_interval = Some(next_interval_e);
                    
                    debug!(id=%tx_id, "Restarting Timer E with interval {:?}", next_interval_e);
                    let cmd_tx_clone_e = self_command_tx.clone();
                    let tx_id_clone_e = tx_id.clone();
                    timer_handles.timer_e = Some(tokio::spawn(async move {
                        tokio::time::sleep(next_interval_e).await;
                        debug!(id=%tx_id_clone_e, "Timer E (restarted) fired");
                        let _ = cmd_tx_clone_e.send(InternalTransactionCommand::Timer("E".to_string())).await;
                    }));
                } else {
                    trace!(id=%tx_id, state=?current_state, "Timer E fired in invalid state, ignoring");
                }
            }
            "F" => {
                timer_handles.timer_f.take();
                let _ = data.events_tx.send(TransactionEvent::TimerTriggered { transaction_id: tx_id.clone(), timer: "F".to_string() }).await;
                if current_state == TransactionState::Trying || current_state == TransactionState::Proceeding {
                    warn!(id=%tx_id, "Timer F (Timeout) fired in state {:?}", current_state);
                    let _ = data.events_tx.send(TransactionEvent::TransactionTimeout { transaction_id: tx_id.clone() }).await;
                    return Ok(Some(TransactionState::Terminated));
                } else {
                    trace!(id=%tx_id, state=?current_state, "Timer F fired in invalid state, ignoring");
                }
            }
            "K" => {
                // Timer K's task itself sends TransitionTo(Terminated).
                // This handler is mainly for logging or if K's task only sent Timer("K").
                // Given the current on_enter_state for Completed, this specific arm for "K"
                // in handle_timer might be redundant if K's task already commanded termination.
                // However, if K's task *only* sends Timer("K"), this logic is essential.
                timer_handles.timer_k.take();
                let _ = data.events_tx.send(TransactionEvent::TimerTriggered { transaction_id: tx_id.clone(), timer: "K".to_string() }).await;
                if current_state == TransactionState::Completed {
                    debug!(id=%tx_id, "Timer K fired in Completed state, logic confirms termination (redundant if task already commanded)");
                    return Ok(Some(TransactionState::Terminated)); // Ensure termination
                } else {
                     trace!(id=%tx_id, state=?current_state, "Timer K fired in invalid state, ignoring");
                }
            }
            _ => {
                warn!(id=%tx_id, timer_name=%timer_name, "Unknown timer triggered for ClientNonInvite");
            }
        }
        Ok(None)
    }
}

impl ClientNonInviteTransaction {
    /// Create a new client non-INVITE transaction.
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
    
    // OLD start_event_loop IS REMOVED
}

impl ClientTransaction for ClientNonInviteTransaction {
    fn initiate(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        let kind = self.kind(); // Get kind for the error message
        
        Box::pin(async move {
            let current_state = data.state.get();
            
            if current_state != TransactionState::Initial {
                // Corrected Error::invalid_state_transition call
                return Err(Error::invalid_state_transition(
                    kind, // Pass the correct kind
                    current_state,
                    TransactionState::Trying,
                    Some(data.id.clone()), // Pass Option<TransactionKey>
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

    // Implement the missing original_request method
    fn original_request(&self) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + '_>> {
        let request_arc = self.data.request.clone();
        Box::pin(async move {
            let req = request_arc.lock().await;
            Some(req.clone()) // Clone the request out of the Mutex guard
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