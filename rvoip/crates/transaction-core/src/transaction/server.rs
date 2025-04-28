use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Sleep;
use std::pin::Pin;
use std::future::Future;
use std::fmt;
use tracing::{debug, error, info, trace, warn};
use tokio::task::JoinHandle;

// Use prelude and specific types
use rvoip_sip_core::prelude::*;
use rvoip_sip_transport::Transport;

use crate::error::{Error, Result};
use crate::transaction::{Transaction, TransactionState, TransactionKind, TransactionKey, TransactionEvent};
use crate::utils;

// Constants for timers (RFC 3261)
const T1: Duration = Duration::from_millis(500);
const T2: Duration = Duration::from_secs(4);
const TIMER_G_INVITE_RETRANSMIT: Duration = T1;         // Starts at T1
const TIMER_H_WAIT_ACK: Duration = Duration::from_secs(32); // 64 * T1
const TIMER_I_ACK_RETRANSMIT: Duration = Duration::from_secs(5); // T4 for unreliable
const TIMER_J_NON_INVITE_WAIT: Duration = Duration::from_secs(32); // 64 * T1 for unreliable

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
             timer_g_interval: TIMER_G_INVITE_RETRANSMIT,
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
        let interval = TIMER_H_WAIT_ACK;
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
        let interval = TIMER_I_ACK_RETRANSMIT;
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
         let id = self.data.id.clone();

         match self.data.state {
             TransactionState::Proceeding => {
                  // Retransmission of INVITE
                  if request.method() == Method::Invite {
                      debug!(id=%id, "Received INVITE retransmission in Proceeding state");
                      // Retransmit last provisional response
                      if let Some(resp) = &self.data.last_response {
                           if resp.status().is_provisional() {
                                self.data.transport.send_message(
                                     Message::Response(resp.clone()),
                                     self.data.remote_addr
                                 ).await.map_err(|e| Error::TransportError(e.to_string()))?;
                           }
                      }
                  } else {
                       warn!(id=%id, method=%request.method(), "Received unexpected request in Proceeding state");
                  }
             }
             TransactionState::Completed => {
                 // Retransmission of INVITE or ACK
                 if request.method() == Method::Invite {
                      debug!(id=%id, "Received INVITE retransmission in Completed state");
                     // Retransmit last final (non-2xx) response
                     if let Some(resp) = &self.data.last_response {
                         if !resp.status().is_success() {
                              self.data.transport.send_message(
                                   Message::Response(resp.clone()),
                                   self.data.remote_addr
                               ).await.map_err(|e| Error::TransportError(e.to_string()))?;
                         }
                     }
                 } else if request.method() == Method::Ack {
                     debug!(id=%id, "Received ACK in Completed state");
                     self.transition_to(TransactionState::Confirmed).await?; // Resets timers G/H, starts I
                     // Inform TU about ACK
                      self.data.events_tx.send(TransactionEvent::AckReceived {
                         transaction_id: id,
                         ack_request: request,
                     }).await?;
                 } else {
                      warn!(id=%id, method=%request.method(), "Received unexpected request in Completed state");
                 }
             }
             TransactionState::Confirmed => {
                  // Retransmission of ACK - absorb it
                  if request.method() == Method::Ack {
                      trace!(id=%id, "Absorbing retransmitted ACK in Confirmed state");
                  } else {
                       warn!(id=%id, method=%request.method(), "Received unexpected request in Confirmed state");
                  }
             }
             // Add missing arms
             TransactionState::Initial => {
                 warn!(id=%id, state=?self.data.state, method=%request.method(), "Received request in unexpected Initial state");
             }
             TransactionState::Trying => {
                 warn!(id=%id, state=?self.data.state, method=%request.method(), "Received request in unexpected Trying state");
             }
              TransactionState::Calling => {
                  warn!(id=%id, state=?self.data.state, method=%request.method(), "Received request in unexpected Calling state");
             }
             TransactionState::Terminated => {
                  warn!(id=%id, state=?self.data.state, method=%request.method(), "Received request in unexpected Terminated state");
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
         let interval = TIMER_J_NON_INVITE_WAIT;
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
                      self.transition_to(TransactionState::Terminated).await?;
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