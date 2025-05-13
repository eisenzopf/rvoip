use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, Sleep};
use std::pin::Pin;
use std::future::Future;
use std::fmt;
use tracing::{debug, error, info, trace, warn};
use tokio::task::JoinHandle;

// Use prelude and specific types
use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;

use crate::error::{Error, Result};
use crate::transaction::{Transaction, TransactionState, TransactionKind, TransactionKey, TransactionEvent}; // Import TransactionEvent
use crate::utils; // Keep utils import

// Standard RFC values in prod code, shorter for tests

// Base timer value (500ms in prod, 50ms for tests)
const T1: Duration = Duration::from_millis(50);

// Maximum retransmission interval (4s in prod, 200ms for tests)
const T2: Duration = Duration::from_millis(200);

// Various non-INVITE timer intervals
const TIMER_B_INVITE_TIMEOUT: Duration = Duration::from_millis(500); // 64*T1, shortened for tests
const TIMER_D_WAIT_TERMINATED: Duration = Duration::from_millis(500); // 32s in RFC, shortened for tests
const TIMER_F_NON_INVITE_TIMEOUT: Duration = Duration::from_millis(500); // 64*T1, shortened for tests
const TIMER_K_WAIT_RESPONSE: Duration = Duration::from_millis(200); // 5s in RFC, shortened for tests

/// Client transaction trait
#[async_trait]
pub trait ClientTransaction: Transaction {
    /// Initiate the transaction by sending the first request.
    /// This starts timers E/F for non-INVITE or A/B for INVITE.
    async fn initiate(&mut self) -> Result<()>;

    /// Process an incoming response for this transaction.
    async fn process_response(&mut self, response: Response) -> Result<()>;
}

/// Shared data for client transactions
// #[derive(Debug)] // Cannot derive Debug because of terminate_signal
struct ClientTxData {
    id: TransactionKey,
    state: TransactionState,
    request: Request,
    last_response: Option<Response>,
    remote_addr: SocketAddr,
    transport: Arc<dyn Transport>,
    /// Channel to send events (like responses, state changes, errors) back to the manager/TU
    events_tx: mpsc::Sender<TransactionEvent>,
    /// Optional sender to signal termination completion
    terminate_signal: Option<oneshot::Sender<()>>,
}

impl fmt::Debug for ClientTxData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientTxData")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("request", &self.request)
            .field("last_response", &self.last_response)
            .field("remote_addr", &self.remote_addr)
            .field("transport", &"Arc<dyn Transport>") // Avoid printing transport details
            .field("events_tx", &self.events_tx)
            .field("terminate_signal", &"Option<oneshot::Sender<()>>") // Don't print sender
            .finish()
    }
}

/// Client INVITE transaction (RFC 3261 Section 17.1.1)
pub struct ClientInviteTransaction {
    data: ClientTxData,
    timer_a_interval: Duration, // Current T1 or T1*2 interval
    timer_a_task: Option<JoinHandle<()>>, // Use JoinHandle
    timer_b_task: Option<JoinHandle<()>>, // Use JoinHandle
    timer_d_task: Option<JoinHandle<()>>, // Use JoinHandle
}

// Manual Debug impl for ClientInviteTransaction
impl fmt::Debug for ClientInviteTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientInviteTransaction")
            .field("data", &self.data)
            .field("timer_a_interval", &self.timer_a_interval)
            .field("timer_a_task", &self.timer_a_task.is_some()) // Show if task exists
            .field("timer_b_task", &self.timer_b_task.is_some())
            .field("timer_d_task", &self.timer_d_task.is_some())
            .finish()
    }
}


