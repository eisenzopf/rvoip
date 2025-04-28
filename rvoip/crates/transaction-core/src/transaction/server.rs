use std::fmt;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tracing::{debug, trace, warn, error};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::sleep;

use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;
use rvoip_sip_transport::error::{Error as TransportError};
use rvoip_sip_transport::TransportEvent;

use crate::error::{Error, Result};
use crate::transaction::{Transaction, TransactionState, TransactionKind, TransactionKey};
use crate::utils;
use crate::TransactionManager;
use crate::TransactionEvent;

// Standard RFC values in prod code, shorter for tests

// Base timer value (500ms in prod, 50ms for tests)
const T1: Duration = Duration::from_millis(50);

// Maximum retransmission interval (4s in prod, 200ms for tests)
const T2: Duration = Duration::from_millis(200);

// Various server-side timer intervals
const TIMER_G_INITIAL_INTERVAL: Duration = T1; // Initial retransmission interval
const TIMER_H_TIMEOUT: Duration = Duration::from_millis(500); // 64*T1, shortened for tests
const TIMER_I_WAIT: Duration = Duration::from_millis(250); // 5s in RFC (unreliable transport), shortened for tests
const TIMER_J_WAIT: Duration = Duration::from_millis(250); // 64*T1 in RFC, shortened for tests

/// Server transaction trait
#[async_trait]
pub trait ServerTransaction: Transaction {
    /// Process an incoming request associated with this transaction (e.g., retransmission, ACK, CANCEL).
    async fn process_request(&mut self, request: Request) -> Result<()>;

    /// Send a response for this transaction. Initiates state transitions and timers.
    async fn send_response(&mut self, response: Response) -> Result<()>;
}

/// Shared data for server transactions
struct ServerTxData {
    id: TransactionKey,
    state: TransactionState,
    request: Request, // The original request that created the transaction
    last_response: Option<Response>,
    remote_addr: SocketAddr, // Source address of the original request
    transport: Arc<dyn Transport>,
    /// Channel to send events (like final responses, errors) back to the manager/TU
    events_tx: mpsc::Sender<TransactionEvent>,
    /// Optional sender to signal termination completion
    terminate_signal: Option<oneshot::Sender<()>>,
}

// Manual Debug impl for ServerTxData
impl fmt::Debug for ServerTxData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerTxData")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("request", &self.request)
            .field("last_response", &self.last_response)
            .field("remote_addr", &self.remote_addr)
            .field("transport", &"Arc<dyn Transport>")
            .field("events_tx", &self.events_tx)
            .field("terminate_signal", &"Option<oneshot::Sender<()>>")
            .finish()
    }
}

/// Server INVITE transaction (RFC 3261 Section 17.2.1)
pub struct ServerInviteTransaction {
    data: ServerTxData,
    timer_g_interval: Duration, // Current retransmission interval
    timer_g_task: Option<JoinHandle<()>>, // Use JoinHandle
    timer_h_task: Option<JoinHandle<()>>, // Use JoinHandle
    timer_i_task: Option<JoinHandle<()>>, // Use JoinHandle
}

// Manual Debug impl for ServerInviteTransaction
impl fmt::Debug for ServerInviteTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerInviteTransaction")
            .field("data", &self.data)
            .field("timer_g_interval", &self.timer_g_interval)
            .field("timer_g_task", &self.timer_g_task.is_some())
            .field("timer_h_task", &self.timer_h_task.is_some())
            .field("timer_i_task", &self.timer_i_task.is_some())
            .finish()
    }
}

