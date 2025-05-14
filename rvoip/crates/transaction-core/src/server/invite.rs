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
use crate::timer::{Timer, TimerType, TimerSettings};
use crate::server::{
    ServerTransaction, ServerTransactionData, CommonServerTransaction
};
use crate::utils;

/// Server INVITE transaction (RFC 3261 Section 17.2.1)
#[derive(Debug, Clone)]
pub struct ServerInviteTransaction {
    data: Arc<ServerTransactionData>,
}

impl ServerInviteTransaction {
    /// Create a new server INVITE transaction.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config: Option<TimerSettings>,
    ) -> Result<Self> {
        if request.method() != Method::Invite {
            return Err(Error::Other("Request must be INVITE for INVITE server transaction".to_string()));
        }

        // Create communication channels for the transaction
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        
        let timer_config = timer_config.unwrap_or_default();
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(TransactionState::Proceeding));
        
        let data = Arc::new(ServerTransactionData {
            id: id.clone(),
            state,
            request: Arc::new(Mutex::new(request)),
            last_response: Arc::new(Mutex::new(None)),
            remote_addr,
            transport,
            events_tx,
            cmd_tx,
            cmd_rx: Arc::new(Mutex::new(cmd_rx)),
            event_loop_handle: Arc::new(Mutex::new(None)),
            timer_config,
        });
        
        let transaction = Self { data: data.clone() };
        
        // Start the event processing loop
        let event_loop_handle = Self::start_event_loop(data.clone());
        
        // Store the handle for cleanup
        if let Ok(mut handle_guard) = data.event_loop_handle.try_lock() {
            *handle_guard = Some(event_loop_handle);
        }
        
        Ok(transaction)
    }
    
    /// Start the main event processing loop for this transaction
    fn start_event_loop(
        data: Arc<ServerTransactionData>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let cmd_rx_mutex = data.cmd_rx.clone();
            let mut cmd_rx = cmd_rx_mutex.lock().await;
            
            debug!(id=%data.id, "Starting INVITE server transaction event loop");
            
            // Track active timers
            let mut timer_g_task: Option<JoinHandle<()>> = None;
            let mut timer_h_task: Option<JoinHandle<()>> = None;
            let mut timer_i_task: Option<JoinHandle<()>> = None;
            let mut timer_j_task: Option<JoinHandle<()>> = None;
            
            // Timer G interval for retransmission
            let mut timer_g_interval = data.timer_config.t1;
            
            // Function to cancel active timers
            let cancel_timers = |g_task: &mut Option<JoinHandle<()>>, 
                                 h_task: &mut Option<JoinHandle<()>>,
                                 i_task: &mut Option<JoinHandle<()>>,
                                 j_task: &mut Option<JoinHandle<()>>| {
                if let Some(handle) = g_task.take() {
                    handle.abort();
                }
                if let Some(handle) = h_task.take() {
                    handle.abort();
                }
                if let Some(handle) = i_task.take() {
                    handle.abort();
                }
                if let Some(handle) = j_task.take() {
                    handle.abort();
                }
                trace!(id=%data.id, "Cancelled active timers");
            };
            
            // Function to start Timer G (response retransmission)
            let start_timer_g = |interval: Duration, data: Arc<ServerTransactionData>, g_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = g_task.take() {
                    old_task.abort();
                }
                
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *g_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer G fired");
                    
                    // Notify about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "G".to_string() 
                    }).await;
                    
                    // Send command to process the timer
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("G".to_string())).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer G");
            };
            
            // Function to start Timer H (wait time for ACK)
            let start_timer_h = |data: Arc<ServerTransactionData>, h_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = h_task.take() {
                    old_task.abort();
                }
                
                let interval = data.timer_config.wait_time_h;
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *h_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer H fired");
                    
                    // Notify about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "H".to_string() 
                    }).await;
                    
                    // Send command to process the timer
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("H".to_string())).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer H");
            };
            
            // Function to start Timer I (wait time in Confirmed state)
            let start_timer_i = |data: Arc<ServerTransactionData>, i_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = i_task.take() {
                    old_task.abort();
                }
                
                let interval = data.timer_config.wait_time_i;
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *i_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer I fired");
                    
                    // Notify about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "I".to_string() 
                    }).await;
                    
                    // Send command to process the timer
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("I".to_string())).await;
                    
                    // For Timer I, we can also directly send a command to transition to Terminated state
                    let _ = cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer I");
            };
            
            // Function to start Timer J (wait time for non-INVITE request retransmissions)
            let start_timer_j = |data: Arc<ServerTransactionData>, j_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = j_task.take() {
                    old_task.abort();
                }
                
                let interval = data.timer_config.wait_time_j;
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *j_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer J fired");
                    
                    // Notify about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "J".to_string() 
                    }).await;
                    
                    // Send command to process the timer
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("J".to_string())).await;
                    
                    // For Timer J, we can also directly send a command to transition to Terminated state
                    let _ = cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer J");
            };
            
            // Handle transition to a new state
            let handle_transition = async |new_state: TransactionState, 
                                   data: &Arc<ServerTransactionData>,
                                   g_task: &mut Option<JoinHandle<()>>,
                                   h_task: &mut Option<JoinHandle<()>>,
                                   i_task: &mut Option<JoinHandle<()>>,
                                   j_task: &mut Option<JoinHandle<()>>,
                                   timer_g_interval: &mut Duration| -> Result<()> {
                let current_state = data.state.get();
                if current_state == new_state {
                    return Ok(());
                }
                
                // First validate the transition
                AtomicTransactionState::validate_transition(current_state, new_state, TransactionKind::InviteServer)?;
                
                debug!(id=%data.id, "State transition: {:?} -> {:?}", current_state, new_state);
                
                // Cancel existing timers
                cancel_timers(g_task, h_task, i_task, j_task);
                
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
                    TransactionState::Completed => {
                        // Start Timer G for retransmitting the response
                        start_timer_g(data.timer_config.t1, data.clone(), g_task);
                        
                        // Start Timer H to guard against lost ACKs
                        start_timer_h(data.clone(), h_task);
                        
                        *timer_g_interval = data.timer_config.t1;
                    },
                    TransactionState::Confirmed => {
                        // Start Timer I
                        start_timer_i(data.clone(), i_task);
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
                            &mut timer_g_task, 
                            &mut timer_h_task, 
                            &mut timer_i_task,
                            &mut timer_j_task,
                            &mut timer_g_interval
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
                        if let Message::Request(request) = msg {
                            let method = request.method();
                            let current_state = data.state.get();
                            
                            if method == Method::Invite {
                                // INVITE retransmission
                                if current_state == TransactionState::Proceeding {
                                    debug!(id=%data.id, "Received INVITE retransmission in Proceeding state");
                                    
                                    // According to RFC 3261, resend the last provisional response
                                    let last_response = data.last_response.lock().await;
                                    if let Some(response) = &*last_response {
                                        if let Err(e) = data.transport.send_message(
                                            Message::Response(response.clone()),
                                            data.remote_addr
                                        ).await {
                                            error!(id=%data.id, error=%e, "Failed to retransmit response");
                                        }
                                    }
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Ignoring INVITE retransmission in non-proceeding state");
                                }
                            } else if method == Method::Ack {
                                // ACK received
                                if current_state == TransactionState::Completed {
                                    debug!(id=%data.id, "Received ACK in Completed state");
                                    
                                    // Transition to Confirmed state
                                    let _ = handle_transition(
                                        TransactionState::Confirmed, 
                                        &data, 
                                        &mut timer_g_task, 
                                        &mut timer_h_task, 
                                        &mut timer_i_task,
                                        &mut timer_j_task,
                                        &mut timer_g_interval
                                    ).await;
                                    
                                    // Notify TU about ACK
                                    let _ = data.events_tx.send(TransactionEvent::AckReceived {
                                        transaction_id: data.id.clone(),
                                        request: request.clone(),
                                    }).await;
                                } else if current_state == TransactionState::Confirmed {
                                    // ACK retransmission, already in Confirmed state
                                    trace!(id=%data.id, "Received duplicate ACK in Confirmed state, ignoring");
                                } else {
                                    warn!(id=%data.id, state=?current_state, "Received ACK in unexpected state");
                                }
                            } else if method == Method::Cancel {
                                // CANCEL request
                                if current_state == TransactionState::Proceeding {
                                    debug!(id=%data.id, "Received CANCEL in Proceeding state");
                                    
                                    // Notify TU about CANCEL
                                    let _ = data.events_tx.send(TransactionEvent::CancelReceived {
                                        transaction_id: data.id.clone(),
                                        cancel_request: request.clone(),
                                    }).await;
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Ignoring CANCEL in non-proceeding state");
                                }
                            } else {
                                warn!(id=%data.id, method=%method, "Received unexpected request method");
                            }
                        } else {
                            warn!(id=%data.id, "Received non-request message");
                        }
                    },
                    InternalTransactionCommand::Timer(timer) => {
                        match timer.as_str() {
                            "G" => {
                                // Timer G logic - retransmit the final response
                                let current_state = data.state.get();
                                if current_state == TransactionState::Completed {
                                    debug!(id=%data.id, "Timer G triggered, retransmitting final response");
                                    
                                    // Retransmit the final response
                                    let response_guard = data.last_response.lock().await;
                                    if let Some(response) = &*response_guard {
                                        if let Err(e) = data.transport.send_message(
                                            Message::Response(response.clone()),
                                            data.remote_addr
                                        ).await {
                                            error!(id=%data.id, error=%e, "Failed to retransmit response");
                                        }
                                    }
                                    
                                    // Double interval, capped by T2
                                    timer_g_interval = std::cmp::min(timer_g_interval * 2, data.timer_config.t2);
                                    
                                    // Restart timer G with new interval
                                    start_timer_g(timer_g_interval, data.clone(), &mut timer_g_task);
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Timer G fired in invalid state, ignoring");
                                }
                            },
                            "H" => {
                                // Timer H logic - ACK timeout
                                let current_state = data.state.get();
                                if current_state == TransactionState::Completed {
                                    warn!(id=%data.id, "Timer H (ACK Timeout) fired in Completed state");
                                    
                                    // Transition to Terminated
                                    let _ = handle_transition(
                                        TransactionState::Terminated, 
                                        &data, 
                                        &mut timer_g_task, 
                                        &mut timer_h_task, 
                                        &mut timer_i_task,
                                        &mut timer_j_task,
                                        &mut timer_g_interval
                                    ).await;
                                    
                                    // Notify TU about timeout
                                    let _ = data.events_tx.send(TransactionEvent::TransactionTimeout {
                                        transaction_id: data.id.clone(),
                                    }).await;
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Timer H fired in invalid state, ignoring");
                                }
                            },
                            "I" => {
                                // Timer I logic - terminate after waiting for retransmissions in Confirmed state
                                let current_state = data.state.get();
                                if current_state == TransactionState::Confirmed {
                                    debug!(id=%data.id, "Timer I fired in Confirmed state, terminating");
                                    
                                    // Transition to Terminated
                                    let _ = handle_transition(
                                        TransactionState::Terminated, 
                                        &data, 
                                        &mut timer_g_task, 
                                        &mut timer_h_task, 
                                        &mut timer_i_task,
                                        &mut timer_j_task,
                                        &mut timer_g_interval
                                    ).await;
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Timer I fired in invalid state, ignoring");
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
                            &mut timer_g_task, 
                            &mut timer_h_task, 
                            &mut timer_i_task,
                            &mut timer_j_task,
                            &mut timer_g_interval
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
                            &mut timer_g_task, 
                            &mut timer_h_task, 
                            &mut timer_i_task,
                            &mut timer_j_task,
                            &mut timer_g_interval
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
            cancel_timers(&mut timer_g_task, &mut timer_h_task, &mut timer_i_task, &mut timer_j_task);
            debug!(id=%data.id, "INVITE server transaction event loop ended");
        })
    }
}

impl CommonServerTransaction for ServerInviteTransaction {
    fn data(&self) -> &Arc<ServerTransactionData> {
        &self.data
    }
}

impl Transaction for ServerInviteTransaction {
    fn id(&self) -> &TransactionKey {
        &self.data.id
    }

    fn kind(&self) -> TransactionKind {
        TransactionKind::InviteServer
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

impl TransactionAsync for ServerInviteTransaction {
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

impl ServerTransaction for ServerInviteTransaction {
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
            let is_success = status.is_success();
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
            
            // For preliminary responses in Proceeding state, stay in Proceeding
            if is_provisional && current_state == TransactionState::Proceeding {
                // Stays in Proceeding state, no state change
                trace!(id=%data.id, "Sent provisional response, staying in Proceeding state");
                return Ok(());
            }
            
            // For 2xx responses, directly terminate the transaction
            if is_success {
                debug!(id=%data.id, "Sent 2xx response, transitioning to Terminated");
                
                // TU level will handle reliable delivery of 2xx responses
                data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await
                    .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
                
                return Ok(());
            }
            
            // For >= 300 responses, transition to Completed
            if !is_provisional && !is_success && current_state == TransactionState::Proceeding {
                debug!(id=%data.id, "Sent >= 300 response, transitioning to Completed");
                
                data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Completed)).await
                    .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
            }
            
            Ok(())
        })
    }
}

// Implementation of the event loop for the server transaction
impl ServerInviteTransaction {
    // Send ACK received event to the TU
    async fn handle_ack(&self, request: Request) -> Result<()> {
        // Send to TU
        debug!(id=%self.data.id, "Received ACK for non-2xx response");
        
        self.data.events_tx.send(TransactionEvent::AckReceived {
            transaction_id: self.data.id.clone(),
            request: request.clone(),
        }).await.map_err(|_| Error::Other("Failed to send event".to_string()))
    }
    
    // Handle CANCEL request
    async fn handle_cancel(&self, request: Request) -> Result<()> {
        debug!(id=%self.data.id, "Received CANCEL");
        
        // Forward to TU
        self.data.events_tx.send(TransactionEvent::CancelReceived {
            transaction_id: self.data.id.clone(),
            cancel_request: request.clone(),
        }).await.map_err(|_| Error::Other("Failed to send event".to_string()))
    }
} 