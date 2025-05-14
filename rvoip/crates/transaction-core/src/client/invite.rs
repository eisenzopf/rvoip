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
    InternalTransactionCommand, AtomicTransactionState
};
use crate::timer::{Timer, TimerType, TimerSettings};
use crate::client::{
    ClientTransaction, ClientTransactionData, CommandSender, CommandReceiver,
    CommonClientTransaction
};
use crate::utils;

/// Client INVITE transaction (RFC 3261 Section 17.1.1)
#[derive(Debug, Clone)]
pub struct ClientInviteTransaction {
    data: Arc<ClientTransactionData>,
}

impl ClientInviteTransaction {
    /// Create a new client INVITE transaction.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config: Option<TimerSettings>,
    ) -> Result<Self> {
        if request.method() != Method::Invite {
            return Err(Error::Other("Request must be INVITE for INVITE client transaction".to_string()));
        }

        // Create communication channels for the transaction
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        
        let timer_config = timer_config.unwrap_or_default();
        let state = Arc::new(AtomicTransactionState::new(TransactionState::Initial));
        
        let data = Arc::new(ClientTransactionData {
            id: id.clone(),
            state,
            request: Arc::new(Mutex::new(request)),
            last_response: Arc::new(Mutex::new(None)),
            remote_addr,
            transport,
            events_tx,
            cmd_tx,
            event_loop_handle: Arc::new(Mutex::new(None)),
            timer_config,
        });
        
        let transaction = Self { data: data.clone() };
        
        // Start the event processing loop - pass cmd_rx as parameter
        let event_loop_handle = Self::start_event_loop(data.clone(), cmd_rx);
        
        // Store the handle for cleanup
        if let Ok(mut handle_guard) = data.event_loop_handle.try_lock() {
            *handle_guard = Some(event_loop_handle);
        }
        
        Ok(transaction)
    }
    
    /// Start the main event processing loop for this transaction
    fn start_event_loop(
        data: Arc<ClientTransactionData>,
        mut cmd_rx: CommandReceiver, // Take cmd_rx as parameter
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            // Remove the access to cmd_rx through data.cmd_rx
            debug!(id=%data.id, "Starting INVITE client transaction event loop");
            
            // Track active timers
            let mut timer_a_task: Option<JoinHandle<()>> = None;
            let mut timer_b_task: Option<JoinHandle<()>> = None;
            let mut timer_d_task: Option<JoinHandle<()>> = None;
            
            // Timer A interval for retransmission
            let mut timer_a_interval = data.timer_config.t1;
            
            // Function to cancel active timers
            let cancel_timers = |a_task: &mut Option<JoinHandle<()>>, 
                                 b_task: &mut Option<JoinHandle<()>>, 
                                 d_task: &mut Option<JoinHandle<()>>| {
                if let Some(handle) = a_task.take() {
                    handle.abort();
                }
                if let Some(handle) = b_task.take() {
                    handle.abort();
                }
                if let Some(handle) = d_task.take() {
                    handle.abort();
                }
                trace!(id=%data.id, "Cancelled active timers");
            };
            
            // Function to start Timer A (retransmission)
            let start_timer_a = |interval: Duration, data: Arc<ClientTransactionData>, a_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = a_task.take() {
                    old_task.abort();
                }
                
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *a_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer A fired");
                    
                    // First notify the TU about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "A".to_string() 
                    }).await;
                    
                    // Then send a command to process the timer event
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("A".to_string())).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer A");
            };
            
            // Function to start Timer B (transaction timeout)
            let start_timer_b = |data: Arc<ClientTransactionData>, b_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = b_task.take() {
                    old_task.abort();
                }
                
                let interval = data.timer_config.transaction_timeout;
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *b_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer B fired");
                    
                    // Notify about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "B".to_string() 
                    }).await;
                    
                    // Send command to process the timer
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("B".to_string())).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer B");
            };
            
            // Function to start Timer D (wait for response retransmissions)
            let start_timer_d = |data: Arc<ClientTransactionData>, d_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = d_task.take() {
                    old_task.abort();
                }
                
                let interval = data.timer_config.wait_time_d;
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *d_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer D fired");
                    
                    // Notify about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "D".to_string() 
                    }).await;
                    
                    // Send command to process the timer
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("D".to_string())).await;
                    
                    // For Timer D, we can also directly send a command to transition to Terminated state
                    // This ensures termination even if the command queue is backed up
                    let _ = cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer D");
            };
            
            // Handle transition to a new state
            let handle_transition = async |new_state: TransactionState, 
                                   data: &Arc<ClientTransactionData>,
                                   a_task: &mut Option<JoinHandle<()>>,
                                   b_task: &mut Option<JoinHandle<()>>,
                                   d_task: &mut Option<JoinHandle<()>>,
                                   timer_a_interval: &mut Duration| -> Result<()> {
                let current_state = data.state.get();
                if current_state == new_state {
                    return Ok(());
                }
                
                // First validate the transition
                AtomicTransactionState::validate_transition(current_state, new_state, TransactionKind::InviteClient)?;
                
                debug!(id=%data.id, "State transition: {:?} -> {:?}", current_state, new_state);
                
                // Cancel existing timers
                cancel_timers(a_task, b_task, d_task);
                
                // Update the state
                let prev_state = data.state.set(new_state);
                
                // Notify about state change
                let _ = data.events_tx.send(TransactionEvent::StateChanged {
                    transaction_id: data.id.clone(),
                    previous_state: prev_state,
                    new_state,
                }).await;
                
                // Start appropriate timers for the new state
                match new_state {
                    TransactionState::Calling => {
                        start_timer_a(data.timer_config.t1, data.clone(), a_task);
                        start_timer_b(data.clone(), b_task);
                        *timer_a_interval = data.timer_config.t1;
                    },
                    TransactionState::Completed => {
                        // Start Timer D for non-2xx final responses
                        let response_guard = data.last_response.lock().await;
                        if let Some(response) = &*response_guard {
                            if !response.status().is_success() {
                                start_timer_d(data.clone(), d_task);
                            }
                        }
                    },
                    TransactionState::Terminated => {
                        // If we're terminating, send a termination event
                        let _ = data.events_tx.send(TransactionEvent::TransactionTerminated {
                            transaction_id: data.id.clone(),
                        }).await;
                    },
                    _ => {},
                }
                
                Ok(())
            };
            
            // Main event processing loop
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    InternalTransactionCommand::TransitionTo(new_state) => {
                        if let Err(e) = handle_transition(
                            new_state, 
                            &data, 
                            &mut timer_a_task, 
                            &mut timer_b_task, 
                            &mut timer_d_task,
                            &mut timer_a_interval
                        ).await {
                            error!(id=%data.id, error=%e, "Failed to transition state");
                            
                            // If transition fails, notify TU about the error
                            let _ = data.events_tx.send(TransactionEvent::Error {
                                transaction_id: Some(data.id.clone()),
                                error: e.to_string(),
                            }).await;
                        }
                    },
                    InternalTransactionCommand::ProcessMessage(msg) => {
                        if let Message::Response(response) = msg {
                            // Process response according to current state
                            let current_state = data.state.get();
                            let status = response.status();
                            let is_provisional = status.is_provisional();
                            let is_success = status.is_success();
                            let is_failure = !is_provisional && !is_success;
                            
                            // Store the response
                            {
                                let mut last_response = data.last_response.lock().await;
                                *last_response = Some(response.clone());
                            }
                            
                            match current_state {
                                TransactionState::Calling => {
                                    // Cancel retransmission timers
                                    cancel_timers(&mut timer_a_task, &mut timer_b_task, &mut timer_d_task);
                                    
                                    if is_provisional {
                                        // 1xx -> Proceeding
                                        let _ = handle_transition(
                                            TransactionState::Proceeding, 
                                            &data, 
                                            &mut timer_a_task, 
                                            &mut timer_b_task, 
                                            &mut timer_d_task,
                                            &mut timer_a_interval
                                        ).await;
                                        
                                        // Notify TU
                                        let _ = data.events_tx.send(TransactionEvent::ProvisionalResponse {
                                            transaction_id: data.id.clone(),
                                            response: response.clone(),
                                        }).await;
                                    } else if is_success {
                                        // 2xx -> Terminated (RFC 3261 17.1.1.2)
                                        let _ = handle_transition(
                                            TransactionState::Terminated, 
                                            &data, 
                                            &mut timer_a_task, 
                                            &mut timer_b_task, 
                                            &mut timer_d_task,
                                            &mut timer_a_interval
                                        ).await;
                                        
                                        // Notify TU
                                        let _ = data.events_tx.send(TransactionEvent::SuccessResponse {
                                            transaction_id: data.id.clone(),
                                            response: response.clone(),
                                        }).await;
                                    } else if is_failure {
                                        // 3xx-6xx -> Completed
                                        let _ = handle_transition(
                                            TransactionState::Completed, 
                                            &data, 
                                            &mut timer_a_task, 
                                            &mut timer_b_task, 
                                            &mut timer_d_task,
                                            &mut timer_a_interval
                                        ).await;
                                        
                                        // Create and send ACK for non-2xx final response
                                        match Self::create_ack_for_response(&data, &response).await {
                                            Ok(ack) => {
                                                // Send the ACK request
                                                if let Err(e) = data.transport.send_message(
                                                    Message::Request(ack),
                                                    data.remote_addr
                                                ).await {
                                                    error!(id=%data.id, error=%e, "Failed to send ACK");
                                                    
                                                    // Send error event and transition to terminated
                                                    let _ = data.events_tx.send(TransactionEvent::Error {
                                                        transaction_id: Some(data.id.clone()),
                                                        error: format!("Transport error sending ACK: {}", e),
                                                    }).await;
                                                    
                                                    // Transition to terminated state
                                                    let _ = data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                                                }
                                            },
                                            Err(e) => {
                                                error!(id=%data.id, error=%e, "Failed to create ACK for 3xx-6xx response");
                                                
                                                // Send error event and transition to terminated
                                                let _ = data.events_tx.send(TransactionEvent::Error {
                                                    transaction_id: Some(data.id.clone()),
                                                    error: format!("Failed to create ACK: {}", e),
                                                }).await;
                                                
                                                // Transition to terminated state
                                                let _ = data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                                            }
                                        }
                                        
                                        // Notify TU
                                        let _ = data.events_tx.send(TransactionEvent::FailureResponse {
                                            transaction_id: data.id.clone(),
                                            response: response.clone(),
                                        }).await;
                                    }
                                },
                                TransactionState::Proceeding => {
                                    if is_provisional {
                                        // Additional 1xx in Proceeding state, forward to TU
                                        let _ = data.events_tx.send(TransactionEvent::ProvisionalResponse {
                                            transaction_id: data.id.clone(),
                                            response: response.clone(),
                                        }).await;
                                    } else if is_success {
                                        // 2xx responses transition directly to Terminated (RFC 3261 17.1.1.2)
                                        let _ = handle_transition(
                                            TransactionState::Terminated, 
                                            &data, 
                                            &mut timer_a_task, 
                                            &mut timer_b_task, 
                                            &mut timer_d_task,
                                            &mut timer_a_interval
                                        ).await;
                                        
                                        // Notify TU
                                        let _ = data.events_tx.send(TransactionEvent::SuccessResponse {
                                            transaction_id: data.id.clone(),
                                            response: response.clone(),
                                        }).await;
                                    } else if is_failure {
                                        // 3xx-6xx transition to Completed
                                        let _ = handle_transition(
                                            TransactionState::Completed, 
                                            &data, 
                                            &mut timer_a_task, 
                                            &mut timer_b_task, 
                                            &mut timer_d_task,
                                            &mut timer_a_interval
                                        ).await;
                                        
                                        // Create and send ACK for non-2xx final response
                                        match Self::create_ack_for_response(&data, &response).await {
                                            Ok(ack) => {
                                                // Send the ACK request
                                                if let Err(e) = data.transport.send_message(
                                                    Message::Request(ack),
                                                    data.remote_addr
                                                ).await {
                                                    error!(id=%data.id, error=%e, "Failed to send ACK");
                                                    
                                                    // Send error event and transition to terminated
                                                    let _ = data.events_tx.send(TransactionEvent::Error {
                                                        transaction_id: Some(data.id.clone()),
                                                        error: format!("Transport error sending ACK: {}", e),
                                                    }).await;
                                                    
                                                    // Transition to terminated state
                                                    let _ = data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                                                }
                                            },
                                            Err(e) => {
                                                error!(id=%data.id, error=%e, "Failed to create ACK for 3xx-6xx response");
                                                
                                                // Send error event and transition to terminated
                                                let _ = data.events_tx.send(TransactionEvent::Error {
                                                    transaction_id: Some(data.id.clone()),
                                                    error: format!("Failed to create ACK: {}", e),
                                                }).await;
                                                
                                                // Transition to terminated state
                                                let _ = data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                                            }
                                        }
                                        
                                        // Notify TU
                                        let _ = data.events_tx.send(TransactionEvent::FailureResponse {
                                            transaction_id: data.id.clone(),
                                            response: response.clone(),
                                        }).await;
                                    }
                                },
                                TransactionState::Completed => {
                                    if is_failure {
                                        // Received retransmission of final error response, resend ACK
                                        debug!(id=%data.id, "Received retransmission of error response in Completed state, resending ACK");
                                        
                                        // Resend ACK for retransmission of response
                                        match Self::create_ack_for_response(&data, &response).await {
                                            Ok(ack) => {
                                                // Send the ACK request
                                                if let Err(e) = data.transport.send_message(
                                                    Message::Request(ack),
                                                    data.remote_addr
                                                ).await {
                                                    error!(id=%data.id, error=%e, "Failed to send ACK");
                                                    
                                                    // Log but don't terminate - this is just a retransmission
                                                    // Transactions are more resilient to transport errors during retransmissions
                                                }
                                            },
                                            Err(e) => {
                                                error!(id=%data.id, error=%e, "Failed to create ACK for retransmitted response");
                                                
                                                // Log but don't terminate - this is just a retransmission attempt
                                                // The initial ACK was likely sent successfully
                                            }
                                        }
                                    } else {
                                        trace!(id=%data.id, state=?current_state, status=%status, 
                                              "Ignoring success or provisional response in Completed state");
                                    }
                                },
                                _ => {
                                    warn!(id=%data.id, state=?current_state, status=%status, 
                                         "Received response in unexpected state");
                                }
                            }
                        } else {
                            warn!(id=%data.id, "Received non-response message");
                        }
                    },
                    InternalTransactionCommand::Timer(timer) => {
                        match timer.as_str() {
                            "A" => {
                                let current_state = data.state.get();
                                if current_state == TransactionState::Calling {
                                    debug!(id=%data.id, "Timer A triggered, retransmitting INVITE");
                                    
                                    // Retransmit the request
                                    let request_guard = data.request.lock().await;
                                    if let Err(e) = data.transport.send_message(
                                        Message::Request(request_guard.clone()),
                                        data.remote_addr
                                    ).await {
                                        error!(id=%data.id, error=%e, "Failed to retransmit request");
                                    }
                                    
                                    // Double interval, capped by T2
                                    timer_a_interval = std::cmp::min(timer_a_interval * 2, data.timer_config.t2);
                                    
                                    // Restart timer A with new interval
                                    start_timer_a(timer_a_interval, data.clone(), &mut timer_a_task);
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Timer A fired in invalid state, ignoring");
                                }
                            },
                            "B" => {
                                // Timer B logic - transaction timeout
                                let current_state = data.state.get();
                                if current_state == TransactionState::Calling {
                                    warn!(id=%data.id, "Timer B (Timeout) fired in Calling state");
                                    
                                    // Transition to Terminated
                                    let _ = handle_transition(
                                        TransactionState::Terminated, 
                                        &data, 
                                        &mut timer_a_task, 
                                        &mut timer_b_task, 
                                        &mut timer_d_task,
                                        &mut timer_a_interval
                                    ).await;
                                    
                                    // Notify TU about timeout
                                    let _ = data.events_tx.send(TransactionEvent::TransactionTimeout {
                                        transaction_id: data.id.clone(),
                                    }).await;
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Timer B fired in invalid state, ignoring");
                                }
                            },
                            "D" => {
                                // Timer D logic - terminate after waiting for retransmissions
                                let current_state = data.state.get();
                                if current_state == TransactionState::Completed {
                                    debug!(id=%data.id, "Timer D fired in Completed state, terminating");
                                    
                                    // Transition to Terminated
                                    let _ = handle_transition(
                                        TransactionState::Terminated, 
                                        &data, 
                                        &mut timer_a_task, 
                                        &mut timer_b_task, 
                                        &mut timer_d_task,
                                        &mut timer_a_interval
                                    ).await;
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Timer D fired in invalid state, ignoring");
                                }
                            },
                            _ => {
                                warn!(id=%data.id, timer=%timer, "Unknown timer triggered");
                            }
                        }
                    },
                    InternalTransactionCommand::TransportError => {
                        error!(id=%data.id, "Transport error occurred, terminating transaction");
                        
                        // Transition to Terminated state
                        let _ = handle_transition(
                            TransactionState::Terminated, 
                            &data, 
                            &mut timer_a_task, 
                            &mut timer_b_task, 
                            &mut timer_d_task,
                            &mut timer_a_interval
                        ).await;
                        
                        // Notify TU about transport error
                        let _ = data.events_tx.send(TransactionEvent::TransportError {
                            transaction_id: data.id.clone(),
                        }).await;
                    },
                    InternalTransactionCommand::Terminate => {
                        debug!(id=%data.id, "Received explicit termination command");
                        
                        // Transition to Terminated state
                        let _ = handle_transition(
                            TransactionState::Terminated, 
                            &data, 
                            &mut timer_a_task, 
                            &mut timer_b_task, 
                            &mut timer_d_task,
                            &mut timer_a_interval
                        ).await;
                        
                        // Stop processing events
                        break;
                    }
                }
                
                // If we've reached Terminated state, stop processing events
                if data.state.get() == TransactionState::Terminated {
                    debug!(id=%data.id, "Transaction reached Terminated state, stopping event loop");
                    break;
                }
            }
            
            // Final cleanup
            cancel_timers(&mut timer_a_task, &mut timer_b_task, &mut timer_d_task);
            debug!(id=%data.id, "INVITE client transaction event loop ended");
        })
    }
    
    /// Helper method to create an ACK for a non-2xx response
    async fn create_ack_for_response(data: &Arc<ClientTransactionData>, response: &Response) -> Result<Request> {
        let request_guard = data.request.lock().await;
        
        utils::create_ack_from_invite(&request_guard, response)
    }
}