impl ClientInviteTransaction {
    /// Create a new client INVITE transaction.
    /// The manager ensures the request has a valid Via header with branch.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
    ) -> Result<Self> {
        if request.method() != Method::Invite {
            return Err(Error::Other("Request must be INVITE for INVITE client transaction".to_string()));
        }

        // Create terminate signal channel
        let (terminate_tx, terminate_rx) = oneshot::channel();

         let mut tx = Self {
            data: ClientTxData {
                id,
                state: TransactionState::Initial,
                request,
                last_response: None,
                remote_addr,
                transport,
                events_tx,
                terminate_signal: Some(terminate_tx), // Store sender half
            },
            timer_a_interval: T1,
            timer_a_task: None,
            timer_b_task: None,
            timer_d_task: None,
         };

        // Spawn a task to listen for the termination signal
        tx.spawn_termination_listener(terminate_rx);

        Ok(tx)
    }

    /// Transition to a new state, handling timer logic.
    async fn transition_to(&mut self, new_state: TransactionState) -> Result<()> {
        if self.data.state == new_state {
            return Ok(()); // No transition needed
        }
        debug!(id = %self.data.id, "State transition: {:?} -> {:?}", self.data.state, new_state);

        // Cancel timers from the old state
        self.cancel_timers();

        // Validate state transition (simplified)
        // Add more robust validation if needed based on RFC state machine diagrams
        match (self.data.state, new_state) {
            (_, TransactionState::Terminated) => {
                 // Signal termination if channel exists
                 if let Some(sender) = self.data.terminate_signal.take() {
                     let _ = sender.send(()); // Ignore result, receiver might have dropped
                 }
            }
            (TransactionState::Initial, TransactionState::Calling) => {}
            (TransactionState::Calling, TransactionState::Proceeding) => {}
            (TransactionState::Calling, TransactionState::Completed) => {}
            (TransactionState::Proceeding, TransactionState::Completed) => {}
            (TransactionState::Completed, TransactionState::Terminated) => {} // Happens via Timer D or QuickTerminate
            _ => return Err(Error::InvalidStateTransition(
                format!("[{}] Invalid state transition: {:?} -> {:?}", self.data.id, self.data.state, new_state)
            )),
        }

        // Store previous state before changing
        let previous_state = self.data.state;
        self.data.state = new_state;

        // Start timers for the new state
        self.start_timers_for_state(new_state);

        // Notify TU about state change
        let _ = self.data.events_tx.send(TransactionEvent::StateChanged { 
            transaction_id: self.data.id.clone(), 
            previous_state,
            new_state 
        }).await;

        Ok(())
    }

     /// Spawns a task that cleans up when termination is signaled.
     fn spawn_termination_listener(&mut self, terminate_rx: oneshot::Receiver<()>) {
         let id = self.data.id.clone();
         tokio::spawn(async move {
             // Wait for the termination signal or for the receiver to be dropped
             let _ = terminate_rx.await;
             debug!(id=%id, "Termination signal received or channel dropped. Invite client transaction cleanup.");
             // Perform any final cleanup if necessary
         });
     }

    /// Cancel all active timers.
    fn cancel_timers(&mut self) {
        if let Some(handle) = self.timer_a_task.take() { handle.abort(); }
        if let Some(handle) = self.timer_b_task.take() { handle.abort(); }
        if let Some(handle) = self.timer_d_task.take() { handle.abort(); }
        trace!(id=%self.data.id, "Cancelled active timers");
    }

    /// Start timers based on the current state.
    fn start_timers_for_state(&mut self, state: TransactionState) {
        match state {
            TransactionState::Calling => {
                self.start_timer_a();
                self.start_timer_b();
            }
            TransactionState::Completed => {
                 if let Some(resp) = &self.data.last_response {
                      // Only start Timer D for non-2xx responses
                      if !resp.status().is_success() {
                            self.start_timer_d();
                      }
                      // We removed the QuickTerminate timer since 2xx responses 
                      // now transition directly to Terminated state
                 }
            }
            _ => {} // No timers needed for Initial, Proceeding, Terminated
        }
    }

    /// Start Timer A (retransmission timer)
    fn start_timer_a(&mut self) {
        let interval = self.timer_a_interval;
        let events_tx = self.data.events_tx.clone();
        let id = self.data.id.clone();
        // Spawn task, store JoinHandle
        self.timer_a_task = Some(tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            debug!(id=%id, "Timer A fired");
            // Send event to manager queue
            let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "A".to_string() }).await;
        }));
        trace!(id=%self.data.id, interval = ?interval, "Started Timer A");
    }

    /// Start Timer B (timeout timer)
    fn start_timer_b(&mut self) {
        let interval = TIMER_B_INVITE_TIMEOUT;
        let events_tx = self.data.events_tx.clone();
        let id = self.data.id.clone();
        self.timer_b_task = Some(tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            debug!(id=%id, "Timer B fired");
            let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "B".to_string() }).await;
        }));
         trace!(id=%self.data.id, interval = ?interval, "Started Timer B");
    }

     /// Start Timer D (wait ACK retransmit timer)
    fn start_timer_d(&mut self) {
        let interval = TIMER_D_WAIT_TERMINATED;
        let events_tx = self.data.events_tx.clone();
        let id = self.data.id.clone();
        self.timer_d_task = Some(tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            debug!(id=%id, "Timer D fired");
            let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "D".to_string() }).await;
        }));
        trace!(id=%self.data.id, interval = ?interval, "Started Timer D");
    }


    /// Handle internal timer events.
    async fn on_timer(&mut self, timer: &str) -> Result<()> {
        match timer {
            "A" => {
                // Timer A logic (retransmit INVITE, double interval, restart A)
                if self.data.state == TransactionState::Calling {
                    debug!(id=%self.data.id, "Timer A triggered, retransmitting INVITE request");
                    // Retransmit INVITE
                    self.data.transport.send_message(
                        Message::Request(self.data.request.clone()),
                        self.data.remote_addr
                    ).await.map_err(|e| Error::TransportError(e.to_string()))?;

                    // Double interval, capped by T2
                    self.timer_a_interval = std::cmp::min(self.timer_a_interval * 2, T2);
                    // Restart timer A
                    self.start_timer_a();
                } else {
                    trace!(id=%self.data.id, state=?self.data.state, "Timer A fired in invalid state, ignoring.");
                }
            }
            "B" => {
                // Timer B logic (timeout)
                if self.data.state == TransactionState::Calling {
                    warn!(id=%self.data.id, "Timer B (Timeout) fired");
                    self.transition_to(TransactionState::Terminated).await?;
                    // Inform TU about transaction timeout
                    self.data.events_tx.send(TransactionEvent::TransactionTimeout {
                        transaction_id: self.data.id.clone(),
                    }).await?;
                } else {
                    trace!(id=%self.data.id, state=?self.data.state, "Timer B fired in invalid state, ignoring.");
                }
            }
            "D" => {
                // Timer D logic (terminate after waiting for retransmissions)
                if self.data.state == TransactionState::Completed {
                    debug!(id=%self.data.id, "Timer D fired in Completed state, terminating");
                    self.transition_to(TransactionState::Terminated).await?;
                } else {
                    trace!(id=%self.data.id, state=?self.data.state, "Timer D fired in invalid state, ignoring.");
                }
            }
            _ => warn!(id=%self.data.id, timer=timer, "Unknown timer triggered"),
        }
        Ok(())
    }


    /// Create an ACK request for a non-2xx final response received in Calling or Proceeding state.
    fn create_internal_ack(&self, response: &Response) -> Result<Request> {
        // Get request URI from the original INVITE's Request-URI
        let request_uri = self.data.request.uri().to_string();
        let mut ack_builder = RequestBuilder::new(Method::Ack, &request_uri)?;

        // Copy Route headers from original INVITE (if present)
        for header in self.data.request.headers.iter() {
             if let TypedHeader::Route(route) = header {
                 ack_builder = ack_builder.header(TypedHeader::Route(route.clone()));
             }
        }
        if let Some(from_header) = self.data.request.header(&HeaderName::From) {
             if let TypedHeader::From(from) = from_header {
                 ack_builder = ack_builder.header(TypedHeader::From(from.clone()));
             } else {
                 return Err(Error::Other("Original INVITE request has invalid From header".into()));
             }
         } else {
             return Err(Error::Other("Original INVITE request missing From header".into()));
         }
        if let Some(to_header) = response.header(&HeaderName::To) {
            if let TypedHeader::To(to) = to_header {
                ack_builder = ack_builder.header(TypedHeader::To(to.clone()));
            } else {
                return Err(Error::Other("Response has invalid To header".into()));
            }
        } else {
            return Err(Error::Other("Response missing To header".into()));
        }
         if let Some(call_id_header) = self.data.request.header(&HeaderName::CallId) {
             if let TypedHeader::CallId(call_id) = call_id_header {
                 ack_builder = ack_builder.header(TypedHeader::CallId(call_id.clone()));
             } else {
                 return Err(Error::Other("Original INVITE request has invalid Call-ID header".into()));
             }
         } else {
             return Err(Error::Other("Original INVITE request missing Call-ID".into()));
         }
        if let Some(cseq_header) = self.data.request.header(&HeaderName::CSeq) {
             if let TypedHeader::CSeq(cseq) = cseq_header {
                 ack_builder = ack_builder.header(TypedHeader::CSeq(CSeq::new(cseq.sequence(), Method::Ack)));
             } else {
                 return Err(Error::Other("Original INVITE request has invalid CSeq header".into()));
             }
         } else {
             return Err(Error::Other("Original INVITE request missing CSeq".into()));
         }

        ack_builder = ack_builder.header(TypedHeader::MaxForwards(MaxForwards::new(70)));
        ack_builder = ack_builder.header(TypedHeader::ContentLength(ContentLength::new(0)));

        Ok(ack_builder.build())
    }
}

#[async_trait]
impl Transaction for ClientInviteTransaction {
    fn id(&self) -> &TransactionKey {
        &self.data.id
    }

    fn kind(&self) -> TransactionKind {
        TransactionKind::InviteClient
    }

