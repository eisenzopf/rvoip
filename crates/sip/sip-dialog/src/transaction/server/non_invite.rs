use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{debug, error, trace, warn};

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::{Transport, TransportRoute};

use crate::transaction::common_logic;
use crate::transaction::error::{Error, Result};
use crate::transaction::logic::TransactionLogic;
use crate::transaction::runner::run_transaction_loop;
use crate::transaction::server::{
    CommonServerTransaction, ServerTransaction, ServerTransactionData,
};
use crate::transaction::timer::{TimerFactory, TimerManager, TimerSettings};
use crate::transaction::timer_utils;
use crate::transaction::utils;
use crate::transaction::{
    AtomicTransactionState, InternalTransactionCommand, Transaction, TransactionAsync,
    TransactionEvent, TransactionKey, TransactionKind, TransactionState,
    DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
};

/// Server non-INVITE transaction (RFC 3261 Section 17.2.2)
#[derive(Debug, Clone)]
pub struct ServerNonInviteTransaction {
    data: Arc<ServerTransactionData>,
    /// Logic instance held so the spawned transaction loop keeps the
    /// same state machine.
    #[allow(dead_code)]
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
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Timer J fired in Completed state, terminating");
                // Timer J automatically transitions to Terminated, no need to return a state
                Ok(None)
            }
            _ => {
                trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), state=?current_state, "Timer J fired in invalid state, ignoring");
                Ok(None)
            }
        }
    }

    // Process a retransmitted SIP request
    async fn process_request_retransmission(
        &self,
        data: &Arc<ServerTransactionData>,
        _request: Request,
        current_state: TransactionState,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;

        match current_state {
            TransactionState::Trying
            | TransactionState::Proceeding
            | TransactionState::Completed => {
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), state=?current_state, "Received request retransmission");

                // If in Completed state, retransmit the last response
                if current_state == TransactionState::Completed {
                    let last_response = data.last_response.lock().await;
                    if let Some(response) = &*last_response {
                        if let Err(e) = data
                            .transport
                            .send_message_via(
                                Message::Response(response.clone()),
                                data.response_route.clone(),
                            )
                            .await
                        {
                            error!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), error=%crate::transaction::safe_diagnostics::SafeOpaqueError::new(&e), "Failed to retransmit response");
                        }
                    }
                }

                // No state transition needed for request retransmission
                Ok(None)
            }
            _ => {
                // Requests in other states are ignored
                trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), state=?current_state, "Ignoring request in state {:?}", current_state);
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

    async fn send_server_response(
        &self,
        data: &Arc<ServerTransactionData>,
        mut response: Response,
        current_state: TransactionState,
        _timer_handles: &mut ServerNonInviteTimerHandles,
    ) -> Result<crate::transaction::logic::ServerResponseDisposition> {
        let is_provisional = response.status().is_provisional();
        crate::transaction::utils::stamp_response_via_with_source(&mut response, data.remote_addr);

        let mut response_guard = data.last_response.lock().await;
        if !matches!(
            current_state,
            TransactionState::Trying | TransactionState::Proceeding
        ) || response_guard
            .as_ref()
            .is_some_and(|prior| !prior.status().is_provisional())
        {
            if !is_provisional {
                data.mark_final_response_failed_before_write();
            }
            return Err(Error::Other(format!(
                "cannot send non-INVITE server response in {current_state:?} state"
            )));
        }

        if !is_provisional {
            // Retain immutable replay material before entering the conservative
            // stream-write boundary. Any error/cancellation after this point is
            // wire-unknown and must preserve Timer J ownership.
            *response_guard = Some(response.clone());
            data.mark_final_response_write_in_flight();
        }
        let write_result = data
            .transport
            .send_message_via(
                Message::Response(response.clone()),
                data.response_route.clone(),
            )
            .await;
        if let Err(error) = write_result {
            if !is_provisional {
                data.mark_final_response_failed_after_write_boundary();
            }
            return Err(Error::transport_error(error, "Failed to send response"));
        }
        if is_provisional {
            *response_guard = Some(response);
        } else {
            data.mark_final_response_wire_written();
        }
        drop(response_guard);

        let next_state = match (current_state, is_provisional) {
            (TransactionState::Trying, true) => Some(TransactionState::Proceeding),
            (TransactionState::Trying | TransactionState::Proceeding, false) => {
                Some(TransactionState::Completed)
            }
            _ => None,
        };
        Ok(crate::transaction::logic::ServerResponseDisposition {
            next_state,
            cancel_timer_100: false,
        })
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
        _previous_state: TransactionState,
        _timer_handles: &mut ServerNonInviteTimerHandles,
        _command_tx: mpsc::Sender<InternalTransactionCommand>,
    ) -> Result<()> {
        let tx_id = &data.id;

        match new_state {
            TransactionState::Trying => {
                trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Entered Trying state. No timers are started yet until a response is sent.");
            }
            TransactionState::Proceeding => {
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Entered Proceeding state after sending provisional response.");
                // No timers are started in Proceeding state for non-INVITE server transactions
            }
            TransactionState::Completed => {
                debug!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Entered Completed state after sending final response.");
                // RFC 3261 section 17.2.2: Timer J is 64*T1 for an
                // unreliable transport and zero for a reliable transport.
                // Avoid allocating a timer task for TCP/TLS/WS/WSS and move
                // directly to Terminated after the Completed event has been
                // published.
                if timer_utils::uses_unreliable_transport(
                    &data.response_route,
                    data.transport.default_transport_type(),
                ) {
                    if !data
                        .clone()
                        .schedule_compact_timer_j(data.timer_config.wait_time_j)
                        .await
                        && !data.clone().schedule_termination().await
                    {
                        data.state.set(TransactionState::Terminated);
                    }
                } else if !data.clone().schedule_termination().await {
                    data.state.set(TransactionState::Terminated);
                }
            }
            TransactionState::Terminated => {
                trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Entered Terminated state. Specific timers should have been cancelled by runner.");
                // Unregister from timer manager when terminated
                let timer_manager = self.timer_factory.timer_manager();
                timer_utils::unregister_transaction(&timer_manager, tx_id).await;
            }
            _ => {
                trace!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Entered unhandled state {:?} in on_enter_state", new_state);
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
            "J" => {
                self.handle_timer_j_trigger(data, current_state, self_command_tx)
                    .await
            }
            _ => {
                warn!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), timer_class=%crate::transaction::safe_diagnostics::SafeTimerName::new(timer_name), timer_len=timer_name.len(), "Unknown timer triggered for ServerNonInvite");
                Ok(None)
            }
        }
    }

    async fn process_message(
        &self,
        data: &Arc<ServerTransactionData>,
        message: Message,
        current_state: TransactionState,
        _timer_handles: &mut ServerNonInviteTimerHandles,
    ) -> Result<Option<TransactionState>> {
        let tx_id = &data.id;

        match message {
            Message::Request(request) => {
                self.process_request_retransmission(data, request, current_state)
                    .await
            }
            Message::Response(_) => {
                warn!(id=%crate::transaction::safe_diagnostics::SafeTransactionKey::new(&tx_id), "Server transaction received a Response, ignoring");
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
        Self::new_with_command_channel_capacity(
            id,
            request,
            remote_addr,
            transport,
            events_tx,
            timer_config_override,
            DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
        )
    }

    /// Create a new server non-INVITE transaction with a configured
    /// command-channel capacity.
    pub fn new_with_command_channel_capacity(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config_override: Option<TimerSettings>,
        command_channel_capacity: usize,
    ) -> Result<Self> {
        Self::new_with_response_route_and_command_channel_capacity(
            id,
            request,
            TransportRoute::new(remote_addr),
            transport,
            events_tx,
            timer_config_override,
            command_channel_capacity,
        )
    }

    /// Create a server non-INVITE transaction bound to the exact ingress route.
    pub fn new_with_response_route_and_command_channel_capacity(
        id: TransactionKey,
        request: Request,
        response_route: TransportRoute,
        transport: Arc<dyn Transport>,
        events_tx: impl Into<crate::transaction::event_sender::TransactionEventSender>,
        timer_config_override: Option<TimerSettings>,
        command_channel_capacity: usize,
    ) -> Result<Self> {
        let timer_manager = Arc::new(TimerManager::new(timer_config_override.clone()));
        Self::new_with_response_route_command_capacity_and_timer_manager(
            id,
            request,
            response_route,
            transport,
            events_tx,
            timer_config_override,
            command_channel_capacity,
            timer_manager,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_response_route_command_capacity_and_timer_manager(
        id: TransactionKey,
        request: Request,
        response_route: TransportRoute,
        transport: Arc<dyn Transport>,
        events_tx: impl Into<crate::transaction::event_sender::TransactionEventSender>,
        timer_config_override: Option<TimerSettings>,
        command_channel_capacity: usize,
        timer_manager: Arc<TimerManager>,
    ) -> Result<Self> {
        if request.method() == Method::Invite || request.method() == Method::Ack {
            return Err(Error::Other(
                "Request must not be INVITE or ACK for non-INVITE server transaction".to_string(),
            ));
        }

        let timer_config = timer_config_override.unwrap_or_default();
        let (cmd_tx, local_cmd_rx) = mpsc::channel(command_channel_capacity.max(1));
        let remote_addr = response_route.destination;

        let data = Arc::new(ServerTransactionData {
            id: id.clone(),
            state: Arc::new(AtomicTransactionState::new(TransactionState::Trying)),
            lifecycle: Arc::new(std::sync::atomic::AtomicU8::new(0)), // TransactionLifecycle::Active
            request: Arc::new(request.clone()),
            last_response: Arc::new(Mutex::new(None)),
            final_response_wire_written: std::sync::atomic::AtomicBool::new(false),
            final_response_supervision_state: std::sync::atomic::AtomicU64::new(0),
            final_response_supervision_notify: tokio::sync::Notify::new(),
            remote_addr,
            response_route,
            transport,
            events_tx: events_tx.into(),
            cmd_tx: cmd_tx.clone(),
            event_loop_handle: Arc::new(Mutex::new(None)),
            termination_cleanup_tx: std::sync::OnceLock::new(),
            lifecycle_scheduler: std::sync::OnceLock::new(),
            compact_retention_reservation: std::sync::OnceLock::new(),
            transaction_admission_owner: std::sync::OnceLock::new(),
            manager_admission_lifecycle: std::sync::OnceLock::new(),
            terminal_event_publication:
                crate::transaction::event_sender::TerminalEventPublication::new(),
            timer_config: timer_config.clone(),
        });

        let logic = Arc::new(ServerNonInviteLogic {
            _data_marker: std::marker::PhantomData,
            timer_factory: TimerFactory::new(Some(timer_config), timer_manager),
        });

        let data_for_runner = data.clone();
        let logic_for_runner = logic.clone();

        // The receiver has exactly one owner: the transaction runner. Keeping
        // it out of `ServerTransactionData` avoids a second mutex/Arc and the
        // dummy replacement receiver that used to be allocated per server
        // transaction.
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
        utils::transaction_key_from_message(message)
            .map(|key| key == self.data.id)
            .unwrap_or(false)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl TransactionAsync for ServerNonInviteTransaction {
    fn process_event<'a>(
        &'a self,
        event_type: &'a str,
        message: Option<Message>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event_type {
                "request" => {
                    if let Some(Message::Request(request)) = message {
                        self.process_request(request).await
                    } else {
                        Err(Error::Other("Expected Request message".to_string()))
                    }
                }
                "response" => {
                    if let Some(Message::Response(response)) = message {
                        self.send_response(response).await
                    } else {
                        Err(Error::Other("Expected Response message".to_string()))
                    }
                }
                _ => Err(Error::Other(format!(
                    "Unhandled event type: {}",
                    event_type
                ))),
            }
        })
    }

    fn send_command<'a>(
        &'a self,
        cmd: InternalTransactionCommand,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let data = self.data.clone();

        Box::pin(async move {
            data.cmd_tx
                .send(cmd)
                .await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))
        })
    }

    fn original_request<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + 'a>> {
        Box::pin(async move { Some((*self.data.request).clone()) })
    }

    fn last_response<'a>(&'a self) -> Pin<Box<dyn Future<Output = Option<Response>> + Send + 'a>> {
        Box::pin(async move { self.data.last_response.lock().await.clone() })
    }
}

