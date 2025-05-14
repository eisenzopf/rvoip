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
use crate::client::{
    ClientTransaction, ClientTransactionData, CommandSender, CommandReceiver,
    CommonClientTransaction
};
use crate::utils;

/// Client non-INVITE transaction (RFC 3261 Section 17.1.2)
#[derive(Debug, Clone)]
pub struct ClientNonInviteTransaction {
    data: Arc<ClientTransactionData>,
}

impl ClientNonInviteTransaction {
    /// Create a new client non-INVITE transaction.
    pub fn new(
        id: TransactionKey,
        request: Request,
        remote_addr: SocketAddr,
        transport: Arc<dyn Transport>,
        events_tx: mpsc::Sender<TransactionEvent>,
        timer_config: Option<TimerSettings>,
    ) -> Result<Self> {
        if request.method() == Method::Invite || request.method() == Method::Ack {
            return Err(Error::Other("Request must not be INVITE or ACK for non-INVITE client transaction".to_string()));
        }

        // Create communication channels for the transaction
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        
        let timer_config = timer_config.unwrap_or_default();
        let state = Arc::new(crate::transaction::AtomicTransactionState::new(TransactionState::Initial));
        
        let data = Arc::new(ClientTransactionData {
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
        data: Arc<ClientTransactionData>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            let cmd_rx_mutex = data.cmd_rx.clone();
            let mut cmd_rx = cmd_rx_mutex.lock().await;
            
            debug!(id=%data.id, "Starting non-INVITE client transaction event loop");
            
            // Track active timers
            let mut timer_e_task: Option<JoinHandle<()>> = None;
            let mut timer_f_task: Option<JoinHandle<()>> = None;
            let mut timer_k_task: Option<JoinHandle<()>> = None;
            
            // Timer E interval for retransmission
            let mut timer_e_interval = data.timer_config.t1;
            
            // Function to cancel active timers
            let cancel_timers = |e_task: &mut Option<JoinHandle<()>>, 
                                 f_task: &mut Option<JoinHandle<()>>, 
                                 k_task: &mut Option<JoinHandle<()>>| {
                if let Some(handle) = e_task.take() {
                    handle.abort();
                }
                if let Some(handle) = f_task.take() {
                    handle.abort();
                }
                if let Some(handle) = k_task.take() {
                    handle.abort();
                }
                trace!(id=%data.id, "Cancelled active timers");
            };
            
            // Function to start Timer E (retransmission)
            let start_timer_e = |interval: Duration, data: Arc<ClientTransactionData>, e_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = e_task.take() {
                    old_task.abort();
                }
                
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *e_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer E fired");
                    
                    // First notify the TU about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "E".to_string() 
                    }).await;
                    
                    // Then send a command to process the timer event
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("E".to_string())).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer E");
            };
            
            // Function to start Timer F (transaction timeout)
            let start_timer_f = |data: Arc<ClientTransactionData>, f_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = f_task.take() {
                    old_task.abort();
                }
                
                let interval = data.timer_config.transaction_timeout;
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *f_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer F fired");
                    
                    // Notify about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "F".to_string() 
                    }).await;
                    
                    // Send command to process the timer
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("F".to_string())).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer F");
            };
            
            // Function to start Timer K (wait for response retransmissions)
            let start_timer_k = |data: Arc<ClientTransactionData>, k_task: &mut Option<JoinHandle<()>>| {
                if let Some(old_task) = k_task.take() {
                    old_task.abort();
                }
                
                let interval = data.timer_config.wait_time_k;
                let id = data.id.clone();
                let events_tx = data.events_tx.clone();
                let cmd_tx = data.cmd_tx.clone();
                
                *k_task = Some(tokio::spawn(async move {
                    tokio::time::sleep(interval).await;
                    debug!(id=%id, "Timer K fired");
                    
                    // Notify about the timer event
                    let _ = events_tx.send(TransactionEvent::TimerTriggered { 
                        transaction_id: id.clone(), 
                        timer: "K".to_string() 
                    }).await;
                    
                    // Send command to process the timer
                    let _ = cmd_tx.send(InternalTransactionCommand::Timer("K".to_string())).await;
                    
                    // For Timer K, we can also directly send a command to transition to Terminated state
                    let _ = cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Terminated)).await;
                }));
                
                trace!(id=%data.id, interval=?interval, "Started Timer K");
            };
            
            // Handle transition to a new state
            let handle_transition = async |new_state: TransactionState, 
                                   data: &Arc<ClientTransactionData>,
                                   e_task: &mut Option<JoinHandle<()>>,
                                   f_task: &mut Option<JoinHandle<()>>,
                                   k_task: &mut Option<JoinHandle<()>>,
                                   timer_e_interval: &mut Duration| -> Result<()> {
                let current_state = data.state.get();
                if current_state == new_state {
                    return Ok(());
                }
                
                // First validate the transition
                AtomicTransactionState::validate_transition(current_state, new_state, TransactionKind::NonInviteClient)?;
                
                debug!(id=%data.id, "State transition: {:?} -> {:?}", current_state, new_state);
                
                // Cancel existing timers
                cancel_timers(e_task, f_task, k_task);
                
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
                    TransactionState::Trying => {
                        start_timer_e(data.timer_config.t1, data.clone(), e_task);
                        start_timer_f(data.clone(), f_task);
                        *timer_e_interval = data.timer_config.t1;
                    },
                    TransactionState::Completed => {
                        // Start Timer K for reliability
                        start_timer_k(data.clone(), k_task);
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
                            &mut timer_e_task, 
                            &mut timer_f_task, 
                            &mut timer_k_task,
                            &mut timer_e_interval
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
                            let is_final = !is_provisional;
                            
                            // Store the response
                            {
                                let mut last_response = data.last_response.lock().await;
                                *last_response = Some(response.clone());
                            }
                            
                            match current_state {
                                TransactionState::Trying | TransactionState::Proceeding => {
                                    // Cancel retransmission timers
                                    cancel_timers(&mut timer_e_task, &mut timer_f_task, &mut timer_k_task);
                                    
                                    if is_provisional {
                                        // 1xx -> Proceeding
                                        let _ = handle_transition(
                                            TransactionState::Proceeding, 
                                            &data, 
                                            &mut timer_e_task, 
                                            &mut timer_f_task, 
                                            &mut timer_k_task,
                                            &mut timer_e_interval
                                        ).await;
                                        
                                        // Restart timer E for retransmissions in Proceeding state
                                        start_timer_e(timer_e_interval, data.clone(), &mut timer_e_task);
                                        
                                        // Notify TU
                                        let _ = data.events_tx.send(TransactionEvent::ProvisionalResponse {
                                            transaction_id: data.id.clone(),
                                            response: response.clone(),
                                        }).await;
                                    } else if is_final {
                                        // 2xx-6xx -> Completed
                                        let _ = handle_transition(
                                            TransactionState::Completed, 
                                            &data, 
                                            &mut timer_e_task, 
                                            &mut timer_f_task, 
                                            &mut timer_k_task,
                                            &mut timer_e_interval
                                        ).await;
                                        
                                        // Notify TU about success or failure
                                        if status.is_success() {
                                            let _ = data.events_tx.send(TransactionEvent::SuccessResponse {
                                                transaction_id: data.id.clone(),
                                                response: response.clone(),
                                            }).await;
                                        } else {
                                            let _ = data.events_tx.send(TransactionEvent::FailureResponse {
                                                transaction_id: data.id.clone(),
                                                response: response.clone(),
                                            }).await;
                                        }
                                    }
                                },
                                TransactionState::Completed => {
                                    // Retransmission of final response, ignore
                                    trace!(id=%data.id, "Received retransmission of final response in Completed state, ignoring");
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
                            "E" => {
                                // Timer E logic - retransmit the request
                                let current_state = data.state.get();
                                if current_state == TransactionState::Trying || current_state == TransactionState::Proceeding {
                                    debug!(id=%data.id, "Timer E triggered, retransmitting request");
                                    
                                    // Retransmit the request
                                    let request_guard = data.request.lock().await;
                                    if let Err(e) = data.transport.send_message(
                                        Message::Request(request_guard.clone()),
                                        data.remote_addr
                                    ).await {
                                        error!(id=%data.id, error=%e, "Failed to retransmit request");
                                    }
                                    
                                    // Double interval, capped by T2
                                    timer_e_interval = std::cmp::min(timer_e_interval * 2, data.timer_config.t2);
                                    
                                    // Restart timer E with new interval
                                    start_timer_e(timer_e_interval, data.clone(), &mut timer_e_task);
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Timer E fired in invalid state, ignoring");
                                }
                            },
                            "F" => {
                                // Timer F logic - transaction timeout
                                let current_state = data.state.get();
                                if current_state == TransactionState::Trying || current_state == TransactionState::Proceeding {
                                    warn!(id=%data.id, "Timer F (Timeout) fired in state {:?}", current_state);
                                    
                                    // Transition to Terminated
                                    let _ = handle_transition(
                                        TransactionState::Terminated, 
                                        &data, 
                                        &mut timer_e_task, 
                                        &mut timer_f_task, 
                                        &mut timer_k_task,
                                        &mut timer_e_interval
                                    ).await;
                                    
                                    // Notify TU about timeout
                                    let _ = data.events_tx.send(TransactionEvent::TransactionTimeout {
                                        transaction_id: data.id.clone(),
                                    }).await;
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Timer F fired in invalid state, ignoring");
                                }
                            },
                            "K" => {
                                // Timer K logic - terminate after waiting for retransmissions
                                let current_state = data.state.get();
                                if current_state == TransactionState::Completed {
                                    debug!(id=%data.id, "Timer K fired in Completed state, terminating");
                                    
                                    // Transition to Terminated
                                    let _ = handle_transition(
                                        TransactionState::Terminated, 
                                        &data, 
                                        &mut timer_e_task, 
                                        &mut timer_f_task, 
                                        &mut timer_k_task,
                                        &mut timer_e_interval
                                    ).await;
                                } else {
                                    trace!(id=%data.id, state=?current_state, "Timer K fired in invalid state, ignoring");
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
                            &mut timer_e_task, 
                            &mut timer_f_task, 
                            &mut timer_k_task,
                            &mut timer_e_interval
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
                            &mut timer_e_task, 
                            &mut timer_f_task, 
                            &mut timer_k_task,
                            &mut timer_e_interval
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
            cancel_timers(&mut timer_e_task, &mut timer_f_task, &mut timer_k_task);
            debug!(id=%data.id, "Non-INVITE client transaction event loop ended");
        })
    }
}

impl CommonClientTransaction for ClientNonInviteTransaction {
    fn data(&self) -> &Arc<ClientTransactionData> {
        &self.data
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
        utils::transaction_key_from_message(message).map(|key| key == self.data.id).unwrap_or(false)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl TransactionAsync for ClientNonInviteTransaction {
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

impl ClientTransaction for ClientNonInviteTransaction {
    fn initiate(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        let data = self.data.clone();
        
        Box::pin(async move {
            let current_state = data.state.get();
            
            if current_state != TransactionState::Initial {
                return Err(Error::invalid_state_transition(
                    TransactionKind::NonInviteClient,
                    current_state,
                    TransactionState::Trying,
                    Some(data.id.clone())
                ));
            }
            
            debug!(id=%data.id, "Sending initial request");
            
            // Transition to Trying state
            data.state.set(TransactionState::Trying);
            
            // Notify the transaction about the state change
            data.cmd_tx.send(InternalTransactionCommand::TransitionTo(TransactionState::Trying)).await
                .map_err(|e| Error::Other(format!("Failed to send transition command: {}", e)))?;
            
            // Send the request
            let request = {
                let request_guard = data.request.lock().await;
                request_guard.clone()
            };
            
            data.transport.send_message(Message::Request(request), data.remote_addr)
                .await
                .map_err(|e| Error::transport_error(e, "Failed to send request"))?;
            
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