    fn state(&self) -> TransactionState {
        self.data.state
    }

    fn transport(&self) -> Arc<dyn Transport> {
        self.data.transport.clone()
    }

    fn remote_addr(&self) -> SocketAddr {
         self.data.remote_addr
     }

    async fn process_event(&mut self, event_type: &str, message: Option<Message>) -> Result<()> {
         match event_type {
             "response" => {
                 if let Some(Message::Response(resp)) = message {
                     self.process_response(resp).await
                 } else {
                     warn!(id=%self.data.id, "Received non-response message event");
                     Ok(())
                 }
             }
             "timer" => {
                 error!(id=%self.data.id, "process_event called for timer event");
                 Ok(())
             }
             "transport_err" => {
                 error!(id=%self.data.id, "Transport error occurred, terminating transaction");
                 self.transition_to(TransactionState::Terminated).await?;
                 self.data.events_tx.send(TransactionEvent::TransportError {
                    transaction_id: self.data.id.clone(),
                 }).await?;
                 Ok(())
             }
             _ => {
                 warn!(id=%self.data.id, event=event_type, "Unhandled transaction event type");
                 Ok(())
             }
         }
     }

     async fn handle_timer(&mut self, timer_name: String) -> Result<()> {
         self.on_timer(&timer_name).await
     }


    fn matches(&self, message: &Message) -> bool {
        crate::utils::transaction_key_from_message(message).map(|key| key == self.data.id).unwrap_or(false)
    }

    // Keep original_request and last_response accessors if needed by TU via manager
    fn original_request(&self) -> &Request {
        &self.data.request
    }

    fn last_response(&self) -> Option<&Response> {
        self.data.last_response.as_ref()
    }
}

#[async_trait]
impl ClientTransaction for ClientInviteTransaction {
    /// Initiate the transaction by sending the first INVITE.
    async fn initiate(&mut self) -> Result<()> {
        match self.data.state {
            TransactionState::Initial => {
                debug!(id=%self.data.id, "Sending initial INVITE request");
                // Send request via transport
                self.data.transport.send_message(
                    Message::Request(self.data.request.clone()),
                    self.data.remote_addr
                ).await.map_err(|e| Error::TransportError(e.to_string()))?;

                // Transition state *after* successful send
                self.transition_to(TransactionState::Calling).await?;
                Ok(())
            },
            _ => {
                error!(id=%self.data.id, state=?self.data.state, "Cannot initiate transaction in non-Initial state");
                Err(Error::InvalidStateTransition(
                    format!("Cannot initiate INVITE transaction in {:?} state", self.data.state)
                ))
            }
        }
    }

    /// Process an incoming response.
    async fn process_response(&mut self, response: Response) -> Result<()> {
        let status = response.status();
        let is_provisional = status.is_provisional();
        let is_success = status.is_success();
        let is_failure = !is_provisional && !is_success;

        let id = self.data.id.clone();

        // Store the response for later access
        self.data.last_response = Some(response.clone());

        match self.data.state {
            TransactionState::Calling => {
                self.cancel_timers();
                if is_provisional { 
                    // 1xx responses move to Proceeding state
                    self.transition_to(TransactionState::Proceeding).await?;
                    
                    // Notify TU of provisional response
                    self.data.events_tx.send(TransactionEvent::ProvisionalResponse {
                        transaction_id: id,
                        response: response.clone(),
                    }).await?;
                }
                else if is_success { 
                    // 2xx responses should go directly to Terminated (RFC 3261 17.1.1.2)
                    self.transition_to(TransactionState::Terminated).await?;
                    
                    // Notify TU of success response
                    self.data.events_tx.send(TransactionEvent::SuccessResponse {
                        transaction_id: id,
                        response: response.clone(),
                    }).await?;
                }
                else if is_failure { 
                    // 3xx-6xx responses
                    self.transition_to(TransactionState::Completed).await?;
                    
                    // For non-2xx responses, we need to send ACK
                    let ack = self.create_internal_ack(&response)?;
                    self.data.transport.send_message(
                        Message::Request(ack),
                        self.data.remote_addr
                    ).await.map_err(|e| Error::TransportError(e.to_string()))?;
                    
                    // Notify TU of failure response
                    self.data.events_tx.send(TransactionEvent::FailureResponse {
                        transaction_id: id,
                        response: response.clone(),
                    }).await?;
                }
            }
            TransactionState::Proceeding => {
                if is_provisional { 
                    // Additional 1xx in Proceeding state, forward to TU
                    self.data.events_tx.send(TransactionEvent::ProvisionalResponse {
                        transaction_id: id,
                        response: response.clone(),
                    }).await?;
                }
                else if is_success { 
                    // 2xx responses should transition directly to Terminated (RFC 3261 17.1.1.2)
                    self.transition_to(TransactionState::Terminated).await?;
                    
                    // Notify TU of success response
                    self.data.events_tx.send(TransactionEvent::SuccessResponse {
                        transaction_id: id,
                        response: response.clone(),
                    }).await?;
                }
                else if is_failure { 
                    // 3xx-6xx responses transition to Completed
                    self.transition_to(TransactionState::Completed).await?;
                    
                    // For non-2xx responses, we need to send ACK
                    let ack = self.create_internal_ack(&response)?;
                    self.data.transport.send_message(
                        Message::Request(ack),
                        self.data.remote_addr
                    ).await.map_err(|e| Error::TransportError(e.to_string()))?;
                    
                    // Notify TU of failure response
                    self.data.events_tx.send(TransactionEvent::FailureResponse {
                        transaction_id: id,
                        response: response.clone(),
                    }).await?;
                }
            }
            TransactionState::Completed => {
                if is_failure { 
                    // Received retransmission of final response
                    // Resend ACK (RFC 3261 section 17.1.1.3)
                    debug!(id=%id, "Received retransmission of error response in Completed state, resending ACK");
                    
                    if let Some(last_resp) = &self.data.last_response {
                        let ack = self.create_internal_ack(last_resp)?;
                        self.data.transport.send_message(
                            Message::Request(ack),
                            self.data.remote_addr
                        ).await.map_err(|e| Error::TransportError(e.to_string()))?;
                    }
                } else { 
                    trace!(id=%id, state=?self.data.state, %status, "Ignoring success or provisional response in Completed state");
                }
            }
            TransactionState::Terminated | TransactionState::Initial | TransactionState::Trying | TransactionState::Confirmed => {
                 warn!(id=%id, state=?self.data.state, status=%status, "Received response in unexpected state");
            }
        }
        Ok(())
    }
}

/// Client non-INVITE transaction (RFC 3261 Section 17.1.2)
pub struct ClientNonInviteTransaction {
    data: ClientTxData,
    timer_e_interval: Duration, // Current T1 or T1*2 interval
    timer_e_task: Option<JoinHandle<()>>, // Use JoinHandle
    timer_f_task: Option<JoinHandle<()>>, // Use JoinHandle
    timer_k_task: Option<JoinHandle<()>>, // Use JoinHandle
}