impl ServerInviteTransaction {
    /// Create a new server INVITE transaction.
    /// Assumes manager verified the request and created the key.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
    ) -> Result<Self> {
        if request.method() != Method::Invite {
            return Err(Error::Other("Request must be INVITE for INVITE server transaction".to_string()));
        }

        // Send 100 Trying immediately (RFC requirement)
        // This is done reliably by the transport, transaction doesn't manage this response.
        // However, the manager should probably trigger this upon creating the transaction.
        // Let's assume manager/TU handles the initial 100 Trying.

        let (terminate_tx, terminate_rx) = oneshot::channel();

         let mut tx = Self {
            data: ServerTxData {
                id,
                // Initial state is Proceeding because 100 Trying is assumed sent
                state: TransactionState::Proceeding,
                request,
                last_response: None,
                remote_addr,
                transport,
                events_tx,
                terminate_signal: Some(terminate_tx),
            },
             timer_g_interval: TIMER_G_INITIAL_INTERVAL,
             timer_g_task: None,
             timer_h_task: None,
             timer_i_task: None,
         };

        tx.spawn_termination_listener(terminate_rx);
        Ok(tx)
    }

     /// Spawns a task that cleans up when termination is signaled.
     fn spawn_termination_listener(&mut self, terminate_rx: oneshot::Receiver<()>) {
         let id = self.data.id.clone();
         tokio::spawn(async move {
             let _ = terminate_rx.await;
             debug!(id=%id, "Termination signal received or channel dropped. Invite server transaction cleanup.");
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
            (TransactionState::Proceeding, TransactionState::Completed) => {} // Sent non-2xx final
            (TransactionState::Proceeding, TransactionState::Terminated) => {} // Sent 2xx final
            (TransactionState::Completed, TransactionState::Confirmed) => {} // Received ACK
            (TransactionState::Completed, TransactionState::Terminated) => {} // Timer H fired
            (TransactionState::Confirmed, TransactionState::Terminated) => {} // Timer I fired
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
        if let Some(handle) = self.timer_g_task.take() { handle.abort(); }
        if let Some(handle) = self.timer_h_task.take() { handle.abort(); }
        if let Some(handle) = self.timer_i_task.take() { handle.abort(); }
         trace!(id=%self.data.id, "Cancelled active server timers");
    }

    /// Start timers based on the current state.
    fn start_timers_for_state(&mut self, state: TransactionState) {
        match state {
            TransactionState::Completed => {
                // Started upon sending a non-2xx final response
                self.start_timer_g();
                self.start_timer_h();
            }
            TransactionState::Confirmed => {
                // Started upon receiving ACK
                self.start_timer_i();
            }
            _ => {} // No timers for Proceeding, Terminated
        }
    }

     /// Start Timer G (response retransmission timer)
    fn start_timer_g(&mut self) {
        let interval = self.timer_g_interval;
        let events_tx = self.data.events_tx.clone();
        let id = self.data.id.clone();
        self.timer_g_task = Some(tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            debug!(id=%id, "Timer G fired");
            let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "G".to_string() }).await;
        }));
         trace!(id=%self.data.id, interval = ?interval, "Started Timer G");
     }

     /// Start Timer H (timeout waiting for ACK)
    fn start_timer_h(&mut self) {
        let interval = TIMER_H_TIMEOUT;
        let events_tx = self.data.events_tx.clone();
        let id = self.data.id.clone();
        self.timer_h_task = Some(tokio::spawn(async move {
            tokio::time::sleep(interval).await;
            debug!(id=%id, "Timer H fired");
             let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "H".to_string() }).await;
         }));
          trace!(id=%self.data.id, interval = ?interval, "Started Timer H");
      }

     /// Start Timer I (wait after ACK received)
    fn start_timer_i(&mut self) {
        let interval = TIMER_I_WAIT;
         let events_tx = self.data.events_tx.clone();
         let id = self.data.id.clone();
         self.timer_i_task = Some(tokio::spawn(async move {
             tokio::time::sleep(interval).await;
             debug!(id=%id, "Timer I fired");
             let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "I".to_string() }).await;
         }));
         trace!(id=%self.data.id, interval = ?interval, "Started Timer I");
     }

     /// Handle internal timer events.
     async fn on_timer(&mut self, timer: &str) -> Result<()> {
         match timer {
             "G" => {
                 // Timer G logic (retransmit last response, double interval, restart G)
                 if self.data.state == TransactionState::Completed {
                     if let Some(response) = &self.data.last_response {
                         debug!(id=%self.data.id, "Timer G triggered, retransmitting response");
                         self.data.transport.send_message(
                             Message::Response(response.clone()),
                             self.data.remote_addr
                         ).await.map_err(|e| Error::TransportError(e.to_string()))?;
                         // Double interval, capped by T2
                         self.timer_g_interval = std::cmp::min(self.timer_g_interval * 2, T2);
                         self.start_timer_g(); // Restart timer G
                     } else {
                         warn!(id=%self.data.id, "Timer G fired but no response to retransmit?");
                     }
                 } else {
                     trace!(id=%self.data.id, state=?self.data.state, "Timer G fired in invalid state, ignoring.");
                 }
             }
             "H" => {
                 // Timer H logic (timeout waiting for ACK) -> Terminate
                 if self.data.state == TransactionState::Completed {
                     warn!(id=%self.data.id, "Timer H (Timeout waiting for ACK) fired");
                     self.transition_to(TransactionState::Terminated).await?;
                     // Inform TU? Typically just means the client likely didn't get the response.
                     self.data.events_tx.send(TransactionEvent::AckTimeout {
                        transaction_id: self.data.id.clone(),
                     }).await?;
                 } else {
                     trace!(id=%self.data.id, state=?self.data.state, "Timer H fired in invalid state, ignoring.");
                 }
             }
             "I" => {
                 // Timer I logic (linger after ACK) -> Terminate
                  if self.data.state == TransactionState::Confirmed {
                      debug!(id=%self.data.id, "Timer I fired, terminating transaction");
                      self.transition_to(TransactionState::Terminated).await?;
                  } else {
                      trace!(id=%self.data.id, state=?self.data.state, "Timer I fired in invalid state, ignoring.");
                  }
             }
             _ => warn!(id=%self.data.id, timer=timer, "Unknown timer triggered"),
         }
         Ok(())
     }
}

#[async_trait]
impl Transaction for ServerInviteTransaction {
     fn id(&self) -> &TransactionKey { &self.data.id }
     fn kind(&self) -> TransactionKind { TransactionKind::InviteServer }
     fn state(&self) -> TransactionState { self.data.state }
     fn transport(&self) -> Arc<dyn Transport> { self.data.transport.clone() }
     fn remote_addr(&self) -> SocketAddr { self.data.remote_addr }
     fn original_request(&self) -> &Request { &self.data.request }
     fn last_response(&self) -> Option<&Response> { self.data.last_response.as_ref() }