impl CommonClientTransaction for ClientInviteTransaction {
    fn data(&self) -> &Arc<ClientTransactionData> {
        &self.data
    }
}

impl Transaction for ClientInviteTransaction {
    fn id(&self) -> &TransactionKey {
        &self.data.id
    }

    fn kind(&self) -> TransactionKind {
        TransactionKind::InviteClient
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

impl TransactionAsync for ClientInviteTransaction {
    fn process_event<'a>(
        &'a self,
        event_type: &'a str,
        message: Option<Message>
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match event_type {
                "response" => {
                    if let Some(Message::Response(response)) = message {
                        self.process_response(response).await
                    } else {
                        Err(Error::Other("Expected Response message".to_string()))
                    }
                },
                "send" => {
                    self.initiate().await
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

impl ClientTransaction for ClientInviteTransaction {
    fn initiate(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        
        Box::pin(async move {
            let current_state = data.state.get();
            
            if current_state != TransactionState::Initial {
                return Err(Error::invalid_state_transition(
                    TransactionKind::InviteClient,
                    current_state,
                    TransactionState::Calling,
                    Some(data.id.clone())
                ));
            }
            
            debug!(id=%data.id, "Sending initial INVITE request");
            
            // Transition to Calling state
            data.state.set(TransactionState::Calling);
            
            // Notify the transaction about the state change
            data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Calling)).await
                .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
            
            // Send the request
            let request = {
                let request_guard = data.request.lock().await;
                request_guard.clone()
            };
            
            data.transport.send_message(Message::Request(request), data.remote_addr)
                .await
                .map_err(|e| Error::transport_error(e, "Failed to send INVITE request"))?;
            
            Ok(())
        })
    }
    
    fn process_response(&self, response: Response) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        self.process_response_common(response)
    }

    fn original_request<'a>(&'a self) -> Pin<Box<dyn Future<Output = Option<Request>> + Send + 'a>> {
        Box::pin(async move {
            Some(self.data.request.lock().await.clone())
        })
    }
} 