// Manual Debug impl for ClientNonInviteTransaction
impl fmt::Debug for ClientNonInviteTransaction {
     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
         f.debug_struct("ClientNonInviteTransaction")
             .field("data", &self.data)
             .field("timer_e_interval", &self.timer_e_interval)
             .field("timer_e_task", &self.timer_e_task.is_some())
             .field("timer_f_task", &self.timer_f_task.is_some())
             .field("timer_k_task", &self.timer_k_task.is_some())
             .finish()
     }
 }

impl ClientNonInviteTransaction {
    /// Create a new client non-INVITE transaction.
    /// Manager ensures Via header with branch exists.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
    ) -> Result<Self> {
        if request.method() == Method::Invite || request.method() == Method::Ack {
            return Err(Error::Other("Request must not be INVITE or ACK for non-INVITE client transaction".to_string()));
        }

        let (terminate_tx, terminate_rx) = oneshot::channel();

         let mut tx = Self {
            data: ClientTxData {
                id,
                state: TransactionState::Initial,
                request,
                last_response: None,
                remote_addr,
                transport,
                events_tx,
                 terminate_signal: Some(terminate_tx),
            },
            timer_e_interval: T1,
            timer_e_task: None,
            timer_f_task: None,
            timer_k_task: None,
        };

         tx.spawn_termination_listener(terminate_rx);
         Ok(tx)
    }

     /// Spawns a task that cleans up when termination is signaled.
     fn spawn_termination_listener(&mut self, terminate_rx: oneshot::Receiver<()>) {
         let id = self.data.id.clone();
         tokio::spawn(async move {
             let _ = terminate_rx.await;
             debug!(id=%id, "Termination signal received or channel dropped. Non-invite client transaction cleanup.");
             // Perform any final cleanup if necessary
         });
     }

    /// Transition to a new state, handling timer logic.
    async fn transition_to(&mut self, new_state: TransactionState) -> Result<()> {
        if self.data.state == new_state { return Ok(()); }
        debug!(id=%self.data.id, "State transition: {:?} -> {:?}", self.data.state, new_state);

        self.cancel_timers();

        // Validate state transition
        match (self.data.state, new_state) {
             (_, TransactionState::Terminated) => {
                 if let Some(sender) = self.data.terminate_signal.take() {
                     let _ = sender.send(());
                 }
            }
            (TransactionState::Initial, TransactionState::Trying) => {}
            (TransactionState::Trying, TransactionState::Proceeding) => {}
            (TransactionState::Trying, TransactionState::Completed) => {}
            (TransactionState::Proceeding, TransactionState::Completed) => {}
            (TransactionState::Completed, TransactionState::Terminated) => {} // Happens via Timer K
            _ => return Err(Error::InvalidStateTransition(
                format!("[{}] Invalid state transition: {:?} -> {:?}", self.data.id, self.data.state, new_state)
            )),
        }

        // Store previous state before changing
        let previous_state = self.data.state;
        self.data.state = new_state;
        
        self.start_timers_for_state(new_state);
        
        // Notify TU about state change
        let _ = self.data.events_tx.send(TransactionEvent::StateChanged { 
            transaction_id: self.data.id.clone(), 
            previous_state,
            new_state 
        }).await;
        
        Ok(())
    }

    /// Cancel all active timers.
    fn cancel_timers(&mut self) {
        if let Some(handle) = self.timer_e_task.take() { handle.abort(); }
        if let Some(handle) = self.timer_f_task.take() { handle.abort(); }
        if let Some(handle) = self.timer_k_task.take() { handle.abort(); }
         trace!(id=%self.data.id, "Cancelled active timers");
    }

    /// Start timers based on the current state.
    fn start_timers_for_state(&mut self, state: TransactionState) {
        match state {
            TransactionState::Trying | TransactionState::Proceeding => {
                // Start Timer E (retransmission)
                self.start_timer_e();
                // Start Timer F (timeout) - only if not already running
                if self.timer_f_task.is_none() && state == TransactionState::Trying {
                    self.start_timer_f();
                }
            }
            TransactionState::Completed => {
                 // Start Timer K (wait for retransmissions)
                 self.start_timer_k();
            }
            _ => {} // No timers needed for Initial, Terminated
        }
    }

    /// Start Timer E (retransmission timer)
    fn start_timer_e(&mut self) {
         let interval = self.timer_e_interval;
         let events_tx = self.data.events_tx.clone();
         let id = self.data.id.clone();
         self.timer_e_task = Some(tokio::spawn(async move {
             tokio::time::sleep(interval).await;
             debug!(id=%id, "Timer E fired");
             let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "E".to_string() }).await;
         }));
          trace!(id=%self.data.id, interval = ?interval, "Started Timer E");
     }

     /// Start Timer F (timeout timer)
    fn start_timer_f(&mut self) {
        let interval = TIMER_F_NON_INVITE_TIMEOUT;
        let events_tx = self.data.events_tx.clone();
        let id = self.data.id.clone();
        self.timer_f_task = Some(tokio::spawn(async move {
            tokio::time::sleep(interval).await;
             debug!(id=%id, "Timer F fired");
             let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "F".to_string() }).await;
         }));
          trace!(id=%self.data.id, interval = ?interval, "Started Timer F");
     }

     /// Start Timer K (wait retransmit timer)
    fn start_timer_k(&mut self) {
        let interval = TIMER_K_WAIT_RESPONSE;
        let events_tx = self.data.events_tx.clone();
        let id = self.data.id.clone();
        
        // For Timer K, we'll have it directly transition the transaction to Terminated
        // This avoids race conditions or issues with the timer event not being processed
        self.timer_k_task = Some(tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            debug!(id=%id, "Timer K fired, directly terminating transaction");
            
            // Send state change event to indicate the transition
            let _ = events_tx.send(TransactionEvent::StateChanged { 
                transaction_id: id.clone(), 
                previous_state: TransactionState::Completed,
                new_state: TransactionState::Terminated 
            }).await;
            
            // Send termination event directly
            if let Err(e) = events_tx.send(TransactionEvent::TransactionTerminated { 
                transaction_id: id.clone() 
            }).await {
                error!(id=%id, error=%e, "Failed to send termination event");
            } else {
                debug!(id=%id, "Transaction termination event sent successfully");
            }
        }));
        
        trace!(id=%self.data.id, interval = ?interval, "Started Timer K");
    }

     /// Handle internal timer events.
     async fn on_timer(&mut self, timer: &str) -> Result<()> {
         match timer {
             "E" => {
                 // Timer E logic (retransmit request, double interval, restart E)
                 if self.data.state == TransactionState::Trying || self.data.state == TransactionState::Proceeding {
                     debug!(id=%self.data.id, "Timer E triggered, retransmitting request");
                     // Retransmit request
                     self.data.transport.send_message(
                         Message::Request(self.data.request.clone()),
                         self.data.remote_addr
                     ).await.map_err(|e| Error::TransportError(e.to_string()))?;

                     // Double interval, capped by T2
                     self.timer_e_interval = std::cmp::min(self.timer_e_interval * 2, T2);
                     // Restart timer E
                     self.start_timer_e();
                 } else {
                     trace!(id=%self.data.id, state=?self.data.state, "Timer E fired in invalid state, ignoring.");
                 }
             }
             "F" => {
                 // Timer F logic (timeout)
                 if self.data.state == TransactionState::Trying || self.data.state == TransactionState::Proceeding {
                     warn!(id=%self.data.id, "Timer F (Timeout) fired, terminating transaction");
                     self.transition_to(TransactionState::Terminated).await?;
                     // Inform TU about timeout
                     self.data.events_tx.send(TransactionEvent::TransactionTimeout {
                        transaction_id: self.data.id.clone(),
                     }).await?;
                 } else {
                     trace!(id=%self.data.id, state=?self.data.state, "Timer F fired in invalid state, ignoring.");
                 }
             }
             "K" => {
                 // Timer K logic (terminate after waiting for retransmissions)
                 if self.data.state == TransactionState::Completed {
                     debug!(id=%self.data.id, "Timer K fired in Completed state, terminating");
                     
                     // Store the previous state
                     let previous_state = self.data.state;
                     
                     // Force the state to Terminated without relying on transition_to
                     self.data.state = TransactionState::Terminated;
                     
                     // Cancel all timers
                     self.cancel_timers();
                     
                     // Signal termination if channel exists
                     if let Some(sender) = self.data.terminate_signal.take() {
                         let _ = sender.send(()); // Ignore result, receiver might have dropped
                     }
                     
                     // Send state change event manually since we're bypassing transition_to
                     let _ = self.data.events_tx.send(TransactionEvent::StateChanged { 
                         transaction_id: self.data.id.clone(), 
                         previous_state,
                         new_state: TransactionState::Terminated 
                     }).await;
                     
                     // Send an explicit termination event to ensure the transaction manager can clean up
                     self.data.events_tx.send(TransactionEvent::TransactionTerminated { 
                         transaction_id: self.data.id.clone() 
                     }).await?;
                     
                     debug!(id=%self.data.id, "Transaction forcefully terminated");
                 } else {
                     trace!(id=%self.data.id, state=?self.data.state, "Timer K fired in invalid state, ignoring.");
                 }
             }
             _ => warn!(id=%self.data.id, timer=timer, "Unknown timer triggered"),
         }
         Ok(())
     }
}