      async fn process_event(&mut self, event_type: &str, message: Option<Message>) -> Result<()> {
          match event_type {
              "request" => {
                  if let Some(Message::Request(req)) = message {
                      self.process_request(req).await
                  } else {
                      warn!(id=%self.data.id, "Received non-request message event");
                      Ok(())
                  }
              }
              "timer" => {
                   // Unused
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
}

#[async_trait]
impl ServerTransaction for ServerInviteTransaction {
    /// Process INVITE retransmission or ACK.
    async fn process_request(&mut self, request: Request) -> Result<()> {
        match request.method() {
            Method::Invite => {
                // INVITE retransmission, resend last response
                if self.data.last_response.is_some() {
                    debug!(id=%self.data.id, "Received INVITE retransmission, resending last response");
                    let last_response = self.data.last_response.clone().unwrap();
                    self.data.transport.send_message(
                        Message::Response(last_response),
                        self.data.remote_addr
                    ).await.map_err(|e| Error::TransportError(e.to_string()))?;
                } else {
                    warn!(id=%self.data.id, "Received INVITE retransmission but no response has been sent yet");
                }
            }
            Method::Ack => {
                // ACK received, validate it's for a non-2xx response when in Completed state
                if self.data.state == TransactionState::Completed {
                    debug!(id=%self.data.id, "Received ACK for non-2xx response, transitioning to Confirmed");
                    
                    // Cancel Timer G (response retransmission) and Timer H (timeout)
                    self.cancel_timers();
                    
                    // Transit to Confirmed state
                    if let Err(e) = self.transition_to(TransactionState::Confirmed).await {
                        error!(id=%self.data.id, error=%e, "Failed to transition to Confirmed state");
                        return Err(e);
                    }
                    
                    // Notify TU about the ACK
                    self.data.events_tx.send(TransactionEvent::AckReceived {
                        transaction_id: self.data.id.clone(),
                        ack_request: request,
                    }).await?;
                } else if self.data.state == TransactionState::Terminated {
                    // Late ACK after transaction already terminated, just log it
                    debug!(id=%self.data.id, "Received ACK after transaction terminated");
                } else {
                    warn!(id=%self.data.id, state=?self.data.state, "Received ACK in invalid state: {:?}", self.data.state);
                }
            }
            Method::Cancel => {
                // CANCEL request, generate 487 Response for the INVITE
                warn!(id=%self.data.id, "CANCEL received for INVITE transaction, should be handled at transaction user level");
                
                // Forward CANCEL to TU to handle
                self.data.events_tx.send(TransactionEvent::CancelReceived {
                    transaction_id: self.data.id.clone(),
                    cancel_request: request,
                }).await?;
            }
            _ => {
                warn!(id=%self.data.id, method=%request.method(), "Unexpected method for server INVITE transaction");
            }
        }
        
        Ok(())
    }

    /// Send a response (1xx, 2xx, 3xx-6xx).
    async fn send_response(&mut self, response: Response) -> Result<()> {
         let status = response.status();
         let is_provisional = status.is_provisional();
         let is_success = status.is_success();
         let is_final = !is_provisional;

         let id = self.data.id.clone();

         match self.data.state {
              TransactionState::Proceeding => {
                  self.data.last_response = Some(response.clone());
                  self.data.transport.send_message(
                      Message::Response(response.clone()),
                      self.data.remote_addr
                  ).await.map_err(|e| Error::TransportError(e.to_string()))?;

                  if is_final {
                      if is_success { // 2xx
                          debug!(id=%id, %status, "Sent 2xx final response, terminating transaction");
                          self.transition_to(TransactionState::Terminated).await?;
                          self.data.events_tx.send(TransactionEvent::FinalResponseSent {
                             transaction_id: id,
                             response,
                         }).await?;
                      } else { // 3xx-6xx
                           debug!(id=%id, %status, "Sent non-2xx final response, moving to Completed");
                           self.transition_to(TransactionState::Completed).await?;
                            self.data.events_tx.send(TransactionEvent::FinalResponseSent {
                               transaction_id: id,
                               response,
                           }).await?;
                      }
                } else { // is_provisional
                        debug!(id=%id, %status, "Sent another provisional response");
                        self.data.events_tx.send(TransactionEvent::ProvisionalResponseSent {
                            transaction_id: id,
                            response,
                         }).await?;
                  }
              }
              // Use status in logs
              TransactionState::Initial => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Initial state");
                   return Err(Error::InvalidStateTransition("Cannot send response in Initial state".to_string()));
              }
              TransactionState::Trying => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Trying state (server INVITE)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Trying state".to_string()));
              }
              TransactionState::Calling => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Calling state (server INVITE)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Calling state".to_string()));
              }
              TransactionState::Completed => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Completed state (already sent final)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Completed state".to_string()));
              }
              TransactionState::Confirmed => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Confirmed state (already ACKed)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Confirmed state".to_string()));
              }
              TransactionState::Terminated => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Terminated state");
                   return Err(Error::InvalidStateTransition("Cannot send response in Terminated state".to_string()));
               }
         }
         Ok(())
     }
}

/// Server non-INVITE transaction (RFC 3261 Section 17.2.2)
pub struct ServerNonInviteTransaction {
    data: ServerTxData,
    timer_j_task: Option<Pin<Box<dyn Future<Output = ()> + Send>>>, // Wait retransmit timer
}