impl ServerTransaction for ServerNonInviteTransaction {
    fn process_request(
        &self,
        request: Request,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();

        Box::pin(async move {
            data.cmd_tx
                .send(InternalTransactionCommand::ProcessMessage(
                    Message::Request(request),
                ))
                .await
                .map_err(|e| Error::Other(format!("Failed to send command: {}", e)))?;

            Ok(())
        })
    }

    fn send_response(
        &self,
        response: Response,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();

        Box::pin(async move {
            let final_response = !response.status().is_provisional();
            if final_response && !data.begin_final_response_supervision() {
                return Err(Error::Other(
                    "final server response is already supervised".to_string(),
                ));
            }
            let operation = match crate::transaction::server::SupervisedServerResponse::new(
                Arc::clone(&data),
                response,
            ) {
                Ok(operation) => operation,
                Err(error) => {
                    if final_response {
                        data.mark_final_response_failed_before_write();
                    }
                    return Err(error);
                }
            };
            if let Err(error) = data
                .cmd_tx
                .send(InternalTransactionCommand::SupervisedServerResponse(
                    Arc::clone(&operation),
                ))
                .await
            {
                if final_response {
                    data.mark_final_response_failed_before_write();
                }
                return Err(Error::Other(format!(
                    "Failed to enqueue supervised server response: {error}"
                )));
            }
            operation.wait().await
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
        // `Arc<Request>` — no lock needed.
        Some((*self.data.request).clone())
    }

    fn original_request_matches_dialog(
        &self,
        call_id: &str,
        from_tag: &str,
        to_tag: Option<&str>,
    ) -> bool {
        let request = &self.data.request;
        let Some(req_call_id) = request.call_id() else {
            return false;
        };
        if req_call_id.value() != call_id {
            return false;
        }
        let Some(req_from) = request.from_tag() else {
            return false;
        };
        if req_from != from_tag {
            return false;
        }
        match (request.to_tag(), to_tag) {
            (Some(req_to), Some(ack_to)) => req_to == ack_to,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transaction::server::FinalResponseCompletionDisposition;
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use rvoip_sip_core::types::status::StatusCode;
    use std::collections::VecDeque;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::sync::Notify;
    use tokio::time::timeout as TokioTimeout;

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

        async fn wait_for_message_sent(
            &self,
            duration: Duration,
        ) -> std::result::Result<(), tokio::time::error::Elapsed> {
            TokioTimeout(duration, self.message_sent_notifier.notified()).await
        }
    }

    #[async_trait::async_trait]
    impl Transport for UnitTestMockTransport {
        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        async fn send_message(
            &self,
            message: Message,
            destination: SocketAddr,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            self.sent_messages
                .lock()
                .await
                .push_back((message.clone(), destination));
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

    #[derive(Debug)]
    struct FirstSendBlockingTransport {
        sent_messages: Arc<Mutex<VecDeque<(Message, SocketAddr)>>>,
        local_addr: SocketAddr,
        block_next_send: AtomicBool,
        fail_after_release: bool,
        send_entered: Notify,
        release_send: Notify,
    }

    impl FirstSendBlockingTransport {
        fn new(local_addr: SocketAddr) -> Arc<Self> {
            Self::with_result(local_addr, false)
        }

        fn failing(local_addr: SocketAddr) -> Arc<Self> {
            Self::with_result(local_addr, true)
        }

        fn with_result(local_addr: SocketAddr, fail_after_release: bool) -> Arc<Self> {
            Arc::new(Self {
                sent_messages: Arc::new(Mutex::new(VecDeque::new())),
                local_addr,
                block_next_send: AtomicBool::new(true),
                fail_after_release,
                send_entered: Notify::new(),
                release_send: Notify::new(),
            })
        }
    }

    #[async_trait::async_trait]
    impl Transport for FirstSendBlockingTransport {
        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok(self.local_addr)
        }

        async fn send_message(
            &self,
            message: Message,
            destination: SocketAddr,
        ) -> std::result::Result<(), rvoip_sip_transport::Error> {
            if self.block_next_send.swap(false, Ordering::AcqRel) {
                self.send_entered.notify_one();
                self.release_send.notified().await;
            }
            if self.fail_after_release {
                return Err(rvoip_sip_transport::Error::ProtocolError(
                    "injected send failure after transport entry".to_string(),
                ));
            }
            self.sent_messages
                .lock()
                .await
                .push_back((message, destination));
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

    async fn setup_test_environment(request_method: Method, target_uri_str: &str) -> TestSetup {
        setup_test_environment_with_transport(request_method, target_uri_str, None).await
    }

    async fn setup_test_environment_with_transport(
        request_method: Method,
        target_uri_str: &str,
        transport_type: Option<rvoip_sip_transport::transport::TransportType>,
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

        let tx_key =
            TransactionKey::from_request(&request).expect("Failed to create tx key from request");

        let settings = TimerSettings {
            t1: Duration::from_millis(50),
            transaction_timeout: Duration::from_millis(200),
            wait_time_j: if transport_type.is_some() {
                Duration::from_secs(5)
            } else {
                Duration::from_millis(100)
            },
            ..Default::default()
        };

        let transaction = match transport_type {
            Some(transport_type) => {
                ServerNonInviteTransaction::new_with_response_route_and_command_channel_capacity(
                    tx_key,
                    request,
                    TransportRoute::new(remote_addr).with_transport_type(transport_type),
                    mock_transport.clone() as Arc<dyn Transport>,
                    tu_events_tx,
                    Some(settings),
                    DEFAULT_TRANSACTION_COMMAND_CHANNEL_CAPACITY,
                )
                .unwrap()
            }
            None => ServerNonInviteTransaction::new(
                tx_key,
                request,
                remote_addr,
                mock_transport.clone() as Arc<dyn Transport>,
                tu_events_tx,
                Some(settings),
            )
            .unwrap(),
        };

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
            Some(status_code.reason_phrase()),
        )
        .build()
    }

    #[tokio::test]
    async fn test_server_noninvite_creation() {
        let setup = setup_test_environment(Method::Register, "sip:registrar.example.com").await;
        assert_eq!(setup.transaction.state(), TransactionState::Trying);
        assert!(setup
            .transaction
            .data
            .event_loop_handle
            .lock()
            .await
            .is_some());
    }

    #[tokio::test]
    async fn final_response_outcome_waits_through_write_in_flight() {
        let setup = setup_test_environment(Method::Bye, "sip:bob@example.com").await;
        assert!(setup.transaction.data.begin_final_response_supervision());
        setup.transaction.data.mark_final_response_write_in_flight();

        assert!(TokioTimeout(
            Duration::from_millis(10),
            setup.transaction.data.await_final_response_wire_outcome()
        )
        .await
        .is_err());
        assert!(setup
            .transaction
            .data
            .final_response_may_have_reached_wire());

        setup
            .transaction
            .data
            .mark_final_response_failed_after_write_boundary();
        assert!(
            setup
                .transaction
                .data
                .await_final_response_wire_outcome()
                .await
        );
    }

    #[tokio::test]
    async fn final_response_failed_before_write_is_not_wire_ambiguous() {
        let setup = setup_test_environment(Method::Bye, "sip:bob@example.com").await;
        assert!(setup.transaction.data.begin_final_response_supervision());
        setup
            .transaction
            .data
            .mark_final_response_failed_before_write();

        assert!(
            !setup
                .transaction
                .data
                .await_final_response_wire_outcome()
                .await
        );
        assert!(!setup
            .transaction
            .data
            .final_response_may_have_reached_wire());
        assert!(
            setup.transaction.data.begin_final_response_supervision(),
            "a response proven not to have crossed the transport boundary must be retryable"
        );
    }

    #[tokio::test]
    async fn stale_failed_before_write_guard_cannot_poison_retry_generation() {
        let setup = setup_test_environment(Method::Info, "sip:bob@example.com").await;
        let original_request = (*setup.transaction.data.request).clone();
        let response = build_simple_response(StatusCode::Ok, &original_request);

        assert!(setup.transaction.data.begin_final_response_supervision());
        let first_generation = setup
            .transaction
            .data
            .current_final_response_supervision_generation()
            .expect("first supervision generation");
        let stale_operation = crate::transaction::server::SupervisedServerResponse::new(
            Arc::clone(&setup.transaction.data),
            response,
        )
        .expect("first supervised response");
        setup
            .transaction
            .data
            .mark_final_response_failed_before_write();
        assert!(setup.transaction.data.begin_final_response_supervision());
        assert_eq!(
            setup
                .transaction
                .data
                .await_final_response_completion_for_generation(first_generation)
                .await,
            FinalResponseCompletionDisposition::ZeroWireRetryable,
            "an old generation must not follow the replacement attempt"
        );

        // Dropping an operation from the first failed generation must not
        // transition the newer PENDING generation back to FAILED_BEFORE_WRITE.
        drop(stale_operation);
        assert!(
            !setup.transaction.data.begin_final_response_supervision(),
            "stale generation changed the active response supervision state"
        );
    }

    #[tokio::test]
    async fn cancelled_final_response_waiting_for_full_command_queue_can_retry() {
        let local_addr: SocketAddr = "127.0.0.1:5090".parse().unwrap();
        let remote_addr: SocketAddr = "127.0.0.1:5070".parse().unwrap();
        let transport = FirstSendBlockingTransport::new(local_addr);
        let (events_tx, _events_rx) = mpsc::channel(16);
        let request = SimpleRequestBuilder::new(Method::Info, "sip:bob@example.com")
            .expect("request builder")
            .from("Alice", "sip:alice@example.com", Some("fromtag"))
            .to("Bob", "sip:bob@example.com", Some("totag"))
            .call_id("full-command-queue-response-retry")
            .cseq(1)
            .via(
                &remote_addr.to_string(),
                "UDP",
                Some("z9hG4bK.full-response-queue"),
            )
            .build();
        let transaction_id =
            TransactionKey::from_request(&request).expect("transaction key from INFO");
        let transaction = Arc::new(
            ServerNonInviteTransaction::new_with_command_channel_capacity(
                transaction_id,
                request.clone(),
                remote_addr,
                Arc::clone(&transport) as Arc<dyn Transport>,
                events_tx,
                Some(TimerSettings {
                    t1: Duration::from_millis(10),
                    wait_time_j: Duration::from_millis(20),
                    ..Default::default()
                }),
                1,
            )
            .expect("non-INVITE server transaction"),
        );

        // Occupy the sole runner with a provisional transport write.
        let provisional = build_simple_response(StatusCode::Trying, &request);
        let provisional_task = {
            let transaction = Arc::clone(&transaction);
            tokio::spawn(async move { transaction.send_response(provisional).await })
        };
        TokioTimeout(Duration::from_secs(1), transport.send_entered.notified())
            .await
            .expect("provisional response reached blocking transport");

        // Fill the one-entry command queue while the runner is blocked, then
        // start a final response that must wait for command admission.
        transaction
            .send_command(InternalTransactionCommand::ProcessMessage(
                Message::Request(request.clone()),
            ))
            .await
            .expect("fill command queue");
        let cancelled_final = {
            let transaction = Arc::clone(&transaction);
            let response = build_simple_response(StatusCode::Ok, &request);
            tokio::spawn(async move { transaction.send_response(response).await })
        };
        TokioTimeout(Duration::from_secs(1), async {
            while !transaction
                .data
                .final_response_supervision_is_pending_for_test()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("final response reached bounded command admission");
        assert!(!cancelled_final.is_finished());
        cancelled_final.abort();
        let _ = cancelled_final.await;
        assert!(!transaction.data.final_response_may_have_reached_wire());

        // Retry before making queue capacity available. The retry owns a new
        // generation and blocks at the same admission point instead of being
        // rejected as an already-supervised final response.
        let retry = {
            let transaction = Arc::clone(&transaction);
            let response = build_simple_response(StatusCode::Ok, &request);
            tokio::spawn(async move { transaction.send_response(response).await })
        };
        TokioTimeout(Duration::from_secs(1), async {
            while !transaction
                .data
                .final_response_supervision_is_pending_for_test()
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("retry acquired a new supervision generation");

        transport.release_send.notify_one();
        provisional_task
            .await
            .expect("provisional task joined")
            .expect("provisional response sent");
        TokioTimeout(Duration::from_secs(1), retry)
            .await
            .expect("retry completed")
            .expect("retry task joined")
            .expect("retried final response sent");

        let sent = transport.sent_messages.lock().await;
        let response_statuses = sent
            .iter()
            .filter_map(|(message, _)| match message {
                Message::Response(response) => Some(response.status_code()),
                Message::Request(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(response_statuses, vec![100, 200]);
    }

    #[tokio::test]
    async fn cancelled_waiter_after_enqueue_observes_runner_owned_completion() {
        let local_addr: SocketAddr = "127.0.0.1:5090".parse().unwrap();
        let remote_addr: SocketAddr = "127.0.0.1:5070".parse().unwrap();
        let transport = FirstSendBlockingTransport::new(local_addr);
        let (events_tx, _events_rx) = mpsc::channel(16);
        let request = SimpleRequestBuilder::new(Method::Info, "sip:bob@example.com")
            .expect("request builder")
            .from("Alice", "sip:alice@example.com", Some("fromtag"))
            .to("Bob", "sip:bob@example.com", Some("totag"))
            .call_id("cancelled-final-response-waiter")
            .cseq(1)
            .via(
                &remote_addr.to_string(),
                "UDP",
                Some("z9hG4bK.cancelled-final-response-waiter"),
            )
            .build();
        let transaction_id =
            TransactionKey::from_request(&request).expect("transaction key from INFO");
        let transaction = Arc::new(
            ServerNonInviteTransaction::new(
                transaction_id,
                request.clone(),
                remote_addr,
                Arc::clone(&transport) as Arc<dyn Transport>,
                events_tx,
                Some(TimerSettings {
                    t1: Duration::from_millis(10),
                    wait_time_j: Duration::from_millis(20),
                    ..Default::default()
                }),
            )
            .expect("non-INVITE server transaction"),
        );

        let caller = {
            let transaction = Arc::clone(&transaction);
            let response = build_simple_response(StatusCode::Ok, &request);
            tokio::spawn(async move { transaction.send_response(response).await })
        };
        TokioTimeout(Duration::from_secs(1), transport.send_entered.notified())
            .await
            .expect("runner entered the transport write");
        let generation = transaction
            .data
            .current_final_response_supervision_generation()
            .expect("active final-response generation");

        // The command has been accepted and the transaction runner owns the
        // transport operation. Cancelling only the API waiter must not cancel
        // the write or make a second final response eligible.
        caller.abort();
        let _ = caller.await;
        let completion = {
            let data = Arc::clone(&transaction.data);
            tokio::spawn(async move {
                data.await_final_response_completion_for_generation(generation)
                    .await
            })
        };
        tokio::task::yield_now().await;
        assert!(!completion.is_finished());
        assert!(transaction.data.final_response_may_have_reached_wire());

        transport.release_send.notify_one();
        let disposition = TokioTimeout(Duration::from_secs(1), completion)
            .await
            .expect("completion became authoritative")
            .expect("completion task joined");
        assert_eq!(
            disposition,
            FinalResponseCompletionDisposition::WrittenSuccessTerminal
        );
        assert!(transaction.data.final_response_may_have_reached_wire());
        assert!(
            !transaction.data.begin_final_response_supervision(),
            "a written final response must remain terminal"
        );

        let sent = transport.sent_messages.lock().await;
        let final_statuses = sent
            .iter()
            .filter_map(|(message, _)| match message {
                Message::Response(response) => Some(response.status_code()),
                Message::Request(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(final_statuses, vec![200]);
    }

    #[tokio::test]
    async fn transport_error_after_write_boundary_is_wire_unknown_and_terminal() {
        let local_addr: SocketAddr = "127.0.0.1:5090".parse().unwrap();
        let remote_addr: SocketAddr = "127.0.0.1:5070".parse().unwrap();
        let transport = FirstSendBlockingTransport::failing(local_addr);
        let (events_tx, _events_rx) = mpsc::channel(16);
        let request = SimpleRequestBuilder::new(Method::Info, "sip:bob@example.com")
            .expect("request builder")
            .from("Alice", "sip:alice@example.com", Some("fromtag"))
            .to("Bob", "sip:bob@example.com", Some("totag"))
            .call_id("final-response-wire-unknown")
            .cseq(1)
            .via(
                &remote_addr.to_string(),
                "UDP",
                Some("z9hG4bK.final-response-wire-unknown"),
            )
            .build();
        let transaction_id =
            TransactionKey::from_request(&request).expect("transaction key from INFO");
        let transaction = Arc::new(
            ServerNonInviteTransaction::new(
                transaction_id,
                request.clone(),
                remote_addr,
                Arc::clone(&transport) as Arc<dyn Transport>,
                events_tx,
                Some(TimerSettings {
                    t1: Duration::from_millis(10),
                    wait_time_j: Duration::from_millis(20),
                    ..Default::default()
                }),
            )
            .expect("non-INVITE server transaction"),
        );

        let send = {
            let transaction = Arc::clone(&transaction);
            let response = build_simple_response(StatusCode::Ok, &request);
            tokio::spawn(async move { transaction.send_response(response).await })
        };
        TokioTimeout(Duration::from_secs(1), transport.send_entered.notified())
            .await
            .expect("runner entered the transport write");
        let generation = transaction
            .data
            .current_final_response_supervision_generation()
            .expect("active final-response generation");
        let completion = {
            let data = Arc::clone(&transaction.data);
            tokio::spawn(async move {
                data.await_final_response_completion_for_generation(generation)
                    .await
            })
        };

        transport.release_send.notify_one();
        TokioTimeout(Duration::from_secs(1), send)
            .await
            .expect("send completed")
            .expect("send task joined")
            .expect_err("injected transport failure must surface");
        let disposition = TokioTimeout(Duration::from_secs(1), completion)
            .await
            .expect("completion became authoritative")
            .expect("completion task joined");
        assert_eq!(
            disposition,
            FinalResponseCompletionDisposition::WireUnknownErrorTerminal
        );
        assert!(transaction.data.final_response_may_have_reached_wire());
        assert!(
            !transaction.data.begin_final_response_supervision(),
            "a write-boundary transport error must not permit a duplicate final response"
        );
        assert!(transport.sent_messages.lock().await.is_empty());
    }

    #[tokio::test]
    async fn closed_command_queue_failure_does_not_permanently_claim_final_response() {
        let setup = setup_test_environment(Method::Info, "sip:bob@example.com").await;
        let original_request = (*setup.transaction.data.request).clone();
        let runner = setup
            .transaction
            .data
            .event_loop_handle
            .lock()
            .await
            .take()
            .expect("transaction runner");
        runner.abort();
        let _ = runner.await;

        let first = setup
            .transaction
            .send_response(build_simple_response(StatusCode::Ok, &original_request))
            .await
            .expect_err("closed command queue must reject first response");
        assert!(first.to_string().contains("Failed to enqueue"));
        assert!(!setup
            .transaction
            .data
            .final_response_may_have_reached_wire());

        let second = setup
            .transaction
            .send_response(build_simple_response(
                StatusCode::NotImplemented,
                &original_request,
            ))
            .await
            .expect_err("closed command queue must reject retry");
        assert!(
            second.to_string().contains("Failed to enqueue"),
            "retry was blocked by stale response supervision: {second}"
        );
        assert!(!setup
            .transaction
            .data
            .final_response_may_have_reached_wire());
    }

    #[tokio::test]
    async fn test_server_noninvite_send_provisional_response() {
        let mut setup = setup_test_environment(Method::Register, "sip:registrar.example.com").await;

        // Create a provisional response
        let original_request = (*setup.transaction.data.request).clone();
        let prov_response = build_simple_response(StatusCode::Trying, &original_request);

        // Send the response
        setup
            .transaction
            .send_response(prov_response.clone())
            .await
            .expect("send_response failed");

        // Wait for the response to be sent
        setup
            .mock_transport
            .wait_for_message_sent(Duration::from_millis(100))
            .await
            .expect("Response should be sent quickly");

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
            Ok(Some(TransactionEvent::StateChanged {
                transaction_id,
                previous_state,
                new_state,
            })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(previous_state, TransactionState::Trying);
                assert_eq!(new_state, TransactionState::Proceeding);
            }
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
        let original_request = (*setup.transaction.data.request).clone();
        let final_response = build_simple_response(StatusCode::Ok, &original_request);

        // Send the response
        setup
            .transaction
            .send_response(final_response.clone())
            .await
            .expect("send_response failed");

        // Wait for the response to be sent
        setup
            .mock_transport
            .wait_for_message_sent(Duration::from_millis(100))
            .await
            .expect("Response should be sent quickly");

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
            Ok(Some(TransactionEvent::StateChanged {
                transaction_id,
                previous_state,
                new_state,
            })) => {
                assert_eq!(transaction_id, *setup.transaction.id());
                assert_eq!(previous_state, TransactionState::Trying);
                assert_eq!(new_state, TransactionState::Completed);
            }
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
    async fn reliable_transport_uses_zero_timer_j() {
        let setup = setup_test_environment_with_transport(
            Method::Register,
            "sip:registrar.example.com",
            Some(rvoip_sip_transport::transport::TransportType::Tls),
        )
        .await;
        let response =
            build_simple_response(StatusCode::Ok, setup.transaction.data.request.as_ref());
        setup
            .transaction
            .send_response(response)
            .await
            .expect("send_response failed");

        TokioTimeout(Duration::from_millis(500), async {
            while setup.transaction.state() != TransactionState::Terminated {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("reliable server transaction should not wait for Timer J");
    }

    #[tokio::test]
    async fn test_server_noninvite_retransmit_final_response() {
        let mut setup = setup_test_environment(Method::Register, "sip:registrar.example.com").await;

        // Create and send final response
        let original_request = (*setup.transaction.data.request).clone();
        let final_response = build_simple_response(StatusCode::Ok, &original_request);
        setup
            .transaction
            .send_response(final_response.clone())
            .await
            .expect("send_response failed");

        // Wait for response to be sent and state to change to Completed
        setup
            .mock_transport
            .wait_for_message_sent(Duration::from_millis(100))
            .await
            .expect("Response should be sent quickly");
        setup.mock_transport.get_sent_message().await;

        // Wait for state transition
        match TokioTimeout(Duration::from_millis(100), setup.tu_events_rx.recv()).await {
            Ok(Some(TransactionEvent::StateChanged { new_state, .. })) => {
                assert_eq!(new_state, TransactionState::Completed);
            }
            _ => panic!("Expected StateChanged event"),
        }

        // Process a retransmitted request
        setup
            .transaction
            .process_request(original_request.clone())
            .await
            .expect("process_request failed");

        // Verify that the response was retransmitted
        setup
            .mock_transport
            .wait_for_message_sent(Duration::from_millis(100))
            .await
            .expect("Response should be retransmitted");
        let retrans_msg_info = setup.mock_transport.get_sent_message().await;
        assert!(
            retrans_msg_info.is_some(),
            "Response should have been retransmitted"
        );
        if let Some((msg, _)) = retrans_msg_info {
            assert!(msg.is_response());
            if let Message::Response(resp) = msg {
                assert_eq!(resp.status_code(), StatusCode::Ok.as_u16());
            }
        }
    }
}