#[async_trait]
impl Transaction for ClientNonInviteTransaction {
     fn id(&self) -> &TransactionKey {
         &self.data.id
     }

     fn kind(&self) -> TransactionKind {
         TransactionKind::NonInviteClient
     }

     fn state(&self) -> TransactionState {
         self.data.state
     }

     fn transport(&self) -> Arc<dyn Transport> {
         self.data.transport.clone()
     }

      fn remote_addr(&self) -> SocketAddr {
          self.data.remote_addr
      }

      async fn process_event(&mut self, event_type: &str, message: Option<Message>) -> Result<()> {
          match event_type {
              "response" => {
                  if let Some(Message::Response(resp)) = message {
                      self.process_response(resp).await
                  } else {
                      warn!(id=%self.data.id, "Received non-response message event");
                      Ok(())
                  }
              }
              "timer" => {
                  error!(id=%self.data.id, "process_event called for timer event");
                  Ok(())
              }
              "transport_err" => {
                  error!(id=%self.data.id, "Transport error occurred, terminating transaction");
                  self.transition_to(TransactionState::Terminated).await?;
                   // Notify TU
                  self.data.events_tx.send(TransactionEvent::TransportError {
                     transaction_id: self.data.id.clone(),
                  }).await?;
                  Ok(())
              }
              _ => {
                  warn!(id=%self.data.id, event=event_type, "Unhandled transaction event type");
                  Ok(())
              }
          }
      }

       async fn handle_timer(&mut self, timer_name: String) -> Result<()> {
           self.on_timer(&timer_name).await
       }


     fn matches(&self, message: &Message) -> bool {
          crate::utils::transaction_key_from_message(message).map(|key| key == self.data.id).unwrap_or(false)
     }

      // Keep original_request and last_response accessors if needed by TU via manager
     fn original_request(&self) -> &Request {
         &self.data.request
     }

     fn last_response(&self) -> Option<&Response> {
         self.data.last_response.as_ref()
     }
}

#[async_trait]
impl ClientTransaction for ClientNonInviteTransaction {
     /// Initiate the transaction by sending the first request.
    async fn initiate(&mut self) -> Result<()> {
        match self.data.state {
            TransactionState::Initial => {
                debug!(id=%self.data.id, method=%self.data.request.method(), "Sending initial non-INVITE request");
                // Send request via transport
                 self.data.transport.send_message(
                     Message::Request(self.data.request.clone()),
                     self.data.remote_addr
                 ).await.map_err(|e| Error::TransportError(e.to_string()))?;

                 // Transition state *after* successful send
                 self.transition_to(TransactionState::Trying).await?;
                Ok(())
            },
            _ => {
                 error!(id=%self.data.id, state=?self.data.state, "Cannot initiate transaction in non-Initial state");
                Err(Error::InvalidStateTransition(
                     format!("Cannot initiate non-INVITE transaction in {:?} state", self.data.state)
                 ))
             }
        }
    }

     /// Process an incoming response.
    async fn process_response(&mut self, response: Response) -> Result<()> {
        let status = response.status();
        let is_provisional = status.is_provisional();
        let is_success = status.is_success();
        let is_failure = !is_provisional && !is_success;
        let is_final = !is_provisional;

        let id = self.data.id.clone();
        
        // Store the response for later access
        self.data.last_response = Some(response.clone());

        match self.data.state {
             TransactionState::Trying => {
                 self.cancel_timers();
                 if is_provisional {
                     // Move to Proceeding state for 1xx responses
                     self.transition_to(TransactionState::Proceeding).await?;
                     
                     // Notify TU of provisional response
                     self.data.events_tx.send(TransactionEvent::ProvisionalResponse {
                         transaction_id: id,
                         response: response.clone(),
                     }).await?;
                 }
                 else if is_final {
                     // Move to Completed state for final responses
                     self.transition_to(TransactionState::Completed).await?;
                     
                     if is_success {
                         // Notify TU of success response
                         self.data.events_tx.send(TransactionEvent::SuccessResponse {
                             transaction_id: id,
                             response: response.clone(),
                         }).await?;
                     } else { // is_failure
                         // Notify TU of failure response
                         self.data.events_tx.send(TransactionEvent::FailureResponse {
                             transaction_id: id,
                             response: response.clone(),
                         }).await?;
                     }
                 }
             }
             TransactionState::Proceeding => {
                 if is_provisional {
                     // Additional 1xx in Proceeding state, forward to TU
                     self.data.events_tx.send(TransactionEvent::ProvisionalResponse {
                         transaction_id: id,
                         response: response.clone(),
                     }).await?;
                 }
                 else if is_final {
                     // Move to Completed state for final responses
                     self.transition_to(TransactionState::Completed).await?;
                     
                     if is_success {
                         // Notify TU of success response
                         self.data.events_tx.send(TransactionEvent::SuccessResponse {
                             transaction_id: id,
                             response: response.clone(),
                         }).await?;
                     } else { // is_failure
                         // Notify TU of failure response 
                         self.data.events_tx.send(TransactionEvent::FailureResponse {
                             transaction_id: id,
                             response: response.clone(),
                         }).await?;
                     }
                 }
             }
             TransactionState::Completed | TransactionState::Terminated | TransactionState::Initial | TransactionState::Calling | TransactionState::Confirmed => {
                 trace!(id=%id, state=?self.data.state, %status, "Ignoring response in {:?} state", self.data.state);
             }
        }
        Ok(())
    }
}

