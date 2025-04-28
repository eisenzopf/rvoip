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

// Constants for timers (RFC 3261)
const T1: Duration = Duration::from_millis(500);
const T2: Duration = Duration::from_secs(4);
const TIMER_B_INVITE_TIMEOUT: Duration = Duration::from_secs(32); // 64 * T1
const TIMER_D_WAIT_ACK: Duration = Duration::from_secs(32);      // > 32s for unreliable
const TIMER_F_NON_INVITE_TIMEOUT: Duration = Duration::from_secs(32); // 64 * T1
const TIMER_K_WAIT_RESPONSE: Duration = Duration::from_secs(5);   // For unreliable transport

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

        self.data.state = new_state;

        // Start timers for the new state
        self.start_timers_for_state(new_state);

        // Notify TU about state change (optional, depends on required granularity)
        // self.data.events_tx.send(TransactionEvent::StateChanged { transaction_id: self.data.id.clone(), state: new_state }).await?;

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
                      // Use status check methods
                      if !resp.status().is_success() {
                            self.start_timer_d();
                      } else {
                           let events_tx = self.data.events_tx.clone();
                           let id = self.data.id.clone();
                           // Spawn task directly, store JoinHandle
                            self.timer_d_task = Some(tokio::spawn(async move {
                                tokio::time::sleep(Duration::from_millis(10)).await; // Very short delay
                                debug!(id=%id, "Short delay after 2xx completed, transitioning to Terminated");
                                // Send TimerTriggered event to manager to handle state change
                                let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "QuickTerminate".to_string() }).await;
                            }));
                      }
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
        let interval = TIMER_D_WAIT_ACK;
         let events_tx = self.data.events_tx.clone();
         let id = self.data.id.clone();
         self.timer_d_task = Some(tokio::spawn(async move {
             tokio::time::sleep(interval).await;
             debug!(id=%id, "Timer D fired");
             let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "D".to_string() }).await;
         }));
         trace!(id=%self.data.id, interval = ?interval, "Started Timer D");
     }


    /// Handle internal timer events dispatched from the manager.
    async fn on_timer(&mut self, timer: &str) -> Result<()> {
        match timer {
            "A" => {
                // Timer A logic (retransmit INVITE, double interval, restart A)
                 if self.data.state == TransactionState::Calling {
                      debug!(id=%self.data.id, "Timer A triggered in Calling state, retransmitting INVITE");
                      // Retransmit request
                      self.data.transport.send_message(
                          Message::Request(self.data.request.clone()),
                          self.data.remote_addr
                      ).await.map_err(|e| Error::TransportError(e.to_string()))?; // Map transport error

                      // Double interval, capped by T2
                      self.timer_a_interval = std::cmp::min(self.timer_a_interval * 2, T2);
                      // Restart timer A
                      self.start_timer_a();
                 } else {
                    trace!(id=%self.data.id, state=?self.data.state, "Timer A fired in non-Calling state, ignoring.");
                 }
            }
            "B" => {
                // Timer B logic (timeout)
                if self.data.state == TransactionState::Calling || self.data.state == TransactionState::Proceeding {
                    warn!(id=%self.data.id, "Timer B (Timeout) fired");
                    self.transition_to(TransactionState::Terminated).await?; // Terminate on timeout
                    // Inform TU about the timeout
                     self.data.events_tx.send(TransactionEvent::TransactionTimeout {
                        transaction_id: self.data.id.clone(),
                    }).await?; // mpsc send error converted in error.rs
                 } else {
                     trace!(id=%self.data.id, state=?self.data.state, "Timer B fired in invalid state, ignoring.");
                 }
            }
            "D" => {
                // Timer D logic (terminate after waiting for ACK retransmissions)
                if self.data.state == TransactionState::Completed {
                     debug!(id=%self.data.id, "Timer D fired in Completed state, terminating");
                     self.transition_to(TransactionState::Terminated).await?;
                     // No specific event needed, TU was already informed of final response
                } else {
                     trace!(id=%self.data.id, state=?self.data.state, "Timer D fired in invalid state, ignoring.");
                 }
            }
            "QuickTerminate" => {
                // Special timer for terminating quickly after 2xx
                if self.data.state == TransactionState::Completed {
                    debug!(id=%self.data.id, "QuickTerminate timer fired, terminating");
                     self.transition_to(TransactionState::Terminated).await?;
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

        match self.data.state {
            TransactionState::Calling => {
                self.cancel_timers();
                if is_provisional { /* ... */ }
                else if is_success { /* ... */ }
                else if is_failure { /* ... */ }
            }
            TransactionState::Proceeding => {
                 if is_provisional { /* ... */ }
                 else if is_success { /* ... */ }
                 else if is_failure { /* ... */ }
            }
            TransactionState::Completed => {
                 if is_failure { /* ... */ }
                 else { /* ... */ }
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

        self.data.state = new_state;
        self.start_timers_for_state(new_state);
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
        self.timer_k_task = Some(tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            debug!(id=%id, "Timer K fired");
             let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "K".to_string() }).await;
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
                     warn!(id=%self.data.id, "Timer F (Timeout) fired");
                     self.transition_to(TransactionState::Terminated).await?;
                     // Inform TU
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
                     self.transition_to(TransactionState::Terminated).await?;
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

        match self.data.state {
             TransactionState::Trying => {
                 self.cancel_timers();
                 if is_provisional { /* ... */ }
                 else if is_final { /* ... */
                     if is_success { /* ... */ }
                     else { /* is_failure */ /* ... */ }
                 }
             }
             TransactionState::Proceeding => {
                 if is_provisional { /* ... */ }
                 else if is_final { /* ... */
                     if is_success { /* ... */ }
                     else { /* is_failure */ /* ... */ }
                 }
             }
             TransactionState::Completed | TransactionState::Terminated | TransactionState::Initial | TransactionState::Calling | TransactionState::Confirmed => {
                 trace!(id=%id, state=?self.data.state, %status, "Ignoring response");
             }
        }
        Ok(())
    }
} 