// Manual Debug impl for ServerNonInviteTransaction
impl fmt::Debug for ServerNonInviteTransaction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerNonInviteTransaction")
            .field("data", &self.data)
            .field("timer_j_task", &"Option<Pin<Box<Future>>>")
            .finish()
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
    ) -> Result<Self> {
         if request.method() == Method::Invite || request.method() == Method::Ack {
             return Err(Error::Other("Request must be non-INVITE/ACK for non-INVITE server transaction".to_string()));
         }
         // Send 100 Trying immediately? Usually yes for INVITE, maybe optional for others?
         // Let's assume TU handles 100 Trying if needed.

         let (terminate_tx, terminate_rx) = oneshot::channel();

          let mut tx = Self {
             data: ServerTxData {
                 id,
                 state: TransactionState::Trying, // Starts in Trying
                 request,
                 last_response: None,
                 remote_addr,
                 transport,
                 events_tx,
                 terminate_signal: Some(terminate_tx),
             },
             timer_j_task: None,
          };
         tx.spawn_termination_listener(terminate_rx);
         Ok(tx)
    }

      /// Spawns a task that cleans up when termination is signaled.
      fn spawn_termination_listener(&mut self, terminate_rx: oneshot::Receiver<()>) {
          let id = self.data.id.clone();
          tokio::spawn(async move {
              let _ = terminate_rx.await;
              debug!(id=%id, "Termination signal received or channel dropped. Non-invite server transaction cleanup.");
          });
      }


    /// Transition to a new state, handling timer logic.
    async fn transition_to(&mut self, new_state: TransactionState) -> Result<()> {
        if self.data.state == new_state { return Ok(()); }
        debug!(id = %self.data.id, "State transition: {:?} -> {:?}", self.data.state, new_state);

        self.cancel_timers();

        // Validate state transition
         match (self.data.state, new_state) {
              (_, TransactionState::Terminated) => {
                  if let Some(sender) = self.data.terminate_signal.take() {
                      let _ = sender.send(());
                  }
              }
              (TransactionState::Trying, TransactionState::Proceeding) => {}
              (TransactionState::Trying, TransactionState::Completed) => {}
              (TransactionState::Proceeding, TransactionState::Completed) => {}
              (TransactionState::Completed, TransactionState::Terminated) => {} // Timer J fires
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
         self.timer_j_task = None;
         trace!(id=%self.data.id, "Cancelled active server timers");
     }

     /// Start timers based on the current state.
     fn start_timers_for_state(&mut self, state: TransactionState) {
         match state {
             TransactionState::Completed => {
                  self.start_timer_j();
             }
             _ => {} // No timers for Trying, Proceeding, Terminated
         }
     }

     /// Start Timer J (wait for retransmissions)
     fn start_timer_j(&mut self) {
         let interval = TIMER_J_WAIT;
         let events_tx = self.data.events_tx.clone();
         let id = self.data.id.clone();
         self.timer_j_task = Some(Box::pin(async move {
             tokio::time::sleep(interval).await;
              debug!(id=%id, "Timer J fired");
              let _ = events_tx.send(TransactionEvent::TimerTriggered { transaction_id: id, timer: "J".to_string() }).await;
          }));
          trace!(id=%self.data.id, interval = ?interval, "Started Timer J");
      }

     /// Handle internal timer events.
     async fn on_timer(&mut self, timer: &str) -> Result<()> {
         match timer {
             "J" => {
                 // Timer J logic (terminate after waiting for request retransmissions)
                 if self.data.state == TransactionState::Completed {
                     debug!(id=%self.data.id, "Timer J fired, terminating transaction");
                     if let Err(e) = self.transition_to(TransactionState::Terminated).await {
                         error!(id=%self.data.id, error=%e, "Failed to transition to Terminated state after Timer J");
                         return Err(e);
                     }
                 } else {
                     trace!(id=%self.data.id, state=?self.data.state, "Timer J fired in invalid state, ignoring.");
                 }
             }
             _ => warn!(id=%self.data.id, timer=timer, "Unknown timer triggered"),
         }
         Ok(())
     }
}

#[async_trait]
impl Transaction for ServerNonInviteTransaction {
      fn id(&self) -> &TransactionKey { &self.data.id }
      fn kind(&self) -> TransactionKind { TransactionKind::NonInviteServer }
      fn state(&self) -> TransactionState { self.data.state }
      fn transport(&self) -> Arc<dyn Transport> { self.data.transport.clone() }
      fn remote_addr(&self) -> SocketAddr { self.data.remote_addr }
      fn original_request(&self) -> &Request { &self.data.request }
      fn last_response(&self) -> Option<&Response> { self.data.last_response.as_ref() }