mod test_helpers {
    use super::*;
    use crate::error::{Error as TransactionError, Result};
    use std::sync::{Arc, Mutex};
    use std::str::FromStr;
    use rvoip_sip_core::prelude::*;

    #[derive(Debug)]
    pub struct MockTransport {
        pub sent_messages: Arc<Mutex<Vec<Message>>>,
    }

    impl MockTransport {
        pub fn new() -> Self {
            Self {
                sent_messages: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    #[async_trait]
    impl Transport for MockTransport {
        async fn send_message(&self, message: Message, _destination: SocketAddr) -> std::result::Result<(), rvoip_sip_transport::Error> {
            self.sent_messages.lock().unwrap().push(message);
            Ok(())
        }

        fn local_addr(&self) -> std::result::Result<SocketAddr, rvoip_sip_transport::Error> {
            Ok("127.0.0.1:5060".parse().unwrap())
        }

        async fn close(&self) -> std::result::Result<(), rvoip_sip_transport::Error> {
            Ok(())
        }

        fn is_closed(&self) -> bool {
            false
        }
    }

    pub fn create_test_invite_request() -> Message {
        let uri = Uri::sip("bob@example.com");
        let from_uri = Uri::sip("alice@example.com");
        
        // Create address and add tag to uri
        let mut from_uri_with_tag = from_uri.clone();
        from_uri_with_tag = from_uri_with_tag.with_parameter(Param::tag("fromtag123"));
        let from_addr = Address::new(from_uri_with_tag);
        let to_addr = Address::new(uri.clone());
        
        let request = RequestBuilder::new(Method::Invite, uri.to_string().as_str()).unwrap()
            .header(TypedHeader::From(From::new(from_addr)))
            .header(TypedHeader::To(To::new(to_addr)))
            .header(TypedHeader::CallId(CallId::new("test-call-id")))
            .header(TypedHeader::CSeq(CSeq::new(1, Method::Invite)))
            .header(TypedHeader::Via(Via::new("SIP", "2.0", "UDP", "192.168.1.1", Some(5060), vec![Param::branch("z9hG4bK1234")]).unwrap()))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();
            
        Message::Request(request)
    }

    pub fn create_test_non_invite_request(method_str: &str) -> Message {
        let method = match method_str {
            "REGISTER" => Method::Register,
            "OPTIONS" => Method::Options,
            "BYE" => Method::Bye,
            "CANCEL" => Method::Cancel,
            _ => Method::Register, // Default to REGISTER
        };
        
        let uri = Uri::sip("bob@example.com");
        let from_uri = Uri::sip("alice@example.com");
        
        // Create address and add tag to uri
        let mut from_uri_with_tag = from_uri.clone();
        from_uri_with_tag = from_uri_with_tag.with_parameter(Param::tag("fromtag123"));
        let from_addr = Address::new(from_uri_with_tag);
        let to_addr = Address::new(uri.clone());
        
        let request = RequestBuilder::new(method.clone(), uri.to_string().as_str()).unwrap()
            .header(TypedHeader::From(From::new(from_addr)))
            .header(TypedHeader::To(To::new(to_addr)))
            .header(TypedHeader::CallId(CallId::new("test-call-id")))
            .header(TypedHeader::CSeq(CSeq::new(1, method)))
            .header(TypedHeader::Via(Via::new("SIP", "2.0", "UDP", "192.168.1.1", Some(5060), vec![Param::branch("z9hG4bK1234")]).unwrap()))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();
            
        Message::Request(request)
    }

    pub fn create_test_response(request: &Message, status_code: u16) -> Message {
        // Get the status code as a proper StatusCode
        let status = match status_code {
            100 => StatusCode::Trying,
            180 => StatusCode::Ringing,
            200 => StatusCode::Ok,
            400 => StatusCode::BadRequest,
            404 => StatusCode::NotFound,
            _ => StatusCode::Trying, // Default
        };
        
        let request = match request {
            Message::Request(req) => req,
            _ => panic!("Expected Request message"),
        };
        
        let mut response_builder = ResponseBuilder::new(status, None);
        
        // Copy essential headers
        if let Some(header) = request.header(&HeaderName::Via) {
            response_builder = response_builder.header(header.clone());
        }
        if let Some(header) = request.header(&HeaderName::From) {
            response_builder = response_builder.header(header.clone());
        }
        if let Some(header) = request.header(&HeaderName::To) {
            // For final responses, add a tag if none exists
            if status_code >= 200 {
                let to_header = match header {
                    TypedHeader::To(to) => {
                        let to_addr = to.address().clone();
                        let uri_with_tag = if !to_addr.uri.parameters.iter().any(|p| match p {
                            Param::Tag(_) => true,
                            _ => false
                        }) {
                            to_addr.uri.with_parameter(Param::tag("resp-tag"))
                        } else {
                            to_addr.uri.clone()
                        };
                        let addr_with_tag = Address::new(uri_with_tag);
                        TypedHeader::To(To::new(addr_with_tag))
                    },
                    _ => header.clone(),
                };
                response_builder = response_builder.header(to_header);
            } else {
                response_builder = response_builder.header(header.clone());
            }
        }
        if let Some(header) = request.header(&HeaderName::CallId) {
            response_builder = response_builder.header(header.clone());
        }
        if let Some(header) = request.header(&HeaderName::CSeq) {
            response_builder = response_builder.header(header.clone());
        }
        
        let response = response_builder
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build();
            
        Message::Response(response)
    }

    pub async fn process_response_for_invite_tests(
        transaction: &mut ClientInviteTransaction,
        response: Message
    ) -> std::result::Result<(), TransactionError> {
        if let Message::Response(resp) = response {
            transaction.process_response(resp).await
        } else {
            Err(TransactionError::Other("Expected Response message".to_string()))
        }
    }

    pub async fn process_response_for_non_invite_tests(
        transaction: &mut ClientNonInviteTransaction,
        response: Message
    ) -> std::result::Result<(), TransactionError> {
        if let Message::Response(resp) = response {
            transaction.process_response(resp).await
        } else {
            Err(TransactionError::Other("Expected Response message".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::str::FromStr;
    use std::sync::Arc;
    use tokio::sync::mpsc;
    use rvoip_sip_transport::Error as TransportError;
    
    // Mock transport implementation
    #[derive(Debug, Clone)]
    struct MockTransport {
        sent_messages: Arc<std::sync::Mutex<Vec<(Message, SocketAddr)>>>,
        local_addr: SocketAddr,
        should_fail: bool,
    }
    
    impl MockTransport {
        fn new(local_addr: SocketAddr) -> Self {
            Self {
                sent_messages: Arc::new(std::sync::Mutex::new(Vec::new())),
                local_addr,
                should_fail: false,
            }
        }
        
        fn with_failure(local_addr: SocketAddr) -> Self {
            Self {
                sent_messages: Arc::new(std::sync::Mutex::new(Vec::new())),
                local_addr,
                should_fail: true,
            }
        }
        
        fn get_sent_messages(&self) -> Vec<(Message, SocketAddr)> {
            self.sent_messages.lock().unwrap().clone()
        }
    }
    
    #[async_trait]
    impl Transport for MockTransport {
        async fn send_message(&self, message: Message, destination: SocketAddr) -> std::result::Result<(), TransportError> {
            if self.should_fail {
                return Err(TransportError::Other("Simulated transport error".to_string()));
            }
            
            self.sent_messages.lock().unwrap().push((message, destination));
            Ok(())
        }
        
        fn local_addr(&self) -> std::result::Result<SocketAddr, TransportError> {
            Ok(self.local_addr)
        }
        
        async fn close(&self) -> std::result::Result<(), TransportError> {
            Ok(()) // Do nothing for test mock
        }
        
        fn is_closed(&self) -> bool {
            false // Always return false for testing
        }
    }
    
    // Helper to create a test INVITE request
    fn create_test_invite() -> Request {
        let uri = Uri::sip("bob@example.com");
        let from_uri = Uri::sip("alice@example.com");
        
        // Create address and add tag to uri
        let mut from_uri_with_tag = from_uri.clone();
        from_uri_with_tag = from_uri_with_tag.with_parameter(Param::tag("fromtag123"));
        let from_addr = Address::new(from_uri_with_tag);
        let to_addr = Address::new(uri.clone());
        
        RequestBuilder::new(Method::Invite, uri.to_string().as_str()).unwrap()
            .header(TypedHeader::From(From::new(from_addr)))
            .header(TypedHeader::To(To::new(to_addr)))
            .header(TypedHeader::CallId(CallId::new("test-call-id")))
            .header(TypedHeader::CSeq(CSeq::new(1, Method::Invite)))
            .header(TypedHeader::Via(Via::new("SIP", "2.0", "UDP", "192.168.1.1", Some(5060), vec![Param::branch("z9hG4bK1234")]).unwrap()))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }
    
    // Helper to create a test non-INVITE request
    fn create_test_register() -> Request {
        let uri = Uri::sip("registrar.example.com");
        let from_uri = Uri::sip("alice@example.com");
        
        // Create address and add tag to uri
        let mut from_uri_with_tag = from_uri.clone();
        from_uri_with_tag = from_uri_with_tag.with_parameter(Param::tag("fromtag123"));
        let from_addr = Address::new(from_uri_with_tag);
        
        RequestBuilder::new(Method::Register, uri.to_string().as_str()).unwrap()
            .header(TypedHeader::From(From::new(from_addr)))
            .header(TypedHeader::To(To::new(Address::new(from_uri.clone()))))
            .header(TypedHeader::CallId(CallId::new("test-reg-id")))
            .header(TypedHeader::CSeq(CSeq::new(1, Method::Register)))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }
    
    // Helper to create a Via header with a branch
    fn add_via_header(request: &mut Request, branch: &str) {
        let via = Via::new(
            "SIP", 
            "2.0", 
            "UDP", 
            "127.0.0.1", 
            Some(5060),
            vec![Param::branch(branch)]
        ).unwrap();
        
        request.headers.retain(|h| !matches!(h, TypedHeader::Via(_)));
        request.headers.insert(0, TypedHeader::Via(via));
    }
    
    // Helper to create a response from a request
    fn create_response(request: &Request, status_code: StatusCode) -> Response {
        let mut builder = ResponseBuilder::new(status_code, None);
        
        // Copy essential headers
        if let Some(header) = request.header(&HeaderName::Via) {
            builder = builder.header(header.clone());
        }
        if let Some(header) = request.header(&HeaderName::From) {
            builder = builder.header(header.clone());
        }
        if let Some(header) = request.header(&HeaderName::To) {
            builder = builder.header(header.clone());
        }
        if let Some(header) = request.header(&HeaderName::CallId) {
            builder = builder.header(header.clone());
        }
        if let Some(header) = request.header(&HeaderName::CSeq) {
            builder = builder.header(header.clone());
        }
        
        builder = builder.header(TypedHeader::ContentLength(ContentLength::new(0)));
        
        builder.build()
    }
    
    #[tokio::test]
    async fn test_client_invite_transaction_creation() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, _events_rx) = mpsc::channel(10);
        
        let mut request = create_test_invite();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let result = ClientInviteTransaction::new(
            transaction_id.clone(),
            request,
            remote_addr,
            mock_transport.clone(),
            events_tx,
        );
        
        assert!(result.is_ok());
        let transaction = result.unwrap();
        
        // Verify initial state
        assert_eq!(transaction.state(), TransactionState::Initial);
        assert_eq!(transaction.kind(), TransactionKind::InviteClient);
        assert_eq!(transaction.remote_addr(), remote_addr);
        assert_eq!(transaction.id(), &transaction_id);
    }
    
    #[tokio::test]
    async fn test_client_invite_transaction_initiate() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, _events_rx) = mpsc::channel(10);
        
        let mut request = create_test_invite();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let mut transaction = ClientInviteTransaction::new(
            transaction_id,
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Initiate the transaction
        let result = transaction.initiate().await;
        assert!(result.is_ok());
        
        // Verify state changed to Calling
        assert_eq!(transaction.state(), TransactionState::Calling);
        
        // Verify message was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        
        let (message, dest) = &sent_messages[0];
        assert_eq!(dest, &remote_addr);
        
        match message {
            Message::Request(req) => {
                assert_eq!(req.method(), Method::Invite);
            },
            _ => panic!("Expected Request message"),
        }
    }
    
    #[tokio::test]
    async fn test_client_invite_transaction_transport_error() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::with_failure(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let mut request = create_test_invite();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let mut transaction = ClientInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Initiate the transaction - should fail due to transport error
        let result = transaction.initiate().await;
        assert!(result.is_err());
        
        // Verify state is still Initial
        assert_eq!(transaction.state(), TransactionState::Initial);
    }
    
    #[tokio::test]
    async fn test_client_invite_transaction_process_response() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let mut request = create_test_invite();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let mut transaction = ClientInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Initiate the transaction
        transaction.initiate().await.unwrap();
        
        // Process a provisional response (100 Trying)
        let trying_response = create_response(&request, StatusCode::Trying);
        
        // Process the response
        transaction.process_response(trying_response).await.unwrap();
        
        // Verify state changed to Proceeding
        assert_eq!(transaction.state(), TransactionState::Proceeding);
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::ProvisionalResponse { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected ProvisionalResponse event, got {:?}", event),
        }
        
        // Process a final response (200 OK)
        let ok_response = create_response(&request, StatusCode::Ok);
        
        // Process the response
        transaction.process_response(ok_response).await.unwrap();
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::SuccessResponse { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected SuccessResponse event, got {:?}", event),
        }
        
        // Verify state changed to Terminated (RFC 3261 requires immediate termination for 2xx)
        assert_eq!(transaction.state(), TransactionState::Terminated);
    }
    
    #[tokio::test]
    async fn test_client_non_invite_transaction_creation() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, _events_rx) = mpsc::channel(10);
        
        let mut request = create_test_register();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let result = ClientNonInviteTransaction::new(
            transaction_id.clone(),
            request,
            remote_addr,
            mock_transport.clone(),
            events_tx,
        );
        
        assert!(result.is_ok());
        let transaction = result.unwrap();
        
        // Verify initial state
        assert_eq!(transaction.state(), TransactionState::Initial);
        assert_eq!(transaction.kind(), TransactionKind::NonInviteClient);
        assert_eq!(transaction.remote_addr(), remote_addr);
        assert_eq!(transaction.id(), &transaction_id);
    }
    
