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

/// Server non-INVITE transaction (RFC 3261 Section 17.2.2)
#[derive(Debug, Clone)]
pub struct ServerNonInviteTransaction {
    data: Arc<ServerTransactionData>,
}

impl ServerNonInviteTransaction {
    /// Create a new server non-INVITE transaction.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config: Option<TimerSettings>,
    ) -> Result<Self> {
        if request.method() == Method::Invite || request.method() == Method::Ack {
            return Err(Error::Other("Request must not be INVITE or ACK for non-INVITE server transaction".to_string()));
        }

        // Create communication channels for the transaction
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        
        let timer_config = timer_config.unwrap_or_default();
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(TransactionState::Trying));
        
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
            
            debug!(id=%data.id, "Starting non-INVITE server transaction event loop");
            
            // Track active timers
            let mut timer_j_task: Option<JoinHandle<()>> = None;
            
            // Function to cancel active timers
            let cancel_timers = |j_task: &mut Option<JoinHandle<()>>| {
                if let Some(handle) = j_task.take() {
                    handle.abort();
                }
                trace!(id=%data.id, "Cancelled active timers");
            };
            
            // Function to start Timer J (wait time for request retransmissions)
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
                                   j_task: &mut Option<JoinHandle<()>>| -> Result<()> {
                let current_state = data.state.get();
                if current_state == new_state {
                    return Ok(());
                }
                
                // First validate the transition
                AtomicTransactionState::validate_transition(current_state, new_state, TransactionKind::NonInviteServer)?;
                
                debug!(id=%data.id, "State transition: {:?} -> {:?}", current_state, new_state);
                
                // Cancel existing timers
                cancel_timers(j_task);
                
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
                        // Start Timer J for reliability
                        start_timer_j(data.clone(), j_task);
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
                            &mut timer_j_task
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
                            
                            // For non-INVITE, we only care about retransmissions
                            if current_state == TransactionState::Trying || 
                               current_state == TransactionState::Proceeding || 
                               current_state == TransactionState::Completed {
                                debug!(id=%data.id, "Received request retransmission in state {:?}", current_state);
                                
                                // Retransmit the last response in Completed state
                                if current_state == TransactionState::Completed {
                                    let last_response = data.last_response.lock().await;
                                    if let Some(response) = &*last_response {
                                        if let Err(e) = data.transport.send_message(
                                            Message::Response(response.clone()),
                                            data.remote_addr
                                        ).await {
                                            error!(id=%data.id, error=%e, "Failed to retransmit response");
                                        }
                                    }
                                }
                            } else {
                                warn!(id=%data.id, state=?current_state, method=%method, "Received request in unexpected state");
                            }
                        } else {
                            warn!(id=%data.id, "Received non-request message");
                        }
                    },
                    InternalTransactionCommand::Timer(timer) => {
                        if timer == "J" {
                            // Timer J logic - terminate after waiting for retransmissions
                            let current_state = data.state.get();
                            if current_state == TransactionState::Completed {
                                debug!(id=%data.id, "Timer J fired in Completed state, terminating");
                                
                                // Transition to Terminated
                                let _ = handle_transition(
                                    TransactionState::Terminated, 
                                    &data, 
                                    &mut timer_j_task
                                ).await;
                            } else {
                                trace!(id=%data.id, state=?current_state, "Timer J fired in invalid state, ignoring");
                            }
                        } else {
                            warn!(id=%data.id, timer=%timer, "Unknown timer triggered");
                        }
                    },
                    InternalTransactionCommand::TransportError => {
                        error!(id=%data.id, "Transport error occurred, terminating transaction");
                        
                        // Transition to Terminated state
                        let _ = handle_transition(
                            TransactionState::Terminated, 
                            &data, 
                            &mut timer_j_task
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
                            &mut timer_j_task
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
            cancel_timers(&mut timer_j_task);
            debug!(id=%data.id, "Non-INVITE server transaction event loop ended");
        })
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
        utils::transaction_key_from_message(message).map(|key| key == self.data.id).unwrap_or(false)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl TransactionAsync for ServerNonInviteTransaction {
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

impl ServerTransaction for ServerNonInviteTransaction {
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
            
            // State transitions
            if current_state == TransactionState::Trying {
                if is_provisional {
                    // 1xx -> Proceeding
                    debug!(id=%data.id, "Sent provisional response, transitioning to Proceeding");
                    data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Proceeding)).await
                        .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
                } else {
                    // Final response -> Completed
                    debug!(id=%data.id, "Sent final response, transitioning to Completed");
                    data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Completed)).await
                        .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
                }
            } else if current_state == TransactionState::Proceeding {
                if !is_provisional {
                    // Final response -> Completed
                    debug!(id=%data.id, "Sent final response, transitioning to Completed");
                    data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Completed)).await
                        .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
                }
            }
            
            Ok(())
        })
    }
} 