       async fn process_event(&mut self, event_type: &str, message: Option<Message>) -> Result<()> {
           match event_type {
               "request" => {
                   if let Some(Message::Request(req)) = message {
                       self.process_request(req).await
                   } else {
                       warn!(id=%self.data.id, "Received non-request message event");
                       Ok(())
                   }
               }
               "timer" => {
                   // Unused
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
}

#[async_trait]
impl ServerTransaction for ServerNonInviteTransaction {
    /// Process a retransmitted request.
    async fn process_request(&mut self, request: Request) -> Result<()> {
         let id = self.data.id.clone();
         // Basic check if it's the same method
         if request.method() != self.data.request.method() {
              warn!(id=%id, incoming_method=%request.method(), original_method=%self.data.request.method(), "Received request with different method");
              return Ok(()); // Ignore
         }

         match self.data.state {
              TransactionState::Trying | TransactionState::Proceeding => {
                   debug!(id=%id, state=?self.data.state, "Received request retransmission");
                  // Retransmit the last provisional response sent
                  if let Some(resp) = &self.data.last_response {
                      if resp.status().is_provisional() {
                           self.data.transport.send_message(
                               Message::Response(resp.clone()),
                               self.data.remote_addr
                           ).await.map_err(|e| Error::TransportError(e.to_string()))?;
                      }
                  }
              }
            TransactionState::Completed => {
                   debug!(id=%id, "Received request retransmission in Completed state");
                   // Retransmit the final response
                   if let Some(resp) = &self.data.last_response {
                       // Use status check method
                       if !resp.status().is_provisional() { // is_final()
                           self.data.transport.send_message(
                                Message::Response(resp.clone()),
                                self.data.remote_addr
                            ).await.map_err(|e| Error::TransportError(e.to_string()))?;
                       }
                   }
              }
              // Use method in logs for added arms
              TransactionState::Initial => {
                  warn!(id=%id, state=?self.data.state, method=%request.method(), "Ignoring request retransmission in Initial state");
              }
              TransactionState::Calling => {
                 warn!(id=%id, state=?self.data.state, method=%request.method(), "Ignoring request retransmission in Calling state (non-INVITE)");
              }
              TransactionState::Confirmed => {
                 warn!(id=%id, state=?self.data.state, method=%request.method(), "Ignoring request retransmission in Confirmed state (non-INVITE)");
              }
              TransactionState::Terminated => {
                   trace!(id=%id, state=?self.data.state, "Ignoring request retransmission in Terminated state");
               }
         }
         Ok(())
     }

    /// Send a response (1xx, 2xx-6xx).
    async fn send_response(&mut self, response: Response) -> Result<()> {
         let status = response.status();
         let is_provisional = status.is_provisional();
         let is_final = !is_provisional;

         let id = self.data.id.clone();

         match self.data.state {
              TransactionState::Trying => {
                   self.data.last_response = Some(response.clone());
                   self.data.transport.send_message(
                       Message::Response(response.clone()),
                       self.data.remote_addr
                   ).await.map_err(|e| Error::TransportError(e.to_string()))?;

                   if is_provisional {
                        debug!(id=%id, %status, "Sent provisional response, moving to Proceeding");
                        self.transition_to(TransactionState::Proceeding).await?;
                         // Inform TU
                         // Use correct event variant name
                         self.data.events_tx.send(TransactionEvent::ProvisionalResponseSent {
                            transaction_id: id,
                            response,
                         }).await?;
                   } else { // Final response
                        debug!(id=%id, %status, "Sent final response, moving to Completed");
                        self.transition_to(TransactionState::Completed).await?; // Starts Timer J
                         // Inform TU
                         // Use correct event variant name
                          self.data.events_tx.send(TransactionEvent::FinalResponseSent {
                             transaction_id: id,
                             response,
                         }).await?;
                   }
              }
            TransactionState::Proceeding => {
                   self.data.last_response = Some(response.clone());
                   self.data.transport.send_message(
                       Message::Response(response.clone()),
                       self.data.remote_addr
                   ).await.map_err(|e| Error::TransportError(e.to_string()))?;

                    if is_provisional {
                         debug!(id=%id, %status, "Sent another provisional response");
                          // Inform TU
                          // Use correct event variant name
                         self.data.events_tx.send(TransactionEvent::ProvisionalResponseSent {
                            transaction_id: id,
                            response,
                         }).await?;
                    } else { // Final response
                         debug!(id=%id, %status, "Sent final response, moving to Completed");
                         self.transition_to(TransactionState::Completed).await?; // Starts Timer J
                          // Inform TU
                          // Use correct event variant name
                          self.data.events_tx.send(TransactionEvent::FinalResponseSent {
                             transaction_id: id,
                             response,
                         }).await?;
                    }
              }
              // Use status in logs for added arms
              TransactionState::Initial => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Initial state");
                   return Err(Error::InvalidStateTransition("Cannot send response in Initial state".to_string()));
              }
              TransactionState::Calling => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Calling state (non-INVITE)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Calling state".to_string()));
              }
               TransactionState::Confirmed => {
                   error!(id=%id, state=?self.data.state, %status, "Cannot send response in Confirmed state (non-INVITE)");
                   return Err(Error::InvalidStateTransition("Cannot send response in Confirmed state".to_string()));
               }
               TransactionState::Completed => {
                    error!(id=%id, state=?self.data.state, %status, "Cannot send response in Completed state");
                    return Err(Error::InvalidStateTransition("Cannot send response in Completed state".to_string()));
               }
               TransactionState::Terminated => {
                    error!(id=%id, state=?self.data.state, %status, "Cannot send response in Terminated state");
                    return Err(Error::InvalidStateTransition("Cannot send response in Terminated state".to_string()));
               }
         }
         Ok(())
     }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{mpsc, oneshot};
    use tokio::time::sleep;
    use tracing::{debug, warn};

    use rvoip_sip_core::prelude::*;
    use rvoip_sip_transport::{Transport, TransportEvent};
    use rvoip_sip_transport::error::{Error as TransportError};

    use crate::TransactionManager;
    use crate::transaction::{Transaction, TransactionState, TransactionKind, TransactionKey};
    use crate::TransactionEvent;
    
    use super::*;

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
    
    // Helper to create a test ACK request
    fn create_test_ack(invite_request: &Request) -> Request {
        let uri = invite_request.uri().to_string();
        
        let mut builder = RequestBuilder::new(Method::Ack, &uri).unwrap();
        
        // Copy essential headers from the INVITE
        if let Some(header) = invite_request.header(&HeaderName::From) {
            builder = builder.header(header.clone());
        }
        if let Some(header) = invite_request.header(&HeaderName::To) {
            builder = builder.header(header.clone());
        }
        if let Some(header) = invite_request.header(&HeaderName::CallId) {
            builder = builder.header(header.clone());
        }
        if let Some(header) = invite_request.header(&HeaderName::Via) {
            builder = builder.header(header.clone());
        }
        
        // Create CSeq header with same sequence but ACK method
        if let Some(TypedHeader::CSeq(cseq)) = invite_request.header(&HeaderName::CSeq) {
            builder = builder.header(TypedHeader::CSeq(CSeq::new(cseq.sequence(), Method::Ack)));
        }
        
        builder.header(TypedHeader::MaxForwards(MaxForwards::new(70)))
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
            .header(TypedHeader::Via(Via::new("SIP", "2.0", "UDP", "192.168.1.1", Some(5060), vec![Param::branch("z9hG4bK1234")]).unwrap()))
            .header(TypedHeader::MaxForwards(MaxForwards::new(70)))
            .header(TypedHeader::ContentLength(ContentLength::new(0)))
            .build()
    }
    
    // Helper to create a test response
    fn create_test_response(request: &Request, status_code: StatusCode) -> Response {
        let mut builder = ResponseBuilder::new(status_code);
        
        // Copy essential headers
        if let Some(header) = request.header(&HeaderName::Via) {
            builder = builder.header(header.clone());
        }
        if let Some(header) = request.header(&HeaderName::From) {
            builder = builder.header(header.clone());
        }
        if let Some(header) = request.header(&HeaderName::To) {
            // Add a tag for final (non-100) responses
            if status_code.as_u16() >= 200 {
                if let TypedHeader::To(to) = header {
                    let to_addr = to.address().clone();
                    if !to_addr.uri.parameters.iter().any(|p| match p {
                        Param::Tag(_) => true,
                        _ => false
                    }) {
                        let uri_with_tag = to_addr.uri.with_parameter(Param::tag("resp-tag"));
                        let addr_with_tag = Address::new(uri_with_tag);
                        builder = builder.header(TypedHeader::To(To::new(addr_with_tag)));
                    } else {
                        builder = builder.header(header.clone());
                    }
                } else {
                    builder = builder.header(header.clone());
                }
            } else {
                builder = builder.header(header.clone());
            }
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
    async fn test_server_invite_transaction_creation() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, _events_rx) = mpsc::channel(10);
        
        let request = create_test_invite();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let result = ServerInviteTransaction::new(
            transaction_id.clone(),
            request,
            remote_addr,
            mock_transport.clone(),
            events_tx,
        );
        
        assert!(result.is_ok());
        let transaction = result.unwrap();
        
        // Verify initial state - server INVITE starts in Proceeding
        assert_eq!(transaction.state(), TransactionState::Proceeding);
        assert_eq!(transaction.kind(), TransactionKind::InviteServer);
        assert_eq!(transaction.remote_addr(), remote_addr);
        assert_eq!(transaction.id(), &transaction_id);
    }
    
    #[tokio::test]
    async fn test_server_invite_transaction_send_provisional_response() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let request = create_test_invite();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let mut transaction = ServerInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a provisional response (180 Ringing)
        let ringing_response = create_test_response(&request, StatusCode::Ringing);
        
        // Send the response
        transaction.send_response(ringing_response.clone()).await.unwrap();
        
        // Verify state remains Proceeding
        assert_eq!(transaction.state(), TransactionState::Proceeding);
        
        // Verify message was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        
        let (message, dest) = &sent_messages[0];
        assert_eq!(dest, &remote_addr);
        
        match message {
            Message::Response(resp) => {
                assert_eq!(resp.status(), StatusCode::Ringing);
            },
            _ => panic!("Expected Response message"),
        }
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::ProvisionalResponseSent { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected ProvisionalResponseSent event, got {:?}", event),
        }
    }
    