    #[tokio::test]
    async fn test_client_non_invite_transaction_initiate() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, _events_rx) = mpsc::channel(10);
        
        let mut request = create_test_register();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let mut transaction = ClientNonInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Initiate the transaction
        transaction.initiate().await.unwrap();
        
        // Verify state changed to Trying
        assert_eq!(transaction.state(), TransactionState::Trying);
        
        // Verify message was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        
        let (message, dest) = &sent_messages[0];
        assert_eq!(dest, &remote_addr);
        
        match message {
            Message::Request(req) => {
                assert_eq!(req.method(), Method::Register);
            },
            _ => panic!("Expected Request message"),
        }
    }
    
    #[tokio::test]
    async fn test_client_non_invite_transaction_process_response() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let mut request = create_test_register();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let mut transaction = ClientNonInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Initiate the transaction
        transaction.initiate().await.unwrap();
        
        // Process a provisional response (100 Trying)
        let trying_response = create_response(&request, StatusCode::Trying);
        
        // Process the response
        transaction.process_response(trying_response).await.unwrap();
        
        // Verify state changed to Proceeding
        assert_eq!(transaction.state(), TransactionState::Proceeding);
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::ProvisionalResponse { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected ProvisionalResponse event, got {:?}", event),
        }
        
        // Process a final response (200 OK)
        let ok_response = create_response(&request, StatusCode::Ok);
        
        // Process the response
        transaction.process_response(ok_response).await.unwrap();
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::SuccessResponse { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected SuccessResponse event, got {:?}", event),
        }
        
        // Verify state changed to Completed
        assert_eq!(transaction.state(), TransactionState::Completed);
    }
    
    #[tokio::test]
    async fn test_client_transaction_with_error_response() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let mut request = create_test_register();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let mut transaction = ClientNonInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Initiate the transaction
        transaction.initiate().await.unwrap();
        
        // Process an error response (404 Not Found)
        let not_found_response = create_response(&request, StatusCode::NotFound);
        
        // Process the response
        transaction.process_response(not_found_response).await.unwrap();
        
        // Verify state changed to Completed
        assert_eq!(transaction.state(), TransactionState::Completed);
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::FailureResponse { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected FailureResponse event"),
        }
    }
    
    #[tokio::test]
    async fn test_invite_client_transaction_matches() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, _) = mpsc::channel(10);
        
        let mut request = create_test_invite();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let transaction = ClientInviteTransaction::new(
            transaction_id,
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Create a matching response
        let response = create_response(&request, StatusCode::Ok);
        
        // Test matches method
        assert!(transaction.matches(&Message::Response(response.clone())));
        
        // Create a non-matching response with different branch
        let mut response2 = response.clone();
        let branch2 = utils::generate_branch();
        let via = Via::new(
            "SIP", 
            "2.0", 
            "UDP", 
            "127.0.0.1", 
            Some(5060),
            vec![Param::branch(&branch2)]
        ).unwrap();
        
        response2.headers.retain(|h| !matches!(h, TypedHeader::Via(_)));
        response2.headers.push(TypedHeader::Via(via));
        
        // This should not match
        assert!(!transaction.matches(&Message::Response(response2)));
    }

    #[tokio::test]
    async fn test_non_invite_transaction_state_transitions() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let mut request = create_test_register();
        let branch = utils::generate_branch();
        add_via_header(&mut request, &branch);
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let mut transaction = ClientNonInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Initiate the transaction
        transaction.initiate().await.unwrap();
        
        // Check initial state
        assert_eq!(transaction.state(), TransactionState::Trying);
        
        // Process 100 Trying response
        let trying_response = create_response(&request, StatusCode::Trying);
        transaction.process_response(trying_response).await.unwrap();
        
        // State should transition to PROCEEDING
        assert_eq!(transaction.state(), TransactionState::Proceeding);
        
        // Check event was received
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::ProvisionalResponse { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected ProvisionalResponse event"),
        }
        
        // Process 200 OK
        let ok_response = create_response(&request, StatusCode::Ok);
        transaction.process_response(ok_response).await.unwrap();
        
        // State should be Completed
        assert_eq!(transaction.state(), TransactionState::Completed);
        
        // Check event was received
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::SuccessResponse { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected SuccessResponse event"),
        }
        
        // Create a new transaction to test error path
        let (events_tx, mut events_rx) = mpsc::channel(10);
        let mut request = create_test_register();
        add_via_header(&mut request, &branch);
        
        let mut transaction = ClientNonInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Initiate the transaction
        transaction.initiate().await.unwrap();
        
        // Process 404 Not Found
        let not_found_response = create_response(&request, StatusCode::NotFound);
        transaction.process_response(not_found_response).await.unwrap();
        
        // State should be Completed
        assert_eq!(transaction.state(), TransactionState::Completed);
        
        // Check event was received
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::FailureResponse { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected FailureResponse event"),
        }
    }
} 