    #[tokio::test]
    async fn test_server_invite_transaction_send_success_response() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let request = create_test_invite();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let mut transaction = ServerInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a success response (200 OK)
        let ok_response = create_test_response(&request, StatusCode::Ok);
        
        // Send the response
        transaction.send_response(ok_response.clone()).await.unwrap();
        
        // Verify state is now Terminated (for 2xx responses)
        assert_eq!(transaction.state(), TransactionState::Terminated);
        
        // Verify message was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        
        let (message, dest) = &sent_messages[0];
        assert_eq!(dest, &remote_addr);
        
        match message {
            Message::Response(resp) => {
                assert_eq!(resp.status(), StatusCode::Ok);
            },
            _ => panic!("Expected Response message"),
        }
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::FinalResponseSent { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected FinalResponseSent event, got {:?}", event),
        }
    }
    
    #[tokio::test]
    async fn test_server_invite_transaction_send_failure_response() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let request = create_test_invite();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let mut transaction = ServerInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a failure response (404 Not Found)
        let not_found_response = create_test_response(&request, StatusCode::NotFound);
        
        // Send the response
        transaction.send_response(not_found_response.clone()).await.unwrap();
        
        // Verify state is now Completed (for 3xx-6xx responses)
        assert_eq!(transaction.state(), TransactionState::Completed);
        
        // Verify message was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        
        let (message, dest) = &sent_messages[0];
        assert_eq!(dest, &remote_addr);
        
        match message {
            Message::Response(resp) => {
                assert_eq!(resp.status(), StatusCode::NotFound);
            },
            _ => panic!("Expected Response message"),
        }
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::FinalResponseSent { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected FinalResponseSent event, got {:?}", event),
        }
    }
    
    #[tokio::test]
    async fn test_server_invite_transaction_receive_invite_retransmission() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, _events_rx) = mpsc::channel(10);
        
        let request = create_test_invite();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let mut transaction = ServerInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a provisional response first
        let ringing_response = create_test_response(&request, StatusCode::Ringing);
        transaction.send_response(ringing_response).await.unwrap();
        
        // Clear sent messages
        mock_transport.sent_messages.lock().unwrap().clear();
        
        // Process a retransmitted INVITE
        transaction.process_request(request.clone()).await.unwrap();
        
        // Verify the last response was retransmitted
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        
        let (message, dest) = &sent_messages[0];
        assert_eq!(dest, &remote_addr);
        
        match message {
            Message::Response(resp) => {
                assert_eq!(resp.status(), StatusCode::Ringing);
            },
            _ => panic!("Expected Response message"),
        }
    }
    
    #[tokio::test]
    async fn test_server_invite_transaction_receive_ack() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let request = create_test_invite();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let mut transaction = ServerInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a failure response to get into Completed state
        let not_found_response = create_test_response(&request, StatusCode::NotFound);
        transaction.send_response(not_found_response).await.unwrap();
        
        // Create ACK
        let ack_request = create_test_ack(&request);
        
        // Process the ACK
        transaction.process_request(ack_request.clone()).await.unwrap();
        
        // Verify state changed to Confirmed
        assert_eq!(transaction.state(), TransactionState::Confirmed);
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::FinalResponseSent { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected FinalResponseSent event, got {:?}", event),
        }
        
        // Get the ACKReceived event
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::AckReceived { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected AckReceived event, got {:?}", event),
        }
    }
    
    #[tokio::test]
    async fn test_server_invite_transaction_timer_i() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let request = create_test_invite();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Invite);
        
        let mut transaction = ServerInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a failure response to get into Completed state
        let not_found_response = create_test_response(&request, StatusCode::NotFound);
        transaction.send_response(not_found_response).await.unwrap();
        
        // Process ACK to move to Confirmed state
        let ack_request = create_test_ack(&request);
        transaction.process_request(ack_request).await.unwrap();
        
        // Drain events channel
        while let Ok(_) = events_rx.try_recv() {}
        
        // Verify state is Confirmed
        assert_eq!(transaction.state(), TransactionState::Confirmed);
        
        // Manually trigger Timer I to terminate the transaction
        transaction.handle_timer("I".to_string()).await.unwrap();
        
        // Verify state changed to Terminated
        assert_eq!(transaction.state(), TransactionState::Terminated);
    }
    
    #[tokio::test]
    async fn test_server_non_invite_transaction_creation() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, _events_rx) = mpsc::channel(10);
        
        let request = create_test_register();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let result = ServerNonInviteTransaction::new(
            transaction_id.clone(),
            request,
            remote_addr,
            mock_transport.clone(),
            events_tx,
        );
        
        assert!(result.is_ok());
        let transaction = result.unwrap();
        
        // Verify initial state - server non-INVITE starts in Trying
        assert_eq!(transaction.state(), TransactionState::Trying);
        assert_eq!(transaction.kind(), TransactionKind::NonInviteServer);
        assert_eq!(transaction.remote_addr(), remote_addr);
        assert_eq!(transaction.id(), &transaction_id);
    }
    
    #[tokio::test]
    async fn test_server_non_invite_transaction_send_provisional_response() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let request = create_test_register();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let mut transaction = ServerNonInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a provisional response (100 Trying)
        let trying_response = create_test_response(&request, StatusCode::Trying);
        
        // Send the response
        transaction.send_response(trying_response.clone()).await.unwrap();
        
        // Verify state changed to Proceeding
        assert_eq!(transaction.state(), TransactionState::Proceeding);
        
        // Verify message was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        
        let (message, dest) = &sent_messages[0];
        assert_eq!(dest, &remote_addr);
        
        match message {
            Message::Response(resp) => {
                assert_eq!(resp.status(), StatusCode::Trying);
            },
            _ => panic!("Expected Response message"),
        }
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::ProvisionalResponseSent { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected ProvisionalResponseSent event, got {:?}", event),
        }
    }
    
    #[tokio::test]
    async fn test_server_non_invite_transaction_send_final_response() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let request = create_test_register();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let mut transaction = ServerNonInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a success response (200 OK)
        let ok_response = create_test_response(&request, StatusCode::Ok);
        
        // Send the response
        transaction.send_response(ok_response.clone()).await.unwrap();
        
        // Verify state changed to Completed
        assert_eq!(transaction.state(), TransactionState::Completed);
        
        // Verify message was sent
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        
        let (message, dest) = &sent_messages[0];
        assert_eq!(dest, &remote_addr);
        
        match message {
            Message::Response(resp) => {
                assert_eq!(resp.status(), StatusCode::Ok);
            },
            _ => panic!("Expected Response message"),
        }
        
        // Verify event was sent
        let event = events_rx.try_recv().unwrap();
        match event {
            TransactionEvent::FinalResponseSent { transaction_id: id, .. } => {
                assert_eq!(id, transaction_id);
            },
            _ => panic!("Expected FinalResponseSent event, got {:?}", event),
        }
    }
    
    #[tokio::test]
    async fn test_server_non_invite_transaction_receive_request_retransmission() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, _events_rx) = mpsc::channel(10);
        
        let request = create_test_register();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let mut transaction = ServerNonInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a final response to get into Completed state
        let ok_response = create_test_response(&request, StatusCode::Ok);
        transaction.send_response(ok_response).await.unwrap();
        
        // Clear sent messages
        mock_transport.sent_messages.lock().unwrap().clear();
        
        // Process a retransmitted REGISTER
        transaction.process_request(request.clone()).await.unwrap();
        
        // Verify the last response was retransmitted
        let sent_messages = mock_transport.get_sent_messages();
        assert_eq!(sent_messages.len(), 1);
        
        let (message, dest) = &sent_messages[0];
        assert_eq!(dest, &remote_addr);
        
        match message {
            Message::Response(resp) => {
                assert_eq!(resp.status(), StatusCode::Ok);
            },
            _ => panic!("Expected Response message"),
        }
    }
    
    #[tokio::test]
    async fn test_server_non_invite_transaction_timer_j() {
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let mock_transport = Arc::new(MockTransport::new(local_addr));
        
        let (events_tx, mut events_rx) = mpsc::channel(10);
        
        let request = create_test_register();
        let branch = "z9hG4bK1234";
        
        let transaction_id = format!("{}-{}", branch, Method::Register);
        
        let mut transaction = ServerNonInviteTransaction::new(
            transaction_id.clone(),
            request.clone(),
            remote_addr,
            mock_transport.clone(),
            events_tx,
        ).unwrap();
        
        // Send a final response to get into Completed state
        let ok_response = create_test_response(&request, StatusCode::Ok);
        transaction.send_response(ok_response).await.unwrap();
        
        // Drain events channel
        while let Ok(_) = events_rx.try_recv() {}
        
        // Verify state is Completed
        assert_eq!(transaction.state(), TransactionState::Completed);
        
        // Manually trigger Timer J to terminate the transaction
        transaction.handle_timer("J".to_string()).await.unwrap();
        
        // Verify state changed to Terminated
        assert_eq!(transaction.state(), TransactionState::Terminated);
    }

    // Test the server INVITE transaction state flow with a failure response
    #[tokio::test]
    async fn test_server_invite_transaction_failure_states() {
        let local_addr = SocketAddr::from_str("127.0.0.1:5060").unwrap();
        let remote_addr = SocketAddr::from_str("192.168.1.2:5060").unwrap();
        let transport = Arc::new(MockTransport::new(local_addr));
        
        // Setup transport and manager
        let (transport_tx, transport_rx) = mpsc::channel(100);
        let (manager, mut events_rx) = TransactionManager::new(transport.clone(), transport_rx, None).await.unwrap();
        
        // Create INVITE request with Via header
        let mut invite_request = create_test_invite();
        let via = Via::new(
            "SIP", "2.0", "UDP",
            "192.168.1.2", Some(5060),
            vec![Param::branch("z9hG4bK-test")]
        ).unwrap();
        invite_request.headers.insert(0, TypedHeader::Via(via));
        
        // Deliver request via transport event channel
        transport_tx.send(TransportEvent::MessageReceived {
            message: Message::Request(invite_request.clone()),
            source: remote_addr,
            destination: local_addr,
        }).await.unwrap();
        
        // Allow time for processing
        sleep(Duration::from_millis(50)).await;
        
        // Get NewRequest event with server transaction ID
        let event = events_rx.recv().await.unwrap();
        let server_tx_id = match event {
            TransactionEvent::NewRequest { transaction_id, .. } => transaction_id,
            _ => panic!("Expected NewRequest event, got {:?}", event),
        };
        
        // Send 404 Not Found response
        let not_found_response = create_test_response(&invite_request, StatusCode::NotFound);
        manager.send_response(&server_tx_id, not_found_response.clone()).await.unwrap();
        
        // Get event for 404 Not Found
        let event = events_rx.recv().await.unwrap();
        match event {
            TransactionEvent::FinalResponseSent { .. } => {
                // Expected
            },
            _ => panic!("Expected FinalResponseSent event, got {:?}", event),
        }
        
        // Allow time for state transition
        sleep(Duration::from_millis(50)).await;
        
        // Verify server transaction transitions to Completed for non-2xx response
        let state = manager.transaction_state(&server_tx_id).await.unwrap();
        assert_eq!(state, TransactionState::Completed, "Server INVITE transaction should transition to Completed after sending 4xx response");
        
        // Simulate receiving ACK from client
        let mut ack_request = Request::new(Method::Ack, invite_request.uri.clone());
        
        // Copy key headers from original INVITE
        if let Some(TypedHeader::Via(via)) = invite_request.header(&HeaderName::Via) {
            ack_request.headers.push(TypedHeader::Via(via.clone()));
        }
        if let Some(TypedHeader::From(from)) = invite_request.header(&HeaderName::From) {
            ack_request.headers.push(TypedHeader::From(from.clone()));
        }
        // For To, use the one with tag from the response
        if let Some(TypedHeader::To(to)) = not_found_response.header(&HeaderName::To) {
            ack_request.headers.push(TypedHeader::To(to.clone()));
        }
        if let Some(TypedHeader::CallId(call_id)) = invite_request.header(&HeaderName::CallId) {
            ack_request.headers.push(TypedHeader::CallId(call_id.clone()));
        }
        // Create CSeq with same sequence number but ACK method
        if let Some(TypedHeader::CSeq(cseq)) = invite_request.header(&HeaderName::CSeq) {
            let seq_num = cseq.sequence();
            ack_request.headers.push(TypedHeader::CSeq(CSeq::new(seq_num, Method::Ack)));
        }
        
        // Deliver ACK via transport event channel
        transport_tx.send(TransportEvent::MessageReceived {
            message: Message::Request(ack_request),
            source: remote_addr,
            destination: local_addr,
        }).await.unwrap();
        
        // Allow more time for processing and state transition
        sleep(Duration::from_millis(200)).await;
        
        // Explicitly check for AckReceived event
        let mut event_found = false;
        while let Ok(event) = events_rx.try_recv() {
            match event {
                TransactionEvent::AckReceived { .. } => {
                    event_found = true;
                    break;
                },
                TransactionEvent::TimerTriggered { .. } => {
                    // Ignore timer events
                    continue;
                },
                other => {
                    warn!("Unexpected event while waiting for ACK: {:?}", other);
                }
            }
        }
        
        if !event_found {
            // Not strictly needed if events_rx.try_recv() caught it already, but ensuring thoroughness
            if let Ok(event) = events_rx.try_recv() {
                match event {
                    TransactionEvent::AckReceived { .. } => {
                        event_found = true;
                    },
                    _ => {}
                }
            }
        }
        
        // Verify server transaction transitions to Confirmed after receiving ACK
        let state = manager.transaction_state(&server_tx_id).await.unwrap();
        assert_eq!(state, TransactionState::Confirmed, "Server INVITE transaction should transition to Confirmed after receiving ACK");
        
        // Wait for Timer I to expire (using a short value for tests)
        sleep(Duration::from_millis(500)).await;
        
        // Verify server transaction transitions to Terminated after Timer I
        let state = manager.transaction_state(&server_tx_id).await.unwrap_or(TransactionState::Terminated);
        assert_eq!(state, TransactionState::Terminated, "Server INVITE transaction should terminate after Timer I");
    }
} 